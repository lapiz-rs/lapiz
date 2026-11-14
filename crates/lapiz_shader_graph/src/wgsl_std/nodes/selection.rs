use iced_core::Length;
use lapiz_i18n::Translated;
use lapiz_shader_graph_derive::stateless;
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::combo_box;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use wesl::syntax::{
    BinaryExpression, BinaryOperator, Declaration, DeclarationKind, Expression, FunctionCall,
    Ident, Span, Spanned, Statement, TypeExpression,
};
use wesl_quote::quote_statement;

use crate::{
    GraphElement,
    graph::{
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeUpdateContext, GraphNodeViewContext,
            StatelessCommonGraphNode,
        },
        slot::{ErasedGraphLiteralUpdateMessage, GraphDefaultInputSlot, GraphDefaultOutputSlot},
    },
    wgsl_std::types::{
        primitive::{BoolType, F32Type},
        vector::Vec2FType,
    },
};

/// Shader graph node that compares two [`F32Type`] scalars using a relational operator and outputs a [`BoolType`].
#[derive(Default, Clone)]
pub struct CompareNode;

/// Relational comparison operator for [`CompareNode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[display(style = "snake_case")]
pub enum CompareNodeMode {
    #[display("Less Than")]
    LessThan,
    #[display("Less Equal")]
    LessEqual,
    #[display("Greater Than")]
    GreaterThan,
    #[display("Greater Equal")]
    GreaterEqual,
    Equal,
}

impl CompareNodeMode {
    pub const ALL: [CompareNodeMode; 5] = [
        CompareNodeMode::LessThan,
        CompareNodeMode::LessEqual,
        CompareNodeMode::GreaterThan,
        CompareNodeMode::GreaterEqual,
        CompareNodeMode::Equal,
    ];
}

/// UI message emitted by [`CompareNode`].
#[derive(Clone)]
pub enum CompareNodeMessage {
    ModeChanged(CompareNodeMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for CompareNode {
    type State = CompareNodeMode;
    type Message = CompareNodeMessage;

    fn id(&self) -> &'static str {
        "compare_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        CompareNodeMode::LessThan
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CompareNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("lhs".into()),
            GraphDefaultInputSlot::new::<F32Type>("rhs".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<BoolType>("result".into())]
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        ctx.view_all_slots_with_header(
            combo_box(
                CompareNodeMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(Translated)
                    .collect::<Vec<_>>(),
                Some(Translated(*state)),
                |option| CompareNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(Length::Fill),
            CompareNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            CompareNodeMessage::ModeChanged(mode) => *state = mode,
            CompareNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let lhs = ctx.get_input(0)?;
        let rhs = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;

        let statement = match state {
            CompareNodeMode::LessThan => quote_statement! { let #output = #lhs < #rhs; },
            CompareNodeMode::LessEqual => quote_statement! { let #output = #lhs <= #rhs; },
            CompareNodeMode::GreaterThan => quote_statement! { let #output = #lhs > #rhs; },
            CompareNodeMode::GreaterEqual => quote_statement! { let #output = #lhs >= #rhs; },
            CompareNodeMode::Equal => quote_statement! { let #output = #lhs == #rhs; },
        };

        Ok(format!("{statement}\n"))
    }
}

/// Shader graph node that selects between two [`F32Type`] scalars based on a [`BoolType`] condition using WGSL `select`.
#[derive(Default, Clone)]
pub struct ScalarSelectNode;

#[stateless]
impl StatelessCommonGraphNode for ScalarSelectNode {
    fn id(&self) -> &'static str {
        "scalar_select_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ScalarSelectNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<BoolType>("condition".into()),
            GraphDefaultInputSlot::new::<F32Type>("false".into()),
            GraphDefaultInputSlot::new::<F32Type>("true".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let condition = ctx.get_input(0)?;
        let false_value = ctx.get_input(1)?;
        let true_value = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            quote_statement! { let #output = select(#false_value, #true_value, #condition); }
        ))
    }
}

/// Shader graph node that selects between two [`Vec2FType`] vectors based on a [`BoolType`] condition using WGSL `select`.
#[derive(Default, Clone)]
pub struct VectorSelectNode;

#[stateless]
impl StatelessCommonGraphNode for VectorSelectNode {
    fn id(&self) -> &'static str {
        "vector_select_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(VectorSelectNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<BoolType>("condition".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("false".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("true".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let condition = ctx.get_input(0)?;
        let false_value = ctx.get_input(1)?;
        let true_value = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            quote_statement! { let #output = select(#false_value, #true_value, #condition); }
        ))
    }
}
