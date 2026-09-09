use iced_core::{Border, Element, Length, Shadow, Theme, Vector};
use lapiz_runtime::Renderer;

use crate::button::{self, Button, activated_style, transparent};
use crate::flex::{self, Flex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    #[default]
    Line,
    Block,
}

struct Tab<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    selected: bool,
    message: Option<Message>,
}

pub struct TabBar<'a, Message> {
    tabs: Vec<Tab<'a, Message>>,
    variant: Variant,
    width: Length,
    height: Length,
}

impl<'a, Message> TabBar<'a, Message> {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            variant: Variant::Line,
            width: Length::Fit,
            height: Length::Fixed(26.0),
        }
    }

    pub fn push(
        mut self,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        selected: bool,
        message: Message,
    ) -> Self {
        self.tabs.push(Tab {
            content: content.into(),
            selected,
            message: Some(message),
        });
        self
    }

    pub fn push_disabled(
        mut self,
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        selected: bool,
    ) -> Self {
        self.tabs.push(Tab {
            content: content.into(),
            selected,
            message: None,
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

    pub fn line(mut self) -> Self {
        self.variant = Variant::Line;
        self
    }

    pub fn block(mut self) -> Self {
        self.variant = Variant::Block;
        self
    }
}

impl<Message> Default for TabBar<'_, Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, Message: 'a> From<TabBar<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: TabBar<'a, Message>) -> Self {
        let variant = value.variant;
        let tabs = value.tabs.into_iter().map(move |tab| {
            let selected = tab.selected;
            Button::new(tab.content)
                .height(Length::Fill)
                .padding([0, 12])
                .style(move |theme, status| style(theme, status, variant, selected))
                .on_press_maybe(tab.message)
                .into()
        });
        Flex::row(tabs)
            .width(value.width)
            .height(value.height)
            .style(tab_bar)
            .into()
    }
}

fn style(theme: &Theme, status: button::Status, variant: Variant, selected: bool) -> button::Style {
    if variant == Variant::Block && selected {
        return activated_style(theme, status);
    }
    let p = theme.palette();
    let mut style = transparent(theme, status);
    if selected {
        style.text_color = p.background.base.text;
        if variant == Variant::Line {
            style.border.width = 0.0;
            style.shadow = Shadow {
                color: p.primary.base.color,
                offset: Vector::new(0.0, 2.0),
                blur_radius: 0.0,
            };
        }
    }
    style
}

fn tab_bar(theme: &Theme, _status: flex::Status) -> flex::Style {
    let p = theme.palette();
    flex::Style::default().border(Border {
        radius: 0.0.into(),
        width: 1.0,
        color: p.background.strong.color,
    })
}
