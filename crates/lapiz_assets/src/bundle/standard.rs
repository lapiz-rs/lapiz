use std::{
    fs::{self, File, metadata},
    io::{Cursor, Read as _, read_to_string},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Error, Result, bail};
use chrono::DateTime;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use zip::{ZipArchive, result::ZipError};

use crate::{
    asset::{ErasedAsset, UntypedAssetId},
    bundle::{AssetBundle, AssetBundleMetadata, BundleId, BundleManifest, ErasedAssetBundle},
    loader::ErasedAssetSerializer,
    source::AssetSource,
    tag::{ASSET_TAGS_EXT, AssetTags, TagFile},
};

/// An [`AssetSource`] that scans an asset root for packaged standard `.lazurite` bundles.
pub struct StandardAssetBundleSource;

impl AssetSource for StandardAssetBundleSource {
    fn scan(root: &Path) -> Vec<Arc<dyn ErasedAssetBundle>> {
        let (bundles, errors) = StandardAssetBundle::scan_bundles(root);
        for error in errors {
            log::error!("{}", error);
        }
        bundles.into_iter().map(|b| Arc::new(b) as _).collect()
    }
}

/// A packaged, read-only [`AssetBundle`] backed by a `.lazurite` ZIP archive.
///
/// Loads metadata, manifests, assets, and tags directly from an archive on disk.
/// Any modifications or overrides to assets contained in this bundle are handled by
/// writing revisioned copies into an external modified folder rather than updating the ZIP archive.
pub struct StandardAssetBundle {
    path: PathBuf,
    archive: RwLock<ZipArchive<File>>,
}

impl StandardAssetBundle {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let archive = ZipArchive::new(File::open(&path)?)?;

        Ok(Self {
            path,
            archive: archive.into(),
        })
    }

    pub fn scan_bundles(root: impl AsRef<Path>) -> (Vec<Self>, Vec<Error>) {
        let mut bundles = Vec::new();
        let mut errors = Vec::new();
        scan_bundles(root, &mut bundles, &mut errors);
        (bundles, errors)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn scan_bundles(
    root: impl AsRef<Path>,
    bundles: &mut Vec<StandardAssetBundle>,
    errors: &mut Vec<Error>,
) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) => {
            errors.push(e.into());
            return;
        }
    };

    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };

        let path = entry.path();
        if path.is_file() {
            let ext = path.extension().and_then(|ext| ext.to_str());
            if ext == Some("lazurite") {
                match StandardAssetBundle::new(&path) {
                    Ok(bundle) => bundles.push(bundle),
                    Err(e) => errors.push(e),
                }
            }
        } else if path.is_dir() {
            scan_bundles(path, bundles, errors);
        }
    }
}

/// Header metadata deserialized from `metadata.toml` inside a `.lazurite` bundle archive.
#[derive(Serialize, Deserialize)]
pub struct StandardAssetBundleMetadata {
    pub bundle_id: BundleId,
    pub name: String,
}

impl AssetBundle for StandardAssetBundle {
    const READONLY: bool = true;

    fn metadata(&self) -> Result<AssetBundleMetadata> {
        let mut archive = self.archive.write();
        let content = read_to_string(archive.by_name("metadata.toml")?)?;
        let bundle_meta = toml::from_str::<StandardAssetBundleMetadata>(&content)?;
        let last_modified = DateTime::from(metadata(&self.path)?.modified()?);

        Ok(AssetBundleMetadata {
            bundle_id: bundle_meta.bundle_id,
            name: bundle_meta.name,
            last_modified,
        })
    }

    fn manifest(&self) -> Result<BundleManifest> {
        let mut archive = self.archive.write();
        let content = read_to_string(archive.by_name("manifest.toml")?)?;
        Ok(toml::from_str(&content)?)
    }

    fn read_asset(
        &self,
        path: &Path,
        serializer: &dyn ErasedAssetSerializer,
    ) -> Result<Arc<dyn ErasedAsset>> {
        let mut archive = self.archive.write();
        let mut file = archive.by_name(path.to_str().unwrap_or_default())?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        let asset = serializer.read(&mut Cursor::new(buffer))?;
        Ok(asset.into())
    }

    fn add_asset(
        &self,
        _: &Path,
        _: &dyn ErasedAsset,
        _: &dyn ErasedAssetSerializer,
    ) -> Result<UntypedAssetId> {
        bail!("Unsupported writing to standard asset bundle")
    }

    fn read_tag(&self, tag: &Path) -> Result<TagFile> {
        let path = tag.to_string_lossy().replace('\\', "/");
        let mut archive = self.archive.write();
        let mut file = archive.by_name(&path)?;
        Ok(toml::from_str(&read_to_string(&mut file)?)?)
    }

    fn add_tag(&self, _: &Path, _: &TagFile) -> Result<()> {
        bail!("Unsupported writing to standard asset bundle")
    }

    fn read_asset_tags(&self, path: &Path) -> Result<Option<AssetTags>> {
        let path = path
            .with_added_extension(ASSET_TAGS_EXT)
            .to_string_lossy()
            .replace('\\', "/");
        let mut archive = self.archive.write();
        let mut file = match archive.by_name(&path) {
            Ok(file) => file,
            Err(ZipError::FileNotFound) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let content = read_to_string(&mut file)?;
        Ok(Some(toml::from_str(&content)?))
    }

    fn write_asset_tags(&self, _: &Path, _: &AssetTags) -> Result<()> {
        bail!("Unsupported writing to standard asset bundle")
    }
}
