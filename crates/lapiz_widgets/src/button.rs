use iced_core::Renderer as _;
use iced_core::{
    Alignment, Background, Border, Color, Element, Event, Layout, Length, Padding, Rectangle,
    Shadow, Shell, Size, Theme, Vector, Widget, layout, overlay, pointer,
    pointer::mouse,
    renderer,
    widget::{Operation, Tree, tree},
};
use lapiz_runtime::Renderer;

use crate::callback::{Callback, publish};

pub use iced_widget::button::{Catalog, Status, Style, StyleFn};

pub struct Button<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    press: Callback<'a, Message>,
    width: Length,
    height: Length,
    padding: Padding,
    clip: bool,
    interactive: bool,
    class: <Theme as Catalog>::Class<'a>,
}

impl<'a, Message> Button<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            content: content.into(),
            press: Callback::Empty,
            width: Length::Fit,
            height: Length::Fixed(26.0),
            padding: Padding {
                top: 5.0,
                right: 10.0,
                bottom: 5.0,
                left: 10.0,
            },
            clip: false,
            interactive: false,
            class: Box::new(default),
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }

    pub fn interactive(mut self) -> Self {
        self.interactive = true;
        self
    }

    crate::callback_methods!(press);

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.class = Box::new(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    pub fn primary(self) -> Self {
        self.style(primary)
    }

    pub fn transparent(self) -> Self {
        self.style(transparent)
    }

    pub fn outline(self) -> Self {
        self.style(outline)
    }

    pub fn danger(self) -> Self {
        self.style(danger)
    }

    pub fn activated(self, activated: bool) -> Self {
        if activated {
            self.style(activated_style)
        } else {
            self
        }
    }
}

#[derive(Debug, Default)]
struct State {
    pressed: bool,
    status: Option<Status>,
}

impl<Message> Widget<Message, Theme, Renderer> for Button<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
        self.width = self.width.stack(self.content.as_widget().size().width);
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
        layout::positioned(
            limits,
            self.width,
            self.height,
            self.padding,
            |limits| {
                self.content
                    .as_widget_mut()
                    .layout(&mut tree.children[0], renderer, limits)
            },
            |content, size| content.align(Alignment::Center, Alignment::Center, size),
        )
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
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                layout.children().next().unwrap(),
                renderer,
                operation,
            );
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            shell,
            viewport,
        );
        if !shell.is_event_captured() {
            let state = tree.state.downcast_mut::<State>();
            match event {
                Event::Pointer(event)
                    if event.is_primary_press()
                        && self.press.is_set()
                        && cursor.is_over(layout.bounds()) =>
                {
                    state.pressed = true;
                    shell.capture_event();
                }
                Event::Pointer(e @ pointer::Event::PointerReleased { .. })
                    if e.is_primary_release() && state.pressed =>
                {
                    state.pressed = false;
                    if cursor.is_over(layout.bounds())
                        && let Some(message) = publish(&mut self.press)
                    {
                        shell.publish(message);
                    }
                    shell.capture_event();
                }
                Event::Pointer(pointer::Event::PointerLeft {
                    kind: pointer::Kind::Touch(_),
                }) => {
                    state.pressed = false;
                }
                _ => {}
            }
        }
        let state = tree.state.downcast_mut::<State>();
        let status = if !self.press.is_set() && !self.interactive {
            Status::Disabled
        } else if cursor.is_over(layout.bounds()) {
            if state.pressed {
                Status::Pressed
            } else {
                Status::Hovered
            }
        } else {
            Status::Active
        };
        if state.status != Some(status) {
            state.status = Some(status);
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _renderer_style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let style = theme.style(&self.class, state.status.unwrap_or(Status::Disabled));
        let bounds = layout.bounds();
        if style.background.is_some() || style.border.width > 0.0 || style.shadow.color.a > 0.0 {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style.border,
                    shadow: style.shadow,
                    snap: style.snap,
                },
                style
                    .background
                    .unwrap_or(Background::Color(Color::TRANSPARENT)),
            );
        }
        let viewport = if self.clip {
            bounds.intersection(viewport).unwrap_or(*viewport)
        } else {
            *viewport
        };
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            layout.children().next().unwrap(),
            cursor,
            &viewport,
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
        if (self.press.is_set() || self.interactive) && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
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
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<Button<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Button<'a, Message>) -> Self {
        Element::new(value)
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let base = Style {
        background: Some(Background::Color(p.background.weakest.color)),
        text_color: p.background.base.text,
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        },
        ..Style::default()
    };
    match status {
        Status::Active => base,
        Status::Hovered => Style {
            text_color: p.primary.strong.color,
            border: Border {
                color: p.primary.base.color,
                ..base.border
            },
            ..base
        },
        Status::Pressed => Style {
            background: Some(Background::Color(p.primary.weak.color)),
            border: Border {
                color: p.primary.base.color,
                ..base.border
            },
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

pub fn primary(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let base = Style {
        background: Some(Background::Color(p.primary.base.color)),
        text_color: p.primary.base.text,
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.primary.base.color,
        },
        shadow: Shadow {
            color: with_alpha(p.primary.base.color, 0.22),
            offset: Vector::new(2.0, 2.0),
            blur_radius: 0.0,
        },
        ..Style::default()
    };
    match status {
        Status::Active => base,
        Status::Hovered => Style {
            background: Some(Background::Color(p.primary.strong.color)),
            border: Border {
                color: p.primary.strong.color,
                ..base.border
            },
            ..base
        },
        Status::Pressed => Style {
            background: Some(Background::Color(p.primary.strong.color)),
            shadow: Shadow::default(),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

pub fn transparent(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let base = Style {
        text_color: p.background.weak.text,
        border: Border::default(),
        ..Style::default()
    };
    match status {
        Status::Active => base,
        Status::Hovered => Style {
            background: Some(Background::Color(with_alpha(p.primary.base.color, 0.1))),
            text_color: p.primary.strong.color,
            ..base
        },
        Status::Pressed => Style {
            background: Some(Background::Color(with_alpha(p.primary.base.color, 0.16))),
            text_color: p.primary.strong.color,
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

pub fn outline(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let base = Style {
        text_color: p.background.base.text,
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.stronger.color,
        },
        ..Style::default()
    };
    match status {
        Status::Active => base,
        Status::Hovered | Status::Pressed => Style {
            background: Some(Background::Color(with_alpha(p.primary.base.color, 0.08))),
            text_color: p.primary.strong.color,
            border: Border {
                color: p.primary.base.color,
                ..base.border
            },
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

pub fn danger(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let base = Style {
        text_color: p.danger.base.color,
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.danger.base.color,
        },
        ..Style::default()
    };
    match status {
        Status::Active => base,
        Status::Hovered | Status::Pressed => Style {
            background: Some(Background::Color(p.danger.base.color)),
            text_color: p.primary.base.text,
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

pub fn activated_style(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let mut style = primary(theme, status);
    style.shadow = Shadow::default();
    style.background = Some(Background::Color(match status {
        Status::Hovered => p.primary.strong.color,
        Status::Pressed => p.primary.strong.color,
        _ => p.primary.base.color,
    }));
    style
}

fn disabled(mut style: Style) -> Style {
    style.background = style.background.map(|background| match background {
        Background::Color(mut color) => {
            color.a *= 0.4;
            Background::Color(color)
        }
        background => background,
    });
    style.text_color.a *= 0.4;
    style.border.color.a *= 0.4;
    style.shadow = Shadow::default();
    style
}

pub fn icon_button<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Button<'a, Message> {
    Button::new(content)
        .width(24)
        .height(24)
        .padding(5)
        .transparent()
}

pub fn toggle_button<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    activated: bool,
) -> Button<'a, Message> {
    Button::new(content)
        .height(24)
        .padding([0, 10])
        .transparent()
        .activated(activated)
}

fn with_alpha(mut color: Color, alpha: f32) -> Color {
    color.a = alpha;
    color
}
