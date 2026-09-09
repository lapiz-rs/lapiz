use iced_core::{Border, Element, Length, Padding, Pixels, Theme, alignment, text};
use iced_widget::{Container, Text, container};
use lapiz_runtime::Renderer;

pub use iced_widget::container::{Catalog, Style, StyleFn};

pub struct Kbd<'a> {
    content: Text<'a, Theme, Renderer>,
    width: Length,
    height: Length,
    padding: Padding,
    class: <Theme as Catalog>::Class<'a>,
}

impl<'a> Kbd<'a> {
    pub fn new(content: impl text::IntoFragment<'a>) -> Self {
        Self {
            content: Text::new(content).size(10).wrapping(text::Wrapping::None),
            width: Length::Shrink,
            height: Length::Fixed(18.0),
            padding: Padding::from([1, 4]),
            class: Box::new(default),
        }
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.content = self.content.size(size);
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

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self {
        self.class = Box::new(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    pub fn accent(self) -> Self {
        self.style(accent)
    }
}

impl<'a, Message: 'a> From<Kbd<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Kbd<'a>) -> Self {
        Container::new(value.content)
            .width(value.width)
            .height(value.height)
            .padding(value.padding)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
            .class(value.class)
            .into()
    }
}

pub fn default(theme: &Theme) -> Style {
    let p = theme.palette();
    container::Style::default()
        .background(p.background.weakest.color)
        .color(p.background.weak.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        })
}

pub fn accent(theme: &Theme) -> Style {
    let p = theme.palette();
    container::Style::default()
        .background(p.primary.weak.color)
        .color(p.primary.strong.color)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.primary.base.color,
        })
}
