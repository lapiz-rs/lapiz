use iced_core::{Background, Color, Element, Length, Pixels, Theme, text};
pub use iced_widget::toggler::{Catalog, Status, Style, StyleFn};
use lapiz_runtime::Renderer;

pub struct Switch<'a, Message> {
    inner: iced_widget::Toggler<'a, Message, Theme, Renderer>,
}

impl<'a, Message> Switch<'a, Message> {
    pub fn new(checked: bool, on_toggle: impl Fn(bool) -> Message + 'a) -> Self {
        Self {
            inner: iced_widget::Toggler::new(checked)
                .on_toggle(on_toggle)
                .size(16)
                .spacing(8)
                .text_size(12)
                .style(default),
        }
    }

    pub fn label(mut self, label: impl text::IntoFragment<'a>) -> Self {
        self.inner = self.inner.label(label);
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.inner = self.inner.size(size);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    pub fn spacing(mut self, spacing: impl Into<Pixels>) -> Self {
        self.inner = self.inner.spacing(spacing);
        self
    }

    pub fn text_size(mut self, size: impl Into<Pixels>) -> Self {
        self.inner = self.inner.text_size(size);
        self
    }

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.inner = self.inner.class(class);
        self
    }

    pub fn compact(self) -> Self {
        self.size(16).spacing(8).text_size(12)
    }
}

impl<'a, Message: 'a> From<Switch<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Switch<'a, Message>) -> Self {
        value.inner.into()
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let (checked, hovered, disabled) = match status {
        Status::Active { is_toggled } => (is_toggled, false, false),
        Status::Hovered { is_toggled } => (is_toggled, true, false),
        Status::Disabled { is_toggled } => (is_toggled, false, true),
    };
    let mut background = if checked {
        p.primary.weak.color
    } else {
        p.background.strongest.color
    };
    let mut foreground = if checked {
        p.primary.base.color
    } else {
        p.background.stronger.color
    };
    if disabled {
        background.a *= 0.4;
        foreground.a *= 0.4;
    }
    Style {
        background: Background::Color(background),
        background_border_width: 1.0,
        background_border_color: if hovered || checked {
            p.primary.base.color
        } else {
            p.background.strong.color
        },
        foreground: Background::Color(foreground),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: Some(p.background.base.text),
        border_radius: Some(0.0.into()),
        padding_ratio: 0.15,
    }
}
