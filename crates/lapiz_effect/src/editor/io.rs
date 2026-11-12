use std::{mem, sync::Arc};

use iced_core::Length;
use indexmap::IndexMap;
use lapiz_i18n::t;
use lapiz_shader_graph::{
    GraphElement, GraphRenderer, GraphTheme,
    graph::{slot::ErasedGraphValueType, variable::GraphTypeRegistry},
};
use lapiz_widgets::{
    button, column, combo_box, component::Component, flex::Flex, icon, icon_button, label, row,
    scrollable, text_input,
};
use uuid::Uuid;

use crate::{
    asset::{EffectInputSlotId, EffectOutputSlotId},
    instance::{EffectInputSlot, EffectInstance, EffectOutputSlot},
    nodes::TypeChoice,
};

/// A UI [`Component`] for adding, removing, renaming, retyping, and reordering external input and output slots on an [`EffectInstance`].
pub struct EffectIoEditor<'a> {
    instance: &'a EffectInstance,
    type_options: Vec<TypeChoice>,
}

impl<'a> EffectIoEditor<'a> {
    pub fn new(instance: &'a EffectInstance, type_registry: &GraphTypeRegistry) -> Self {
        Self {
            instance,
            type_options: type_registry
                .all_types()
                .values()
                .map(|ty| TypeChoice {
                    label: ty.id().id,
                    ty: ty.clone(),
                })
                .collect(),
        }
    }

    fn selected_type(&self, ty: &Arc<dyn ErasedGraphValueType>) -> Option<TypeChoice> {
        self.type_options
            .iter()
            .find(|choice| choice.ty.id() == ty.id())
            .cloned()
    }

    fn view_inputs(&self) -> GraphElement<'a, EffectIoEditorEvent> {
        let len = self.instance.inputs.len();
        let rows = self
            .instance
            .inputs
            .values()
            .enumerate()
            .map(|(index, slot)| {
                let id = slot.id;
                self.slot_row(
                    &slot.name,
                    &slot.ty,
                    move |name| EffectIoEditorMessage::RenameInput(id, name),
                    move |choice| EffectIoEditorMessage::RetypeInput(id, choice),
                    EffectIoEditorMessage::RemoveInput(id),
                    (index > 0).then(|| EffectIoEditorMessage::MoveInput { index, up: true }),
                    (index + 1 < len)
                        .then(|| EffectIoEditorMessage::MoveInput { index, up: false }),
                )
            })
            .collect::<Vec<_>>();
        scrollable(Flex::column(rows).gap(4.0))
            .height(Length::Fill)
            .into()
    }

    fn view_outputs(&self) -> GraphElement<'a, EffectIoEditorEvent> {
        let len = self.instance.outputs.len();
        let rows = self
            .instance
            .outputs
            .values()
            .enumerate()
            .map(|(index, slot)| {
                let id = slot.id;
                self.slot_row(
                    &slot.name,
                    &slot.ty,
                    move |name| EffectIoEditorMessage::RenameOutput(id, name),
                    move |choice| EffectIoEditorMessage::RetypeOutput(id, choice),
                    EffectIoEditorMessage::RemoveOutput(id),
                    (index > 0).then(|| EffectIoEditorMessage::MoveOutput { index, up: true }),
                    (index + 1 < len)
                        .then(|| EffectIoEditorMessage::MoveOutput { index, up: false }),
                )
            })
            .collect::<Vec<_>>();
        scrollable(Flex::column(rows).gap(4.0))
            .height(Length::Fill)
            .into()
    }

    fn slot_row(
        &self,
        name: &str,
        ty: &Arc<dyn ErasedGraphValueType>,
        rename: impl Fn(String) -> EffectIoEditorMessage + 'a,
        retype: impl Fn(TypeChoice) -> EffectIoEditorMessage + 'a,
        remove: EffectIoEditorMessage,
        move_up: Option<EffectIoEditorMessage>,
        move_down: Option<EffectIoEditorMessage>,
    ) -> GraphElement<'a, EffectIoEditorEvent> {
        let mut controls = row![
            text_input(&t!("name"), name)
                .on_input(move |name| EffectIoEditorEvent::Apply(rename(name)))
                .width(Length::Fill),
            combo_box(
                self.type_options.clone(),
                self.selected_type(ty),
                move |choice| EffectIoEditorEvent::Apply(retype(choice)),
            )
            .placeholder(t!("type")),
        ]
        .gap(4.0);
        if let Some(message) = move_up {
            controls = controls.push(
                icon_button(icon::chevron_up().size(13))
                    .on_press(EffectIoEditorEvent::Apply(message)),
            );
        }
        if let Some(message) = move_down {
            controls = controls.push(
                icon_button(icon::chevron_down().size(13))
                    .on_press(EffectIoEditorEvent::Apply(message)),
            );
        }
        controls
            .push(
                button(icon::trash().size(13))
                    .width(24)
                    .height(24)
                    .padding(5)
                    .danger()
                    .on_press(EffectIoEditorEvent::Apply(remove)),
            )
            .into()
    }

    fn add_row(
        &self,
        name: &str,
        selected: Option<TypeChoice>,
        on_name: impl Fn(String) -> EffectIoEditorEvent + 'a,
        on_type: impl Fn(TypeChoice) -> EffectIoEditorEvent + 'a,
        add: EffectIoEditorEvent,
        enabled: bool,
        label: String,
    ) -> GraphElement<'a, EffectIoEditorEvent> {
        row![
            text_input(&t!("name"), name)
                .on_input(on_name)
                .width(Length::Fill),
            combo_box(self.type_options.clone(), selected, on_type).placeholder(t!("type")),
            button(lapiz_widgets::label(label)).on_press_maybe(enabled.then_some(add)),
        ]
        .gap(4.0)
        .into()
    }
}

impl<'a> Component<'a, EffectIoEditorMessage, GraphTheme, GraphRenderer> for EffectIoEditor<'a> {
    type State = EffectIoEditorState;
    type Event = EffectIoEditorEvent;

    fn update(
        &mut self,
        state: &mut Self::State,
        event: Self::Event,
        _renderer: &GraphRenderer,
    ) -> Option<EffectIoEditorMessage> {
        match event {
            EffectIoEditorEvent::NewInputNameChanged(name) => {
                state.new_input_name = name;
                None
            }
            EffectIoEditorEvent::NewInputTypeSelected(choice) => {
                state.new_input_type = Some(choice);
                None
            }
            EffectIoEditorEvent::AddInputPressed => {
                let ty = state.new_input_type.clone()?;
                let name = mem::take(&mut state.new_input_name);
                Some(EffectIoEditorMessage::AddInput { name, ty })
            }
            EffectIoEditorEvent::NewOutputNameChanged(name) => {
                state.new_output_name = name;
                None
            }
            EffectIoEditorEvent::NewOutputTypeSelected(choice) => {
                state.new_output_type = Some(choice);
                None
            }
            EffectIoEditorEvent::AddOutputPressed => {
                let ty = state.new_output_type.clone()?;
                let name = mem::take(&mut state.new_output_name);
                Some(EffectIoEditorMessage::AddOutput { name, ty })
            }
            EffectIoEditorEvent::Apply(message) => Some(message),
        }
    }

    fn view(&self, state: &Self::State) -> GraphElement<'a, Self::Event> {
        column![
            label(t!("inputs")).strong(),
            self.view_inputs(),
            self.add_row(
                &state.new_input_name,
                state.new_input_type.clone(),
                EffectIoEditorEvent::NewInputNameChanged,
                EffectIoEditorEvent::NewInputTypeSelected,
                EffectIoEditorEvent::AddInputPressed,
                !state.new_input_name.trim().is_empty() && state.new_input_type.is_some(),
                t!("add_input"),
            ),
            label(t!("outputs")).strong(),
            self.view_outputs(),
            self.add_row(
                &state.new_output_name,
                state.new_output_type.clone(),
                EffectIoEditorEvent::NewOutputNameChanged,
                EffectIoEditorEvent::NewOutputTypeSelected,
                EffectIoEditorEvent::AddOutputPressed,
                !state.new_output_name.trim().is_empty() && state.new_output_type.is_some(),
                t!("add_output"),
            ),
        ]
        .gap(6.0)
        .into()
    }
}

/// Internal UI state for the new-input and new-output creation rows in [`EffectIoEditor`].
#[derive(Default)]
pub struct EffectIoEditorState {
    new_input_name: String,
    new_input_type: Option<TypeChoice>,
    new_output_name: String,
    new_output_type: Option<TypeChoice>,
}

/// Internal component event handled by [`EffectIoEditor`].
#[derive(Debug, Clone)]
pub enum EffectIoEditorEvent {
    NewInputNameChanged(String),
    NewInputTypeSelected(TypeChoice),
    AddInputPressed,
    NewOutputNameChanged(String),
    NewOutputTypeSelected(TypeChoice),
    AddOutputPressed,
    Apply(EffectIoEditorMessage),
}

/// A mutation message emitted by [`EffectIoEditor`] to modify the external inputs or outputs of an [`EffectInstance`].
#[derive(Debug, Clone)]
pub enum EffectIoEditorMessage {
    RenameInput(EffectInputSlotId, String),
    RetypeInput(EffectInputSlotId, TypeChoice),
    MoveInput { index: usize, up: bool },
    RemoveInput(EffectInputSlotId),
    AddInput { name: String, ty: TypeChoice },
    RenameOutput(EffectOutputSlotId, String),
    RetypeOutput(EffectOutputSlotId, TypeChoice),
    MoveOutput { index: usize, up: bool },
    RemoveOutput(EffectOutputSlotId),
    AddOutput { name: String, ty: TypeChoice },
}

pub(crate) fn apply(instance: &mut EffectInstance, message: EffectIoEditorMessage) {
    match message {
        EffectIoEditorMessage::RenameInput(id, name) => {
            if let Some(slot) = instance.inputs.get_mut(&id) {
                slot.name = name;
            }
        }
        EffectIoEditorMessage::RetypeInput(id, choice) => {
            if let Some(slot) = instance.inputs.get_mut(&id) {
                slot.ty = choice.ty;
            }
        }
        EffectIoEditorMessage::MoveInput { index, up } => {
            move_slot(&mut instance.inputs, index, up)
        }
        EffectIoEditorMessage::RemoveInput(id) => {
            instance.inputs.shift_remove(&id);
        }
        EffectIoEditorMessage::AddInput { name, ty } => {
            let id = EffectInputSlotId::new(Uuid::new_v4());
            instance.inputs.insert(
                id,
                EffectInputSlot {
                    name,
                    id,
                    ty: ty.ty,
                },
            );
        }
        EffectIoEditorMessage::RenameOutput(id, name) => {
            if let Some(slot) = instance.outputs.get_mut(&id) {
                slot.name = name;
            }
        }
        EffectIoEditorMessage::RetypeOutput(id, choice) => {
            if let Some(slot) = instance.outputs.get_mut(&id) {
                slot.ty = choice.ty;
            }
        }
        EffectIoEditorMessage::MoveOutput { index, up } => {
            move_slot(&mut instance.outputs, index, up)
        }
        EffectIoEditorMessage::RemoveOutput(id) => {
            instance.outputs.shift_remove(&id);
        }
        EffectIoEditorMessage::AddOutput { name, ty } => {
            let id = EffectOutputSlotId::new(Uuid::new_v4());
            instance.outputs.insert(
                id,
                EffectOutputSlot {
                    name,
                    id,
                    ty: ty.ty,
                },
            );
        }
    }
}

pub(crate) fn move_slot<K, V>(slots: &mut IndexMap<K, V>, index: usize, up: bool) {
    let destination = if up {
        index.checked_sub(1)
    } else {
        Some(index + 1)
    };
    if let Some(destination) = destination.filter(|&destination| destination < slots.len()) {
        slots.swap_indices(index, destination);
    }
}
