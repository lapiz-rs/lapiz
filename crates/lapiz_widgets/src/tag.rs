use iced_core::{Border, Color, Element, Padding, Theme, text};
pub use iced_widget::container::{Catalog, Style, StyleFn};
use iced_widget::{Container, Text, container};
use lapiz_runtime::Renderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Primary,
    Accent,
    Success,
    Warning,
    Danger,
}

pub struct Tag<'a, Message> {
    inner: Container<'a, Message, Theme, Renderer>,
}

impl<'a, Message> Tag<'a, Message> {
    pub fn new(content: impl text::IntoFragment<'a>) -> Self {
        Self {
            inner: Container::new(Text::new(content).size(10))
                .padding(Padding::from([1, 6]))
                .style(neutral),
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

    pub fn tone(self, tone: Tone) -> Self {
        match tone {
            Tone::Neutral => self.style(neutral),
            Tone::Primary => self.style(primary),
            Tone::Accent => self.style(accent),
            Tone::Success => self.style(success),
            Tone::Warning => self.style(warning),
            Tone::Danger => self.style(danger),
        }
    }
}

impl<'a, Message: 'a> From<Tag<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Tag<'a, Message>) -> Self {
        value.inner.into()
    }
}

pub fn neutral(theme: &Theme) -> Style {
    styled(
        theme,
        theme.palette().background.weakest.color,
        theme.palette().background.weak.text,
    )
}

pub fn primary(theme: &Theme) -> Style {
    styled(
        theme,
        theme.palette().primary.weak.color,
        theme.palette().primary.strong.color,
    )
}

pub fn accent(theme: &Theme) -> Style {
    styled(
        theme,
        theme.palette().secondary.weak.color,
        theme.palette().secondary.strong.color,
    )
}

pub fn success(theme: &Theme) -> Style {
    styled(
        theme,
        theme.palette().success.weak.color,
        theme.palette().success.base.color,
    )
}

pub fn warning(theme: &Theme) -> Style {
    styled(
        theme,
        theme.palette().warning.weak.color,
        theme.palette().warning.base.color,
    )
}

pub fn danger(theme: &Theme) -> Style {
    styled(
        theme,
        theme.palette().danger.weak.color,
        theme.palette().danger.base.color,
    )
}

fn styled(theme: &Theme, background: Color, color: Color) -> Style {
    container::Style::default()
        .background(background)
        .color(color)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: theme.palette().background.strong.color,
        })
}
