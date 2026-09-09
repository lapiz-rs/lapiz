use iced_core::{Element, Padding, Pixels, Theme, text};
use iced_widget::{Column, column};
use lapiz_runtime::Renderer;

use crate::label::Label;

pub struct Form<'a, Message> {
    items: Vec<(
        Element<'a, Message, Theme, Renderer>,
        Element<'a, Message, Theme, Renderer>,
    )>,
    padding: Padding,
    spacing: Pixels,
}

impl<'a, Message> Default for Form<'a, Message> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, Message> Form<'a, Message> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            padding: Padding::default(),
            spacing: Pixels(6.0),
        }
    }

    pub fn push<V>(mut self, label: impl text::IntoFragment<'a>, value: V) -> Self
    where
        Message: 'a,
        V: Into<Element<'a, Message, Theme, Renderer>>,
    {
        self.items.push((Label::new(label).into(), value.into()));
        self
    }

    pub fn extend<I>(mut self, items: I) -> Self
    where
        I: IntoIterator<
            Item = (
                Element<'a, Message, Theme, Renderer>,
                Element<'a, Message, Theme, Renderer>,
            ),
        >,
    {
        self.items.extend(items);
        self
    }

    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }
}

impl<'a, Message: 'a> From<Form<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(form: Form<'a, Message>) -> Element<'a, Message, Theme, Renderer> {
        let Form {
            items,
            padding,
            spacing,
        } = form;
        Column::new()
            .padding(padding)
            .spacing(spacing)
            .extend(
                items
                    .into_iter()
                    .map(|(label, value)| column![label, value].spacing(spacing * 0.5).into()),
            )
            .into()
    }
}
