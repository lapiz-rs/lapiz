use std::{fmt, sync::Arc};

use iced_core::Length;
use lapiz_i18n::t;
use lapiz_widgets::{checkbox, column, combo_box, component::Component, row, text_input};

use crate::{
    GraphElement, GraphRenderer, GraphTheme,
    graph::{
        slot::{ErasedGraphValueType, GraphValueType},
        variable::GraphTypeRegistry,
    },
    wgsl_std::types::{
        atomic::{AtomicI32Type, AtomicU32Type},
        primitive::{I32Type, U32Type},
    },
};

/// A UI component for selecting a [`GraphValueType`] (including fixed-length array variants) from a [`GraphTypeRegistry`].
pub struct GraphTypeSelector<'a, Message> {
    type_registry: &'a GraphTypeRegistry,
    on_select: Box<dyn Fn(Option<Arc<dyn ErasedGraphValueType>>) -> Message + 'a>,
}

impl<'a, Message> GraphTypeSelector<'a, Message> {
    pub fn new(
        type_registry: &'a GraphTypeRegistry,
        on_select: impl Fn(Option<Arc<dyn ErasedGraphValueType>>) -> Message + 'a,
    ) -> Self {
        Self {
            type_registry,
            on_select: Box::new(on_select),
        }
    }

    fn type_options(&self, array: bool) -> Vec<GraphTypeChoice> {
        self.type_registry
            .all_types()
            .values()
            .filter(|ty| {
                !array
                    || ty.wgsl_array_element_stride().is_some()
                    || ty.is::<AtomicI32Type>()
                    || ty.is::<AtomicU32Type>()
            })
            .map(|ty| GraphTypeChoice { ty: ty.clone() })
            .collect()
    }

    fn resolve_type(
        &self,
        state: &GraphTypeSelectorState,
    ) -> Option<Arc<dyn ErasedGraphValueType>> {
        let choice = state.selected.as_ref()?;
        if !state.array {
            return Some(choice.ty.clone());
        }

        let len = state
            .element_count
            .parse::<u32>()
            .ok()
            .filter(|len| *len > 0)?;
        if choice.ty.is::<AtomicI32Type>() {
            return self
                .type_registry
                .atomic_array_type(&GraphValueType::id(&I32Type).id, len);
        }
        if choice.ty.is::<AtomicU32Type>() {
            return self
                .type_registry
                .atomic_array_type(&GraphValueType::id(&U32Type).id, len);
        }
        self.type_registry.array_type(&choice.ty.id().id, len)
    }
}

impl<'a, Message> Component<'a, Message, GraphTheme, GraphRenderer>
    for GraphTypeSelector<'a, Message>
{
    type State = GraphTypeSelectorState;
    type Event = GraphTypeSelectorEvent;

    fn update(
        &mut self,
        state: &mut Self::State,
        event: Self::Event,
        _renderer: &GraphRenderer,
    ) -> Option<Message> {
        match event {
            GraphTypeSelectorEvent::ArrayToggled(array) => {
                state.array = array;
                if array
                    && state.selected.as_ref().is_some_and(|choice| {
                        choice.ty.wgsl_array_element_stride().is_none()
                            && !choice.ty.is::<AtomicI32Type>()
                            && !choice.ty.is::<AtomicU32Type>()
                    })
                {
                    state.selected = None;
                }
            }
            GraphTypeSelectorEvent::ElementCountChanged(element_count) => {
                state.element_count = element_count;
            }
            GraphTypeSelectorEvent::TypeSelected(choice) => {
                state.selected = Some(choice);
            }
        }
        Some((self.on_select)(self.resolve_type(state)))
    }

    fn view(&self, state: &Self::State) -> GraphElement<'a, Self::Event> {
        let options = self.type_options(state.array);
        let selected = state
            .selected
            .as_ref()
            .filter(|selected| options.contains(selected))
            .cloned();
        let selector = combo_box(options, selected, GraphTypeSelectorEvent::TypeSelected)
            .placeholder(t!("type"))
            .width(Length::Fill);

        let mut array_controls = row![
            checkbox(state.array)
                .label(t!("array"))
                .on_toggle(GraphTypeSelectorEvent::ArrayToggled),
        ]
        .gap(6.0);
        if state.array {
            array_controls = array_controls.push(
                text_input(&t!("element_count"), &state.element_count)
                    .on_input(GraphTypeSelectorEvent::ElementCountChanged)
                    .width(80),
            );
        }

        column![array_controls, selector].gap(4.0).into()
    }
}

/// A selectable value-type entry displayed in a [`GraphTypeSelector`] combo box.
#[derive(Clone)]
pub struct GraphTypeChoice {
    ty: Arc<dyn ErasedGraphValueType>,
}

impl PartialEq for GraphTypeChoice {
    fn eq(&self, other: &Self) -> bool {
        self.ty.id() == other.ty.id()
    }
}

impl fmt::Display for GraphTypeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.ty.id().id)
    }
}

/// Internal UI state for [`GraphTypeSelector`], tracking the array toggle, length input, and selected base type.
pub struct GraphTypeSelectorState {
    array: bool,
    element_count: String,
    selected: Option<GraphTypeChoice>,
}

impl Default for GraphTypeSelectorState {
    fn default() -> Self {
        Self {
            array: false,
            element_count: "1".into(),
            selected: None,
        }
    }
}

/// Internal UI event handled by [`GraphTypeSelector`].
#[derive(Clone)]
pub enum GraphTypeSelectorEvent {
    ArrayToggled(bool),
    ElementCountChanged(String),
    TypeSelected(GraphTypeChoice),
}
