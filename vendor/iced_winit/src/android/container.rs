//! A widget that lays out and draws the logical windows of the application.

use crate::core;
use crate::core::layout;
use crate::core::overlay;
use crate::core::pointer::{self, mouse};
use crate::core::renderer;
use crate::core::widget::{self, Widget};
use crate::core::window::Id;
use crate::core::{Color, Element, Length, Point, Rectangle, Size, Vector};

/// The content of a logical window: a view drawn at a fixed position and
/// size inside the native window.
pub struct Window<'a, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    pub id: Id,
    pub position: Point,
    pub size: Size,
    pub background: Color,
    pub content: Element<'a, Message, Theme, Renderer>,
}

impl<'a, Message, Theme, Renderer> Window<'a, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    pub fn bounds(&self) -> Rectangle {
        Rectangle::new(self.position, self.size)
    }
}

/// Lays out its [`Window`] children at their absolute positions, from
/// bottom to top.
///
/// This is the root widget of the Android runtime: every logical window of
/// the application is a child of this container, and the whole tree is a
/// single [`UserInterface`](crate::runtime::user_interface::UserInterface).
pub struct Windows<'a, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    focus: Option<Id>,
    windows: Vec<Window<'a, Message, Theme, Renderer>>,
}

impl<'a, Message, Theme, Renderer> Windows<'a, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    /// Creates an empty [`Windows`] container.
    pub fn new() -> Self {
        Self {
            focus: None,
            windows: Vec::new(),
        }
    }

    pub fn focus(mut self, id: Option<Id>) -> Self {
        self.focus = id;
        self
    }

    /// Adds a window on top of the existing ones.
    pub fn push(mut self, window: Window<'a, Message, Theme, Renderer>) -> Self {
        self.windows.push(window);

        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Windows<'_, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    fn size(&self) -> Size<core::Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children_custom(
            &mut self.windows,
            |tree, window| tree.diff(&mut window.content),
            |window| widget::Tree::new(&window.content),
        );
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let bounds = limits.max();

        let children = self
            .windows
            .iter_mut()
            .zip(&mut tree.children)
            .map(|(window, state)| {
                let limits = layout::Limits::new(window.size, window.size);

                window
                    .content
                    .as_widget_mut()
                    .layout(state, renderer, &limits)
                    .move_to(window.position)
            })
            .collect::<Vec<_>>();

        layout::Node::with_children(bounds, children)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &core::Event,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut core::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let pointer_event = match event {
            core::Event::Pointer(pointer_event) => pointer_event,
            _ => {
                // Keyboard, window, and input-method events go to the
                // focused logical window. Until the first press—or after
                // the focused window closes—the root window (the
                // bottom-most one) is focused, like the main window of a
                // desktop application.
                let focused = self
                    .focus
                    .filter(|focus| {
                        self.windows.iter().any(|window| window.id == *focus)
                    })
                    .or_else(|| self.windows.first().map(|window| window.id));

                let Some(index) = focused.and_then(|focused| {
                    self.windows
                        .iter()
                        .position(|window| window.id == focused)
                }) else {
                    return;
                };

                if let Some(((window, state), layout)) = self
                    .windows
                    .get_mut(index)
                    .zip(tree.children.get_mut(index))
                    .zip(layout.children().nth(index))
                {
                    window.content.as_widget_mut().update(
                        state, event, layout, cursor, renderer, shell, viewport,
                    );
                }

                return;
            }
        };

        let position = match pointer_event {
            pointer::Event::PointerEntered { position, .. }
            | pointer::Event::PointerMoved { position, .. }
            | pointer::Event::PointerPressed { position, .. }
            | pointer::Event::PointerReleased { position, .. } => Some(*position),
            pointer::Event::PointerLeft { .. }
            | pointer::Event::WheelScrolled { .. } => cursor.position(),
        };

        // Hover and scrolling follow the pointer: the topmost window
        // under it receives them, regardless of the keyboard focus.
        let topmost = position.and_then(|position| {
            self.windows
                .iter()
                .rev()
                .find(|window| window.bounds().contains(position))
                .map(|window| window.id)
        });

        match pointer_event {
            pointer::Event::PointerPressed { .. } => {
                let mut unfocus = widget::operation::focusable::unfocus();

                for ((window, state), layout) in self
                    .windows
                    .iter_mut()
                    .zip(&mut tree.children)
                    .zip(layout.children())
                {
                    if Some(window.id) != topmost {
                        window.content.as_widget_mut().operate(
                            state,
                            layout,
                            renderer,
                            &mut unfocus,
                        );
                    }
                }
            }
            pointer::Event::PointerReleased { .. } => {
                // Widgets pressed by an earlier press—possibly in another
                // window—clear their state when they see the release.
                // Widgets that were not pressed ignore it.
                for ((window, state), layout) in self
                    .windows
                    .iter_mut()
                    .zip(&mut tree.children)
                    .zip(layout.children())
                {
                    window.content.as_widget_mut().update(
                        state, event, layout, cursor, renderer, shell, viewport,
                    );
                }

                return;
            }
            _ => {}
        }

        let Some(index) = topmost
            .and_then(|topmost| {
                self.windows.iter().position(|window| window.id == topmost)
            })
        else {
            return;
        };

        if let Some(((window, state), layout)) = self
            .windows
            .get_mut(index)
            .zip(tree.children.get_mut(index))
            .zip(layout.children().nth(index))
        {
            window
                .content
                .as_widget_mut()
                .update(state, event, layout, cursor, renderer, shell, viewport);
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((window, state), layout) in self
            .windows
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .filter(|(_, layout)| layout.bounds().intersects(viewport))
        {
            renderer.with_layer(window.bounds(), |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: window.bounds(),
                        ..Default::default()
                    },
                    window.background,
                );

                window
                    .content
                    .as_widget()
                    .draw(state, renderer, theme, style, layout, cursor, viewport);
            });
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: layout::Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let children = self
            .windows
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
            .filter_map(|((window, state), layout)| {
                window.content.as_widget_mut().overlay(
                    state,
                    layout,
                    renderer,
                    viewport,
                    translation,
                )
            })
            .collect::<Vec<_>>();

        (!children.is_empty()).then(|| overlay::Group::with_children(children).overlay())
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: layout::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        for ((window, state), layout) in self
            .windows
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            window
                .content
                .as_widget_mut()
                .operate(state, layout, renderer, operation);
        }
    }
}

impl<'a, Message, Theme, Renderer> From<Windows<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    fn from(windows: Windows<'a, Message, Theme, Renderer>) -> Self {
        Element::new(windows)
    }
}
