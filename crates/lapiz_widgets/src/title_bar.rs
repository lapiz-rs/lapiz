use iced_core::{Border, Element, Length, Theme};
use iced_wgpu::Renderer;
use iced_widget::container;

use crate::button::{self, Button};
use crate::callback::Callback;
use crate::flex::{self, Flex};
use crate::icon;

pub type Style = container::Style;

pub struct TitleBar<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    minimize: Callback<'a, Message>,
    maximize: Callback<'a, Message>,
    close: Callback<'a, Message>,
    class: <Theme as flex::Catalog>::Class<'a>,
}

impl<'a, Message> TitleBar<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            content: content.into(),
            minimize: Callback::Empty,
            maximize: Callback::Empty,
            close: Callback::Empty,
            class: Box::new(default),
        }
    }

    crate::callback_methods!(minimize);
    crate::callback_methods!(maximize);
    crate::callback_methods!(close);

    pub fn style(mut self, style: impl Fn(&Theme, flex::Status) -> Style + 'a) -> Self {
        self.class = Box::new(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as flex::Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }
}

impl<'a, Message: 'a> From<TitleBar<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: TitleBar<'a, Message>) -> Self {
        let mut controls = Flex::row(Vec::new()).height(Length::Fill);
        if value.minimize.is_set() {
            controls = controls.push(
                Button::new(icon::win_minimize().size(12))
                    .width(38)
                    .height(Length::Fill)
                    .padding([10, 13])
                    .transparent()
                    .on_press_with_callback(value.minimize),
            );
        }
        if value.maximize.is_set() {
            controls = controls.push(
                Button::new(icon::win_maximize().size(12))
                    .width(38)
                    .height(Length::Fill)
                    .padding([10, 13])
                    .transparent()
                    .on_press_with_callback(value.maximize),
            );
        }
        if value.close.is_set() {
            controls = controls.push(
                Button::new(icon::win_close().size(12))
                    .width(40)
                    .height(Length::Fill)
                    .padding([10, 14])
                    .style(close_button)
                    .on_press_with_callback(value.close),
            );
        }
        Flex::row([value.content, controls.into()])
            .width(Length::Fill)
            .height(32)
            .class(value.class)
            .into()
    }
}

fn close_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(p.danger.base.color.into()),
            text_color: p.danger.base.text,
            ..Default::default()
        },
        button::Status::Active | button::Status::Disabled => button::transparent(theme, status),
    }
}

pub fn default(theme: &Theme, _status: flex::Status) -> Style {
    let p = theme.extended_palette();
    Style::default()
        .background(p.background.base.color)
        .color(p.background.base.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        })
}
