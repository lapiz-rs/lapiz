use std::sync::{Arc, LazyLock};

use lapiz_assets::AssetAppExt as _;
use lapiz_runtime::{Runtime, plugin::Plugin};

use crate::{
    asset::EffectAssetSerializer,
    shader_graph::{
        builtin::{
            builtin_nodes, builtin_types,
            nodes::effect_io::{DispatchIndexNode, PassInputNode, PassOutputNode},
        },
        graph::{node::GraphNodeRegistry, variable::GraphTypeRegistry},
    },
};

lapiz_i18n::define_i18n!("effect");

pub mod asset;
pub mod editor;
pub mod instance;
pub mod nodes;
pub mod render;
pub mod shader_graph;

/// Registers effect localization and the [`EffectAssetSerializer`] with the [`Runtime`].
pub struct EffectPlugin;

impl Plugin for EffectPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.globals_mut()
            .add_asset_serializer::<EffectAssetSerializer>();
    }
}

pub static EFFECT_GRAPH_TYPES: LazyLock<Arc<GraphTypeRegistry>> =
    LazyLock::new(|| Arc::new(effect_graph_types()));
pub static EFFECT_GRAPH_NODES: LazyLock<Arc<GraphNodeRegistry>> =
    LazyLock::new(|| Arc::new(effect_nodes()));

fn effect_graph_types() -> GraphTypeRegistry {
    let mut types = GraphTypeRegistry::default();
    types.merge(builtin_types());
    types
}

pub fn effect_nodes() -> GraphNodeRegistry {
    let mut nodes = GraphNodeRegistry::with_capacity();
    nodes.merge(builtin_nodes());
    nodes.register::<PassInputNode>();
    nodes.register::<PassOutputNode>();
    nodes.register::<DispatchIndexNode>();
    nodes
}
