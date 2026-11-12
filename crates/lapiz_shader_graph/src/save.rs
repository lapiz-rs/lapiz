use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet, hash_map::Entry},
    io::{self, Read, Write},
    path::Path,
    sync::Arc,
};

use anyhow::Result;
use iced_core::Point;
use lapiz_assets::{
    asset::{Asset, AssetId},
    loader::AssetSerializer,
    store::AssetRegistry,
};
use serde::{Deserialize, Serialize};
use toml::{de, ser};

use crate::graph::{
    Graph, GraphResources,
    function::{
        GRAPH_FUNCTION_NODE_REGISTRY, GRAPH_FUNCTION_TYPE_REGISTRY, GraphFunction, GraphFunctionId,
    },
    node::{
        GraphNodeCreateSlotsContext, GraphNodeData, GraphNodeDefaultStateContext, GraphNodeId,
        StatefulGraphNode,
    },
    slot::{
        GraphInputSlotData, GraphInputSlotId, GraphOutputSlotData, GraphOutputSlotId, GraphSlots,
    },
    variable::{GraphLiteral, GraphTypeRegistry},
};

/// Converts shader graph node states and auxiliary data to and from [`toml::Value`].
///
/// Types implementing [`Serialize`] and [`Deserialize`] automatically implement this trait, while
/// types that need access to [`GraphResources`] (such as asset handles or dynamic slot schemas)
/// during deserialization can provide a custom implementation.
pub trait GraphSerializable: Sized {
    fn to_toml(&self) -> Result<toml::Value>;
    fn from_toml(value: toml::Value, resources: &GraphResources) -> Result<Self>;
}

impl<'de, T: Serialize + Deserialize<'de>> GraphSerializable for T {
    fn to_toml(&self) -> Result<toml::Value> {
        Ok(toml::Value::try_from(self)?)
    }

    fn from_toml(value: toml::Value, _resources: &GraphResources) -> Result<Self> {
        Ok(Self::deserialize(value)?)
    }
}

/// An error encountered while reconstructing a [`Graph`] from its serialized representation.
#[derive(Debug, thiserror::Error)]
pub enum GraphDeserializeError {
    #[error("Value type not found: {0}")]
    TypeNotFound(String),
    #[error("Node type not found: {0}")]
    NodeNotFound(String),
    #[error("Unmatched input slot count on node {node:?}: expected {expected}, found {found}")]
    UnmatchedInputSlotCount {
        node: GraphNodeId,
        expected: usize,
        found: usize,
    },
    #[error(
        "Unmatched output slot count on node {node_name}({node:?}): expected {expected}, found {found}"
    )]
    UnmatchedOutputSlotCount {
        node: GraphNodeId,
        node_name: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("Missing input slot on node {0:?}: {1:?}")]
    MissingInputSlot(GraphNodeId, GraphInputSlotId),
    #[error("Missing output slot on node {0:?}: {1:?}")]
    MissingOutputSlot(GraphNodeId, GraphOutputSlotId),
    #[error("Input slot {1:?} on node {0:?} is connected to missing output slot {2:?}")]
    MissingConnectedSlot(GraphNodeId, GraphInputSlotId, GraphOutputSlotId),
    #[error("Failed to deserialize literal data: {0}")]
    LiteralDeserializeError(anyhow::Error),
    #[error("Failed to deserialize node state: {0}")]
    NodeStateDeserializeError(anyhow::Error),
    #[error("Deserialization error: {0}")]
    DeserializerError(de::Error),
}

impl Graph {
    pub fn to_toml(&self) -> Result<String, anyhow::Error> {
        let graph = self.as_serialized()?;
        Ok(toml::to_string(&graph)?)
    }

    pub fn from_toml(
        s: &str,
        resources: GraphResources,
    ) -> (Option<Self>, Vec<GraphDeserializeError>) {
        let graph = match toml::from_str::<SerializableGraph>(s) {
            Ok(g) => g,
            Err(e) => {
                return (None, vec![GraphDeserializeError::DeserializerError(e)]);
            }
        };
        Self::from_serialized(&graph, resources)
    }

    pub fn from_serialized(
        serialized: &SerializableGraph,
        resources: GraphResources,
    ) -> (Option<Self>, Vec<GraphDeserializeError>) {
        let SerializableGraph {
            nodes,
            inputs,
            outputs,
        } = serialized;

        let mut rs = (None, Vec::new());
        let errs = &mut rs.1;

        let mut graph_nodes = HashMap::with_capacity(nodes.len());
        let mut graph_inputs = HashMap::with_capacity(inputs.len());
        let mut graph_outputs = HashMap::with_capacity(outputs.len());

        'node_loop: for ser_node in nodes {
            let Some(node_inst) = resources.node_registry.get(&ser_node.data.name) else {
                errs.push(GraphDeserializeError::NodeNotFound(
                    ser_node.data.name.clone(),
                ));
                continue;
            };

            let mut node = StatefulGraphNode::new(
                node_inst,
                GraphNodeDefaultStateContext {
                    resources: &resources,
                },
            );
            match node.deserialize_and_set_state(ser_node.state.clone(), &resources) {
                Ok(_) => {}
                Err(e) => {
                    errs.push(GraphDeserializeError::NodeStateDeserializeError(e));
                    continue 'node_loop;
                }
            }

            let raw_inputs = node.create_inputs(GraphNodeCreateSlotsContext {
                resources: &resources,
            });
            if raw_inputs.len() != ser_node.inputs.len() {
                errs.push(GraphDeserializeError::UnmatchedInputSlotCount {
                    node: ser_node.id,
                    expected: raw_inputs.len(),
                    found: ser_node.inputs.len(),
                });
                continue;
            }
            let mut node_inputs = HashMap::with_capacity(raw_inputs.len());

            for (index, default) in raw_inputs.into_iter().enumerate() {
                let id = ser_node.inputs[index];
                let Some(slot) = inputs.get(&id) else {
                    errs.push(GraphDeserializeError::MissingInputSlot(ser_node.id, id));
                    continue 'node_loop;
                };

                let type_id = default.ty.id();
                let value_type_obj = match resources.type_registry.resolve_type(&type_id.id) {
                    Some(t) => t,
                    None => {
                        errs.push(GraphDeserializeError::TypeNotFound(type_id.id));
                        continue 'node_loop;
                    }
                };

                let literal_value = match value_type_obj
                    .deserialize_literal(slot.data.clone(), &resources.assets)
                {
                    Ok(val) => val,
                    Err(e) => {
                        errs.push(GraphDeserializeError::LiteralDeserializeError(e));
                        continue 'node_loop;
                    }
                };

                node_inputs.insert(
                    id,
                    GraphInputSlotData {
                        node_id: ser_node.id,
                        name: default.name,
                        data: GraphLiteral::new_boxed(literal_value, value_type_obj.clone()),
                        connected: slot.connected,
                    },
                );
            }

            let raw_outputs = node.create_outputs(GraphNodeCreateSlotsContext {
                resources: &resources,
            });
            if raw_outputs.len() != ser_node.outputs.len() {
                errs.push(GraphDeserializeError::UnmatchedOutputSlotCount {
                    node: ser_node.id,
                    node_name: node.id(),
                    expected: raw_outputs.len(),
                    found: ser_node.outputs.len(),
                });
                continue;
            }
            let mut node_outputs = HashMap::with_capacity(raw_outputs.len());

            for (index, default) in raw_outputs.into_iter().enumerate() {
                let id = ser_node.outputs[index];
                let Some(_) = outputs.get(&id) else {
                    errs.push(GraphDeserializeError::MissingOutputSlot(ser_node.id, id));
                    continue 'node_loop;
                };

                node_outputs.insert(
                    id,
                    GraphOutputSlotData {
                        node_id: ser_node.id,
                        name: default.name,
                        data_ty: default.ty,
                        connected: HashSet::new(),
                    },
                );
            }

            graph_nodes.insert(
                ser_node.id,
                GraphNodeData {
                    position: Point::new(ser_node.position[0], ser_node.position[1]),
                    data: node,
                    inputs: ser_node.inputs.clone(),
                    outputs: ser_node.outputs.clone(),
                },
            );
            graph_inputs.extend(node_inputs);
            graph_outputs.extend(node_outputs);
        }

        for (input_id, input) in &mut graph_inputs {
            if let Some(connected_id) = input.connected {
                match graph_outputs.entry(connected_id) {
                    Entry::Occupied(mut e) => {
                        e.get_mut().connected.insert(*input_id);
                    }
                    Entry::Vacant(_) => {
                        errs.push(GraphDeserializeError::MissingConnectedSlot(
                            input.node_id,
                            *input_id,
                            connected_id,
                        ));
                        input.connected = None;
                    }
                }
            }
        }

        rs.0 = Some(Graph {
            nodes: graph_nodes,
            slots: GraphSlots {
                inputs: graph_inputs,
                outputs: graph_outputs,
            },
            resources,
            cached_run_order: Default::default(),
            cached_signature: Default::default(),
        });

        rs
    }

    pub fn as_serialized(&self) -> Result<SerializableGraph, anyhow::Error> {
        let nodes = self
            .nodes
            .iter()
            .try_fold(Vec::new(), |mut acc, (node_id, node)| {
                acc.push(SerializableNodeData {
                    id: *node_id,
                    position: [node.position.x, node.position.y],
                    inputs: node.inputs.clone(),
                    outputs: node.outputs.clone(),
                    data: GraphNodeTypeId {
                        name: node.data.id().to_string(),
                    },
                    state: node.data.serialize_state()?,
                });

                Result::<_, anyhow::Error>::Ok(acc)
            })?;

        let inputs =
            self.slots
                .inputs
                .iter()
                .try_fold(HashMap::default(), |mut acc, (id, slot)| {
                    acc.insert(
                        *id,
                        SerializableInputSlotData {
                            connected: slot.connected,
                            data: slot
                                .data
                                .ty()
                                .serialize_literal(slot.data.value(), &self.resources.assets)?,
                        },
                    );

                    Result::<_, anyhow::Error>::Ok(acc)
                })?;

        let outputs = self
            .slots
            .outputs
            .keys()
            .map(|id| (*id, SerializableOutputSlotData {}))
            .collect();

        Ok(SerializableGraph {
            nodes,
            inputs,
            outputs,
        })
    }
}

/// The serializable DTO representation of a [`Graph`], including its nodes, input slots, and output slots.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SerializableGraph {
    pub nodes: Vec<SerializableNodeData>,
    pub inputs: HashMap<GraphInputSlotId, SerializableInputSlotData>,
    pub outputs: HashMap<GraphOutputSlotId, SerializableOutputSlotData>,
}

/// A string-based identifier that uniquely names a registered shader graph value type.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GraphValueTypeId {
    pub id: String,
}

impl GraphValueTypeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl Borrow<str> for GraphValueTypeId {
    fn borrow(&self) -> &str {
        &self.id
    }
}

/// A string-based identifier that uniquely names a registered shader graph node type.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct GraphNodeTypeId {
    pub name: String,
}

/// The serialized representation of a single node in a [`SerializableGraph`].
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializableNodeData {
    pub id: GraphNodeId,
    pub data: GraphNodeTypeId,
    pub position: [f32; 2],
    pub inputs: Arc<[GraphInputSlotId]>,
    pub outputs: Arc<[GraphOutputSlotId]>,
    pub state: toml::Value,
}

/// The serialized state of an input slot, storing its optional connection and fallback literal value.
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializableInputSlotData {
    pub connected: Option<GraphOutputSlotId>,
    pub data: toml::Value,
}

/// Marker DTO representing an output slot entry in a [`SerializableGraph`].
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializableOutputSlotData {}

/// The serialized signature of a shader graph function, including its name, return type, and parameters.
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializableGraphFunctionSignature {
    pub name: String,
    pub ret_type: GraphValueTypeId,
    pub params: Vec<SerializableGraphVariable>,
}

/// A serialized shader variable entry pairing an identifier with a [`GraphValueTypeId`].
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializableGraphVariable {
    pub identifier: String,
    pub ty: GraphValueTypeId,
}

/// An error that can occur when deserializing a [`SerializableGraphLiteral`].
#[derive(Debug, thiserror::Error)]
pub enum SerializableGraphLiteralError {
    #[error("Type not found: {0}")]
    TypeNotFound(String),
    #[error("Failed to deserialize literal data: {0}")]
    LiteralDeserializeError(#[from] anyhow::Error),
}

/// A type-tagged TOML value representing a serialized [`GraphLiteral`].
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializableGraphLiteral {
    pub ty: String,
    pub value: toml::Value,
}

impl SerializableGraphLiteral {
    pub fn serialize(
        literal: &GraphLiteral,
        assets: &AssetRegistry,
    ) -> Result<SerializableGraphLiteral> {
        Ok(SerializableGraphLiteral {
            ty: literal.ty().id().id,
            value: literal.ty().serialize_literal(literal.value(), assets)?,
        })
    }

    pub fn deserialize(
        &self,
        type_registry: &GraphTypeRegistry,
        assets: &AssetRegistry,
    ) -> Result<GraphLiteral, SerializableGraphLiteralError> {
        let ty = type_registry
            .resolve_type(&self.ty)
            .ok_or_else(|| SerializableGraphLiteralError::TypeNotFound(self.ty.clone()))?;

        let literal_value = ty.deserialize_literal(self.value.clone(), assets)?;

        Ok(GraphLiteral::new_boxed(literal_value, ty.clone()))
    }
}

/// A serializable [`Asset`] storing a reusable [`GraphFunction`] and its underlying [`SerializableGraph`].
#[derive(Serialize, Deserialize, Clone)]
pub struct SerializableGraphFunction {
    pub id: GraphFunctionId,
    pub name: String,
    pub graph: SerializableGraph,
}

impl SerializableGraphFunction {
    pub fn serialize_func(func: &GraphFunction) -> Result<Self, anyhow::Error> {
        Ok(SerializableGraphFunction {
            id: func.id,
            name: func.name.clone(),
            graph: func.graph.as_serialized()?,
        })
    }

    pub fn deserialize_func(
        &self,
        asset_id: Option<AssetId<SerializableGraphFunction>>,
        assets: &AssetRegistry,
    ) -> (Option<GraphFunction>, Vec<GraphDeserializeError>) {
        let resources = GraphResources {
            type_registry: GRAPH_FUNCTION_TYPE_REGISTRY.clone(),
            node_registry: GRAPH_FUNCTION_NODE_REGISTRY.clone(),
            assets: assets.clone(),
        };

        let (maybe_graph, err) = Graph::from_serialized(&self.graph, resources);

        let func = maybe_graph.map(|graph| GraphFunction {
            asset_id,
            id: self.id,
            name: self.name.clone(),
            graph,
        });

        (func, err)
    }
}

impl Asset for SerializableGraphFunction {
    const TYPE_NAME: &'static str = "shader_graph_function";
}

/// An [`AssetSerializer`] that reads and writes [`SerializableGraphFunction`] assets as `.lsf` TOML files.
#[derive(Default)]
pub struct SerializableGraphFunctionSerializer;

/// An error that can occur when reading or writing a [`SerializableGraphFunction`] asset.
#[derive(Debug, thiserror::Error)]
pub enum SerializableGraphFunctionSerializerError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),
    #[error("Serialization error: {0}")]
    TomlSer(#[from] ser::Error),
    #[error("Deserialization error: {0}")]
    TomlDe(#[from] de::Error),
}

impl AssetSerializer for SerializableGraphFunctionSerializer {
    type Asset = SerializableGraphFunction;

    fn file_extension() -> &'static str {
        "lsf"
    }

    fn read(&self,_relative_path: &Path, reader: &mut dyn Read) -> Result<Self::Asset> {
        let mut buf = String::new();
        reader.read_to_string(&mut buf)?;
        Ok(toml::from_str(&buf)?)
    }

    fn write(&self, asset: &Self::Asset, writer: &mut dyn Write) -> Result<()> {
        let serialized = toml::to_string(asset)?;
        writer.write_all(serialized.as_bytes())?;
        Ok(())
    }
}
