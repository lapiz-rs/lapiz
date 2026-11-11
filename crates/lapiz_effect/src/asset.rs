//! Persistent effect definitions. Pass ports live in graph nodes, not in this asset.

use std::io::{self, Read, Write};

use anyhow::Result;
use lapiz_assets::{asset::Asset, loader::AssetSerializer};
use lapiz_shader_graph::save::SerializableGraph;
use lapiz_utils::wrapper;
use serde::{Deserialize, Serialize};
use toml::{de, ser};
use uuid::Uuid;

wrapper! {
    /// A strongly-typed unique identifier for a pass within an [`EffectAsset`] or [`EffectInstance`](crate::instance::EffectInstance).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub EffectPassId : Uuid
}

wrapper! {
    /// A strongly-typed unique identifier for an external input slot of an [`EffectAsset`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub EffectInputSlotId : Uuid
}

wrapper! {
    /// A strongly-typed unique identifier for an external output slot of an [`EffectAsset`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub EffectOutputSlotId : Uuid
}

wrapper! {
    /// A strongly-typed unique identifier for a [`PassInputNode`](crate::nodes::PassInputNode) binding inside an effect pass.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub EffectPassInputSlotId : Uuid
}

wrapper! {
    /// A strongly-typed unique identifier for a [`PassOutputNode`](crate::nodes::PassOutputNode) binding inside an effect pass.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub EffectPassOutputSlotId : Uuid
}

/// A persistent multi-pass GPU shader effect [`Asset`] stored as a `.lef` TOML file.
///
/// `EffectAsset` captures the serialized representation of an effect's external input and output
/// slots ([`SerializableEffectInputSlot`], [`SerializableEffectOutputSlot`]) along with its
/// ordered sequence of compute passes ([`SerializableEffectPass`]). At runtime it is instantiated
/// into an [`EffectInstance`](crate::instance::EffectInstance) for editing or compiled into an
/// [`EffectRenderer`](crate::render::EffectRenderer) for GPU execution.
#[derive(Clone, Serialize, Deserialize)]
pub struct EffectAsset {
    pub name: String,
    pub passes: Vec<SerializableEffectPass>,
    pub inputs: Vec<SerializableEffectInputSlot>,
    pub outputs: Vec<SerializableEffectOutputSlot>,
}

impl Asset for EffectAsset {
    const TYPE_NAME: &'static str = "effect";
}

/// An [`AssetSerializer`] that reads and writes [`EffectAsset`] definitions as `.lef` TOML files.
#[derive(Default)]
pub struct EffectAssetSerializer;

/// An error that can occur when reading or writing an [`EffectAsset`].
#[derive(Debug, thiserror::Error)]
pub enum EffectAssetSerializerError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    TomlDe(#[from] de::Error),
    #[error(transparent)]
    TomlSer(#[from] ser::Error),
}

impl AssetSerializer for EffectAssetSerializer {
    type Asset = EffectAsset;

    fn file_extension() -> &'static str {
        "lef"
    }

    fn read(&self, reader: &mut dyn Read) -> Result<Self::Asset> {
        let mut source = String::new();
        reader.read_to_string(&mut source)?;
        Ok(toml::from_str(&source)?)
    }

    fn write(&self, asset: &Self::Asset, writer: &mut dyn Write) -> Result<()> {
        writer.write_all(toml::to_string(asset)?.as_bytes())?;
        Ok(())
    }
}

/// The serialized representation of a single pass within an [`EffectAsset`].
#[derive(Clone, Serialize, Deserialize)]
pub struct SerializableEffectPass {
    pub id: EffectPassId,
    pub name: String,
    pub graph: SerializableGraph,
    pub dispatch_strategy: EffectPassDispatchStrategy,
}

/// The serialized definition of an external input slot on an [`EffectAsset`].
#[derive(Clone, Serialize, Deserialize)]
pub struct SerializableEffectInputSlot {
    pub name: String,
    pub id: EffectInputSlotId,
    pub ty: String,
}

/// The serialized definition of an external output slot on an [`EffectAsset`].
#[derive(Clone, Serialize, Deserialize)]
pub struct SerializableEffectOutputSlot {
    pub name: String,
    pub id: EffectOutputSlotId,
    pub ty: String,
}

/// Determines how compute workgroups and `dispatch_index` values are scheduled for an effect pass.
///
/// - `Once`: Dispatches a single invocation (`[1, 1, 1]`) with `dispatch_index = 0u`.
/// - `EveryBufferElement`: Dispatches 1D workgroups across the length of the specified array or
///   atomic-array pass input slot, providing a `u32` element index.
/// - `EveryOutputLayerPixel`: Dispatches 2D tiled workgroups across the allocated tiles of the
///   specified writable output layer, providing a `vec2i` pixel coordinate.
/// - `EveryInputLayerPixel`: Dispatches 2D tiled workgroups across the tiles of the specified
///   input layer, providing a `vec2i` pixel coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectPassDispatchStrategy {
    Once,
    EveryBufferElement(EffectPassInputSlotId),
    EveryOutputLayerPixel(EffectPassOutputSlotId),
    EveryInputLayerPixel(EffectPassInputSlotId),
}
