use iced_core::{Element, Length, Theme};
use iced_widget::scrollable;
use lapiz_runtime::Renderer;

use crate::{button::Button, callback::Callback, flex::Flex, icon, label::Label, panel::Panel};

struct Item<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    selected: bool,
    message: Option<Message>,
}

pub struct PickList<'a, Message> {
    available: Vec<Item<'a, Message>>,
    selected: Vec<Item<'a, Message>>,
    available_label: String,
    selected_label: String,
    move_to_selected: Callback<'a, Message>,
    move_to_available: Callback<'a, Message>,
    width: Length,
    height: Length,
}

impl<'a, Message> PickList<'a, Message> {
    pub fn new() -> Self {
        Self {
            available: Vec::new(),
            selected: Vec::new(),
            available_label: String::from("Available"),
            selected_label: String::from("Active"),
            move_to_selected: Callback::Empty,
            move_to_available: Callback::Empty,
            width: Length::Fill,
            height: Length::Fixed(150.0),
        }
    }

    pub fn labels(mut self, available: impl Into<String>, selected: impl Into<String>) -> Self {
        self.available_label = available.into();
        self.selected_label = selected.into();
        self
    }

    pub fn available_item(
        mut self,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        selected: bool,
        message: Message,
    ) -> Self {
        self.available.push(Item {
            content: content.into(),
            selected,
            message: Some(message),
        });
        self
    }

    pub fn selected_item(
        mut self,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        selected: bool,
        message: Message,
    ) -> Self {
        self.selected.push(Item {
            content: content.into(),
            selected,
            message: Some(message),
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

    crate::callback_methods!(move_to_selected);
    crate::callback_methods!(move_to_available);
}

impl<Message> Default for PickList<'_, Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, Message: 'a> From<PickList<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: PickList<'a, Message>) -> Self {
        let left = list_panel(value.available_label, value.available, item_button);
        let right = list_panel(value.selected_label, value.selected, item_button);
        let controls = Flex::column([
            Button::new(icon::chevron_right().size(13))
                .width(24)
                .height(24)
                .padding(5.5)
                .on_press_with_callback(value.move_to_selected)
                .into(),
            Button::new(icon::chevron_left().size(13))
                .width(24)
                .height(24)
                .padding(5.5)
                .on_press_with_callback(value.move_to_available)
                .into(),
        ])
        .gap(4);
        Flex::row([left, controls.into(), right])
            .width(value.width)
            .height(value.height)
            .gap(8)
            .into()
    }
}

fn list_panel<'a, Message: 'a>(
    label: String,
    items: Vec<Item<'a, Message>>,
    item_button: fn(Item<'a, Message>) -> Element<'a, Message, Theme, Renderer>,
) -> Element<'a, Message, Theme, Renderer> {
    let header = Flex::row([
        Label::new(label).muted().into(),
        Label::new(items.len().to_string()).faint().into(),
    ])
    .width(Length::Fill)
    .space_between()
    .padding([4, 8]);
    Panel::new(Flex::column([
        header.into(),
        scrollable(Flex::column(items.into_iter().map(item_button)).width(Length::Fill))
            .height(Length::Fill)
            .into(),
    ]))
    .inset()
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn item_button<'a, Message: 'a>(item: Item<'a, Message>) -> Element<'a, Message, Theme, Renderer> {
    Button::new(item.content)
        .width(Length::Fill)
        .height(24)
        .padding([0, 8])
        .transparent()
        .activated(item.selected)
        .on_press_maybe(item.message)
        .into()
}
