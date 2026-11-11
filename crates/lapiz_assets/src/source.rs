use std::{any::TypeId, collections::HashMap, path::Path, sync::Arc};

use lapiz_runtime::global::{Global, Globals};

use crate::bundle::ErasedAssetBundle;

/// A discovery scanner that locates and constructs asset bundles from an asset root.
///
/// Implementations scan a directory for specific bundle formats (such as standard `.lazurite`
/// files or uncompressed asset folders) and return discovered [`ErasedAssetBundle`] instances.
pub trait AssetSource: 'static {
    fn scan(root: &Path) -> Vec<Arc<dyn ErasedAssetBundle>>;
}

/// A global registry of asset bundle discovery scanners.
///
/// Stores factory functions for registered [`AssetSource`] implementations, allowing
/// the asset system to scan an asset root directory across multiple bundle formats.
#[derive(Default)]
pub struct AssetSourceRegistry {
    inner: HashMap<TypeId, Box<dyn Fn(&Path) -> Vec<Arc<dyn ErasedAssetBundle>>>>,
}

impl Global for AssetSourceRegistry {}

impl AssetSourceRegistry {
    pub fn register<T: AssetSource>(&mut self) {
        self.inner.insert(TypeId::of::<T>(), Box::new(T::scan));
    }

    pub fn scan_all(&self, root: &Path) -> Vec<Arc<dyn ErasedAssetBundle>> {
        self.inner.values().flat_map(|scan| scan(root)).collect()
    }
}

/// Extension trait providing builder-style registration of [`AssetSource`]s on [`Globals`].
pub trait AssetSourceRegistryExt {
    fn add_asset_source<T: AssetSource>(&mut self) -> &mut Self;
}

impl AssetSourceRegistryExt for Globals {
    fn add_asset_source<T: AssetSource>(&mut self) -> &mut Self {
        self.global_mut::<AssetSourceRegistry>().register::<T>();
        self
    }
}
