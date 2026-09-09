use iced_core::{Border, Element, Length, Theme};
use lapiz_runtime::Renderer;

use crate::flex::{Flex, Status, Style};

pub struct StatusBar<'a, Message> {
    inner: Flex<'a, Message>,
}

impl<'a, Message> StatusBar<'a, Message> {
    pub fn new(children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            inner: Flex::row(children)
                .width(Length::Fill)
                .height(22)
                .padding([0, 4])
                .gap(1)
                .style(status_bar),
        }
    }

    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self {
        self.inner = self.inner.style(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as crate::flex::Catalog>::Class<'a>>) -> Self {
        self.inner = self.inner.class(class);
        self
    }
}

impl<'a, Message: 'a> From<StatusBar<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: StatusBar<'a, Message>) -> Self {
        value.inner.into()
    }
}

pub fn status_bar(theme: &Theme, _status: Status) -> Style {
    let p = theme.palette();
    Style::default()
        .background(p.background.base.color)
        .color(p.background.weak.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        })
}
