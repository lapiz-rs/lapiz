use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use anyhow::anyhow;

use glam::{Vec2, Vec3, Vec3Swizzles};
use iced_core::{
    Clipboard, Event, Layout, Length, Rectangle, Shell, Size, Widget, layout, mouse, renderer,
    widget::{Operation, Tree, tree},
};
use iced_widget::{column, container, row, text, text_editor, text_input};
use indexmap::IndexMap;
use lapiz_i18n::{Translated, t};
use lapiz_math::curve::CubicCurve;
use lapiz_utils::{random_oklch_hue_chroma, wrapper};
use lapiz_widgets::{
    button::Button, combo_box::ComboBox, curve_edit::CurveEdit, fluent_builder::When, label::Label,
    popover::Popover,
};
use parking_lot::Mutex;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn button<'a, Message: 'a>(label: impl Into<String>) -> Button<'a, Message> {
    Button::new(Label::new(label.into()))
}

use crate::{
    GraphElement, GraphRenderer, GraphTheme,
    graph::{
        Graph, GraphData, GraphResources, GraphVarIdentGenerator,
        external::{ExternalVariableId, generate_external_variable_name},
        function::GraphFunctionId,
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeRegistry, GraphNodeUpdateContext,
            GraphNodeUpdateSignatureContext, GraphNodeViewContext, StatelessCommonGraphNode,
        },
        slot::{
            ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphDefaultInputSlot,
            GraphDefaultOutputSlot, GraphValueType,
        },
        texture::TextureId,
    },
    save::{GraphSerializable, SerializableGraph},
    wgsl_std::types::{BoolType, ColorType, F32Type, I32Type, RectType, TextureType, Vec2FType},
};

use lapiz_shader_graph_derive::stateless;

#[derive(Default, Clone)]
pub struct ScalarMathNode;

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

#[derive(Clone)]
pub enum ScalarMathNodeMessage {
    ModeChanged(ScalarMathNodeMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for ScalarMathNode {
    type State = ScalarMathNodeMode;
    type Message = ScalarMathNodeMessage;

    fn id(&self) -> &'static str {
        "scalar_math_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        ScalarMathNodeMode::Add
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ScalarMathNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
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
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
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
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            ScalarMathNodeMessage::ModeChanged(mode) => *state = mode,
            ScalarMathNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let a = ctx.get_input(0);
        let b = ctx.get_input(1);
        let c = ctx.get_input(2);

        let expression = match state {
            ScalarMathNodeMode::Add => format!("{} + {}", a?, b?),
            ScalarMathNodeMode::Subtract => format!("{} - {}", a?, b?),
            ScalarMathNodeMode::Multiply => format!("{} * {}", a?, b?),
            ScalarMathNodeMode::Divide => format!("{} / {}", a?, b?),
            ScalarMathNodeMode::Acos => format!("acos({})", a?),
            ScalarMathNodeMode::Acosh => format!("acosh({})", a?),
            ScalarMathNodeMode::Asin => format!("asin({})", a?),
            ScalarMathNodeMode::Asinh => format!("asinh({})", a?),
            ScalarMathNodeMode::Atan => format!("atan({})", a?),
            ScalarMathNodeMode::Atanh => format!("atanh({})", a?),
            ScalarMathNodeMode::Ceil => format!("ceil({})", a?),
            ScalarMathNodeMode::Cos => format!("cos({})", a?),
            ScalarMathNodeMode::Cosh => format!("cosh({})", a?),
            ScalarMathNodeMode::Degrees => format!("degrees({})", a?),
            ScalarMathNodeMode::Exp => format!("exp({})", a?),
            ScalarMathNodeMode::Exp2 => format!("exp2({})", a?),
            ScalarMathNodeMode::Floor => format!("floor({})", a?),
            ScalarMathNodeMode::Fract => format!("fract({})", a?),
            ScalarMathNodeMode::InverseSqrt => format!("inverseSqrt({})", a?),
            ScalarMathNodeMode::Ln => format!("log({})", a?),
            ScalarMathNodeMode::Log2 => format!("log2({})", a?),
            ScalarMathNodeMode::Max => format!("max({}, {})", a?, b?),
            ScalarMathNodeMode::Min => format!("min({}, {})", a?, b?),
            ScalarMathNodeMode::Mix => format!("mix({}, {}, {})", a?, b?, c?),
            ScalarMathNodeMode::Pow => format!("pow({}, {})", a?, b?),
            ScalarMathNodeMode::Radians => format!("radians({})", a?),
            ScalarMathNodeMode::Round => format!("round({})", a?),
            ScalarMathNodeMode::Saturate => format!("saturate({})", a?),
            ScalarMathNodeMode::Sign => format!("sign({})", a?),
            ScalarMathNodeMode::Sin => format!("sin({})", a?),
            ScalarMathNodeMode::Sinh => format!("sinh({})", a?),
            ScalarMathNodeMode::Sqrt => format!("sqrt({})", a?),
            ScalarMathNodeMode::Tan => format!("tan({})", a?),
            ScalarMathNodeMode::Tanh => format!("tanh({})", a?),
            ScalarMathNodeMode::Trunc => format!("trunc({})", a?),
        };
        let output = ctx.get_output(0)?;

        Ok(format!("let {} = {};\n", output, expression))
    }
}

#[derive(Default, Clone)]
pub struct VectorMathNode;

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

#[derive(Clone)]
pub enum VectorMathNodeMessage {
    ModeChanged(VectorMathNodeMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for VectorMathNode {
    type State = VectorMathNodeMode;
    type Message = VectorMathNodeMessage;

    fn id(&self) -> &'static str {
        "vector_math_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        VectorMathNodeMode::Add
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(VectorMathNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
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
        _: GraphNodeCreateSlotsContext<'_, Data>,
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
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
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
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            VectorMathNodeMessage::ModeChanged(mode) => *state = mode,
            VectorMathNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let a = ctx.get_input(0);
        let b = ctx.get_input(1);
        let c = ctx.get_input(2);
        let output = ctx.get_output(0)?;

        Ok(format!(
            "let {} = {};\n",
            output,
            match state {
                VectorMathNodeMode::Add => format!("{} + {}", a?, b?),
                VectorMathNodeMode::Subtract => format!("{} - {}", a?, b?),
                VectorMathNodeMode::Multiply => format!("{} * {}", a?, b?),
                VectorMathNodeMode::Divide => format!("{} / {}", a?, b?),
                VectorMathNodeMode::Acos => format!("acos({})", a?),
                VectorMathNodeMode::Acosh => format!("acosh({})", a?),
                VectorMathNodeMode::Asin => format!("asin({})", a?),
                VectorMathNodeMode::Asinh => format!("asinh({})", a?),
                VectorMathNodeMode::Atan => format!("atan({})", a?),
                VectorMathNodeMode::Atanh => format!("atanh({})", a?),
                VectorMathNodeMode::Ceil => format!("ceil({})", a?),
                VectorMathNodeMode::Cos => format!("cos({})", a?),
                VectorMathNodeMode::Cosh => format!("cosh({})", a?),
                VectorMathNodeMode::Degrees => format!("degrees({})", a?),
                VectorMathNodeMode::Distance => format!("distance({}, {})", a?, b?),
                VectorMathNodeMode::Dot => format!("dot({}, {})", a?, b?),
                VectorMathNodeMode::Exp => format!("exp({})", a?),
                VectorMathNodeMode::Exp2 => format!("exp2({})", a?),
                VectorMathNodeMode::Floor => format!("floor({})", a?),
                VectorMathNodeMode::Fract => format!("fract({})", a?),
                VectorMathNodeMode::InverseSqrt => format!("inverseSqrt({})", a?),
                VectorMathNodeMode::Ln => format!("log({})", a?),
                VectorMathNodeMode::Length => format!("length({})", a?),
                VectorMathNodeMode::Log2 => format!("log2({})", a?),
                VectorMathNodeMode::Max => format!("max({}, {})", a?, b?),
                VectorMathNodeMode::Min => format!("min({}, {})", a?, b?),
                VectorMathNodeMode::Mix => format!("mix({}, {}, {})", a?, b?, c?),
                VectorMathNodeMode::Pow => format!("pow({}, {})", a?, b?),
                VectorMathNodeMode::Radians => format!("radians({})", a?),
                VectorMathNodeMode::Reflect => format!("reflect({}, {})", a?, b?),
                VectorMathNodeMode::Round => format!("round({})", a?),
                VectorMathNodeMode::Saturate => format!("saturate({})", a?),
                VectorMathNodeMode::Sign => format!("sign({})", a?),
                VectorMathNodeMode::Sin => format!("sin({})", a?),
                VectorMathNodeMode::Sinh => format!("sinh({})", a?),
                VectorMathNodeMode::Sqrt => format!("sqrt({})", a?),
                VectorMathNodeMode::Tan => format!("tan({})", a?),
                VectorMathNodeMode::Tanh => format!("tanh({})", a?),
                VectorMathNodeMode::Trunc => format!("trunc({})", a?),
            }
        ))
    }
}

#[derive(Default, Clone)]
pub struct RectMathNode;

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

#[derive(Clone)]
pub enum RectMathNodeMessage {
    ModeChanged(RectMathNodeMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for RectMathNode {
    type State = RectMathNodeMode;
    type Message = RectMathNodeMessage;

    fn id(&self) -> &'static str {
        "rect_math_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        RectMathNodeMode::Union
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RectMathNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
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
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<RectType>("result".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
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
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            RectMathNodeMessage::ModeChanged(mode) => *state = mode,
            RectMathNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let a = ctx.get_input(0)?;
        let b = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "let {} = {};\n",
            output,
            match state {
                RectMathNodeMode::Union => {
                    format!("Rect(min({}.min, {}.min), max({}.max, {}.max))", a, b, a, b)
                }
                RectMathNodeMode::Intersection => {
                    format!("Rect(max({}.min, {}.min), min({}.max, {}.max))", a, b, a, b)
                }
                RectMathNodeMode::Inflate => {
                    let min = ctx.ident_generator.next_output();
                    let max = ctx.ident_generator.next_output();
                    format!(
                        "
                        let {min} = {a}.min - {b};
                        let {max} = {a}.max + {b};
                        var {output} = Rect({min} , {max});
                        if any({min} > {max}) {{
                            {output} = Rect(vec2f(1.0, -1.0));
                        }}
                    ",
                    )
                }
                RectMathNodeMode::Shrink => {
                    let min = ctx.ident_generator.next_output();
                    let max = ctx.ident_generator.next_output();
                    format!(
                        "
                        let {min} = {a}.min + {b};
                        let {max} = {a}.max - {b};
                        var {output} = Rect({min} , {max});
                        if any({min} > {max}) {{
                            {output} = Rect(vec2f(1.0, -1.0));
                        }}
                    ",
                    )
                }
            }
        ))
    }
}

#[derive(Default, Clone)]
pub struct CompareNode;

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

#[derive(Clone)]
pub enum CompareNodeMessage {
    ModeChanged(CompareNodeMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for CompareNode {
    type State = CompareNodeMode;
    type Message = CompareNodeMessage;

    fn id(&self) -> &'static str {
        "compare_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        CompareNodeMode::LessThan
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CompareNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("lhs".into()),
            GraphDefaultInputSlot::new::<F32Type>("rhs".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<BoolType>("result".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
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
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            CompareNodeMessage::ModeChanged(mode) => *state = mode,
            CompareNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let lhs = ctx.get_input(0)?;
        let rhs = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;
        let operator = match state {
            CompareNodeMode::LessThan => "<",
            CompareNodeMode::LessEqual => "<=",
            CompareNodeMode::GreaterThan => ">",
            CompareNodeMode::GreaterEqual => ">=",
            CompareNodeMode::Equal => "==",
        };

        Ok(format!("let {} = {} {} {};\n", output, lhs, operator, rhs))
    }
}

#[derive(Default, Clone)]
pub struct ScalarSelectNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for ScalarSelectNode {
    fn id(&self) -> &'static str {
        "scalar_select_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ScalarSelectNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<BoolType>("condition".into()),
            GraphDefaultInputSlot::new::<F32Type>("false".into()),
            GraphDefaultInputSlot::new::<F32Type>("true".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let condition = ctx.get_input(0)?;
        let false_value = ctx.get_input(1)?;
        let true_value = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "let {} = select({}, {}, {});\n",
            output, false_value, true_value, condition
        ))
    }
}

#[derive(Default, Clone)]
pub struct VectorSelectNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for VectorSelectNode {
    fn id(&self) -> &'static str {
        "vector_select_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(VectorSelectNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<BoolType>("condition".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("false".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("true".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let condition = ctx.get_input(0)?;
        let false_value = ctx.get_input(1)?;
        let true_value = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "let {} = select({}, {}, {});\n",
            output, false_value, true_value, condition
        ))
    }
}

#[derive(Default, Clone)]
pub struct TimeNode;

pub struct GraphTimes {
    pub now: f32,
    pub stroke_begin: f32,
}

pub trait GraphDataWithTime: GraphData {
    fn time(&self) -> GraphTimes;
    fn wgsl_variable() -> String;
}

#[stateless]
impl<Data: GraphDataWithTime> StatelessCommonGraphNode<Data> for TimeNode {
    fn id(&self) -> &'static str {
        "time_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(TimeNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("now".into()),
            GraphDefaultOutputSlot::new::<F32Type>("stroke_begin".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let now = ctx.get_output(0)?;
        let stroke_begin = ctx.get_output(1)?;
        let accessor = Data::wgsl_variable();

        Ok(format!(
            "
let {} = {}.now;
let {} = {}.stroke_begin;
                ",
            now, accessor, stroke_begin, accessor
        ))
    }
}

#[derive(Default, Clone)]
pub struct ClampNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for ClampNode {
    fn id(&self) -> &'static str {
        "clamp_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ClampNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("value".into()),
            GraphDefaultInputSlot::new::<F32Type>("min".into()),
            GraphDefaultInputSlot::new::<F32Type>("max".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_value = ctx.get_input(0)?;
        let input_min = ctx.get_input(1)?;
        let input_max = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "let {} = clamp({}, {}, {});\n",
            output, input_value, input_min, input_max
        ))
    }
}

#[derive(Default, Clone)]
pub struct StepNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for StepNode {
    fn id(&self) -> &'static str {
        "step_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(StepNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("edge".into()),
            GraphDefaultInputSlot::new::<F32Type>("x".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_edge = ctx.get_input(0)?;
        let input_x = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "let {} = step({}, {});\n",
            output, input_edge, input_x
        ))
    }
}

#[derive(Default, Clone)]
pub struct SmoothStepNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for SmoothStepNode {
    fn id(&self) -> &'static str {
        "smooth_step_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SmoothStepNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("edge0".into()),
            GraphDefaultInputSlot::new::<F32Type>("edge1".into()),
            GraphDefaultInputSlot::new::<F32Type>("x".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_edge0 = ctx.get_input(0)?;
        let input_edge1 = ctx.get_input(1)?;
        let input_x = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "let {} = smoothstep({}, {}, {});\n",
            output, input_edge0, input_edge1, input_x
        ))
    }
}

#[derive(Default, Clone)]
pub struct SplitComponentsNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for SplitComponentsNode {
    fn id(&self) -> &'static str {
        "split_components_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SplitComponentsNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("vector".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("x".into()),
            GraphDefaultOutputSlot::new::<F32Type>("y".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_vector = ctx.get_input(0)?;
        let output_x = ctx.get_output(0)?;
        let output_y = ctx.get_output(1)?;

        Ok(format!(
            "let {} = {}.x;\nlet {} = {}.y;\n",
            output_x, input_vector, output_y, input_vector
        ))
    }
}

#[derive(Default, Clone)]
pub struct CombineComponentsNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for CombineComponentsNode {
    fn id(&self) -> &'static str {
        "combine_components_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CombineComponentsNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("x".into()),
            GraphDefaultInputSlot::new::<F32Type>("y".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("vector".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_x = ctx.get_input(0)?;
        let input_y = ctx.get_input(1)?;
        let output_vector = ctx.get_output(0)?;

        Ok(format!(
            "let {} = vec2f({}, {});\n",
            output_vector, input_x, input_y
        ))
    }
}

#[derive(Default, Clone)]
pub struct CombineColorComponentsNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for CombineColorComponentsNode {
    fn id(&self) -> &'static str {
        "combine_color_components_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CombineColorComponentsNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("r".into()),
            GraphDefaultInputSlot::new::<F32Type>("g".into()),
            GraphDefaultInputSlot::new::<F32Type>("b".into()),
            GraphDefaultInputSlot::new::<F32Type>("a".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_r = ctx.get_input(0)?;
        let input_g = ctx.get_input(1)?;
        let input_b = ctx.get_input(2)?;
        let input_a = ctx.get_input(3)?;
        let output_color = ctx.get_output(0)?;

        Ok(format!(
            "let {} = vec4f({}, {}, {}, {});\n",
            output_color, input_r, input_g, input_b, input_a
        ))
    }
}

#[derive(Default, Clone)]
pub struct SplitColorComponentsNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for SplitColorComponentsNode {
    fn id(&self) -> &'static str {
        "split_color_components_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SplitColorComponentsNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<ColorType>("color".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("r".into()),
            GraphDefaultOutputSlot::new::<F32Type>("g".into()),
            GraphDefaultOutputSlot::new::<F32Type>("b".into()),
            GraphDefaultOutputSlot::new::<F32Type>("a".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_color = ctx.get_input(0)?;
        let output_r = ctx.get_output(0)?;
        let output_g = ctx.get_output(1)?;
        let output_b = ctx.get_output(2)?;
        let output_a = ctx.get_output(3)?;

        Ok(format!(
            "let {} = {}.r;\nlet {} = {}.g;\nlet {} = {}.b;\nlet {} = {}.a;\n",
            output_r,
            input_color,
            output_g,
            input_color,
            output_b,
            input_color,
            output_a,
            input_color
        ))
    }
}

#[derive(Default, Clone)]
pub struct GetPixelColorNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for GetPixelColorNode {
    fn id(&self) -> &'static str {
        "get_pixel_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(GetPixelColorNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<TextureType>("texture".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("position".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_texture = ctx.get_input(0)?;
        let input_position = ctx.get_input(1)?;
        let output_color = ctx.get_output(0)?;

        Ok(format!(
            // TODO: sample_local_texture is only defined in `brush_template.wesl`
            "let {} = sample_local_texture({}, vec2u({}));\n",
            output_color, input_texture, input_position
        ))
    }
}

#[derive(Default, Clone)]
pub struct TextureNode;

#[derive(Clone)]
pub enum TextureNodeMessage {
    TextureChanged(TextureId),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for TextureNode {
    type State = TextureId;
    type Message = TextureNodeMessage;

    fn id(&self) -> &'static str {
        "texture_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        TextureId::NULL
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(TextureNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<TextureType>("texture".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        let texture_storage = ctx.resources.textures.load();
        let textures = texture_storage.all().values().cloned().collect::<Vec<_>>();
        let selected = textures
            .iter()
            .find(|texture| Some(texture.external_id) == **state)
            .cloned();
        ctx.view_all_slots_with_header(
            ComboBox::new(textures, selected, |texture| {
                TextureNodeMessage::TextureChanged(TextureId(Some(texture.external_id)))
            })
            .width(Length::Fill),
            TextureNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            TextureNodeMessage::TextureChanged(id) => *state = id,
            TextureNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        // It's the external user's responsibility to generate the correct texture binding.
        // The binding should be a texture binding_array. The index of each used texture in graph
        // is corresponding to array index returned by TextureStorage::used_textures()
        let index = ctx.texture_usage.use_texture(*state);
        Ok(format!("let {} = {}u;\n", ctx.get_output(0)?, index))
    }
}

// TODO: Mixing in different color spaces.
#[derive(Default, Clone)]
pub struct ColorMixNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for ColorMixNode {
    fn id(&self) -> &'static str {
        "color_mix_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ColorMixNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("color_a".into()),
            GraphDefaultInputSlot::new::<ColorType>("color_b".into()),
            GraphDefaultInputSlot::new::<F32Type>("factor".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("result".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_color_a = ctx.get_input(0)?;
        let input_color_b = ctx.get_input(1)?;
        let input_factor = ctx.get_input(2)?;
        let output = ctx.get_output(0)?;

        Ok(format!(
            "let {} = mix({}, {}, {});\n",
            output, input_color_a, input_color_b, input_factor
        ))
    }
}

#[derive(Default, Clone)]
pub struct TextureSizeNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for TextureSizeNode {
    fn id(&self) -> &'static str {
        "texture_size_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(TextureSizeNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<TextureType>("texture".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("size".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_texture = ctx.get_input(0)?;
        let output_size = ctx.get_output(0)?;

        Ok(format!(
            "let {} = vec2f(texture_bounds[{}].max - texture_bounds[{}].min);\n",
            output_size, input_texture, input_texture
        ))
    }
}

static UNIQUE_COUNTER: AtomicU32 = AtomicU32::new(0);

#[derive(Default, Clone)]
pub struct GraphFunctionNode;

#[derive(Clone)]
pub struct GraphFunctionReference {
    pub id: GraphFunctionId,
    pub name: String,
}

impl std::fmt::Display for GraphFunctionReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

impl PartialEq for GraphFunctionReference {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Serialize, Deserialize)]
pub struct GraphFunctionNodeState {
    pub id: Option<GraphFunctionId>,
}

#[derive(Clone)]
pub enum GraphFunctionNodeMessage {
    FunctionChanged(GraphFunctionId),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for GraphFunctionNode {
    type State = GraphFunctionNodeState;
    type Message = GraphFunctionNodeMessage;

    fn id(&self) -> &'static str {
        "function_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        GraphFunctionNodeState { id: None }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(GraphFunctionNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        let functions = ctx.resources.functions.load();
        let Some(func) = state.id.as_ref().and_then(|id| functions.get(id)) else {
            return Vec::new();
        };

        func.graph
            .signature()
            .inputs
            .iter()
            .map(|(_, var)| {
                GraphDefaultInputSlot::new_boxed(
                    var.identifier().to_string(),
                    dyn_clone::clone_box(var.ty()),
                )
            })
            .collect()
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        let functions = ctx.resources.functions.load();
        let Some(func) = state.id.as_ref().and_then(|id| functions.get(id)) else {
            return Vec::new();
        };

        func.graph
            .signature()
            .outputs
            .iter()
            .map(|(_, var)| {
                GraphDefaultOutputSlot::new_boxed(
                    var.identifier().to_string(),
                    dyn_clone::clone_box(var.ty()),
                )
            })
            .collect()
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        let function_storage = ctx.resources.functions.load();
        let functions = function_storage
            .all()
            .iter()
            .map(|(id, graph)| GraphFunctionReference {
                id: *id,
                name: graph.name.clone(),
            })
            .collect::<Vec<_>>();
        let selected = functions
            .iter()
            .find(|reference| Some(reference.id) == state.id)
            .cloned();
        ctx.view_all_slots_with_header(
            ComboBox::new(functions, selected, |reference| {
                GraphFunctionNodeMessage::FunctionChanged(reference.id)
            })
            .width(Length::Fill),
            GraphFunctionNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            GraphFunctionNodeMessage::FunctionChanged(id) => state.id = Some(id),
            GraphFunctionNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let Some(id) = state.id.as_ref() else {
            return Ok(Default::default());
        };
        let functions = ctx.resources.functions.load();
        let Some(func) = functions.get(id) else {
            return Ok(Default::default());
        };

        let input_idents = (0..ctx.inputs.len()).try_fold(
            Vec::with_capacity(ctx.inputs.len()),
            |mut acc, i| {
                acc.push(ctx.get_input(i)?);
                Ok::<_, GraphNodeCodeGenError>(acc)
            },
        )?;

        let (output_idents, _, code) = func
            .graph
            .compile(
                input_idents,
                GraphVarIdentGenerator::new(format!(
                    "{}_{}",
                    id.to_string().replace('-', "_"),
                    UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed)
                )),
                ctx.texture_usage,
            )
            .map_err(|e| GraphNodeCodeGenError::Custom(e.into()))?;

        for (slot_id, output_ident) in ctx.outputs.iter().zip(output_idents) {
            ctx.output_slot_idents.insert(*slot_id, output_ident);
        }

        Ok(code)
    }
}

#[derive(Default, Clone)]
pub struct GraphInputNode;

#[derive(Default, Serialize, Deserialize)]
pub struct GraphInputNodeState {
    pub name: String,
    pub ty: Option<&'static str>,
}

#[derive(Clone)]
pub enum GraphInputNodeMessage {
    NameChanged(String),
    TypeChanged(&'static str),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for GraphInputNode {
    type State = GraphInputNodeState;
    type Message = GraphInputNodeMessage;

    fn id(&self) -> &'static str {
        "graph_input_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        GraphInputNodeState::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(GraphInputNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        let Some(ty) = state
            .ty
            .and_then(|ty| ctx.resources.type_registry.get_type(ty))
            .map(dyn_clone::clone_box)
        else {
            return vec![];
        };

        vec![GraphDefaultOutputSlot::new_boxed(state.name.clone(), ty)]
    }

    fn update_signature(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeUpdateSignatureContext<'_, Data>,
    ) {
        ctx.require_output_slot_as_graph_input(0, state.name.clone());
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        let types = ctx
            .resources
            .type_registry
            .all_types()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        ctx.view_all_slots_with_header(
            column![
                text_input(&t!("name"), &state.name)
                    .size(12.0)
                    .style(lapiz_widgets::text_input::default)
                    .on_input(GraphInputNodeMessage::NameChanged),
                ComboBox::new(types, state.ty, GraphInputNodeMessage::TypeChanged)
                    .width(Length::Fill),
            ]
            .spacing(2),
            GraphInputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
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
        _: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(Default::default())
    }
}

#[derive(Default, Clone)]
pub struct GraphOutputNode;

#[derive(Default, Serialize, Deserialize)]
pub struct GraphOutputNodeState {
    pub name: String,
    pub ty: Option<&'static str>,
}

#[derive(Clone)]
pub enum GraphOutputNodeMessage {
    NameChanged(String),
    TypeChanged(&'static str),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for GraphOutputNode {
    type State = GraphOutputNodeState;
    type Message = GraphOutputNodeMessage;

    fn id(&self) -> &'static str {
        "graph_output_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        GraphOutputNodeState::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(GraphOutputNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        let Some(ty) = state
            .ty
            .and_then(|ty| ctx.resources.type_registry.get_type(ty))
            .map(dyn_clone::clone_box)
        else {
            return vec![];
        };

        vec![GraphDefaultInputSlot::new_boxed(state.name.clone(), ty)]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![]
    }

    fn update_signature(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeUpdateSignatureContext<'_, Data>,
    ) {
        ctx.require_input_slot_as_graph_output(0, state.name.clone());
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        let types = ctx
            .resources
            .type_registry
            .all_types()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        ctx.view_all_slots_with_header(
            column![
                text_input(&t!("name"), &state.name)
                    .size(12.0)
                    .style(lapiz_widgets::text_input::default)
                    .on_input(GraphOutputNodeMessage::NameChanged),
                ComboBox::new(types, state.ty, GraphOutputNodeMessage::TypeChanged)
                    .width(Length::Fill),
            ]
            .spacing(2),
            GraphOutputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
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
        _: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(Default::default())
    }
}

#[derive(Clone)]
pub struct ExternalVariableReference {
    pub id: ExternalVariableId,
    pub name: String,
}

impl PartialEq for ExternalVariableReference {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl std::fmt::Display for ExternalVariableReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

#[derive(Default, Clone)]
pub struct ExternalVariableNode;

#[derive(Clone)]
pub enum ExternalVariableNodeMessage {
    VariableChanged(ExternalVariableId),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for ExternalVariableNode {
    type State = Option<ExternalVariableId>;
    type Message = ExternalVariableNodeMessage;

    fn id(&self) -> &'static str {
        "external_variable_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ExternalVariableNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        if let Some(id) = state.as_ref() {
            match ctx.resources.external_vars.get(id) {
                Some(var) => vec![GraphDefaultOutputSlot::new_boxed(
                    var.name.clone(),
                    dyn_clone::clone_box(var.value.ty()),
                )],
                None => {
                    let all = ctx
                        .resources
                        .external_vars
                        .all()
                        .iter()
                        .map(|entry| {
                            let v = entry.value();
                            format!("{}({})", v.name, v.id)
                        })
                        .collect::<Vec<_>>()
                        .join(", ");

                    log::error!(
                        "Selected external variable {} not found in storage: {}",
                        id,
                        all
                    );
                    vec![]
                }
            }
        } else {
            vec![]
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let id = state
            .as_ref()
            .ok_or(anyhow::anyhow!("No external literal selected"))?;
        let var = ctx.resources.external_vars.get(id).ok_or(anyhow::anyhow!(
            "Selected external literal not found in storage"
        ))?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {} = {};\n",
            output,
            generate_external_variable_name(&var)
        ))
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        None
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        let variables = ctx
            .resources
            .external_vars
            .all()
            .iter()
            .map(|entry| ExternalVariableReference {
                id: entry.id,
                name: entry.name.clone(),
            })
            .collect::<Vec<_>>();
        let selected = variables
            .iter()
            .find(|reference| Some(reference.id) == *state)
            .cloned();
        ctx.view_all_slots_with_header(
            ComboBox::new(variables, selected, |reference| {
                ExternalVariableNodeMessage::VariableChanged(reference.id)
            })
            .width(Length::Fill),
            ExternalVariableNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            ExternalVariableNodeMessage::VariableChanged(id) => *state = Some(id),
            ExternalVariableNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }
}

pub const CUBIC_CURVE_MAX_CONTROL_POINTS: usize = 16;

#[derive(Default, Clone)]
pub struct CurveNode;

#[derive(Serialize, Deserialize)]
pub struct CurveNodeState {
    pub control_points: Vec<Vec2>,
}

impl Default for CurveNodeState {
    fn default() -> Self {
        Self {
            control_points: vec![Vec2::ZERO, Vec2::ONE],
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SerializableCurveNodeState {
    pub tension: f32,
    pub control_points: Vec<Vec2>,
}

#[derive(Clone)]
pub enum CurveNodeMessage {
    CurveChanged(CubicCurve),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for CurveNode {
    type State = CurveNodeState;
    type Message = CurveNodeMessage;

    fn id(&self) -> &'static str {
        "curve_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        Default::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CurveNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<F32Type>("x".into())]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("y".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            CurveEdit::new(CubicCurve::new(state.control_points.clone()))
                .width(Length::Fill)
                .height(Length::Fixed(128.0))
                .on_change(CurveNodeMessage::CurveChanged),
            CurveNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            CurveNodeMessage::CurveChanged(curve) => {
                state.control_points = curve.control_points().to_vec();
            }
            CurveNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let num_control_points = state.control_points.len();
        let mut control_points = state.control_points.clone();
        control_points.resize(CUBIC_CURVE_MAX_CONTROL_POINTS, Vec2::ZERO);
        let mut derivatives = CubicCurve::calculate_derivatives(&state.control_points);
        derivatives.resize(CUBIC_CURVE_MAX_CONTROL_POINTS + 1, 0.0);

        Ok(format!(
            "
let {} = render::math::sample_cubic_curve(
    render::math::CubicCurve(
        array<vec2f, {}>({}),
        array<f32, {}>({}),
        {}
    ),
    {}
);
            ",
            ctx.get_output(0)?,
            CUBIC_CURVE_MAX_CONTROL_POINTS,
            control_points
                .iter()
                .map(|p| format!("vec2({:.5}, {:.5})", p.x, p.y))
                .collect::<Vec<_>>()
                .join(", "),
            CUBIC_CURVE_MAX_CONTROL_POINTS + 1,
            derivatives
                .iter()
                .map(|d| format!("{:.5}", d))
                .collect::<Vec<_>>()
                .join(", "),
            num_control_points,
            ctx.get_input(0)?
        ))
    }
}

#[derive(Default, Clone)]
pub struct RandomNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for RandomNode {
    fn id(&self) -> &'static str {
        "random_number_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RandomNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<F32Type>("seed".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("scalar".into()),
            GraphDefaultOutputSlot::new::<Vec2FType>("vec2".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = render::hash::hash11({});\nlet {} = render::hash::hash21({});\n",
            ctx.get_output(0)?,
            ctx.get_input(0)?,
            ctx.get_output(1)?,
            ctx.get_input(0)?
        ))
    }
}

impl RandomNode {
    pub fn hash11(mut p: f32) -> f32 {
        p = (p * 0.1031).fract();
        p *= p + 33.33;
        p *= p + p;
        p.fract()
    }

    pub fn hash21(p: f32) -> Vec2 {
        let mut p3 = (Vec3::splat(p) * Vec3::new(0.1031, 0.1030, 0.0973)).fract();
        p3 += p3.dot(p3.yzx() + Vec3::splat(33.33));
        ((Vec2::new(p3.x, p3.x) + Vec2::new(p3.y, p3.z)) * Vec2::new(p3.z, p3.y)).fract()
    }
}

#[derive(Default, Clone)]
pub struct RepeatIterationNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for RepeatIterationNode {
    fn id(&self) -> &'static str {
        "repeat_iteration_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RepeatIterationNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        Vec::new()
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<I32Type>("iteration".into())]
    }

    fn update_signature(&self, mut ctx: GraphNodeUpdateSignatureContext<'_, Data>) {
        ctx.require_output_slot_as_graph_input(0, "Iteration".into());
    }

    fn generate_code(
        &self,
        _: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(String::new())
    }
}

wrapper! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    pub RepeatVariableId : Uuid
}

#[derive(Clone)]
pub struct RepeatLocalSchema {
    pub id: RepeatVariableId,
    pub name: String,
    pub ty: Box<dyn ErasedGraphValueType>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SerializableRepeatLocalSchema {
    pub id: RepeatVariableId,
    pub name: String,
    pub ty: String,
}

impl<Data: GraphData> GraphSerializable<Data> for RepeatLocalSchema {
    fn to_toml(&self) -> anyhow::Result<toml::Value> {
        let serializable = SerializableRepeatLocalSchema {
            id: self.id,
            name: self.name.clone(),
            ty: self.ty.name().to_string(),
        };
        Ok(toml::Value::try_from(serializable)?)
    }

    fn from_toml(value: toml::Value, resources: &GraphResources<Data>) -> anyhow::Result<Self> {
        let serializable = SerializableRepeatLocalSchema::deserialize(value)?;
        let ty = resources
            .type_registry
            .get_type(&serializable.ty)
            .ok_or_else(|| anyhow::anyhow!("Unknown type: {}", serializable.ty))?;
        Ok(RepeatLocalSchema {
            id: serializable.id,
            name: serializable.name,
            ty: dyn_clone::clone_box(ty),
        })
    }
}

#[derive(Clone)]
struct RepeatLocalSchemaDraft {
    pub id: RepeatVariableId,
    pub name: String,
    pub ty: Option<Box<dyn ErasedGraphValueType>>,
}

#[derive(Clone, Default)]
struct RepeatSchemaDraft {
    locals: IndexMap<RepeatVariableId, RepeatLocalSchemaDraft>,
}

impl RepeatSchemaDraft {
    pub fn new(locals: &IndexMap<RepeatVariableId, RepeatLocalSchema>) -> Self {
        Self {
            locals: locals
                .iter()
                .map(|(id, schema)| {
                    (
                        *id,
                        RepeatLocalSchemaDraft {
                            id: *id,
                            name: schema.name.clone(),
                            ty: Some(schema.ty.clone()),
                        },
                    )
                })
                .collect(),
        }
    }

    pub fn finalize(&self) -> IndexMap<RepeatVariableId, RepeatLocalSchema> {
        self.locals
            .iter()
            .map(|(id, schema)| {
                (
                    *id,
                    RepeatLocalSchema {
                        id: *id,
                        name: schema.name.clone(),
                        ty: schema.ty.clone().unwrap(),
                    },
                )
            })
            .collect()
    }
}

pub struct RepeatNodeState<Data: GraphData> {
    locals: Arc<Mutex<IndexMap<RepeatVariableId, RepeatLocalSchema>>>,
    revision: u64,
    body: Graph<Data>,
    schema_draft: Option<RepeatSchemaDraft>,
}

impl<Data: GraphData> RepeatNodeState<Data> {
    pub fn body(&self) -> &Graph<Data> {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut Graph<Data> {
        &mut self.body
    }

    pub fn add_local<T: GraphValueType + Default>(&mut self, name: String) -> RepeatVariableId {
        let id = RepeatVariableId::new(Uuid::new_v4());
        self.locals.lock().insert(
            id,
            RepeatLocalSchema {
                id,
                name,
                ty: Box::new(T::default()),
            },
        );
        self.revision += 1;
        id
    }

    pub fn sync_body_nodes(&mut self) {
        let input_ids = self
            .body
            .nodes
            .iter()
            .filter(|(_, node)| node.data.state::<RepeatInputNode>().is_some())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let output_ids = self
            .body
            .nodes
            .iter()
            .filter(|(_, node)| node.data.state::<RepeatOutputNode>().is_some())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();

        let locals = self.locals.lock().clone();
        for id in input_ids {
            self.body.update_node_state::<RepeatInputNode>(id, |st| {
                if let Some(variable) = st.variable
                    && !locals.contains_key(&variable)
                {
                    st.variable = None;
                }
            });
        }
        for id in output_ids {
            self.body.update_node_state::<RepeatOutputNode>(id, |st| {
                if let Some(variable) = st.variable
                    && !locals.contains_key(&variable)
                {
                    st.variable = None;
                }
            });
        }
        self.body.invalidate_cache();
    }
}

#[derive(Serialize, Deserialize)]
struct SerializableRepeatNodeState {
    locals: Vec<SerializableRepeatLocalSchema>,
    body: SerializableGraph,
}

impl<Data: GraphData> GraphSerializable<Data> for RepeatNodeState<Data> {
    fn to_toml(&self) -> anyhow::Result<toml::Value> {
        let locals = self
            .locals
            .lock()
            .values()
            .map(|local| SerializableRepeatLocalSchema {
                id: local.id,
                name: local.name.clone(),
                ty: local.ty.clone().name().to_string(),
            })
            .collect();
        let body = self.body.as_serialized()?;
        Ok(toml::Value::try_from(SerializableRepeatNodeState {
            locals,
            body,
        })?)
    }

    fn from_toml(value: toml::Value, resources: &GraphResources<Data>) -> anyhow::Result<Self> {
        let serialized = SerializableRepeatNodeState::deserialize(value)?;
        let locals =
            serialized
                .locals
                .into_iter()
                .try_fold(IndexMap::new(), |mut locals, local| {
                    locals.insert(
                        local.id,
                        RepeatLocalSchema {
                            id: local.id,
                            name: local.name,
                            ty: dyn_clone::clone_box(
                                resources.type_registry.get_type(&local.ty).ok_or_else(|| {
                                    anyhow!("Type {} not found in registry", local.ty)
                                })?,
                            ),
                        },
                    );

                    Result::<_, anyhow::Error>::Ok(locals)
                })?;
        let locals = Arc::new(Mutex::new(locals));

        let repeat_node_extra = {
            let mut r = GraphNodeRegistry::default();
            r.register_boxed(Box::new(RepeatInputNode {
                locals: locals.clone(),
            }));
            r.register_boxed(Box::new(RepeatOutputNode {
                locals: locals.clone(),
            }));
            r.register::<RepeatIterationNode>();
            r
        };

        let mut node_registry = resources.node_registry.as_ref().clone();
        node_registry.merge(repeat_node_extra);

        let body_resources = GraphResources {
            type_registry: resources.type_registry.clone(),
            node_registry: Arc::new(node_registry),
            textures: resources.textures.clone(),
            functions: resources.functions.clone(),
            external_vars: resources.external_vars.clone(),
        };
        let (body, errors) = Graph::from_serialized(&serialized.body, body_resources);
        if !errors.is_empty() {
            return Err(anyhow!("Repeat body deserialization failed: {errors:?}"));
        }
        let body = body.ok_or_else(|| anyhow!("Repeat body is missing"))?;

        let mut state = Self {
            locals,
            revision: 0,
            body,
            schema_draft: None,
        };
        state.sync_body_nodes();
        Ok(state)
    }
}

#[derive(Default, Clone)]
pub struct RepeatInputNode {
    pub locals: Arc<Mutex<IndexMap<RepeatVariableId, RepeatLocalSchema>>>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RepeatInputNodeState {
    pub variable: Option<RepeatVariableId>,
}

#[derive(Clone)]
pub enum RepeatInputNodeMessage {
    VariableChanged(RepeatVariableId),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for RepeatInputNode {
    type State = RepeatInputNodeState;
    type Message = RepeatInputNodeMessage;

    fn id(&self) -> &'static str {
        "repeat_input_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        RepeatInputNodeState::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RepeatInputNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        Vec::new()
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        let locals = self.locals.lock();
        let Some(local) = state.variable.as_ref().and_then(|id| locals.get(id)) else {
            return Vec::new();
        };
        vec![GraphDefaultOutputSlot::new_boxed(
            format!("{} Current", local.name),
            local.ty.clone(),
        )]
    }

    fn update_signature(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeUpdateSignatureContext<'_, Data>,
    ) {
        let locals = self.locals.lock();
        let Some(local) = state.variable.as_ref().and_then(|id| locals.get(id)) else {
            return;
        };
        ctx.require_output_slot_as_graph_input(0, local.name.clone());
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        let locals = repeat_variable_references(&self.locals.lock());
        let selected = state
            .variable
            .and_then(|id| locals.iter().find(|reference| reference.id == id).cloned());
        ctx.view_all_slots_with_header(
            ComboBox::new(locals, selected, |reference| {
                RepeatInputNodeMessage::VariableChanged(reference.id)
            })
            .width(Length::Fill),
            RepeatInputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            RepeatInputNodeMessage::VariableChanged(variable) => state.variable = Some(variable),
            RepeatInputNodeMessage::LiteralUpdate(literal) => ctx.update_literal(literal),
        }
    }

    fn generate_code(
        &self,
        _: &Self::State,
        _: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(String::new())
    }
}

#[derive(Default, Clone)]
pub struct RepeatOutputNode {
    pub locals: Arc<Mutex<IndexMap<RepeatVariableId, RepeatLocalSchema>>>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RepeatOutputNodeState {
    pub variable: Option<RepeatVariableId>,
}

#[derive(Clone)]
pub enum RepeatOutputNodeMessage {
    VariableChanged(RepeatVariableId),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for RepeatOutputNode {
    type State = RepeatOutputNodeState;
    type Message = RepeatOutputNodeMessage;

    fn id(&self) -> &'static str {
        "repeat_output_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        RepeatOutputNodeState::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RepeatOutputNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        let locals = self.locals.lock();
        let Some(local) = state.variable.as_ref().and_then(|id| locals.get(id)) else {
            return Vec::new();
        };
        vec![GraphDefaultInputSlot::new_boxed(
            format!("{} Next", local.name),
            local.ty.clone(),
        )]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        Vec::new()
    }

    fn update_signature(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeUpdateSignatureContext<'_, Data>,
    ) {
        let locals = self.locals.lock();
        let Some(local) = state.variable.as_ref().and_then(|id| locals.get(id)) else {
            return;
        };
        ctx.require_input_slot_as_graph_output(0, local.name.clone());
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        let locals = repeat_variable_references(&self.locals.lock());
        let selected = state
            .variable
            .and_then(|id| locals.iter().find(|reference| reference.id == id).cloned());
        ctx.view_all_slots_with_header(
            ComboBox::new(locals, selected, |reference| {
                RepeatOutputNodeMessage::VariableChanged(reference.id)
            })
            .width(Length::Fill),
            RepeatOutputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            RepeatOutputNodeMessage::VariableChanged(variable) => state.variable = Some(variable),
            RepeatOutputNodeMessage::LiteralUpdate(literal) => ctx.update_literal(literal),
        }
    }

    fn generate_code(
        &self,
        _: &Self::State,
        _: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(String::new())
    }
}

#[derive(Clone)]
struct RepeatVariableReference {
    id: RepeatVariableId,
    name: String,
}

impl std::fmt::Display for RepeatVariableReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.name.fmt(f)
    }
}

impl PartialEq for RepeatVariableReference {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

fn repeat_variable_references(
    locals: &IndexMap<RepeatVariableId, RepeatLocalSchema>,
) -> Vec<RepeatVariableReference> {
    locals
        .values()
        .map(|local| RepeatVariableReference {
            id: local.id,
            name: local.name.clone(),
        })
        .collect()
}

#[derive(Default, Clone)]
pub struct RepeatNode;

#[derive(Clone)]
pub enum RepeatNodeMessage {
    ToggleEditor,
    EditorAddLocal,
    EditorRemoveLocal(RepeatVariableId),
    EditorMoveLocalUp(RepeatVariableId),
    EditorMoveLocalDown(RepeatVariableId),
    EditorRenameLocal(RepeatVariableId, String),
    EditorChangeLocalType(RepeatVariableId, String),
    EditorConfirm,
    EditorCancel,
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

fn repeat_schema_editor_view<Data: GraphData>(
    state: &RepeatNodeState<Data>,
    resources: &GraphResources<Data>,
) -> GraphElement<'static, RepeatNodeMessage> {
    let type_names = resources
        .type_registry
        .all_types()
        .keys()
        .copied()
        .collect::<Vec<&'static str>>();
    let draft = state.schema_draft.as_ref().expect("editor must be open");

    let rows = draft
        .locals
        .values()
        .map(|local| {
            let id = local.id;
            column![
                text_input(&t!("variable_name"), &local.name)
                    .size(12.0)
                    .style(lapiz_widgets::text_input::default)
                    .on_input(move |name| { RepeatNodeMessage::EditorRenameLocal(id, name) }),
                row![
                    ComboBox::new(
                        type_names.clone(),
                        local.ty.as_ref().map(|t| t.name()),
                        move |ty| { RepeatNodeMessage::EditorChangeLocalType(id, ty.to_string()) },
                    )
                    .width(Length::Fill),
                    button("Up").on_press(RepeatNodeMessage::EditorMoveLocalUp(id)),
                    button("Down").on_press(RepeatNodeMessage::EditorMoveLocalDown(id)),
                    button("Delete").on_press(RepeatNodeMessage::EditorRemoveLocal(id)),
                ]
                .spacing(4),
            ]
            .spacing(4)
            .into()
        })
        .collect::<Vec<GraphElement<'static, RepeatNodeMessage>>>();

    let valid = draft
        .locals
        .values()
        .all(|local| !local.name.is_empty() && local.name.trim() == local.name)
        && draft.locals.values().all(|local| {
            draft
                .locals
                .values()
                .filter(|other| other.name == local.name)
                .count()
                == 1
        });

    let panel = column(rows)
        .width(Length::Fixed(300.0))
        .padding(4)
        .spacing(6)
        .push(row![button("Add Variable").on_press(RepeatNodeMessage::EditorAddLocal)].spacing(6))
        .push(
            row![
                button("Cancel").on_press(RepeatNodeMessage::EditorCancel),
                button("Confirm").when(valid, |b| b.on_press(RepeatNodeMessage::EditorConfirm))
            ]
            .spacing(4),
        );

    container(panel)
        .style(|theme| container::Style {
            background: Some(theme.extended_palette().background.base.color.into()),
            ..container::transparent(theme)
        })
        .into()
}

impl<Data: GraphData> GraphNode<Data> for RepeatNode {
    type State = RepeatNodeState<Data>;

    type Message = RepeatNodeMessage;

    fn id(&self) -> &'static str {
        "repeat_node"
    }

    fn default_state(&self, ctx: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        let locals = Arc::new(Mutex::new(IndexMap::new()));
        let repeat_node_extra = {
            let mut r = GraphNodeRegistry::default();
            r.register_boxed(Box::new(RepeatInputNode {
                locals: locals.clone(),
            }));
            r.register_boxed(Box::new(RepeatOutputNode {
                locals: locals.clone(),
            }));
            r.register::<RepeatIterationNode>();
            r
        };

        let mut node_registry = ctx.resources.node_registry.as_ref().clone();
        node_registry.merge(repeat_node_extra);

        let body_resources = GraphResources {
            type_registry: ctx.resources.type_registry.clone(),
            node_registry: Arc::new(node_registry),
            textures: ctx.resources.textures.clone(),
            functions: ctx.resources.functions.clone(),
            external_vars: ctx.resources.external_vars.clone(),
        };

        RepeatNodeState {
            locals,
            revision: 0,
            body: Graph::new(body_resources),
            schema_draft: None,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RepeatNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        std::iter::once(GraphDefaultInputSlot::new::<I32Type>("iterations".into()))
            .chain(state.locals.lock().values().map(|local| {
                GraphDefaultInputSlot::new_boxed(format!("{} In", local.name), local.ty.clone())
            }))
            .collect()
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        state
            .locals
            .lock()
            .values()
            .map(|local| {
                GraphDefaultOutputSlot::new_boxed(format!("{} Out", local.name), local.ty.clone())
            })
            .collect()
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'static, Self::Message> {
        let trigger = button("Edit").on_press(RepeatNodeMessage::ToggleEditor);
        let content = state
            .schema_draft
            .as_ref()
            .map(|_| repeat_schema_editor_view(state, ctx.resources));
        let popover = Popover::new(trigger).content(content);
        ctx.view_all_slots_with_header(popover, RepeatNodeMessage::LiteralUpdate)
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            RepeatNodeMessage::ToggleEditor => {
                if state.schema_draft.is_some() {
                    state.schema_draft = None;
                } else {
                    state.schema_draft = Some(RepeatSchemaDraft::new(&state.locals.lock()));
                }
            }
            RepeatNodeMessage::EditorAddLocal => {
                if let Some(draft) = &mut state.schema_draft {
                    let new_id = RepeatVariableId::new(Uuid::new_v4());
                    draft.locals.insert(
                        new_id,
                        RepeatLocalSchemaDraft {
                            id: new_id,
                            name: String::new(),
                            ty: None,
                        },
                    );
                }
            }
            RepeatNodeMessage::EditorRemoveLocal(id) => {
                if let Some(draft) = &mut state.schema_draft {
                    draft.locals.shift_remove(&id);
                }
            }
            RepeatNodeMessage::EditorMoveLocalUp(id) => {
                if let Some(draft) = &mut state.schema_draft
                    && let Some(index) = draft.locals.get_index_of(&id)
                    && index > 0
                {
                    draft.locals.swap_indices(index, index - 1);
                }
            }
            RepeatNodeMessage::EditorMoveLocalDown(id) => {
                if let Some(draft) = &mut state.schema_draft
                    && let Some(index) = draft.locals.get_index_of(&id)
                    && index + 1 < draft.locals.len()
                {
                    draft.locals.swap_indices(index, index + 1);
                }
            }
            RepeatNodeMessage::EditorRenameLocal(id, name) => {
                if let Some(draft) = &mut state.schema_draft
                    && let Some(local) = draft.locals.get_mut(&id)
                {
                    local.name = name;
                }
            }
            RepeatNodeMessage::EditorChangeLocalType(id, ty) => {
                if let Some(draft) = &mut state.schema_draft
                    && let Some(local) = draft.locals.get_mut(&id)
                {
                    local.ty = Some(dyn_clone::clone_box(
                        ctx.resources.type_registry.get_type(&ty).unwrap(),
                    ));
                }
            }
            RepeatNodeMessage::EditorConfirm => {
                let Some(draft) = &mut state.schema_draft else {
                    return;
                };
                *state.locals.lock() = draft.finalize();
                state.revision += 1;
                state.schema_draft = None;
                state.sync_body_nodes();
            }
            RepeatNodeMessage::EditorCancel => {
                state.schema_draft = None;
            }
            RepeatNodeMessage::LiteralUpdate(literal) => ctx.update_literal(literal),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let locals = state.locals.lock().clone();
        if locals.len() + 1 != ctx.inputs.len() || locals.len() != ctx.outputs.len() {
            return Err(anyhow!("Repeat parent slot invariant is invalid").into());
        }

        let iterations = ctx.get_input(0)?;
        let body = &state.body;
        let signature = body.signature();

        let mut current = HashMap::with_capacity(locals.len());
        let mut code = String::new();
        for (index, local) in locals.values().enumerate() {
            let value = ctx.ident_generator.next_output();
            code.push_str(&format!(
                "var {value} = {};
",
                ctx.get_input(index + 1)?
            ));
            current.insert(local.id, value);
        }

        let iteration = ctx.ident_generator.next_output();
        let mut body_inputs = Vec::with_capacity(signature.inputs.len());
        for slot_id in signature.inputs.keys() {
            let slot = body
                .slots
                .get_output(slot_id)
                .ok_or(GraphNodeCodeGenError::MissingOutputSlot)?;
            let node = body.get_node(&slot.node_id).ok_or_else(|| {
                GraphNodeCodeGenError::Custom(anyhow!("Repeat body node is missing"))
            })?;
            if node.data.is::<RepeatIterationNode>() {
                body_inputs.push(iteration.clone());
                continue;
            }
            let variable = node
                .data
                .state::<RepeatInputNode>()
                .and_then(|state| state.variable)
                .ok_or_else(|| anyhow!("Repeat Input has an invalid variable"))?;
            body_inputs.push(
                current
                    .get(&variable)
                    .cloned()
                    .ok_or_else(|| anyhow!("Repeat variable {variable} is not a local"))?,
            );
        }

        let mut next_slots = HashMap::with_capacity(locals.len());
        for slot_id in signature.outputs.keys() {
            let slot = body
                .slots
                .get_input(slot_id)
                .ok_or(GraphNodeCodeGenError::MissingInputSlot)?;
            let node = body
                .get_node(&slot.node_id)
                .ok_or_else(|| anyhow!("Repeat body node is missing"))?;
            let variable = node
                .data
                .state::<RepeatOutputNode>()
                .and_then(|state| state.variable)
                .ok_or_else(|| anyhow!("Repeat Output has an invalid variable"))?;
            if next_slots.insert(variable, *slot_id).is_some() {
                return Err(anyhow!("Repeat variable {variable} has duplicate outputs").into());
            }
        }
        for local in locals.values() {
            if !next_slots.contains_key(&local.id) {
                return Err(anyhow!(
                    "Repeat body is missing a Repeat Output for variable '{}'",
                    local.name
                )
                .into());
            }
        }

        code.push_str(&format!(
            "for (var {iteration} = 0i; {iteration} < {iterations}; {iteration}++) {{\n"
        ));
        let (body_output_idents, _, body_code) = body
            .compile(
                body_inputs,
                GraphVarIdentGenerator::new(format!(
                    "repeat_{}",
                    UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed)
                )),
                ctx.texture_usage,
            )
            .map_err(|error| GraphNodeCodeGenError::Custom(error.into()))?;
        code.push_str(&body_code);

        let body_outputs = signature
            .outputs
            .keys()
            .copied()
            .zip(body_output_idents)
            .collect::<HashMap<_, _>>();
        for local in locals.values() {
            let input_slot_id = next_slots[&local.id];
            let next = body_outputs
                .get(&input_slot_id)
                .ok_or_else(|| anyhow!("Repeat body output value of {} is missing", local.name))?;
            let input_slot = body
                .slots
                .get_input(&input_slot_id)
                .ok_or(GraphNodeCodeGenError::MissingInputSlot)?;
            let next = if let Some(connected) = input_slot.connected {
                let output_slot = body
                    .slots
                    .get_output(&connected)
                    .ok_or(GraphNodeCodeGenError::MissingOutputSlot)?;
                if output_slot.data_ty.name() != input_slot.data.ty().name() {
                    ctx.resources
                        .type_registry
                        .try_wgsl_cast(&*output_slot.data_ty, input_slot.data.ty(), next)
                        .ok_or(GraphNodeCodeGenError::FailedToCastVariable)?
                } else {
                    next.clone()
                }
            } else {
                next.clone()
            };
            code.push_str(&format!(
                "{} = {next};
",
                current[&local.id]
            ));
        }

        code.push_str("}\n");

        for (slot_id, local) in ctx.outputs.iter().zip(locals.values()) {
            ctx.output_slot_idents
                .insert(*slot_id, current[&local.id].clone());
        }

        Ok(code)
    }

    fn subgraphs<'a>(&self, state: &'a Self::State) -> Vec<&'a Graph<Data>> {
        vec![&state.body]
    }

    fn subgraphs_mut<'a>(&mut self, state: &'a mut Self::State) -> Vec<&'a mut Graph<Data>> {
        vec![&mut state.body]
    }
}

wrapper! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    pub CustomExpressionVariableId : Uuid
}

#[derive(Clone)]
pub struct CustomExpressionVariable {
    pub id: CustomExpressionVariableId,
    pub display_name: String,
    pub name: String,
    pub ty: Box<dyn ErasedGraphValueType>,
}

#[derive(Serialize, Deserialize)]
struct SerializableCustomExpressionVariable {
    id: CustomExpressionVariableId,
    display_name: String,
    name: String,
    ty: String,
}

#[derive(Default)]
pub struct CustomExpressionNodeState {
    inputs: IndexMap<CustomExpressionVariableId, CustomExpressionVariable>,
    outputs: IndexMap<CustomExpressionVariableId, CustomExpressionVariable>,
    code: String,
    draft: Option<CustomExpressionDraft>,
}

impl CustomExpressionNodeState {
    pub fn set_code(&mut self, code: impl Into<String>) {
        self.code = code.into();
    }

    pub fn add_input<T: GraphValueType + Default>(
        &mut self,
        name: impl Into<String>,
    ) -> CustomExpressionVariableId {
        Self::add_variable::<T>(&mut self.inputs, name)
    }

    pub fn add_output<T: GraphValueType + Default>(
        &mut self,
        name: impl Into<String>,
    ) -> CustomExpressionVariableId {
        Self::add_variable::<T>(&mut self.outputs, name)
    }

    fn add_variable<T: GraphValueType + Default>(
        variables: &mut IndexMap<CustomExpressionVariableId, CustomExpressionVariable>,
        name: impl Into<String>,
    ) -> CustomExpressionVariableId {
        let name = name.into();
        let id = CustomExpressionVariableId::new(Uuid::new_v4());
        variables.insert(
            id,
            CustomExpressionVariable {
                id,
                display_name: name.clone(),
                name,
                ty: Box::new(T::default()),
            },
        );
        id
    }
}

#[derive(Clone)]
struct CustomExpressionVariableDraft {
    id: CustomExpressionVariableId,
    display_name: String,
    name: String,
    ty: Option<Box<dyn ErasedGraphValueType>>,
}

#[derive(Clone, Copy)]
pub enum CustomExpressionVariableKind {
    Input,
    Output,
}

struct CustomExpressionDraft {
    inputs: IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft>,
    outputs: IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft>,
}

impl CustomExpressionDraft {
    fn new(state: &CustomExpressionNodeState) -> Self {
        let to_draft = |variables: &IndexMap<_, CustomExpressionVariable>| {
            variables
                .iter()
                .map(|(id, variable)| {
                    (
                        *id,
                        CustomExpressionVariableDraft {
                            id: *id,
                            display_name: variable.display_name.clone(),
                            name: variable.name.clone(),
                            ty: Some(variable.ty.clone()),
                        },
                    )
                })
                .collect()
        };
        Self {
            inputs: to_draft(&state.inputs),
            outputs: to_draft(&state.outputs),
        }
    }

    fn variables_mut(
        &mut self,
        kind: CustomExpressionVariableKind,
    ) -> &mut IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft> {
        match kind {
            CustomExpressionVariableKind::Input => &mut self.inputs,
            CustomExpressionVariableKind::Output => &mut self.outputs,
        }
    }

    fn finalize(
        variables: &IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft>,
    ) -> IndexMap<CustomExpressionVariableId, CustomExpressionVariable> {
        variables
            .iter()
            .map(|(id, variable)| {
                (
                    *id,
                    CustomExpressionVariable {
                        id: *id,
                        display_name: variable.display_name.clone(),
                        name: variable.name.clone(),
                        ty: variable.ty.clone().unwrap(),
                    },
                )
            })
            .collect()
    }
}

#[derive(Serialize, Deserialize)]
struct SerializableCustomExpressionNodeState {
    inputs: Vec<SerializableCustomExpressionVariable>,
    outputs: Vec<SerializableCustomExpressionVariable>,
    code: String,
}

impl<Data: GraphData> GraphSerializable<Data> for CustomExpressionNodeState {
    fn to_toml(&self) -> anyhow::Result<toml::Value> {
        let serialize = |variables: &IndexMap<_, CustomExpressionVariable>| {
            variables
                .values()
                .map(|variable| SerializableCustomExpressionVariable {
                    id: variable.id,
                    display_name: variable.display_name.clone(),
                    name: variable.name.clone(),
                    ty: variable.ty.name().to_string(),
                })
                .collect()
        };
        Ok(toml::Value::try_from(
            SerializableCustomExpressionNodeState {
                inputs: serialize(&self.inputs),
                outputs: serialize(&self.outputs),
                code: self.code.clone(),
            },
        )?)
    }

    fn from_toml(value: toml::Value, resources: &GraphResources<Data>) -> anyhow::Result<Self> {
        let serialized = SerializableCustomExpressionNodeState::deserialize(value)?;
        let deserialize = |variables: Vec<SerializableCustomExpressionVariable>| {
            variables
                .into_iter()
                .map(|variable| {
                    let ty = resources
                        .type_registry
                        .get_type(&variable.ty)
                        .ok_or_else(|| anyhow!("Unknown type"))?;
                    Ok((
                        variable.id,
                        CustomExpressionVariable {
                            id: variable.id,
                            display_name: variable.display_name,
                            name: variable.name,
                            ty: dyn_clone::clone_box(ty),
                        },
                    ))
                })
                .collect::<anyhow::Result<IndexMap<_, _>>>()
        };
        Ok(Self {
            inputs: deserialize(serialized.inputs)?,
            outputs: deserialize(serialized.outputs)?,
            code: serialized.code,
            draft: None,
        })
    }
}

struct CustomExpressionCodeEditor<'a> {
    code: &'a str,
}

// TODO Probably avoid this pattern? We are storing the actual state in widget tree,
//      because text_editor::Content is not sync, but GraphNode::State must be sync.
struct CustomExpressionCodeEditorState {
    content: RefCell<text_editor::Content<GraphRenderer>>,
}

fn custom_expression_text_editor(
    content: &text_editor::Content<GraphRenderer>,
) -> text_editor::TextEditor<
    '_,
    iced_core::text::highlighter::PlainText,
    text_editor::Action,
    GraphTheme,
    GraphRenderer,
> {
    text_editor(content)
        .placeholder("WGSL")
        .size(12.0)
        .height(Length::Fixed(140.0))
        .on_action(std::convert::identity)
}

impl Widget<CustomExpressionNodeMessage, GraphTheme, GraphRenderer>
    for CustomExpressionCodeEditor<'_>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<CustomExpressionCodeEditorState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(CustomExpressionCodeEditorState {
            content: RefCell::new(text_editor::Content::with_text(self.code)),
        })
    }

    fn children(&self) -> Vec<Tree> {
        let content = text_editor::Content::with_text(self.code);
        let editor = custom_expression_text_editor(&content);
        vec![Tree::new(&editor as &dyn Widget<_, _, _>)]
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<CustomExpressionCodeEditorState>();
        if state.content.borrow().text() != self.code {
            *state.content.borrow_mut() = text_editor::Content::with_text(self.code);
        }
        let content = state.content.borrow();
        let editor = custom_expression_text_editor(&content);
        tree.children[0].diff(&editor as &dyn Widget<_, _, _>);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fixed(140.0))
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &GraphRenderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        custom_expression_text_editor(&state.content.borrow()).layout(
            &mut tree.children[0],
            renderer,
            limits,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &GraphRenderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        custom_expression_text_editor(&state.content.borrow()).operate(
            &mut tree.children[0],
            layout,
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &GraphRenderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, CustomExpressionNodeMessage>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        let mut actions = Vec::new();
        let mut child_shell = Shell::new(&mut actions);
        custom_expression_text_editor(&state.content.borrow()).update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut child_shell,
            viewport,
        );
        shell.merge(child_shell, |action| {
            let mut content = state.content.borrow_mut();
            content.perform(action);
            CustomExpressionNodeMessage::CodeChanged(content.text())
        });
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &GraphRenderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        custom_expression_text_editor(&state.content.borrow()).mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut GraphRenderer,
        theme: &GraphTheme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        custom_expression_text_editor(&state.content.borrow()).draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
}

#[derive(Default, Clone)]
pub struct CustomExpressionNode;

#[derive(Clone)]
pub enum CustomExpressionNodeMessage {
    ToggleEditor,
    AddVariable(CustomExpressionVariableKind),
    RemoveVariable(CustomExpressionVariableKind, CustomExpressionVariableId),
    MoveVariableUp(CustomExpressionVariableKind, CustomExpressionVariableId),
    MoveVariableDown(CustomExpressionVariableKind, CustomExpressionVariableId),
    ChangeDisplayName(
        CustomExpressionVariableKind,
        CustomExpressionVariableId,
        String,
    ),
    ChangeName(
        CustomExpressionVariableKind,
        CustomExpressionVariableId,
        String,
    ),
    ChangeType(
        CustomExpressionVariableKind,
        CustomExpressionVariableId,
        String,
    ),
    CodeChanged(String),
    Confirm,
    Cancel,
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

fn custom_expression_variable_rows<Data: GraphData>(
    variables: &IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft>,
    kind: CustomExpressionVariableKind,
    resources: &GraphResources<Data>,
) -> Vec<GraphElement<'static, CustomExpressionNodeMessage>> {
    let type_names = resources
        .type_registry
        .all_types()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    variables
        .values()
        .map(|variable| {
            let id = variable.id;
            column![
                row![
                    text_input("Slot Name", &variable.display_name)
                        .size(12.0)
                        .style(lapiz_widgets::text_input::default)
                        .on_input(move |name| {
                            CustomExpressionNodeMessage::ChangeDisplayName(kind, id, name)
                        }),
                    text_input("WGSL Name", &variable.name)
                        .size(12.0)
                        .style(lapiz_widgets::text_input::default)
                        .on_input(move |name| {
                            CustomExpressionNodeMessage::ChangeName(kind, id, name)
                        }),
                ]
                .spacing(4),
                row![
                    ComboBox::new(
                        type_names.clone(),
                        variable.ty.as_ref().map(|ty| ty.name()),
                        move |ty| {
                            CustomExpressionNodeMessage::ChangeType(kind, id, ty.to_string())
                        }
                    )
                    .width(Length::Fill),
                    button("Up").on_press(CustomExpressionNodeMessage::MoveVariableUp(kind, id)),
                    button("Down")
                        .on_press(CustomExpressionNodeMessage::MoveVariableDown(kind, id)),
                    button("Delete")
                        .on_press(CustomExpressionNodeMessage::RemoveVariable(kind, id)),
                ]
                .spacing(4),
            ]
            .spacing(4)
            .into()
        })
        .collect()
}

fn is_custom_expression_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn custom_expression_editor_view<Data: GraphData>(
    state: &CustomExpressionNodeState,
    resources: &GraphResources<Data>,
) -> GraphElement<'static, CustomExpressionNodeMessage> {
    let draft = state.draft.as_ref().expect("editor must be open");
    let input_rows = custom_expression_variable_rows(
        &draft.inputs,
        CustomExpressionVariableKind::Input,
        resources,
    );
    let output_rows = custom_expression_variable_rows(
        &draft.outputs,
        CustomExpressionVariableKind::Output,
        resources,
    );
    let variables = draft
        .inputs
        .values()
        .chain(draft.outputs.values())
        .collect::<Vec<_>>();
    let valid = variables.iter().all(|variable| {
        !variable.display_name.is_empty()
            && variable.display_name.trim() == variable.display_name
            && is_custom_expression_identifier(&variable.name)
            && variable.ty.is_some()
            && variables
                .iter()
                .filter(|other| other.name == variable.name)
                .count()
                == 1
    });
    let panel = column![]
        .width(Length::Fixed(500.0))
        .padding(4)
        .spacing(6)
        .push(text(t!("inputs")).size(12))
        .extend(input_rows)
        .push(
            button("Add Input").on_press(CustomExpressionNodeMessage::AddVariable(
                CustomExpressionVariableKind::Input,
            )),
        )
        .push(text(t!("outputs")).size(12))
        .extend(output_rows)
        .push(
            button("Add Output").on_press(CustomExpressionNodeMessage::AddVariable(
                CustomExpressionVariableKind::Output,
            )),
        )
        .push(
            row![
                button("Cancel").on_press(CustomExpressionNodeMessage::Cancel),
                button("Confirm").when(valid, |button| {
                    button.on_press(CustomExpressionNodeMessage::Confirm)
                }),
            ]
            .spacing(4),
        );
    container(panel)
        .style(|theme| container::Style {
            background: Some(theme.extended_palette().background.base.color.into()),
            ..container::transparent(theme)
        })
        .into()
}

impl<Data: GraphData> GraphNode<Data> for CustomExpressionNode {
    type State = CustomExpressionNodeState;

    type Message = CustomExpressionNodeMessage;

    fn id(&self) -> &'static str {
        "custom_expression_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        CustomExpressionNodeState {
            inputs: IndexMap::new(),
            outputs: IndexMap::new(),
            code: String::new(),
            draft: None,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CustomExpressionNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        state
            .inputs
            .values()
            .map(|variable| {
                GraphDefaultInputSlot::new_boxed(variable.display_name.clone(), variable.ty.clone())
            })
            .collect()
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        state
            .outputs
            .values()
            .map(|variable| {
                GraphDefaultOutputSlot::new_boxed(
                    variable.display_name.clone(),
                    variable.ty.clone(),
                )
            })
            .collect()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'a, Self::Message> {
        let trigger = button("Edit").on_press(CustomExpressionNodeMessage::ToggleEditor);
        let content = state
            .draft
            .as_ref()
            .map(|_| custom_expression_editor_view(state, ctx.resources));
        ctx.view_all_slots_with_header(
            column![
                Popover::new(trigger).content(content),
                GraphElement::new(CustomExpressionCodeEditor { code: &state.code })
            ]
            .spacing(4),
            CustomExpressionNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            CustomExpressionNodeMessage::ToggleEditor => {
                state.draft = if state.draft.is_some() {
                    None
                } else {
                    Some(CustomExpressionDraft::new(state))
                };
            }
            CustomExpressionNodeMessage::AddVariable(kind) => {
                if let Some(draft) = &mut state.draft {
                    let id = CustomExpressionVariableId::new(Uuid::new_v4());
                    draft.variables_mut(kind).insert(
                        id,
                        CustomExpressionVariableDraft {
                            id,
                            display_name: String::new(),
                            name: String::new(),
                            ty: None,
                        },
                    );
                }
            }
            CustomExpressionNodeMessage::RemoveVariable(kind, id) => {
                if let Some(draft) = &mut state.draft {
                    draft.variables_mut(kind).shift_remove(&id);
                }
            }
            CustomExpressionNodeMessage::MoveVariableUp(kind, id) => {
                if let Some(draft) = &mut state.draft {
                    let variables = draft.variables_mut(kind);
                    if let Some(index) = variables.get_index_of(&id)
                        && index > 0
                    {
                        variables.swap_indices(index, index - 1);
                    }
                }
            }
            CustomExpressionNodeMessage::MoveVariableDown(kind, id) => {
                if let Some(draft) = &mut state.draft {
                    let variables = draft.variables_mut(kind);
                    if let Some(index) = variables.get_index_of(&id)
                        && index + 1 < variables.len()
                    {
                        variables.swap_indices(index, index + 1);
                    }
                }
            }
            CustomExpressionNodeMessage::ChangeDisplayName(kind, id, name) => {
                if let Some(variable) = state
                    .draft
                    .as_mut()
                    .and_then(|draft| draft.variables_mut(kind).get_mut(&id))
                {
                    variable.display_name = name;
                }
            }
            CustomExpressionNodeMessage::ChangeName(kind, id, name) => {
                if let Some(variable) = state
                    .draft
                    .as_mut()
                    .and_then(|draft| draft.variables_mut(kind).get_mut(&id))
                {
                    variable.name = name;
                }
            }
            CustomExpressionNodeMessage::ChangeType(kind, id, ty) => {
                if let Some(variable) = state
                    .draft
                    .as_mut()
                    .and_then(|draft| draft.variables_mut(kind).get_mut(&id))
                {
                    variable.ty = ctx
                        .resources
                        .type_registry
                        .get_type(&ty)
                        .map(dyn_clone::clone_box);
                }
            }
            CustomExpressionNodeMessage::CodeChanged(code) => state.code = code,
            CustomExpressionNodeMessage::Confirm => {
                let Some(draft) = state.draft.take() else {
                    return;
                };
                state.inputs = CustomExpressionDraft::finalize(&draft.inputs);
                state.outputs = CustomExpressionDraft::finalize(&draft.outputs);
            }
            CustomExpressionNodeMessage::Cancel => state.draft = None,
            CustomExpressionNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        if state.inputs.len() != ctx.inputs.len() || state.outputs.len() != ctx.outputs.len() {
            return Err(anyhow!("Invalid slots").into());
        }

        let names = state
            .inputs
            .values()
            .chain(state.outputs.values())
            .map(|variable| variable.name.as_str())
            .collect::<Vec<_>>();

        let mut outputs = Vec::with_capacity(state.outputs.len());
        let mut code = String::new();
        for (index, variable) in state.outputs.values().enumerate() {
            let mut output = ctx.get_output(index)?;
            while names.contains(&output.as_str()) {
                output = ctx.ident_generator.next_output();
                ctx.output_slot_idents
                    .insert(ctx.outputs[index], output.clone());
            }
            let (ty, _) = variable
                .ty
                .wgsl_type()
                .ok_or_else(|| GraphNodeCodeGenError::Custom(anyhow!("Invalid type")))?;
            code.push_str(&format!("var {output}: {ty};\n"));
            outputs.push(output);
        }

        code.push_str("{\n");

        for (index, variable) in state.inputs.values().enumerate() {
            code.push_str(&format!(
                "let {} = {};\n",
                variable.name,
                ctx.get_input(index)?
            ));
        }
        for variable in state.outputs.values() {
            let (ty, _) = variable
                .ty
                .wgsl_type()
                .ok_or_else(|| GraphNodeCodeGenError::Custom(anyhow!("Invalid type")))?;
            code.push_str(&format!("var {}: {ty};\n", variable.name));
        }
        code.push_str(&state.code);
        if !state.code.is_empty() && !state.code.ends_with('\n') {
            code.push('\n');
        }
        for ((_, variable), output) in state.outputs.iter().zip(outputs) {
            code.push_str(&format!("{output} = {};\n", variable.name));
        }

        code.push_str("}\n");
        Ok(code)
    }
}
