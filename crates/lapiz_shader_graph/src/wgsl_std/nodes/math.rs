use iced_core::Length;
use lapiz_i18n::Translated;
use lapiz_shader_graph_derive::stateless;
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::combo_box;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use wesl::syntax::{
    AssignmentOperator, AssignmentStatement, BinaryExpression, BinaryOperator, CompoundStatement,
    Declaration, DeclarationKind, Expression, FunctionCall, Ident, IfClause, IfStatement,
    LiteralExpression, ModulePath, NamedComponentExpression, PathOrigin, Span, Spanned, Statement,
    TypeExpression, UnaryExpression, UnaryOperator,
};
use wesl_quote::{quote_expression, quote_statement};

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
    wgsl_std::types::{compound::RectType, primitive::F32Type, vector::Vec2FType},
};

/// Shader graph node that performs a configurable unary, binary, or ternary mathematical operation on [`F32Type`] scalars.
#[derive(Default, Clone)]
pub struct ScalarMathNode;

/// Mathematical operation mode for [`ScalarMathNode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[display(style = "snake_case")]
pub enum ScalarMathNodeMode {
    Add,
    Subtract,
    Multiply,
    Divide,
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Ceil,
    Cos,
    Cosh,
    Degrees,
    Exp,
    Exp2,
    Floor,
    Fract,
    InverseSqrt,
    Ln,
    Log2,
    Max,
    Min,
    Mix,
    Pow,
    Radians,
    Round,
    Saturate,
    Sign,
    Sin,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
    Trunc,
}

impl ScalarMathNodeMode {
    pub const ALL: [ScalarMathNodeMode; 35] = [
        ScalarMathNodeMode::Add,
        ScalarMathNodeMode::Subtract,
        ScalarMathNodeMode::Multiply,
        ScalarMathNodeMode::Divide,
        ScalarMathNodeMode::Acos,
        ScalarMathNodeMode::Acosh,
        ScalarMathNodeMode::Asin,
        ScalarMathNodeMode::Asinh,
        ScalarMathNodeMode::Atan,
        ScalarMathNodeMode::Atanh,
        ScalarMathNodeMode::Ceil,
        ScalarMathNodeMode::Cos,
        ScalarMathNodeMode::Cosh,
        ScalarMathNodeMode::Degrees,
        ScalarMathNodeMode::Exp,
        ScalarMathNodeMode::Exp2,
        ScalarMathNodeMode::Floor,
        ScalarMathNodeMode::Fract,
        ScalarMathNodeMode::InverseSqrt,
        ScalarMathNodeMode::Ln,
        ScalarMathNodeMode::Log2,
        ScalarMathNodeMode::Max,
        ScalarMathNodeMode::Min,
        ScalarMathNodeMode::Mix,
        ScalarMathNodeMode::Pow,
        ScalarMathNodeMode::Radians,
        ScalarMathNodeMode::Round,
        ScalarMathNodeMode::Saturate,
        ScalarMathNodeMode::Sign,
        ScalarMathNodeMode::Sin,
        ScalarMathNodeMode::Sinh,
        ScalarMathNodeMode::Sqrt,
        ScalarMathNodeMode::Tan,
        ScalarMathNodeMode::Tanh,
        ScalarMathNodeMode::Trunc,
    ];
}

/// UI message emitted by [`ScalarMathNode`].
#[derive(Clone)]
pub enum ScalarMathNodeMessage {
    ModeChanged(ScalarMathNodeMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for ScalarMathNode {
    type State = ScalarMathNodeMode;
    type Message = ScalarMathNodeMessage;

    fn id(&self) -> &'static str {
        "scalar_math_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        ScalarMathNodeMode::Add
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ScalarMathNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        match state {
            ScalarMathNodeMode::Add | ScalarMathNodeMode::Max | ScalarMathNodeMode::Min => vec![
                GraphDefaultInputSlot::new::<F32Type>("a".into()),
                GraphDefaultInputSlot::new::<F32Type>("b".into()),
            ],
            ScalarMathNodeMode::Subtract => vec![
                GraphDefaultInputSlot::new::<F32Type>("minuend".into()),
                GraphDefaultInputSlot::new::<F32Type>("subtrahend".into()),
            ],
            ScalarMathNodeMode::Multiply => vec![
                GraphDefaultInputSlot::new::<F32Type>("a".into()),
                GraphDefaultInputSlot::new::<F32Type>("b".into()),
            ],
            ScalarMathNodeMode::Divide => vec![
                GraphDefaultInputSlot::new::<F32Type>("dividend".into()),
                GraphDefaultInputSlot::new::<F32Type>("divisor".into()),
            ],
            ScalarMathNodeMode::Pow => vec![
                GraphDefaultInputSlot::new::<F32Type>("base".into()),
                GraphDefaultInputSlot::new::<F32Type>("exponent".into()),
            ],
            ScalarMathNodeMode::Acosh => {
                vec![GraphDefaultInputSlot::new::<F32Type>("x".into())]
            }
            ScalarMathNodeMode::Mix => vec![
                GraphDefaultInputSlot::new::<F32Type>("a".into()),
                GraphDefaultInputSlot::new::<F32Type>("b".into()),
                GraphDefaultInputSlot::new::<F32Type>("factor".into()),
            ],
            ScalarMathNodeMode::Ln
            | ScalarMathNodeMode::Log2
            | ScalarMathNodeMode::Sqrt
            | ScalarMathNodeMode::InverseSqrt => {
                vec![GraphDefaultInputSlot::new::<F32Type>("x".into())]
            }
            ScalarMathNodeMode::Acos
            | ScalarMathNodeMode::Asin
            | ScalarMathNodeMode::Asinh
            | ScalarMathNodeMode::Atan
            | ScalarMathNodeMode::Atanh
            | ScalarMathNodeMode::Ceil
            | ScalarMathNodeMode::Cos
            | ScalarMathNodeMode::Cosh
            | ScalarMathNodeMode::Degrees
            | ScalarMathNodeMode::Exp
            | ScalarMathNodeMode::Exp2
            | ScalarMathNodeMode::Floor
            | ScalarMathNodeMode::Fract
            | ScalarMathNodeMode::Radians
            | ScalarMathNodeMode::Round
            | ScalarMathNodeMode::Saturate
            | ScalarMathNodeMode::Sign
            | ScalarMathNodeMode::Sin
            | ScalarMathNodeMode::Sinh
            | ScalarMathNodeMode::Tan
            | ScalarMathNodeMode::Tanh
            | ScalarMathNodeMode::Trunc => {
                vec![GraphDefaultInputSlot::new::<F32Type>("x".into())]
            }
        }
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            combo_box(
                ScalarMathNodeMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(Translated)
                    .collect::<Vec<_>>(),
                Some(Translated(*state)),
                |option| ScalarMathNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(Length::Fill),
            ScalarMathNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            ScalarMathNodeMessage::ModeChanged(mode) => *state = mode,
            ScalarMathNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let a = ctx.get_input(0)?;
        let b = ctx.get_input(1);
        let c = ctx.get_input(2);

        let expression = match state {
            ScalarMathNodeMode::Add => {
                let b = b?;
                quote_expression! { #a + #b }
            }
            ScalarMathNodeMode::Subtract => {
                let b = b?;
                quote_expression! { #a - #b }
            }
            ScalarMathNodeMode::Multiply => {
                let b = b?;
                quote_expression! { #a * #b }
            }
            ScalarMathNodeMode::Divide => {
                let b = b?;
                quote_expression! { #a / #b }
            }
            ScalarMathNodeMode::Acos => quote_expression! { acos(#a) },
            ScalarMathNodeMode::Acosh => quote_expression! { acosh(#a) },
            ScalarMathNodeMode::Asin => quote_expression! { asin(#a) },
            ScalarMathNodeMode::Asinh => quote_expression! { asinh(#a) },
            ScalarMathNodeMode::Atan => quote_expression! { atan(#a) },
            ScalarMathNodeMode::Atanh => quote_expression! { atanh(#a) },
            ScalarMathNodeMode::Ceil => quote_expression! { ceil(#a) },
            ScalarMathNodeMode::Cos => quote_expression! { cos(#a) },
            ScalarMathNodeMode::Cosh => quote_expression! { cosh(#a) },
            ScalarMathNodeMode::Degrees => quote_expression! { degrees(#a) },
            ScalarMathNodeMode::Exp => quote_expression! { exp(#a) },
            ScalarMathNodeMode::Exp2 => quote_expression! { exp2(#a) },
            ScalarMathNodeMode::Floor => quote_expression! { floor(#a) },
            ScalarMathNodeMode::Fract => quote_expression! { fract(#a) },
            ScalarMathNodeMode::InverseSqrt => quote_expression! { inverseSqrt(#a) },
            ScalarMathNodeMode::Ln => quote_expression! { log(#a) },
            ScalarMathNodeMode::Log2 => quote_expression! { log2(#a) },
            ScalarMathNodeMode::Max => {
                let b = b?;
                quote_expression! { max(#a, #b) }
            }
            ScalarMathNodeMode::Min => {
                let b = b?;
                quote_expression! { min(#a, #b) }
            }
            ScalarMathNodeMode::Mix => {
                let (b, c) = (b?, c?);
                quote_expression! { mix(#a, #b, #c) }
            }
            ScalarMathNodeMode::Pow => {
                let b = b?;
                quote_expression! { pow(#a, #b) }
            }
            ScalarMathNodeMode::Radians => quote_expression! { radians(#a) },
            ScalarMathNodeMode::Round => quote_expression! { round(#a) },
            ScalarMathNodeMode::Saturate => quote_expression! { saturate(#a) },
            ScalarMathNodeMode::Sign => quote_expression! { sign(#a) },
            ScalarMathNodeMode::Sin => quote_expression! { sin(#a) },
            ScalarMathNodeMode::Sinh => quote_expression! { sinh(#a) },
            ScalarMathNodeMode::Sqrt => quote_expression! { sqrt(#a) },
            ScalarMathNodeMode::Tan => quote_expression! { tan(#a) },
            ScalarMathNodeMode::Tanh => quote_expression! { tanh(#a) },
            ScalarMathNodeMode::Trunc => quote_expression! { trunc(#a) },
        };
        let output = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            quote_statement! { let #output = #expression; }
        ))
    }
}

/// Shader graph node that performs a configurable mathematical or geometric operation on 2D vectors ([`Vec2FType`]).
#[derive(Default, Clone)]
pub struct VectorMathNode;

/// Mathematical operation mode for [`VectorMathNode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[display(style = "snake_case")]
pub enum VectorMathNodeMode {
    Add,
    Subtract,
    Multiply,
    Divide,
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Ceil,
    Cos,
    Cosh,
    Degrees,
    Distance,
    Dot,
    Exp,
    Exp2,
    Floor,
    Fract,
    InverseSqrt,
    Ln,
    Length,
    Log2,
    Max,
    Min,
    Mix,
    Pow,
    Radians,
    Reflect,
    Round,
    Saturate,
    Sign,
    Sin,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
    Trunc,
}

impl VectorMathNodeMode {
    pub const ALL: [VectorMathNodeMode; 39] = [
        VectorMathNodeMode::Add,
        VectorMathNodeMode::Subtract,
        VectorMathNodeMode::Multiply,
        VectorMathNodeMode::Divide,
        VectorMathNodeMode::Acos,
        VectorMathNodeMode::Acosh,
        VectorMathNodeMode::Asin,
        VectorMathNodeMode::Asinh,
        VectorMathNodeMode::Atan,
        VectorMathNodeMode::Atanh,
        VectorMathNodeMode::Ceil,
        VectorMathNodeMode::Cos,
        VectorMathNodeMode::Cosh,
        VectorMathNodeMode::Degrees,
        VectorMathNodeMode::Distance,
        VectorMathNodeMode::Dot,
        VectorMathNodeMode::Exp,
        VectorMathNodeMode::Exp2,
        VectorMathNodeMode::Floor,
        VectorMathNodeMode::Fract,
        VectorMathNodeMode::InverseSqrt,
        VectorMathNodeMode::Ln,
        VectorMathNodeMode::Length,
        VectorMathNodeMode::Log2,
        VectorMathNodeMode::Max,
        VectorMathNodeMode::Min,
        VectorMathNodeMode::Mix,
        VectorMathNodeMode::Pow,
        VectorMathNodeMode::Radians,
        VectorMathNodeMode::Reflect,
        VectorMathNodeMode::Round,
        VectorMathNodeMode::Saturate,
        VectorMathNodeMode::Sign,
        VectorMathNodeMode::Sin,
        VectorMathNodeMode::Sinh,
        VectorMathNodeMode::Sqrt,
        VectorMathNodeMode::Tan,
        VectorMathNodeMode::Tanh,
        VectorMathNodeMode::Trunc,
    ];
}

/// UI message emitted by [`VectorMathNode`].
#[derive(Clone)]
pub enum VectorMathNodeMessage {
    ModeChanged(VectorMathNodeMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for VectorMathNode {
    type State = VectorMathNodeMode;
    type Message = VectorMathNodeMessage;

    fn id(&self) -> &'static str {
        "vector_math_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        VectorMathNodeMode::Add
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(VectorMathNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        match state {
            VectorMathNodeMode::Add | VectorMathNodeMode::Max | VectorMathNodeMode::Min => vec![
                GraphDefaultInputSlot::new::<Vec2FType>("a".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("b".into()),
            ],
            VectorMathNodeMode::Subtract => vec![
                GraphDefaultInputSlot::new::<Vec2FType>("minuend".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("subtrahend".into()),
            ],
            VectorMathNodeMode::Multiply => vec![
                GraphDefaultInputSlot::new::<Vec2FType>("a".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("b".into()),
            ],
            VectorMathNodeMode::Divide => vec![
                GraphDefaultInputSlot::new::<Vec2FType>("dividend".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("divisor".into()),
            ],
            VectorMathNodeMode::Pow => vec![
                GraphDefaultInputSlot::new::<Vec2FType>("base".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("exponent".into()),
            ],
            VectorMathNodeMode::Distance | VectorMathNodeMode::Dot => vec![
                GraphDefaultInputSlot::new::<Vec2FType>("a".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("b".into()),
            ],
            VectorMathNodeMode::Reflect => vec![
                GraphDefaultInputSlot::new::<Vec2FType>("incident".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("normal".into()),
            ],
            VectorMathNodeMode::Mix => vec![
                GraphDefaultInputSlot::new::<Vec2FType>("a".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("b".into()),
                GraphDefaultInputSlot::new::<Vec2FType>("factor".into()),
            ],
            VectorMathNodeMode::Acosh => {
                vec![GraphDefaultInputSlot::new::<Vec2FType>("x".into())]
            }
            VectorMathNodeMode::Ln
            | VectorMathNodeMode::Log2
            | VectorMathNodeMode::Sqrt
            | VectorMathNodeMode::InverseSqrt => {
                vec![GraphDefaultInputSlot::new::<Vec2FType>("x".into())]
            }
            VectorMathNodeMode::Length => {
                vec![GraphDefaultInputSlot::new::<Vec2FType>("vector".into())]
            }
            VectorMathNodeMode::Acos
            | VectorMathNodeMode::Asin
            | VectorMathNodeMode::Asinh
            | VectorMathNodeMode::Atan
            | VectorMathNodeMode::Atanh
            | VectorMathNodeMode::Ceil
            | VectorMathNodeMode::Cos
            | VectorMathNodeMode::Cosh
            | VectorMathNodeMode::Degrees
            | VectorMathNodeMode::Exp
            | VectorMathNodeMode::Exp2
            | VectorMathNodeMode::Floor
            | VectorMathNodeMode::Fract
            | VectorMathNodeMode::Radians
            | VectorMathNodeMode::Round
            | VectorMathNodeMode::Saturate
            | VectorMathNodeMode::Sign
            | VectorMathNodeMode::Sin
            | VectorMathNodeMode::Sinh
            | VectorMathNodeMode::Tan
            | VectorMathNodeMode::Tanh
            | VectorMathNodeMode::Trunc => {
                vec![GraphDefaultInputSlot::new::<Vec2FType>("x".into())]
            }
        }
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        match state {
            VectorMathNodeMode::Add
            | VectorMathNodeMode::Subtract
            | VectorMathNodeMode::Multiply
            | VectorMathNodeMode::Divide
            | VectorMathNodeMode::Acos
            | VectorMathNodeMode::Acosh
            | VectorMathNodeMode::Asin
            | VectorMathNodeMode::Asinh
            | VectorMathNodeMode::Atan
            | VectorMathNodeMode::Atanh
            | VectorMathNodeMode::Ceil
            | VectorMathNodeMode::Cos
            | VectorMathNodeMode::Cosh
            | VectorMathNodeMode::Degrees
            | VectorMathNodeMode::Exp
            | VectorMathNodeMode::Exp2
            | VectorMathNodeMode::Floor
            | VectorMathNodeMode::Fract
            | VectorMathNodeMode::InverseSqrt
            | VectorMathNodeMode::Ln
            | VectorMathNodeMode::Log2
            | VectorMathNodeMode::Max
            | VectorMathNodeMode::Min
            | VectorMathNodeMode::Mix
            | VectorMathNodeMode::Pow
            | VectorMathNodeMode::Radians
            | VectorMathNodeMode::Reflect
            | VectorMathNodeMode::Round
            | VectorMathNodeMode::Saturate
            | VectorMathNodeMode::Sign
            | VectorMathNodeMode::Sin
            | VectorMathNodeMode::Sinh
            | VectorMathNodeMode::Sqrt
            | VectorMathNodeMode::Tan
            | VectorMathNodeMode::Tanh
            | VectorMathNodeMode::Trunc => {
                vec![GraphDefaultOutputSlot::new::<Vec2FType>("result".into())]
            }

            VectorMathNodeMode::Dot | VectorMathNodeMode::Distance | VectorMathNodeMode::Length => {
                vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
            }
        }
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            combo_box(
                VectorMathNodeMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(Translated)
                    .collect::<Vec<_>>(),
                Some(Translated(*state)),
                |option| VectorMathNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(Length::Fill),
            VectorMathNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            VectorMathNodeMessage::ModeChanged(mode) => *state = mode,
            VectorMathNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let a = ctx.get_input(0)?;
        let b = ctx.get_input(1);
        let c = ctx.get_input(2);
        let output = ctx.get_output(0)?;

        let expression = match state {
            VectorMathNodeMode::Add => {
                let b = b?;
                quote_expression! { #a + #b }
            }
            VectorMathNodeMode::Subtract => {
                let b = b?;
                quote_expression! { #a - #b }
            }
            VectorMathNodeMode::Multiply => {
                let b = b?;
                quote_expression! { #a * #b }
            }
            VectorMathNodeMode::Divide => {
                let b = b?;
                quote_expression! { #a / #b }
            }
            VectorMathNodeMode::Acos => quote_expression! { acos(#a) },
            VectorMathNodeMode::Acosh => quote_expression! { acosh(#a) },
            VectorMathNodeMode::Asin => quote_expression! { asin(#a) },
            VectorMathNodeMode::Asinh => quote_expression! { asinh(#a) },
            VectorMathNodeMode::Atan => quote_expression! { atan(#a) },
            VectorMathNodeMode::Atanh => quote_expression! { atanh(#a) },
            VectorMathNodeMode::Ceil => quote_expression! { ceil(#a) },
            VectorMathNodeMode::Cos => quote_expression! { cos(#a) },
            VectorMathNodeMode::Cosh => quote_expression! { cosh(#a) },
            VectorMathNodeMode::Degrees => quote_expression! { degrees(#a) },
            VectorMathNodeMode::Distance => {
                let b = b?;
                quote_expression! { distance(#a, #b) }
            }
            VectorMathNodeMode::Dot => {
                let b = b?;
                quote_expression! { dot(#a, #b) }
            }
            VectorMathNodeMode::Exp => quote_expression! { exp(#a) },
            VectorMathNodeMode::Exp2 => quote_expression! { exp2(#a) },
            VectorMathNodeMode::Floor => quote_expression! { floor(#a) },
            VectorMathNodeMode::Fract => quote_expression! { fract(#a) },
            VectorMathNodeMode::InverseSqrt => quote_expression! { inverseSqrt(#a) },
            VectorMathNodeMode::Ln => quote_expression! { log(#a) },
            VectorMathNodeMode::Length => quote_expression! { length(#a) },
            VectorMathNodeMode::Log2 => quote_expression! { log2(#a) },
            VectorMathNodeMode::Max => {
                let b = b?;
                quote_expression! { max(#a, #b) }
            }
            VectorMathNodeMode::Min => {
                let b = b?;
                quote_expression! { min(#a, #b) }
            }
            VectorMathNodeMode::Mix => {
                let (b, c) = (b?, c?);
                quote_expression! { mix(#a, #b, #c) }
            }
            VectorMathNodeMode::Pow => {
                let b = b?;
                quote_expression! { pow(#a, #b) }
            }
            VectorMathNodeMode::Radians => quote_expression! { radians(#a) },
            VectorMathNodeMode::Reflect => {
                let b = b?;
                quote_expression! { reflect(#a, #b) }
            }
            VectorMathNodeMode::Round => quote_expression! { round(#a) },
            VectorMathNodeMode::Saturate => quote_expression! { saturate(#a) },
            VectorMathNodeMode::Sign => quote_expression! { sign(#a) },
            VectorMathNodeMode::Sin => quote_expression! { sin(#a) },
            VectorMathNodeMode::Sinh => quote_expression! { sinh(#a) },
            VectorMathNodeMode::Sqrt => quote_expression! { sqrt(#a) },
            VectorMathNodeMode::Tan => quote_expression! { tan(#a) },
            VectorMathNodeMode::Tanh => quote_expression! { tanh(#a) },
            VectorMathNodeMode::Trunc => quote_expression! { trunc(#a) },
        };

        Ok(format!(
            "{}\n",
            quote_statement! { let #output = #expression; }
        ))
    }
}

/// Shader graph node that computes union, intersection, inflation, or shrinking on [`RectType`] bounding boxes.
#[derive(Default, Clone)]
pub struct RectMathNode;

/// Geometric operation mode for [`RectMathNode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display)]
#[display(style = "snake_case")]
pub enum RectMathNodeMode {
    Union,
    Intersection,
    Inflate,
    Shrink,
}

impl RectMathNodeMode {
    pub const ALL: [RectMathNodeMode; 4] = [
        RectMathNodeMode::Union,
        RectMathNodeMode::Intersection,
        RectMathNodeMode::Inflate,
        RectMathNodeMode::Shrink,
    ];
}

/// UI message emitted by [`RectMathNode`].
#[derive(Clone)]
pub enum RectMathNodeMessage {
    ModeChanged(RectMathNodeMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for RectMathNode {
    type State = RectMathNodeMode;
    type Message = RectMathNodeMessage;

    fn id(&self) -> &'static str {
        "rect_math_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        RectMathNodeMode::Union
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RectMathNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        match state {
            RectMathNodeMode::Union | RectMathNodeMode::Intersection => vec![
                GraphDefaultInputSlot::new::<RectType>("a".into()),
                GraphDefaultInputSlot::new::<RectType>("b".into()),
            ],
            RectMathNodeMode::Inflate | RectMathNodeMode::Shrink => {
                vec![
                    GraphDefaultInputSlot::new::<RectType>("rect".into()),
                    GraphDefaultInputSlot::new::<Vec2FType>("mix_amount".into()),
                ]
            }
        }
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<RectType>("result".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            combo_box(
                RectMathNodeMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(Translated)
                    .collect::<Vec<_>>(),
                Some(Translated(*state)),
                |option| RectMathNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(Length::Fill),
            RectMathNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            RectMathNodeMessage::ModeChanged(mode) => *state = mode,
            RectMathNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let a = ctx.get_input(0)?;
        let b = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;

        let code = match state {
            RectMathNodeMode::Union => quote_statement! {
                let #output = render::math::Rect(min(#a.min, #b.min), max(#a.max, #b.max));
            }
            .to_string(),
            RectMathNodeMode::Intersection => quote_statement! {
                let #output = render::math::Rect(max(#a.min, #b.min), min(#a.max, #b.max));
            }
            .to_string(),
            RectMathNodeMode::Inflate => {
                let min = Ident::new(ctx.ident_generator.next_output());
                let max = Ident::new(ctx.ident_generator.next_output());
                [
                    quote_statement! { let #min = #a.min - #b; }.to_string(),
                    quote_statement! { let #max = #a.max + #b; }.to_string(),
                    quote_statement! { var #output = render::math::Rect(#min, #max); }.to_string(),
                    quote_statement! {
                        if any(#min > #max) {
                            #output = render::math::Rect(vec2f(1.0, -1.0));
                        }
                    }
                    .to_string(),
                ]
                .join("\n")
            }
            RectMathNodeMode::Shrink => {
                let min = Ident::new(ctx.ident_generator.next_output());
                let max = Ident::new(ctx.ident_generator.next_output());
                [
                    quote_statement! { let #min = #a.min + #b; }.to_string(),
                    quote_statement! { let #max = #a.max - #b; }.to_string(),
                    quote_statement! { var #output = render::math::Rect(#min, #max); }.to_string(),
                    quote_statement! {
                        if any(#min > #max) {
                            #output = render::math::Rect(vec2f(1.0, -1.0));
                        }
                    }
                    .to_string(),
                ]
                .join("\n")
            }
        };

        Ok(format!("{code}\n"))
    }
}

/// Shader graph node that restricts a scalar value ([`F32Type`]) to a `[min, max]` range using WGSL `clamp`.
#[derive(Default, Clone)]
pub struct ClampNode;

#[stateless]
impl StatelessCommonGraphNode for ClampNode {
    fn id(&self) -> &'static str {
        "clamp_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ClampNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("value".into()),
            GraphDefaultInputSlot::new::<F32Type>("min".into()),
            GraphDefaultInputSlot::new::<F32Type>("max".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_value = ctx.get_input(0)?;
        let input_min = ctx.get_input(1)?;
        let input_max = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            quote_statement! { let #output = clamp(#input_value, #input_min, #input_max); }
        ))
    }
}

/// Shader graph node that evaluates the WGSL `step(edge, x)` threshold function on [`F32Type`] scalars.
#[derive(Default, Clone)]
pub struct StepNode;

#[stateless]
impl StatelessCommonGraphNode for StepNode {
    fn id(&self) -> &'static str {
        "step_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(StepNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("edge".into()),
            GraphDefaultInputSlot::new::<F32Type>("x".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_edge = ctx.get_input(0)?;
        let input_x = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            quote_statement! { let #output = step(#input_edge, #input_x); }
        ))
    }
}

/// Shader graph node that evaluates Hermite interpolation between two thresholds using WGSL `smoothstep`.
#[derive(Default, Clone)]
pub struct SmoothStepNode;

#[stateless]
impl StatelessCommonGraphNode for SmoothStepNode {
    fn id(&self) -> &'static str {
        "smooth_step_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SmoothStepNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("edge0".into()),
            GraphDefaultInputSlot::new::<F32Type>("edge1".into()),
            GraphDefaultInputSlot::new::<F32Type>("x".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_edge0 = ctx.get_input(0)?;
        let input_edge1 = ctx.get_input(1)?;
        let input_x = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            quote_statement! { let #output = smoothstep(#input_edge0, #input_edge1, #input_x); }
        ))
    }
}
