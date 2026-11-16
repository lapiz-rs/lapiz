use std::{
    ffi::OsStr,
    fs::{self, File, create_dir_all, metadata, read_to_string},
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use chrono::DateTime;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xxhash_rust::xxh3::xxh3_128;

use crate::{
    asset::{ErasedAsset, UntypedAssetId},
    bundle::{AssetBundle, AssetBundleMetadata, BundleId, BundleManifest, ErasedAssetBundle},
    loader::ErasedAssetSerializer,
    source::AssetSource,
    tag::{TAG_EXT, TAGGED_ASSETS_EXT, TagFile, TaggedAssets},
};

/// An [`AssetSource`] that discovers loose asset bundles in the `builtin_assets` directory.
pub struct BuiltinAssetsDirectorySource;

impl AssetSource for BuiltinAssetsDirectorySource {
    fn scan(root: &Path) -> Vec<Arc<dyn ErasedAssetBundle>> {
        if let Ok(bundle) = AssetDirectory::new(root.join("builtin_assets")) {
            vec![Arc::new(bundle)]
        } else {
            vec![]
        }
    }
}

const METADATA_FILE_NAME: &str = "metadata.toml";

/// On-disk metadata stored in `metadata.toml` for an uncompressed asset directory bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDirectoryMetadata {
    pub bundle_id: BundleId,
}

/// A filesystem directory representing an uncompressed, mutable [`AssetBundle`].
///
/// Discovers assets and tags by recursively traversing the directory tree. Because it is
/// mutable (`READONLY = false`), newly created assets (at revision 0) and tags can be written
/// directly into the directory structure, while subsequent asset revisions are still tracked
/// in `.modified`.
pub struct AssetDirectory {
    root: PathBuf,
    id: BundleId,
    name: String,
    manifest: Mutex<BundleManifest>,
}

impl AssetDirectory {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let metadata_path = root.join(METADATA_FILE_NAME);
        let metadata = if metadata_path.exists() {
            toml::from_str(&read_to_string(&metadata_path)?)?
        } else {
            let metadata = AssetDirectoryMetadata {
                bundle_id: BundleId::new(Uuid::new_v4()),
            };
            File::create_new(&metadata_path)?.write_all(toml::to_string(&metadata)?.as_bytes())?;
            metadata
        };

        let mut manifest = BundleManifest::default();
        scan_dir_dfs(root, root, &metadata.bundle_id, &mut manifest)?;

        Ok(Self {
            id: metadata.bundle_id,
            name,
            root: root.into(),
            manifest: Mutex::new(manifest),
        })
    }
}

impl AssetBundle for AssetDirectory {
    const READONLY: bool = false;

    fn metadata(&self) -> Result<AssetBundleMetadata> {
        let last_modified = DateTime::from(metadata(&self.root)?.modified()?);

        Ok(AssetBundleMetadata {
            bundle_id: self.id,
            name: self.name.clone(),
            last_modified,
        })
    }

    fn manifest(&self) -> Result<BundleManifest> {
        Ok(self.manifest.lock().clone())
    }

    fn read_asset(
        &self,
        path: &Path,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<Arc<dyn ErasedAsset>> {
        let asset_path = self.root.join(path);
        let mut file = File::open(&asset_path)?;
        let asset = serializer.read(path, &mut file)?;
        Ok(asset.into())
    }

    fn add_asset(
        &self,
        path: &Path,
        asset: &dyn ErasedAsset,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<UntypedAssetId> {
        let path = path_clean::clean(path);
        let asset_path = self.root.join(&path);
        if let Some(parent) = asset_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&asset_path)?;
        serializer.write(asset, &mut file)?;
        let asset_id = asset_id_from_relative_path(&self.id, &path);
        self.manifest.lock().assets.insert(asset_id, path);
        Ok(asset_id)
    }

    fn read_tag(&self, tag: &Path) -> Result<TagFile> {
        let path = self.root.join(tag);
        Ok(toml::from_str(&read_to_string(path)?)?)
    }

    fn add_tag(&self, path: &Path, tag: &TagFile) -> Result<()> {
        let path = path_clean::clean(path);
        let tag_path = self.root.join(&path);
        if let Some(parent) = tag_path.parent() {
            create_dir_all(parent)?;
        }
        File::create(tag_path)?.write_all(toml::to_string(tag)?.as_bytes())?;

        self.manifest.lock().tags.insert(tag.id, path);

        Ok(())
    }

    fn read_tagged_assets(&self, path: &Path) -> Result<Option<TaggedAssets>> {
        let path = self.root.join(path).with_extension(TAGGED_ASSETS_EXT);
        if !path.exists() {
            return Ok(None);
        }

        Ok(Some(toml::from_str(&read_to_string(path)?)?))
    }

    fn write_tagged_assets(&self, path: &Path, assets: &TaggedAssets) -> Result<()> {
        let path = self.root.join(path).with_extension(TAGGED_ASSETS_EXT);
        if let Some(parent) = path.parent() {
            create_dir_all(parent)?;
        }
        File::create(path)?.write_all(toml::to_string(assets)?.as_bytes())?;
        Ok(())
    }
}

fn scan_dir_dfs(
    current_path: &Path,
    root: &Path,
    bundle_id: &BundleId,
    manifest: &mut BundleManifest,
) -> Result<()> {
    let entries = fs::read_dir(current_path)?;
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };

        let path = entry.path();
        if path.is_dir() {
            scan_dir_dfs(&path, root, bundle_id, manifest)?;
        } else {
            if path.extension() == Some(OsStr::new(TAGGED_ASSETS_EXT)) {
                continue;
            }

            let relative_path = path.strip_prefix(root).unwrap();
            if relative_path == Path::new(METADATA_FILE_NAME) {
                continue;
            }

            if path.extension() == Some(OsStr::new(TAG_EXT)) {
                let tag = toml::from_slice::<TagFile>(&fs::read(&path)?)?;
                manifest.tags.insert(tag.id, relative_path.into());
            } else {
                let asset_id = asset_id_from_relative_path(bundle_id, relative_path);

                manifest.assets.insert(asset_id, relative_path.into());
            }
        }
    }

    Ok(())
}

fn asset_id_from_relative_path(bundle_id: &BundleId, path: &Path) -> UntypedAssetId {
    let path_str = path_clean::clean(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/");
    let path_bytes = path_str.as_bytes();
    let mut key = Vec::with_capacity(bundle_id.as_bytes().len() + path_bytes.len());
    key.extend_from_slice(bundle_id.as_bytes());
    key.extend_from_slice(path_bytes);

    UntypedAssetId::new(Uuid::from_u128(xxh3_128(&key)))
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::Path};

    use anyhow::Result;
    use indexmap::IndexSet;
    use uuid::Uuid;

    use super::{AssetDirectory, asset_id_from_relative_path};
    use crate::{
        asset::UntypedAssetId,
        bundle::{AssetBundle as _, BundleId},
        tag::TaggedAssets,
    };

    #[test]
    fn asset_id_is_namespaced_by_bundle_id() {
        let path = Path::new("brushes/sample.lapiz");
        let first_bundle = BundleId::new(Uuid::from_u128(1));
        let second_bundle = BundleId::new(Uuid::from_u128(2));

        let first_id = asset_id_from_relative_path(&first_bundle, path);
        assert_eq!(
            first_id,
            asset_id_from_relative_path(&first_bundle, Path::new("./brushes/sample.lapiz"))
        );
        assert_ne!(first_id, asset_id_from_relative_path(&second_bundle, path));
    }

    #[test]
    fn tagged_assets_round_trip_order_and_empty_override() -> Result<()> {
        let root = env::temp_dir().join(format!("lapiz-tagged-assets-{}", Uuid::new_v4()));
        fs::create_dir_all(&root)?;
        let bundle = AssetDirectory::new(&root)?;
        let tag_path = Path::new("tags/sample.tag");
        let first = UntypedAssetId::new(Uuid::from_u128(2));
        let second = UntypedAssetId::new(Uuid::from_u128(1));

        assert!(bundle.read_tagged_assets(tag_path)?.is_none());
        bundle.write_tagged_assets(
            tag_path,
            &TaggedAssets {
                assets: IndexSet::from([first, second]),
            },
        )?;
        assert_eq!(
            bundle
                .read_tagged_assets(tag_path)?
                .unwrap()
                .assets
                .into_iter()
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        assert!(root.join("tags/sample.tagged_assets").is_file());

        let rescanned = AssetDirectory::new(&root)?.manifest()?;
        assert!(rescanned.assets.is_empty());
        assert!(rescanned.tags.is_empty());

        bundle.write_tagged_assets(tag_path, &TaggedAssets::default())?;
        assert!(
            bundle
                .read_tagged_assets(tag_path)?
                .unwrap()
                .assets
                .is_empty()
        );
        fs::write(root.join("tags/sample.tagged_assets"), "invalid toml[")?;
        assert!(bundle.read_tagged_assets(tag_path).is_err());

        fs::remove_dir_all(root)?;
        Ok(())
    }
}
