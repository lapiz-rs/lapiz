use std::{
    collections::HashMap,
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

use anyhow::{Result, anyhow};
use lapiz_runtime::global::Global;

use crate::asset::{Asset, ErasedAsset};

/// A global registry of asset serializers indexed by file extension.
///
/// Maps file extensions to their corresponding [`ErasedAssetSerializer`] instances, enabling
/// dynamic serializer lookup when reading and writing asset files from bundles.
#[derive(Default, Clone)]
pub struct AssetSerializerRegistry {
    serializers: HashMap<&'static str, Arc<dyn ErasedAssetSerializer>>,
}

impl Global for AssetSerializerRegistry {}

impl AssetSerializerRegistry {
    pub fn register<T: AssetSerializer + Default>(&mut self) {
        self.serializers
            .insert(T::file_extension(), Arc::new(T::default()));
    }

    pub fn get(&self, ext: &str) -> Option<Arc<dyn ErasedAssetSerializer>> {
        self.serializers.get(ext).cloned()
    }

    pub fn get_for_path(&self, path: impl AsRef<Path>) -> Result<Arc<dyn ErasedAssetSerializer>> {
        let path = path.as_ref();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| anyhow!("Missing extension for asset path: {}", path.display()))?;
        self.get(ext)
            .ok_or_else(|| anyhow!("No serializer found for asset extension: {ext}"))
    }
}

/// Serializer and deserializer for a specific [`Asset`] type.
///
/// Associates an asset type with a file extension and provides methods to read and write
/// asset instances from and to byte streams.
///
/// # Registration
///
/// Implementations must be registered with the [`AssetSerializerRegistry`] (or via
/// [`AssetAppExt::add_asset_serializer`](crate::AssetAppExt::add_asset_serializer)) to allow
/// bundles to automatically select the correct serializer when loading files.
///
/// All types implementing `AssetSerializer` automatically implement [`ErasedAssetSerializer`].
pub trait AssetSerializer: Send + Sync + 'static {
    type Asset: Asset;
    fn file_extension() -> &'static str;
    fn read(&self, relative_path: &Path, reader: &mut dyn Read) -> Result<Self::Asset>;
    fn write(&self, asset: &Self::Asset, writer: &mut dyn Write) -> Result<()>;
}

pub trait ErasedAssetSerializer: Send + Sync + 'static {
    fn file_extension(&self) -> &'static str;
    fn asset_type_name(&self) -> &'static str;
    fn read(&self, relative_path: &Path, reader: &mut dyn Read) -> Result<Box<dyn ErasedAsset>>;
    fn write(&self, asset: &dyn ErasedAsset, writer: &mut dyn Write) -> Result<()>;
}

impl<T: AssetSerializer> ErasedAssetSerializer for T {
    fn file_extension(&self) -> &'static str {
        <Self as AssetSerializer>::file_extension()
    }

    fn asset_type_name(&self) -> &'static str {
        <<Self as AssetSerializer>::Asset>::TYPE_NAME
    }

    fn read(&self, relative_path: &Path, reader: &mut dyn Read) -> Result<Box<dyn ErasedAsset>> {
        Ok(Box::new(<Self as AssetSerializer>::read(
            self,
            relative_path,
            reader,
        )?))
    }

    fn write(&self, asset: &dyn ErasedAsset, writer: &mut dyn Write) -> Result<()> {
        let asset = asset
            .as_any()
            .downcast_ref::<<Self as AssetSerializer>::Asset>()
            .ok_or_else(|| {
                anyhow!(
                    "Asset type mismatch for serializer {}",
                    <Self as AssetSerializer>::file_extension()
                )
            })?;
        <Self as AssetSerializer>::write(self, asset, writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializer_lookup_errors_keep_path_and_extension() {
        let registry = AssetSerializerRegistry::default();
        assert_eq!(
            registry
                .get_for_path("brushes/preset")
                .err()
                .unwrap()
                .to_string(),
            "Missing extension for asset path: brushes/preset"
        );
        assert_eq!(
            registry
                .get_for_path("brushes/preset.lapiz")
                .err()
                .unwrap()
                .to_string(),
            "No serializer found for asset extension: lapiz"
        );
    }
}
