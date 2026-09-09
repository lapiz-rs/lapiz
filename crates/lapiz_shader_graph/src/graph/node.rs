use std::{
    any::Any,
    collections::{BTreeMap, HashMap, hash_map::Entry},
    marker::PhantomData,
    sync::Arc,
};

use anyhow::Result;
use downcast_rs::Downcast;
use dyn_clone::DynClone;
use iced_core::{Length, Point};
use iced_widget::Column;
use lapiz_i18n::t;
pub use lapiz_shader_graph_derive::stateless;
use lapiz_utils::{cloneable_any::ClonableAnySync, wrapper};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    GraphElement,
    editor::slot::{input_slot, output_slot},
    graph::{
        Graph, GraphData, GraphResources, GraphSignature, GraphVarIdentGenerator,
        slot::{
            ErasedGraphLiteralUpdateMessage, GraphDefaultInputSlot, GraphDefaultOutputSlot,
            GraphInputSlotData, GraphInputSlotId, GraphOutputSlotData, GraphOutputSlotId,
            GraphSlots,
        },
        texture::GraphTextureUsageRecorder,
        variable::{GraphLiteralValue, GraphVariable},
    },
    save::GraphSerializable,
};

wrapper! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub GraphNodeId : Uuid
}

pub trait GraphNode<Data: GraphData>: Send + Sync + 'static + DynClone {
    type State: Send + Sync + 'static + GraphSerializable<Data>;
    type Message: Send + Sync + 'static + Clone;

    fn id(&self) -> &'static str;
    fn default_state(&self, ctx: GraphNodeDefaultStateContext<'_, Data>) -> Self::State;
    fn header_hue_chroma(&self) -> (f32, f32);
    fn create_inputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot>;
    fn create_outputs(
        &self,
        state: &Self::State,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot>;
    fn update_signature(&self, _: &Self::State, _: GraphNodeUpdateSignatureContext<'_, Data>) {}
    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'a, Self::Message>;
    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        ctx: GraphNodeUpdateContext<'_, Data>,
    );
    fn generate_code(
        &self,
        state: &Self::State,
        ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError>;
    fn serialize_state(&self, state: &Self::State) -> Result<toml::Value> {
        state.to_toml()
    }
    fn deserialize_state(
        &self,
        value: toml::Value,
        resources: &GraphResources<Data>,
    ) -> Result<Self::State> {
        Self::State::from_toml(value, resources)
    }
    fn subgraphs<'a>(&self, _state: &'a Self::State) -> Vec<&'a Graph<Data>> {
        Vec::new()
    }
    fn subgraphs_mut<'a>(&mut self, _state: &'a mut Self::State) -> Vec<&'a mut Graph<Data>> {
        Vec::new()
    }
}

#[derive(Clone)]
pub struct ErasedGraphNodeMessage {
    pub inner: Box<dyn ClonableAnySync>,
    pub id: GraphNodeId,
}

impl std::fmt::Debug for ErasedGraphNodeMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErasedGraphNodeMessage")
            .field("id", &self.id)
            .finish()
    }
}

pub trait ErasedGraphNode<Data: GraphData>: Send + Sync + 'static + DynClone + Downcast {
    fn id(&self) -> &'static str;
    fn default_state(
        &self,
        ctx: GraphNodeDefaultStateContext<'_, Data>,
    ) -> Box<dyn Any + Send + Sync>;
    fn header_hue_chroma(&self) -> (f32, f32);
    fn create_inputs(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot>;
    fn create_outputs(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot>;
    fn update_signature(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeUpdateSignatureContext<'_, Data>,
    );
    fn view<'a>(
        &self,
        node_id: GraphNodeId,
        state: &'a (dyn Any + Send + Sync),
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'a, ErasedGraphNodeMessage>;
    fn update(
        &self,
        state: &mut (dyn Any + Send + Sync),
        message: ErasedGraphNodeMessage,
        ctx: GraphNodeUpdateContext<'_, Data>,
    );
    fn generate_code(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError>;
    fn serialize_state(&self, state: &(dyn Any + Send + Sync)) -> Result<toml::Value>;
    fn deserialize_state(
        &self,
        value: toml::Value,
        resources: &GraphResources<Data>,
    ) -> Result<Box<dyn Any + Send + Sync>>;
    fn subgraphs<'a>(&self, state: &'a (dyn Any + Send + Sync)) -> Vec<&'a Graph<Data>>;
    fn subgraphs_mut<'a>(
        &mut self,
        state: &'a mut (dyn Any + Send + Sync),
    ) -> Vec<&'a mut Graph<Data>>;
}

dyn_clone::clone_trait_object!(<Data> ErasedGraphNode<Data>);
downcast_rs::impl_downcast!(ErasedGraphNode<Data> where Data: GraphData);

impl<T: GraphNode<Data>, Data: GraphData> ErasedGraphNode<Data> for T {
    fn id(&self) -> &'static str {
        self.id()
    }

    fn default_state(
        &self,
        ctx: GraphNodeDefaultStateContext<'_, Data>,
    ) -> Box<dyn Any + Send + Sync> {
        Box::new(self.default_state(ctx))
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        self.header_hue_chroma()
    }

    fn create_inputs(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
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
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
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
        ctx: GraphNodeUpdateSignatureContext<'_, Data>,
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
        ctx: GraphNodeViewContext<'_, Data>,
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
        ctx: GraphNodeUpdateContext<'_, Data>,
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

    fn generate_code(
        &self,
        state: &(dyn Any + Send + Sync),
        ctx: GraphNodeCodeGenContext<'_, Data>,
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
        resources: &GraphResources<Data>,
    ) -> Result<Box<dyn Any + Send + Sync>> {
        Ok(Box::new(self.deserialize_state(value, resources)?))
    }

    fn subgraphs<'a>(&self, state: &'a (dyn Any + Send + Sync)) -> Vec<&'a Graph<Data>> {
        self.subgraphs(
            state
                .downcast_ref::<T::State>()
                .expect("failed to downcast graph node state"),
        )
    }

    fn subgraphs_mut<'a>(
        &mut self,
        state: &'a mut (dyn Any + Send + Sync),
    ) -> Vec<&'a mut Graph<Data>> {
        self.subgraphs_mut(
            state
                .downcast_mut::<T::State>()
                .expect("failed to downcast graph node state"),
        )
    }
}

pub struct StatefulGraphNode<Data: GraphData> {
    state: Box<dyn Any + Send + Sync>,
    data: Box<dyn ErasedGraphNode<Data>>,
}

impl<Data: GraphData> StatefulGraphNode<Data> {
    pub fn new(
        node: Box<dyn ErasedGraphNode<Data>>,
        ctx: GraphNodeDefaultStateContext<'_, Data>,
    ) -> Self {
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
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> GraphElement<'a, ErasedGraphNodeMessage> {
        self.data.view(node_id, self.state.as_ref(), ctx)
    }

    pub fn update(
        &mut self,
        message: ErasedGraphNodeMessage,
        ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        self.data.update(self.state.as_mut(), message, ctx);
    }

    pub fn generate_code(
        &self,
        ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        self.data.generate_code(self.state.as_ref(), ctx)
    }

    pub fn serialize_state(&self) -> Result<toml::Value> {
        self.data.serialize_state(self.state.as_ref())
    }

    pub fn deserialize_and_set_state(
        &mut self,
        value: toml::Value,
        resources: &GraphResources<Data>,
    ) -> Result<()> {
        self.state = self.data.deserialize_state(value, resources)?;
        Ok(())
    }

    pub fn subgraphs(&self) -> Vec<&Graph<Data>> {
        self.data.subgraphs(self.state.as_ref())
    }

    pub fn subgraphs_mut(&mut self) -> Vec<&mut Graph<Data>> {
        self.data.subgraphs_mut(self.state.as_mut())
    }

    pub fn create_inputs(
        &self,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        self.data.create_inputs(self.state.as_ref(), ctx)
    }

    pub fn create_outputs(
        &self,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        self.data.create_outputs(self.state.as_ref(), ctx)
    }

    pub fn update_signature(&self, ctx: GraphNodeUpdateSignatureContext<'_, Data>) {
        self.data.update_signature(self.state.as_ref(), ctx);
    }

    pub fn is<T: GraphNode<Data>>(&self) -> bool {
        self.data.downcast_ref::<T>().is_some()
    }

    pub fn state<T: GraphNode<Data>>(&self) -> Option<&T::State> {
        self.state.downcast_ref()
    }

    pub fn state_mut<T: GraphNode<Data>>(&mut self) -> Option<&mut T::State> {
        self.state.downcast_mut()
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct StatelessState {
    #[serde(skip)]
    _private: (),
}

pub trait StatelessCommonGraphNode<Data: GraphData>: Send + Sync + 'static + DynClone {
    fn id(&self) -> &'static str;
    fn header_hue_chroma(&self) -> (f32, f32);
    fn create_inputs(
        &self,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot>;
    fn create_outputs(
        &self,
        ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot>;
    fn update_signature(&self, _: GraphNodeUpdateSignatureContext<'_, Data>) {}
    fn generate_code(
        &self,
        ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError>;
}

pub struct GraphNodeData<Data: GraphData> {
    pub position: Point,
    pub data: StatefulGraphNode<Data>,
    pub inputs: Arc<[GraphInputSlotId]>,
    pub outputs: Arc<[GraphOutputSlotId]>,
}

impl<Data: GraphData> GraphNodeData<Data> {
    pub fn view<'a>(
        &'a self,
        node_id: GraphNodeId,
        slots: &GraphSlots,
        resources: &GraphResources<Data>,
    ) -> GraphElement<'a, ErasedGraphNodeMessage> {
        self.data.view(
            node_id,
            GraphNodeViewContext {
                inputs: &self.inputs,
                outputs: &self.outputs,
                slots,
                resources,
                _marker: PhantomData,
            },
        )
    }
}

#[derive(Clone, Copy)]
pub struct GraphNodeDefaultStateContext<'a, Data: GraphData> {
    pub resources: &'a GraphResources<Data>,
    pub _marker: PhantomData<Data>,
}

pub struct GraphNodeCreateSlotsContext<'a, Data: GraphData> {
    pub resources: &'a GraphResources<Data>,
    pub _marker: PhantomData<Data>,
}

pub struct GraphNodeViewContext<'a, Data: GraphData> {
    pub inputs: &'a [GraphInputSlotId],
    pub outputs: &'a [GraphOutputSlotId],
    pub slots: &'a GraphSlots,
    pub resources: &'a GraphResources<Data>,
    pub _marker: PhantomData<Data>,
}

impl<Data: GraphData> GraphNodeViewContext<'_, Data> {
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
        Some(input_slot(slot_id, t!(&slot.name), slot).map(map_literal))
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
                Some(input_slot(*id, t!(&slot.name), slot).map(map_literal))
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
        Column::new()
            .push(
                Column::with_children(self.view_all_inputs(map_literal))
                    .width(Length::Fill)
                    .spacing(4),
            )
            .push(
                Column::with_children(self.view_all_outputs())
                    .width(Length::Fill)
                    .spacing(4),
            )
            .width(Length::Fill)
            .spacing(2)
            .into()
    }

    pub fn view_all_slots_with_header<'a, Message: 'static>(
        &self,
        header: impl Into<GraphElement<'a, Message>>,
        map_literal: impl Fn(ErasedGraphLiteralUpdateMessage) -> Message + Copy + 'static,
    ) -> GraphElement<'a, Message> {
        Column::new()
            .push(header)
            .push(self.view_all_slots(map_literal))
            .spacing(2)
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

pub struct GraphNodeUpdateContext<'a, Data: GraphData> {
    pub inputs: &'a [GraphInputSlotId],
    pub slots: &'a mut GraphSlots,
    pub resources: &'a GraphResources<Data>,
    pub _marker: PhantomData<Data>,
}

impl<Data: GraphData> GraphNodeUpdateContext<'_, Data> {
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

pub struct GraphNodeUpdateSignatureContext<'a, Data: GraphData> {
    pub inputs: &'a [GraphInputSlotId],
    pub outputs: &'a [GraphOutputSlotId],
    pub slots: &'a GraphSlots,
    pub signature: &'a mut GraphSignature,
    pub resources: &'a GraphResources<Data>,
    pub _marker: PhantomData<Data>,
}

impl<Data: GraphData> GraphNodeUpdateSignatureContext<'_, Data> {
    pub fn require_output_slot_as_graph_input(&mut self, index: usize, name: String) {
        let Some(slot_id) = self.outputs.get(index) else {
            return;
        };
        let Some(slot) = self.slots.outputs.get(slot_id) else {
            return;
        };

        self.signature.inputs.insert(
            *slot_id,
            GraphVariable::new_boxed(name, dyn_clone::clone_box(&*slot.data_ty)),
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
            GraphVariable::new_boxed(name, dyn_clone::clone_box(slot.data.ty())),
        );
    }
}

pub struct GraphNodeCodeGenContext<'a, Data: GraphData> {
    pub inputs: &'a [GraphInputSlotId],
    pub outputs: &'a [GraphOutputSlotId],
    pub graph_slots: &'a GraphSlots,
    pub output_slot_idents: &'a mut HashMap<GraphOutputSlotId, String>,
    pub ident_generator: &'a mut GraphVarIdentGenerator,
    pub resources: &'a GraphResources<Data>,
    pub texture_usage: &'a mut GraphTextureUsageRecorder,
    pub _marker: PhantomData<Data>,
}

impl<Data: GraphData> GraphNodeCodeGenContext<'_, Data> {
    pub fn get_input(&self, index: usize) -> Result<String, GraphNodeCodeGenError> {
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

        if output_slot.data_ty.name() != slot.data.ty().name() {
            self.resources
                .type_registry
                .try_wgsl_cast(&*output_slot.data_ty, slot.data.ty(), ident)
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

    pub fn get_output(&mut self, index: usize) -> Result<String, GraphNodeCodeGenError> {
        let slot_id = self
            .outputs
            .get(index)
            .ok_or(GraphNodeCodeGenError::SlotIndexOutOfBounds)?;
        Ok(match self.output_slot_idents.entry(*slot_id) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => entry.insert(self.ident_generator.next_output()).clone(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GraphNodeCodeGenError {
    #[error("Input slot index out of bounds")]
    SlotIndexOutOfBounds,
    #[error("Missing input slot")]
    MissingInputSlot,
    #[error("Missing output slot")]
    MissingOutputSlot,
    #[error("Failed to cast variable")]
    FailedToCastVariable,
    #[error("Failed to convert literal to code")]
    LiteralToCodeFailed,
    #[error(transparent)]
    Custom(#[from] anyhow::Error),
}

#[derive(Debug)]
pub struct ContextualGraphNodeCodeGenError {
    pub node_id: GraphNodeId,
    pub node_title: String,
    pub err: GraphNodeCodeGenError,
    pub code: String,
}

impl std::fmt::Display for ContextualGraphNodeCodeGenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Error in node {:?} of type {}: {}\nCode already generated:\n{}",
            self.node_id, self.node_title, self.err, self.code
        )
    }
}

pub struct GraphNodeRegistry<Data: GraphData> {
    nodes: BTreeMap<&'static str, Box<dyn ErasedGraphNode<Data>>>,
}

impl<Data: GraphData> Default for GraphNodeRegistry<Data> {
    fn default() -> Self {
        Self {
            nodes: Default::default(),
        }
    }
}

impl<Data: GraphData> Clone for GraphNodeRegistry<Data> {
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
        }
    }
}

impl<Data: GraphData> GraphNodeRegistry<Data> {
    pub fn with_capacity() -> Self {
        Self {
            nodes: BTreeMap::new(),
        }
    }

    pub fn register<T: ErasedGraphNode<Data> + Default>(&mut self) {
        let node = Box::new(T::default());
        self.nodes.insert(node.id(), node);
    }

    pub fn register_boxed(&mut self, node: Box<dyn ErasedGraphNode<Data>>) {
        self.nodes.insert(node.id(), node);
    }

    pub fn get(&self, name: &str) -> Option<Box<dyn ErasedGraphNode<Data>>> {
        self.nodes.get(name).cloned()
    }

    pub fn all(&self) -> &BTreeMap<&'static str, Box<dyn ErasedGraphNode<Data>>> {
        &self.nodes
    }

    pub fn merge(&mut self, other: GraphNodeRegistry<Data>) {
        self.nodes.extend(other.nodes);
    }
}
