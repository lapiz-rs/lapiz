use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::Utc;
use lapiz_runtime::service::Service;
use path_clean::PathClean;

use crate::{
    asset::{Asset, AssetHandle, AssetId, AssetMetadata, ErasedAsset, UntypedAssetId},
    bundle::{
        AssetBundle, AssetBundleCache, BundleId, BundleManifest, BundleSnapshot, ErasedAssetBundle,
        read_asset_tags_file, read_tag_file, scan_bundle_assets,
    },
    error::{AssetErrorKind, AssetResult},
    index_db::{AssetFilter, AssetIndexDb, TagFilter, UntypedAssetFilter},
    loader::AssetSerializerRegistry,
    tag::{AssetTags, Tag, TagId},
};

pub struct AssetRegistry {
    root: PathBuf,
    bundles: HashMap<BundleId, Arc<AssetBundleCache>>,
    serializers: Arc<AssetSerializerRegistry>,
    index_db: Arc<AssetIndexDb>,
}

impl Service for AssetRegistry {}

impl AssetRegistry {
    pub fn new(
        root: impl AsRef<Path>,
        serializers: Arc<AssetSerializerRegistry>,
    ) -> AssetResult<Self> {
        let root = root.as_ref();
        let bundles = HashMap::new();
        let index_db = AssetIndexDb::connect(root.join("index.sqlite3"))?;

        Ok(Self {
            root: root.to_path_buf(),
            bundles,
            index_db: Arc::new(index_db),
            serializers,
        })
    }

    pub fn index_db(&self) -> &AssetIndexDb {
        &self.index_db
    }

    pub fn bundles(&self) -> impl Iterator<Item = &Arc<AssetBundleCache>> {
        self.bundles.values()
    }

    pub fn add_asset<T: Asset>(
        &self,
        bundle_id: BundleId,
        path: impl AsRef<Path>,
        asset: Arc<T>,
    ) -> AssetResult<AssetId<T>> {
        let bundle = self
            .bundles
            .get(&bundle_id)
            .ok_or_else(|| AssetErrorKind::BundleNotFound(bundle_id))?;
        let asset_id = bundle.add_asset(&path, asset.clone())?;
        self.index_db.add_asset(&AssetMetadata {
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
        &mut self,
        bundles: impl IntoIterator<Item = Arc<dyn ErasedAssetBundle>>,
    ) -> AssetResult<()> {
        let snapshots = bundles
            .into_iter()
            .map(|bundle| self.scan_bundle(bundle))
            .collect::<AssetResult<Vec<_>>>()?;
        if snapshots.is_empty() {
            return Ok(());
        }

        self.index_db.sync_bundles(&snapshots)?;

        for snapshot in snapshots {
            self.bundles.insert(
                snapshot.metadata.bundle_id,
                Arc::new(AssetBundleCache::new(
                    self.root.clone(),
                    snapshot,
                    self.serializers.clone(),
                )),
            );
        }
        Ok(())
    }

    fn scan_bundle(&self, bundle: Arc<dyn ErasedAssetBundle>) -> AssetResult<BundleSnapshot> {
        let metadata = bundle.metadata().map_err(AssetErrorKind::BundleError)?;
        let manifest = bundle.manifest().map_err(AssetErrorKind::BundleError)?;
        let assets =
            scan_bundle_assets(&self.root, metadata.clone(), &manifest, &self.serializers)?;
        let tags = scan_tags(bundle.as_ref(), &manifest, metadata.bundle_id)?;
        let asset_tags = scan_asset_tags(
            self.root.as_path(),
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

    pub fn add_bundle<B: AssetBundle>(&mut self, bundle: B) -> AssetResult<()> {
        self.add_erased_bundles([Arc::new(bundle) as _])
    }

    pub fn handle<T: Asset>(&self, asset_id: AssetId<T>) -> AssetResult<AssetHandle<T>> {
        let bundle_id = self.index_db.get_asset(&asset_id.into_untyped())?.bundle_id;
        let bundle = self
            .bundles
            .get(&bundle_id)
            .ok_or_else(|| AssetErrorKind::BundleNotFound(bundle_id))?;

        Ok(AssetHandle::new(
            asset_id,
            bundle.clone(),
            self.index_db.clone(),
        ))
    }

    pub fn all_handles_of<T: Asset>(&self) -> AssetResult<Vec<AssetHandle<T>>> {
        Ok(
            self.metadata_to_handles(self.index_db.get_assets(UntypedAssetFilter {
                ty: Some(T::TYPE_NAME.to_string()),
                ..Default::default()
            })?),
        )
    }

    pub fn all_handles_of_filtered<T: Asset>(
        &self,
        filter: AssetFilter<T>,
    ) -> AssetResult<Vec<AssetHandle<T>>> {
        Ok(self.metadata_to_handles(self.index_db.get_assets(filter.into_untyped())?))
    }

    pub fn all_tags(&self) -> AssetResult<Vec<Tag>> {
        self.index_db.get_tags(TagFilter::default())
    }

    pub fn all_tags_of<T: Asset>(&self) -> AssetResult<Vec<Tag>> {
        self.index_db.get_tags(TagFilter {
            asset_ty: Some(Some(T::TYPE_NAME.to_string())),
            ..Default::default()
        })
    }

    pub fn all_tags_filtered(&self, filter: TagFilter) -> AssetResult<Vec<Tag>> {
        self.index_db.get_tags(filter)
    }

    pub fn add_tag(&self, mut tag: Tag) -> AssetResult<()> {
        let bundle = self
            .bundles
            .get(&tag.bundle_id)
            .ok_or_else(|| AssetErrorKind::BundleNotFound(tag.bundle_id))?;
        tag.relative_path = PathBuf::from(&tag.relative_path)
            .clean()
            .to_string_lossy()
            .replace('\\', "/");
        bundle.add_tag(&tag)?;
        self.index_db.add_tag(tag)
    }

    pub fn delete_tag(&self, tag_id: &TagId) -> AssetResult<()> {
        self.index_db.delete_tag(tag_id)
    }

    pub fn serializers(&self) -> &AssetSerializerRegistry {
        &self.serializers
    }

    fn metadata_to_handles<T: Asset>(&self, metadata: Vec<AssetMetadata>) -> Vec<AssetHandle<T>> {
        metadata
            .into_iter()
            .filter_map(|meta| {
                Some(AssetHandle::new(
                    meta.asset_id.into_typed(),
                    self.bundles.get(&meta.bundle_id)?.clone(),
                    self.index_db.clone(),
                ))
            })
            .collect()
    }
}

fn scan_tags(
    bundle: &dyn ErasedAssetBundle,
    manifest: &BundleManifest,
    bundle_id: BundleId,
) -> AssetResult<Vec<Tag>> {
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
) -> AssetResult<HashMap<UntypedAssetId, AssetTags>> {
    let mut asset_tags = HashMap::new();

    for asset in assets {
        let asset_path = manifest
            .assets
            .get(&asset.asset_id)
            .ok_or_else(|| AssetErrorKind::AssetPathNotFound(asset.asset_id))?;
        let tags = read_asset_tags_file(assets_root, asset_path, bundle_id, bundle)?;
        asset_tags.insert(asset.asset_id, tags.unwrap_or_default());
    }

    Ok(asset_tags)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        io::{self, Read, Write},
    };

    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use super::*;
    use crate::tag::TagFile;
    use crate::{
        bundle::directory::AssetDirectory,
        loader::{AssetRegistryBuilder, AssetSerializer},
        tag::AssetTags,
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
        type Error = io::Error;

        fn file_extension() -> &'static str {
            "storetest"
        }

        fn read(&self, reader: &mut dyn Read) -> Result<Self::Asset, Self::Error> {
            let mut value = String::new();
            reader.read_to_string(&mut value)?;
            Ok(TestAsset {
                value: value.trim().parse().map_err(io::Error::other)?,
            })
        }

        fn write(&self, asset: &Self::Asset, writer: &mut dyn Write) -> Result<(), Self::Error> {
            write!(writer, "{}", asset.value)
        }
    }

    #[test]
    fn batch_sync_resolves_cross_bundle_tags_independent_of_order() -> AssetResult<()> {
        let root = temp_root("cross-bundle");
        let assets_root = root.join("assets-bundle");
        let tags_root = root.join("tags-bundle");
        std::fs::create_dir_all(&assets_root)?;
        std::fs::create_dir_all(&tags_root)?;

        let tag = TagFile::new(
            "Test tag".to_string(),
            Some(TestAsset::TYPE_NAME.to_string()),
        );
        std::fs::write(assets_root.join("sample.storetest"), "42")?;
        std::fs::write(
            assets_root.join("sample.storetest.tags"),
            toml::to_string(&AssetTags {
                tags: BTreeSet::from([tag.id]),
            })?,
        )?;
        std::fs::write(tags_root.join("test.tag"), toml::to_string(&tag)?)?;

        let assets_bundle = AssetDirectory::new(&assets_root)?;
        let assets_bundle_id = AssetBundle::metadata(&assets_bundle)
            .map_err(|error| AssetErrorKind::BundleError(Box::new(error)))?
            .bundle_id;
        let tags_bundle = AssetDirectory::new(&tags_root)?;
        let tags_bundle_id = AssetBundle::metadata(&tags_bundle)
            .map_err(|error| AssetErrorKind::BundleError(Box::new(error)))?
            .bundle_id;
        let mut builder = registry_builder(&root);
        builder.add_bundle(Arc::new(assets_bundle));
        builder.add_bundle(Arc::new(tags_bundle));
        let registry = builder.try_build()?;

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
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn bundle_sync_removes_tags_missing_from_disk() -> AssetResult<()> {
        let root = temp_root("missing-tag-sync");
        let bundle_root = root.join("tag-bundle");
        std::fs::create_dir_all(&bundle_root)?;
        let tag = TagFile::new("Removed tag".to_string(), None);
        std::fs::write(bundle_root.join("removed.tag"), toml::to_string(&tag)?)?;

        let mut builder = registry_builder(&root);
        builder.add_bundle(Arc::new(AssetDirectory::new(&bundle_root)?));
        let mut registry = builder.try_build()?;
        assert_eq!(
            registry.index_db().get_tag(tag.id)?.relative_path,
            "removed.tag"
        );

        std::fs::remove_file(bundle_root.join("removed.tag"))?;
        let bundle: Arc<dyn ErasedAssetBundle> = Arc::new(AssetDirectory::new(&bundle_root)?);
        registry.add_erased_bundles([bundle])?;

        assert!(registry.index_db().get_tag(tag.id).is_err());
        assert!(registry.index_db().restore_tag(&tag.id).is_err());

        drop(registry);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn duplicate_tag_ids_across_bundles_are_rejected() -> AssetResult<()> {
        let root = temp_root("duplicate-tag-id");
        let first_root = root.join("first-tags");
        let second_root = root.join("second-tags");
        std::fs::create_dir_all(&first_root)?;
        std::fs::create_dir_all(&second_root)?;
        let tag = TagFile::new("Duplicate".to_string(), None);
        let serialized = toml::to_string(&tag)?;
        std::fs::write(first_root.join("first.tag"), &serialized)?;
        std::fs::write(second_root.join("second.tag"), serialized)?;

        let mut builder = registry_builder(&root);
        builder.add_bundle(Arc::new(AssetDirectory::new(&first_root)?));
        builder.add_bundle(Arc::new(AssetDirectory::new(&second_root)?));
        assert!(builder.try_build().is_err());

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn startup_removes_tags_from_unloaded_bundles() -> AssetResult<()> {
        let root = temp_root("unloaded-tag-bundle");
        let bundle_root = root.join("tag-bundle");
        std::fs::create_dir_all(&bundle_root)?;
        let tag = TagFile::new("Unloaded tag".to_string(), None);
        std::fs::write(bundle_root.join("unloaded.tag"), toml::to_string(&tag)?)?;
        let bundle = AssetDirectory::new(&bundle_root)?;
        let bundle_id = AssetBundle::metadata(&bundle)
            .map_err(|error| AssetErrorKind::BundleError(Box::new(error)))?
            .bundle_id;

        let mut builder = registry_builder(&root);
        builder.add_bundle(Arc::new(bundle));
        let registry = builder.try_build()?;
        assert!(registry.index_db().get_tag(tag.id).is_ok());
        drop(registry);

        let registry = registry_builder(&root).try_build()?;
        assert!(registry.index_db().get_tag(tag.id).is_err());
        assert!(registry.index_db().get_bundle(&bundle_id).is_err());

        drop(registry);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn bundle_sync_does_not_restore_deleted_assets() -> AssetResult<()> {
        let root = temp_root("deleted-asset-sync");
        let bundle_root = root.join("asset-bundle");
        std::fs::create_dir_all(&bundle_root)?;
        std::fs::write(bundle_root.join("sample.storetest"), "9")?;

        let mut builder = registry_builder(&root);
        builder.add_bundle(Arc::new(AssetDirectory::new(&bundle_root)?));
        let mut registry = builder.try_build()?;

        let handle = registry.all_handles_of::<TestAsset>()?.remove(0);
        let asset_id = handle.untyped_id();
        handle.delete()?;

        let bundle: Arc<dyn ErasedAssetBundle> = Arc::new(AssetDirectory::new(&bundle_root)?);
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
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn bundle_sync_keeps_active_assets_and_removes_missing_assets() -> AssetResult<()> {
        let root = temp_root("missing-asset-sync");
        let bundle_root = root.join("asset-bundle");
        std::fs::create_dir_all(&bundle_root)?;
        std::fs::write(bundle_root.join("retained.storetest"), "1")?;
        std::fs::write(bundle_root.join("removed.storetest"), "2")?;

        let mut builder = registry_builder(&root);
        builder.add_bundle(Arc::new(AssetDirectory::new(&bundle_root)?));
        let mut registry = builder.try_build()?;
        assert_eq!(registry.all_handles_of::<TestAsset>()?.len(), 2);

        std::fs::remove_file(bundle_root.join("removed.storetest"))?;
        let bundle: Arc<dyn ErasedAssetBundle> = Arc::new(AssetDirectory::new(&bundle_root)?);
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
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn corrupt_sidecar_does_not_advance_bundle_metadata() -> AssetResult<()> {
        let root = temp_root("corrupt-sidecar");
        let bundle_root = root.join("asset-bundle");
        std::fs::create_dir_all(&bundle_root)?;
        std::fs::write(bundle_root.join("sample.storetest"), "9")?;
        let bundle = AssetDirectory::new(&bundle_root)?;
        let bundle_id = AssetBundle::metadata(&bundle)
            .map_err(|error| AssetErrorKind::BundleError(Box::new(error)))?
            .bundle_id;

        let mut builder = registry_builder(&root);
        builder.add_bundle(Arc::new(bundle));
        let registry = builder.try_build()?;
        let previous_metadata = registry.index_db().get_bundle(&bundle_id)?;
        drop(registry);

        std::fs::write(bundle_root.join("sample.storetest.tags"), "tags = [")?;
        let mut builder = registry_builder(&root);
        builder.add_bundle(Arc::new(AssetDirectory::new(&bundle_root)?));
        assert!(builder.try_build().is_err());

        let index = AssetIndexDb::connect(root.join("index.sqlite3"))?;
        assert_eq!(
            index.get_bundle(&bundle_id)?.last_modified,
            previous_metadata.last_modified
        );
        drop(index);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    fn registry_builder(root: &Path) -> AssetRegistryBuilder {
        let mut builder = AssetRegistryBuilder::default();
        builder.set_root(root.to_path_buf());
        builder.add_serializer::<TestAssetSerializer>();
        builder
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lapiz-{name}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
}
