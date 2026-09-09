use iced_core::{Element, Length, Theme};
use lapiz_runtime::Renderer;

use crate::{button::Button, callback::Callback, flex::Flex, icon, panel::Panel};

pub struct Collapsible<'a, Message> {
    header: Element<'a, Message, Theme, Renderer>,
    content: Element<'a, Message, Theme, Renderer>,
    open: bool,
    toggle: Callback<'a, Message>,
    width: Length,
}

impl<'a, Message> Collapsible<'a, Message> {
    pub fn new(
        header: impl Into<Element<'a, Message, Theme, Renderer>>,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        open: bool,
    ) -> Self {
        Self {
            header: header.into(),
            content: content.into(),
            open,
            toggle: Callback::Empty,
            width: Length::Fill,
        }
    }

    crate::callback_methods!(toggle);

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }
}

impl<'a, Message: 'a> From<Collapsible<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: Collapsible<'a, Message>) -> Self {
        let indicator: Element<'a, Message, Theme, Renderer> = if value.open {
            icon::chevron_down().size(13).into()
        } else {
            icon::chevron_right().size(13).into()
        };
        let header = Button::new(
            Flex::row([indicator, value.header])
                .width(Length::Fill)
                .gap(6),
        )
        .width(Length::Fill)
        .height(26)
        .padding([0, 8])
        .transparent()
        .on_press_with_callback(value.toggle);
        let body = if value.open {
            Flex::column([header.into(), value.content])
        } else {
            Flex::column([header.into()])
        };
        Panel::new(body.width(Length::Fill))
            .width(value.width)
            .into()
    }
}
