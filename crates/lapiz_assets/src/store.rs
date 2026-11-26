use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use dashmap::DashMap;
use indexmap::IndexSet;
use lapiz_runtime::{event::Event as _, global::Global};

use crate::{
    asset::{Asset, AssetHandle, AssetId, AssetMetadata, ErasedAsset as _, UntypedAssetId},
    bundle::{
        AssetBundle, AssetBundleCache, BundleId, BundleManifest, BundleSnapshot, ErasedAssetBundle,
        read_tag_file, read_tagged_assets_file, read_tagged_tags_file, scan_bundle_assets,
    },
    event::{AssetEvent, TagEvent},
    index_db::{AssetFilter, AssetIndexDb, TagFilter, UntypedAssetFilter},
    loader::AssetSerializerRegistry,
    tag::{PositionInTag, Tag, TagFile, TagId, TaggedAssets},
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

    pub fn bundle(&self, id: &BundleId) -> Result<Arc<AssetBundleCache>> {
        self.inner
            .bundles
            .get(id)
            .map(|bundle| bundle.clone())
            .ok_or_else(|| anyhow!("Asset bundle not found for bundle ID: {id}"))
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

        AssetEvent::broadcast(AssetEvent::Added {
            id: asset_id,
            ty: T::TYPE_NAME,
            bundle_id,
        });
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

        let bundle_ids = snapshots
            .iter()
            .map(|snapshot| snapshot.metadata.bundle_id)
            .collect::<Arc<[BundleId]>>();

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

        AssetEvent::broadcast(AssetEvent::BundlesChanged { bundle_ids });
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
        let (tags, tag_children) = scan_tags(
            &self.inner.root,
            bundle.as_ref(),
            &manifest,
            metadata.bundle_id,
        )?;
        let tagged_assets = scan_tagged_assets(
            self.inner.root.as_path(),
            &metadata.bundle_id,
            bundle.as_ref(),
            &manifest,
        )?;

        Ok(BundleSnapshot {
            bundle,
            metadata,
            manifest,
            assets,
            tags,
            tag_children,
            tagged_assets,
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

        Ok(AssetHandle::new(asset_id, bundle, self.clone()))
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

    pub fn write_tag(&self, tag: &Tag) -> Result<()> {
        let bundle = self.inner.bundles.get(&tag.bundle_id).ok_or_else(|| {
            anyhow!(
                "Asset bundle not found for bundle ID: {} (tag ID: {})",
                tag.bundle_id,
                tag.id
            )
        })?;
        self.inner.index_db.update_tag(tag.clone())?;
        bundle.write_tag(tag)?;

        TagEvent::broadcast(TagEvent::Updated { tag_id: tag.id });
        Ok(())
    }

    pub fn create_tag(
        &self,
        bundle_id: BundleId,
        relative_path: String,
        file: TagFile,
    ) -> Result<Tag> {
        let tag = Tag {
            id: file.id,
            bundle_id,
            relative_path,
            name: file.name,
            asset_ty: file.asset_ty,
        };
        let bundle = self.bundle(&bundle_id)?;
        bundle.write_tag(&tag)?;
        self.index_db().add_tag(tag.clone())?;

        TagEvent::broadcast(TagEvent::Created { tag_id: tag.id });
        Ok(tag)
    }

    pub fn remove_asset_from_tag(&self, asset_id: UntypedAssetId, tag_id: TagId) -> Result<()> {
        let tag = self.index_db().get_tag(tag_id)?;
        let bundle = self.bundle(&tag.bundle_id)?;
        let mut members = bundle.read_tagged_assets(&tag_id)?;
        if !members.assets.shift_remove(&asset_id) {
            bail!("Asset {asset_id} is not assigned to tag {}", tag.name);
        }
        bundle.write_tagged_assets(&tag_id, &members)?;
        self.index_db().edit_tagged_assets(&tag_id, &members)?;

        TagEvent::broadcast(TagEvent::AssetsChanged { tag_id });
        Ok(())
    }

    pub fn add_asset_to_tag_at(
        &self,
        asset_id: UntypedAssetId,
        tag_id: TagId,
        position: PositionInTag<UntypedAssetId>,
    ) -> Result<()> {
        let metadata = self.inner.index_db.get_asset(&asset_id)?;
        let tag = self.inner.index_db.get_tag(tag_id)?;
        if let Some(expected) = &tag.asset_ty
            && metadata.ty != *expected
        {
            bail!(
                "Tag {tag_id} requires asset type {expected}, got {}",
                metadata.ty
            );
        }

        let bundle = self.bundle(&tag.bundle_id)?;
        let mut assets = bundle.read_tagged_assets(&tag_id)?;
        assets.insert(asset_id, position)?;
        bundle.write_tagged_assets(&tag_id, &assets)?;
        self.inner.index_db.edit_tagged_assets(&tag_id, &assets)?;

        TagEvent::broadcast(TagEvent::AssetsChanged { tag_id });
        Ok(())
    }

    pub fn sync_type_tag_assets(&self) -> Result<()> {
        let mut changed = false;
        for tag in self.all_tags()?.into_iter().filter(Tag::is_type_tag) {
            let bundle = self.bundle(&tag.bundle_id)?;
            let mut members = bundle.read_tagged_assets(&tag.id)?;
            let before = members.assets.clone();
            for asset in self.index_db().get_assets(UntypedAssetFilter {
                ty: tag.asset_ty.clone(),
                ..Default::default()
            })? {
                members.assets.insert(asset.asset_id);
            }
            if members.assets != before {
                bundle.write_tagged_assets(&tag.id, &members)?;
                self.index_db().edit_tagged_assets(&tag.id, &members)?;
                changed = true;
            }
        }

        if changed {
            TagEvent::broadcast(TagEvent::TypeTagsSynced);
        }
        Ok(())
    }

    pub fn move_tag_at(
        &self,
        child_id: TagId,
        old_parent: &Tag,
        new_parent: &Tag,
        position: PositionInTag<TagId>,
    ) -> Result<()> {
        if old_parent.id == new_parent.id {
            return self.add_tag_to_tag_at(new_parent, &child_id, position);
        }

        let destination_bundle = self.bundle(&new_parent.bundle_id)?;
        let original = destination_bundle.read_tagged_tags(&new_parent.id)?;
        self.add_tag_to_tag_at_inner(new_parent, &child_id, position)?;
        if let Err(error) = self.remove_child_from_tag_inner(old_parent, &child_id) {
            destination_bundle.write_tagged_tags(&new_parent.id, &original)?;
            self.index_db()
                .edit_tagged_tags(&new_parent.id, &original)?;
            return Err(error);
        }

        TagEvent::broadcast(TagEvent::ChildrenChanged {
            parent_id: old_parent.id,
        });
        TagEvent::broadcast(TagEvent::ChildrenChanged {
            parent_id: new_parent.id,
        });
        Ok(())
    }

    pub fn add_tag_to_tag_at(
        &self,
        parent: &Tag,
        child_id: &TagId,
        position: PositionInTag<TagId>,
    ) -> Result<()> {
        self.add_tag_to_tag_at_inner(parent, child_id, position)?;

        TagEvent::broadcast(TagEvent::ChildrenChanged {
            parent_id: parent.id,
        });
        Ok(())
    }

    pub fn remove_child_from_tag(&self, parent: &Tag, child_id: &TagId) -> Result<()> {
        self.remove_child_from_tag_inner(parent, child_id)?;

        TagEvent::broadcast(TagEvent::ChildrenChanged {
            parent_id: parent.id,
        });
        Ok(())
    }

    fn add_tag_to_tag_at_inner(
        &self,
        parent: &Tag,
        child_id: &TagId,
        position: PositionInTag<TagId>,
    ) -> Result<()> {
        let parent = self.inner.index_db.get_tag(parent.id)?;
        self.inner.index_db.get_tag(*child_id)?;
        let bundle = self.bundle(&parent.bundle_id)?;
        let mut children = bundle.read_tagged_tags(&parent.id)?;
        children.insert(*child_id, position)?;
        self.inner
            .index_db
            .validate_tagged_tags(&parent.id, &children)?;

        bundle.write_tagged_tags(&parent.id, &children)?;
        self.inner.index_db.edit_tagged_tags(&parent.id, &children)
    }

    fn remove_child_from_tag_inner(&self, parent: &Tag, child_id: &TagId) -> Result<()> {
        let parent = self.inner.index_db.get_tag(parent.id)?;
        let bundle = self.bundle(&parent.bundle_id)?;
        let mut children = bundle.read_tagged_tags(&parent.id)?;
        if !children.tags.shift_remove(child_id) {
            bail!("Tag {child_id} is not a child of tag {}", parent.id);
        }
        self.inner
            .index_db
            .validate_tagged_tags(&parent.id, &children)?;

        bundle.write_tagged_tags(&parent.id, &children)?;
        self.inner.index_db.edit_tagged_tags(&parent.id, &children)
    }

    pub fn delete_tag(&self, tag_id: &TagId) -> Result<()> {
        self.inner.index_db.delete_tag(tag_id)?;

        TagEvent::broadcast(TagEvent::Deleted { tag_id: *tag_id });
        Ok(())
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
                    self.clone(),
                ))
            })
            .collect()
    }
}

fn scan_tags(
    assets_root: &Path,
    bundle: &dyn ErasedAssetBundle,
    manifest: &BundleManifest,
    bundle_id: BundleId,
) -> Result<(Vec<Tag>, HashMap<TagId, IndexSet<TagId>>)> {
    let mut tags = Vec::new();
    let mut tag_children = HashMap::new();

    for path in manifest.tags.values() {
        let tag = read_tag_file(assets_root, path, &bundle_id, bundle)?;
        let children =
            read_tagged_tags_file(assets_root, path, &bundle_id, bundle)?.unwrap_or_default();
        tag_children.insert(tag.id, children.tags);
        tags.push(Tag {
            id: tag.id,
            bundle_id,
            relative_path: path.to_string_lossy().replace('\\', "/"),
            name: tag.name,
            asset_ty: tag.asset_ty,
        });
    }

    Ok((tags, tag_children))
}

fn scan_tagged_assets(
    assets_root: &Path,
    bundle_id: &BundleId,
    bundle: &dyn ErasedAssetBundle,
    manifest: &BundleManifest,
) -> Result<HashMap<TagId, TaggedAssets>> {
    manifest
        .tags
        .iter()
        .map(|(tag_id, path)| {
            let assets = read_tagged_assets_file(assets_root, path, bundle_id, bundle)?;
            Ok((*tag_id, assets.unwrap_or_default()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        io::{self, Read, Write},
    };

    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use crate::{
        asset::UntypedAssetId,
        bundle::directory::AssetDirectory,
        loader::AssetSerializer,
        store::*,
        tag::{PositionInTag, TagFile, TaggedTags},
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

        fn read(&self, _relative_path: &Path, reader: &mut dyn Read) -> Result<Self::Asset> {
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
    fn tagged_assets_are_authoritative_across_bundles_and_index_rebuilds() -> Result<()> {
        for reverse in [false, true] {
            let root = temp_root("tagged-assets");
            let assets_root = root.join("assets");
            let tags_root = root.join("tags");
            fs::create_dir_all(&assets_root)?;
            fs::create_dir_all(&tags_root)?;
            for name in ["a", "b", "c"] {
                fs::write(assets_root.join(format!("{name}.storetest")), "42")?;
            }
            let tag = TagFile::new("Test".into(), Some(TestAsset::TYPE_NAME.into()));
            fs::write(tags_root.join("test.tag"), toml::to_string(&tag)?)?;
            let assets_bundle = AssetDirectory::new(&assets_root)?;
            let manifest = AssetBundle::manifest(&assets_bundle)?;
            let id = |name: &str| {
                *manifest
                    .assets
                    .iter()
                    .find(|(_, path)| **path == Path::new(name))
                    .unwrap()
                    .0
            };
            let [a, b, c] = [id("a.storetest"), id("b.storetest"), id("c.storetest")];
            let missing = UntypedAssetId::new(Uuid::new_v4());
            fs::write(
                tags_root.join("test.tagged_assets"),
                toml::to_string(&TaggedAssets {
                    assets: IndexSet::from([c, missing, a, b]),
                })?,
            )?;
            let mut bundles = vec![
                Arc::new(assets_bundle) as Arc<dyn ErasedAssetBundle>,
                Arc::new(AssetDirectory::new(&tags_root)?),
            ];
            if reverse {
                bundles.reverse();
            }
            let registry = registry_with_bundles(&root, bundles)?;
            assert_eq!(tag_asset_ids(&registry, tag.id)?, vec![c, a, b]);
            let handle = registry.handle(a.into_typed::<TestAsset>())?;
            assert_eq!(
                handle.read_tags()?.into_iter().collect::<Vec<_>>(),
                vec![tag.id]
            );
            handle.add_to_tag_at(&tag.id, PositionInTag::First)?;
            assert_eq!(tag_asset_ids(&registry, tag.id)?, vec![a, c, b]);
            handle.add_to_tag_at(&tag.id, PositionInTag::After(b))?;
            assert_eq!(tag_asset_ids(&registry, tag.id)?, vec![c, b, a]);
            let mut updated = registry.index_db().get_tag(tag.id)?;
            updated.name = "Renamed".into();
            registry.write_tag(&updated)?;
            assert_eq!(registry.index_db().get_tag(tag.id)?.name, "Renamed");
            handle.remove_tag(&tag.id)?;
            assert!(handle.read_tags()?.is_empty());
            assert!(handle.remove_tag(&tag.id).is_err());
            handle.add_to_tag_at(&tag.id, PositionInTag::default())?;
            let persisted = toml::from_str::<TaggedAssets>(&fs::read_to_string(
                tags_root.join("test.tagged_assets"),
            )?)?;
            assert_eq!(
                persisted.assets.into_iter().collect::<Vec<_>>(),
                vec![c, missing, b, a]
            );
            assert!(!assets_root.join("a.storetest.tags").exists());
            drop(handle);
            drop(registry);
            fs::remove_file(root.join("index.sqlite3"))?;
            let registry = registry_with_bundles(
                &root,
                vec![
                    Arc::new(AssetDirectory::new(&tags_root)?),
                    Arc::new(AssetDirectory::new(&assets_root)?),
                ],
            )?;
            assert_eq!(tag_asset_ids(&registry, tag.id)?, vec![c, b, a]);

            fs::write(tags_root.join("test.tagged_assets"), "assets = []")?;
            registry.add_bundle(AssetDirectory::new(&tags_root)?)?;
            assert!(tag_asset_ids(&registry, tag.id)?.is_empty());
            fs::remove_file(tags_root.join("test.tagged_assets"))?;
            registry.add_bundle(AssetDirectory::new(&tags_root)?)?;
            assert!(tag_asset_ids(&registry, tag.id)?.is_empty());
            drop(registry);
            fs::remove_dir_all(root)?;
        }
        Ok(())
    }

    #[test]
    fn tagged_asset_mutations_preserve_data_on_errors() -> Result<()> {
        let root = temp_root("tagged-assets-errors");
        let bundle_root = root.join("bundle");
        fs::create_dir_all(&bundle_root)?;
        let tag = TagFile::new("Typed".into(), Some(TestAsset::TYPE_NAME.into()));
        fs::write(bundle_root.join("test.tag"), toml::to_string(&tag)?)?;
        let bundle = AssetDirectory::new(&bundle_root)?;
        let bundle_id = AssetBundle::metadata(&bundle)?.bundle_id;
        let registry = registry_with_bundles(&root, vec![Arc::new(bundle)])?;
        let a = registry
            .add_asset(bundle_id, "a.storetest", Arc::new(TestAsset { value: 1 }))?
            .into_untyped();
        let b = registry
            .add_asset(bundle_id, "b.storetest", Arc::new(TestAsset { value: 2 }))?
            .into_untyped();
        let a_handle = registry.handle(a.into_typed::<TestAsset>())?;
        let b_handle = registry.handle(b.into_typed::<TestAsset>())?;
        a_handle.add_to_tag_at(&tag.id, PositionInTag::Last)?;
        b_handle.add_to_tag_at(&tag.id, PositionInTag::First)?;
        let sidecar = bundle_root.join("test.tagged_assets");
        let original = fs::read_to_string(&sidecar)?;
        let missing = UntypedAssetId::new(Uuid::new_v4());
        for (asset, tag_id, position) in [
            (a, tag.id, PositionInTag::Before(missing)),
            (a, tag.id, PositionInTag::After(missing)),
            (missing, tag.id, PositionInTag::Last),
            (a, TagId::new(Uuid::new_v4()), PositionInTag::Last),
        ] {
            assert!(
                registry
                    .handle(asset.into_typed::<TestAsset>())
                    .and_then(|handle| handle.add_to_tag_at(&tag_id, position))
                    .is_err()
            );
            assert_eq!(fs::read_to_string(&sidecar)?, original);
            assert_eq!(tag_asset_ids(&registry, tag.id)?, vec![b, a]);
        }

        let mut metadata = registry.index_db().get_tag(tag.id)?;
        metadata.asset_ty = Some("incompatible_test_asset".into());
        registry.write_tag(&metadata)?;
        assert!(
            a_handle
                .add_to_tag_at(&tag.id, PositionInTag::First)
                .is_err()
        );
        assert_eq!(fs::read_to_string(&sidecar)?, original);
        a_handle.remove_tag(&tag.id)?;
        assert_eq!(tag_asset_ids(&registry, tag.id)?, vec![b]);
        metadata.asset_ty = Some(TestAsset::TYPE_NAME.into());
        registry.write_tag(&metadata)?;

        fs::write(&sidecar, "assets = [")?;
        assert!(
            a_handle
                .add_to_tag_at(&tag.id, PositionInTag::Last)
                .is_err()
        );
        assert_eq!(fs::read_to_string(&sidecar)?, "assets = [");
        assert_eq!(tag_asset_ids(&registry, tag.id)?, vec![b]);

        drop(a_handle);
        drop(b_handle);
        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn tag_asset_ids(registry: &AssetRegistry, tag: TagId) -> Result<Vec<UntypedAssetId>> {
        Ok(registry
            .index_db()
            .get_assets(UntypedAssetFilter {
                tag: Some(tag),
                ..Default::default()
            })?
            .into_iter()
            .map(|asset| asset.asset_id)
            .collect())
    }

    #[test]
    fn tag_children_sync_and_persist_across_bundles() -> Result<()> {
        for reverse_order in [false, true] {
            let root = temp_root("tag-children-sync");
            let parents_root = root.join("parents");
            let child_root = root.join("child");
            fs::create_dir_all(&parents_root)?;
            fs::create_dir_all(&child_root)?;

            let child = TagFile::new("Child".to_string(), None);
            let missing_child = TagId::new(Uuid::new_v4());
            let parent = TagFile::new("Parent".to_string(), None);
            let other_parent = TagFile::new("Other parent".to_string(), None);
            let members = TaggedTags {
                tags: IndexSet::from([child.id, missing_child]),
            };
            fs::write(
                parents_root.join("parent.tagged_tags"),
                toml::to_string(&members)?,
            )?;
            fs::write(
                parents_root.join("other.tagged_tags"),
                toml::to_string(&TaggedTags {
                    tags: IndexSet::from([child.id]),
                })?,
            )?;
            fs::write(parents_root.join("parent.tag"), toml::to_string(&parent)?)?;
            fs::write(
                parents_root.join("other.tag"),
                toml::to_string(&other_parent)?,
            )?;
            fs::write(child_root.join("child.tag"), toml::to_string(&child)?)?;

            let mut bundles = vec![
                Arc::new(AssetDirectory::new(&parents_root)?) as Arc<dyn ErasedAssetBundle>,
                Arc::new(AssetDirectory::new(&child_root)?) as Arc<dyn ErasedAssetBundle>,
            ];
            if reverse_order {
                bundles.reverse();
            }
            let registry = registry_with_bundles(&root, bundles)?;
            assert_eq!(
                registry.index_db().get_child_tags(&parent.id)?[0].id,
                child.id
            );
            assert_eq!(
                registry.index_db().get_child_tags(&other_parent.id)?[0].id,
                child.id
            );
            assert!(registry.index_db().get_child_tags(&child.id)?.is_empty());

            registry.add_bundle(AssetDirectory::new(&child_root)?)?;
            assert_eq!(
                registry.index_db().get_child_tags(&other_parent.id)?[0].id,
                child.id
            );

            let mut updated_parent = registry.index_db().get_tag(parent.id)?;
            updated_parent.name = "Updated parent".to_string();
            registry.write_tag(&updated_parent)?;
            let written =
                toml::from_str::<TagFile>(&fs::read_to_string(parents_root.join("parent.tag"))?)?;
            assert_eq!(written.name, updated_parent.name);
            assert_eq!(
                fs::read_to_string(parents_root.join("parent.tagged_tags"))?,
                toml::to_string(&members)?
            );

            registry.remove_child_from_tag(&updated_parent, &child.id)?;
            registry.write_tag(&updated_parent)?;
            registry.add_bundle(AssetDirectory::new(&parents_root)?)?;
            assert!(registry.index_db().get_child_tags(&parent.id)?.is_empty());
            assert_eq!(
                registry.index_db().get_child_tags(&other_parent.id)?[0].id,
                child.id
            );

            drop(registry);
            fs::remove_file(root.join("index.sqlite3"))?;
            let registry = registry_with_bundles(
                &root,
                vec![
                    Arc::new(AssetDirectory::new(&parents_root)?),
                    Arc::new(AssetDirectory::new(&child_root)?),
                ],
            )?;
            assert!(registry.index_db().get_child_tags(&parent.id)?.is_empty());
            assert_eq!(
                registry.index_db().get_child_tags(&other_parent.id)?[0].id,
                child.id
            );
            let persisted = toml::from_str::<TaggedTags>(&fs::read_to_string(
                parents_root.join("parent.tagged_tags"),
            )?)?;
            assert_eq!(persisted.tags, IndexSet::from([missing_child]));
            fs::remove_file(parents_root.join("other.tagged_tags"))?;
            registry.add_bundle(AssetDirectory::new(&parents_root)?)?;
            assert!(
                registry
                    .index_db()
                    .get_child_tags(&other_parent.id)?
                    .is_empty()
            );

            drop(registry);
            fs::remove_dir_all(root)?;
        }
        Ok(())
    }

    #[test]
    fn tagged_tags_insert_and_move_without_rewriting_tag_files() -> Result<()> {
        let root = temp_root("add-child-to-tag");
        let bundle_root = root.join("tags");
        fs::create_dir_all(&bundle_root)?;
        let existing_child = TagFile::new("Existing child".to_string(), None);
        let child = TagFile::new("Child".to_string(), None);
        let parent = TagFile::new("Parent".to_string(), None);
        fs::write(
            bundle_root.join("parent.tagged_tags"),
            toml::to_string(&TaggedTags {
                tags: IndexSet::from([existing_child.id]),
            })?,
        )?;
        for (path, tag) in [
            ("parent.tag", &parent),
            ("existing.tag", &existing_child),
            ("child.tag", &child),
        ] {
            fs::write(bundle_root.join(path), toml::to_string(tag)?)?;
        }
        let child_content = fs::read_to_string(bundle_root.join("child.tag"))?;
        let registry =
            registry_with_bundles(&root, vec![Arc::new(AssetDirectory::new(&bundle_root)?)])?;
        let parent = registry.index_db().get_tag(parent.id)?;
        let child = registry.index_db().get_tag(child.id)?;

        let parent_content = fs::read_to_string(bundle_root.join("parent.tag"))?;
        registry.add_tag_to_tag_at(&parent, &child.id, PositionInTag::First)?;
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[0].id,
            child.id
        );
        registry.add_tag_to_tag_at(&parent, &child.id, PositionInTag::After(existing_child.id))?;
        assert!(
            registry
                .add_tag_to_tag_at(&child, &parent.id, PositionInTag::Last)
                .is_err()
        );
        assert!(
            registry
                .add_tag_to_tag_at(&parent, &parent.id, PositionInTag::Last)
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(bundle_root.join("parent.tag"))?,
            parent_content
        );

        let children = IndexSet::from([existing_child.id, child.id]);
        assert_eq!(
            registry
                .index_db()
                .get_child_tags(&parent.id)?
                .into_iter()
                .map(|tag| tag.id)
                .collect::<Vec<_>>(),
            children.iter().copied().collect::<Vec<_>>()
        );
        let written = toml::from_str::<TaggedTags>(&fs::read_to_string(
            bundle_root.join("parent.tagged_tags"),
        )?)?;
        assert_eq!(
            written.tags.into_iter().collect::<Vec<_>>(),
            children.into_iter().collect::<Vec<_>>()
        );
        assert_eq!(
            fs::read_to_string(bundle_root.join("child.tag"))?,
            child_content
        );

        registry.add_bundle(AssetDirectory::new(&bundle_root)?)?;
        assert_eq!(registry.index_db().get_child_tags(&parent.id)?.len(), 2);
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[1].id,
            child.id
        );

        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn tagged_tags_removal_persists_empty_sidecar() -> Result<()> {
        let root = temp_root("remove-child-from-tag");
        let bundle_root = root.join("tags");
        fs::create_dir_all(&bundle_root)?;
        let retained_child = TagFile::new("Retained child".to_string(), None);
        let child = TagFile::new("Child".to_string(), None);
        let parent = TagFile::new("Parent".to_string(), None);
        fs::write(
            bundle_root.join("parent.tagged_tags"),
            toml::to_string(&TaggedTags {
                tags: IndexSet::from([retained_child.id, child.id]),
            })?,
        )?;
        for (path, tag) in [
            ("parent.tag", &parent),
            ("retained.tag", &retained_child),
            ("child.tag", &child),
        ] {
            fs::write(bundle_root.join(path), toml::to_string(tag)?)?;
        }
        let child_content = fs::read_to_string(bundle_root.join("child.tag"))?;
        let registry =
            registry_with_bundles(&root, vec![Arc::new(AssetDirectory::new(&bundle_root)?)])?;
        let parent = registry.index_db().get_tag(parent.id)?;

        registry.remove_child_from_tag(&parent, &child.id)?;
        assert!(registry.remove_child_from_tag(&parent, &child.id).is_err());
        assert_eq!(
            registry
                .index_db()
                .get_child_tags(&parent.id)?
                .into_iter()
                .map(|tag| tag.id)
                .collect::<Vec<_>>(),
            vec![retained_child.id]
        );
        let written = toml::from_str::<TaggedTags>(&fs::read_to_string(
            bundle_root.join("parent.tagged_tags"),
        )?)?;
        assert_eq!(
            written.tags.into_iter().collect::<Vec<_>>(),
            vec![retained_child.id]
        );
        assert_eq!(
            fs::read_to_string(bundle_root.join("child.tag"))?,
            child_content
        );

        registry.add_bundle(AssetDirectory::new(&bundle_root)?)?;
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[0].id,
            retained_child.id
        );
        registry.remove_child_from_tag(&parent, &retained_child.id)?;
        let written = toml::from_str::<TaggedTags>(&fs::read_to_string(
            bundle_root.join("parent.tagged_tags"),
        )?)?;
        assert!(written.tags.is_empty());
        registry.add_bundle(AssetDirectory::new(&bundle_root)?)?;
        assert!(registry.index_db().get_child_tags(&parent.id)?.is_empty());

        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn tagged_tag_errors_preserve_sidecar_and_index() -> Result<()> {
        let root = temp_root("tagged-tags-errors");
        let bundle_root = root.join("tags");
        fs::create_dir_all(&bundle_root)?;
        let parent = TagFile::new("Parent".into(), None);
        let child = TagFile::new("Child".into(), None);
        let other = TagFile::new("Other".into(), None);
        for (path, tag) in [
            ("parent.tag", &parent),
            ("child.tag", &child),
            ("other.tag", &other),
        ] {
            fs::write(bundle_root.join(path), toml::to_string(tag)?)?;
        }
        let registry =
            registry_with_bundles(&root, vec![Arc::new(AssetDirectory::new(&bundle_root)?)])?;
        let parent = registry.index_db().get_tag(parent.id)?;
        let child = registry.index_db().get_tag(child.id)?;
        registry.add_tag_to_tag_at(&parent, &child.id, PositionInTag::Last)?;
        let sidecar = bundle_root.join("parent.tagged_tags");
        let original = fs::read_to_string(&sidecar)?;
        let missing = TagId::new(Uuid::new_v4());
        for (id, position) in [
            (parent.id, PositionInTag::Last),
            (missing, PositionInTag::Last),
            (other.id, PositionInTag::Before(missing)),
            (other.id, PositionInTag::After(missing)),
        ] {
            assert!(registry.add_tag_to_tag_at(&parent, &id, position).is_err());
            assert_eq!(fs::read_to_string(&sidecar)?, original);
            assert_eq!(
                registry.index_db().get_child_tags(&parent.id)?[0].id,
                child.id
            );
        }
        assert!(
            registry
                .add_tag_to_tag_at(&child, &parent.id, PositionInTag::Last)
                .is_err()
        );
        assert!(!bundle_root.join("child.tagged_tags").exists());
        assert!(registry.remove_child_from_tag(&parent, &other.id).is_err());
        registry.delete_tag(&other.id)?;
        assert!(
            registry
                .add_tag_to_tag_at(&parent, &other.id, PositionInTag::Last)
                .is_err()
        );
        assert_eq!(fs::read_to_string(&sidecar)?, original);

        fs::write(&sidecar, "tags = [")?;
        assert!(
            registry
                .add_tag_to_tag_at(&parent, &child.id, PositionInTag::First)
                .is_err()
        );
        assert!(registry.remove_child_from_tag(&parent, &child.id).is_err());
        assert_eq!(fs::read_to_string(&sidecar)?, "tags = [");
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[0].id,
            child.id
        );
        assert!(
            registry
                .add_bundle(AssetDirectory::new(&bundle_root)?)
                .is_err()
        );
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[0].id,
            child.id
        );

        fs::remove_file(&sidecar)?;
        fs::create_dir(&sidecar)?;
        assert!(
            registry
                .add_tag_to_tag_at(&parent, &child.id, PositionInTag::Last)
                .is_err()
        );
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[0].id,
            child.id
        );
        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn readonly_tagged_tags_survive_index_rebuilds_without_changing_archive() -> Result<()> {
        use std::{collections::BTreeMap, fs::File};

        use zip::{ZipWriter, write::SimpleFileOptions};

        use crate::bundle::standard::{StandardAssetBundle, StandardAssetBundleMetadata};

        let root = temp_root("readonly-tagged-tags");
        let archive_path = root.join("tags.lazurite");
        let parent = TagFile::new("Parent".into(), None);
        let child = TagFile::new("Child".into(), None);
        let bundle_id = BundleId::new(Uuid::new_v4());
        let manifest = BundleManifest {
            assets: BTreeMap::new(),
            tags: BTreeMap::from([
                (parent.id, "tags/parent.tag".into()),
                (child.id, "tags/child.tag".into()),
            ]),
        };
        let mut archive = ZipWriter::new(File::create(&archive_path)?);
        for (path, content) in [
            (
                "metadata.toml",
                toml::to_string(&StandardAssetBundleMetadata {
                    bundle_id,
                    name: "Tags".into(),
                })?,
            ),
            ("manifest.toml", toml::to_string(&manifest)?),
            ("tags/parent.tag", toml::to_string(&parent)?),
            ("tags/child.tag", toml::to_string(&child)?),
            (
                "tags/parent.tagged_tags",
                toml::to_string(&TaggedTags {
                    tags: IndexSet::from([child.id]),
                })?,
            ),
        ] {
            archive.start_file(path, SimpleFileOptions::default())?;
            archive.write_all(content.as_bytes())?;
        }
        archive.finish()?;
        let original = fs::read(&archive_path)?;
        let registry = registry_with_bundles(
            &root,
            vec![Arc::new(StandardAssetBundle::new(&archive_path)?)],
        )?;
        let parent = registry.index_db().get_tag(parent.id)?;
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[0].id,
            child.id
        );

        let sidecar = registry
            .bundle(&bundle_id)?
            .absolute_modified_path("tags/parent.tagged_tags");
        fs::create_dir_all(sidecar.parent().unwrap())?;
        fs::write(&sidecar, "tags = [")?;
        assert!(registry.remove_child_from_tag(&parent, &child.id).is_err());
        fs::remove_file(&sidecar)?;
        // A blocked override directory must not change the indexed hierarchy.
        fs::remove_dir(sidecar.parent().unwrap())?;
        fs::write(sidecar.parent().unwrap(), "blocked")?;
        assert!(registry.remove_child_from_tag(&parent, &child.id).is_err());
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[0].id,
            child.id
        );
        fs::remove_file(sidecar.parent().unwrap())?;

        registry.remove_child_from_tag(&parent, &child.id)?;
        assert!(
            toml::from_str::<TaggedTags>(&fs::read_to_string(&sidecar)?)?
                .tags
                .is_empty()
        );
        assert_eq!(fs::read(&archive_path)?, original);
        drop(registry);
        fs::remove_file(root.join("index.sqlite3"))?;
        let registry = registry_with_bundles(
            &root,
            vec![Arc::new(StandardAssetBundle::new(&archive_path)?)],
        )?;
        assert!(registry.index_db().get_child_tags(&parent.id)?.is_empty());
        registry.add_tag_to_tag_at(&parent, &child.id, PositionInTag::First)?;
        drop(registry);
        fs::remove_file(root.join("index.sqlite3"))?;
        let registry = registry_with_bundles(
            &root,
            vec![Arc::new(StandardAssetBundle::new(&archive_path)?)],
        )?;
        assert_eq!(
            registry.index_db().get_child_tags(&parent.id)?[0].id,
            child.id
        );
        assert_eq!(fs::read(&archive_path)?, original);
        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn tag_children_without_sidecar_are_empty() -> Result<()> {
        let root = temp_root("tagged-tags-no-sidecar");
        let bundle_root = root.join("tags");
        fs::create_dir_all(&bundle_root)?;
        let parent = TagFile::new("Parent".into(), None);
        let child = TagFile::new("Child".into(), None);
        let mut content = toml::Value::try_from(&parent)?;
        content
            .as_table_mut()
            .unwrap()
            .insert("children".into(), toml::Value::try_from(vec![child.id])?);
        fs::write(bundle_root.join("parent.tag"), toml::to_string(&content)?)?;
        fs::write(bundle_root.join("child.tag"), toml::to_string(&child)?)?;
        let registry =
            registry_with_bundles(&root, vec![Arc::new(AssetDirectory::new(&bundle_root)?)])?;
        assert!(registry.index_db().get_child_tags(&parent.id)?.is_empty());
        assert!(!bundle_root.join("parent.tagged_tags").exists());
        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn tag_children_sync_rejects_cycles() -> Result<()> {
        let root = temp_root("tag-children-cycle");
        let bundle_root = root.join("tags");
        fs::create_dir_all(&bundle_root)?;
        let parent = TagFile::new("Parent".to_string(), None);
        let child = TagFile::new("Child".to_string(), None);
        fs::write(
            bundle_root.join("parent.tagged_tags"),
            toml::to_string(&TaggedTags {
                tags: IndexSet::from([child.id]),
            })?,
        )?;
        fs::write(
            bundle_root.join("child.tagged_tags"),
            toml::to_string(&TaggedTags {
                tags: IndexSet::from([parent.id]),
            })?,
        )?;
        fs::write(bundle_root.join("parent.tag"), toml::to_string(&parent)?)?;
        fs::write(bundle_root.join("child.tag"), toml::to_string(&child)?)?;

        let registry = registry_with_bundles(&root, vec![])?;
        assert!(
            registry
                .add_bundle(AssetDirectory::new(&bundle_root)?)
                .is_err()
        );
        assert!(registry.all_tags()?.is_empty());

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
        let tag = TagFile::new("Test".into(), None);
        fs::write(bundle_root.join("test.tag"), toml::to_string(&tag)?)?;
        let bundle = AssetDirectory::new(&bundle_root)?;
        let bundle_id = AssetBundle::metadata(&bundle)?.bundle_id;

        let registry = registry_with_bundles(&root, vec![Arc::new(bundle)])?;
        let previous_metadata = registry.index_db().get_bundle(&bundle_id)?;
        drop(registry);

        fs::write(bundle_root.join("test.tagged_assets"), "assets = [")?;
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

    #[test]
    fn asset_lifecycle_broadcasts_events() -> Result<()> {
        let root = temp_root("asset-events");
        let bundle_root = root.join("bundle");
        fs::create_dir_all(&bundle_root)?;
        let bundle = AssetDirectory::new(&bundle_root)?;
        let bundle_id = AssetBundle::metadata(&bundle)?.bundle_id;

        let mut rx = AssetEvent::channel().1.activate_cloned();
        macro_rules! drain_for {
            ($target_bundle:expr, $target_asset:expr) => {{
                let mut matched = Vec::new();
                while let Ok(event) = rx.try_recv() {
                    let belongs = match &event {
                        AssetEvent::BundlesChanged { bundle_ids } => {
                            bundle_ids.contains(&$target_bundle)
                        }
                        AssetEvent::Added { id, bundle_id, .. } => {
                            *bundle_id == $target_bundle || $target_asset == Some(*id)
                        }
                        AssetEvent::Modified { id, .. } | AssetEvent::Removed { id, .. } => {
                            $target_asset == Some(*id)
                        }
                    };
                    if belongs {
                        matched.push(event);
                    }
                }
                matched
            }};
        }

        let registry = registry_with_bundles(&root, vec![Arc::new(bundle)])?;
        let bundle_events = drain_for!(bundle_id, None);
        assert_eq!(
            bundle_events,
            vec![AssetEvent::BundlesChanged {
                bundle_ids: Arc::from([bundle_id]),
            }]
        );
        assert!(bundle_events[0].matches_type(TestAsset::TYPE_NAME));
        assert!(bundle_events[0].matches_type("other_asset"));

        let asset_id = registry
            .add_asset(
                bundle_id,
                "item.storetest",
                Arc::new(TestAsset { value: 1 }),
            )?
            .into_untyped();
        let added = drain_for!(bundle_id, Some(asset_id));
        assert_eq!(
            added,
            vec![AssetEvent::Added {
                id: asset_id,
                ty: TestAsset::TYPE_NAME,
                bundle_id,
            }]
        );
        assert!(added[0].matches_type(TestAsset::TYPE_NAME));
        assert!(!added[0].matches_type("other_asset"));

        let handle = registry.handle(asset_id.into_typed::<TestAsset>())?;
        handle.update(TestAsset { value: 2 })?;
        handle.write()?;
        let modified = drain_for!(bundle_id, Some(asset_id));
        assert_eq!(
            modified,
            vec![AssetEvent::Modified {
                id: asset_id,
                ty: TestAsset::TYPE_NAME,
                revision: 1,
            }]
        );

        handle.delete()?;
        let removed = drain_for!(bundle_id, Some(asset_id));
        assert_eq!(
            removed,
            vec![AssetEvent::Removed {
                id: asset_id,
                ty: TestAsset::TYPE_NAME,
            }]
        );

        drop(handle);
        drop(registry);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn tag_mutations_broadcast_events() -> Result<()> {
        let root = temp_root("tag-events");
        let bundle_root = root.join("bundle");
        fs::create_dir_all(&bundle_root)?;
        let bundle = AssetDirectory::new(&bundle_root)?;
        let bundle_id = AssetBundle::metadata(&bundle)?.bundle_id;
        let registry = registry_with_bundles(&root, vec![Arc::new(bundle)])?;

        let asset_id = registry
            .add_asset(
                bundle_id,
                "item.storetest",
                Arc::new(TestAsset { value: 1 }),
            )?
            .into_untyped();

        let mut rx = TagEvent::channel().1.activate_cloned();
        macro_rules! drain_for {
            ($ids:expr) => {{
                let mut matched = Vec::new();
                while let Ok(event) = rx.try_recv() {
                    let belongs = match event {
                        TagEvent::Created { tag_id }
                        | TagEvent::Updated { tag_id }
                        | TagEvent::Deleted { tag_id }
                        | TagEvent::AssetsChanged { tag_id } => $ids.contains(&tag_id),
                        TagEvent::ChildrenChanged { parent_id } => $ids.contains(&parent_id),
                        TagEvent::TypeTagsSynced => false,
                    };
                    if belongs {
                        matched.push(event);
                    }
                }
                matched
            }};
        }

        let parent = registry.create_tag(
            bundle_id,
            "parent.tag".into(),
            TagFile::new("Parent".into(), Some(TestAsset::TYPE_NAME.into())),
        )?;
        let other_parent = registry.create_tag(
            bundle_id,
            "other.tag".into(),
            TagFile::new("Other".into(), Some(TestAsset::TYPE_NAME.into())),
        )?;
        let child = registry.create_tag(
            bundle_id,
            "child.tag".into(),
            TagFile::new("Child".into(), Some(TestAsset::TYPE_NAME.into())),
        )?;
        let ids = [parent.id, other_parent.id, child.id];
        assert_eq!(
            drain_for!(&ids),
            vec![
                TagEvent::Created { tag_id: parent.id },
                TagEvent::Created {
                    tag_id: other_parent.id
                },
                TagEvent::Created { tag_id: child.id },
            ]
        );

        let mut renamed = child.clone();
        renamed.name = "Child Renamed".into();
        registry.write_tag(&renamed)?;
        registry.add_asset_to_tag_at(asset_id, child.id, PositionInTag::Last)?;
        registry.remove_asset_from_tag(asset_id, child.id)?;
        registry.add_tag_to_tag_at(&parent, &child.id, PositionInTag::Last)?;
        registry.move_tag_at(child.id, &parent, &other_parent, PositionInTag::Last)?;
        registry.remove_child_from_tag(&other_parent, &child.id)?;
        registry.delete_tag(&child.id)?;

        assert_eq!(
            drain_for!(&ids),
            vec![
                TagEvent::Updated { tag_id: child.id },
                TagEvent::AssetsChanged { tag_id: child.id },
                TagEvent::AssetsChanged { tag_id: child.id },
                TagEvent::ChildrenChanged {
                    parent_id: parent.id
                },
                TagEvent::ChildrenChanged {
                    parent_id: parent.id
                },
                TagEvent::ChildrenChanged {
                    parent_id: other_parent.id
                },
                TagEvent::ChildrenChanged {
                    parent_id: other_parent.id
                },
                TagEvent::Deleted { tag_id: child.id },
            ]
        );

        drop(registry);
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
