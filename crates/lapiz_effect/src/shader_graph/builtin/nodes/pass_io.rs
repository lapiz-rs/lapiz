use std::{fmt, iter, sync::Arc};

use anyhow::{Context as _, Result};
use iced_core::{Length, widget::Void};
use lapiz_i18n::t;
use lapiz_widgets::{column, combo_box, component::component, flex::Flex, label, row, text_input};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wesl::syntax::{
    Declaration, DeclarationKind, Expression, Ident, Span, Spanned, Statement, TypeExpression,
};
use wesl_quote::quote_statement;

use crate::{
    asset::{
        EffectOutputSlotId, EffectPassDispatchStrategy, EffectPassInputSlotId,
        EffectPassOutputSlotId,
    },
    random_oklch_hue_chroma,
    render::{pass_input_ident, pass_output_ident},
    shader_graph::{
        GraphElement,
        builtin::types::{primitive::U32Type, vector::Vec2IType},
        editor::types::GraphTypeSelector,
        graph::{
            GraphResources,
            node::{
                GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError,
                GraphNodeCreateSlotsContext, GraphNodeDefaultStateContext, GraphNodeInputDef,
                GraphNodeInputSource, GraphNodeOutputDef, GraphNodeOutputTarget,
                GraphNodeUpdateContext, GraphNodeViewContext, PassInput, PassInputChoice,
                PassOutputChoice, PassOutputDef,
            },
            slot::{
                ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphDefaultInputSlot,
                GraphDefaultOutputSlot,
            },
        },
        save::GraphSerializable,
    },
};

/// A shader graph node that reads an external effect input or an upstream pass's output into the current pass.
///
/// `PassInputNode` dynamically configures its output slots via
/// [`ErasedGraphValueType::push_output_slots`] to match the type of the bound [`PassInput`] source.
#[derive(Default, Clone)]
pub struct PassInputNode;

/// A selectable shader value type option used in pass and effect I/O editors.
#[derive(Clone)]
pub struct TypeChoice {
    pub label: String,
    pub ty: Arc<dyn ErasedGraphValueType>,
}

impl fmt::Debug for TypeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypeChoice")
            .field("label", &self.label)
            .finish()
    }
}

impl PartialEq for TypeChoice {
    fn eq(&self, other: &Self) -> bool {
        self.ty.id() == other.ty.id()
    }
}

impl fmt::Display for TypeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

#[derive(Clone, PartialEq)]
struct PassInputSelectorChoice(Option<PassInputChoice>);

impl fmt::Display for PassInputSelectorChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(choice) => f.write_str(&choice.label),
            None => f.write_str(&t!("unbound")),
        }
    }
}

/// Per-instance state for [`PassInputNode`], storing its unique slot ID, bound [`PassInput`], and synchronized choices.
#[derive(Clone)]
pub struct PassInputNodeState {
    pub id: EffectPassInputSlotId,
    pub input: Option<PassInput>,
    pub cached_ty: Option<Arc<dyn ErasedGraphValueType>>,
    // Refreshed by EffectInstance::sync_pass_graph_effect_properties, not serialized.
    pub available_sources: Arc<[PassInputChoice]>,
}

#[derive(Serialize, Deserialize)]
struct SerializablePassInputNodeState {
    id: EffectPassInputSlotId,
    input: Option<PassInput>,
    ty: Option<String>,
}

impl GraphSerializable for PassInputNodeState {
    fn to_toml(&self) -> Result<toml::Value> {
        SerializablePassInputNodeState {
            id: self.id,
            input: self.input,
            ty: self.cached_ty.as_ref().map(|ty| ty.id().id),
        }
        .to_toml()
    }

    fn from_toml(value: toml::Value, resources: &GraphResources) -> Result<Self> {
        let serializable = SerializablePassInputNodeState::from_toml(value, resources)?;
        let cached_ty = serializable
            .ty
            .map(|ty| {
                resources
                    .type_registry
                    .resolve_type(&ty)
                    .with_context(|| format!("Unknown pass input type '{ty}'"))
            })
            .transpose()?;
        Ok(Self {
            id: serializable.id,
            input: serializable.input,
            cached_ty,
            available_sources: Vec::new().into(),
        })
    }
}

/// UI message emitted by [`PassInputNode`].
#[derive(Debug, Clone)]
pub enum PassInputNodeMessage {
    SourceSelected(Option<PassInputChoice>),
}

impl GraphNode for PassInputNode {
    type State = PassInputNodeState;
    type Message = PassInputNodeMessage;

    fn id(&self) -> &'static str {
        "pass_input_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        Self::State {
            id: EffectPassInputSlotId::new(Uuid::new_v4()),
            input: None,
            cached_ty: None,
            available_sources: Vec::new().into(),
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PassInputNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        match &state.cached_ty {
            None => vec![],
            Some(ty) => ty.as_input_node_outputs(),
        }
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        let options = iter::once(PassInputSelectorChoice(None))
            .chain(
                state
                    .available_sources
                    .iter()
                    .cloned()
                    .map(|choice| PassInputSelectorChoice(Some(choice))),
            )
            .collect::<Vec<_>>();
        let selected = match state.input {
            None => Some(PassInputSelectorChoice(None)),
            Some(input) => state
                .available_sources
                .iter()
                .find(|choice| choice.input == input)
                .cloned()
                .map(|choice| PassInputSelectorChoice(Some(choice))),
        };
        let selector = combo_box(options, selected, |choice| {
            PassInputNodeMessage::SourceSelected(choice.0)
        })
        .placeholder(t!("unbound"))
        .width(Length::Fill);

        column![selector, Flex::column(ctx.view_all_outputs()).gap(2.0),]
            .gap(4.0)
            .width(Length::Fill)
            .into()
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        _: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            PassInputNodeMessage::SourceSelected(Some(choice)) => {
                state.input = Some(choice.input);
                state.cached_ty = Some(choice.ty);
            }
            PassInputNodeMessage::SourceSelected(None) => {
                state.input = None;
                state.cached_ty = None;
            }
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let Some(ty) = &state.cached_ty else {
            return Ok(String::new());
        };
        let name = pass_input_ident(state.id);

        Ok(ty.transfer_input_variable_to_output_slot(&name, 0, &mut ctx)?)
    }

    fn as_pass_input(&self, state: &Self::State) -> Option<GraphNodeInputDef> {
        let input = state.input?;
        let label = state
            .available_sources
            .iter()
            .find(|choice| choice.input == input)
            .map(|choice| choice.label.clone())
            .unwrap_or_else(|| t!("unbound"));

        Some(GraphNodeInputDef {
            label,
            ty: state.cached_ty.clone(),
            id: state.id,
            source: match input {
                PassInput::Pass(id) => GraphNodeInputSource::Pass(id),
                PassInput::Effect(id) => GraphNodeInputSource::Effect(id),
            },
        })
    }

    fn on_available_pass_input_choices_changed(
        &self,
        state: &mut Self::State,
        new_inputs: Arc<[PassInputChoice]>,
    ) -> bool {
        state.available_sources = new_inputs;

        match state.input {
            Some(input) => {
                if let Some(choice) = state
                    .available_sources
                    .iter()
                    .find(|choice| choice.input == input)
                {
                    let ty_changed =
                        state.cached_ty.as_ref().map(|ty| ty.id()) != Some(choice.ty.id());
                    state.cached_ty = Some(choice.ty.clone());
                    ty_changed
                } else {
                    state.input = None;
                    state.cached_ty = None;
                    true
                }
            }
            None => state.cached_ty.take().is_some(),
        }
    }
}

/// A shader graph node that writes a pass's computed result to an external effect output or an intermediate pass buffer.
///
/// `PassOutputNode` dynamically configures its input slots via
/// [`ErasedGraphValueType::push_input_slots`] to match the type of the bound [`PassOutput`] target.
#[derive(Default, Clone)]
pub struct PassOutputNode;

#[derive(Serialize, Deserialize)]
struct SerializablePassOutputDef {
    name: String,
    ty: String,
}

impl GraphSerializable for PassOutputDef {
    fn to_toml(&self) -> Result<toml::Value> {
        let serializable = SerializablePassOutputDef {
            name: self.name.clone(),
            ty: self.ty.id().id,
        };
        serializable.to_toml()
    }

    fn from_toml(value: toml::Value, resources: &GraphResources) -> Result<Self> {
        let serializable = SerializablePassOutputDef::from_toml(value, resources)?;
        Ok(PassOutputDef {
            name: serializable.name,
            ty: resources
                .type_registry
                .resolve_type(&serializable.ty)
                .with_context(|| format!("Unknown pass output type '{}'", serializable.ty))?
                .clone(),
        })
    }
}

/// Identifies the destination bound to a [`PassOutputNode`]: either a pass-local buffer or an effect-level output.
#[derive(Clone)]
pub enum PassOutput {
    Pass(PassOutputDef),
    Effect(EffectOutputSlotId),
}

/// Per-instance state for [`PassOutputNode`], storing its unique slot ID, target [`PassOutput`], and synchronized choices.
pub struct PassOutputNodeState {
    pub id: EffectPassOutputSlotId,
    pub output: Option<PassOutput>,
    pub cached_ty: Option<Arc<dyn ErasedGraphValueType>>,
    // Refreshed by EffectInstance::sync_pass_graph_effect_properties, not serialized.
    pub available_targets: Arc<[PassOutputChoice]>,
}

#[derive(Serialize, Deserialize)]
enum SerializablePassOutput {
    Pass(SerializablePassOutputDef),
    Effect(EffectOutputSlotId),
}

#[derive(Serialize, Deserialize)]
struct SerializablePassOutputNodeState {
    id: EffectPassOutputSlotId,
    output: Option<SerializablePassOutput>,
    ty: Option<String>,
}

impl GraphSerializable for PassOutputNodeState {
    fn to_toml(&self) -> Result<toml::Value> {
        SerializablePassOutputNodeState {
            id: self.id,
            output: self.output.as_ref().map(|output| match output {
                PassOutput::Pass(def) => SerializablePassOutput::Pass(SerializablePassOutputDef {
                    name: def.name.clone(),
                    ty: def.ty.id().id,
                }),
                PassOutput::Effect(id) => SerializablePassOutput::Effect(*id),
            }),
            ty: self.cached_ty.as_ref().map(|ty| ty.id().id),
        }
        .to_toml()
    }

    fn from_toml(value: toml::Value, resources: &GraphResources) -> Result<Self> {
        let serializable = SerializablePassOutputNodeState::from_toml(value, resources)?;
        let output = serializable
            .output
            .map(|output| -> Result<PassOutput> {
                match output {
                    SerializablePassOutput::Pass(def) => Ok(PassOutput::Pass(PassOutputDef {
                        name: def.name,
                        ty: resources
                            .type_registry
                            .resolve_type(&def.ty)
                            .with_context(|| format!("Unknown pass output type '{}'", def.ty))?
                            .clone(),
                    })),
                    SerializablePassOutput::Effect(id) => Ok(PassOutput::Effect(id)),
                }
            })
            .transpose()?;
        let cached_ty = serializable
            .ty
            .map(|ty| {
                resources
                    .type_registry
                    .resolve_type(&ty)
                    .with_context(|| format!("Unknown pass output type '{ty}'"))
            })
            .transpose()?;
        Ok(Self {
            id: serializable.id,
            output,
            cached_ty,
            available_targets: Vec::new().into(),
        })
    }
}

#[derive(Clone, PartialEq)]
enum PassOutputSelectorChoice {
    Unbound,
    Effect(PassOutputChoice),
    LocalBuffer,
}

impl fmt::Display for PassOutputSelectorChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unbound => f.write_str(&t!("unbound")),
            Self::Effect(choice) => f.write_str(&choice.label),
            Self::LocalBuffer => f.write_str(&t!("local_buffer")),
        }
    }
}

/// UI message emitted by [`PassOutputNode`].
#[derive(Clone)]
pub enum PassOutputNodeMessage {
    UnboundSelected,
    EffectTargetSelected(PassOutputChoice),
    LocalBufferSelected,
    LocalNameChanged(String),
    LocalTypeSelected(Option<Arc<dyn ErasedGraphValueType>>),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for PassOutputNode {
    type State = PassOutputNodeState;
    type Message = PassOutputNodeMessage;

    fn id(&self) -> &'static str {
        "pass_output_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        Self::State {
            id: EffectPassOutputSlotId::new(Uuid::new_v4()),
            output: None,
            cached_ty: None,
            available_targets: Vec::new().into(),
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PassOutputNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        match &state.output {
            None => vec![],
            Some(PassOutput::Pass(def)) => def.ty.as_output_node_inputs(),
            Some(PassOutput::Effect(_)) => state
                .cached_ty
                .as_ref()
                .map_or_else(Vec::new, |ty| ty.as_output_node_inputs()),
        }
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![]
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        let options = iter::once(PassOutputSelectorChoice::Unbound)
            .chain(
                state
                    .available_targets
                    .iter()
                    .cloned()
                    .map(PassOutputSelectorChoice::Effect),
            )
            .chain(iter::once(PassOutputSelectorChoice::LocalBuffer))
            .collect::<Vec<_>>();
        let selected = match &state.output {
            None => Some(PassOutputSelectorChoice::Unbound),
            Some(PassOutput::Effect(id)) => state
                .available_targets
                .iter()
                .find(|choice| choice.target == *id)
                .cloned()
                .map(PassOutputSelectorChoice::Effect),
            Some(PassOutput::Pass(_)) => Some(PassOutputSelectorChoice::LocalBuffer),
        };
        let selector = combo_box(options, selected, |choice| match choice {
            PassOutputSelectorChoice::Unbound => PassOutputNodeMessage::UnboundSelected,
            PassOutputSelectorChoice::Effect(choice) => {
                PassOutputNodeMessage::EffectTargetSelected(choice)
            }
            PassOutputSelectorChoice::LocalBuffer => PassOutputNodeMessage::LocalBufferSelected,
        })
        .placeholder(t!("unbound"))
        .width(Length::Fill);

        let local_editor = match &state.output {
            Some(PassOutput::Pass(def)) => Some(
                column![
                    row![
                        label(t!("name")),
                        text_input("", &def.name)
                            .on_input(PassOutputNodeMessage::LocalNameChanged)
                            .width(Length::Fill),
                    ]
                    .gap(4.0),
                    component(
                        GraphTypeSelector::new(
                            &ctx.resources.type_registry,
                            PassOutputNodeMessage::LocalTypeSelected,
                        )
                        .selected(&def.ty),
                    ),
                ]
                .gap(2.0),
            ),
            _ => None,
        };

        let content = column![selector].gap(4.0).width(Length::Fill);
        let content = match local_editor {
            Some(editor) => content.push(editor),
            None => content,
        };
        content
            .push(Flex::column(
                ctx.view_all_inputs(PassOutputNodeMessage::LiteralUpdate),
            ))
            .into()
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            PassOutputNodeMessage::UnboundSelected => {
                state.output = None;
                state.cached_ty = None;
            }
            PassOutputNodeMessage::EffectTargetSelected(choice) => {
                state.output = Some(PassOutput::Effect(choice.target));
                state.cached_ty = Some(choice.ty);
            }
            PassOutputNodeMessage::LocalBufferSelected => {
                if let Some(ty) = ctx.resources.type_registry.all_types().values().next() {
                    state.cached_ty = Some(ty.clone());
                    state.output = Some(PassOutput::Pass(PassOutputDef {
                        name: "buffer".into(),
                        ty: ty.clone(),
                    }));
                } else {
                    state.output = None;
                    state.cached_ty = None;
                }
            }
            PassOutputNodeMessage::LocalNameChanged(name) => {
                if let Some(PassOutput::Pass(def)) = &mut state.output {
                    def.name = name;
                }
            }
            PassOutputNodeMessage::LocalTypeSelected(Some(ty)) => {
                if let Some(PassOutput::Pass(def)) = &mut state.output {
                    def.ty = ty.clone();
                    state.cached_ty = Some(ty);
                }
            }
            PassOutputNodeMessage::LocalTypeSelected(None) => {}
            PassOutputNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let Some(output) = &state.output else {
            return Ok(String::new());
        };
        let ty = match output {
            PassOutput::Pass(def) => def.ty.clone(),
            PassOutput::Effect(_) => state
                .cached_ty
                .clone()
                .context("Effect output type has not been synchronized")?,
        };
        let name = pass_output_ident(state.id);

        Ok(ty.read_output_slot_to_input_variable(&name, 0, &mut ctx)?)
    }

    fn as_pass_output(&self, state: &Self::State) -> Option<GraphNodeOutputDef> {
        let output = state.output.as_ref()?;
        let (label, ty, target) = match output {
            PassOutput::Pass(def) => (
                format!("{} ({})", def.name, def.ty.id().id),
                Some(def.ty.clone()),
                GraphNodeOutputTarget::Pass(def.clone()),
            ),
            PassOutput::Effect(id) => (
                state
                    .available_targets
                    .iter()
                    .find(|choice| choice.target == *id)
                    .map(|choice| choice.label.clone())
                    .unwrap_or_else(|| t!("unbound")),
                state.cached_ty.clone(),
                GraphNodeOutputTarget::Effect(*id),
            ),
        };

        Some(GraphNodeOutputDef {
            label,
            ty,
            id: state.id,
            target,
        })
    }

    fn on_available_pass_output_choices_changed(
        &self,
        state: &mut Self::State,
        new_outputs: Arc<[PassOutputChoice]>,
    ) -> bool {
        state.available_targets = new_outputs;

        match &state.output {
            Some(PassOutput::Effect(id)) => {
                if let Some(choice) = state
                    .available_targets
                    .iter()
                    .find(|choice| choice.target == *id)
                {
                    let ty_changed =
                        state.cached_ty.as_ref().map(|ty| ty.id()) != Some(choice.ty.id());
                    state.cached_ty = Some(choice.ty.clone());
                    ty_changed
                } else {
                    state.output = None;
                    state.cached_ty = None;
                    true
                }
            }
            Some(PassOutput::Pass(def)) => {
                let ty_changed = state.cached_ty.as_ref().map(|ty| ty.id()) != Some(def.ty.id());
                state.cached_ty = Some(def.ty.clone());
                ty_changed
            }
            None => state.cached_ty.take().is_some(),
        }
    }
}

impl PassOutputNodeState {
    pub fn new(output: PassOutput) -> Self {
        Self {
            id: EffectPassOutputSlotId::new(Uuid::new_v4()),
            output: Some(output),
            cached_ty: None,
            available_targets: Vec::new().into(),
        }
    }
}

/// A shader graph node that outputs the current compute invocation's dispatch index or pixel coordinate.
///
/// Its output slot adapts to the owning pass's [`EffectPassDispatchStrategy`]: no output for
/// [`EffectPassDispatchStrategy::Once`], a `u32` buffer index for
/// [`EffectPassDispatchStrategy::EveryBufferElement`], or a `vec2i` pixel coordinate for layer
/// pixel dispatch strategies.
#[derive(Default, Clone)]
pub struct DispatchIndexNode;

/// Per-instance state for [`DispatchIndexNode`], caching the owning pass's [`EffectPassDispatchStrategy`].
#[derive(Clone, Serialize, Deserialize)]
pub struct DispatchIndexNodeState {
    pub cached_dispatch_strategy: EffectPassDispatchStrategy,
}

/// Uninhabited message enum for [`DispatchIndexNode`], which has no interactive controls.
#[derive(Clone)]
pub enum DispatchIndexNodeMessage {}

impl GraphNode for DispatchIndexNode {
    type State = DispatchIndexNodeState;
    type Message = DispatchIndexNodeMessage;

    fn id(&self) -> &'static str {
        "dispatch_index_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        DispatchIndexNodeState {
            cached_dispatch_strategy: EffectPassDispatchStrategy::Once,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(DispatchIndexNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        match state.cached_dispatch_strategy {
            EffectPassDispatchStrategy::Once => vec![],
            EffectPassDispatchStrategy::EveryBufferElement(_) => {
                vec![GraphDefaultOutputSlot::new::<U32Type>("index".into())]
            }
            EffectPassDispatchStrategy::EveryOutputLayerPixel(_)
            | EffectPassDispatchStrategy::EveryInputLayerPixel(_) => {
                vec![GraphDefaultOutputSlot::new::<Vec2IType>(
                    "pixel_position".into(),
                )]
            }
        }
    }

    fn view<'a>(
        &self,
        _: &'a Self::State,
        _: GraphNodeViewContext<'_>,
    ) -> GraphElement<'a, Self::Message> {
        Void.into()
    }

    fn update(&self, _: &mut Self::State, message: Self::Message, _: GraphNodeUpdateContext<'_>) {
        match message {}
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        if state.cached_dispatch_strategy == EffectPassDispatchStrategy::Once {
            return Ok(String::new());
        }

        let output = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            // Dispatch index is passed into graph() as a function param, and its type is dynamically aliased to
            // correct one.
            quote_statement! {
                let #output = dispatch_index;
            }
        ))
    }
}
