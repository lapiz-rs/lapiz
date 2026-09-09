use std::ops::RangeInclusive;

use iced_core::{Background, Border, Element, Length, Pixels, Theme};
pub use iced_widget::slider::{Catalog, Handle, HandleShape, Rail, Status, Style, StyleFn};
use lapiz_runtime::Renderer;
use num_traits::FromPrimitive;

pub struct Slider<'a, T, Message>
where
    T: Copy + From<u8> + PartialOrd,
    Message: Clone,
{
    inner: iced_widget::Slider<'a, T, Message, Theme>,
}

impl<'a, T, Message> Slider<'a, T, Message>
where
    T: Copy + From<u8> + PartialOrd,
    Message: Clone,
{
    pub fn new(range: RangeInclusive<T>, value: T, on_change: impl Fn(T) -> Message + 'a) -> Self {
        Self {
            inner: iced_widget::Slider::new(range, value, on_change).style(default),
        }
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    pub fn height(mut self, height: impl Into<Pixels>) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    pub fn step(mut self, step: impl num_traits::AsPrimitive<f64>) -> Self {
        self.inner = self.inner.step(step);
        self
    }

    pub fn shift_step(mut self, step: impl num_traits::AsPrimitive<f64>) -> Self {
        self.inner = self.inner.shift_step(step);
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
}

impl<'a, T, Message> From<Slider<'a, T, Message>> for Element<'a, Message, Theme, Renderer>
where
    T: Copy + From<u8> + PartialOrd + Into<f64> + FromPrimitive + num_traits::AsPrimitive<f64> + 'a,
    Message: Clone + 'a,
{
    fn from(value: Slider<'a, T, Message>) -> Self {
        value.inner.into()
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let p = theme.palette();
    let active = status == Status::Dragged;
    Style {
        rail: Rail {
            backgrounds: (
                Background::Color(p.primary.base.color),
                Background::Color(p.background.strongest.color),
            ),
            width: 6.0,
            border: Border {
                radius: 0.0.into(),
                width: 1.0,
                color: p.background.strong.color,
            },
        },
        handle: Handle {
            shape: HandleShape::Rectangle {
                width: 8,
                border_radius: 0.0.into(),
            },
            background: Background::Color(if active {
                p.primary.base.color
            } else {
                p.background.weak.color
            }),
            border_width: 1.0,
            border_color: if status == Status::Hovered || active {
                p.primary.base.color
            } else {
                p.background.stronger.color
            },
        },
    }
}
