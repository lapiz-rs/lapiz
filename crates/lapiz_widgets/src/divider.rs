use iced_core::{Element, Pixels, Theme};
use iced_widget::rule;
use lapiz_runtime::Renderer;

pub use iced_widget::rule::{Catalog, FillMode, Style, StyleFn};

pub struct Divider<'a> {
    inner: rule::Rule<'a, Theme>,
}

impl<'a> Divider<'a> {
    pub fn horizontal(thickness: impl Into<Pixels>) -> Self {
        Self {
            inner: rule::horizontal(thickness).style(default),
        }
    }

    pub fn vertical(thickness: impl Into<Pixels>) -> Self {
        Self {
            inner: rule::vertical(thickness).style(default),
        }
    }

    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.inner = self.inner.class(class);
        self
    }

    pub fn strong(self) -> Self {
        self.style(strong)
    }
}

impl<'a, Message: 'a> From<Divider<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Divider<'a>) -> Self {
        value.inner.into()
    }
}

pub fn default(theme: &Theme) -> Style {
    Style {
        color: theme.palette().background.strong.color,
        radius: 0.0.into(),
        fill_mode: FillMode::Full,
        snap: true,
    }
}

pub fn strong(theme: &Theme) -> Style {
    Style {
        color: theme.palette().background.stronger.color,
        ..default(theme)
    }
}
