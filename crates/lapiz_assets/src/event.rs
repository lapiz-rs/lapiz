use std::{any::TypeId, collections::HashSet, sync::Arc};

use futures::stream;
use iced_futures::Subscription;
use lapiz_runtime::event::Event;

use crate::{
    asset::{Asset, UntypedAssetId},
    bundle::BundleId,
    tag::TagId,
};

/// Event broadcast when assets or bundles change in [`AssetRegistry`](crate::store::AssetRegistry).
#[derive(Clone, Debug, Eq, Event, PartialEq)]
pub enum AssetEvent {
    Added {
        id: UntypedAssetId,
        ty: &'static str,
        bundle_id: BundleId,
    },
    Modified {
        id: UntypedAssetId,
        ty: &'static str,
        revision: u32,
    },
    Removed {
        id: UntypedAssetId,
        ty: &'static str,
    },
    BundlesChanged {
        bundle_ids: Arc<[BundleId]>,
    },
}

impl AssetEvent {
    /// Returns the asset type name for single-asset events, or `None` for bundle-wide changes.
    pub fn asset_ty(&self) -> Option<&'static str> {
        match self {
            Self::Added { ty, .. } | Self::Modified { ty, .. } | Self::Removed { ty, .. } => {
                Some(*ty)
            }
            Self::BundlesChanged { .. } => None,
        }
    }

    /// Returns whether this event may affect assets of type `ty`.
    ///
    /// Bundle-wide changes always match because a bundle can contain any asset type.
    pub fn matches_type(&self, ty: &str) -> bool {
        self.asset_ty().is_none_or(|event_ty| event_ty == ty)
    }

    /// Subscribes only to events that may affect assets of type `T`.
    pub fn listen_for<T: Asset>() -> Subscription<Self> {
        Subscription::run_with((TypeId::of::<Self>(), TypeId::of::<T>()), |_| {
            let receiver = Self::channel().1.activate_cloned();
            stream::unfold(receiver, |mut receiver| async move {
                loop {
                    let event = receiver.recv().await.ok()?;
                    if event.matches_type(T::TYPE_NAME) {
                        return Some((event, receiver));
                    }
                }
            })
        })
    }

    /// Subscribes only to events that may affect any of the specified asset types.
    pub fn listen_for_types(types: impl IntoIterator<Item = &'static str>) -> Subscription<Self> {
        let mut sorted = types.into_iter().collect::<Vec<_>>();
        sorted.sort_unstable();
        sorted.dedup();

        Subscription::run_with((TypeId::of::<Self>(), sorted), |(_, types)| {
            let types = types.iter().copied().collect::<HashSet<_>>();
            let receiver = Self::channel().1.activate_cloned();
            stream::unfold((receiver, types), |(mut receiver, types)| async move {
                loop {
                    let event = receiver.recv().await.ok()?;
                    if event.asset_ty().is_none_or(|ty| types.contains(ty)) {
                        return Some((event, (receiver, types)));
                    }
                }
            })
        })
    }
}

/// Event broadcast when tags, tag hierarchies, or tag-asset memberships change in [`AssetRegistry`](crate::store::AssetRegistry).
#[derive(Clone, Debug, Eq, Event, PartialEq)]
pub enum TagEvent {
    Created { tag_id: TagId },
    Updated { tag_id: TagId },
    Deleted { tag_id: TagId },
    AssetsChanged { tag_id: TagId },
    ChildrenChanged { parent_id: TagId },
    TypeTagsSynced,
}
