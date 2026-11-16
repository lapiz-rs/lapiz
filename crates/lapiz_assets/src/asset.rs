use std::{
    fmt, fs,
    hash::{Hash, Hasher},
    marker::PhantomData,
    ops::Deref,
    sync::Arc,
};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use downcast_rs::DowncastSync;
use indexmap::IndexSet;
use lapiz_utils::wrapper;
use parse_display::Display;
use rusqlite::types::{FromSql, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    bundle::{AssetBundleCache, BundleId},
    store::AssetRegistry,
    tag::{AssetPosition, TagId},
};

wrapper! {
    /// A type-erased unique identifier for an asset.
    ///
    /// Wraps a [`Uuid`] to identify an asset across bundles without compile-time knowledge of its
    /// concrete asset type. It can be converted into a strongly typed [`AssetId<T>`] via
    /// [`into_typed`](UntypedAssetId::into_typed).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Display)]
    #[display("{0}")]
    pub UntypedAssetId: Uuid
}

impl UntypedAssetId {
    pub fn into_typed<T: Asset>(self) -> AssetId<T> {
        AssetId::new(self.0)
    }
}

impl FromSql for UntypedAssetId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Ok(Self(Uuid::column_result(value)?))
    }
}

impl ToSql for UntypedAssetId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

/// A strongly-typed unique identifier for an [`Asset`] of type `T`.
///
/// This wraps a [`Uuid`] alongside a [`PhantomData<T>`] marker to enforce type safety
/// at compile time when referencing and retrieving assets.
///
/// An [`AssetId`] can be converted into a type-erased [`UntypedAssetId`] via
/// [`into_untyped`](AssetId::into_untyped) for storage or dynamic dispatch.
#[derive(Display)]
#[display("{id}")]
pub struct AssetId<T: Asset> {
    id: Uuid,
    _marker: PhantomData<T>,
}

impl<T: Asset> fmt::Debug for AssetId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("AssetId").field(&self.id).finish()
    }
}

impl<T: Asset> Clone for AssetId<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Asset> Copy for AssetId<T> {}

impl<T: Asset> PartialEq for AssetId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: Asset> Eq for AssetId<T> {}

impl<T: Asset> Hash for AssetId<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: Asset> Serialize for AssetId<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.id.serialize(serializer)
    }
}

impl<'de, T: Asset> Deserialize<'de> for AssetId<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = Uuid::deserialize(deserializer)?;
        Ok(Self {
            id,
            _marker: PhantomData,
        })
    }
}

impl<T: Asset> Deref for AssetId<T> {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl<T: Asset> AssetId<T> {
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    pub fn into_untyped(self) -> UntypedAssetId {
        UntypedAssetId::new(self.id)
    }
}

/// A piece of data that can be loaded, stored, and managed by the asset subsystem.
///
/// Types implementing `Asset` can be referenced by typed [`AssetHandle`]s and [`AssetId`]s,
/// indexed by the [`AssetIndexDb`](crate::index_db::AssetIndexDb), and persisted by matching [`AssetSerializer`](crate::loader::AssetSerializer)s.
///
/// # Requirements
///
/// Implementing types must satisfy `Send + Sync + 'static` and implement [`DowncastSync`] (typically
/// provided automatically via `downcast_rs`). Each asset must declare a unique [`TYPE_NAME`](Asset::TYPE_NAME)
/// used as its identifier in database records and metadata.
///
/// All types implementing `Asset` automatically implement the type-erased [`ErasedAsset`] trait.
pub trait Asset: Send + Sync + 'static + DowncastSync {
    const TYPE_NAME: &'static str;
}

pub trait ErasedAsset: Send + Sync + 'static + DowncastSync {
    fn type_name(&self) -> &'static str;
}

downcast_rs::impl_downcast!(sync ErasedAsset);

impl<T: Asset> ErasedAsset for T {
    fn type_name(&self) -> &'static str {
        T::TYPE_NAME
    }
}

/// Metadata describing an asset stored in the asset system and index database.
///
/// Tracks the asset's identity, concrete type name, containing bundle, relative path,
/// revision number, last modification timestamp, and in-memory residency status.
#[derive(Debug, Clone)]
pub struct AssetMetadata {
    pub asset_id: UntypedAssetId,
    pub ty: String,
    pub bundle_id: BundleId,
    pub relative_path: String,
    pub revision: u32,
    pub last_modified: DateTime<Utc>,
    pub in_memory: bool,
}

/// A handle to a loaded or indexed asset of type `T`.
///
/// `AssetHandle` serves as the primary gateway for interacting with an individual asset.
/// It holds a typed [`AssetId<T>`], a reference to the asset's parent [`AssetBundleCache`],
/// and its [`AssetRegistry`] for index queries and tag membership changes.
///
/// # Capabilities
///
/// - **Retrieval**: Load or fetch the cached in-memory instance via [`get`](AssetHandle::get).
/// - **Modification**: Update the in-memory asset via [`update`](AssetHandle::update) and persist changes
///   to disk with automated revision tracking via [`write`](AssetHandle::write).
/// - **Deletion**: Remove the asset from the in-memory cache and index database via [`delete`](AssetHandle::delete).
/// - **Tagging**: Inspect, assign, and remove categorization [`TagId`]s associated with the asset.
pub struct AssetHandle<T: Asset> {
    id: AssetId<T>,
    bundle: Arc<AssetBundleCache>,
    registry: AssetRegistry,
}

impl<T: Asset> Clone for AssetHandle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            bundle: Arc::clone(&self.bundle),
            registry: self.registry.clone(),
        }
    }
}

impl<T: Asset> PartialEq for AssetHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: Asset> Eq for AssetHandle<T> {}

impl<T: Asset> Hash for AssetHandle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: Asset> AssetHandle<T> {
    pub(crate) fn new(
        id: AssetId<T>,
        bundle: Arc<AssetBundleCache>,
        registry: AssetRegistry,
    ) -> Self {
        Self {
            id,
            bundle,
            registry,
        }
    }

    pub fn id(&self) -> AssetId<T> {
        self.id
    }

    pub fn untyped_id(&self) -> UntypedAssetId {
        self.id.into_untyped()
    }

    pub fn bundle(&self) -> &AssetBundleCache {
        self.bundle.as_ref()
    }

    pub fn get(&self) -> Result<Arc<T>> {
        let dynamic = match self.bundle.get_cached_asset(&self.untyped_id()) {
            Ok(cached) => cached,
            Err(_) => {
                let metadata = self.metadata()?;
                self.bundle
                    .read_asset(self.untyped_id(), metadata.revision)?
            }
        };

        dynamic.downcast_arc::<T>().map_err(|_| {
            anyhow!(
                "Failed to downcast asset into desired type: {}",
                T::TYPE_NAME
            )
        })
    }

    pub fn update(&self, asset: T) -> Result<()> {
        self.bundle
            .update_asset(self.untyped_id(), Arc::new(asset))?;
        self.registry.index_db().update_asset(&self.untyped_id())?;

        Ok(())
    }

    pub fn write(&self) -> Result<()> {
        let metadata = self.metadata()?;
        if !metadata.in_memory {
            return Ok(());
        }
        let new_path = self
            .bundle
            .write_asset(&self.untyped_id(), metadata.revision)?;
        let last_modified =
            fs::metadata(self.bundle.absolute_modified_path(&new_path))?.modified()?;
        self.registry.index_db().write_asset(
            &self.untyped_id(),
            new_path.to_str().unwrap(),
            last_modified.into(),
        )?;
        Ok(())
    }

    pub fn delete(&self) -> Result<()> {
        let id = self.untyped_id();
        self.bundle.delete_cached_asset(&id)?;
        self.registry.index_db().delete_asset(&id)?;
        Ok(())
    }

    pub fn read_tags(&self) -> Result<IndexSet<TagId>> {
        self.registry.index_db().get_asset_tags(&self.untyped_id())
    }

    pub fn add_to_position_of_tag(&self, tag_id: &TagId, position: AssetPosition) -> Result<()> {
        let metadata = self.metadata()?;
        let tag = self.registry.index_db().get_tag(*tag_id)?;
        if let Some(expected_ty) = &tag.asset_ty
            && metadata.ty != *expected_ty
        {
            bail!(
                "Tag {tag_id} cannot be added to asset {} of type {}; expected {expected_ty}",
                self.untyped_id(),
                metadata.ty
            );
        }

        let bundle = self.registry.bundle(&tag.bundle_id)?;
        let mut assets = bundle.read_tagged_assets(tag_id)?;
        assets.insert(self.untyped_id(), position)?;

        bundle.write_tagged_assets(tag_id, &assets)?;
        self.registry
            .index_db()
            .edit_tagged_assets(tag_id, &assets)?;
        Ok(())
    }

    pub fn remove_tag(&self, tag_id: &TagId) -> Result<()> {
        let tag = self.registry.index_db().get_tag(*tag_id)?;
        let bundle = self.registry.bundle(&tag.bundle_id)?;
        let mut assets = bundle.read_tagged_assets(tag_id)?;
        if !assets.assets.shift_remove(&self.untyped_id()) {
            bail!(
                "Asset {} is not assigned to tag {}",
                self.untyped_id(),
                tag.name
            );
        }

        bundle.write_tagged_assets(tag_id, &assets)?;
        self.registry.index_db().edit_tagged_assets(tag_id, &assets)
    }

    pub fn metadata(&self) -> Result<AssetMetadata> {
        self.registry.index_db().get_asset(&self.untyped_id())
    }
}
