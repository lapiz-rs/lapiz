use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
};

use iced_core::{Point, Rectangle, Size};

use crate::shader_graph::graph::{Graph, node::GraphNodeId};

const RANK_GAP: f32 = 120.0;
const NODE_GAP: f32 = 40.0;
const COMPONENT_GAP: f32 = 160.0;
const CROSSING_REDUCTION_PASSES: usize = 4;
const MAX_CENTER_ALIGNMENT_PASSES: usize = 16;
const CENTER_ALIGNMENT_EPSILON: f32 = 0.01;

impl Graph {
    pub fn format(
        &mut self,
        node_sizes: &HashMap<GraphNodeId, Rectangle>,
        nodes_to_format: &HashSet<GraphNodeId>,
    ) {
        if self.nodes.is_empty() {
            return;
        }

        if !self
            .nodes
            .keys()
            .any(|node_id| nodes_to_format.contains(node_id))
        {
            return;
        }

        let missing_size_count = self
            .nodes
            .keys()
            .filter(|node_id| !node_sizes.contains_key(node_id))
            .count();
        if missing_size_count != 0 {
            log::warn!(
                "Skipping graph formatting because {missing_size_count} nodes have not been measured"
            );
            return;
        }

        let invalid_size_count = node_sizes
            .iter()
            .filter(|(node_id, size)| {
                self.nodes.contains_key(node_id)
                    && (!size.width.is_finite()
                        || !size.height.is_finite()
                        || size.width <= 0.0
                        || size.height <= 0.0)
            })
            .count();
        if invalid_size_count != 0 {
            log::warn!(
                "Skipping graph formatting because {invalid_size_count} nodes have invalid sizes"
            );
            return;
        }

        let mut node_ids = self.nodes.keys().copied().collect::<Vec<_>>();
        node_ids.sort_by(|a, b| {
            let a_node = &self.nodes[a];
            let b_node = &self.nodes[b];
            stable_position_cmp(a_node.position, b_node.position)
                .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
        });

        let node_indices = node_ids
            .iter()
            .enumerate()
            .map(|(index, node_id)| (*node_id, index))
            .collect::<HashMap<_, _>>();
        let mut nodes = node_ids
            .iter()
            .map(|node_id| {
                let node = &self.nodes[node_id];
                let size = node_sizes[node_id];
                LayoutNode {
                    id: *node_id,
                    width: size.width,
                    height: size.height,
                    old_position: node.position,
                    predecessors: Vec::new(),
                    successors: Vec::new(),
                }
            })
            .collect::<Vec<_>>();

        let mut edge_weights = BTreeMap::<(usize, usize), u64>::new();
        for (from_id, from_node) in &self.nodes {
            let Some(&from_index) = node_indices.get(from_id) else {
                continue;
            };
            for output_id in from_node.outputs.iter() {
                let Some(output) = self.slots.outputs.get(output_id) else {
                    continue;
                };
                for input_id in &output.connected {
                    let Some(to_id) = self.slots.inputs.get(input_id).map(|input| input.node_id)
                    else {
                        continue;
                    };
                    let Some(&to_index) = node_indices.get(&to_id) else {
                        continue;
                    };
                    if from_index != to_index {
                        *edge_weights.entry((from_index, to_index)).or_default() += 1;
                    }
                }
            }
        }

        for ((from, to), weight) in edge_weights {
            nodes[from].successors.push(LayoutEdge { node: to, weight });
            nodes[to]
                .predecessors
                .push(LayoutEdge { node: from, weight });
        }

        let movable = node_ids
            .iter()
            .map(|node_id| nodes_to_format.contains(node_id))
            .collect::<Vec<_>>();
        let ideal_positions = compute_positions(&nodes);
        let positions = apply_partial_layout(&nodes, &movable, &ideal_positions);
        for (index, node_id) in node_ids.into_iter().enumerate() {
            if movable[index]
                && let Some(node) = self.nodes.get_mut(&node_id)
            {
                node.position = positions[index];
            }
        }
    }
}

#[derive(Clone, Copy)]
struct LayoutEdge {
    node: usize,
    weight: u64,
}

struct LayoutNode {
    id: GraphNodeId,
    width: f32,
    height: f32,
    old_position: Point<f32>,
    predecessors: Vec<LayoutEdge>,
    successors: Vec<LayoutEdge>,
}

fn compute_positions(nodes: &[LayoutNode]) -> Vec<Point<f32>> {
    if nodes.is_empty() {
        return Vec::new();
    }

    let strongly_connected = strongly_connected_components(nodes);
    let mut component_of_node = vec![0; nodes.len()];
    for (component_index, component) in strongly_connected.iter().enumerate() {
        for &node in component {
            component_of_node[node] = component_index;
        }
    }

    let mut component_successors = vec![BTreeMap::new(); strongly_connected.len()];
    for (from, node) in nodes.iter().enumerate() {
        let from_component = component_of_node[from];
        for edge in &node.successors {
            let to_component = component_of_node[edge.node];
            if from_component != to_component {
                *component_successors[from_component]
                    .entry(to_component)
                    .or_default() += edge.weight;
            }
        }
    }

    let component_ranks = minimum_edge_length_ranks(&component_successors);
    let ranks = component_of_node
        .iter()
        .map(|component| component_ranks[*component])
        .collect::<Vec<_>>();
    let weak_components = weakly_connected_components(nodes);
    let mut positions = vec![Point::default(); nodes.len()];
    let mut component_top = 0.0;

    for component in weak_components {
        let min_rank = component.iter().map(|node| ranks[*node]).min().unwrap_or(0);
        let max_rank = component.iter().map(|node| ranks[*node]).max().unwrap_or(0);
        let mut layers = vec![Vec::new(); max_rank - min_rank + 1];
        for node in component {
            layers[ranks[node] - min_rank].push(node);
        }
        for layer in &mut layers {
            layer.sort_by(|a, b| {
                stable_position_cmp(nodes[*a].old_position, nodes[*b].old_position)
                    .then_with(|| nodes[*a].id.0.as_bytes().cmp(nodes[*b].id.0.as_bytes()))
            });
        }

        reduce_crossings(&mut layers, &ranks, nodes);

        let layer_widths = layers
            .iter()
            .map(|layer| {
                layer
                    .iter()
                    .map(|node| nodes[*node].width)
                    .fold(0.0, f32::max)
            })
            .collect::<Vec<_>>();
        let mut layer_centers = vec![0.0; layers.len()];
        if let Some(first_width) = layer_widths.first() {
            layer_centers[0] = *first_width * 0.5;
        }
        for rank in 1..layers.len() {
            layer_centers[rank] = layer_centers[rank - 1]
                + layer_widths[rank - 1] * 0.5
                + RANK_GAP
                + layer_widths[rank] * 0.5;
        }

        let layer_heights = layers
            .iter()
            .map(|layer| {
                layer.iter().map(|node| nodes[*node].height).sum::<f32>()
                    + NODE_GAP * layer.len().saturating_sub(1) as f32
            })
            .collect::<Vec<_>>();
        let initial_component_height = layer_heights.iter().copied().fold(0.0, f32::max);
        let mut node_centers_y = vec![0.0; nodes.len()];

        for (rank, layer) in layers.iter().enumerate() {
            let mut top = (initial_component_height - layer_heights[rank]) * 0.5;
            for &node in layer {
                node_centers_y[node] = top + nodes[node].height * 0.5;
                top += nodes[node].height + NODE_GAP;
            }
        }

        align_node_centers(&layers, &ranks, nodes, &mut node_centers_y);

        let component_min_top = layers
            .iter()
            .flatten()
            .map(|node| node_centers_y[*node] - nodes[*node].height * 0.5)
            .fold(f32::INFINITY, f32::min);
        let component_max_bottom = layers
            .iter()
            .flatten()
            .map(|node| node_centers_y[*node] + nodes[*node].height * 0.5)
            .fold(f32::NEG_INFINITY, f32::max);
        let component_offset = component_top - component_min_top;

        for (rank, layer) in layers.iter().enumerate() {
            for &node in layer {
                positions[node] = Point::new(
                    layer_centers[rank] - nodes[node].width * 0.5,
                    node_centers_y[node] + component_offset - nodes[node].height * 0.5,
                );
            }
        }

        component_top += component_max_bottom - component_min_top + COMPONENT_GAP;
    }

    let anchor = nodes
        .iter()
        .map(|node| node.old_position)
        .filter(|position| position.x.is_finite() && position.y.is_finite())
        .reduce(|a, b| Point::new(a.x.min(b.x), a.y.min(b.y)))
        .unwrap_or_default();

    for position in &mut positions {
        position.x += anchor.x;
        position.y += anchor.y;
    }

    positions
}

// Partial layout keeps each movable component's ideal internal geometry. Fixed
// boundary neighbors determine its translation, then fixed rectangles and
// already placed movable components constrain the nearest clear vertical slot.
fn apply_partial_layout(
    nodes: &[LayoutNode],
    movable: &[bool],
    ideal_positions: &[Point<f32>],
) -> Vec<Point<f32>> {
    debug_assert_eq!(nodes.len(), movable.len());
    debug_assert_eq!(nodes.len(), ideal_positions.len());

    let mut positions = nodes
        .iter()
        .map(|node| node.old_position)
        .collect::<Vec<_>>();
    let has_fixed_nodes = movable.iter().any(|movable| !movable);
    let mut obstacles = nodes
        .iter()
        .enumerate()
        .filter(|(node, _)| !movable[*node])
        .map(|(_, node)| Rectangle::new(node.old_position, Size::new(node.width, node.height)))
        .collect::<Vec<_>>();

    for component in movable_components(nodes, movable) {
        let mut weighted_delta_x = 0.0_f64;
        let mut weighted_delta_y = 0.0_f64;
        let mut anchor_weight = 0.0_f64;
        for &node in &component {
            for edge in nodes[node]
                .predecessors
                .iter()
                .chain(&nodes[node].successors)
            {
                if movable[edge.node] {
                    continue;
                }
                let weight = edge.weight as f64;
                weighted_delta_x += (nodes[edge.node].old_position.x - ideal_positions[edge.node].x)
                    as f64
                    * weight;
                weighted_delta_y += (nodes[edge.node].old_position.y - ideal_positions[edge.node].y)
                    as f64
                    * weight;
                anchor_weight += weight;
            }
        }

        let translation = if anchor_weight > 0.0 {
            Point::new(
                (weighted_delta_x / anchor_weight) as f32,
                (weighted_delta_y / anchor_weight) as f32,
            )
        } else if has_fixed_nodes {
            let old_center = component
                .iter()
                .fold(Point::<f32>::default(), |center, node| {
                    let old = nodes[*node].old_position;
                    Point::new(center.x + old.x, center.y + old.y)
                });
            let ideal_center = component
                .iter()
                .fold(Point::<f32>::default(), |center, node| {
                    let ideal = ideal_positions[*node];
                    Point::new(center.x + ideal.x, center.y + ideal.y)
                });
            Point::new(
                (old_center.x - ideal_center.x) / component.len() as f32,
                (old_center.y - ideal_center.y) / component.len() as f32,
            )
        } else {
            Point::default()
        };

        let component_rects = component
            .iter()
            .map(|node| {
                let ideal = ideal_positions[*node];
                let node = &nodes[*node];
                Rectangle::new(
                    Point::new(ideal.x + translation.x, ideal.y + translation.y),
                    Size::new(node.width, node.height),
                )
            })
            .collect::<Vec<_>>();
        let vertical_offset = nearest_clear_vertical_offset(&component_rects, &obstacles);

        for (&node, rect) in component.iter().zip(component_rects) {
            let top_left = Point::new(rect.x, rect.y + vertical_offset);
            positions[node] = top_left;
            obstacles.push(Rectangle::new(top_left, rect.size()));
        }
    }

    positions
}

fn movable_components(nodes: &[LayoutNode], movable: &[bool]) -> Vec<Vec<usize>> {
    let mut visited = vec![false; nodes.len()];
    let mut components = Vec::new();
    for start in 0..nodes.len() {
        if !movable[start] || visited[start] {
            continue;
        }

        visited[start] = true;
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some(node) = queue.pop_front() {
            component.push(node);
            for neighbor in nodes[node]
                .predecessors
                .iter()
                .chain(&nodes[node].successors)
                .map(|edge| edge.node)
            {
                if movable[neighbor] && !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

fn nearest_clear_vertical_offset(rects: &[Rectangle], obstacles: &[Rectangle]) -> f32 {
    let mut forbidden = Vec::new();
    for rect in rects {
        for obstacle in obstacles {
            if rect.x < obstacle.x + obstacle.width && rect.x + rect.width > obstacle.x {
                forbidden.push((
                    obstacle.y - NODE_GAP - (rect.y + rect.height),
                    obstacle.y + NODE_GAP - rect.y,
                ));
            }
        }
    }
    forbidden.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));

    let mut merged = Vec::<(f32, f32)>::new();
    for interval in forbidden {
        if let Some(last) = merged.last_mut()
            && interval.0 <= last.1
        {
            last.1 = last.1.max(interval.1);
        } else {
            merged.push(interval);
        }
    }

    merged
        .into_iter()
        .find_map(|(start, end)| {
            (start < 0.0 && end > 0.0).then(|| if -start <= end { start } else { end })
        })
        .unwrap_or(0.0)
}

fn align_node_centers(
    layers: &[Vec<usize>],
    ranks: &[usize],
    nodes: &[LayoutNode],
    centers_y: &mut [f32],
) {
    for _ in 0..MAX_CENTER_ALIGNMENT_PASSES {
        let mut max_movement = 0.0_f32;
        for layer in layers {
            max_movement = max_movement.max(align_layer_centers(layer, ranks, nodes, centers_y));
        }
        for layer in layers.iter().rev() {
            max_movement = max_movement.max(align_layer_centers(layer, ranks, nodes, centers_y));
        }
        if max_movement <= CENTER_ALIGNMENT_EPSILON {
            break;
        }
    }
}

// A layer's unconstrained optimum is the weighted average of neighboring
// centers. Weighted isotonic regression projects those targets onto the
// fixed-order, non-overlapping positions in one pass.
fn align_layer_centers(
    layer: &[usize],
    ranks: &[usize],
    nodes: &[LayoutNode],
    centers_y: &mut [f32],
) -> f32 {
    struct AlignmentBlock {
        start: usize,
        end: usize,
        weight: f64,
        weighted_target: f64,
    }

    if layer.is_empty() {
        return 0.0;
    }

    let mut desired_centers = Vec::with_capacity(layer.len());
    let mut weights = Vec::with_capacity(layer.len());
    for &node in layer {
        let mut weighted_center = 0.0_f64;
        let mut total_weight = 0.0_f64;
        for edge in nodes[node]
            .predecessors
            .iter()
            .chain(&nodes[node].successors)
        {
            if ranks[edge.node] != ranks[node] {
                weighted_center += centers_y[edge.node] as f64 * edge.weight as f64;
                total_weight += edge.weight as f64;
            }
        }

        if total_weight == 0.0 {
            desired_centers.push(centers_y[node] as f64);
            weights.push(1.0);
        } else {
            desired_centers.push(weighted_center / total_weight);
            weights.push(total_weight);
        }
    }

    let mut offsets = vec![0.0_f64; layer.len()];
    for index in 1..layer.len() {
        offsets[index] = offsets[index - 1]
            + nodes[layer[index - 1]].height as f64 * 0.5
            + NODE_GAP as f64
            + nodes[layer[index]].height as f64 * 0.5;
    }

    let mut blocks = Vec::<AlignmentBlock>::with_capacity(layer.len());
    for index in 0..layer.len() {
        let target = desired_centers[index] - offsets[index];
        blocks.push(AlignmentBlock {
            start: index,
            end: index + 1,
            weight: weights[index],
            weighted_target: target * weights[index],
        });

        while blocks.len() >= 2 {
            let right = &blocks[blocks.len() - 1];
            let left = &blocks[blocks.len() - 2];
            if left.weighted_target / left.weight <= right.weighted_target / right.weight {
                break;
            }

            let right = blocks.pop().unwrap();
            let left = blocks.pop().unwrap();
            blocks.push(AlignmentBlock {
                start: left.start,
                end: right.end,
                weight: left.weight + right.weight,
                weighted_target: left.weighted_target + right.weighted_target,
            });
        }
    }

    let mut max_movement = 0.0_f32;
    for block in blocks {
        let block_position = block.weighted_target / block.weight;
        for index in block.start..block.end {
            let node = layer[index];
            let center = (block_position + offsets[index]) as f32;
            max_movement = max_movement.max((centers_y[node] - center).abs());
            centers_y[node] = center;
        }
    }
    max_movement
}

// Each rank is represented by threshold booleans `rank >= k`. The weighted
// edge-length objective and all precedence constraints then form an s-t min-cut
// problem. This produces the same minimum-edge-length rank objective used by
// network-simplex rankers without adding layout-only nodes to the graph.
fn minimum_edge_length_ranks(successors: &[BTreeMap<usize, u64>]) -> Vec<usize> {
    if successors.is_empty() {
        return Vec::new();
    }

    let mut indegrees = vec![0; successors.len()];
    for edges in successors {
        for &to in edges.keys() {
            indegrees[to] += 1;
        }
    }

    let mut ready = indegrees
        .iter()
        .enumerate()
        .filter_map(|(node, indegree)| (*indegree == 0).then_some(node))
        .collect::<BTreeSet<_>>();
    let mut topological_order = Vec::with_capacity(successors.len());
    let mut earliest_ranks = vec![0; successors.len()];
    while let Some(node) = ready.pop_first() {
        topological_order.push(node);
        for &successor in successors[node].keys() {
            earliest_ranks[successor] = earliest_ranks[successor].max(earliest_ranks[node] + 1);
            indegrees[successor] -= 1;
            if indegrees[successor] == 0 {
                ready.insert(successor);
            }
        }
    }
    debug_assert_eq!(topological_order.len(), successors.len());

    let max_rank = earliest_ranks.iter().copied().max().unwrap_or(0);
    if max_rank == 0 {
        return vec![0; successors.len()];
    }

    let mut incoming_weights = vec![0_u64; successors.len()];
    let mut outgoing_weights = vec![0_u64; successors.len()];
    for (from, edges) in successors.iter().enumerate() {
        for (&to, &weight) in edges {
            outgoing_weights[from] += weight;
            incoming_weights[to] += weight;
        }
    }

    let finite_cut_capacity = incoming_weights
        .iter()
        .zip(&outgoing_weights)
        .map(|(incoming, outgoing)| incoming.abs_diff(*outgoing))
        .sum::<u64>()
        .saturating_mul(max_rank as u64);
    let infinite_capacity = finite_cut_capacity.saturating_add(1);
    let source = 0;
    let sink = 1;
    let threshold_node = |node: usize, threshold: usize| 2 + node * max_rank + threshold - 1;
    let mut flow = FlowNetwork::new(2 + successors.len() * max_rank);

    for node in 0..successors.len() {
        let incoming = incoming_weights[node];
        let outgoing = outgoing_weights[node];
        for threshold in 1..=max_rank {
            let variable = threshold_node(node, threshold);
            if incoming > outgoing {
                flow.add_edge(variable, sink, incoming - outgoing);
            } else if outgoing > incoming {
                flow.add_edge(source, variable, outgoing - incoming);
            }
            if threshold > 1 {
                flow.add_edge(
                    variable,
                    threshold_node(node, threshold - 1),
                    infinite_capacity,
                );
            }
        }
    }

    for (from, edges) in successors.iter().enumerate() {
        if !edges.is_empty() {
            flow.add_edge(threshold_node(from, max_rank), sink, infinite_capacity);
        }
        for &to in edges.keys() {
            flow.add_edge(source, threshold_node(to, 1), infinite_capacity);
            for threshold in 1..max_rank {
                flow.add_edge(
                    threshold_node(from, threshold),
                    threshold_node(to, threshold + 1),
                    infinite_capacity,
                );
            }
        }
    }

    flow.max_flow(source, sink);
    let reachable = flow.reachable_from(source);
    let ranks = (0..successors.len())
        .map(|node| {
            (1..=max_rank)
                .take_while(|threshold| reachable[threshold_node(node, *threshold)])
                .count()
        })
        .collect::<Vec<_>>();

    debug_assert!(
        successors
            .iter()
            .enumerate()
            .all(|(from, edges)| edges.keys().all(|to| ranks[*to] > ranks[from]))
    );
    ranks
}

#[derive(Clone, Copy)]
struct FlowEdge {
    to: usize,
    reverse: usize,
    capacity: u64,
}

struct FlowNetwork {
    edges: Vec<Vec<FlowEdge>>,
}

impl FlowNetwork {
    fn new(node_count: usize) -> Self {
        Self {
            edges: vec![Vec::new(); node_count],
        }
    }

    fn add_edge(&mut self, from: usize, to: usize, capacity: u64) {
        let forward_reverse = self.edges[to].len();
        let backward_reverse = self.edges[from].len();
        self.edges[from].push(FlowEdge {
            to,
            reverse: forward_reverse,
            capacity,
        });
        self.edges[to].push(FlowEdge {
            to: from,
            reverse: backward_reverse,
            capacity: 0,
        });
    }

    fn max_flow(&mut self, source: usize, sink: usize) -> u64 {
        let mut total_flow = 0_u64;
        loop {
            let levels = self.levels_from(source);
            if levels[sink] == usize::MAX {
                return total_flow;
            }
            let mut next_edges = vec![0; self.edges.len()];
            loop {
                let flow = self.send_flow(source, sink, u64::MAX, &levels, &mut next_edges);
                if flow == 0 {
                    break;
                }
                total_flow = total_flow.saturating_add(flow);
            }
        }
    }

    fn levels_from(&self, source: usize) -> Vec<usize> {
        let mut levels = vec![usize::MAX; self.edges.len()];
        levels[source] = 0;
        let mut queue = VecDeque::from([source]);
        while let Some(node) = queue.pop_front() {
            for edge in &self.edges[node] {
                if edge.capacity > 0 && levels[edge.to] == usize::MAX {
                    levels[edge.to] = levels[node] + 1;
                    queue.push_back(edge.to);
                }
            }
        }
        levels
    }

    fn send_flow(
        &mut self,
        node: usize,
        sink: usize,
        available: u64,
        levels: &[usize],
        next_edges: &mut [usize],
    ) -> u64 {
        if node == sink {
            return available;
        }

        while next_edges[node] < self.edges[node].len() {
            let edge_index = next_edges[node];
            let edge = self.edges[node][edge_index];
            if edge.capacity > 0 && levels[edge.to] == levels[node] + 1 {
                let sent = self.send_flow(
                    edge.to,
                    sink,
                    available.min(edge.capacity),
                    levels,
                    next_edges,
                );
                if sent > 0 {
                    self.edges[node][edge_index].capacity -= sent;
                    self.edges[edge.to][edge.reverse].capacity += sent;
                    return sent;
                }
            }
            next_edges[node] += 1;
        }
        0
    }

    fn reachable_from(&self, source: usize) -> Vec<bool> {
        let mut reachable = vec![false; self.edges.len()];
        reachable[source] = true;
        let mut queue = VecDeque::from([source]);
        while let Some(node) = queue.pop_front() {
            for edge in &self.edges[node] {
                if edge.capacity > 0 && !reachable[edge.to] {
                    reachable[edge.to] = true;
                    queue.push_back(edge.to);
                }
            }
        }
        reachable
    }
}

fn reduce_crossings(layers: &mut [Vec<usize>], ranks: &[usize], nodes: &[LayoutNode]) {
    if layers.len() < 2 {
        return;
    }

    let mut order = vec![usize::MAX; nodes.len()];
    for layer in layers.iter() {
        update_layer_order(layer, &mut order);
    }

    for _ in 0..CROSSING_REDUCTION_PASSES {
        for layer in layers.iter_mut().skip(1) {
            reorder_layer(layer, &order, ranks, nodes, true);
            update_layer_order(layer, &mut order);
        }

        let last_layer = layers.len() - 1;
        for layer in layers[..last_layer].iter_mut().rev() {
            reorder_layer(layer, &order, ranks, nodes, false);
            update_layer_order(layer, &mut order);
        }
    }
}

fn reorder_layer(
    layer: &mut [usize],
    order: &[usize],
    ranks: &[usize],
    nodes: &[LayoutNode],
    use_predecessors: bool,
) {
    layer.sort_by(|a, b| {
        barycenter(*a, order, ranks, nodes, use_predecessors)
            .total_cmp(&barycenter(*b, order, ranks, nodes, use_predecessors))
            .then_with(|| order[*a].cmp(&order[*b]))
            .then_with(|| nodes[*a].id.0.as_bytes().cmp(nodes[*b].id.0.as_bytes()))
    });
}

fn barycenter(
    node: usize,
    order: &[usize],
    ranks: &[usize],
    nodes: &[LayoutNode],
    use_predecessors: bool,
) -> f64 {
    let edges = if use_predecessors {
        &nodes[node].predecessors
    } else {
        &nodes[node].successors
    };
    let mut weighted_sum = 0.0;
    let mut total_weight = 0_u64;
    for edge in edges {
        let is_neighbor_rank = if use_predecessors {
            ranks[edge.node] < ranks[node]
        } else {
            ranks[edge.node] > ranks[node]
        };
        if is_neighbor_rank && order[edge.node] != usize::MAX {
            weighted_sum += order[edge.node] as f64 * edge.weight as f64;
            total_weight += edge.weight;
        }
    }
    if total_weight == 0 {
        order[node] as f64
    } else {
        weighted_sum / total_weight as f64
    }
}

fn update_layer_order(layer: &[usize], order: &mut [usize]) {
    for (index, node) in layer.iter().enumerate() {
        order[*node] = index;
    }
}

fn weakly_connected_components(nodes: &[LayoutNode]) -> Vec<Vec<usize>> {
    let mut visited = vec![false; nodes.len()];
    let mut components = Vec::new();

    for start in 0..nodes.len() {
        if visited[start] {
            continue;
        }
        visited[start] = true;
        let mut queue = VecDeque::from([start]);
        let mut component = Vec::new();
        while let Some(node) = queue.pop_front() {
            component.push(node);
            for neighbor in nodes[node]
                .predecessors
                .iter()
                .chain(&nodes[node].successors)
                .map(|edge| edge.node)
            {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }

    components
}

fn strongly_connected_components(nodes: &[LayoutNode]) -> Vec<Vec<usize>> {
    struct TarjanState {
        next_index: usize,
        indices: Vec<Option<usize>>,
        lowlinks: Vec<usize>,
        stack: Vec<usize>,
        on_stack: Vec<bool>,
        components: Vec<Vec<usize>>,
    }

    fn visit(node: usize, nodes: &[LayoutNode], state: &mut TarjanState) {
        let node_index = state.next_index;
        state.next_index += 1;
        state.indices[node] = Some(node_index);
        state.lowlinks[node] = node_index;
        state.stack.push(node);
        state.on_stack[node] = true;

        for edge in &nodes[node].successors {
            let successor = edge.node;
            if state.indices[successor].is_none() {
                visit(successor, nodes, state);
                state.lowlinks[node] = state.lowlinks[node].min(state.lowlinks[successor]);
            } else if state.on_stack[successor] {
                state.lowlinks[node] = state.lowlinks[node].min(state.indices[successor].unwrap());
            }
        }

        if state.lowlinks[node] == state.indices[node].unwrap() {
            let mut component = Vec::new();
            loop {
                let member = state.stack.pop().unwrap();
                state.on_stack[member] = false;
                component.push(member);
                if member == node {
                    break;
                }
            }
            component.sort_unstable();
            state.components.push(component);
        }
    }

    let mut state = TarjanState {
        next_index: 0,
        indices: vec![None; nodes.len()],
        lowlinks: vec![0; nodes.len()],
        stack: Vec::new(),
        on_stack: vec![false; nodes.len()],
        components: Vec::new(),
    };
    for node in 0..nodes.len() {
        if state.indices[node].is_none() {
            visit(node, nodes, &mut state);
        }
    }
    state.components
}

fn stable_position_cmp(a: Point, b: Point) -> Ordering {
    a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x))
}
