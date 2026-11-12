use iced_core::{Element, Length, Theme};
use lapiz_runtime::Renderer;

use crate::{button, callback::Callback, flex::Flex};

/// A single button item inside a [`SegmentedControl`].
struct Segment<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    selected: bool,
    message: Callback<'a, Message>,
}

/// A horizontal row of connected segment buttons for selecting between mutually exclusive modes or views.
///
/// `SegmentedControl` renders its segments inside a bordered surface [`Flex`] row,
/// highlighting active segments with [`Button::activated`](crate::button::Button::activated) styling.
pub struct SegmentedControl<'a, Message> {
    segments: Vec<Segment<'a, Message>>,
    width: Length,
    height: Length,
}

impl<'a, Message: 'a> SegmentedControl<'a, Message> {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
            width: Length::Fit,
            height: Length::Fixed(24.0),
        }
    }

    pub fn push(
        mut self,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        selected: bool,
        message: Message,
    ) -> Self {
        self.segments.push(Segment {
            content: content.into(),
            selected,
            message: Callback::Value(message),
        });
        self
    }

    pub fn push_with(
        mut self,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        selected: bool,
        message: impl Fn() -> Message + 'a,
    ) -> Self {
        self.segments.push(Segment {
            content: content.into(),
            selected,
            message: Callback::Func(Box::new(message)),
        });
        self
    }

    pub fn push_disabled(
        mut self,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        selected: bool,
    ) -> Self {
        self.segments.push(Segment {
            content: content.into(),
            selected,
            message: Callback::Empty,
        });
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
}

impl<'a, Message: 'a> Default for SegmentedControl<'a, Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, Message: 'a> From<SegmentedControl<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(value: SegmentedControl<'a, Message>) -> Self {
        let buttons = value.segments.into_iter().map(|segment| {
            let b = button(segment.content)
                .height(Length::Fill)
                .padding([0, 12])
                .transparent()
                .activated(segment.selected);

            match segment.message {
                Callback::Empty => b,
                Callback::Value(message) => b.on_press(message),
                Callback::Func(message) => b.on_press_with(message),
            }
            .into()
        });
        Flex::row(buttons)
            .width(value.width)
            .height(value.height)
            .surface()
            .into()
    }
}
