use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, anyhow};
use chrono::Utc;
use dashmap::DashMap;
use lapiz_runtime::global::Global;
use path_clean::PathClean as _;

use crate::{
    asset::{Asset, AssetHandle, AssetId, AssetMetadata, ErasedAsset as _, UntypedAssetId},
    bundle::{
        AssetBundle, AssetBundleCache, BundleId, BundleManifest, BundleSnapshot, ErasedAssetBundle,
        read_asset_tags_file, read_tag_file, scan_bundle_assets,
    },
    index_db::{AssetFilter, AssetIndexDb, TagFilter, UntypedAssetFilter},
    loader::AssetSerializerRegistry,
    tag::{AssetTags, Tag, TagId},
};

/// Central service managing asset bundles, in-memory caches, serialization, and index queries.
///
/// `AssetRegistry` acts as the primary access point for the asset subsystem. It holds an
/// [`AssetIndexDb`] for persistent metadata queries and an active collection of [`AssetBundleCache`]
/// instances for reading and mutating assets.
///
/// # Key Operations
///
/// - **Handle Retrieval**: Access loaded assets through typed [`AssetHandle`] references.
/// - **Asset Insertion**: Register newly created assets into specific bundles.
/// - **Queries**: Query assets and tags matching typed [`AssetFilter`] criteria.
/// - **Bundle Management**: Add and scan bundles from registered sources.
#[derive(Clone)]
pub struct AssetRegistry {
    inner: Arc<AssetRegistryInner>,
}

impl Global for AssetRegistry {}

pub struct AssetRegistryInner {
    root: PathBuf,
    bundles: DashMap<BundleId, Arc<AssetBundleCache>>,
    serializers: Arc<AssetSerializerRegistry>,
    index_db: Arc<AssetIndexDb>,
}

impl AssetRegistry {
    pub fn new(root: impl AsRef<Path>, serializers: Arc<AssetSerializerRegistry>) -> Result<Self> {
        let root = root.as_ref();
        let index_db = AssetIndexDb::connect(root.join("index.sqlite3"))?;

        Ok(Self {
            inner: Arc::new(AssetRegistryInner {
                root: root.to_path_buf(),
                bundles: DashMap::new(),
                index_db: Arc::new(index_db),
                serializers,
            }),
        })
    }

    pub fn new_in_memory(serializers: Arc<AssetSerializerRegistry>) -> Self {
        Self {
            inner: Arc::new(AssetRegistryInner {
                root: PathBuf::new(),
                bundles: DashMap::new(),
                serializers,
                index_db: Arc::new(AssetIndexDb::open_in_memory().expect("in-memory index db")),
            }),
        }
    }

    pub fn index_db(&self) -> &AssetIndexDb {
        &self.inner.index_db
    }

    pub fn bundles(&self) -> impl Iterator<Item = Arc<AssetBundleCache>> + '_ {
        self.inner.bundles.iter().map(|entry| entry.value().clone())
    }

    pub fn add_asset<T: Asset>(
        &self,
        bundle_id: BundleId,
        path: impl AsRef<Path>,
        asset: Arc<T>,
    ) -> Result<AssetId<T>> {
        let bundle = self
            .inner
            .bundles
            .get(&bundle_id)
            .ok_or_else(|| anyhow!("Asset bundle not found for bundle ID: {bundle_id}"))?
            .clone();
        let asset_id = bundle.add_asset(&path, asset.clone())?;
        self.inner.index_db.add_asset(&AssetMetadata {
            asset_id,
            ty: asset.type_name().to_string(),
            bundle_id,
            relative_path: path.as_ref().to_path_buf().to_string_lossy().to_string(),
            revision: 0,
            // TODO: This can be different from that in fs. But this field is only used for tags to determine if they are outdated.
            //       Probably fix it?
            last_modified: Utc::now(),
            in_memory: false,
        })?;
        Ok(asset_id.into_typed())
    }

    pub fn add_erased_bundles(
        &self,
        bundles: impl IntoIterator<Item = Arc<dyn ErasedAssetBundle>>,
    ) -> Result<()> {
        let snapshots = bundles
            .into_iter()
            .map(|bundle| self.scan_bundle(bundle))
            .collect::<Result<Vec<_>>>()?;
        if snapshots.is_empty() {
            return Ok(());
        }

        self.inner.index_db.sync_bundles(&snapshots)?;

        for snapshot in snapshots {
            self.inner.bundles.insert(
                snapshot.metadata.bundle_id,
                Arc::new(AssetBundleCache::new(
                    self.inner.root.clone(),
                    snapshot,
                    self.inner.serializers.clone(),
                )),
            );
        }
        Ok(())
    }

    fn scan_bundle(&self, bundle: Arc<dyn ErasedAssetBundle>) -> Result<BundleSnapshot> {
        let metadata = bundle.metadata()?;
        let manifest = bundle.manifest()?;
        let assets = scan_bundle_assets(
            &self.inner.root,
            metadata.clone(),
            &manifest,
            &self.inner.serializers,
        )?;
        let tags = scan_tags(bundle.as_ref(), &manifest, metadata.bundle_id)?;
        let asset_tags = scan_asset_tags(
            self.inner.root.as_path(),
            &metadata.bundle_id,
            bundle.as_ref(),
            &manifest,
            &assets,
        )?;

        Ok(BundleSnapshot {
            bundle,
            metadata,
            manifest,
            assets,
            tags,
            asset_tags,
        })
    }

    pub fn add_bundle<B: AssetBundle>(&self, bundle: B) -> Result<()> {
        self.add_erased_bundles([Arc::new(bundle) as _])
    }

    pub fn handle<T: Asset>(&self, asset_id: AssetId<T>) -> Result<AssetHandle<T>> {
        let bundle_id = self
            .inner
            .index_db
            .get_asset(&asset_id.into_untyped())?
            .bundle_id;
        let bundle = self
            .inner
            .bundles
            .get(&bundle_id)
            .ok_or_else(|| anyhow!("Asset bundle not found for bundle ID: {bundle_id}"))?
            .clone();

        Ok(AssetHandle::new(
            asset_id,
            bundle,
            self.inner.index_db.clone(),
        ))
    }

    pub fn all_handles_of<T: Asset>(&self) -> Result<Vec<AssetHandle<T>>> {
        Ok(
            self.metadata_to_handles(self.inner.index_db.get_assets(UntypedAssetFilter {
                ty: Some(T::TYPE_NAME.to_string()),
                ..Default::default()
            })?),
        )
    }

    pub fn all_handles_of_filtered<T: Asset>(
        &self,
        filter: AssetFilter<T>,
    ) -> Result<Vec<AssetHandle<T>>> {
        Ok(self.metadata_to_handles(self.inner.index_db.get_assets(filter.into_untyped())?))
    }

    pub fn all_tags(&self) -> Result<Vec<Tag>> {
        self.inner.index_db.get_tags(TagFilter::default())
    }

    pub fn all_tags_of<T: Asset>(&self) -> Result<Vec<Tag>> {
        self.inner.index_db.get_tags(TagFilter {
            asset_ty: Some(Some(T::TYPE_NAME.to_string())),
            ..Default::default()
        })
    }

    pub fn all_tags_filtered(&self, filter: TagFilter) -> Result<Vec<Tag>> {
        self.inner.index_db.get_tags(filter)
    }

    pub fn add_tag(&self, mut tag: Tag) -> Result<()> {
        let bundle = self
            .inner
            .bundles
            .get(&tag.bundle_id)
            .ok_or_else(|| anyhow!("Asset bundle not found for bundle ID: {}", tag.bundle_id))?
            .clone();
        tag.relative_path = PathBuf::from(&tag.relative_path)
            .clean()
            .to_string_lossy()
            .replace('\\', "/");
        bundle.add_tag(&tag)?;
        self.inner.index_db.add_tag(tag)
    }

    pub fn delete_tag(&self, tag_id: &TagId) -> Result<()> {
        self.inner.index_db.delete_tag(tag_id)
    }

    pub fn serializers(&self) -> &AssetSerializerRegistry {
        &self.inner.serializers
    }

    fn metadata_to_handles<T: Asset>(&self, metadata: Vec<AssetMetadata>) -> Vec<AssetHandle<T>> {
        metadata
            .into_iter()
            .filter_map(|meta| {
                Some(AssetHandle::new(
                    meta.asset_id.into_typed(),
                    self.inner.bundles.get(&meta.bundle_id)?.clone(),
                    self.inner.index_db.clone(),
                ))
            })
            .collect()
    }
}

fn scan_tags(
    bundle: &dyn ErasedAssetBundle,
    manifest: &BundleManifest,
    bundle_id: BundleId,
) -> Result<Vec<Tag>> {
    manifest
        .tags
        .values()
        .map(|path| {
            let tag = read_tag_file(path, bundle)?;
            Ok(Tag {
                id: tag.id,
                bundle_id,
                relative_path: path.to_string_lossy().replace('\\', "/"),
                name: tag.name,
                asset_ty: tag.asset_ty,
            })
        })
        .collect()
}

fn scan_asset_tags(
    assets_root: &Path,
    bundle_id: &BundleId,
    bundle: &dyn ErasedAssetBundle,
    manifest: &BundleManifest,
    assets: &[AssetMetadata],
) -> Result<HashMap<UntypedAssetId, AssetTags>> {
    let mut asset_tags = HashMap::new();

    for asset in assets {
        let asset_path = manifest
            .assets
            .get(&asset.asset_id)
            .ok_or_else(|| anyhow!("Asset path not found for asset ID: {}", asset.asset_id))?;
        let tags = read_asset_tags_file(assets_root, asset_path, bundle_id, bundle)?;
        asset_tags.insert(asset.asset_id, tags.unwrap_or_default());
    }

    Ok(asset_tags)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        env, fs,
        io::{self, Read, Write},
    };

    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use super::*;
    use crate::{
        bundle::directory::AssetDirectory,
        loader::AssetSerializer,
        tag::{AssetTags, TagFile},
    };

    #[derive(Debug, Serialize, Deserialize)]
    struct TestAsset {
        value: u32,
    }

    impl Asset for TestAsset {
        const TYPE_NAME: &'static str = "store_test_asset";
    }

    #[derive(Default)]
    struct TestAssetSerializer;

    impl AssetSerializer for TestAssetSerializer {
        type Asset = TestAsset;

        fn file_extension() -> &'static str {
            "storetest"
        }

        fn read(&self, reader: &mut dyn Read) -> Result<Self::Asset> {
            let mut value = String::new();
            reader.read_to_string(&mut value)?;
            Ok(TestAsset {
                value: value.trim().parse().map_err(io::Error::other)?,
            })
        }

        fn write(&self, asset: &Self::Asset, writer: &mut dyn Write) -> Result<()> {
            write!(writer, "{}", asset.value)?;
            Ok(())
        }
    }

    #[test]
    fn batch_sync_resolves_cross_bundle_tags_independent_of_order() -> Result<()> {
        let root = temp_root("cross-bundle");
        let assets_root = root.join("assets-bundle");
        let tags_root = root.join("tags-bundle");
        fs::create_dir_all(&assets_root)?;
        fs::create_dir_all(&tags_root)?;

        let tag = TagFile::new(
            "Test tag".to_string(),
            Some(TestAsset::TYPE_NAME.to_string()),
        );
        fs::write(assets_root.join("sample.storetest"), "42")?;
        fs::write(
            assets_root.join("sample.storetest.tags"),
            toml::to_string(&AssetTags {
                tags: BTreeSet::from([tag.id]),
            })?,
        )?;
        fs::write(tags_root.join("test.tag"), toml::to_string(&tag)?)?;

        let assets_bundle = AssetDirectory::new(&assets_root)?;
        let assets_bundle_id = AssetBundle::metadata(&assets_bundle)?.bundle_id;
        let tags_bundle = AssetDirectory::new(&tags_root)?;
        let tags_bundle_id = AssetBundle::metadata(&tags_bundle)?.bundle_id;
        let registry =
            registry_with_bundles(&root, vec![Arc::new(assets_bundle), Arc::new(tags_bundle)])?;

        let stored_tag = registry.index_db().get_tag(tag.id)?;
        assert_eq!(stored_tag.bundle_id, tags_bundle_id);
        assert_eq!(stored_tag.relative_path, "test.tag");

        let handles = registry.all_handles_of::<TestAsset>()?;
        assert_eq!(handles.len(), 1);
        let handle = &handles[0];
        assert_eq!(handle.read_tags()?, BTreeSet::from([tag.id]));

        handle.remove_tag(&tag.id)?;
        assert!(handle.read_tags()?.is_empty());
        assert!(
            registry
                .all_handles_of_filtered::<TestAsset>(AssetFilter::new().with_tag(tag.id))?
                .is_empty()
        );
        assert!(handle.remove_tag(&tag.id).is_err());

        handle.add_tag(&tag.id)?;
        assert_eq!(handle.read_tags()?, BTreeSet::from([tag.id]));
        assert_eq!(
            registry
                .all_handles_of_filtered::<TestAsset>(AssetFilter::new().with_tag(tag.id))?
                .len(),
            1
        );
        assert!(handle.add_tag(&tag.id).is_err());

        let added_tag = Tag {
            id: TagId::new(Uuid::new_v4()),
            bundle_id: assets_bundle_id,
            relative_path: "runtime/added.tag".to_string(),
            name: "Added at runtime".to_string(),
            asset_ty: None,
        };
        registry.add_tag(added_tag.clone())?;
        assert!(assets_root.join("runtime/added.tag").is_file());
        let stored_tag = registry.index_db().get_tag(added_tag.id)?;
        assert_eq!(stored_tag.bundle_id, added_tag.bundle_id);
        assert_eq!(stored_tag.relative_path, added_tag.relative_path);
        assert_eq!(stored_tag.name, added_tag.name);
        assert_eq!(stored_tag.asset_ty, added_tag.asset_ty);

        drop(handles);
        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn bundle_sync_removes_tags_missing_from_disk() -> Result<()> {
        let root = temp_root("missing-tag-sync");
        let bundle_root = root.join("tag-bundle");
        fs::create_dir_all(&bundle_root)?;
        let tag = TagFile::new("Removed tag".to_string(), None);
        fs::write(bundle_root.join("removed.tag"), toml::to_string(&tag)?)?;

        let registry =
            registry_with_bundles(&root, vec![Arc::new(AssetDirectory::new(&bundle_root)?)])?;
        assert_eq!(
            registry.index_db().get_tag(tag.id)?.relative_path,
            "removed.tag"
        );

        fs::remove_file(bundle_root.join("removed.tag"))?;
        let bundle = Arc::new(AssetDirectory::new(&bundle_root)?) as Arc<dyn ErasedAssetBundle>;
        registry.add_erased_bundles([bundle])?;

        assert!(registry.index_db().get_tag(tag.id).is_err());
        assert!(registry.index_db().restore_tag(&tag.id).is_err());

        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn duplicate_tag_ids_across_bundles_are_rejected() -> Result<()> {
        let root = temp_root("duplicate-tag-id");
        let first_root = root.join("first-tags");
        let second_root = root.join("second-tags");
        fs::create_dir_all(&first_root)?;
        fs::create_dir_all(&second_root)?;
        let tag = TagFile::new("Duplicate".to_string(), None);
        let serialized = toml::to_string(&tag)?;
        fs::write(first_root.join("first.tag"), &serialized)?;
        fs::write(second_root.join("second.tag"), serialized)?;

        assert!(
            registry_with_bundles(
                &root,
                vec![
                    Arc::new(AssetDirectory::new(&first_root)?),
                    Arc::new(AssetDirectory::new(&second_root)?),
                ],
            )
            .is_err()
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn startup_removes_tags_from_unloaded_bundles() -> Result<()> {
        let root = temp_root("unloaded-tag-bundle");
        let bundle_root = root.join("tag-bundle");
        fs::create_dir_all(&bundle_root)?;
        let tag = TagFile::new("Unloaded tag".to_string(), None);
        fs::write(bundle_root.join("unloaded.tag"), toml::to_string(&tag)?)?;
        let bundle = AssetDirectory::new(&bundle_root)?;
        let bundle_id = AssetBundle::metadata(&bundle)?.bundle_id;

        let registry = registry_with_bundles(&root, vec![Arc::new(bundle)])?;
        assert!(registry.index_db().get_tag(tag.id).is_ok());
        drop(registry);

        let registry = registry_with_bundles(&root, vec![])?;
        assert!(registry.index_db().get_tag(tag.id).is_err());
        assert!(registry.index_db().get_bundle(&bundle_id).is_err());

        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn bundle_sync_does_not_restore_deleted_assets() -> Result<()> {
        let root = temp_root("deleted-asset-sync");
        let bundle_root = root.join("asset-bundle");
        fs::create_dir_all(&bundle_root)?;
        fs::write(bundle_root.join("sample.storetest"), "9")?;

        let registry =
            registry_with_bundles(&root, vec![Arc::new(AssetDirectory::new(&bundle_root)?)])?;

        let handle = registry.all_handles_of::<TestAsset>()?.remove(0);
        let asset_id = handle.untyped_id();
        handle.delete()?;

        let bundle = Arc::new(AssetDirectory::new(&bundle_root)?) as Arc<dyn ErasedAssetBundle>;
        registry.add_erased_bundles([bundle])?;

        assert!(registry.all_handles_of::<TestAsset>()?.is_empty());
        let deleted = registry.index_db().get_assets(UntypedAssetFilter {
            is_deleted: true,
            ..Default::default()
        })?;
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].asset_id, asset_id);

        drop(handle);
        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn bundle_sync_keeps_active_assets_and_removes_missing_assets() -> Result<()> {
        let root = temp_root("missing-asset-sync");
        let bundle_root = root.join("asset-bundle");
        fs::create_dir_all(&bundle_root)?;
        fs::write(bundle_root.join("retained.storetest"), "1")?;
        fs::write(bundle_root.join("removed.storetest"), "2")?;

        let registry =
            registry_with_bundles(&root, vec![Arc::new(AssetDirectory::new(&bundle_root)?)])?;
        assert_eq!(registry.all_handles_of::<TestAsset>()?.len(), 2);

        fs::remove_file(bundle_root.join("removed.storetest"))?;
        let bundle = Arc::new(AssetDirectory::new(&bundle_root)?) as Arc<dyn ErasedAssetBundle>;
        registry.add_erased_bundles([bundle])?;

        let active = registry
            .index_db()
            .get_assets(UntypedAssetFilter::default())?;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].relative_path, "retained.storetest");

        let deleted = registry.index_db().get_assets(UntypedAssetFilter {
            is_deleted: true,
            ..Default::default()
        })?;
        assert!(deleted.is_empty());

        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn corrupt_sidecar_does_not_advance_bundle_metadata() -> Result<()> {
        let root = temp_root("corrupt-sidecar");
        let bundle_root = root.join("asset-bundle");
        fs::create_dir_all(&bundle_root)?;
        fs::write(bundle_root.join("sample.storetest"), "9")?;
        let bundle = AssetDirectory::new(&bundle_root)?;
        let bundle_id = AssetBundle::metadata(&bundle)?.bundle_id;

        let registry = registry_with_bundles(&root, vec![Arc::new(bundle)])?;
        let previous_metadata = registry.index_db().get_bundle(&bundle_id)?;
        drop(registry);

        fs::write(bundle_root.join("sample.storetest.tags"), "tags = [")?;
        assert!(
            registry_with_bundles(&root, vec![Arc::new(AssetDirectory::new(&bundle_root)?)],)
                .is_err()
        );

        let index = AssetIndexDb::connect(root.join("index.sqlite3"))?;
        assert_eq!(
            index.get_bundle(&bundle_id)?.last_modified,
            previous_metadata.last_modified
        );
        drop(index);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn registry_with_bundles(
        root: &Path,
        bundles: Vec<Arc<dyn ErasedAssetBundle>>,
    ) -> Result<AssetRegistry> {
        let mut serializers = AssetSerializerRegistry::default();
        serializers.register::<TestAssetSerializer>();
        let registry = AssetRegistry::new(root, Arc::new(serializers))?;
        registry.add_erased_bundles(bundles)?;
        let loaded_ids = registry
            .bundles()
            .map(|bundle| bundle.metadata().bundle_id)
            .collect();
        registry.index_db().remove_unloaded_bundles(&loaded_ids)?;
        Ok(registry)
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("lapiz-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
