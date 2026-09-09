use iced_core::{Background, Color, Element, Length, Pixels, Theme};
use iced_widget::svg;
use lapiz_runtime::Renderer;

use crate::button::{self, Button};
use crate::{flex::Flex, icon::Icon, label::Label};

pub struct Radio<'a, Message> {
    label: String,
    selected: bool,
    message: Option<Message>,
    width: Length,
    size: Pixels,
    spacing: Pixels,
    text_size: Pixels,
    class: <Theme as Catalog>::Class<'a>,
}

impl<'a, Message> Radio<'a, Message> {
    pub fn new<V: Eq + Copy>(
        label: impl Into<String>,
        value: V,
        selected: Option<V>,
        on_click: impl FnOnce(V) -> Message,
    ) -> Self {
        Self {
            label: label.into(),
            selected: selected == Some(value),
            message: Some(on_click(value)),
            width: Length::Shrink,
            size: Pixels(15.0),
            spacing: Pixels(8.0),
            text_size: Pixels(12.0),
            class: <Theme as Catalog>::default(),
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = size.into();
        self
    }

    pub fn spacing(mut self, spacing: impl Into<Pixels>) -> Self {
        self.spacing = spacing.into();
        self
    }

    pub fn text_size(mut self, size: impl Into<Pixels>) -> Self {
        self.text_size = size.into();
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.class = Box::new(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }
}

impl<'a, Message: 'a> From<Radio<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Radio<'a, Message>) -> Self {
        let selected = value.selected;
        let class = value.class;
        let handle = if selected {
            svg::Handle::from_memory(include_bytes!("../assets/icons/radio-on.svg").as_slice())
        } else {
            svg::Handle::from_memory(include_bytes!("../assets/icons/radio-off.svg").as_slice())
        };
        let indicator = Icon::new(handle)
            .size(value.size.0)
            .style(move |theme, status| {
                let status = if status == svg::Status::Hovered {
                    Status::Hovered {
                        is_selected: selected,
                    }
                } else {
                    Status::Active {
                        is_selected: selected,
                    }
                };
                let style = theme.style(&class, status);
                svg::Style {
                    color: Some(if selected {
                        style.dot_color
                    } else {
                        style.border_color
                    }),
                }
            });
        Button::new(
            Flex::row([
                indicator.into(),
                Label::new(value.label).size(value.text_size).into(),
            ])
            .gap(value.spacing),
        )
        .width(value.width)
        .padding(0)
        .style(button_style)
        .on_press_maybe(value.message)
        .into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active { is_selected: bool },
    Hovered { is_selected: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub background: Background,
    pub dot_color: Color,
    pub border_width: f32,
    pub border_color: Color,
    pub text_color: Option<Color>,
}

pub trait Catalog: Sized {
    type Class<'a>;

    fn default<'a>() -> Self::Class<'a>;

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let (selected, hovered) = match status {
        Status::Active { is_selected } => (is_selected, false),
        Status::Hovered { is_selected } => (is_selected, true),
    };
    Style {
        background: Background::Color(if selected {
            p.primary.weak.color
        } else {
            p.background.strongest.color
        }),
        dot_color: p.primary.base.color,
        border_width: 1.0,
        border_color: if selected || hovered {
            p.primary.base.color
        } else {
            p.background.strong.color
        },
        text_color: Some(p.background.base.text),
    }
}

fn button_style(theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        text_color: theme.palette().background.base.text,
        ..button::Style::default()
    }
}
