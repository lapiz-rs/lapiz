use std::sync::{Arc, LazyLock};

use lapiz_assets::asset::AssetId;
use lapiz_utils::wrapper;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    graph::{Graph, node::GraphNodeRegistry, variable::GraphTypeRegistry},
    save::SerializableGraphFunction,
    wgsl_std::{
        builtin_nodes, builtin_types,
        nodes::graph_io::{GraphInputNode, GraphOutputNode},
    },
};

pub static GRAPH_FUNCTION_TYPE_REGISTRY: LazyLock<Arc<GraphTypeRegistry>> =
    LazyLock::new(|| Arc::new(builtin_types()));
pub static GRAPH_FUNCTION_NODE_REGISTRY: LazyLock<Arc<GraphNodeRegistry>> = LazyLock::new(|| {
    let mut nodes = builtin_nodes();
    nodes.register::<GraphInputNode>();
    nodes.register::<GraphOutputNode>();
    Arc::new(nodes)
});

wrapper! {
    /// A strongly-typed unique identifier for a [`GraphFunction`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    #[display("{0}")]
    pub GraphFunctionId : Uuid
}

/// A reusable, user-defined shader function backed by its own [`Graph`].
///
/// `GraphFunction` encapsulates a standalone subgraph whose external signature is defined by
/// [`GraphInputNode`] and [`GraphOutputNode`] instances, allowing it to be called from other
/// graphs via [`GraphFunctionNode`](crate::wgsl_std::nodes::function::GraphFunctionNode) and saved
/// as a [`SerializableGraphFunction`] asset.
pub struct GraphFunction {
    // FIXME This should always exist
    pub asset_id: Option<AssetId<SerializableGraphFunction>>,
    pub id: GraphFunctionId,
    pub name: String,
    pub graph: Graph,
}
