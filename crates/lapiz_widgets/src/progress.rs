use std::ops::RangeInclusive;

use iced_core::{Background, Border, Element, Length, Theme};
pub use iced_widget::progress_bar::{Catalog, Style, StyleFn};
use lapiz_runtime::Renderer;

pub struct ProgressBar<'a> {
    inner: iced_widget::ProgressBar<'a, Theme>,
}

impl<'a> ProgressBar<'a> {
    pub fn new(range: RangeInclusive<f32>, value: f32) -> Self {
        Self {
            inner: iced_widget::ProgressBar::new(range, value).style(default),
        }
    }

    pub fn length(mut self, length: impl Into<Length>) -> Self {
        self.inner = self.inner.length(length);
        self
    }

    pub fn girth(mut self, girth: impl Into<Length>) -> Self {
        self.inner = self.inner.girth(girth);
        self
    }

    pub fn vertical(mut self) -> Self {
        self.inner = self.inner.vertical(true);
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.inner = self.inner.class(class);
        self
    }

    pub fn success(self) -> Self {
        self.style(success)
    }

    pub fn warning(self) -> Self {
        self.style(warning)
    }

    pub fn danger(self) -> Self {
        self.style(danger)
    }
}

impl<'a, Message: 'a> From<ProgressBar<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: ProgressBar<'a>) -> Self {
        value.inner.into()
    }
}

pub fn default(theme: &Theme) -> Style {
    styled(theme, theme.palette().primary.base.color)
}

pub fn success(theme: &Theme) -> Style {
    styled(theme, theme.palette().success.base.color)
}

pub fn warning(theme: &Theme) -> Style {
    styled(theme, theme.palette().warning.base.color)
}

pub fn danger(theme: &Theme) -> Style {
    styled(theme, theme.palette().danger.base.color)
}

fn styled(theme: &Theme, bar: iced_core::Color) -> Style {
    let p = theme.palette();
    Style {
        background: Background::Color(p.background.strongest.color),
        bar: Background::Color(bar),
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        },
    }
}
