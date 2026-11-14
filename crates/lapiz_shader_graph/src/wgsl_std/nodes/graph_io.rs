use iced_core::Length;
use lapiz_i18n::t;
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::{column, combo_box, text_input, text_input::default};
use serde::{Deserialize, Serialize};

use crate::{
    GraphElement,
    graph::{
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeUpdateContext, GraphNodeUpdateSignatureContext,
            GraphNodeViewContext,
        },
        slot::{ErasedGraphLiteralUpdateMessage, GraphDefaultInputSlot, GraphDefaultOutputSlot},
    },
};

/// A shader graph node that declares a typed external input parameter on the owning [`Graph`](crate::graph::Graph)'s signature.
#[derive(Default, Clone)]
pub struct GraphInputNode;

/// Per-instance state for [`GraphInputNode`], storing the parameter's name and value type ID.
#[derive(Default, Serialize, Deserialize)]
pub struct GraphInputNodeState {
    pub name: String,
    pub ty: Option<String>,
}

/// UI message emitted by [`GraphInputNode`].
#[derive(Clone)]
pub enum GraphInputNodeMessage {
    NameChanged(String),
    TypeChanged(String),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for GraphInputNode {
    type State = GraphInputNodeState;
    type Message = GraphInputNodeMessage;

    fn id(&self) -> &'static str {
        "graph_input_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        GraphInputNodeState::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(GraphInputNode)
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
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        let Some(ty) = state
            .ty
            .as_deref()
            .and_then(|ty| ctx.resources.type_registry.resolve_type(ty))
        else {
            return vec![];
        };

        vec![GraphDefaultOutputSlot::new_boxed(
            state.name.clone(),
            ty.clone(),
        )]
    }

    fn update_signature(&self, state: &Self::State, mut ctx: GraphNodeUpdateSignatureContext<'_>) {
        ctx.require_output_slot_as_graph_input(0, state.name.clone());
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        let types = ctx
            .resources
            .type_registry
            .all_types()
            .keys()
            .map(|id| id.id.clone())
            .collect::<Vec<_>>();
        ctx.view_all_slots_with_header(
            column![
                text_input(&t!("name"), &state.name)
                    .size(12.0)
                    .style(default)
                    .on_input(GraphInputNodeMessage::NameChanged),
                combo_box(types, state.ty.clone(), GraphInputNodeMessage::TypeChanged)
                    .width(Length::Fill),
            ]
            .gap(2.0),
            GraphInputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            GraphInputNodeMessage::NameChanged(name) => state.name = name,
            GraphInputNodeMessage::TypeChanged(ty) => state.ty = Some(ty),
            GraphInputNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        _: &Self::State,
        _: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(Default::default())
    }
}

/// A shader graph node that declares a typed external output value on the owning [`Graph`](crate::graph::Graph)'s signature.
#[derive(Default, Clone)]
pub struct GraphOutputNode;

/// Per-instance state for [`GraphOutputNode`], storing the output's name and value type ID.
#[derive(Default, Serialize, Deserialize)]
pub struct GraphOutputNodeState {
    pub name: String,
    pub ty: Option<String>,
}

/// UI message emitted by [`GraphOutputNode`].
#[derive(Clone)]
pub enum GraphOutputNodeMessage {
    NameChanged(String),
    TypeChanged(String),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for GraphOutputNode {
    type State = GraphOutputNodeState;
    type Message = GraphOutputNodeMessage;

    fn id(&self) -> &'static str {
        "graph_output_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        GraphOutputNodeState::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(GraphOutputNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        let Some(ty) = state
            .ty
            .as_deref()
            .and_then(|ty| ctx.resources.type_registry.resolve_type(ty))
        else {
            return vec![];
        };

        vec![GraphDefaultInputSlot::new_boxed(state.name.clone(), ty)]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![]
    }

    fn update_signature(&self, state: &Self::State, mut ctx: GraphNodeUpdateSignatureContext<'_>) {
        ctx.require_input_slot_as_graph_output(0, state.name.clone());
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        let types = ctx
            .resources
            .type_registry
            .all_types()
            .keys()
            .map(|id| id.id.clone())
            .collect::<Vec<_>>();
        ctx.view_all_slots_with_header(
            column![
                text_input(&t!("name"), &state.name)
                    .size(12.0)
                    .style(default)
                    .on_input(GraphOutputNodeMessage::NameChanged),
                combo_box(types, state.ty.clone(), GraphOutputNodeMessage::TypeChanged)
                    .width(Length::Fill),
            ]
            .gap(2.0),
            GraphOutputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            GraphOutputNodeMessage::NameChanged(name) => state.name = name,
            GraphOutputNodeMessage::TypeChanged(ty) => state.ty = Some(ty),
            GraphOutputNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        _: &Self::State,
        _: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(Default::default())
    }
}
