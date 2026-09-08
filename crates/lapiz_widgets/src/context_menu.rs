use iced_core::{
    Clipboard, Element, Event, Layout, Length, Point, Rectangle, Shell, Size, Theme, Vector,
    Widget,
    keyboard::{self, key},
    layout::{Limits, Node},
    mouse, overlay, renderer, touch,
    widget::{Operation, Tree, tree},
    window,
};
use iced_wgpu::Renderer;

pub struct ContextMenu<'a, Message, F> {
    underlay: Element<'a, Message, Theme, Renderer>,
    overlay: F,
}

#[derive(Default)]
struct State {
    show: bool,
    cursor_position: Point,
}

impl<'a, Message, F> ContextMenu<'a, Message, F> {
    pub fn new(underlay: impl Into<Element<'a, Message, Theme, Renderer>>, overlay: F) -> Self {
        Self {
            underlay: underlay.into(),
            overlay,
        }
    }
}

impl<'a, Message, F> Widget<Message, Theme, Renderer> for ContextMenu<'a, Message, F>
where
    F: Fn() -> Element<'a, Message, Theme, Renderer>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.underlay), Tree::new((self.overlay)())]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.underlay, &(self.overlay)()]);
    }

    fn size(&self) -> Size<Length> {
        self.underlay.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        self.underlay
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if state.show {
            let mut content = (self.overlay)();
            content.as_widget_mut().diff(&mut tree.children[1]);
            content
                .as_widget_mut()
                .operate(&mut tree.children[1], layout, renderer, operation);
        } else {
            self.underlay.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                renderer,
                operation,
            );
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if *event == Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
            && cursor.is_over(layout.bounds())
        {
            let state = tree.state.downcast_mut::<State>();
            state.cursor_position = cursor.position().unwrap_or_default();
            state.show = !state.show;
            shell.capture_event();
        }

        self.underlay.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.underlay.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
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
        self.underlay.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_mut::<State>();
        if !state.show {
            return self.underlay.as_widget_mut().overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            );
        }

        let position = state.cursor_position;
        let mut content = (self.overlay)();
        content.as_widget_mut().diff(&mut tree.children[1]);
        Some(
            ContextMenuOverlay {
                position: position + translation,
                tree: &mut tree.children[1],
                content,
                state,
            }
            .overlay(),
        )
    }
}

impl<'a, Message, F> From<ContextMenu<'a, Message, F>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    F: Fn() -> Element<'a, Message, Theme, Renderer> + 'a,
{
    fn from(value: ContextMenu<'a, Message, F>) -> Self {
        Element::new(value)
    }
}

struct ContextMenuOverlay<'a, Message> {
    position: Point,
    tree: &'a mut Tree,
    content: Element<'a, Message, Theme, Renderer>,
    state: &'a mut State,
}

impl<'a, Message: 'a> ContextMenuOverlay<'a, Message> {
    fn overlay(self) -> overlay::Element<'a, Message, Theme, Renderer> {
        overlay::Element::new(Box::new(self))
    }
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for ContextMenuOverlay<'_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> Node {
        let limits = Limits::new(Size::ZERO, bounds);
        let mut content = self
            .content
            .as_widget_mut()
            .layout(self.tree, renderer, &limits);

        // Try to stay inside the viewport.
        let mut position = self.position;
        if position.x + content.size().width > bounds.width {
            position.x = f32::max(0.0, position.x - content.size().width);
        }
        if position.y + content.size().height > bounds.height {
            position.y = f32::max(0.0, position.y - content.size().height);
        }
        content.move_to_mut(position);

        Node::with_children(bounds, vec![content])
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let content_layout = layout
            .children()
            .next()
            .expect("widget: Layout should have a content layout.");

        let mut forward_event_to_children = true;
        let mut capture_event = false;

        match &event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                if *key == keyboard::Key::Named(key::Named::Escape) {
                    self.state.show = false;
                    forward_event_to_children = false;
                    shell.capture_event();
                }
            }

            Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left | mouse::Button::Right,
            ))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if cursor.is_over(content_layout.bounds()) {
                    capture_event = true;
                } else {
                    self.state.show = false;
                    forward_event_to_children = false;
                    shell.request_redraw();
                }
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                // Close when released because buttons send their message on release.
                self.state.show = false;
                capture_event = true;
            }

            Event::Window(window::Event::Resized { .. }) => {
                self.state.show = false;
                forward_event_to_children = false;
                capture_event = true;
            }

            _ => {}
        }

        if forward_event_to_children {
            self.content.as_widget_mut().update(
                self.tree,
                event,
                content_layout,
                cursor,
                renderer,
                clipboard,
                shell,
                &layout.bounds(),
            );
        }
        if capture_event {
            shell.capture_event();
        }
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        let content_layout = layout
            .children()
            .next()
            .expect("widget: Layout should have a content layout.");

        self.content
            .as_widget_mut()
            .operate(self.tree, content_layout, renderer, operation);
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let content_layout = layout
            .children()
            .next()
            .expect("widget: Layout should have a content layout.");

        self.content.as_widget().mouse_interaction(
            self.tree,
            content_layout,
            cursor,
            &layout.bounds(),
            renderer,
        )
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let content_layout = layout
            .children()
            .next()
            .expect("widget: Layout should have a content layout.");

        self.content.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            content_layout,
            cursor,
            &layout.bounds(),
        );
    }
}
