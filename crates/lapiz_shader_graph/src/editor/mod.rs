use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use bevy_color::{Oklcha, Srgba};
use iced_core::{
    Background, Border, Color, Element, Event, Layout, Length, Point, Shell, Size, Transformation,
    Vector,
    alignment::Vertical,
    gradient::ColorStop,
    keyboard::{self, key},
    layout::{self, Limits, Node},
    overlay, pointer,
    pointer::{
        button,
        mouse::{self, Interaction},
    },
    renderer::{self, Quad},
    theme::{Base, Mode},
    widget::{Operation, Tree, tree},
};
use iced_graphics::{
    geometry::{self, Frame, Stroke},
    gradient::Linear,
};
use iced_widget::{
    column, container,
    core::{Rectangle, Widget, pointer::mouse::Cursor},
    overlay::menu,
    row, stack,
};
use indexmap::IndexMap;
use lapiz_i18n::t;
use lapiz_widgets::{button::Button, icon, label::Label};
use uuid::Uuid;

use crate::{
    GraphRenderer, GraphTheme,
    editor::slot::{GraphSlotId, GraphSlotPinPositionCollection},
    graph::{
        Graph, GraphData, GraphResources,
        node::{ErasedGraphNodeMessage, GraphNodeData, GraphNodeId},
        slot::{GraphInputSlotId, GraphOutputSlotId, GraphSlots},
    },
};

pub mod slot;

pub const NODE_WIDTH: f32 = 200.0;

#[derive(Default)]
pub struct GraphEditorState {
    pub path: Vec<GraphEditorPathComponent>,
}

impl GraphEditorState {
    pub fn update<Data: GraphData>(
        &mut self,
        graph: &mut Graph<Data>,
        message: GraphEditorMessage,
    ) {
        match message {
            GraphEditorMessage::Graph(message) => {
                let target = self.resolve_subgraph_mut(graph);
                target.update(message);
            }
            GraphEditorMessage::Editor(GraphEditorEditorMessage::EnterSubgraph(
                comp,
                _snapshot,
            )) => {
                self.path.push(comp);
            }
            GraphEditorMessage::Editor(GraphEditorEditorMessage::BackToSubgraphOrMain(
                maybe_index,
            )) => {
                if let Some(index) = maybe_index {
                    self.path.truncate(index + 1);
                } else {
                    self.path.pop();
                }
            }
        }
    }

    pub fn resolve_subgraph<'a, Data: GraphData>(&self, main: &'a Graph<Data>) -> &'a Graph<Data> {
        let mut current = main;
        for comp in &self.path {
            let node = current.get_node(&comp.node_id).unwrap();
            current = node
                .data
                .subgraphs()
                .into_iter()
                .nth(comp.subgraph_index)
                .unwrap();
        }
        current
    }

    pub fn resolve_subgraph_mut<'a, Data: GraphData>(
        &self,
        main: &'a mut Graph<Data>,
    ) -> &'a mut Graph<Data> {
        let mut current = main;
        for comp in &self.path {
            let node = current.get_node_mut(&comp.node_id).unwrap();
            let subgraph = node
                .data
                .subgraphs_mut()
                .into_iter()
                .nth(comp.subgraph_index)
                .unwrap();
            current = subgraph;
        }
        current
    }
}

pub struct GraphEditor<'a, Data: GraphData> {
    graph: &'a Graph<Data>,
    editor_state: &'a GraphEditorState,
}

impl<'a, Data: GraphData> GraphEditor<'a, Data> {
    pub fn new(graph: &'a Graph<Data>, editor_state: &'a GraphEditorState) -> Self {
        Self {
            graph,
            editor_state,
        }
    }
}

impl<'a, Data: GraphData> From<GraphEditor<'a, Data>>
    for Element<'a, GraphEditorMessage, GraphTheme, GraphRenderer>
{
    fn from(value: GraphEditor<'a, Data>) -> Self {
        let GraphEditor {
            graph,
            editor_state,
        } = value;
        let editor_view = GraphEditorView::new(editor_state.resolve_subgraph(graph), editor_state);

        let mut cur_graph = graph;
        let subgraph_path = editor_state.path.iter().enumerate().map(|(index, comp)| {
            let node = cur_graph.get_node(&comp.node_id).unwrap();
            cur_graph = node
                .data
                .subgraphs()
                .into_iter()
                .nth(comp.subgraph_index)
                .unwrap();
            Button::new(Label::new(t!(node.data.id())))
                .on_press(GraphEditorMessage::Editor(
                    GraphEditorEditorMessage::BackToSubgraphOrMain(Some(index)),
                ))
                .into()
        });
        let main_graph_path = Button::new(Label::new(t!("main_graph"))).on_press(
            GraphEditorMessage::Editor(GraphEditorEditorMessage::BackToSubgraphOrMain(None)),
        );
        let path_breadcrumb = row![main_graph_path].extend(subgraph_path);
        stack!(editor_view, path_breadcrumb).into()
    }
}

#[derive(Debug, Clone)]
pub struct GraphEditorSnapshot {
    pub translation: Vector,
    pub selected_nodes: Vec<GraphNodeId>,
}

#[derive(Debug, Clone)]
pub enum GraphEditorMessage {
    Graph(GraphEditorGraphMessage),
    Editor(GraphEditorEditorMessage),
}

#[derive(Debug, Clone)]
pub enum GraphEditorGraphMessage {
    NodeCreateRequest(Point, &'static str, GraphNodeId),
    NodeMoveRequest(Point, GraphNodeId),
    NodeDeleteRequest(GraphNodeId),
    EdgeCreateRequest(GraphOutputSlotId, GraphInputSlotId),
    EdgeRemoveRequest(GraphInputSlotId),
    NodeUpdate(ErasedGraphNodeMessage),
    Format(HashMap<GraphNodeId, Rectangle>, HashSet<GraphNodeId>),
}

#[derive(Debug, Clone)]
pub enum GraphEditorEditorMessage {
    EnterSubgraph(GraphEditorPathComponent, GraphEditorSnapshot),
    BackToSubgraphOrMain(Option<usize>),
}

impl<Data: GraphData> Graph<Data> {
    pub fn update(&mut self, message: GraphEditorGraphMessage) {
        match message {
            GraphEditorGraphMessage::NodeCreateRequest(position, name, node_id) => {
                let node = self.resources.node_registry.get(name).unwrap();
                self.insert_boxed_node(node_id, position, node);
            }
            GraphEditorGraphMessage::NodeMoveRequest(position, id) => {
                self.get_node_mut(&id).unwrap().position = position;
            }
            GraphEditorGraphMessage::NodeDeleteRequest(id) => self.delete_node(&id),
            GraphEditorGraphMessage::EdgeCreateRequest(from, to) => {
                self.connect_slots(from, to);
            }
            GraphEditorGraphMessage::EdgeRemoveRequest(to) => self.disconnect_slot(to),
            GraphEditorGraphMessage::NodeUpdate(message) => self.update_node(message),
            GraphEditorGraphMessage::Format(bounds, selected) => self.format(&bounds, &selected),
        }
    }
}

pub struct GraphEditorView<'a, Data: GraphData> {
    graph: DrawableGraph<'a>,
    node_creation_menu_items: Vec<NodeCreationMenuItem>,
    node_creation_menu_class: <GraphTheme as menu::Catalog>::Class<'a>,
    snapshot: Option<GraphEditorSnapshot>,
    subgraphs: HashMap<GraphNodeId, Vec<&'a Graph<Data>>>,
    _state: &'a GraphEditorState,
}

impl<'a, Data: GraphData> GraphEditorView<'a, Data> {
    pub fn new(graph: &'a Graph<Data>, state: &'a GraphEditorState) -> Self {
        Self {
            graph: DrawableGraph::new(graph),
            node_creation_menu_items: graph
                .resources
                .node_registry
                .all()
                .keys()
                .map(|title| NodeCreationMenuItem { node_title: title })
                .collect(),
            node_creation_menu_class: <GraphTheme as menu::Catalog>::default(),
            _state: state,
            snapshot: None,
            subgraphs: graph
                .nodes
                .iter()
                .filter_map(|(node_id, node)| {
                    let subgraphs = node.data.subgraphs();
                    if subgraphs.is_empty() {
                        None
                    } else {
                        Some((*node_id, subgraphs))
                    }
                })
                .collect(),
        }
    }

    pub fn recover(mut self, snapshot: GraphEditorSnapshot) -> Self {
        self.snapshot = Some(snapshot);
        self
    }
}

#[derive(Debug, Clone)]
pub struct GraphEditorPathComponent {
    pub node_id: GraphNodeId,
    pub subgraph_index: usize,
}

#[derive(Clone)]
pub struct NodeCreationMenuItem {
    pub node_title: &'static str,
}

impl std::fmt::Display for NodeCreationMenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&t!(self.node_title))
    }
}

pub struct GraphNodeStyle {
    pub background: Background,
    pub padding: f32,
    pub line_height: f32,
    pub line_spacing: f32,
}

pub struct DrawableGraph<'a> {
    pub nodes: IndexMap<GraphNodeId, DrawableNode<'a>>,
    pub slots: HashMap<GraphSlotId, SlotData>,
    pub edges: HashMap<GraphInputSlotId, DrawableEdge>,
    pub vert_in_loop: HashSet<GraphNodeId>,
}

impl<'a> DrawableGraph<'a> {
    pub fn new<Data: GraphData>(graph: &'a Graph<Data>) -> Self {
        let mut nodes = IndexMap::with_capacity(graph.nodes.len());
        let mut node_indices = HashMap::with_capacity(graph.nodes.len());
        for (index, (id, node)) in graph.nodes.iter().enumerate() {
            nodes.insert(
                *id,
                DrawableNode::new(*id, node, &graph.slots, graph.resources()),
            );
            node_indices.insert(*id, index);
        }

        let edges = graph
            .slots
            .inputs
            .iter()
            .filter_map(|(to, to_slot)| {
                let from = graph.slots.inputs.get(to)?.connected?;
                let from_slot = graph.slots.outputs.get(&from)?;

                let (from_hue, from_chroma) = from_slot.data_ty.hue_chroma();
                let (to_hue, to_chroma) = to_slot.data.ty().hue_chroma();

                Some((
                    *to,
                    DrawableEdge {
                        from,
                        from_hue,
                        from_chroma,
                        to_hue,
                        to_chroma,
                    },
                ))
            })
            .collect();

        let slots = graph
            .slots
            .inputs
            .iter()
            .map(|(id, slot)| {
                let (hue, chroma) = slot.data.ty().hue_chroma();
                ((*id).into(), SlotData { hue, chroma })
            })
            .chain(graph.slots.outputs.iter().map(|(id, slot)| {
                let (hue, chroma) = slot.data_ty.hue_chroma();
                ((*id).into(), SlotData { hue, chroma })
            }))
            .collect();

        Self {
            nodes,
            edges,
            slots,
            vert_in_loop: graph.find_loops().into_iter().flatten().collect(),
        }
    }
}

pub struct SlotData {
    pub hue: f32,
    pub chroma: f32,
}

pub struct DrawableEdge {
    from: GraphOutputSlotId,
    from_hue: f32,
    from_chroma: f32,
    to_hue: f32,
    to_chroma: f32,
}

pub struct DrawableNode<'a> {
    pub node_id: GraphNodeId,
    pub position: Point,
    pub widget: Element<'a, GraphEditorMessage, GraphTheme, GraphRenderer>,
    pub input_slots: Arc<[GraphInputSlotId]>,
    pub output_slots: Arc<[GraphOutputSlotId]>,
}

impl<'a> DrawableNode<'a> {
    pub fn new<Data: GraphData>(
        node_id: GraphNodeId,
        node: &'a GraphNodeData<Data>,
        slots: &GraphSlots,
        resources: &GraphResources<Data>,
    ) -> Self {
        let (header_hue, header_chroma) = node.data.header_hue_chroma();
        let header = container(
            row![
                container(iced_widget::Space::new().width(3).height(10)).style(move |theme| {
                    container::Style {
                        background: Some(themed_color(theme, header_hue, header_chroma).into()),
                        ..Default::default()
                    }
                }),
                Label::new(t!(node.data.id())).size(12).strong(),
                iced_widget::space().width(Length::Fill),
                icon::grip().size(9).muted(),
            ]
            .align_y(Vertical::Center)
            .spacing(6)
            .padding([0, 6])
            .height(24),
        )
        .style(move |theme| {
            let accent = themed_color(theme, header_hue, header_chroma);
            let panel = theme.palette().background.weaker.color;
            container::Style {
                background: Some(accent.mix(panel, 0.2).into()),
                ..Default::default()
            }
        })
        .width(Length::Fill);

        let widget = container(
            column![
                header,
                node.view(node_id, slots, resources)
                    .map(|m| GraphEditorMessage::Graph(GraphEditorGraphMessage::NodeUpdate(m))),
            ]
            .width(NODE_WIDTH),
        )
        .style(|t| container::Style {
            background: Some(t.palette().background.weaker.color.into()),
            ..Default::default()
        });

        Self {
            node_id,
            position: node.position,
            widget: Element::new(widget),
            input_slots: node.inputs.clone(),
            output_slots: node.outputs.clone(),
        }
    }
}

impl<'a, Data: GraphData> Widget<GraphEditorMessage, GraphTheme, GraphRenderer>
    for GraphEditorView<'a, Data>
{
    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(
            &mut self
                .graph
                .nodes
                .iter_mut()
                .map(|(_, n)| &mut n.widget)
                .collect::<Vec<_>>(),
        );
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &GraphRenderer,
        limits: &layout::Limits,
    ) -> Node {
        let state = tree.state.downcast_mut::<State>();
        state.node_bounds.clear();

        let children = self
            .graph
            .nodes
            .values_mut()
            .zip(&mut tree.children)
            .map(|(node, tree)| {
                let layout = node
                    .widget
                    .as_widget_mut()
                    .layout(tree, renderer, &Limits::NONE)
                    .translate(Vector::new(node.position.x, node.position.y));
                state.node_bounds.insert(node.node_id, layout.bounds());
                layout
            })
            .collect();
        Node::with_children(
            limits.resolve(Length::Fill, Length::Fill, Size::ZERO),
            children,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &GraphRenderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            self.graph
                .nodes
                .values_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
                .for_each(|((child, state), layout)| {
                    child
                        .widget
                        .as_widget_mut()
                        .operate(state, layout, renderer, operation);
                });
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &GraphRenderer,
        shell: &mut Shell<'_, GraphEditorMessage>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let view_transformation = state.view_transformation(layout.bounds());
        let inverse_view_transformation = view_transformation.inverse();
        let graph_cursor = cursor * inverse_view_transformation;
        let graph_viewport = *viewport * inverse_view_transformation;

        if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            state.keyboard_modifiers = *modifiers;
        }
        state.slot_pins.clear();
        let mut messages = iced_core::shell::Bus::new();
        let mut children_shell = shell.local(&mut messages);
        for ((child, tree), layout) in self
            .graph
            .nodes
            .values_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child.widget.as_widget_mut().update(
                tree,
                event,
                layout,
                graph_cursor,
                renderer,
                &mut children_shell,
                &graph_viewport,
            );

            child
                .widget
                .as_widget_mut()
                .operate(tree, layout, renderer, &mut state.slot_pins);
        }
        shell.merge(children_shell, |m| m);

        if shell.is_event_captured() {
            dbg!();
            return;
        }

        const SLOT_PIN_SNAP: f32 = 3.0 * 3.0;
        let slot_pin_snap = SLOT_PIN_SNAP / state.view_scale;
        match event {
            Event::Pointer(event) if event.is_secondary_press() => {
                let Some(cursor) = cursor.position_over(layout.bounds()) else {
                    return;
                };
                state.node_creation_menu.position = Some(cursor);
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Pointer(pointer::Event::PointerPressed {
                button: button::Source::Mouse(mouse::Button::Middle),
                ..
            }) => {
                let Some(cursor) = cursor.position_over(layout.bounds()) else {
                    return;
                };

                state.interaction = InteractionState::ViewDragging {
                    cursor_origin: cursor,
                    translation_origin: state.view_translation,
                };
                shell.capture_event();
            }
            Event::Pointer(pointer::Event::PointerReleased {
                button: button::Source::Mouse(mouse::Button::Middle),
                ..
            }) => {
                if matches!(state.interaction, InteractionState::ViewDragging { .. }) {
                    state.interaction = InteractionState::Idle;
                    shell.capture_event();
                }
            }
            Event::Pointer(event) if event.is_primary_press() => {
                if cursor.position_over(layout.bounds()).is_none() {
                    return;
                }
                let Some(cursor) = graph_cursor.position() else {
                    return;
                };

                for (slot_id, slot_pos) in state.slot_pins.all() {
                    let d = slot_pos.distance(cursor);
                    if d > slot_pin_snap {
                        continue;
                    }

                    let resolved_source = match slot_id {
                        GraphSlotId::Input(id) => {
                            shell.publish(GraphEditorMessage::Graph(
                                GraphEditorGraphMessage::EdgeRemoveRequest(*id),
                            ));

                            self.graph
                                .edges
                                .get(id)
                                .map(|e| GraphSlotId::Output(e.from))
                                .unwrap_or(GraphSlotId::Input(*id))
                        }
                        GraphSlotId::Output(id) => GraphSlotId::Output(*id),
                    };
                    let Some(slot_data) = self.graph.slots.get(slot_id) else {
                        continue;
                    };

                    state.interaction = InteractionState::EdgeConnecting {
                        resolved_source,
                        hue: slot_data.hue,
                        chroma: slot_data.chroma,
                    };
                    shell.capture_event();
                    return;
                }

                for (node_index, node_layout) in layout.children().enumerate() {
                    if !node_layout.bounds().contains(cursor) {
                        continue;
                    }

                    let node_id = self.graph.nodes[node_index].node_id;
                    if let Some(last_click_on_node) = &state.last_click_on_node
                        && last_click_on_node.elapsed().as_secs_f32() < 0.2
                        && let Some(_) = self.subgraphs.get(&node_id)
                    {
                        shell.publish(GraphEditorMessage::Editor(
                            GraphEditorEditorMessage::EnterSubgraph(
                                GraphEditorPathComponent {
                                    node_id,
                                    // TODO add support for multiple subgraphs
                                    subgraph_index: 0,
                                },
                                GraphEditorSnapshot {
                                    translation: state.view_translation,
                                    selected_nodes: state.selected_nodes.iter().copied().collect(),
                                },
                            ),
                        ));
                        return;
                    }
                    state.last_click_on_node = Some(Instant::now());

                    if state.selected_nodes.is_empty() {
                        state.selected_nodes.insert(node_id);
                    } else if state.keyboard_modifiers.control() {
                        if !state.selected_nodes.remove(&node_id) {
                            state.selected_nodes.insert(node_id);
                        }
                    } else if !state.selected_nodes.contains(&node_id) {
                        state.selected_nodes.clear();
                        state.selected_nodes.insert(node_id);
                    }
                    state.interaction = InteractionState::NodeDragging {
                        cursor_origin: cursor,
                        node_origin: state
                            .selected_nodes
                            .iter()
                            .filter_map(|id| {
                                self.graph.nodes.get(id).map(|node| (*id, node.position))
                            })
                            .collect(),
                        skip_next_release: false,
                    };
                    shell.request_redraw();
                    shell.capture_event();
                    return;
                }

                let mode = if state.keyboard_modifiers.shift() {
                    MarqueeMode::Add
                } else {
                    state.selected_nodes.clear();
                    MarqueeMode::Replace
                };
                state.interaction = InteractionState::SelectionDragging {
                    cursor_origin: cursor,
                    originally_selected: state.selected_nodes.clone(),
                    mode,
                };
                shell.capture_event();
            }
            Event::Pointer(e @ pointer::Event::PointerReleased { .. })
                if e.is_primary_release() =>
            {
                match std::mem::take(&mut state.interaction) {
                    InteractionState::NodeDragging {
                        cursor_origin,
                        node_origin,
                        skip_next_release,
                    } => {
                        if skip_next_release {
                            state.interaction = InteractionState::NodeDragging {
                                cursor_origin,
                                node_origin,
                                skip_next_release: false,
                            };
                        }
                        shell.capture_event();
                    }
                    InteractionState::EdgeConnecting {
                        resolved_source, ..
                    } => {
                        let mut found = None;
                        for (slot_id, slot_pos) in state.slot_pins.all() {
                            let Some(cursor) = graph_cursor.position() else {
                                return;
                            };
                            if slot_pos.distance(cursor) < slot_pin_snap {
                                found = Some(*slot_id);
                                break;
                            }
                        }

                        if let Some(end) = found {
                            match (resolved_source, end) {
                                (GraphSlotId::Input(to), GraphSlotId::Output(from))
                                | (GraphSlotId::Output(from), GraphSlotId::Input(to)) => {
                                    shell.publish(GraphEditorMessage::Graph(
                                        GraphEditorGraphMessage::EdgeCreateRequest(from, to),
                                    ));
                                }
                                _ => {}
                            }
                        }
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    InteractionState::SelectionDragging {
                        cursor_origin,
                        originally_selected,
                        mode,
                    } => {
                        let Some(cursor) = graph_cursor.position() else {
                            state.interaction = InteractionState::SelectionDragging {
                                cursor_origin,
                                originally_selected,
                                mode,
                            };
                            return;
                        };
                        let selection_rect = Rectangle {
                            x: cursor_origin.x.min(cursor.x),
                            y: cursor_origin.y.min(cursor.y),
                            width: (cursor_origin.x - cursor.x).abs(),
                            height: (cursor_origin.y - cursor.y).abs(),
                        };
                        state.selected_nodes = match mode {
                            MarqueeMode::Replace => HashSet::new(),
                            MarqueeMode::Add => originally_selected,
                        };
                        for (node, layout) in self.graph.nodes.keys().zip(layout.children()) {
                            if selection_rect.intersects(&layout.bounds()) {
                                state.selected_nodes.insert(*node);
                            }
                        }
                        shell.request_redraw();
                        shell.capture_event();
                    }
                    interaction => state.interaction = interaction,
                }
            }
            Event::Pointer(pointer::Event::PointerMoved { .. }) => match &state.interaction {
                InteractionState::Idle => {}
                InteractionState::EdgeConnecting { .. } => {
                    shell.request_redraw();
                    shell.capture_event();
                }
                InteractionState::NodeDragging {
                    cursor_origin,
                    node_origin,
                    ..
                } => {
                    let Some(cursor) = graph_cursor.position() else {
                        return;
                    };
                    for selected in &state.selected_nodes {
                        if let Some(node_origin) = node_origin.get(selected) {
                            shell.publish(GraphEditorMessage::Graph(
                                GraphEditorGraphMessage::NodeMoveRequest(
                                    *node_origin + (cursor - *cursor_origin),
                                    *selected,
                                ),
                            ));
                        }
                    }
                    shell.capture_event();
                }
                InteractionState::SelectionDragging {
                    cursor_origin,
                    originally_selected,
                    mode,
                } => {
                    let Some(cursor) = graph_cursor.position() else {
                        return;
                    };
                    let selection_rect = Rectangle {
                        x: cursor_origin.x.min(cursor.x),
                        y: cursor_origin.y.min(cursor.y),
                        width: (cursor_origin.x - cursor.x).abs(),
                        height: (cursor_origin.y - cursor.y).abs(),
                    };
                    state.selected_nodes = match mode {
                        MarqueeMode::Replace => HashSet::new(),
                        MarqueeMode::Add => originally_selected.clone(),
                    };
                    for (node, layout) in self.graph.nodes.keys().zip(layout.children()) {
                        if selection_rect.intersects(&layout.bounds()) {
                            state.selected_nodes.insert(*node);
                        }
                    }
                    shell.request_redraw();
                    shell.capture_event();
                }
                InteractionState::ViewDragging {
                    cursor_origin,
                    translation_origin,
                } => {
                    let Some(cursor) = cursor.position() else {
                        return;
                    };
                    state.view_translation = *translation_origin + (cursor - *cursor_origin);
                    shell.capture_event();
                    shell.request_redraw();
                }
            },
            Event::Pointer(pointer::Event::WheelScrolled { delta }) => {
                let Some(cursor) = cursor.position_over(layout.bounds()) else {
                    return;
                };
                let Some(graph_cursor) = graph_cursor.position() else {
                    return;
                };
                let delta = match delta {
                    mouse::ScrollDelta::Lines { x: _, y } => *y,
                    mouse::ScrollDelta::Pixels { x: _, y } => *y / 30.0,
                };
                let new_scale = (state.view_scale * 1.1_f32.powf(delta)).clamp(0.1, 10.0);
                let origin = layout.position();

                state.view_scale = new_scale;
                state.view_translation = (cursor - origin) - (graph_cursor - origin) * new_scale;
                shell.capture_event();
                shell.request_redraw();
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                physical_key,
                modifiers,
                repeat,
                ..
            }) => {
                if *repeat {
                    return;
                }
                let key::Physical::Code(key) = *physical_key else {
                    return;
                };

                // TODO: make them configurable
                match key {
                    key::Code::Delete => {
                        for node_id in state.selected_nodes.drain() {
                            shell.publish(GraphEditorMessage::Graph(
                                GraphEditorGraphMessage::NodeDeleteRequest(node_id),
                            ));
                        }
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    key::Code::KeyA if modifiers.contains(keyboard::Modifiers::SHIFT) => {
                        state.node_creation_menu.position = cursor.position();
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    key::Code::KeyF if modifiers.shift() && modifiers.alt() => {
                        shell.publish(GraphEditorMessage::Graph(GraphEditorGraphMessage::Format(
                            state.node_bounds.clone(),
                            state.selected_nodes.clone(),
                        )));
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &GraphRenderer,
    ) -> Interaction {
        let state = tree.state.downcast_ref::<State>();
        let inverse_view_transformation = state.view_transformation(layout.bounds()).inverse();
        let graph_cursor = cursor * inverse_view_transformation;
        let graph_viewport = *viewport * inverse_view_transformation;

        if matches!(state.interaction, InteractionState::NodeDragging { .. }) {
            mouse::Interaction::Grabbing
        } else {
            self.graph
                .nodes
                .values()
                .zip(&tree.children)
                .zip(layout.children())
                .map(|((child, tree), layout)| {
                    child.widget.as_widget().mouse_interaction(
                        tree,
                        layout,
                        graph_cursor,
                        &graph_viewport,
                        renderer,
                    )
                })
                .max()
                .unwrap_or_default()
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut GraphRenderer,
        theme: &iced_core::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let view_transformation = state.view_transformation(layout.bounds());
        let inverse_view_transformation = view_transformation.inverse();
        let graph_cursor = cursor * inverse_view_transformation;
        let graph_viewport = *viewport * inverse_view_transformation;
        {
            use iced_core::Renderer;

            renderer.fill_quad(
                Quad {
                    bounds: layout.bounds(),
                    ..Default::default()
                },
                theme.palette().background.base.color,
            );
        }

        let mut frame = Frame::with_bounds(renderer, graph_viewport);
        {
            let ink = theme.palette().background.base.text;
            let width = 1.0 / view_transformation.scale_factor();
            for (step, alpha) in [(20.0, 0.04), (100.0, 0.08)] {
                let stroke = Stroke {
                    style: ink.scale_alpha(alpha).into(),
                    width,
                    ..Default::default()
                };
                let start = (graph_viewport.x / step).floor() * step;
                let mut x = start;
                while x < graph_viewport.x + graph_viewport.width {
                    frame.stroke(
                        &geometry::Path::line(
                            Point::new(x, graph_viewport.y),
                            Point::new(x, graph_viewport.y + graph_viewport.height),
                        ),
                        stroke,
                    );
                    x += step;
                }
                let start = (graph_viewport.y / step).floor() * step;
                let mut y = start;
                while y < graph_viewport.y + graph_viewport.height {
                    frame.stroke(
                        &geometry::Path::line(
                            Point::new(graph_viewport.x, y),
                            Point::new(graph_viewport.x + graph_viewport.width, y),
                        ),
                        stroke,
                    );
                    y += step;
                }
            }
        }
        for (to, edge) in &self.graph.edges {
            let from_pos = state.slot_pins.get_output(&edge.from);
            let to_pos = state.slot_pins.get_input(to);
            if let (Some(from_pos), Some(to_pos)) = (from_pos, to_pos) {
                let style = if edge.from_hue == edge.to_hue && edge.from_chroma == edge.to_chroma {
                    geometry::Style::Solid(themed_color(theme, edge.from_hue, edge.from_chroma))
                } else {
                    let g = Linear::new(Point::new(0.0, 0.0), Point::new(1000.0, 1000.0))
                        .add_stops([
                            ColorStop {
                                offset: 0.0,
                                color: themed_color(theme, edge.from_hue, edge.from_chroma),
                            },
                            ColorStop {
                                offset: 1.0,
                                color: themed_color(theme, edge.to_hue, edge.to_chroma),
                            },
                        ]);
                    geometry::Style::Gradient(g.into())
                };

                let dx = ((to_pos.x - from_pos.x).abs() * 0.6).clamp(45.0, 160.0);
                frame.stroke(
                    &geometry::Path::new(|path| {
                        path.move_to(*from_pos);
                        path.bezier_curve_to(
                            Point::new(from_pos.x + dx, from_pos.y),
                            Point::new(to_pos.x - dx, to_pos.y),
                            *to_pos,
                        );
                    }),
                    Stroke {
                        style,
                        width: 2.0,
                        ..Default::default()
                    },
                );
                frame.fill_rectangle(
                    Point::new(
                        (from_pos.x + to_pos.x) / 2.0 - 2.5,
                        (from_pos.y + to_pos.y) / 2.0 - 2.5,
                    ),
                    Size::new(5.0, 5.0),
                    themed_color(theme, edge.from_hue, edge.from_chroma),
                );
            }
        }

        {
            use iced_core::Renderer;

            renderer.with_layer(layout.bounds(), |renderer| {
                renderer.with_transformation(view_transformation, |renderer| {
                    use iced_graphics::geometry::Renderer;
                    renderer.draw_geometry(frame.into_geometry());
                });
            });
        }

        {
            use iced_core::Renderer;
            renderer.with_layer(layout.bounds(), |renderer| {
                renderer.with_transformation(view_transformation, |renderer| {
                    for ((child, node_tree), node_layout) in self
                        .graph
                        .nodes
                        .values()
                        .zip(&tree.children)
                        .zip(layout.children())
                        .filter(|(_, layout)| layout.bounds().intersects(&graph_viewport))
                    {
                        let node_bounds = node_layout.bounds();
                        let selected = state.selected_nodes.contains(&child.node_id);
                        let shadow_offset = if selected { 4.0 } else { 3.0 };
                        let shadow_alpha = if selected { 0.28 } else { 0.2 };
                        renderer.fill_quad(
                            Quad {
                                bounds: Rectangle::new(
                                    Point::new(
                                        node_bounds.x + shadow_offset,
                                        node_bounds.y + shadow_offset,
                                    ),
                                    node_bounds.size(),
                                ),
                                ..Default::default()
                            },
                            Color::BLACK.scale_alpha(shadow_alpha),
                        );
                        if selected {
                            renderer.fill_quad(
                                Quad {
                                    bounds: node_bounds.expand(2.0),
                                    border: Border::default()
                                        .width(2.0)
                                        .color(theme.palette().primary.base.color),
                                    ..Default::default()
                                },
                                Color::TRANSPARENT,
                            );
                        }
                        child.widget.as_widget().draw(
                            node_tree,
                            renderer,
                            theme,
                            style,
                            node_layout,
                            graph_cursor,
                            &graph_viewport,
                        );
                        if self.graph.vert_in_loop.contains(&child.node_id) {
                            renderer.fill_quad(
                                Quad {
                                    bounds: node_layout.bounds(),
                                    ..Default::default()
                                },
                                Color::from_rgb8(255, 0, 0).scale_alpha(0.3),
                            );
                        }
                    }
                });
            });
        }

        if let (
            InteractionState::EdgeConnecting {
                resolved_source,
                hue,
                chroma,
            },
            Some(cursor_pos),
        ) = (&state.interaction, graph_cursor.position())
            && let Some(start_pos) = state.slot_pins.get(resolved_source)
        {
            let mut frame = Frame::with_bounds(renderer, graph_viewport);
            let dx = ((cursor_pos.x - start_pos.x).abs() * 0.6).clamp(45.0, 160.0);
            frame.stroke(
                &geometry::Path::new(|path| {
                    path.move_to(*start_pos);
                    path.bezier_curve_to(
                        Point::new(start_pos.x + dx, start_pos.y),
                        Point::new(cursor_pos.x - dx, cursor_pos.y),
                        cursor_pos,
                    );
                }),
                Stroke {
                    style: themed_color(theme, *hue, *chroma).into(),
                    width: 2.0,
                    ..Default::default()
                },
            );

            use iced_core::Renderer;
            renderer.with_layer(layout.bounds(), |renderer| {
                renderer.with_transformation(view_transformation, |renderer| {
                    use iced_graphics::geometry::Renderer;
                    renderer.draw_geometry(frame.into_geometry());
                });
            });
        };

        if let InteractionState::SelectionDragging { cursor_origin, .. } = &state.interaction {
            let Some(cursor_pos) = graph_cursor.position() else {
                return;
            };
            use iced_core::Renderer;
            renderer.with_layer(layout.bounds(), |renderer| {
                renderer.with_transformation(view_transformation, |renderer| {
                    renderer.fill_quad(
                        Quad {
                            bounds: Rectangle {
                                x: cursor_origin.x.min(cursor_pos.x),
                                y: cursor_origin.y.min(cursor_pos.y),
                                width: (cursor_origin.x - cursor_pos.x).abs(),
                                height: (cursor_origin.y - cursor_pos.y).abs(),
                            },
                            border: Border::default()
                                .width(2.0)
                                .color(theme.palette().primary.strong.color.scale_alpha(0.5)),
                            ..Default::default()
                        },
                        theme.palette().primary.base.color.scale_alpha(0.3),
                    );
                });
            });
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &GraphRenderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, GraphEditorMessage, GraphTheme, GraphRenderer>> {
        let view_transformation = tree
            .state
            .downcast_ref::<State>()
            .view_transformation(layout.bounds());
        let inverse_view_transformation = view_transformation.inverse();
        let graph_viewport = *viewport * inverse_view_transformation;

        // Menus inside nodes assume the viewport starts at (0, 0) when measuring the
        // space above/below themselves, so node overlays live in a viewport-relative
        // frame: the viewport origin is subtracted here and restored by
        // TransformedGraphOverlay after layout.
        let viewport_origin = Vector::new(graph_viewport.x, graph_viewport.y);
        for ((child, tree), layout) in self
            .graph
            .nodes
            .values_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            if let Some(overlay) = child.widget.as_widget_mut().overlay(
                tree,
                layout,
                renderer,
                &Rectangle::new(Point::ORIGIN, graph_viewport.size()),
                translation - viewport_origin,
            ) {
                return Some(overlay::Element::new(Box::new(
                    TransformedGraphOverlay::new(overlay, view_transformation, viewport_origin),
                )));
            }
        }

        let state = tree.state.downcast_mut::<State>();
        if let Some(menu_position) = state.node_creation_menu.position {
            let graph_origin = Vector::new(layout.position().x, layout.position().y);
            let graph_cursor = menu_position * inverse_view_transformation;
            let node_position = graph_cursor - graph_origin;
            let NodeCreationMenuState {
                position,
                state: menu_state,
                hovered,
            } = &mut state.node_creation_menu;
            let selected_nodes = &mut state.selected_nodes;
            let interaction = &mut state.interaction;
            let menu = menu::Menu::new(
                menu_state,
                &self.node_creation_menu_items,
                hovered,
                &|item: &NodeCreationMenuItem| item.node_title.to_string(),
                move |name| {
                    position.take();
                    let node_id = GraphNodeId::new(Uuid::new_v4());
                    selected_nodes.clear();
                    selected_nodes.insert(node_id);
                    *interaction = InteractionState::NodeDragging {
                        cursor_origin: graph_cursor,
                        node_origin: HashMap::from([(node_id, node_position)]),
                        skip_next_release: true,
                    };
                    GraphEditorMessage::Graph(GraphEditorGraphMessage::NodeCreateRequest(
                        node_position,
                        name.node_title,
                        node_id,
                    ))
                },
                None,
                &self.node_creation_menu_class,
            )
            .width(200.0)
            .padding(2);

            return Some(menu.overlay(menu_position, *viewport, 0.0, Length::Shrink));
        }

        None
    }
}

impl<'a, Data: GraphData> From<GraphEditorView<'a, Data>>
    for Element<'a, GraphEditorMessage, GraphTheme, GraphRenderer>
{
    fn from(value: GraphEditorView<'a, Data>) -> Self {
        Element::new(value)
    }
}

struct TransformedGraphOverlay<'a> {
    content: overlay::Element<'a, GraphEditorMessage, GraphTheme, GraphRenderer>,
    transformation: Transformation,
    origin: Vector,
}

impl<'a> TransformedGraphOverlay<'a> {
    fn new(
        content: overlay::Element<'a, GraphEditorMessage, GraphTheme, GraphRenderer>,
        transformation: Transformation,
        origin: Vector,
    ) -> Self {
        Self {
            content,
            transformation,
            origin,
        }
    }
}

impl iced_core::Overlay<GraphEditorMessage, GraphTheme, GraphRenderer>
    for TransformedGraphOverlay<'_>
{
    fn layout(&mut self, renderer: &GraphRenderer, bounds: Size) -> Node {
        let content = self
            .content
            .as_overlay_mut()
            .layout(renderer, bounds * self.transformation.inverse())
            .translate(self.origin);
        let content_bounds = content.bounds();
        let transformed_bounds = content_bounds * self.transformation;

        Node::with_children(
            transformed_bounds.size(),
            vec![content.move_to(Point::new(
                content_bounds.x - transformed_bounds.x,
                content_bounds.y - transformed_bounds.y,
            ))],
        )
        .move_to(transformed_bounds.position())
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &GraphRenderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_overlay_mut().operate(
            layout.children().next().unwrap_or(layout),
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &GraphRenderer,
        shell: &mut Shell<'_, GraphEditorMessage>,
    ) {
        self.content.as_overlay_mut().update(
            event,
            layout.children().next().unwrap_or(layout),
            cursor * self.transformation.inverse(),
            renderer,
            shell,
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &GraphRenderer,
    ) -> Interaction {
        self.content.as_overlay().mouse_interaction(
            layout.children().next().unwrap_or(layout),
            cursor * self.transformation.inverse(),
            renderer,
        )
    }

    fn draw(
        &self,
        renderer: &mut GraphRenderer,
        theme: &GraphTheme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: Cursor,
    ) {
        use iced_core::Renderer;

        renderer.with_transformation(self.transformation, |renderer| {
            self.content.as_overlay().draw(
                renderer,
                theme,
                style,
                layout.children().next().unwrap_or(layout),
                cursor * self.transformation.inverse(),
            );
        });
    }

    fn overlay<'a>(
        &'a mut self,
        layout: Layout<'a>,
        renderer: &GraphRenderer,
    ) -> Option<overlay::Element<'a, GraphEditorMessage, GraphTheme, GraphRenderer>> {
        let transformation = self.transformation;

        self.content
            .as_overlay_mut()
            .overlay(layout.children().next().unwrap_or(layout), renderer)
            .map(|overlay| {
                overlay::Element::new(Box::new(TransformedGraphOverlay::new(
                    overlay,
                    transformation,
                    Vector::ZERO,
                )))
            })
    }

    fn index(&self) -> f32 {
        self.content.as_overlay().index()
    }
}

struct State {
    view_translation: Vector,
    view_scale: f32,
    keyboard_modifiers: keyboard::Modifiers,

    node_creation_menu: NodeCreationMenuState,
    last_click_on_node: Option<Instant>,
    selected_nodes: HashSet<GraphNodeId>,
    interaction: InteractionState,
    node_bounds: HashMap<GraphNodeId, Rectangle>,
    slot_pins: GraphSlotPinPositionCollection,
}

impl State {
    fn view_transformation(&self, bounds: Rectangle) -> Transformation {
        let origin = bounds.position();

        Transformation::translate(
            origin.x + self.view_translation.x,
            origin.y + self.view_translation.y,
        ) * Transformation::scale(self.view_scale)
            * Transformation::translate(-origin.x, -origin.y)
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            view_translation: Default::default(),
            view_scale: 1.0,
            keyboard_modifiers: Default::default(),
            node_creation_menu: Default::default(),
            last_click_on_node: Default::default(),
            selected_nodes: Default::default(),
            interaction: Default::default(),
            node_bounds: Default::default(),
            slot_pins: Default::default(),
        }
    }
}

#[derive(Default)]
struct NodeCreationMenuState {
    position: Option<Point>,
    state: menu::State,
    hovered: Option<usize>,
}

#[derive(Default)]
enum InteractionState {
    #[default]
    Idle,
    NodeDragging {
        cursor_origin: Point,
        node_origin: HashMap<GraphNodeId, Point>,
        skip_next_release: bool,
    },
    EdgeConnecting {
        resolved_source: GraphSlotId,
        hue: f32,
        chroma: f32,
    },
    SelectionDragging {
        cursor_origin: Point,
        originally_selected: HashSet<GraphNodeId>,
        mode: MarqueeMode,
    },
    ViewDragging {
        cursor_origin: Point,
        translation_origin: Vector,
    },
}

#[derive(Clone, Copy)]
enum MarqueeMode {
    Replace,
    Add,
}

pub fn themed_color(theme: &GraphTheme, hue: f32, chroma: f32) -> Color {
    let oklch = Oklcha::new(
        match theme.mode() {
            Mode::None => 0.6,
            Mode::Light => 0.7,
            Mode::Dark => 0.72,
        },
        chroma,
        hue,
        1.0,
    );
    let rgb = Srgba::from(oklch);
    Color {
        r: rgb.red,
        g: rgb.green,
        b: rgb.blue,
        a: rgb.alpha,
    }
}
