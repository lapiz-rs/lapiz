use iced_core::{
    Color, Element, Font, Length, Pixels, Theme, alignment, font, text,
    widget::text::{Ellipsis, Style, Wrapping},
};
use lapiz_runtime::Renderer;

/// A themed text label widget with typography and semantic color presets.
///
/// `Label` wraps `iced_widget::Text` with a default `12px` font size, word-or-glyph wrapping,
/// and vertical centering.
///
/// # Typography and Color Presets
///
/// - [`strong`](Self::strong): Renders text with semibold font weight.
/// - [`muted`](Self::muted): Uses the theme's weak background text color for secondary labels.
/// - [`faint`](Self::faint): Uses a translucent weak text color for de-emphasized hints or counts.
/// - [`accent`](Self::accent): Highlights text with the theme's strong primary color.
/// - [`window_title`](Self::window_title): Semibold `13px` preset suited for window and panel titles.
pub struct Label<'a> {
    inner: iced_widget::Text<'a, Theme, Renderer>,
}

impl<'a> Label<'a> {
    pub fn new(content: impl text::IntoFragment<'a>) -> Self {
        Self {
            inner: iced_widget::Text::new(content)
                .size(12)
                .wrapping(Wrapping::WordOrGlyph)
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

    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.inner = self.inner.wrapping(wrapping);
        self
    }

    pub fn ellipsis(mut self, ellipsis: Ellipsis) -> Self {
        self.inner = self.inner.ellipsis(ellipsis);
        self
    }

    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.inner = self.inner.color(color);
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

    pub fn window_title(self) -> Self {
        self.strong().size(13)
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
