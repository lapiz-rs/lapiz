pub use iced_core::widget::text::{Catalog, Style, StyleFn};
use iced_core::{Element, Font, Length, Pixels, Theme, alignment, font, text};
use lapiz_runtime::Renderer;

pub struct Label<'a> {
    inner: iced_widget::Text<'a, Theme, Renderer>,
}

impl<'a> Label<'a> {
    pub fn new(content: impl text::IntoFragment<'a>) -> Self {
        Self {
            inner: iced_widget::Text::new(content)
                .size(12)
                .wrapping(text::Wrapping::None)
                .align_y(alignment::Vertical::Center)
                .style(default),
        }
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.inner = self.inner.size(size);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    pub fn font(mut self, font: impl Into<Font>) -> Self {
        self.inner = self.inner.font(font);
        self
    }

    pub fn align_x(mut self, alignment: impl Into<text::Alignment>) -> Self {
        self.inner = self.inner.align_x(alignment);
        self
    }

    pub fn align_y(mut self, alignment: impl Into<alignment::Vertical>) -> Self {
        self.inner = self.inner.align_y(alignment);
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn strong(self) -> Self {
        self.font(Font {
            weight: font::Weight::Semibold,
            ..Font::DEFAULT
        })
    }

    pub fn muted(self) -> Self {
        self.style(muted)
    }

    pub fn faint(self) -> Self {
        self.style(faint)
    }

    pub fn accent(self) -> Self {
        self.style(accent)
    }
}

impl<'a, Message: 'a> From<Label<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Label<'a>) -> Self {
        value.inner.into()
    }
}

pub fn default(_theme: &Theme) -> Style {
    Style { color: None }
}

pub fn muted(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().background.weak.text),
    }
}

pub fn faint(theme: &Theme) -> Style {
    let mut color = theme.palette().background.weak.text;
    color.a *= 0.7;
    Style { color: Some(color) }
}

pub fn accent(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().primary.strong.color),
    }
}
