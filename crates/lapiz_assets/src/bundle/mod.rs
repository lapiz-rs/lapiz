use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    ffi::OsStr,
    fs::{File, create_dir_all, metadata},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{DateTime, Utc};
use lapiz_utils::wrapper;
use parking_lot::RwLock;
use parse_display::Display;
use path_clean::PathClean;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    asset::{AssetMetadata, ErasedAsset, UntypedAssetId},
    error::{AssetErrorKind, AssetResult},
    loader::{AssetSerializerRegistry, ErasedAssetSerializer},
    tag::{ASSET_TAGS_EXT, AssetTags, TAG_EXT, Tag, TagFile, TagId},
};

pub mod directory;
pub mod standard;

wrapper! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, Serialize, Deserialize)]
    #[display("{0}")]
    pub BundleId: Uuid
}

impl rusqlite::types::FromSql for BundleId {
    fn column_result(value: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
        Ok(Self(Uuid::column_result(value)?))
    }
}

impl rusqlite::types::ToSql for BundleId {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetBundleMetadata {
    pub bundle_id: BundleId,
    pub name: String,
    pub last_modified: DateTime<Utc>,
}

pub(crate) struct BundleSnapshot {
    pub bundle: Arc<dyn ErasedAssetBundle>,
    pub metadata: AssetBundleMetadata,
    pub manifest: BundleManifest,
    pub assets: Vec<AssetMetadata>,
    pub tags: Vec<Tag>,
    pub asset_tags: HashMap<UntypedAssetId, AssetTags>,
}

pub struct AssetBundleCache {
    assets_root: PathBuf,
    metadata: AssetBundleMetadata,
    bundle: Arc<dyn ErasedAssetBundle>,

    manifest: RwLock<BundleManifest>,
    assets: RwLock<HashMap<UntypedAssetId, Arc<dyn ErasedAsset>>>,

    serializers: Arc<AssetSerializerRegistry>,
}

impl AssetBundleCache {
    pub(crate) fn new(
        assets_root: PathBuf,
        snapshot: BundleSnapshot,
        serializers: Arc<AssetSerializerRegistry>,
    ) -> Self {
        Self {
            assets_root,
            metadata: snapshot.metadata,
            bundle: snapshot.bundle,
            manifest: RwLock::new(snapshot.manifest),
            assets: Default::default(),
            serializers,
        }
    }

    pub fn get_cached_asset(&self, id: &UntypedAssetId) -> AssetResult<Arc<dyn ErasedAsset>> {
        let assets = self.assets.read();
        Ok(assets
            .get(id)
            .cloned()
            .ok_or_else(|| AssetErrorKind::AssetNotFound(*id))?)
    }

    pub fn delete_cached_asset(&self, id: &UntypedAssetId) -> AssetResult<()> {
        let mut assets = self.assets.write();
        assets.remove(id);
        Ok(())
    }

    pub fn update_asset(&self, id: UntypedAssetId, asset: Arc<dyn ErasedAsset>) -> AssetResult<()> {
        self.assets.write().insert(id, asset);
        Ok(())
    }

    pub fn write_asset(&self, id: &UntypedAssetId, revision: u32) -> AssetResult<PathBuf> {
        let assets = self.assets.read();
        let asset = assets
            .get(id)
            .ok_or_else(|| AssetErrorKind::AssetNotFound(*id))?;
        let manifest = self.manifest.read();
        let path = manifest
            .assets
            .get(id)
            .ok_or_else(|| AssetErrorKind::AssetPathNotFound(*id))?;
        let serializer = self.serializers.get_for_path(path)?;

        if !self.bundle.is_readonly() && revision == 0 {
            self.bundle
                .add(path, asset.as_ref(), serializer.as_ref())
                .map_err(AssetErrorKind::BundleError)?;
            Ok(path.clone())
        } else {
            write_modified_asset(
                &self.assets_root,
                path,
                &self.metadata.bundle_id,
                asset.as_ref(),
                revision,
                serializer.as_ref(),
            )
        }
    }

    pub fn is_readonly(&self) -> bool {
        self.bundle.is_readonly()
    }

    pub fn add_asset(
        &self,
        path: impl AsRef<Path>,
        asset: Arc<dyn ErasedAsset>,
    ) -> AssetResult<UntypedAssetId> {
        let path = path.as_ref().clean();
        let serializer = self.serializers.get_for_path(&path)?;
        let id = self
            .bundle
            .add(&path, asset.as_ref(), serializer.as_ref())
            .map_err(AssetErrorKind::BundleError)?;

        self.manifest.write().assets.insert(id, path.clone());
        self.assets.write().insert(id, asset);

        Ok(id)
    }

    pub fn metadata(&self) -> &AssetBundleMetadata {
        &self.metadata
    }

    pub fn read_asset_tags(&self, id: &UntypedAssetId) -> AssetResult<AssetTags> {
        let manifest = self.manifest.read();
        let asset_path = manifest
            .assets
            .get(id)
            .cloned()
            .ok_or_else(|| AssetErrorKind::AssetPathNotFound(*id))?;
        let tags = read_asset_tags_file(
            self.assets_root.as_path(),
            &asset_path,
            &self.metadata.bundle_id,
            self.bundle.as_ref(),
        )?
        .unwrap_or_default();
        Ok(tags)
    }

    pub fn write_asset_tags(&self, id: &UntypedAssetId, tags: &[TagId]) -> AssetResult<()> {
        let manifest = self.manifest.read();
        let path = manifest
            .assets
            .get(id)
            .ok_or_else(|| AssetErrorKind::AssetPathNotFound(*id))?;
        let tags = AssetTags {
            tags: tags.iter().cloned().collect(),
        };

        if self.bundle.is_readonly() {
            let modified_path =
                modified_bundle_absolute_path(&self.assets_root, &self.metadata.bundle_id)
                    .join(path.with_added_extension(ASSET_TAGS_EXT));
            if let Some(parent) = modified_path.parent() {
                create_dir_all(parent)?;
            }
            File::create(modified_path)?.write_all(toml::to_string(&tags)?.as_bytes())?;
        } else {
            self.bundle
                .write_asset_tags(path, &tags)
                .map_err(AssetErrorKind::BundleError)?;
        }

        Ok(())
    }

    pub fn read_asset(
        &self,
        id: UntypedAssetId,
        revision: u32,
    ) -> AssetResult<Arc<dyn ErasedAsset>> {
        let id_to_path = self.manifest.read();
        let path = id_to_path
            .assets
            .get(&id)
            .ok_or_else(|| AssetErrorKind::AssetPathNotFound(id))?;
        let serializer = self.serializers.get_for_path(path)?;

        let asset = read_asset_file(
            &self.assets_root,
            revision,
            path,
            &self.metadata.bundle_id,
            self.bundle.as_ref(),
            serializer.as_ref(),
        )?;

        self.assets.write().insert(id, asset.clone());
        Ok(asset)
    }

    pub fn add_tag(&self, tag: &Tag) -> AssetResult<()> {
        let path = PathBuf::from(&tag.relative_path).clean();
        self.bundle
            .add_tag(&path, &TagFile::from(tag.clone()))
            .map_err(AssetErrorKind::BundleError)?;
        self.manifest.write().tags.insert(tag.id, path);
        Ok(())
    }

    pub fn absolute_modified_path(&self, path: impl AsRef<Path>) -> PathBuf {
        modified_bundle_absolute_path(&self.assets_root, &self.metadata.bundle_id).join(path)
    }
}

pub(crate) fn read_asset_file(
    assets_root: &Path,
    revision: u32,
    path: &Path,
    bundle_id: &BundleId,
    bundle: &dyn ErasedAssetBundle,
    serializer: &dyn ErasedAssetSerializer,
) -> AssetResult<Arc<dyn ErasedAsset>> {
    if revision == 0 {
        Ok(bundle
            .read(path, serializer)
            .map_err(AssetErrorKind::BundleError)?)
    } else {
        let path = modified_asset_relative_path(path, revision);
        read_modified_asset(assets_root, &path, bundle_id, serializer)
    }
}

pub(crate) fn read_tag_file(
    tag_path: &Path,
    bundle: &dyn ErasedAssetBundle,
) -> AssetResult<TagFile> {
    Ok(bundle
        .read_tag(tag_path)
        .map_err(AssetErrorKind::BundleError)?)
}

pub(crate) fn read_asset_tags_file(
    assets_root: &Path,
    asset_path: &Path,
    bundle_id: &BundleId,
    bundle: &dyn ErasedAssetBundle,
) -> AssetResult<Option<AssetTags>> {
    if bundle.is_readonly() {
        let modified_path = modified_bundle_absolute_path(assets_root, bundle_id)
            .join(asset_path.with_added_extension(ASSET_TAGS_EXT));
        if modified_path.exists() {
            return Ok(Some(toml::from_str(&std::fs::read_to_string(
                modified_path,
            )?)?));
        }
    }

    Ok(bundle
        .read_asset_tags(asset_path)
        .map_err(AssetErrorKind::BundleError)?)
}

pub(crate) fn scan_bundle_assets(
    assets_root: &Path,
    bundle_meta: AssetBundleMetadata,
    manifest: &BundleManifest,
    serializers: &AssetSerializerRegistry,
) -> AssetResult<Vec<AssetMetadata>> {
    let mut assets = Vec::with_capacity(manifest.assets.len());

    for (id, path) in &manifest.assets {
        let Ok(serializer) = serializers.get_for_path(path) else {
            log::warn!(
                "Skipped loading asset at {} because of unknown extension.",
                path.display(),
            );
            continue;
        };
        assets.push(AssetMetadata {
            asset_id: *id,
            ty: serializer.asset_type_name().to_string(),
            bundle_id: bundle_meta.bundle_id,
            relative_path: path.to_string_lossy().to_string(),
            revision: 0,
            // TODO: This isn't accurate, but... probably good enough for now?
            last_modified: bundle_meta.last_modified,
            in_memory: false,
        });
    }

    let modified = scan_modified_assets(
        assets_root,
        &bundle_meta.bundle_id,
        &manifest.assets,
        serializers,
    )?;
    assets.extend(modified);

    Ok(assets)
}

fn scan_modified_assets(
    assets_root: &Path,
    bundle_id: &BundleId,
    bundle_manifest: &BTreeMap<UntypedAssetId, PathBuf>,
    serializers: &AssetSerializerRegistry,
) -> AssetResult<Vec<AssetMetadata>> {
    let mut assets = Vec::new();
    let modified_bundle_path = modified_bundle_absolute_path(assets_root, bundle_id);
    if !modified_bundle_path.exists() {
        return Ok(assets);
    }

    let reversed_manifest = bundle_manifest
        .iter()
        .map(|(id, path)| (path.clone(), *id))
        .collect::<HashMap<_, _>>();

    scan_modified_assets_dfs(
        &modified_bundle_path,
        bundle_id,
        &modified_bundle_path,
        &mut assets,
        serializers,
        &reversed_manifest,
    )?;

    Ok(assets)
}

fn scan_modified_assets_dfs(
    modified_bundle_path: &Path,
    bundle_id: &BundleId,
    current_path: &Path,
    assets: &mut Vec<AssetMetadata>,
    serializers: &AssetSerializerRegistry,
    path_to_id: &HashMap<PathBuf, UntypedAssetId>,
) -> AssetResult<()> {
    for entry in current_path.read_dir()? {
        let Ok(entry) = entry else {
            continue;
        };

        let path = entry.path();
        if path.is_dir() {
            let _ = scan_modified_assets_dfs(
                modified_bundle_path,
                bundle_id,
                &path,
                assets,
                serializers,
                path_to_id,
            );
        } else if path.is_file() {
            if path.extension() == Some(OsStr::new(ASSET_TAGS_EXT))
                || path.extension() == Some(OsStr::new(TAG_EXT))
            {
                continue;
            }

            let Some(revision) = parse_revision_from_path(&path) else {
                log::warn!(
                    "Skipped loading modified asset at {} because it does not have a revision.",
                    path.display(),
                );
                continue;
            };

            let Ok(serializer) = serializers.get_for_path(&path) else {
                log::warn!(
                    "Skipped loading modified asset at {} because of unknown extension.",
                    path.display(),
                );
                continue;
            };
            let modified_path = path
                .strip_prefix(modified_bundle_path)
                .unwrap()
                .to_string_lossy()
                .to_string();

            let Some(original_path) = restore_original_path(Path::new(&modified_path)) else {
                continue;
            };
            let Some(asset_id) = path_to_id.get(&original_path) else {
                continue;
            };

            assets.push(AssetMetadata {
                asset_id: *asset_id,
                ty: serializer.asset_type_name().to_string(),
                bundle_id: *bundle_id,
                relative_path: modified_path,
                revision,
                last_modified: metadata(&path)?.modified()?.into(),
                in_memory: false,
            });
        }
    }

    Ok(())
}

fn parse_revision_from_path(path: &Path) -> Option<u32> {
    let filename = path.file_stem()?.to_str()?;
    let parts = filename.split(".rev").collect::<Vec<_>>();
    if parts.len() != 2 {
        return None;
    }
    parts[1].split('.').next()?.parse().ok()
}

fn restore_original_path(modified_path: &Path) -> Option<PathBuf> {
    let filename = modified_path.file_stem()?.to_str()?;
    let parts = filename.split(".rev").collect::<Vec<_>>();
    if parts.len() != 2 {
        return None;
    }
    let original_filename = format!(
        "{}.{}",
        parts[0],
        modified_path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
    );
    Some(modified_path.with_file_name(original_filename))
}

fn read_modified_asset(
    assets_root: &Path,
    asset_relative_path: &Path,
    bundle_id: &BundleId,
    serializer: &dyn ErasedAssetSerializer,
) -> AssetResult<Arc<dyn ErasedAsset>> {
    let path = modified_bundle_absolute_path(assets_root, bundle_id).join(asset_relative_path);
    let mut file = File::open(path)?;
    Ok(serializer
        .read(&mut file)
        .map_err(AssetErrorKind::SerializerError)
        .map(Into::into)?)
}

fn write_modified_asset(
    assets_root: &Path,
    original_relative_path: &Path,
    bundle_id: &BundleId,
    asset: &dyn ErasedAsset,
    revision: u32,
    serializer: &dyn ErasedAssetSerializer,
) -> AssetResult<PathBuf> {
    let new_relative_path = modified_asset_relative_path(original_relative_path, revision);
    let modified_bundle_path = modified_bundle_absolute_path(assets_root, bundle_id);
    let modified_asset_path = modified_bundle_path.join(&new_relative_path);

    if let Some(dir) = &modified_asset_path.parent() {
        create_dir_all(dir)?;
    }
    let mut file = File::create(modified_asset_path)?;
    serializer
        .write(asset, &mut file)
        .map_err(AssetErrorKind::SerializerError)?;

    Ok(new_relative_path)
}

fn modified_asset_relative_path(asset_relative_path: &Path, revision: u32) -> PathBuf {
    let ext = asset_relative_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let file_stem = asset_relative_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    asset_relative_path.with_file_name(format!("{}.rev{}.{}", file_stem, revision, ext))
}

pub fn modified_bundle_absolute_path(
    assets_root: impl AsRef<Path>,
    bundle_id: &BundleId,
) -> PathBuf {
    assets_root
        .as_ref()
        .join(format!("{}.modified", bundle_id.0))
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub assets: BTreeMap<UntypedAssetId, PathBuf>,
    pub tags: BTreeMap<TagId, PathBuf>,
}

pub trait AssetBundle: Send + Sync + 'static {
    const READONLY: bool;
    type Error: Error + Sync + Send + 'static;

    fn metadata(&self) -> Result<AssetBundleMetadata, Self::Error>;
    fn manifest(&self) -> Result<BundleManifest, Self::Error>;
    fn read_asset(
        &self,
        path: &Path,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<Arc<dyn ErasedAsset>, Self::Error>;
    fn add_asset(
        &self,
        path: &Path,
        asset: &dyn ErasedAsset,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<UntypedAssetId, Self::Error>;
    fn read_tag(&self, tag: &Path) -> Result<TagFile, Self::Error>;
    fn add_tag(&self, path: &Path, tag: &TagFile) -> Result<(), Self::Error>;
    fn read_asset_tags(&self, path: &Path) -> Result<Option<AssetTags>, Self::Error>;
    fn write_asset_tags(&self, path: &Path, tags: &AssetTags) -> Result<(), Self::Error>;
}

pub trait ErasedAssetBundle: Send + Sync + 'static {
    fn is_readonly(&self) -> bool;
    fn metadata(&self) -> Result<AssetBundleMetadata, Box<dyn Error + Send + Sync + 'static>>;
    fn manifest(&self) -> Result<BundleManifest, Box<dyn Error + Send + Sync + 'static>>;
    fn read(
        &self,
        path: &Path,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<Arc<dyn ErasedAsset>, Box<dyn Error + Send + Sync + 'static>>;
    fn add(
        &self,
        path: &Path,
        asset: &dyn ErasedAsset,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<UntypedAssetId, Box<dyn Error + Send + Sync + 'static>>;
    fn read_tag(&self, path: &Path) -> Result<TagFile, Box<dyn Error + Send + Sync + 'static>>;
    fn add_tag(
        &self,
        path: &Path,
        tag: &TagFile,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>>;
    fn read_asset_tags(
        &self,
        path: &Path,
    ) -> Result<Option<AssetTags>, Box<dyn Error + Send + Sync + 'static>>;
    fn write_asset_tags(
        &self,
        path: &Path,
        tags: &AssetTags,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>>;
}

impl<T: AssetBundle> ErasedAssetBundle for T {
    fn is_readonly(&self) -> bool {
        T::READONLY
    }

    fn metadata(&self) -> Result<AssetBundleMetadata, Box<dyn Error + Send + Sync + 'static>> {
        self.metadata().map_err(Into::into)
    }

    fn manifest(&self) -> Result<BundleManifest, Box<dyn Error + Send + Sync + 'static>> {
        self.manifest().map_err(Into::into)
    }

    fn read(
        &self,
        path: &Path,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<Arc<dyn ErasedAsset>, Box<dyn Error + Send + Sync + 'static>> {
        self.read_asset(path, serializer).map_err(Into::into)
    }

    fn add(
        &self,
        path: &Path,
        asset: &dyn ErasedAsset,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<UntypedAssetId, Box<dyn Error + Send + Sync + 'static>> {
        self.add_asset(path, asset, serializer).map_err(Into::into)
    }

    fn read_tag(&self, path: &Path) -> Result<TagFile, Box<dyn Error + Send + Sync + 'static>> {
        self.read_tag(path).map_err(Into::into)
    }

    fn add_tag(
        &self,
        path: &Path,
        tag: &TagFile,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        self.add_tag(path, tag).map_err(Into::into)
    }

    fn read_asset_tags(
        &self,
        path: &Path,
    ) -> Result<Option<AssetTags>, Box<dyn Error + Send + Sync + 'static>> {
        self.read_asset_tags(path).map_err(Into::into)
    }

    fn write_asset_tags(
        &self,
        path: &Path,
        tags: &AssetTags,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        self.write_asset_tags(path, tags).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, io};

    use super::*;
    use crate::tag::TagId;

    struct ReadonlyBundle {
        id: BundleId,
        asset_id: UntypedAssetId,
        asset_path: PathBuf,
        tags: AssetTags,
    }

    impl AssetBundle for ReadonlyBundle {
        const READONLY: bool = true;
        type Error = io::Error;

        fn metadata(&self) -> Result<AssetBundleMetadata, Self::Error> {
            Ok(AssetBundleMetadata {
                bundle_id: self.id,
                name: "readonly".to_string(),
                last_modified: Utc::now(),
            })
        }

        fn manifest(&self) -> Result<BundleManifest, Self::Error> {
            Ok(BundleManifest {
                assets: BTreeMap::from([(self.asset_id, self.asset_path.clone())]),
                tags: BTreeMap::new(),
            })
        }

        fn read_asset(
            &self,
            _: &Path,
            _: &dyn ErasedAssetSerializer,
        ) -> Result<Arc<dyn ErasedAsset>, Self::Error> {
            Err(io::Error::other("not used by this test"))
        }

        fn add_asset(
            &self,
            _: &Path,
            _: &dyn ErasedAsset,
            _: &dyn ErasedAssetSerializer,
        ) -> Result<UntypedAssetId, Self::Error> {
            Err(io::Error::other("readonly"))
        }

        fn read_tag(&self, _: &Path) -> Result<TagFile, Self::Error> {
            Err(io::Error::other("not used by this test"))
        }

        fn add_tag(&self, _: &Path, _: &TagFile) -> Result<(), Self::Error> {
            Err(io::Error::other("readonly"))
        }

        fn read_asset_tags(&self, _: &Path) -> Result<Option<AssetTags>, Self::Error> {
            Ok(Some(self.tags.clone()))
        }

        fn write_asset_tags(&self, _: &Path, _: &AssetTags) -> Result<(), Self::Error> {
            Err(io::Error::other("readonly"))
        }
    }

    #[test]
    fn readonly_asset_tags_are_overridden_in_modified_directory() -> AssetResult<()> {
        let root = std::env::temp_dir().join(format!("lapiz-readonly-tags-{}", Uuid::new_v4()));
        let bundle_id = BundleId::new(Uuid::from_u128(1));
        let asset_id = UntypedAssetId::new(Uuid::from_u128(2));
        let asset_path = PathBuf::from("brushes/sample.lapiz");
        let base_tag = TagId::new(Uuid::from_u128(3));
        let override_tag = TagId::new(Uuid::from_u128(4));
        let bundle = Arc::new(ReadonlyBundle {
            id: bundle_id,
            asset_id,
            asset_path: asset_path.clone(),
            tags: AssetTags {
                tags: BTreeSet::from([base_tag]),
            },
        });
        let cache = AssetBundleCache::new(
            root.clone(),
            BundleSnapshot {
                bundle,
                metadata: AssetBundleMetadata {
                    bundle_id,
                    name: "readonly".to_string(),
                    last_modified: Utc::now(),
                },
                manifest: BundleManifest {
                    assets: BTreeMap::from([(asset_id, asset_path.clone())]),
                    tags: BTreeMap::new(),
                },
                assets: Vec::new(),
                tags: Vec::new(),
                asset_tags: HashMap::new(),
            },
            Arc::new(AssetSerializerRegistry::default()),
        );

        assert_eq!(
            cache.read_asset_tags(&asset_id)?.tags,
            BTreeSet::from([base_tag])
        );

        cache.write_asset_tags(&asset_id, std::slice::from_ref(&override_tag))?;
        assert_eq!(
            cache.read_asset_tags(&asset_id)?.tags,
            BTreeSet::from([override_tag])
        );
        assert!(
            modified_bundle_absolute_path(&root, &bundle_id)
                .join(asset_path.with_added_extension(ASSET_TAGS_EXT))
                .is_file()
        );

        cache.write_asset_tags(&asset_id, &[])?;
        assert!(cache.read_asset_tags(&asset_id)?.tags.is_empty());
        assert!(
            cache
                .add_tag(&Tag {
                    id: TagId::new(Uuid::new_v4()),
                    bundle_id,
                    relative_path: "new.tag".to_string(),
                    name: "New".to_string(),
                    asset_ty: None,
                })
                .is_err()
        );

        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
