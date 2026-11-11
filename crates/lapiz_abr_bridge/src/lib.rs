// DISCLAIMER
//
// This crate was developed exclusively through manual clean room reverse
// engineering, and the following statements are made with respect to that
// work:
//
// 1. The implementation relies solely on publicly available documentation of
//    the Adobe Brush (ABR) file format and on other existing, independently
//    developed implementations of that format.
//
// 2. Adobe Photoshop was used only as a reference for artifacts produced by
//    hand: sample ABR files were created manually through the Photoshop user
//    interface and examined as reference material for this crate.
//
// 3. No Adobe Photoshop binary, library, or other executable component was
//    disassembled, decompiled, or otherwise inspected.
//
// 4. No script, tool, or automated process was used to run, probe, instrument,
//    or debug Adobe Photoshop.
//
// This crate is an independent implementation of the ABR format. It contains
// no Adobe software and is not affiliated with, endorsed by, or sponsored by
// Adobe Inc.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Error, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use lapiz_abr::Abr;
use lapiz_assets::{
    asset::{AssetId, ErasedAsset, UntypedAssetId},
    bundle::{AssetBundle, AssetBundleMetadata, BundleId, BundleManifest, ErasedAssetBundle},
    loader::ErasedAssetSerializer,
    source::{AssetSource, AssetSourceRegistryExt as _},
    tag::{AssetTags, TagFile},
};
use lapiz_render::texture::Image;
use lapiz_runtime::{Runtime, plugin::Plugin};
use uuid::Uuid;
use xxhash_rust::xxh3::xxh3_128;

pub mod desc;
pub mod patt;
pub mod samp;

/// Plugin that registers [`AbrAssetBundleSource`] so `.abr` files are discovered as asset bundles.
pub struct AbrBridgePlugin;

impl Plugin for AbrBridgePlugin {
    fn build(&self, app: &mut Runtime) {
        app.globals_mut().add_asset_source::<AbrAssetBundleSource>();
    }
}

/// An [`AssetSource`] that scans an asset root for Adobe Brush (`.abr`) files.
pub struct AbrAssetBundleSource;

impl AssetSource for AbrAssetBundleSource {
    fn scan(root: &Path) -> Vec<Arc<dyn ErasedAssetBundle>> {
        let (bundles, errors) = AbrAssetBundle::scan_bundles(root);
        for error in errors {
            log::error!("{}", error);
        }
        bundles.into_iter().map(|b| Arc::new(b) as _).collect()
    }
}

/// A read-only [`AssetBundle`] backed by a parsed Adobe Brush (`.abr`) file.
///
/// Converts the contents of an [`Abr`] archive into in-memory engine assets upon opening:
///
/// - **Samples (`samp`)**: Converted into [`Image`] mask assets (`samp-<id>.lig`).
/// - **Patterns (`patt`)**: Converted into [`Image`] texture assets (`patt-<id>.lig`).
/// - **Descriptors (`desc`)**: Compiled into [`BrushPreset`](lapiz_brush::asset::BrushPreset) assets
///   (`desc-<id>.lapiz`) with generated spacing, main dab, and postprocess shader graphs referencing
///   the converted sample and pattern [`AssetId`]s.
///
/// Asset IDs are deterministically derived as UUID v5 values namespaced by the bundle's [`BundleId`].
pub struct AbrAssetBundle {
    path: PathBuf,
    metadata: AssetBundleMetadata,
    manifest: BundleManifest,
    assets: HashMap<PathBuf, Arc<dyn ErasedAsset>>,
}

impl AbrAssetBundle {
    pub fn parse(path: impl AsRef<Path>, abr: Abr) -> Self {
        let path = path.as_ref().to_path_buf();
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bundle_id = BundleId::new(Uuid::from_u128(xxh3_128(name.as_bytes())));
        let last_modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map(DateTime::<Utc>::from)
            .unwrap_or_else(|_| Utc::now());

        let metadata = AssetBundleMetadata {
            bundle_id,
            name,
            last_modified,
        };
        let mut manifest = BundleManifest {
            assets: BTreeMap::new(),
            tags: BTreeMap::new(),
        };
        let mut assets = HashMap::new();
        let mut sample_assets = HashMap::<Uuid, AssetId<Image>>::new();
        let mut pattern_assets = HashMap::<Uuid, AssetId<Image>>::new();

        for sample in &abr.samples {
            let asset_id = UntypedAssetId::new(Uuid::new_v5(&bundle_id.0, sample.id.as_bytes()));
            let path = PathBuf::from(format!("samp-{asset_id}.lig"));
            match samp::parse_samp(sample) {
                Ok(asset) => {
                    sample_assets.insert(sample.id, asset_id.into_typed());
                    manifest.assets.insert(asset_id, path.clone());
                    assets.insert(path, Arc::new(asset) as Arc<dyn ErasedAsset>);
                }
                Err(error) => {
                    log::error!("Failed to convert samp {}: {error}", sample.id);
                }
            }
        }

        for pattern in &abr.patterns {
            let asset_id = UntypedAssetId::new(Uuid::new_v5(&bundle_id.0, pattern.id.as_bytes()));
            let path = PathBuf::from(format!("patt-{asset_id}.lig"));
            match patt::parse_patt(pattern) {
                Ok(asset) => {
                    pattern_assets.insert(pattern.id, asset_id.into_typed());
                    manifest.assets.insert(asset_id, path.clone());
                    assets.insert(path, Arc::new(asset) as Arc<dyn ErasedAsset>);
                }
                Err(error) => {
                    log::error!(
                        "Failed to convert patt {} ({}): {error}",
                        pattern.name,
                        pattern.id
                    );
                }
            }
        }

        for brush in &abr.descriptors {
            let asset_id = UntypedAssetId::new(Uuid::new_v5(&bundle_id.0, brush.name.as_bytes()));
            let path = PathBuf::from(format!("desc-{asset_id}.lapiz"));
            match desc::parse_desc(brush, &sample_assets, &pattern_assets) {
                Ok(asset) => {
                    manifest.assets.insert(asset_id, path.clone());
                    assets.insert(path, Arc::new(asset) as Arc<dyn ErasedAsset>);
                }
                Err(error) => {
                    log::error!("Failed to convert desc {}: {error}", brush.name);
                }
            }
        }

        Self {
            path,
            metadata,
            manifest,
            assets,
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let abr = Abr::parse(&fs::read(&path)?)?;
        Ok(Self::parse(path, abr))
    }

    pub fn scan_bundles(root: impl AsRef<Path>) -> (Vec<Self>, Vec<Error>) {
        let mut bundles = Vec::new();
        let mut errors = Vec::new();
        scan_bundles_dfs(root.as_ref(), &mut bundles, &mut errors);
        (bundles, errors)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn scan_bundles_dfs(root: &Path, bundles: &mut Vec<AbrAssetBundle>, errors: &mut Vec<Error>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(error.into());
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(error.into());
                continue;
            }
        };
        let path = entry.path();
        if path.is_file() {
            if path.extension() == Some(OsStr::new("abr")) {
                match AbrAssetBundle::open(&path) {
                    Ok(bundle) => bundles.push(bundle),
                    Err(error) => errors.push(error),
                }
            }
        } else if path.is_dir() {
            scan_bundles_dfs(&path, bundles, errors);
        }
    }
}

impl AssetBundle for AbrAssetBundle {
    const READONLY: bool = true;

    fn metadata(&self) -> Result<AssetBundleMetadata> {
        Ok(self.metadata.clone())
    }

    fn manifest(&self) -> Result<BundleManifest> {
        Ok(self.manifest.clone())
    }

    fn read_asset(
        &self,
        path: &Path,
        _: &dyn ErasedAssetSerializer,
    ) -> Result<Arc<dyn ErasedAsset>> {
        self.assets
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow!("Asset not found at path: {}", path.display()))
    }

    fn add_asset(
        &self,
        _: &Path,
        _: &dyn ErasedAsset,
        _: &dyn ErasedAssetSerializer,
    ) -> Result<UntypedAssetId> {
        bail!("Unsupported writing to ABR asset bundle")
    }

    fn read_tag(&self, tag: &Path) -> Result<TagFile> {
        bail!("Tag not found at path: {}", tag.display())
    }

    fn add_tag(&self, _: &Path, _: &TagFile) -> Result<()> {
        bail!("Unsupported writing to ABR asset bundle")
    }

    fn read_asset_tags(&self, _: &Path) -> Result<Option<AssetTags>> {
        Ok(None)
    }

    fn write_asset_tags(&self, _: &Path, _: &AssetTags) -> Result<()> {
        bail!("Unsupported writing to ABR asset bundle")
    }
}
