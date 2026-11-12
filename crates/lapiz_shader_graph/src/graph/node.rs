#![expect(
    clippy::pub_use,
    reason = "stateless attribute macro is exposed for graph node definitions"
)]

use std::{
    any::Any,
    collections::{BTreeMap, HashMap, hash_map::Entry},
    fmt,
    sync::Arc,
};

use anyhow::Result;
use downcast_rs::Downcast;
use dyn_clone::DynClone;
use iced_core::{Length, Point};
use lapiz_i18n::t;
pub use lapiz_shader_graph_derive::stateless;
use lapiz_utils::{cloneable_any::ClonableAnySync, wrapper};
use lapiz_widgets::{column, flex::Flex};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wesl::syntax::{Expression, Ident};

use crate::{
    GraphElement,
    editor::slot::{input_slot, output_slot},
    graph::{
        Graph, GraphResources, GraphSignature, GraphVarIdentGenerator,
        slot::{
            ErasedGraphLiteralUpdateMessage, GraphDefaultInputSlot, GraphDefaultOutputSlot,
            GraphInputSlotData, GraphInputSlotId, GraphOutputSlotData, GraphOutputSlotId,
            GraphSlots,
        },
        variable::{GraphLiteralValue, GraphVariable},
    },
    save::GraphSerializable,
};

wrapper! {
    /// A strongly-typed unique identifier for a node instance within a [`Graph`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub GraphNodeId : Uuid
}

/// The core trait defining a shader graph node's state, slot layout, editor UI, and WGSL code generation.
///
/// Implementors of `GraphNode` represent a distinct node type that can be registered in a
/// [`GraphNodeRegistry`] and instantiated inside a [`Graph`]. Nodes with no per-instance state can
/// implement [`StatelessCommonGraphNode`] (or use the `#[stateless]` attribute macro) instead of
/// implementing `GraphNode` directly.
///
/// # Lifecycle
///
/// - **Instantiation & Serialization**: [`default_state`](Self::default_state) creates the initial
///   [`State`](Self::State) when a node is added to a graph, while
///   [`serialize_state`](Self::serialize_state) and [`deserialize_state`](Self::deserialize_state)
///   persist it to and from TOML.
/// - **Slot Creation**: [`create_inputs`](Self::create_inputs) and
///   [`create_outputs`](Self::create_outputs) declare the node's input and output slots based on
///   its current state. Whenever [`update`](Self::update) mutates the state, the owning [`Graph`]
///   reconciles the node's slots against the newly returned definitions.
/// - **UI & Interaction**: [`view`](Self::view) renders the node's body in the graph editor and
///   emits [`Message`](Self::Message) values handled by [`update`](Self::update).
/// - **Compilation**: [`update_signature`](Self::update_signature) registers any external graph
///   inputs or outputs, [`extra_shader_body`](Self::extra_shader_body) injects top-level helper
///   declarations into the shader module, and [`generate_code`](Self::generate_code) emits the
///   WGSL statements for this node.
pub trait GraphNode: Send + Sync + 'static + DynClone {
    type State: Send + Sync + 'static + GraphSerializable;
    type Message: Send + Sync + 'static + Clone;

    fn id(&self) -> &'static str;
    fn default_state(&self, ctx: GraphNodeDefaultStateContext<'_>) -> Self::State;
    fn header_hue_chroma(&self) -> (f32, f32);
    fn create_inputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot>;
    fn create_outputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot>;
    fn update_signature(&self, _: &Self::State, _: GraphNodeUpdateSignatureContext<'_>) {}
    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'a, Self::Message>;
    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        ctx: GraphNodeUpdateContext<'_>,
    );
    fn extra_shader_body(&self, _: &Self::State, _: GraphNodeInjectShaderBodyContext<'_>) {}
    fn generate_code(
        &self,
        state: &Self::State,
        ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError>;
    fn serialize_state(&self, state: &Self::State) -> Result<toml::Value> {
        state.to_toml()
    }
    fn deserialize_state(
        &self,
        value: toml::Value,
        resources: &GraphResources,
    ) -> Result<Self::State> {
        Self::State::from_toml(value, resources)
    }
    fn subgraphs<'a>(&self, _state: &'a Self::State) -> Vec<&'a Graph> {
        Vec::new()
    }
    fn subgraphs_mut<'a>(&mut self, _state: &'a mut Self::State) -> Vec<&'a mut Graph> {
        Vec::new()
    }
}

/// A type-erased UI message emitted by a node view and tagged with the target [`GraphNodeId`].
#[derive(Clone)]
pub struct ErasedGraphNodeMessage {
    pub inner: Box<dyn ClonableAnySync>,
    pub id: GraphNodeId,
}

impl fmt::Debug for ErasedGraphNodeMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErasedGraphNodeMessage")
            .field("id", &self.id)
            .finish()
    }
}

/// Object-safe, type-erased counterpart to [`GraphNode`] for dynamic dispatch in [`Graph`].
pub trait ErasedGraphNode: Send + Sync + 'static + DynClone + Downcast {
    fn id(&self) -> &'static str;
    fn default_state(&self, ctx: GraphNodeDefaultStateContext<'_>) -> Box<dyn Any + Send + Sync>;
    fn header_hue_chroma(&self) -> (f32, f32);
    fn create_inputs(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot>;
    fn create_outputs(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot>;
    fn update_signature(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeUpdateSignatureContext<'_>,
    );
    fn view<'a>(
        &self,
        node_id: GraphNodeId,
        state: &'a (dyn Any + Send + Sync),
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'a, ErasedGraphNodeMessage>;
    fn update(
        &self,
        state: &mut (dyn Any + Send + Sync),
        message: ErasedGraphNodeMessage,
        ctx: GraphNodeUpdateContext<'_>,
    );
    fn extra_shader_body(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeInjectShaderBodyContext<'_>,
    );
    fn generate_code(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError>;
    fn serialize_state(&self, state: &(dyn Any + Send + Sync)) -> Result<toml::Value>;
    fn deserialize_state(
        &self,
        value: toml::Value,
        resources: &GraphResources,
    ) -> Result<Box<dyn Any + Send + Sync>>;
    fn subgraphs<'a>(&self, state: &'a (dyn Any + Send + Sync)) -> Vec<&'a Graph>;
    fn subgraphs_mut<'a>(&mut self, state: &'a mut (dyn Any + Send + Sync)) -> Vec<&'a mut Graph>;
}

dyn_clone::clone_trait_object!(ErasedGraphNode);
downcast_rs::impl_downcast!(ErasedGraphNode);

impl<T: GraphNode> ErasedGraphNode for T {
    fn id(&self) -> &'static str {
        self.id()
    }

    fn default_state(&self, ctx: GraphNodeDefaultStateContext<'_>) -> Box<dyn Any + Send + Sync> {
        Box::new(self.default_state(ctx))
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        self.header_hue_chroma()
    }

    fn create_inputs(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        self.create_inputs(
            state
                .downcast_ref::<T::State>()
                .expect("failed to downcast graph node state"),
            ctx,
        )
    }

    fn create_outputs(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        self.create_outputs(
            state
                .downcast_ref::<T::State>()
                .expect("failed to downcast graph node state"),
            ctx,
        )
    }

    fn update_signature(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeUpdateSignatureContext<'_>,
    ) {
        self.update_signature(
            state
                .downcast_ref::<T::State>()
                .expect("failed to downcast graph node state"),
            ctx,
        );
    }

    fn view<'a>(
        &self,
        node_id: GraphNodeId,
        state: &'a (dyn Any + Send + Sync),
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'a, ErasedGraphNodeMessage> {
        self.view(
            state
                .downcast_ref::<T::State>()
                .expect("failed to downcast graph node state"),
            ctx,
        )
        .map(move |message| ErasedGraphNodeMessage {
            inner: Box::new(message),
            id: node_id,
        })
    }

    fn update(
        &self,
        state: &mut (dyn Any + Send + Sync),
        message: ErasedGraphNodeMessage,
        ctx: GraphNodeUpdateContext<'_>,
    ) {
        let state = state
            .downcast_mut::<T::State>()
            .expect("failed to downcast graph node state");
        let message = match message.inner.downcast::<T::Message>() {
            Ok(message) => message,
            Err(_) => panic!("failed to downcast graph node message"),
        };
        self.update(state, *message, ctx);
    }

    fn extra_shader_body(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeInjectShaderBodyContext<'_>,
    ) {
        let state = state
            .downcast_ref::<T::State>()
            .expect("failed to downcast graph node state");
        self.extra_shader_body(state, ctx)
    }

    fn generate_code(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        self.generate_code(
            state
                .downcast_ref::<T::State>()
                .expect("failed to downcast graph node state"),
            ctx,
        )
    }

    fn serialize_state(&self, state: &(dyn Any + Send + Sync)) -> Result<toml::Value> {
        self.serialize_state(
            state
                .downcast_ref::<T::State>()
                .expect("failed to downcast graph node state"),
        )
    }

    fn deserialize_state(
        &self,
        value: toml::Value,
        resources: &GraphResources,
    ) -> Result<Box<dyn Any + Send + Sync>> {
        Ok(Box::new(self.deserialize_state(value, resources)?))
    }

    fn subgraphs<'a>(&self, state: &'a (dyn Any + Send + Sync)) -> Vec<&'a Graph> {
        self.subgraphs(
            state
                .downcast_ref::<T::State>()
                .expect("failed to downcast graph node state"),
        )
    }

    fn subgraphs_mut<'a>(&mut self, state: &'a mut (dyn Any + Send + Sync)) -> Vec<&'a mut Graph> {
        self.subgraphs_mut(
            state
                .downcast_mut::<T::State>()
                .expect("failed to downcast graph node state"),
        )
    }
}

/// A type-erased [`ErasedGraphNode`] paired with its boxed per-instance state.
///
/// `StatefulGraphNode` encapsulates the runtime state and behavior of a node inside a [`Graph`],
/// forwarding slot creation, view rendering, state updates, serialization, and code generation
/// to the underlying [`GraphNode`] implementation while allowing typed downcasting via
/// [`state`](Self::state) and [`state_mut`](Self::state_mut).
pub struct StatefulGraphNode {
    state: Box<dyn Any + Send + Sync>,
    data: Box<dyn ErasedGraphNode>,
}

impl StatefulGraphNode {
    pub fn new(node: Box<dyn ErasedGraphNode>, ctx: GraphNodeDefaultStateContext<'_>) -> Self {
        Self {
            state: node.default_state(ctx),
            data: node,
        }
    }

    pub fn id(&self) -> &'static str {
        self.data.id()
    }

    pub fn header_hue_chroma(&self) -> (f32, f32) {
        self.data.header_hue_chroma()
    }

    pub fn view<'a>(
        &'a self,
        node_id: GraphNodeId,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'a, ErasedGraphNodeMessage> {
        self.data.view(node_id, self.state.as_ref(), ctx)
    }

    pub fn update(&mut self, message: ErasedGraphNodeMessage, ctx: GraphNodeUpdateContext<'_>) {
        self.data.update(self.state.as_mut(), message, ctx);
    }

    pub fn generate_code(
        &self,
        ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        self.data.generate_code(self.state.as_ref(), ctx)
    }

    pub fn extra_shader_body(&self, ctx: GraphNodeInjectShaderBodyContext<'_>) {
        self.data.extra_shader_body(self.state.as_ref(), ctx)
    }

    pub fn serialize_state(&self) -> Result<toml::Value> {
        self.data.serialize_state(self.state.as_ref())
    }

    pub fn deserialize_and_set_state(
        &mut self,
        value: toml::Value,
        resources: &GraphResources,
    ) -> Result<()> {
        self.state = self.data.deserialize_state(value, resources)?;
        Ok(())
    }

    pub fn subgraphs(&self) -> Vec<&Graph> {
        self.data.subgraphs(self.state.as_ref())
    }

    pub fn subgraphs_mut(&mut self) -> Vec<&mut Graph> {
        self.data.subgraphs_mut(self.state.as_mut())
    }

    pub fn create_inputs(
        &self,
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        self.data.create_inputs(self.state.as_ref(), ctx)
    }

    pub fn create_outputs(
        &self,
        ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        self.data.create_outputs(self.state.as_ref(), ctx)
    }

    pub fn update_signature(&self, ctx: GraphNodeUpdateSignatureContext<'_>) {
        self.data.update_signature(self.state.as_ref(), ctx);
    }

    pub fn is<T: GraphNode>(&self) -> bool {
        self.data.downcast_ref::<T>().is_some()
    }

    pub fn state<T: GraphNode>(&self) -> Option<&T::State> {
        self.state.downcast_ref()
    }

    pub fn state_mut<T: GraphNode>(&mut self) -> Option<&mut T::State> {
        self.state.downcast_mut()
    }
}

/// Empty state type used by stateless [`GraphNode`] implementations.
#[derive(Default, Serialize, Deserialize)]
pub struct StatelessState {
    #[serde(skip)]
    _private: (),
}

/// A simplified trait for [`GraphNode`] implementations that have no per-instance state or custom controls.
///
/// Types annotated with `#[stateless]` implement this trait and automatically receive a blanket-like
/// [`GraphNode`] implementation backed by [`StatelessState`] and a default slot-list view.
pub trait StatelessCommonGraphNode: Send + Sync + 'static + DynClone {
    fn id(&self) -> &'static str;
    fn header_hue_chroma(&self) -> (f32, f32);
    fn create_inputs(&self, ctx: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot>;
    fn create_outputs(&self, ctx: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot>;
    fn update_signature(&self, _: GraphNodeUpdateSignatureContext<'_>) {}
    fn extra_shader_body(&self, _: GraphNodeInjectShaderBodyContext<'_>) {}
    fn generate_code(
        &self,
        ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError>;
}

/// Per-node entry stored in a [`Graph`], combining canvas position, [`StatefulGraphNode`], and slot IDs.
pub struct GraphNodeData {
    pub position: Point,
    pub data: StatefulGraphNode,
    pub inputs: Arc<[GraphInputSlotId]>,
    pub outputs: Arc<[GraphOutputSlotId]>,
}

impl GraphNodeData {
    pub fn view<'a>(
        &'a self,
        node_id: GraphNodeId,
        slots: &GraphSlots,
        resources: &GraphResources,
    ) -> GraphElement<'a, ErasedGraphNodeMessage> {
        self.data.view(
            node_id,
            GraphNodeViewContext {
                inputs: &self.inputs,
                outputs: &self.outputs,
                slots,
                resources,
            },
        )
    }
}

/// Top-level WGSL declarations (such as helper functions or structs) injected into a compiled shader module.
///
/// Nodes can register `ExtraShaderBody` items during compilation via
/// [`GraphNodeInjectShaderBodyContext::inject_extra_body`]. Each item is deduplicated by its
/// [`key`](Self::key) so shared helper definitions are emitted only once per shader module.
pub trait ExtraShaderBody: Send + Sync + 'static {
    fn key(&self) -> String;
    fn inject_body(&self, shader: &mut String);
}

/// Context passed to [`GraphNode::default_state`] when initializing a new node's state.
#[derive(Clone, Copy)]
pub struct GraphNodeDefaultStateContext<'a> {
    pub resources: &'a GraphResources,
}

/// Context passed to [`GraphNode::create_inputs`] and [`GraphNode::create_outputs`].
pub struct GraphNodeCreateSlotsContext<'a> {
    pub resources: &'a GraphResources,
}

/// Context passed to [`GraphNode::view`] providing access to the node's slots and shared [`GraphResources`].
pub struct GraphNodeViewContext<'a> {
    pub inputs: &'a [GraphInputSlotId],
    pub outputs: &'a [GraphOutputSlotId],
    pub slots: &'a GraphSlots,
    pub resources: &'a GraphResources,
}

impl GraphNodeViewContext<'_> {
    pub fn get_input(&self, index: usize) -> Option<&GraphInputSlotData> {
        self.slots.get_input(self.inputs.get(index)?)
    }

    pub fn get_output(&self, index: usize) -> Option<&GraphOutputSlotData> {
        self.slots.get_output(self.outputs.get(index)?)
    }

    pub fn view_input_slot<'a, Message: 'static>(
        &self,
        index: usize,
        map_literal: impl Fn(ErasedGraphLiteralUpdateMessage) -> Message + Copy + 'static,
    ) -> Option<GraphElement<'a, Message>> {
        let slot_id = *self.inputs.get(index)?;
        let slot = self.slots.get_input(&slot_id)?;
        Some(input_slot(slot_id, t!(&slot.name), slot, &self.resources.assets).map(map_literal))
    }

    pub fn view_output_slot<'a, Message: 'static>(
        &self,
        index: usize,
    ) -> Option<GraphElement<'a, Message>> {
        let slot_id = *self.outputs.get(index)?;
        let slot = self.slots.get_output(&slot_id)?;
        Some(output_slot(slot_id, t!(&slot.name), slot))
    }

    pub fn view_all_inputs<'a, Message: 'static>(
        &self,
        map_literal: impl Fn(ErasedGraphLiteralUpdateMessage) -> Message + Copy + 'static,
    ) -> Vec<GraphElement<'a, Message>> {
        self.inputs
            .iter()
            .filter_map(|id| {
                let slot = self.slots.get_input(id)?;
                Some(input_slot(*id, t!(&slot.name), slot, &self.resources.assets).map(map_literal))
            })
            .collect()
    }

    pub fn view_all_outputs<'a, Message: 'static>(&self) -> Vec<GraphElement<'a, Message>> {
        self.outputs
            .iter()
            .filter_map(|id| {
                let slot = self.slots.get_output(id)?;
                Some(output_slot(*id, t!(&slot.name), slot))
            })
            .collect()
    }

    pub fn view_all_slots<'a, Message: 'static>(
        &self,
        map_literal: impl Fn(ErasedGraphLiteralUpdateMessage) -> Message + Copy + 'static,
    ) -> GraphElement<'a, Message> {
        column!()
            .push(
                Flex::column(self.view_all_inputs(map_literal))
                    .width(Length::Fill)
                    .gap(4.0),
            )
            .push(
                Flex::column(self.view_all_outputs())
                    .width(Length::Fill)
                    .gap(4.0),
            )
            .width(Length::Fill)
            .gap(2.0)
            .into()
    }

    pub fn view_all_slots_with_header<'a, Message: 'static>(
        &self,
        header: impl Into<GraphElement<'a, Message>>,
        map_literal: impl Fn(ErasedGraphLiteralUpdateMessage) -> Message + Copy + 'static,
    ) -> GraphElement<'a, Message> {
        column!()
            .push(header)
            .push(self.view_all_slots(map_literal))
            .width(Length::Fill)
            .gap(2.0)
            .into()
    }

    pub fn all_inputs(&self) -> impl Iterator<Item = (&GraphInputSlotId, &GraphInputSlotData)> {
        self.inputs
            .iter()
            .filter_map(move |id| self.slots.get_input(id).map(|slot| (id, slot)))
    }

    pub fn all_outputs(&self) -> impl Iterator<Item = (&GraphOutputSlotId, &GraphOutputSlotData)> {
        self.outputs
            .iter()
            .filter_map(move |id| self.slots.get_output(id).map(|slot| (id, slot)))
    }
}

/// Context passed to [`GraphNode::update`] for mutating input slot literals and accessing [`GraphResources`].
pub struct GraphNodeUpdateContext<'a> {
    pub inputs: &'a [GraphInputSlotId],
    pub slots: &'a mut GraphSlots,
    pub resources: &'a GraphResources,
}

impl GraphNodeUpdateContext<'_> {
    pub fn get_input(&self, index: usize) -> Option<&GraphInputSlotData> {
        self.slots.get_input(self.inputs.get(index)?)
    }

    pub fn get_input_mut(&mut self, index: usize) -> Option<&mut GraphInputSlotData> {
        let slot_id = self.inputs.get(index)?;
        self.slots.inputs.get_mut(slot_id)
    }

    pub fn update_literal(&mut self, message: ErasedGraphLiteralUpdateMessage) {
        let Some(slot) = self.slots.inputs.get_mut(&message.id) else {
            return;
        };
        slot.data.update(message);
    }
}

/// Context passed to [`GraphNode::update_signature`] for declaring external graph inputs and outputs.
pub struct GraphNodeUpdateSignatureContext<'a> {
    pub inputs: &'a [GraphInputSlotId],
    pub outputs: &'a [GraphOutputSlotId],
    pub slots: &'a GraphSlots,
    pub signature: &'a mut GraphSignature,
    pub resources: &'a GraphResources,
}

impl GraphNodeUpdateSignatureContext<'_> {
    pub fn require_output_slot_as_graph_input(&mut self, index: usize, name: String) {
        let Some(slot_id) = self.outputs.get(index) else {
            return;
        };
        let Some(slot) = self.slots.outputs.get(slot_id) else {
            return;
        };

        self.signature.inputs.insert(
            *slot_id,
            GraphVariable::new_boxed(name, slot.data_ty.clone()),
        );
    }

    pub fn require_input_slot_as_graph_output(&mut self, index: usize, name: String) {
        let Some(slot_id) = self.inputs.get(index) else {
            return;
        };
        let Some(slot) = self.slots.inputs.get(slot_id) else {
            return;
        };

        self.signature.outputs.insert(
            *slot_id,
            GraphVariable::new_boxed(name, slot.data.ty().clone()),
        );
    }
}

/// Context passed to [`GraphNode::extra_shader_body`] for registering [`ExtraShaderBody`] declarations.
pub struct GraphNodeInjectShaderBodyContext<'a> {
    pub inputs: &'a [GraphInputSlotId],
    pub outputs: &'a [GraphOutputSlotId],
    pub graph_slots: &'a GraphSlots,
    pub resources: &'a GraphResources,
    pub extra_body: &'a mut HashMap<String, Box<dyn ExtraShaderBody>>,
}

impl GraphNodeInjectShaderBodyContext<'_> {
    pub fn inject_extra_body<T: ExtraShaderBody>(&mut self, body: T) {
        self.extra_body.insert(body.key(), Box::new(body));
    }
}

/// Context passed to [`GraphNode::generate_code`] for resolving input expressions and allocating output identifiers.
pub struct GraphNodeCodeGenContext<'a> {
    pub inputs: &'a [GraphInputSlotId],
    pub outputs: &'a [GraphOutputSlotId],
    pub graph_slots: &'a GraphSlots,
    pub output_slot_idents: &'a mut HashMap<GraphOutputSlotId, Expression>,
    pub ident_generator: &'a mut GraphVarIdentGenerator,
    pub resources: &'a GraphResources,
}

impl GraphNodeCodeGenContext<'_> {
    pub fn get_input(&self, index: usize) -> Result<Expression, GraphNodeCodeGenError> {
        let slot_id = self
            .inputs
            .get(index)
            .ok_or(GraphNodeCodeGenError::SlotIndexOutOfBounds)?;
        let slot = self
            .graph_slots
            .get_input(slot_id)
            .ok_or(GraphNodeCodeGenError::MissingInputSlot)?;
        let Some(connected) = slot.connected else {
            return slot
                .data
                .to_code()
                .ok_or(GraphNodeCodeGenError::LiteralToCodeFailed);
        };
        let output_slot = self
            .graph_slots
            .get_output(&connected)
            .ok_or(GraphNodeCodeGenError::MissingOutputSlot)?;
        let ident = self
            .output_slot_idents
            .get(&connected)
            .ok_or(GraphNodeCodeGenError::MissingOutputSlot)?;

        if output_slot.data_ty.id() != slot.data.ty().id() {
            self.resources
                .type_registry
                .try_wgsl_cast(
                    &*output_slot.data_ty,
                    slot.data.ty().as_ref(),
                    ident.clone(),
                )
                .ok_or(GraphNodeCodeGenError::FailedToCastVariable)
        } else {
            Ok(ident.clone())
        }
    }

    pub fn get_input_raw<T: GraphLiteralValue>(
        &self,
        index: usize,
    ) -> Result<&T, GraphNodeCodeGenError> {
        let slot_id = self
            .inputs
            .get(index)
            .ok_or(GraphNodeCodeGenError::SlotIndexOutOfBounds)?;
        let slot = self
            .graph_slots
            .get_input(slot_id)
            .ok_or(GraphNodeCodeGenError::MissingInputSlot)?;
        Ok(slot.data.as_ref::<T>())
    }

    pub fn get_output(&mut self, index: usize) -> Result<Ident, GraphNodeCodeGenError> {
        let slot_id = self
            .outputs
            .get(index)
            .ok_or(GraphNodeCodeGenError::SlotIndexOutOfBounds)?;
        Ok(match self.output_slot_idents.entry(*slot_id) {
            Entry::Occupied(entry) => match entry.get() {
                Expression::TypeOrIdentifier(ty)
                    if ty.path.is_none() && ty.template_args.is_none() =>
                {
                    ty.ident.clone()
                }
                _ => return Err(GraphNodeCodeGenError::OutputSlotHoldsExpression),
            },
            Entry::Vacant(entry) => {
                let ident = Ident::new(self.ident_generator.next_output());
                entry.insert(ident.clone().into());
                ident
            }
        })
    }
}

/// An error returned by [`GraphNode::generate_code`] when WGSL code generation for a node fails.
#[derive(Debug, thiserror::Error)]
pub enum GraphNodeCodeGenError {
    #[error("Input slot index out of bounds")]
    SlotIndexOutOfBounds,
    #[error("Missing input slot")]
    MissingInputSlot,
    #[error("Missing output slot")]
    MissingOutputSlot,
    #[error("Output slot already holds a derived expression")]
    OutputSlotHoldsExpression,
    #[error("Failed to cast variable")]
    FailedToCastVariable,
    #[error("Failed to convert literal to code")]
    LiteralToCodeFailed,
    #[error(transparent)]
    Custom(#[from] anyhow::Error),
}

/// A [`GraphNodeCodeGenError`] annotated with the failing node's ID, type identifier, and partial shader code.
#[derive(Debug)]
pub struct ContextualGraphNodeCodeGenError {
    pub node_id: GraphNodeId,
    pub node_title: String,
    pub err: GraphNodeCodeGenError,
    pub code: String,
}

impl fmt::Display for ContextualGraphNodeCodeGenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Error in node {:?} of type {}: {}\nCode already generated:\n{}",
            self.node_id, self.node_title, self.err, self.code
        )
    }
}

/// A registry of prototype [`GraphNode`] instances keyed by their static string identifier.
///
/// `GraphNodeRegistry` is stored in [`GraphResources`] and used by the node creation menu and
/// graph deserializer to look up and clone node prototypes by name.
#[derive(Default)]
pub struct GraphNodeRegistry {
    nodes: BTreeMap<&'static str, Box<dyn ErasedGraphNode>>,
}

impl Clone for GraphNodeRegistry {
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
        }
    }
}

impl GraphNodeRegistry {
    pub fn with_capacity() -> Self {
        Self {
            nodes: BTreeMap::new(),
        }
    }

    pub fn register<T: ErasedGraphNode + Default>(&mut self) {
        let node = Box::new(T::default());
        self.nodes.insert(node.id(), node);
    }

    pub fn register_boxed(&mut self, node: Box<dyn ErasedGraphNode>) {
        self.nodes.insert(node.id(), node);
    }

    pub fn get(&self, name: &str) -> Option<Box<dyn ErasedGraphNode>> {
        self.nodes.get(name).cloned()
    }

    pub fn all(&self) -> &BTreeMap<&'static str, Box<dyn ErasedGraphNode>> {
        &self.nodes
    }

    pub fn merge(&mut self, other: GraphNodeRegistry) {
        self.nodes.extend(other.nodes);
    }
}
