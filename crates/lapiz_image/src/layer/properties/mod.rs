use std::{collections::HashMap, fmt};

use anyhow::Result;
use downcast_rs::Downcast;
use dyn_clone::DynClone;

pub mod builtin;

/// Type-erased map of [`LayerProperty`] values attached to a [`LayerStackNode`](crate::layer::LayerStackNode).
///
/// `LayerProperties` stores heterogeneous property instances keyed by their static string
/// identifier ([`LayerProperty::ident`]) and provides typed lookup and mutation methods.
#[derive(Clone)]
pub struct LayerProperties {
    props: HashMap<&'static str, Box<dyn LayerProperty>>,
}

impl fmt::Debug for LayerProperties {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayerProperties")
            .field("props", &self.props.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl LayerProperties {
    pub fn new(props: HashMap<&'static str, Box<dyn LayerProperty>>) -> Self {
        Self { props }
    }

    pub fn set<T: LayerProperty>(&mut self, value: T) {
        if let Some(prop) = self.props.get_mut(T::ident()) {
            *prop = Box::new(value);
        }
    }

    pub fn get<T: LayerProperty>(&self) -> Option<&T> {
        let value = self.props.get(T::ident())?;
        value.downcast_ref()
    }

    pub fn contains<T: LayerProperty>(&self) -> bool {
        self.props.contains_key(T::ident())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &dyn LayerProperty)> {
        self.props.iter().map(|(k, v)| (*k, v.as_ref()))
    }
}

/// Intermediate map of MessagePack-encoded property byte blobs used when deserializing [`LayerProperties`].
pub struct EncodedLayerProperties {
    pub props: HashMap<String, Vec<u8>>,
}

impl EncodedLayerProperties {
    pub fn from_slice(props: &[u8]) -> Result<Self> {
        Ok(Self {
            props: rmp_serde::from_slice(props)?,
        })
    }
}

/// A serializable, downcastable metadata property that can be stored in [`LayerProperties`].
pub trait LayerProperty: DynClone + Downcast + Send + 'static {
    fn ident() -> &'static str
    where
        Self: Sized;
    fn encode(&self) -> Result<Vec<u8>>;
    fn decode(data: &[u8]) -> Result<Self>
    where
        Self: Sized;
}
downcast_rs::impl_downcast!(LayerProperty);
dyn_clone::clone_trait_object!(LayerProperty);
