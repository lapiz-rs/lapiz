use iced_core::{
    Border, Element, Length, Padding, Rectangle, Size, Theme, Widget, layout, pointer::mouse,
    renderer, widget,
};
use iced_widget::{button, container};
use lapiz_runtime::Renderer;

use crate::{
    button::{Button, transparent},
    callback::{Callback, publish},
    flex::{self, Flex},
    icon,
};

pub type Style = container::Style;

pub struct TitleBar<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    content_padding: Padding,
    minimize: Callback<'a, Message>,
    maximize: Callback<'a, Message>,
    drag: Callback<'a, Message>,
    close: Callback<'a, Message>,
    class: <Theme as flex::Catalog>::Class<'a>,
}

impl<'a, Message> TitleBar<'a, Message> {
    pub fn new(content: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            content: content.into(),
            content_padding: Padding::from([0, 10]),
            minimize: Callback::Empty,
            maximize: Callback::Empty,
            drag: Callback::Empty,
            close: Callback::Empty,
            class: Box::new(default),
        }
    }

    crate::callback_methods!(minimize);
    crate::callback_methods!(maximize);
    crate::callback_methods!(drag);
    crate::callback_methods!(close);

    pub fn style(mut self, style: impl Fn(&Theme, flex::Status) -> Style + 'a) -> Self {
        self.class = Box::new(style);
        self
    }

    pub fn class(mut self, class: impl Into<<Theme as flex::Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    pub fn content_padding(mut self, padding: impl Into<Padding>) -> Self {
        self.content_padding = padding.into();
        self
    }
}

impl<'a, Message: 'a> From<TitleBar<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: TitleBar<'a, Message>) -> Self {
        let mut controls = Flex::row(Vec::new()).height(Length::Fill);
        #[cfg(not(target_os = "android"))]
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
        let row = if value.drag.is_set() {
            Flex::row([
                Flex::row([value.content])
                    .padding(value.content_padding)
                    .into(),
                WindowCaptionRegion::new()
                    .on_drag_with_callback(value.drag)
                    .into(),
                controls.into(),
            ])
        } else {
            Flex::row([value.content, controls.into()])
        };

        row.width(Length::Fill).height(32).class(value.class).into()
    }
}

fn close_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(p.danger.base.color.into()),
            text_color: p.danger.base.text,
            ..Default::default()
        },
        button::Status::Active | button::Status::Disabled => transparent(theme, status),
    }
}

pub fn default(theme: &Theme, _status: flex::Status) -> Style {
    let p = theme.palette();
    Style::default()
        .background(p.background.base.color)
        .color(p.background.base.text)
        .border(Border {
            radius: 0.0.into(),
            width: 1.0,
            color: p.background.strong.color,
        })
}

#[derive(Default)]
pub struct WindowCaptionRegion<'a, Message> {
    drag: Callback<'a, Message>,
}

impl<'a, Message> WindowCaptionRegion<'a, Message> {
    pub fn new() -> Self {
        Self {
            drag: Callback::Empty,
        }
    }

    crate::callback_methods!(drag);
}

impl<'a, Message> Widget<Message, Theme, Renderer> for WindowCaptionRegion<'a, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn update(
        &mut self,
        _tree: &mut widget::Tree,
        event: &iced_core::Event,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        shell: &mut iced_core::Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        match event {
            iced_core::Event::Pointer(event) if event.is_primary_press() => {
                if cursor.is_over(layout.bounds())
                    && let Some(message) = publish(&mut self.drag)
                {
                    shell.publish(message);
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        _renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: layout::Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }
}

impl<'a, Message> From<WindowCaptionRegion<'a, Message>> for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
{
    fn from(value: WindowCaptionRegion<'a, Message>) -> Element<'a, Message, Theme, Renderer> {
        Element::new(value)
    }
}
