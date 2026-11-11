use lapiz_shader_graph_derive::stateless;
use lapiz_utils::random_oklch_hue_chroma;
use wesl::syntax::{
    Declaration, DeclarationKind, Expression, FunctionCall, Ident, NamedComponentExpression, Span,
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
    wgsl_std::types::{compound::ColorType, primitive::F32Type, vector::Vec2FType},
};

/// Shader graph node that splits a 2D vector ([`Vec2FType`]) into its scalar `x` and `y` components ([`F32Type`]).
#[derive(Default, Clone)]
pub struct SplitComponentsNode;

#[stateless]
impl StatelessCommonGraphNode for SplitComponentsNode {
    fn id(&self) -> &'static str {
        "split_components_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SplitComponentsNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("vector".into())]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("x".into()),
            GraphDefaultOutputSlot::new::<F32Type>("y".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_vector = ctx.get_input(0)?;
        let output_x = ctx.get_output(0)?;
        let output_y = ctx.get_output(1)?;

        Ok(format!(
            "{}\n{}\n",
            quote_statement! { let #output_x = #input_vector.x; },
            quote_statement! { let #output_y = #input_vector.y; }
        ))
    }
}

/// Shader graph node that combines two scalar `x` and `y` values ([`F32Type`]) into a 2D vector ([`Vec2FType`]).
#[derive(Default, Clone)]
pub struct CombineComponentsNode;

#[stateless]
impl StatelessCommonGraphNode for CombineComponentsNode {
    fn id(&self) -> &'static str {
        "combine_components_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CombineComponentsNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("x".into()),
            GraphDefaultInputSlot::new::<F32Type>("y".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("vector".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_x = ctx.get_input(0)?;
        let input_y = ctx.get_input(1)?;
        let output_vector = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            quote_statement! { let #output_vector = vec2f(#input_x, #input_y); }
        ))
    }
}

/// Shader graph node that combines four scalar `r`, `g`, `b`, and `a` channels ([`F32Type`]) into a color ([`ColorType`]).
#[derive(Default, Clone)]
pub struct CombineColorComponentsNode;

#[stateless]
impl StatelessCommonGraphNode for CombineColorComponentsNode {
    fn id(&self) -> &'static str {
        "combine_color_components_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CombineColorComponentsNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<F32Type>("r".into()),
            GraphDefaultInputSlot::new::<F32Type>("g".into()),
            GraphDefaultInputSlot::new::<F32Type>("b".into()),
            GraphDefaultInputSlot::new::<F32Type>("a".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_r = ctx.get_input(0)?;
        let input_g = ctx.get_input(1)?;
        let input_b = ctx.get_input(2)?;
        let input_a = ctx.get_input(3)?;
        let output_color = ctx.get_output(0)?;

        Ok(format!(
            "{}\n",
            quote_statement! { let #output_color = vec4f(#input_r, #input_g, #input_b, #input_a); }
        ))
    }
}

/// Shader graph node that splits a color ([`ColorType`]) into its four scalar `r`, `g`, `b`, and `a` channels ([`F32Type`]).
#[derive(Default, Clone)]
pub struct SplitColorComponentsNode;

#[stateless]
impl StatelessCommonGraphNode for SplitColorComponentsNode {
    fn id(&self) -> &'static str {
        "split_color_components_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SplitColorComponentsNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<ColorType>("color".into())]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("r".into()),
            GraphDefaultOutputSlot::new::<F32Type>("g".into()),
            GraphDefaultOutputSlot::new::<F32Type>("b".into()),
            GraphDefaultOutputSlot::new::<F32Type>("a".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input_color = ctx.get_input(0)?;
        let output_r = ctx.get_output(0)?;
        let output_g = ctx.get_output(1)?;
        let output_b = ctx.get_output(2)?;
        let output_a = ctx.get_output(3)?;

        Ok(format!(
            "{}\n{}\n{}\n{}\n",
            quote_statement! { let #output_r = #input_color.r; },
            quote_statement! { let #output_g = #input_color.g; },
            quote_statement! { let #output_b = #input_color.b; },
            quote_statement! { let #output_a = #input_color.a; }
        ))
    }
}
