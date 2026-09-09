use iced_core::{Background, Border, Element, Length, Pixels, Theme, text};
use lapiz_runtime::Renderer;

pub use iced_widget::checkbox::{Catalog, Icon, Status, Style, StyleFn};

pub struct Checkbox<'a, Message> {
    checked: bool,
    toggle: Option<Box<dyn Fn(bool) -> Message + 'a>>,
    label: Option<text::Fragment<'a>>,
    size: f32,
    width: Length,
    spacing: f32,
    text_size: Option<Pixels>,
    class: <Theme as Catalog>::Class<'a>,
}

impl<'a, Message> Checkbox<'a, Message> {
    pub fn new(checked: bool) -> Self {
        Self {
            checked,
            toggle: None,
            label: None,
            size: 15.0,
            width: Length::Shrink,
            spacing: 8.0,
            text_size: Some(Pixels(12.0)),
            class: Box::new(default),
        }
    }

    pub fn label(mut self, label: impl text::IntoFragment<'a>) -> Self {
        self.label = Some(label.into_fragment());
        self
    }

    pub fn on_toggle(mut self, on_toggle: impl Fn(bool) -> Message + 'a) -> Self {
        self.toggle = Some(Box::new(on_toggle));
        self
    }

    pub fn on_toggle_maybe<F>(mut self, on_toggle: Option<F>) -> Self
    where
        F: Fn(bool) -> Message + 'a,
    {
        self.toggle = on_toggle.map(|on_toggle| Box::new(on_toggle) as _);
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = size.into().0;
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn spacing(mut self, spacing: impl Into<Pixels>) -> Self {
        self.spacing = spacing.into().0;
        self
    }

    pub fn text_size(mut self, size: impl Into<Pixels>) -> Self {
        self.text_size = Some(size.into());
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

impl<'a, Message: 'a> From<Checkbox<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Checkbox<'a, Message>) -> Self {
        let mut inner = iced_widget::Checkbox::new(value.checked)
            .size(value.size)
            .width(value.width)
            .spacing(value.spacing)
            .class(value.class);
        inner = inner.icon(Icon {
            font: <Renderer as iced_core::text::Renderer>::ICON_FONT,
            code_point: <Renderer as iced_core::text::Renderer>::CHECKMARK_ICON,
            size: Some(Pixels(value.size * 0.62)),
            line_height: text::LineHeight::Relative(1.0),
            shaping: text::Shaping::Basic,
        });
        if let Some(size) = value.text_size {
            inner = inner.text_size(size);
        }
        if let Some(label) = value.label {
            inner = inner.label(label);
        }
        if let Some(on_toggle) = value.toggle {
            inner = inner.on_toggle(on_toggle);
        }
        inner.into()
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let (checked, hovered, disabled) = match status {
        Status::Active { is_checked } => (is_checked, false, false),
        Status::Hovered { is_checked } => (is_checked, true, false),
        Status::Disabled { is_checked } => (is_checked, false, true),
    };
    let mut style = Style {
        background: Background::Color(if checked {
            p.primary.base.color
        } else {
            p.background.strongest.color
        }),
        icon_color: p.primary.base.text,
        border: Border {
            radius: 0.0.into(),
            width: 1.0,
            color: if checked || hovered {
                p.primary.base.color
            } else {
                p.background.strong.color
            },
        },
        text_color: Some(p.background.base.text),
    };
    if disabled {
        if let Background::Color(ref mut color) = style.background {
            color.a *= 0.4;
        }
        style.border.color.a *= 0.4;
        style.text_color.as_mut().unwrap().a *= 0.4;
    }
    style
}
