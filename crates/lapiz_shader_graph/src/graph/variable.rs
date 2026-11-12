use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use anyhow::Result;
use downcast_rs::Downcast;
use dyn_clone::DynClone;
use wesl::syntax::Expression;
use wgpu::{Device, Queue};

use crate::{
    graph::slot::{ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphValueType},
    save::GraphValueTypeId,
    wgsl_std::types::{
        atomic::{ArrayAtomicI32Type, ArrayAtomicU32Type},
        handle::ArrayType,
    },
};

/// A registry of shader graph value types ([`GraphValueType`]) and implicit type casters ([`GraphVariableCaster`]).
///
/// `GraphTypeRegistry` resolves static and parameterized value types (such as fixed-length arrays
/// and atomic arrays) by their [`GraphValueTypeId`] and determines whether an output slot of one
/// type can be implicitly cast to an input slot of another type during connection validation and
/// WGSL code generation.
#[derive(Default, Clone)]
pub struct GraphTypeRegistry {
    types: BTreeMap<GraphValueTypeId, Arc<dyn ErasedGraphValueType>>,
    casters:
        HashMap<GraphValueTypeId, HashMap<GraphValueTypeId, Arc<dyn ErasedGraphVariableCaster>>>,
}

impl GraphTypeRegistry {
    pub fn register_type<T: GraphValueType + Default>(&mut self) {
        self.register_type_value(T::default());
    }

    pub fn register_type_value<T: GraphValueType>(&mut self, ty: T) {
        self.types.insert(ty.id(), Arc::new(ty));
    }

    pub fn get_type(&self, id: &str) -> Option<&Arc<dyn ErasedGraphValueType>> {
        self.types.get(id)
    }

    pub fn array_type(&self, element_id: &str, len: u32) -> Option<Arc<dyn ErasedGraphValueType>> {
        Some(Arc::new(ArrayType {
            element_type: self.resolve_type(element_id)?,
            len,
        }))
    }

    pub fn atomic_array_type(
        &self,
        element_id: &str,
        len: u32,
    ) -> Option<Arc<dyn ErasedGraphValueType>> {
        match element_id {
            "i32" => Some(Arc::new(ArrayAtomicI32Type { len })),
            "u32" => Some(Arc::new(ArrayAtomicU32Type { len })),
            _ => None,
        }
    }

    pub fn resolve_type(&self, id: &str) -> Option<Arc<dyn ErasedGraphValueType>> {
        if let Some(ty) = self.get_type(id) {
            return Some(ty.clone());
        }
        if let Some(len) = id
            .strip_prefix("array_atomic_i32_")
            .and_then(|len| len.parse().ok())
        {
            return self.atomic_array_type("i32", len);
        }
        if let Some(len) = id
            .strip_prefix("array_atomic_u32_")
            .and_then(|len| len.parse().ok())
        {
            return self.atomic_array_type("u32", len);
        }
        let array = id.strip_prefix("array_")?;
        let (element_id, len) = array.rsplit_once('_')?;
        self.array_type(element_id, len.parse().ok()?)
    }

    pub fn register_caster<T: GraphVariableCaster + Default>(&mut self) {
        let from = T::FromType::default();
        let to = T::ToType::default();
        let from_id = <T::FromType as GraphValueType>::id(&from);
        let to_id = <T::ToType as GraphValueType>::id(&to);
        let caster = Arc::new(T::default());
        self.casters
            .entry(from_id)
            .or_default()
            .insert(to_id, caster);
    }

    pub fn try_wgsl_cast(
        &self,
        from_type: &dyn ErasedGraphValueType,
        to_type: &dyn ErasedGraphValueType,
        value: Expression,
    ) -> Option<Expression> {
        Some(
            self.casters
                .get(&from_type.id())?
                .get(&to_type.id())?
                .wgsl_cast(value),
        )
    }

    pub fn can_cast(&self, from: &dyn ErasedGraphValueType, to: &dyn ErasedGraphValueType) -> bool {
        let from_id = from.id();
        let to_id = to.id();
        self.casters
            .get(&from_id)
            .and_then(|map| map.get(&to_id))
            .is_some()
    }

    pub fn all_types(&self) -> &BTreeMap<GraphValueTypeId, Arc<dyn ErasedGraphValueType>> {
        &self.types
    }

    pub fn all_casters(
        &self,
    ) -> &HashMap<GraphValueTypeId, HashMap<GraphValueTypeId, Arc<dyn ErasedGraphVariableCaster>>>
    {
        &self.casters
    }

    pub fn merge(&mut self, other: GraphTypeRegistry) {
        self.types.extend(other.types);
        for (from, casters) in other.casters {
            self.casters.entry(from).or_default().extend(casters);
        }
    }
}

/// Defines an implicit WGSL conversion from one [`GraphValueType`] to another.
///
/// Registered in a [`GraphTypeRegistry`], a `GraphVariableCaster` allows slots of [`FromType`](Self::FromType)
/// to connect directly to slots of [`ToType`](Self::ToType) by wrapping the source WGSL
/// [`Expression`] in a cast expression during code generation.
pub trait GraphVariableCaster: Send + Sync + 'static + Clone {
    type FromType: GraphValueType + Default;
    type ToType: GraphValueType + Default;
    fn wgsl_cast(&self, variable: Expression) -> Expression;
}

/// Object-safe, type-erased counterpart to [`GraphVariableCaster`].
pub trait ErasedGraphVariableCaster: Send + Sync + 'static + DynClone {
    fn wgsl_cast(&self, variable: Expression) -> Expression;
}

dyn_clone::clone_trait_object!(ErasedGraphVariableCaster);

impl<T: GraphVariableCaster> ErasedGraphVariableCaster for T {
    fn wgsl_cast(&self, variable: Expression) -> Expression {
        self.wgsl_cast(variable)
    }
}

/// Marker trait for CPU-side literal values associated with a [`GraphValueType`].
pub trait GraphLiteralValue: DynClone + Send + Sync + 'static + Downcast {}

downcast_rs::impl_downcast!(GraphLiteralValue);
dyn_clone::clone_trait_object!(GraphLiteralValue);

impl<T: Send + Sync + DynClone + 'static> GraphLiteralValue for T {}

/// Marker trait for GPU-prepared shader resources associated with a [`GraphValueType`].
pub trait GraphShaderLiteralValue: Send + Sync + 'static + Downcast {}

downcast_rs::impl_downcast!(GraphShaderLiteralValue);

impl<T: Send + Sync + 'static> GraphShaderLiteralValue for T {}

/// A type-erased CPU-side literal value paired with its [`ErasedGraphValueType`] descriptor.
///
/// `GraphLiteral` stores the fallback value of an unconnected input slot or external parameter,
/// supporting typed downcasting, UI updates, WGSL literal expression generation, and preparation
/// into a GPU-bound [`GraphShaderLiteral`].
#[derive(Clone)]
pub struct GraphLiteral {
    value: Box<dyn GraphLiteralValue>,
    ty: Arc<dyn ErasedGraphValueType>,
}

impl GraphLiteral {
    pub fn new<T: GraphValueType + Default>(value: T::AssociatedLiteralType) -> Self {
        Self {
            value: Box::new(value),
            ty: Arc::new(T::default()),
        }
    }

    pub fn new_non_default<T: GraphValueType>(value: T::AssociatedLiteralType, ty: T) -> Self {
        Self {
            value: Box::new(value),
            ty: Arc::new(ty),
        }
    }

    pub fn new_boxed(value: Box<dyn GraphLiteralValue>, ty: Arc<dyn ErasedGraphValueType>) -> Self {
        Self { value, ty }
    }

    pub fn new_boxed_default(ty: Arc<dyn ErasedGraphValueType>) -> Self {
        Self {
            value: ty.default_literal(),
            ty,
        }
    }

    pub fn downcast<T: GraphLiteralValue>(self) -> T {
        match self.value.downcast::<T>() {
            Ok(ok) => *ok,
            Err(_) => {
                panic!("Failed to downcast Literal")
            }
        }
    }

    #[allow(
        clippy::should_implement_trait,
        reason = "downcasts to a caller-chosen type instead of implementing AsRef"
    )]
    pub fn as_ref<T: GraphLiteralValue>(&self) -> &T {
        self.value
            .downcast_ref::<T>()
            .expect("Failed to downcast Literal")
    }

    #[allow(
        clippy::should_implement_trait,
        reason = "downcasts to a caller-chosen type instead of implementing AsMut"
    )]
    pub fn as_mut<T: GraphLiteralValue>(&mut self) -> &mut T {
        self.value
            .downcast_mut::<T>()
            .expect("Failed to downcast Literal")
    }

    pub fn try_as_ref<T: GraphLiteralValue>(&self) -> Option<&T> {
        self.value.downcast_ref::<T>()
    }

    pub fn try_as_mut<T: GraphLiteralValue>(&mut self) -> Option<&mut T> {
        self.value.downcast_mut::<T>()
    }

    pub fn ty(&self) -> &Arc<dyn ErasedGraphValueType> {
        &self.ty
    }

    pub fn value(&self) -> &dyn GraphLiteralValue {
        self.value.as_ref()
    }

    pub fn set<T: GraphLiteralValue>(&mut self, value: T) {
        if let Some(x) = self.value.downcast_mut() {
            *x = value;
        } else {
            log::error!("Setting a Literal with a different type");
        }
    }

    pub fn set_boxed(&mut self, value: Box<dyn GraphLiteralValue>) {
        if value.as_ref().type_id() == self.value.as_ref().type_id() {
            self.value = value;
        } else {
            log::error!("Setting a Literal with a different type");
        }
    }

    pub fn update(&mut self, message: ErasedGraphLiteralUpdateMessage) {
        self.ty.update_literal(self.value.as_mut(), message);
    }

    pub fn to_code(&self) -> Option<Expression> {
        self.ty.literal_to_code(self.value.as_ref())
    }

    pub fn prepare_to_shader(&self, device: &Device, queue: &Queue) -> Result<GraphShaderLiteral> {
        let value = self
            .ty
            .prepare_to_shader(self.value.as_ref(), device, queue)?;
        Ok(GraphShaderLiteral {
            value,
            ty: self.ty.clone(),
        })
    }
}

/// A GPU-prepared literal resource paired with its [`ErasedGraphValueType`] descriptor.
///
/// Produced by [`GraphLiteral::prepare_to_shader`], `GraphShaderLiteral` holds the GPU buffers or
/// texture bindings ready to be bound into a compute pipeline via
/// [`ErasedGraphValueType::push_shader_binding`].
pub struct GraphShaderLiteral {
    value: Box<dyn GraphShaderLiteralValue>,
    ty: Arc<dyn ErasedGraphValueType>,
}

impl GraphShaderLiteral {
    pub fn new<T: GraphValueType + Default>(value: T::PreparedShaderType) -> Self {
        Self {
            value: Box::new(value),
            ty: Arc::new(T::default()),
        }
    }

    pub fn new_non_default<T: GraphValueType>(value: T::PreparedShaderType, ty: T) -> Self {
        Self {
            value: Box::new(value),
            ty: Arc::new(ty),
        }
    }

    pub fn new_boxed(
        value: Box<dyn GraphShaderLiteralValue>,
        ty: Arc<dyn ErasedGraphValueType>,
    ) -> Self {
        Self { value, ty }
    }

    pub fn downcast<T: GraphShaderLiteralValue>(self) -> T {
        match self.value.downcast::<T>() {
            Ok(ok) => *ok,
            Err(_) => {
                panic!("Failed to downcast Literal")
            }
        }
    }

    #[allow(
        clippy::should_implement_trait,
        reason = "downcasts to a caller-chosen type instead of implementing AsRef"
    )]
    pub fn as_ref<T: GraphShaderLiteralValue>(&self) -> &T {
        self.value
            .downcast_ref::<T>()
            .expect("Failed to downcast Literal")
    }

    #[allow(
        clippy::should_implement_trait,
        reason = "downcasts to a caller-chosen type instead of implementing AsMut"
    )]
    pub fn as_mut<T: GraphShaderLiteralValue>(&mut self) -> &mut T {
        self.value
            .downcast_mut::<T>()
            .expect("Failed to downcast Literal")
    }

    pub fn try_as_ref<T: GraphShaderLiteralValue>(&self) -> Option<&T> {
        self.value.downcast_ref::<T>()
    }

    pub fn try_as_mut<T: GraphShaderLiteralValue>(&mut self) -> Option<&mut T> {
        self.value.downcast_mut::<T>()
    }

    pub fn ty(&self) -> &Arc<dyn ErasedGraphValueType> {
        &self.ty
    }

    pub fn value(&self) -> &dyn GraphShaderLiteralValue {
        self.value.as_ref()
    }

    pub fn value_mut(&mut self) -> &mut dyn GraphShaderLiteralValue {
        self.value.as_mut()
    }

    pub fn set<T: GraphShaderLiteralValue>(&mut self, value: T) {
        if let Some(x) = self.value.downcast_mut() {
            *x = value;
        } else {
            log::error!("Setting a Literal with a different type");
        }
    }

    pub fn set_boxed(&mut self, value: Box<dyn GraphShaderLiteralValue>) {
        if value.as_ref().type_id() == self.value.as_ref().type_id() {
            self.value = value;
        } else {
            log::error!("Setting a Literal with a different type");
        }
    }
}

/// A named shader variable with an associated [`ErasedGraphValueType`], used in [`GraphSignature`](crate::graph::GraphSignature)s.
#[derive(Clone)]
pub struct GraphVariable {
    identifier: String,
    ty: Arc<dyn ErasedGraphValueType>,
}

impl GraphVariable {
    pub fn new<T: GraphValueType + Default>(identifier: String) -> Self {
        Self {
            identifier,
            ty: Arc::new(T::default()),
        }
    }

    pub fn new_boxed(identifier: String, ty: Arc<dyn ErasedGraphValueType>) -> Self {
        Self { identifier, ty }
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn ty(&self) -> &Arc<dyn ErasedGraphValueType> {
        &self.ty
    }
}
