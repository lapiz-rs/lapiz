use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use anyhow::Result;
use downcast_rs::Downcast;
use dyn_clone::DynClone;
use lapiz_assets::store::AssetRegistry;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::DynamicBindGroupLayoutEntries,
};
use lapiz_utils::wrapper;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wesl::syntax::{
    AssignmentOperator, AssignmentStatement, Attribute, CompoundStatement, Expression, Ident, Span,
    Spanned, Statement, TypeExpression, UnaryExpression, UnaryOperator,
};
use wesl_quote::quote_statement;
use wgpu::{Device, Queue};

use crate::{
    GraphElement,
    graph::{
        node::{GraphNodeCodeGenContext, GraphNodeId},
        variable::{GraphLiteral, GraphLiteralValue, GraphShaderLiteralValue},
    },
    save::GraphValueTypeId,
};

wrapper! {
    /// A strongly-typed unique identifier for an input slot in a [`Graph`](crate::graph::Graph).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    #[display("{0}")]
    pub GraphInputSlotId : Uuid
}

wrapper! {
    /// A strongly-typed unique identifier for an output slot in a [`Graph`](crate::graph::Graph).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    #[display("{0}")]
    pub GraphOutputSlotId : Uuid
}

/// Storage for all input and output slot instances within a [`Graph`](crate::graph::Graph).
///
/// `GraphSlots` maintains the bidirectional connection topology between [`GraphInputSlotData`]
/// (each holding at most one upstream [`GraphOutputSlotId`]) and [`GraphOutputSlotData`] (each
/// tracking the set of downstream [`GraphInputSlotId`]s connected to it).
#[derive(Default)]
pub struct GraphSlots {
    pub(crate) inputs: HashMap<GraphInputSlotId, GraphInputSlotData>,
    pub(crate) outputs: HashMap<GraphOutputSlotId, GraphOutputSlotData>,
}

impl GraphSlots {
    pub fn get_input(&self, id: &GraphInputSlotId) -> Option<&GraphInputSlotData> {
        self.inputs.get(id)
    }

    pub fn get_output(&self, id: &GraphOutputSlotId) -> Option<&GraphOutputSlotData> {
        self.outputs.get(id)
    }

    pub fn get_connected(&self, input_id: &GraphInputSlotId) -> Option<&GraphOutputSlotData> {
        let input = self.inputs.get(input_id)?;
        self.outputs.get(input.connected.as_ref()?)
    }
}

/// A template definition for an input slot returned by [`GraphNode::create_inputs`](crate::graph::node::GraphNode::create_inputs).
pub struct GraphDefaultInputSlot {
    pub name: String,
    pub ty: Arc<dyn ErasedGraphValueType>,
}

impl GraphDefaultInputSlot {
    pub fn new<T: GraphValueType + Default>(name: String) -> Self {
        Self {
            name,
            ty: Arc::new(T::default()),
        }
    }

    pub fn new_boxed(name: String, ty: Arc<dyn ErasedGraphValueType>) -> Self {
        Self { name, ty }
    }
}

/// Runtime data for an input slot on a node, holding its fallback [`GraphLiteral`] and optional upstream connection.
pub struct GraphInputSlotData {
    pub node_id: GraphNodeId,
    pub name: String,
    pub data: GraphLiteral,
    pub connected: Option<GraphOutputSlotId>,
}

/// A template definition for an output slot returned by [`GraphNode::create_outputs`](crate::graph::node::GraphNode::create_outputs).
pub struct GraphDefaultOutputSlot {
    pub name: String,
    pub ty: Arc<dyn ErasedGraphValueType>,
}

impl GraphDefaultOutputSlot {
    pub fn new<T: GraphValueType + Default>(name: String) -> Self {
        Self {
            name,
            ty: Arc::new(T::default()),
        }
    }

    pub fn new_non_default<T: GraphValueType>(name: String, ty: T) -> Self {
        Self {
            name,
            ty: Arc::new(ty),
        }
    }

    pub fn new_boxed(name: String, ty: Arc<dyn ErasedGraphValueType>) -> Self {
        Self { name, ty }
    }
}

/// Runtime data for an output slot on a node, holding its value type and set of connected downstream input slots.
pub struct GraphOutputSlotData {
    pub node_id: GraphNodeId,
    pub name: String,
    pub data_ty: Arc<dyn ErasedGraphValueType>,
    pub connected: HashSet<GraphInputSlotId>,
}

/// The shader binding stage or role for which a [`GraphValueType`] generates layout and binding entries.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GraphShaderStage {
    Input,
    Eval,
    Main,
}

/// Defines a data type that can flow through shader graph slots, be edited as a literal, and bind to GPU shaders.
///
/// Implementors of `GraphValueType` bridge three representations of a shader value:
/// - [`AssociatedLiteralType`](Self::AssociatedLiteralType): The CPU-side literal value edited in
///   unconnected input slots and serialized to TOML.
/// - [`PreparedShaderType`](Self::PreparedShaderType): The GPU-uploaded resource (such as a
///   uniform buffer, storage buffer, or texture binding) created by
///   [`prepare_to_shader`](Self::prepare_to_shader).
/// - WGSL code generation: Inline literal expressions via [`literal_to_code`](Self::literal_to_code),
///   bind group layout/entry declarations via [`push_shader_layout`](Self::push_shader_layout) and
///   [`push_shader_binding`](Self::push_shader_binding), and pass I/O accessors via
///   [`handle_input_values`](Self::handle_input_values) and
///   [`handle_output_values`](Self::handle_output_values).
pub trait GraphValueType: Send + Sync + 'static + DynClone {
    type AssociatedLiteralType: GraphLiteralValue;
    // TODO Better naming
    // This means the resources that is uploaded to gpu and can be binded in pipelines.
    type PreparedShaderType: GraphShaderLiteralValue;
    type Message: GraphLiteralUpdateMessage;

    fn id(&self) -> GraphValueTypeId;

    fn push_shader_layout(
        &self,
        name: &str,
        stage: GraphShaderStage,
        group: u32,
        binding: u32,
        bindings: DynamicBindGroupLayoutEntries,
        shader: String,
    ) -> Result<(u32, DynamicBindGroupLayoutEntries, String)>;
    fn push_shader_binding<'a>(
        &self,
        stage: GraphShaderStage,
        value: &'a Self::PreparedShaderType,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)>;
    fn prepare_to_shader(
        &self,
        data: &Self::AssociatedLiteralType,
        device: &Device,
        queue: &Queue,
    ) -> Result<Self::PreparedShaderType>;

    fn push_input_slots(&self) -> Vec<GraphDefaultInputSlot>
    where
        Self: Sized,
    {
        let ty = Arc::<Self>::from(dyn_clone::clone_box(self));
        vec![GraphDefaultInputSlot::new_boxed("value".into(), ty)]
    }
    fn push_output_slots(&self) -> Vec<GraphDefaultOutputSlot>
    where
        Self: Sized,
    {
        let ty = Arc::<Self>::from(dyn_clone::clone_box(self));
        vec![GraphDefaultOutputSlot::new_boxed("value".into(), ty)]
    }
    fn handle_input_values(
        &self,
        input_name: &str,
        base_index: usize,
        ctx: &mut GraphNodeCodeGenContext,
    ) -> Result<String> {
        ctx.get_output(base_index)?;
        ctx.output_slot_idents.insert(
            ctx.outputs[base_index],
            Ident::new(input_name.to_string()).into(),
        );
        Ok(String::new())
    }
    fn handle_output_values(
        &self,
        output_name: &str,
        base_index: usize,
        ctx: &GraphNodeCodeGenContext,
    ) -> Result<String> {
        let value = ctx.get_input(base_index)?;
        let output = Ident::new(output_name.to_string());
        Ok(quote_statement! { @if(!EVAL) { #output = #value; } }.to_string())
    }

    fn requires_eval(&self) -> bool {
        false
    }
    fn post_eval(
        &self,
        _value: &mut Self::PreparedShaderType,
        _device: &Device,
        _queue: &Queue,
    ) -> Result<()> {
        Ok(())
    }

    fn default_literal(&self) -> Self::AssociatedLiteralType;
    fn wgsl_type_name(&self) -> Option<&'static str>;
    fn wgsl_array_element_stride(&self) -> Option<u64> {
        None
    }
    fn hue_chroma(&self) -> (f32, f32);
    fn view_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        // TODO should this change to GraphResources?
        assets: &AssetRegistry,
    ) -> GraphElement<'static, Self::Message>;
    fn update_literal(&self, data: &mut Self::AssociatedLiteralType, message: Self::Message);
    fn literal_to_code(&self, data: &Self::AssociatedLiteralType) -> Option<Expression>;

    fn generate_extra_shader_body(&self, _stage: GraphShaderStage, _name: &str) -> Option<String> {
        None
    }

    fn serialize_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        assets: &AssetRegistry,
    ) -> Result<toml::Value>;
    fn deserialize_literal(
        &self,
        deserializer: toml::Value,
        assets: &AssetRegistry,
    ) -> Result<Self::AssociatedLiteralType>;
}

/// Marker trait for cloneable, downcastable UI messages that update a [`GraphValueType`]'s literal value.
pub trait GraphLiteralUpdateMessage: DynClone + Send + Sync + 'static + Downcast {}

impl<T: DynClone + Send + Sync + 'static> GraphLiteralUpdateMessage for T {}
downcast_rs::impl_downcast!(GraphLiteralUpdateMessage);

/// A type-erased literal update message paired with the target [`GraphInputSlotId`].
pub struct ErasedGraphLiteralUpdateMessage {
    pub inner: Box<dyn GraphLiteralUpdateMessage>,
    pub id: GraphInputSlotId,
}

impl fmt::Debug for ErasedGraphLiteralUpdateMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErasedGraphLiteralUpdateMessage")
            .field("id", &self.id)
            .finish()
    }
}

impl Clone for ErasedGraphLiteralUpdateMessage {
    fn clone(&self) -> Self {
        Self {
            inner: dyn_clone::clone_box(&*self.inner),
            id: self.id,
        }
    }
}

/// Object-safe, type-erased counterpart to [`GraphValueType`].
pub trait ErasedGraphValueType: Send + Sync + 'static + DynClone + Downcast {
    fn hue_chroma(&self) -> (f32, f32);
    fn id(&self) -> GraphValueTypeId;

    fn push_shader_layout(
        &self,
        name: &str,
        stage: GraphShaderStage,
        group: u32,
        binding: u32,
        bindings: DynamicBindGroupLayoutEntries,
        shader: String,
    ) -> Result<(u32, DynamicBindGroupLayoutEntries, String)>;
    fn push_shader_binding<'a>(
        &self,
        stage: GraphShaderStage,
        value: &'a dyn GraphShaderLiteralValue,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)>;
    fn prepare_to_shader(
        &self,
        data: &dyn GraphLiteralValue,
        device: &Device,
        queue: &Queue,
    ) -> Result<Box<dyn GraphShaderLiteralValue>>;

    // These three values are used in effect passes to allow types to be able to
    // customize its own behavior
    // TODO better naming
    fn push_input_slots(&self) -> Vec<GraphDefaultInputSlot>;
    // TODO better naming
    fn push_output_slots(&self) -> Vec<GraphDefaultOutputSlot>;
    // TODO better naming
    // This actually means to read the value of this type from shader binding.
    // For general types, it just copies the identifier. But for special types like
    // layer, the pixel value needs to call a helper to get.
    fn handle_input_values(
        &self,
        input_name: &str,
        base_index: usize,
        ctx: &mut GraphNodeCodeGenContext,
    ) -> Result<String>;
    // TODO better naming
    fn handle_output_values(
        &self,
        output_name: &str,
        base_index: usize,
        ctx: &mut GraphNodeCodeGenContext,
    ) -> Result<String>;

    // Eval means eval stage in effect
    fn requires_eval(&self) -> bool;
    fn post_eval(
        &self,
        value: &mut dyn GraphShaderLiteralValue,
        device: &Device,
        queue: &Queue,
    ) -> Result<()>;

    // Inject extra shader body into built shader
    // Used in LayerType to generate helpers like xxx_load and xxx_store
    fn generate_extra_shader_body(&self, _stage: GraphShaderStage, _name: &str) -> Option<String> {
        None
    }

    fn default_literal(&self) -> Box<dyn GraphLiteralValue>;
    fn wgsl_type_name(&self) -> Option<&'static str>;
    fn wgsl_array_element_stride(&self) -> Option<u64> {
        None
    }
    fn view_literal(
        &self,
        slot_id: GraphInputSlotId,
        data: &dyn GraphLiteralValue,
        assets: &AssetRegistry,
    ) -> GraphElement<'static, ErasedGraphLiteralUpdateMessage>;
    fn update_literal(
        &self,
        data: &mut dyn GraphLiteralValue,
        message: ErasedGraphLiteralUpdateMessage,
    );
    fn literal_to_code(&self, data: &dyn GraphLiteralValue) -> Option<Expression>;
    fn serialize_literal(
        &self,
        data: &dyn GraphLiteralValue,
        assets: &AssetRegistry,
    ) -> Result<toml::Value>;
    fn deserialize_literal(
        &self,
        deserializer: toml::Value,
        assets: &AssetRegistry,
    ) -> Result<Box<dyn GraphLiteralValue>>;
}

downcast_rs::impl_downcast!(ErasedGraphValueType);
dyn_clone::clone_trait_object!(ErasedGraphValueType);

impl<T: GraphValueType> ErasedGraphValueType for T {
    fn hue_chroma(&self) -> (f32, f32) {
        self.hue_chroma()
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueType::id(self)
    }

    fn push_shader_layout(
        &self,
        name: &str,
        stage: GraphShaderStage,
        group: u32,
        binding: u32,
        bindings: DynamicBindGroupLayoutEntries,
        shader: String,
    ) -> Result<(u32, DynamicBindGroupLayoutEntries, String)> {
        GraphValueType::push_shader_layout(self, name, stage, group, binding, bindings, shader)
    }

    fn push_shader_binding<'a>(
        &self,
        stage: GraphShaderStage,
        value: &'a dyn GraphShaderLiteralValue,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)> {
        GraphValueType::push_shader_binding(
            self,
            stage,
            value
                .downcast_ref::<T::PreparedShaderType>()
                .expect("failed to downcast prepared shader literal"),
            binding,
            bindings,
        )
    }

    fn prepare_to_shader(
        &self,
        data: &dyn GraphLiteralValue,
        device: &Device,
        queue: &Queue,
    ) -> Result<Box<dyn GraphShaderLiteralValue>> {
        Ok(Box::new(GraphValueType::prepare_to_shader(
            self,
            data.downcast_ref::<T::AssociatedLiteralType>()
                .expect("failed to downcast graph literal"),
            device,
            queue,
        )?))
    }

    fn push_input_slots(&self) -> Vec<GraphDefaultInputSlot> {
        GraphValueType::push_input_slots(self)
    }

    fn push_output_slots(&self) -> Vec<GraphDefaultOutputSlot> {
        GraphValueType::push_output_slots(self)
    }

    fn handle_input_values(
        &self,
        input_name: &str,
        base_index: usize,
        ctx: &mut GraphNodeCodeGenContext,
    ) -> Result<String> {
        GraphValueType::handle_input_values(self, input_name, base_index, ctx)
    }

    fn handle_output_values(
        &self,
        output_name: &str,
        base_index: usize,
        ctx: &mut GraphNodeCodeGenContext,
    ) -> Result<String> {
        GraphValueType::handle_output_values(self, output_name, base_index, ctx)
    }

    fn requires_eval(&self) -> bool {
        GraphValueType::requires_eval(self)
    }

    fn post_eval(
        &self,
        value: &mut dyn GraphShaderLiteralValue,
        device: &Device,
        queue: &Queue,
    ) -> Result<()> {
        GraphValueType::post_eval(
            self,
            value
                .downcast_mut::<T::PreparedShaderType>()
                .expect("failed to downcast prepared shader literal"),
            device,
            queue,
        )
    }

    fn generate_extra_shader_body(&self, stage: GraphShaderStage, name: &str) -> Option<String> {
        GraphValueType::generate_extra_shader_body(self, stage, name)
    }

    fn default_literal(&self) -> Box<dyn GraphLiteralValue> {
        Box::new(self.default_literal())
    }

    fn wgsl_type_name(&self) -> Option<&'static str> {
        GraphValueType::wgsl_type_name(self)
    }

    fn wgsl_array_element_stride(&self) -> Option<u64> {
        GraphValueType::wgsl_array_element_stride(self)
    }

    fn view_literal(
        &self,
        slot_id: GraphInputSlotId,
        data: &dyn GraphLiteralValue,
        assets: &AssetRegistry,
    ) -> GraphElement<'static, ErasedGraphLiteralUpdateMessage> {
        self.view_literal(
            data.downcast_ref::<T::AssociatedLiteralType>()
                .expect("failed to downcast graph literal"),
            assets,
        )
        .map(move |message| ErasedGraphLiteralUpdateMessage {
            inner: Box::new(message),
            id: slot_id,
        })
    }

    fn update_literal(
        &self,
        data: &mut dyn GraphLiteralValue,
        message: ErasedGraphLiteralUpdateMessage,
    ) {
        let data = data
            .downcast_mut::<T::AssociatedLiteralType>()
            .expect("failed to downcast graph literal");
        let message = match message.inner.downcast::<T::Message>() {
            Ok(message) => message,
            Err(_) => panic!("failed to downcast graph literal message"),
        };
        self.update_literal(data, *message);
    }

    fn literal_to_code(&self, data: &dyn GraphLiteralValue) -> Option<Expression> {
        self.literal_to_code(
            data.downcast_ref::<T::AssociatedLiteralType>()
                .expect("failed to downcast graph literal"),
        )
    }

    fn serialize_literal(
        &self,
        data: &dyn GraphLiteralValue,
        assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        self.serialize_literal(
            data.downcast_ref::<T::AssociatedLiteralType>()
                .expect("failed to downcast graph literal"),
            assets,
        )
    }

    fn deserialize_literal(
        &self,
        deserializer: toml::Value,
        assets: &AssetRegistry,
    ) -> Result<Box<dyn GraphLiteralValue>> {
        Ok(Box::new(self.deserialize_literal(deserializer, assets)?))
    }
}
