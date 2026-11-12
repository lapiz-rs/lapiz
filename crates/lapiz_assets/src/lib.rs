use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::Result;
use lapiz_runtime::{Runtime, global::Globals, plugin::Plugin};
use lapiz_utils::log_err::LogErr as _;

use crate::{
    bundle::{directory::BuiltinAssetsDirectorySource, standard::StandardAssetBundleSource},
    loader::{AssetSerializer, AssetSerializerRegistry},
    source::{AssetSourceRegistry, AssetSourceRegistryExt as _},
    store::AssetRegistry,
};

pub mod asset;
pub mod bundle;
pub mod embedded;
pub mod index_db;
pub mod loader;
pub mod source;
pub mod store;
pub mod tag;

/// Plugin that initializes the asset subsystem and registers default bundle sources.
///
/// During [`Plugin::build`], this plugin extracts embedded default assets if the asset root is
/// empty, initializes the [`AssetSourceRegistry`] and [`AssetSerializerRegistry`] globals, and
/// registers [`BuiltinAssetsDirectorySource`] and [`StandardAssetBundleSource`].
///
/// During [`Plugin::finish`], it scans all registered sources in the asset root, constructs and
/// synchronizes the [`AssetRegistry`] with its SQLite index database, and inserts the registry
/// into [`Globals`].
pub struct AssetsPlugin {
    asset_root: PathBuf,
}

impl AssetsPlugin {
    pub fn new(asset_root: impl AsRef<Path>) -> Self {
        Self {
            asset_root: asset_root.as_ref().to_path_buf(),
        }
    }
}

impl Plugin for AssetsPlugin {
    fn build(&self, app: &mut Runtime) {
        embedded::extract_if_empty(&self.asset_root).log_err();

        app.add_global::<AssetSourceRegistry>()
            .add_global::<AssetSerializerRegistry>();
        app.globals_mut()
            .add_asset_source::<BuiltinAssetsDirectorySource>()
            .add_asset_source::<StandardAssetBundleSource>();
    }

    fn finish(&self, app: &mut Runtime) {
        let sources = app.globals().global::<AssetSourceRegistry>();
        let all_bundles = sources.scan_all(&self.asset_root);

        let build_registry = || {
            let registry = AssetRegistry::new(
                &self.asset_root,
                app.globals()
                    .global::<AssetSerializerRegistry>()
                    .clone()
                    .into(),
            )?;
            registry.add_erased_bundles(all_bundles)?;
            let loaded_bundle_ids = registry
                .bundles()
                .map(|bundle| bundle.metadata().bundle_id)
                .collect::<HashSet<_>>();
            registry
                .index_db()
                .remove_unloaded_bundles(&loaded_bundle_ids)?;
            Result::<AssetRegistry, anyhow::Error>::Ok(registry)
        };

        if let Ok(registry) = build_registry().logged_err() {
            app.add_global_instance(registry);
        }
    }
}

/// Extension trait providing asset-related helper methods on [`Globals`].
///
/// Offers convenient registration of typed [`AssetSerializer`]s and access to the
/// global [`AssetRegistry`].
pub trait AssetAppExt {
    fn add_asset_serializer<A: AssetSerializer + Default>(&mut self);
    fn assets(&self) -> &AssetRegistry;
}

impl AssetAppExt for Globals {
    fn add_asset_serializer<A: AssetSerializer + Default>(&mut self) {
        self.global_mut::<AssetSerializerRegistry>().register::<A>();
    }

    fn assets(&self) -> &AssetRegistry {
        self.global::<AssetRegistry>()
    }
}
