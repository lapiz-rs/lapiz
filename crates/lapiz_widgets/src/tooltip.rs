use std::time::Duration;

use iced_core::{Border, Color, Element, Pixels, Shadow, Theme, Vector};
use iced_wgpu::Renderer;
use iced_widget::container;

pub use iced_widget::container::{Catalog, Style, StyleFn};
pub use iced_widget::tooltip::Position;

pub struct Tooltip<'a, Message> {
    inner: iced_widget::Tooltip<'a, Message, Theme, Renderer>,
}

impl<'a, Message> Tooltip<'a, Message> {
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        tooltip: impl Into<Element<'a, Message, Theme, Renderer>>,
        position: Position,
    ) -> Self {
        Self {
            inner: iced_widget::Tooltip::new(content, tooltip, position)
                .gap(7)
                .padding(5)
                .style(default),
        }
    }

    pub fn gap(mut self, gap: impl Into<Pixels>) -> Self {
        self.inner = self.inner.gap(gap);
        self
    }

    pub fn padding(mut self, padding: impl Into<Pixels>) -> Self {
        self.inner = self.inner.padding(padding);
        self
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.inner = self.inner.delay(delay);
        self
    }

    pub fn snap_within_viewport(mut self, snap: bool) -> Self {
        self.inner = self.inner.snap_within_viewport(snap);
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
}

impl<'a, Message: 'a> From<Tooltip<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Tooltip<'a, Message>) -> Self {
        value.inner.into()
    }
}

pub fn default(theme: &Theme) -> Style {
    let p = theme.extended_palette();
    container::Style::default()
        .background(p.background.weak.color)
        .color(p.background.weak.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.primary.base.color,
        })
        .shadow(Shadow {
            color: Color::BLACK.scale_alpha(0.22),
            offset: Vector::new(3.0, 3.0),
            blur_radius: 0.0,
        })
}
