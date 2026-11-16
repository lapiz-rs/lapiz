use anyhow::{Result, bail};
use indexmap::IndexSet;
use lapiz_utils::wrapper;
use parse_display::Display;
use rusqlite::types::{self, FromSql, ToSql};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{asset::UntypedAssetId, bundle::BundleId};

wrapper! {
    /// A unique identifier for a [`Tag`].
    ///
    /// Wraps a [`Uuid`] to uniquely identify a categorization tag across bundles and the index database.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Display)]
    #[display("{0}")]
    pub TagId : Uuid
}

impl FromSql for TagId {
    fn column_result(value: types::ValueRef<'_>) -> types::FromSqlResult<Self> {
        Ok(Self(Uuid::column_result(value)?))
    }
}

impl ToSql for TagId {
    fn to_sql(&self) -> rusqlite::Result<types::ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

/// A categorization tag used to organize and query assets.
///
/// Each tag has a unique [`TagId`], a parent [`BundleId`], a relative file path, a display name,
/// and an optional asset type restriction determining which asset types the tag applies to.
#[derive(Debug, Clone)]
pub struct Tag {
    pub id: TagId,
    pub bundle_id: BundleId,
    pub relative_path: String,
    pub name: String,
    pub asset_ty: Option<String>,
}

/// On-disk serialized representation of a tag file (`.tag`).
///
/// Stored as TOML to persist the tag's [`TagId`], human-readable name, and optional target asset type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagFile {
    pub id: TagId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_ty: Option<String>,
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    pub children: IndexSet<TagId>,
}

impl TagFile {
    pub fn new(name: String, asset_ty: Option<String>) -> Self {
        Self {
            id: TagId::new(Uuid::new_v4()),
            name,
            asset_ty,
            children: IndexSet::new(),
        }
    }
}

pub const TAG_EXT: &str = "tag";

/// Ordered asset membership stored beside `name.tag` as `name.tagged_assets`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TaggedAssets {
    pub assets: IndexSet<UntypedAssetId>,
}

pub const TAGGED_ASSETS_EXT: &str = "tagged_assets";

/// A position within a tag's asset list, calculated after removing the asset being moved.
/// References must belong to the tag. Referencing the moved asset leaves its position unchanged.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AssetPosition {
    First,
    #[default]
    Last,
    After(UntypedAssetId),
    Before(UntypedAssetId),
    /// Zero-based index. Positions beyond the end append the asset.
    Absolute(u32),
}

impl TaggedAssets {
    pub(crate) fn insert(
        &mut self,
        asset_id: UntypedAssetId,
        position: AssetPosition,
    ) -> Result<()> {
        let reference = match position {
            AssetPosition::Before(id) | AssetPosition::After(id) => Some(id),
            _ => None,
        };
        if let Some(reference) = reference {
            if !self.assets.contains(&reference) {
                bail!("Reference asset {reference} is not assigned to this tag");
            }
            if reference == asset_id {
                return Ok(());
            }
        }

        self.assets.shift_remove(&asset_id);
        let index = match position {
            AssetPosition::First => 0,
            AssetPosition::Last => self.assets.len(),
            AssetPosition::Before(id) => self.assets.get_index_of(&id).unwrap(),
            AssetPosition::After(id) => self.assets.get_index_of(&id).unwrap() + 1,
            AssetPosition::Absolute(index) => (index as usize).min(self.assets.len()),
        };
        self.assets.shift_insert(index, asset_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_positions_insert_and_move_without_changing_other_members() -> Result<()> {
        let [a, b, c, d] = [1, 2, 3, 4].map(|id| UntypedAssetId::new(Uuid::from_u128(id)));
        for (asset, position, expected) in [
            (d, AssetPosition::First, vec![d, a, b, c]),
            (d, AssetPosition::Last, vec![a, b, c, d]),
            (d, AssetPosition::Before(b), vec![a, d, b, c]),
            (d, AssetPosition::After(b), vec![a, b, d, c]),
            (d, AssetPosition::Absolute(1), vec![a, d, b, c]),
            (d, AssetPosition::Absolute(u32::MAX), vec![a, b, c, d]),
            (c, AssetPosition::First, vec![c, a, b]),
            (a, AssetPosition::Last, vec![b, c, a]),
            (a, AssetPosition::Before(c), vec![b, a, c]),
            (a, AssetPosition::After(b), vec![b, a, c]),
            (c, AssetPosition::Before(a), vec![c, a, b]),
            (c, AssetPosition::After(a), vec![a, c, b]),
            (a, AssetPosition::Absolute(1), vec![b, a, c]),
            (b, AssetPosition::Before(b), vec![a, b, c]),
            (b, AssetPosition::After(b), vec![a, b, c]),
        ] {
            let mut members = TaggedAssets {
                assets: IndexSet::from([a, b, c]),
            };
            members.insert(asset, position)?;
            assert_eq!(
                members.assets.into_iter().collect::<Vec<_>>(),
                expected,
                "{position:?}"
            );
        }
        for asset in [a, d] {
            for position in [AssetPosition::Before(d), AssetPosition::After(d)] {
                let mut members = TaggedAssets {
                    assets: IndexSet::from([a, b, c]),
                };
                assert!(members.insert(asset, position).is_err());
                assert_eq!(
                    members.assets.into_iter().collect::<Vec<_>>(),
                    vec![a, b, c]
                );
            }
        }
        for position in [
            AssetPosition::First,
            AssetPosition::Last,
            AssetPosition::Absolute(0),
            AssetPosition::Absolute(9),
        ] {
            let mut members = TaggedAssets::default();
            members.insert(a, position)?;
            assert_eq!(members.assets.into_iter().collect::<Vec<_>>(), vec![a]);
        }
        assert_eq!(AssetPosition::default(), AssetPosition::Last);
        Ok(())
    }
}
