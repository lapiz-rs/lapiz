use iced_core::{
    Element, Event, Layout, Length, Point, Rectangle, Renderer as _, Shell, Size, Theme, Vector,
    Widget, layout, overlay, pointer,
    pointer::mouse,
    renderer,
    widget::{Operation, Tree, tree},
};
use lapiz_runtime::Renderer;

use crate::callback::{CallbackWith, publish_with};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    #[default]
    Horizontal,
    Vertical,
}

pub struct Splitter<'a, Message> {
    first: Element<'a, Message, Theme, Renderer>,
    second: Element<'a, Message, Theme, Renderer>,
    axis: Axis,
    ratio: f32,
    thickness: f32,
    resize: CallbackWith<'a, f32, Message>,
    width: Length,
    height: Length,
}

impl<'a, Message> Splitter<'a, Message> {
    pub fn horizontal(
        first: impl Into<Element<'a, Message, Theme, Renderer>>,
        second: impl Into<Element<'a, Message, Theme, Renderer>>,
        ratio: f32,
    ) -> Self {
        Self::new(first, second, ratio, Axis::Horizontal)
    }

    pub fn vertical(
        first: impl Into<Element<'a, Message, Theme, Renderer>>,
        second: impl Into<Element<'a, Message, Theme, Renderer>>,
        ratio: f32,
    ) -> Self {
        Self::new(first, second, ratio, Axis::Vertical)
    }

    fn new(
        first: impl Into<Element<'a, Message, Theme, Renderer>>,
        second: impl Into<Element<'a, Message, Theme, Renderer>>,
        ratio: f32,
        axis: Axis,
    ) -> Self {
        Self {
            first: first.into(),
            second: second.into(),
            axis,
            ratio: ratio.clamp(0.05, 0.95),
            thickness: 5.0,
            resize: None,
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    crate::callback_methods!(resize, f32);

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness.max(1.0);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    fn handle_bounds(&self, bounds: Rectangle) -> Rectangle {
        match self.axis {
            Axis::Horizontal => Rectangle {
                x: bounds.x + (bounds.width - self.thickness) * self.ratio,
                y: bounds.y,
                width: self.thickness,
                height: bounds.height,
            },
            Axis::Vertical => Rectangle {
                x: bounds.x,
                y: bounds.y + (bounds.height - self.thickness) * self.ratio,
                width: bounds.width,
                height: self.thickness,
            },
        }
    }
}

#[derive(Default)]
struct State {
    dragging: bool,
}

impl<Message> Widget<Message, Theme, Renderer> for Splitter<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [&mut self.first, &mut self.second]);
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = limits.resolve(self.width, self.height, Size::ZERO);
        let available = match self.axis {
            Axis::Horizontal => size.width - self.thickness,
            Axis::Vertical => size.height - self.thickness,
        }
        .max(0.0);
        let first_main = available * self.ratio;
        let second_main = available - first_main;
        let (first_size, second_size, second_position) = match self.axis {
            Axis::Horizontal => (
                Size::new(first_main, size.height),
                Size::new(second_main, size.height),
                Point::new(first_main + self.thickness, 0.0),
            ),
            Axis::Vertical => (
                Size::new(size.width, first_main),
                Size::new(size.width, second_main),
                Point::new(0.0, first_main + self.thickness),
            ),
        };
        let first_limits = layout::Limits::new(first_size, first_size);
        let second_limits = layout::Limits::new(second_size, second_size);
        let first =
            self.first
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &first_limits);
        let second =
            self.second
                .as_widget_mut()
                .layout(&mut tree.children[1], renderer, &second_limits);
        layout::Node::with_children(size, vec![first, second.move_to(second_position)])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((child, state), layout) in [&mut self.first, &mut self.second]
                .into_iter()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                child
                    .as_widget_mut()
                    .operate(state, layout, renderer, operation);
            }
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let handle = self.handle_bounds(layout.bounds());
        match event {
            Event::Pointer(event) if event.is_primary_press() && cursor.is_over(handle) => {
                state.dragging = true;
                shell.capture_event();
            }
            Event::Pointer(e @ pointer::Event::PointerReleased { .. })
                if e.is_primary_release() && state.dragging =>
            {
                state.dragging = false;
                shell.capture_event();
            }
            Event::Pointer(pointer::Event::PointerMoved { position, .. }) if state.dragging => {
                let bounds = layout.bounds();
                let ratio = match self.axis {
                    Axis::Horizontal => {
                        (position.x - bounds.x) / (bounds.width - self.thickness).max(1.0)
                    }
                    Axis::Vertical => {
                        (position.y - bounds.y) / (bounds.height - self.thickness).max(1.0)
                    }
                }
                .clamp(0.05, 0.95);
                if let Some(message) = publish_with(&mut self.resize, ratio) {
                    shell.publish(message);
                }
                shell.capture_event();
            }
            _ => {}
        }
        if shell.is_event_captured() {
            return;
        }
        for ((child, child_state), child_layout) in [&mut self.first, &mut self.second]
            .into_iter()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child.as_widget_mut().update(
                child_state,
                event,
                child_layout,
                cursor,
                renderer,
                shell,
                viewport,
            );
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((child, state), child_layout) in [&self.first, &self.second]
            .into_iter()
            .zip(&tree.children)
            .zip(layout.children())
        {
            child.as_widget().draw(
                state,
                renderer,
                theme,
                style,
                child_layout,
                cursor,
                viewport,
            );
        }
        let p = theme.palette();
        renderer.fill_quad(
            renderer::Quad {
                bounds: self.handle_bounds(layout.bounds()),
                ..renderer::Quad::default()
            },
            if cursor.is_over(self.handle_bounds(layout.bounds())) {
                p.primary.base.color
            } else {
                p.background.strong.color
            },
        );
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(self.handle_bounds(layout.bounds())) {
            match self.axis {
                Axis::Horizontal => mouse::Interaction::ResizingHorizontally,
                Axis::Vertical => mouse::Interaction::ResizingVertically,
            }
        } else {
            mouse::Interaction::None
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let [first_tree, second_tree] = tree.children.as_mut_slice() else {
            return None;
        };
        let mut layouts = layout.children();
        let first = self.first.as_widget_mut().overlay(
            first_tree,
            layouts.next()?,
            renderer,
            viewport,
            translation,
        );
        let second = self.second.as_widget_mut().overlay(
            second_tree,
            layouts.next()?,
            renderer,
            viewport,
            translation,
        );
        let children = [first, second].into_iter().flatten().collect::<Vec<_>>();
        (!children.is_empty()).then(|| overlay::Group::with_children(children).overlay())
    }
}

impl<'a, Message: 'a> From<Splitter<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Splitter<'a, Message>) -> Self {
        Element::new(value)
    }
}
