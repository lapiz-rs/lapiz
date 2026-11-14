use std::{borrow::Cow, mem, sync::Arc};

use iced_core::Length;
use lapiz_assets::store::AssetRegistry;
use lapiz_i18n::t;
use lapiz_widgets::{
    button, column,
    component::{Component, component},
    container, icon, label, row, scrollable, text_input,
};
use uuid::Uuid;

use crate::{
    GraphElement, GraphRenderer, GraphTheme,
    editor::types::GraphTypeSelector,
    graph::{
        slot::{ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphInputSlotId},
        variable::{GraphLiteral, GraphTypeRegistry},
    },
};

/// A UI component for viewing, adding, renaming, removing, and editing named [`GraphLiteral`] parameters.
pub struct ParametersEditor<'a, Message> {
    parameters: Vec<(Cow<'a, str>, &'a GraphLiteral)>,
    type_registry: &'a GraphTypeRegistry,
    assets: &'a AssetRegistry,
    on_change: Box<dyn Fn(ParametersEditorMessage) -> Message + 'a>,
}

impl<'a, Message> ParametersEditor<'a, Message> {
    pub fn new(
        parameters: impl IntoIterator<Item = (&'a str, &'a GraphLiteral)>,
        type_registry: &'a GraphTypeRegistry,
        assets: &'a AssetRegistry,
        on_change: impl Fn(ParametersEditorMessage) -> Message + 'a,
    ) -> Self {
        Self {
            parameters: parameters
                .into_iter()
                .map(|(name, value)| (Cow::Borrowed(name), value))
                .collect(),
            type_registry,
            assets,
            on_change: Box::new(on_change),
        }
    }
}

impl<'a, Message> Component<'a, Message, GraphTheme, GraphRenderer>
    for ParametersEditor<'a, Message>
{
    type State = ParametersEditorState;
    type Event = ParametersEditorEvent;

    fn update(
        &mut self,
        state: &mut Self::State,
        event: Self::Event,
        _renderer: &GraphRenderer,
    ) -> Option<Message> {
        let message = match event {
            ParametersEditorEvent::NewNameChanged(name) => {
                state.new_name = name;
                return None;
            }
            ParametersEditorEvent::NewTypeSelected(ty) => {
                state.new_type = ty;
                return None;
            }
            ParametersEditorEvent::AddPressed => {
                let ty = state.new_type.clone()?;
                let name = mem::take(&mut state.new_name);
                ParametersEditorMessage::Add {
                    name,
                    value: GraphLiteral::new_boxed_default(ty),
                }
            }
            ParametersEditorEvent::Apply(message) => message,
        };
        Some((self.on_change)(message))
    }

    fn view(&self, state: &Self::State) -> GraphElement<'a, Self::Event> {
        let parameter_rows = self
            .parameters
            .iter()
            .enumerate()
            .map(|(index, (name, value))| {
                let literal = value
                    .ty()
                    .view_literal(
                        GraphInputSlotId::new(Uuid::nil()),
                        value.value(),
                        value.metadata(),
                        self.assets,
                    )
                    .map(move |message| {
                        ParametersEditorEvent::Apply(ParametersEditorMessage::UpdateLiteral {
                            index,
                            message,
                        })
                    });
                column![
                    row![
                        text_input(&t!("name"), name)
                            .on_input(move |name| {
                                ParametersEditorEvent::Apply(ParametersEditorMessage::Rename {
                                    index,
                                    name,
                                })
                            })
                            .width(Length::Fill),
                        label(value.ty().id().id).size(10).faint(),
                        button(icon::trash().size(13))
                            .width(24)
                            .height(24)
                            .padding(5)
                            .danger()
                            .on_press(ParametersEditorEvent::Apply(
                                ParametersEditorMessage::Remove { index },
                            )),
                    ]
                    .gap(4.0),
                    literal,
                ]
                .gap(4.0)
                .into()
            })
            .collect::<Vec<_>>();

        let parameters = if parameter_rows.is_empty() {
            GraphElement::from(container(label(t!("no_parameters")).muted()).height(Length::Fill))
        } else {
            GraphElement::from(scrollable(column(parameter_rows).gap(8.0)).height(Length::Fill))
        };

        let add_row = column![
            text_input(&t!("name"), &state.new_name)
                .on_input(ParametersEditorEvent::NewNameChanged)
                .width(Length::Fill),
            component(GraphTypeSelector::new(
                self.type_registry,
                ParametersEditorEvent::NewTypeSelected,
            )),
            button(row![icon::plus().size(13), label(t!("add_parameter"))].gap(4.0))
                .on_press_maybe(
                    (!state.new_name.trim().is_empty() && state.new_type.is_some())
                        .then_some(ParametersEditorEvent::AddPressed),
                ),
        ]
        .gap(4.0);

        column![parameters, add_row]
            .gap(8.0)
            .height(Length::Fill)
            .into()
    }
}

/// A mutation emitted by [`ParametersEditor`] when a parameter is added, removed, renamed, or edited.
#[derive(Clone)]
pub enum ParametersEditorMessage {
    Add {
        name: String,
        value: GraphLiteral,
    },
    Remove {
        index: usize,
    },
    Rename {
        index: usize,
        name: String,
    },
    UpdateLiteral {
        index: usize,
        message: ErasedGraphLiteralUpdateMessage,
    },
}

/// Internal UI state for the new-parameter creation form in [`ParametersEditor`].
#[derive(Default)]
pub struct ParametersEditorState {
    new_name: String,
    new_type: Option<Arc<dyn ErasedGraphValueType>>,
}

/// Internal component event handled by [`ParametersEditor`].
#[derive(Clone)]
pub enum ParametersEditorEvent {
    NewNameChanged(String),
    NewTypeSelected(Option<Arc<dyn ErasedGraphValueType>>),
    AddPressed,
    Apply(ParametersEditorMessage),
}
