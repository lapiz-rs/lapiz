use std::sync::{Arc, LazyLock};

use lapiz_assets::store::AssetRegistry;
use lapiz_effect::nodes::effect_nodes;
use lapiz_shader_graph::{
    graph::{
        GraphResources,
        node::{
            GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeRegistry, StatelessCommonGraphNode, stateless,
        },
        slot::{GraphDefaultInputSlot, GraphDefaultOutputSlot},
        variable::GraphTypeRegistry,
    },
    wgsl_std::{builtin_types, types::vector::Vec2IType},
};
use lapiz_utils::random_oklch_hue_chroma;
use wesl::syntax::{
    Declaration, DeclarationKind, Expression, Ident, Span, Spanned, Statement, TypeExpression,
};
use wesl_quote::quote_statement;

/// Shader graph node that outputs the 2D integer pixel coordinates (`vec2<i32>`) of the current compute invocation.
#[derive(Default, Clone)]
pub struct PixelPositionNode;

#[stateless]
impl StatelessCommonGraphNode for PixelPositionNode {
    fn id(&self) -> &'static str {
        "pixel_position_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PixelPositionNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2IType>("position".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let output = ctx.get_output(0)?;
        Ok(quote_statement! {
            let #output = dispatch_index;
        }
        .to_string())
    }
}

pub fn filter_graph_nodes() -> GraphNodeRegistry {
    let mut nodes = effect_nodes();
    nodes.register::<PixelPositionNode>();
    nodes
}

pub fn filter_graph_types() -> GraphTypeRegistry {
    builtin_types()
}

pub fn filter_graph_resources(assets: AssetRegistry) -> GraphResources {
    GraphResources {
        type_registry: FILTER_GRAPH_TYPES.clone(),
        node_registry: FILTER_GRAPH_NODES.clone(),
        assets,
    }
}

pub static FILTER_GRAPH_NODES: LazyLock<Arc<GraphNodeRegistry>> =
    LazyLock::new(|| Arc::new(filter_graph_nodes()));

pub static FILTER_GRAPH_TYPES: LazyLock<Arc<GraphTypeRegistry>> =
    LazyLock::new(|| Arc::new(filter_graph_types()));
