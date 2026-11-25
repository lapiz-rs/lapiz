use std::{collections::HashMap, hash::Hash, mem};

use iced_core::{
    Element, Event, Layout, Length, Point, Rectangle, Renderer as _, Shell, Size, Vector, layout,
    overlay,
    pointer::{self, mouse},
    renderer,
    widget::{self, Operation, Widget, tree},
    window,
};
use lapiz_runtime::{Renderer, Theme};

const DRAG_THRESHOLD: f32 = 10.0;
const INDENT: f32 = 20.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TreePosition<Id> {
    Above(Id),
    Below(Id),
    First,
    Last,
}

pub struct TreeDragDropInfo<'a, Id> {
    pub dragged_id: &'a Id,
    pub target_parent_id: Option<&'a Id>,
    pub position: TreePosition<&'a Id>,
}

pub enum TreeNode<'a, Message, Id> {
    Leaf(Id, Element<'a, Message, Theme, Renderer>),
    Branch {
        id: Id,
        label: Element<'a, Message, Theme, Renderer>,
        children: Vec<TreeNode<'a, Message, Id>>,
    },
}

type DropCallback<'a, Message, Id> = dyn Fn(TreeDragDropInfo<'_, Id>) -> Message + 'a;
type ParentFilter<'a, Id> = dyn Fn(Option<&Id>, &Id) -> bool + 'a;

pub struct Tree<'a, Message, Id> {
    rows: Vec<Row<Id>>,
    children: Vec<Element<'a, Message, Theme, Renderer>>,
    height: Length,
    spacing: f32,
    draggable: bool,
    on_press: Option<Box<dyn Fn(&Id) -> Message + 'a>>,
    on_drag_drop: Option<Box<DropCallback<'a, Message, Id>>>,
    can_be_child_of: Option<Box<ParentFilter<'a, Id>>>,
}

struct Row<Id> {
    id: Id,
    parent: Option<usize>,
    depth: usize,
    branch: bool,
    last_sibling: bool,
    end: usize,
}

struct State<Id> {
    ids: Vec<Id>,
    gesture: Option<Gesture<Id>>,
}

struct Gesture<Id> {
    id: Id,
    origin: Point,
    pointer: Point,
    dragging: bool,
}

struct DropTarget {
    parent: Option<usize>,
    position: TreePosition<usize>,
    line: Rectangle,
}

impl<'a, Message, Id> Tree<'a, Message, Id> {
    pub fn new(nodes: Vec<TreeNode<'a, Message, Id>>) -> Self {
        let mut tree = Self {
            rows: Vec::new(),
            children: Vec::new(),
            height: Length::Shrink,
            spacing: 0.0,
            draggable: false,
            on_press: None,
            on_drag_drop: None,
            can_be_child_of: None,
        };
        tree.append(nodes, None, 0);
        tree
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn draggable(mut self, draggable: bool) -> Self {
        self.draggable = draggable;
        self
    }

    pub fn on_press(mut self, callback: impl Fn(&Id) -> Message + 'a) -> Self {
        self.on_press = Some(Box::new(callback));
        self
    }

    pub fn on_drag_drop(
        mut self,
        callback: impl Fn(TreeDragDropInfo<'_, Id>) -> Message + 'a,
    ) -> Self {
        self.on_drag_drop = Some(Box::new(callback));
        self
    }

    pub fn can_be_child_of(mut self, filter: impl Fn(Option<&Id>, &Id) -> bool + 'a) -> Self {
        self.can_be_child_of = Some(Box::new(filter));
        self
    }

    fn append(
        &mut self,
        nodes: Vec<TreeNode<'a, Message, Id>>,
        parent: Option<usize>,
        depth: usize,
    ) {
        let count = nodes.len();
        for (sibling, node) in nodes.into_iter().enumerate() {
            let (id, label, children) = match node {
                TreeNode::Leaf(id, label) => (id, label, None),
                TreeNode::Branch {
                    id,
                    label,
                    children,
                } => (id, label, Some(children)),
            };
            let index = self.rows.len();
            self.rows.push(Row {
                id,
                parent,
                depth,
                branch: children.is_some(),
                last_sibling: sibling + 1 == count,
                end: index + 1,
            });
            self.children.push(label);
            if let Some(children) = children {
                self.append(children, Some(index), depth + 1);
                self.rows[index].end = self.rows.len();
            }
        }
    }

    fn resolve_drop(
        &self,
        layout: Layout<'_>,
        viewport: &Rectangle,
        pointer: Point,
        dragged: usize,
    ) -> Option<DropTarget> {
        let bounds = layout.bounds();
        if !bounds.contains(pointer) || !viewport.contains(pointer) {
            return None;
        }

        let moved = dragged..self.rows[dragged].end;
        let accepts = |parent: Option<usize>| {
            !parent.is_some_and(|index| moved.contains(&index))
                && self.can_be_child_of.as_ref().is_none_or(|filter| {
                    filter(
                        parent.map(|index| &self.rows[index].id),
                        &self.rows[dragged].id,
                    )
                })
        };
        let line = |depth: usize, y: f32| {
            let indent = (depth as f32 * INDENT).min(bounds.width);
            Rectangle {
                x: bounds.x + indent,
                y: y.min(bounds.y + (bounds.height - 2.0).max(0.0)),
                width: bounds.width - indent,
                height: 2.0,
            }
        };
        let Some((index, row_bounds)) = layout
            .children()
            .map(|row| row.bounds())
            .enumerate()
            .filter(|(index, _)| !moved.contains(index))
            .find(|(_, row)| pointer.y < row.y + row.height)
        else {
            let y = layout.children().last().map_or(bounds.y, |row| {
                let bounds = row.bounds();
                bounds.y + bounds.height
            });
            return accepts(None).then(|| DropTarget {
                parent: None,
                position: TreePosition::Last,
                line: line(0, y),
            });
        };

        let row = &self.rows[index];
        if pointer.y < row_bounds.y + row_bounds.height / 2.0 {
            return accepts(row.parent).then(|| DropTarget {
                parent: row.parent,
                position: TreePosition::Above(index),
                line: line(row.depth, row_bounds.y),
            });
        }

        let y = row_bounds.y + row_bounds.height;
        if row.branch && pointer.x >= row_bounds.x && accepts(Some(index)) {
            return Some(DropTarget {
                parent: Some(index),
                position: TreePosition::First,
                line: line(row.depth + 1, y),
            });
        }

        // At the end of consecutive last siblings, the gutter selects the destination depth.
        let mut sibling = index;
        while self.rows[sibling].last_sibling {
            let Some(parent) = self.rows[sibling].parent else {
                break;
            };
            let parent_x = bounds.x + self.rows[parent].depth as f32 * INDENT;
            if !self.rows[parent].last_sibling || pointer.x >= parent_x {
                break;
            }
            sibling = parent;
        }
        let row = &self.rows[sibling];
        let last_row = layout.child(row.end - 1).bounds();
        accepts(row.parent).then(|| DropTarget {
            parent: row.parent,
            position: if row.last_sibling {
                TreePosition::Last
            } else {
                TreePosition::Below(sibling)
            },
            line: line(row.depth, last_row.y + last_row.height),
        })
    }
}

impl<Message, Id: Clone + Eq + Hash + 'static> Widget<Message, Theme, Renderer>
    for Tree<'_, Message, Id>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Id>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Id> {
            ids: Vec::new(),
            gesture: None,
        })
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        let state = tree.state.downcast_mut::<State<Id>>();
        let mut previous = state
            .ids
            .drain(..)
            .zip(mem::take(&mut tree.children))
            .collect::<HashMap<_, _>>();
        tree.children = self
            .rows
            .iter()
            .zip(&mut self.children)
            .map(|(row, child)| {
                let mut state = previous.remove(&row.id).unwrap_or_else(widget::Tree::empty);
                state.diff(child.as_widget_mut());
                state
            })
            .collect();
        state.ids = self.rows.iter().map(|row| row.id.clone()).collect();
        if !self.draggable
            || self.on_drag_drop.is_none()
            || state
                .gesture
                .as_ref()
                .is_some_and(|gesture| !state.ids.contains(&gesture.id))
        {
            state.gesture = None;
        }
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, self.height)
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(Length::Fill).height(self.height);
        let width = limits.max().width;
        let mut y = 0.0;
        let nodes = self
            .rows
            .iter()
            .zip(&mut self.children)
            .zip(&mut tree.children)
            .map(|((row, child), state)| {
                let indent = (row.depth as f32 * INDENT).min(width);
                let row_limits =
                    layout::Limits::new(Size::ZERO, Size::new(width - indent, limits.max().height))
                        .width(Length::Fill);
                let node = child.as_widget_mut().layout(state, renderer, &row_limits);
                let top = y;
                y += node.size().height + self.spacing;
                node.move_to(Point::new(indent, top))
            })
            .collect();
        let size = limits.resolve(
            Length::Fill,
            self.height,
            Size::new(width, (y - self.spacing).max(0.0)),
        );
        layout::Node::with_children(size, nodes)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State<Id>>();
        if matches!(event, Event::Window(window::Event::Unfocused))
            && state.gesture.take().is_some()
        {
            shell.request_redraw();
        }
        if let Some(gesture) = &mut state.gesture {
            match event {
                Event::Pointer(pointer::Event::PointerMoved { position, .. }) => {
                    gesture.pointer = *position;
                    gesture.dragging |= position.distance(gesture.origin) > DRAG_THRESHOLD;
                    shell.capture_event();
                    if gesture.dragging {
                        shell.request_redraw();
                    }
                    return;
                }
                Event::Pointer(event @ pointer::Event::PointerReleased { position, .. })
                    if event.is_primary_release() =>
                {
                    let gesture = state.gesture.take().unwrap();
                    shell.capture_event();
                    shell.request_redraw();
                    if gesture.dragging
                        && let Some(callback) = &self.on_drag_drop
                        && let Some(dragged) = self.rows.iter().position(|row| row.id == gesture.id)
                        && let Some(target) =
                            self.resolve_drop(layout, viewport, *position, dragged)
                    {
                        let position = match target.position {
                            TreePosition::Above(index) => TreePosition::Above(&self.rows[index].id),
                            TreePosition::Below(index) => TreePosition::Below(&self.rows[index].id),
                            TreePosition::First => TreePosition::First,
                            TreePosition::Last => TreePosition::Last,
                        };
                        shell.publish(callback(TreeDragDropInfo {
                            dragged_id: &gesture.id,
                            target_parent_id: target.parent.map(|index| &self.rows[index].id),
                            position,
                        }));
                    }
                    return;
                }
                _ => {}
            }
        }

        for ((child, state), layout) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child
                .as_widget_mut()
                .update(state, event, layout, cursor, renderer, shell, viewport);
            if shell.is_event_captured() {
                return;
            }
        }
        if let Event::Pointer(event @ pointer::Event::PointerPressed { position, .. }) = event
            && event.is_primary_press()
            && viewport.contains(*position)
            && layout.bounds().contains(*position)
            && let Some(index) = layout.children().position(|row| {
                let row = row.bounds();
                position.y >= row.y && position.y < row.y + row.height
            })
        {
            let id = &self.rows[index].id;
            if let Some(callback) = &self.on_press {
                shell.publish(callback(id));
                shell.capture_event();
            }
            if self.draggable && self.on_drag_drop.is_some() {
                tree.state.downcast_mut::<State<Id>>().gesture = Some(Gesture {
                    id: id.clone(),
                    origin: *position,
                    pointer: *position,
                    dragging: false,
                });
                shell.capture_event();
            }
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((child, state), layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
        {
            if layout.bounds().intersects(viewport) {
                child
                    .as_widget()
                    .draw(state, renderer, theme, style, layout, cursor, viewport);
            }
        }
        let state = tree.state.downcast_ref::<State<Id>>();
        if let Some(gesture) = &state.gesture
            && gesture.dragging
            && let Some(dragged) = self.rows.iter().position(|row| row.id == gesture.id)
            && let Some(target) = self.resolve_drop(layout, viewport, gesture.pointer, dragged)
            && let Some(bounds) = target.line.intersection(viewport)
        {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    ..Default::default()
                },
                theme.palette().primary.strong.color,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if tree
            .state
            .downcast_ref::<State<Id>>()
            .gesture
            .as_ref()
            .is_some_and(|gesture| gesture.dragging)
        {
            return mouse::Interaction::Grabbing;
        }
        let interaction = self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, state), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(state, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default();
        if interaction == mouse::Interaction::None
            && (self.on_press.is_some() || (self.draggable && self.on_drag_drop.is_some()))
            && cursor.position().is_some_and(|position| {
                viewport.contains(position)
                    && layout.children().any(|row| row.bounds().contains(position))
            })
        {
            mouse::Interaction::Pointer
        } else {
            interaction
        }
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((child, state), layout) in self
                .children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                child
                    .as_widget_mut()
                    .operate(state, layout, renderer, operation);
            }
        });
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a, Id: Clone + Eq + Hash + 'static> From<Tree<'a, Message, Id>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(tree: Tree<'a, Message, Id>) -> Self {
        Element::new(tree)
    }
}

#[cfg(test)]
mod tests {
    use iced_core::widget::Void;

    use crate::tree::*;

    fn leaf(id: u8) -> TreeNode<'static, (), u8> {
        TreeNode::Leaf(id, Void.into())
    }

    fn branch(id: u8, children: Vec<TreeNode<'static, (), u8>>) -> TreeNode<'static, (), u8> {
        TreeNode::Branch {
            id,
            label: Void.into(),
            children,
        }
    }

    fn layout_for(tree: &Tree<'_, (), u8>) -> layout::Node {
        let nodes = tree
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let x = row.depth as f32 * INDENT;
                layout::Node::new(Size::new(120.0 - x, 20.0))
                    .move_to(Point::new(x, index as f32 * 20.0))
            })
            .collect();
        layout::Node::with_children(Size::new(120.0, 160.0), nodes)
    }

    #[test]
    fn drop_uses_sibling_anchors_in_both_directions() {
        let tree = Tree::new(vec![leaf(0), leaf(1), leaf(2)]);
        let node = layout_for(&tree);
        let layout = Layout::new(&node);
        let viewport = layout.bounds();
        for (dragged, y, expected) in [
            (0, 35.0, TreePosition::Below(1)),
            (2, 25.0, TreePosition::Above(1)),
        ] {
            let target = tree
                .resolve_drop(layout, &viewport, Point::new(80.0, y), dragged)
                .unwrap();
            assert_eq!(target.parent, None);
            assert_eq!(target.position, expected);
        }
    }

    #[test]
    fn bottom_gutter_selects_ancestor_insertion_depth() {
        let tree = Tree::new(vec![leaf(0), branch(1, vec![branch(2, vec![leaf(3)])])]);
        let node = layout_for(&tree);
        let layout = Layout::new(&node);
        let viewport = layout.bounds();
        for (x, parent) in [(60.0, Some(2)), (4.0, Some(1))] {
            let target = tree
                .resolve_drop(layout, &viewport, Point::new(x, 75.0), 0)
                .unwrap();
            assert_eq!(target.parent, parent);
            assert_eq!(target.position, TreePosition::Last);
        }
    }

    #[test]
    fn lower_half_of_branch_accepts_first_child() {
        let tree = Tree::new(vec![leaf(0), branch(1, vec![leaf(2)])]);
        let node = layout_for(&tree);
        let layout = Layout::new(&node);
        let target = tree
            .resolve_drop(layout, &layout.bounds(), Point::new(80.0, 35.0), 0)
            .unwrap();
        assert_eq!(target.parent, Some(1));
        assert_eq!(target.position, TreePosition::First);
        assert_eq!(target.line.position(), Point::new(INDENT, 40.0));
    }

    #[test]
    fn drop_skips_dragged_subtree_and_respects_parent_filter() {
        let tree = Tree::new(vec![
            branch(0, vec![branch(1, vec![leaf(2)])]),
            branch(3, vec![leaf(4)]),
        ])
        .can_be_child_of(|parent, _| parent.is_none());
        let node = layout_for(&tree);
        let layout = Layout::new(&node);
        let viewport = layout.bounds();
        let position = |y| Point::new(80.0, y);
        let target = tree
            .resolve_drop(layout, &viewport, position(35.0), 0)
            .unwrap();
        assert_eq!(target.parent, None);
        assert_eq!(target.position, TreePosition::Above(3));
        assert!(
            tree.resolve_drop(layout, &viewport, position(85.0), 0)
                .is_none()
        );
        let target = tree
            .resolve_drop(layout, &viewport, position(75.0), 0)
            .unwrap();
        assert_eq!(target.position, TreePosition::Last);
        assert_eq!(target.line.y, 100.0);
        assert!(
            tree.resolve_drop(layout, &viewport, position(200.0), 0)
                .is_none()
        );
    }
}
