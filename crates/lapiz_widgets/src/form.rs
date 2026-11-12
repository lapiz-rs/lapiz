use iced_core::{Element, Padding, Pixels, Theme, text};
use lapiz_runtime::Renderer;

use crate::column;

/// A vertical form layout that stacks labeled control pairs.
///
/// Each entry added via [`push`](Self::push) or [`extend`](Self::extend) is arranged as a
/// tight two-row column containing the label above its corresponding value widget, separated
/// from adjacent entries by uniform vertical spacing.
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
        self.items.push((crate::label(label).into(), value.into()));
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
        column(
            items
                .into_iter()
                .map(|(label, value)| column![label, value].gap(spacing * 0.5).into()),
        )
        .padding(padding)
        .gap(spacing)
        .into()
    }
}
