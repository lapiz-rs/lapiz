use glam::Vec2;
use iced_core::Length;
use lapiz_math::curve::CubicCurve;
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::curve_edit;
use serde::{Deserialize, Serialize};
use wesl::syntax::{
    Declaration, DeclarationKind, Expression, FunctionCall, Ident, ModulePath, PathOrigin, Span,
    Spanned, Statement, TemplateArg, TypeExpression,
};
use wesl_quote::quote_statement;

use crate::{
    GraphElement,
    graph::{
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeUpdateContext, GraphNodeViewContext,
        },
        slot::{ErasedGraphLiteralUpdateMessage, GraphDefaultInputSlot, GraphDefaultOutputSlot},
    },
    wgsl_std::types::primitive::F32Type,
};

pub const CUBIC_CURVE_MAX_CONTROL_POINTS: usize = 16;

/// Shader graph node that remaps a scalar input through an editable [`CubicCurve`].
#[derive(Default, Clone)]
pub struct CurveNode;

/// Per-instance state for [`CurveNode`], storing the 2D control points of its cubic spline.
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

/// Serializable DTO for persisting curve tension and control points.
#[derive(Serialize, Deserialize)]
pub struct SerializableCurveNodeState {
    pub tension: f32,
    pub control_points: Vec<Vec2>,
}

/// UI message emitted by [`CurveNode`].
#[derive(Clone)]
pub enum CurveNodeMessage {
    CurveChanged(CubicCurve),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for CurveNode {
    type State = CurveNodeState;
    type Message = CurveNodeMessage;

    fn id(&self) -> &'static str {
        "curve_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        Default::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CurveNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<F32Type>("x".into())]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("y".into())]
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        ctx.view_all_slots_with_header(
            curve_edit(CubicCurve::new(state.control_points.clone()))
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
        mut ctx: GraphNodeUpdateContext<'_>,
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
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let num_control_points = state.control_points.len() as i64;
        let mut control_points = state.control_points.clone();
        control_points.resize(CUBIC_CURVE_MAX_CONTROL_POINTS, Vec2::ZERO);
        let mut derivatives = CubicCurve::calculate_derivatives(&state.control_points);
        derivatives.resize(CUBIC_CURVE_MAX_CONTROL_POINTS + 1, 0.0);

        let output = ctx.get_output(0)?;
        let input_x = ctx.get_input(0)?;
        let capacity = CUBIC_CURVE_MAX_CONTROL_POINTS as i64;
        let derivative_capacity = (CUBIC_CURVE_MAX_CONTROL_POINTS + 1) as i64;
        let control_points_code = Ident::new(
            control_points
                .iter()
                .map(|p| format!("vec2({:.5}, {:.5})", p.x, p.y))
                .collect::<Vec<_>>()
                .join(", "),
        );
        let derivatives_code = Ident::new(
            derivatives
                .iter()
                .map(|d| format!("{:.5}", d))
                .collect::<Vec<_>>()
                .join(", "),
        );

        Ok(format!(
            "{}\n",
            quote_statement! {
                let #output = render::math::sample_cubic_curve(
                    render::math::CubicCurve(
                        array<vec2f, #capacity>(#control_points_code),
                        array<f32, #derivative_capacity>(#derivatives_code),
                        #num_control_points
                    ),
                    #input_x
                );
            }
        ))
    }
}
