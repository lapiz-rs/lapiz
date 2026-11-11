use glam::{Vec2, Vec3, Vec3Swizzles as _};
use lapiz_shader_graph_derive::stateless;
use lapiz_utils::random_oklch_hue_chroma;
use wesl::syntax::{
    Declaration, DeclarationKind, Expression, FunctionCall, Ident, ModulePath, PathOrigin, Span,
    Spanned, Statement, TypeExpression,
};
use wesl_quote::quote_statement;

use crate::{
    graph::{
        node::{
            GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            StatelessCommonGraphNode,
        },
        slot::{GraphDefaultInputSlot, GraphDefaultOutputSlot},
    },
    wgsl_std::types::{primitive::F32Type, vector::Vec2FType},
};

/// Shader graph node that hashes a scalar seed ([`F32Type`]) into pseudo-random scalar and 2D vector values.
#[derive(Default, Clone)]
pub struct RandomNode;

#[stateless]
impl StatelessCommonGraphNode for RandomNode {
    fn id(&self) -> &'static str {
        "random_number_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RandomNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<F32Type>("seed".into())]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("scalar".into()),
            GraphDefaultOutputSlot::new::<Vec2FType>("vec2".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_seed = ctx.get_input(0)?;
        let output_scalar = ctx.get_output(0)?;
        let output_vec2 = ctx.get_output(1)?;

        Ok(format!(
            "{}\n{}\n",
            quote_statement! { let #output_scalar = render::hash::hash11(#input_seed); },
            quote_statement! { let #output_vec2 = render::hash::hash21(#input_seed); }
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
