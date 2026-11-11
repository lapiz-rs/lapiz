use iced_core::{Element, Length, Theme};
use lapiz_runtime::Renderer;

use crate::{button, callback::Callback, flex::Flex, icon, panel};

/// An expandable and collapsible container with a clickable header bar.
///
/// `Collapsible` renders a [`Panel`](crate::panel::Panel) containing a full-width header
/// [`Button`](crate::button::Button) prefixed with a directional chevron icon (`chevron_down`
/// when open, `chevron_right` when closed). When `open` is `true`, the child `content` element
/// is stacked vertically below the header; when `false`, only the header is shown.
///
/// Clicking the header triggers the toggle callback configured via [`on_toggle`](Self::on_toggle),
/// [`on_toggle_maybe`](Self::on_toggle_maybe), or [`on_toggle_with`](Self::on_toggle_with).
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
        let indicator = if value.open {
            Element::from(icon::chevron_down().size(13))
        } else {
            Element::from(icon::chevron_right().size(13))
        };
        let header = button(
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
        panel(body.width(Length::Fill)).width(value.width).into()
    }
}
