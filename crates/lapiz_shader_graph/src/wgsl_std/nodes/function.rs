use std::{
    fmt,
    sync::atomic::{AtomicU32, Ordering},
};

use iced_core::Length;
use lapiz_assets::{
    asset::{AssetHandle, AssetId},
    store::AssetRegistry,
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::combo_box;
use serde::{Deserialize, Serialize};

use crate::{
    GraphElement,
    graph::{
        Graph, GraphResources, GraphVarIdentGenerator,
        function::{GRAPH_FUNCTION_NODE_REGISTRY, GRAPH_FUNCTION_TYPE_REGISTRY},
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeInjectShaderBodyContext, GraphNodeUpdateContext,
            GraphNodeViewContext,
        },
        slot::{ErasedGraphLiteralUpdateMessage, GraphDefaultInputSlot, GraphDefaultOutputSlot},
    },
    save::{GraphSerializable, SerializableGraphFunction},
};

static UNIQUE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A shader graph node that invokes a reusable [`SerializableGraphFunction`] asset inline.
///
/// `GraphFunctionNode` dynamically mirrors the input and output signature of the selected function
/// graph and compiles the function's body with a unique identifier suffix during code generation.
#[derive(Default, Clone)]
pub struct GraphFunctionNode;

/// A selectable reference to a [`SerializableGraphFunction`] asset displayed in [`GraphFunctionNode`].
#[derive(Clone)]
pub struct GraphFunctionReference {
    pub handle: AssetHandle<SerializableGraphFunction>,
    pub name: String,
}

impl fmt::Display for GraphFunctionReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl PartialEq for GraphFunctionReference {
    fn eq(&self, other: &Self) -> bool {
        self.handle.id() == other.handle.id()
    }
}

/// Per-instance state for [`GraphFunctionNode`], holding the selected function asset handle and its instantiated [`Graph`].
pub struct GraphFunctionNodeState {
    pub handle: Option<AssetHandle<SerializableGraphFunction>>,
    /// Instantiated when the handle is set (update or deserialize) and cached
    /// for signature reads and code generation.
    pub cached_graph: Option<Graph>,
}

/// UI message emitted by [`GraphFunctionNode`].
#[derive(Clone)]
pub enum GraphFunctionNodeMessage {
    FunctionChanged(GraphFunctionReference),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphFunctionNodeState {
    fn instantiate(
        handle: &AssetHandle<SerializableGraphFunction>,
        assets: &AssetRegistry,
    ) -> Option<Graph> {
        let function = handle.get().ok()?;
        let resources = GraphResources {
            type_registry: GRAPH_FUNCTION_TYPE_REGISTRY.clone(),
            node_registry: GRAPH_FUNCTION_NODE_REGISTRY.clone(),
            assets: assets.clone(),
        };
        let (graph, errors) = Graph::from_serialized(&function.graph, resources);
        if !errors.is_empty() {
            log::error!(
                "function '{}' failed to deserialize: {:?}",
                function.name,
                errors
            );
            return None;
        }
        graph
    }
}

impl GraphSerializable for GraphFunctionNodeState {
    fn to_toml(&self) -> anyhow::Result<toml::Value> {
        #[derive(Serialize)]
        struct Serializable {
            asset: Option<AssetId<SerializableGraphFunction>>,
        }
        Ok(toml::Value::try_from(Serializable {
            asset: self.handle.as_ref().map(|handle| handle.id()),
        })?)
    }

    fn from_toml(value: toml::Value, resources: &GraphResources) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct Serializable {
            asset: Option<AssetId<SerializableGraphFunction>>,
        }
        let serialized = Serializable::deserialize(value)?;
        let handle = serialized
            .asset
            .map(|id| resources.assets.handle(id))
            .transpose()?;
        let graph = handle
            .as_ref()
            .and_then(|handle| Self::instantiate(handle, &resources.assets));
        Ok(Self {
            handle,
            cached_graph: graph,
        })
    }
}

impl GraphNode for GraphFunctionNode {
    type State = GraphFunctionNodeState;
    type Message = GraphFunctionNodeMessage;

    fn id(&self) -> &'static str {
        "function_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        GraphFunctionNodeState {
            handle: None,
            cached_graph: None,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(GraphFunctionNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        let Some(graph) = state.cached_graph.as_ref() else {
            return Vec::new();
        };
        graph
            .signature()
            .inputs
            .iter()
            .map(|(_, var)| {
                GraphDefaultInputSlot::new_boxed(var.identifier().to_string(), var.ty().clone())
            })
            .collect()
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        let Some(graph) = state.cached_graph.as_ref() else {
            return Vec::new();
        };
        graph
            .signature()
            .outputs
            .iter()
            .map(|(_, var)| {
                GraphDefaultOutputSlot::new_boxed(var.identifier().to_string(), var.ty().clone())
            })
            .collect()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        let functions = ctx
            .resources
            .assets
            .all_handles_of::<SerializableGraphFunction>()
            .unwrap_or_default()
            .into_iter()
            .map(|handle| GraphFunctionReference {
                name: handle
                    .get()
                    .map(|function| function.name.clone())
                    .unwrap_or_default(),
                handle,
            })
            .collect::<Vec<_>>();
        let selected = functions
            .iter()
            .find(|reference| Some(&reference.handle) == state.handle.as_ref())
            .cloned();
        ctx.view_all_slots_with_header(
            combo_box(
                functions,
                selected,
                GraphFunctionNodeMessage::FunctionChanged,
            )
            .width(Length::Fill),
            GraphFunctionNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            GraphFunctionNodeMessage::FunctionChanged(reference) => {
                state.cached_graph =
                    GraphFunctionNodeState::instantiate(&reference.handle, &ctx.resources.assets);
                state.handle = Some(reference.handle);
            }
            GraphFunctionNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn extra_shader_body(&self, state: &Self::State, ctx: GraphNodeInjectShaderBodyContext<'_>) {
        if let Some(graph) = &state.cached_graph {
            graph.collect_extra_shader_bodies(ctx.extra_body);
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let Some(graph) = state.cached_graph.as_ref() else {
            return Ok(Default::default());
        };

        let input_idents = (0..ctx.inputs.len()).try_fold(
            Vec::with_capacity(ctx.inputs.len()),
            |mut acc, i| {
                acc.push(ctx.get_input(i)?);
                Ok::<_, GraphNodeCodeGenError>(acc)
            },
        )?;

        let suffix = format!(
            "function_{}",
            UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let (output_idents, _, code, _) = graph
            .compile(input_idents, GraphVarIdentGenerator::new(suffix))
            .map_err(|e| GraphNodeCodeGenError::Custom(e.into()))?;

        for (slot_id, output_ident) in ctx.outputs.iter().zip(output_idents) {
            ctx.output_slot_idents.insert(*slot_id, output_ident);
        }

        Ok(code)
    }
}
