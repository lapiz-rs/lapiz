use std::{fmt, hash::Hash};

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

impl Tag {
    pub fn type_tag_name(asset_ty: &str) -> String {
        format!("{asset_ty}_asset")
    }

    pub fn type_tag_path(asset_ty: &str) -> String {
        format!("type_tags/{}.tag", Self::type_tag_name(asset_ty))
    }

    pub fn is_type_tag(&self) -> bool {
        self.asset_ty.as_ref().is_some_and(|ty| {
            self.name == Self::type_tag_name(ty) && self.relative_path == Self::type_tag_path(ty)
        })
    }
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
}

impl TagFile {
    pub fn new(name: String, asset_ty: Option<String>) -> Self {
        Self {
            id: TagId::new(Uuid::new_v4()),
            name,
            asset_ty,
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

pub type AssetPositionInTag = PositionInTag<UntypedAssetId>;
pub type TagPositionInTag = PositionInTag<TagId>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PositionInTag<T> {
    First,
    #[default]
    Last,
    After(T),
    Before(T),
    Absolute(u32),
}

impl TaggedAssets {
    pub fn insert(&mut self, asset_id: UntypedAssetId, position: AssetPositionInTag) -> Result<()> {
        position.insert_into(&mut self.assets, asset_id)
    }
}

impl<T: Copy + Eq + Hash + fmt::Display> PositionInTag<T> {
    fn insert_into(self, members: &mut IndexSet<T>, member: T) -> Result<()> {
        let position = self;
        let reference = match position {
            PositionInTag::Before(id) | PositionInTag::After(id) => Some(id),
            _ => None,
        };
        if let Some(reference) = reference {
            if !members.contains(&reference) {
                bail!("Reference member {reference} is not assigned to this tag");
            }
            if reference == member {
                return Ok(());
            }
        }

        members.shift_remove(&member);
        let index = match position {
            PositionInTag::First => 0,
            PositionInTag::Last => members.len(),
            PositionInTag::Before(id) => members.get_index_of(&id).unwrap(),
            PositionInTag::After(id) => members.get_index_of(&id).unwrap() + 1,
            PositionInTag::Absolute(index) => (index as usize).min(members.len()),
        };
        members.shift_insert(index, member);
        Ok(())
    }
}

/// Ordered child tags stored beside `name.tag` as `name.tagged_tags`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TaggedTags {
    pub tags: IndexSet<TagId>,
}

pub const TAGGED_TAGS_EXT: &str = "tagged_tags";

impl TaggedTags {
    pub fn insert(&mut self, tag_id: TagId, position: PositionInTag<TagId>) -> Result<()> {
        position.insert_into(&mut self.tags, tag_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::tag::*;

    #[test]
    fn tag_positions_insert_and_move_without_changing_other_members() -> Result<()> {
        let [a, b, c, d] = [1, 2, 3, 4].map(|id| TagId::new(Uuid::from_u128(id)));
        for (tag, position, expected) in [
            (d, PositionInTag::First, vec![d, a, b, c]),
            (d, PositionInTag::Last, vec![a, b, c, d]),
            (d, PositionInTag::Before(b), vec![a, d, b, c]),
            (d, PositionInTag::After(b), vec![a, b, d, c]),
            (d, PositionInTag::Absolute(1), vec![a, d, b, c]),
            (d, PositionInTag::Absolute(u32::MAX), vec![a, b, c, d]),
            (c, PositionInTag::First, vec![c, a, b]),
            (a, PositionInTag::Last, vec![b, c, a]),
            (a, PositionInTag::Before(c), vec![b, a, c]),
            (c, PositionInTag::After(a), vec![a, c, b]),
            (b, PositionInTag::Before(b), vec![a, b, c]),
            (b, PositionInTag::After(b), vec![a, b, c]),
        ] {
            let mut members = TaggedTags {
                tags: IndexSet::from([a, b, c]),
            };
            members.insert(tag, position)?;
            assert_eq!(
                members.tags.into_iter().collect::<Vec<_>>(),
                expected,
                "{position:?}"
            );
        }
        let mut members = TaggedTags {
            tags: IndexSet::from([a, b, c]),
        };
        for position in [PositionInTag::Before(d), PositionInTag::After(d)] {
            assert!(members.insert(a, position).is_err());
            assert_eq!(members.tags, IndexSet::from([a, b, c]));
        }
        let mut members = TaggedTags::default();
        members.insert(a, PositionInTag::default())?;
        assert_eq!(members.tags, IndexSet::from([a]));
        Ok(())
    }

    #[test]
    fn asset_positions_insert_and_move_without_changing_other_members() -> Result<()> {
        let [a, b, c, d] = [1, 2, 3, 4].map(|id| UntypedAssetId::new(Uuid::from_u128(id)));
        for (asset, position, expected) in [
            (d, PositionInTag::First, vec![d, a, b, c]),
            (d, PositionInTag::Last, vec![a, b, c, d]),
            (d, PositionInTag::Before(b), vec![a, d, b, c]),
            (d, PositionInTag::After(b), vec![a, b, d, c]),
            (d, PositionInTag::Absolute(1), vec![a, d, b, c]),
            (d, PositionInTag::Absolute(u32::MAX), vec![a, b, c, d]),
            (c, PositionInTag::First, vec![c, a, b]),
            (a, PositionInTag::Last, vec![b, c, a]),
            (a, PositionInTag::Before(c), vec![b, a, c]),
            (a, PositionInTag::After(b), vec![b, a, c]),
            (c, PositionInTag::Before(a), vec![c, a, b]),
            (c, PositionInTag::After(a), vec![a, c, b]),
            (a, PositionInTag::Absolute(1), vec![b, a, c]),
            (b, PositionInTag::Before(b), vec![a, b, c]),
            (b, PositionInTag::After(b), vec![a, b, c]),
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
            for position in [PositionInTag::Before(d), PositionInTag::After(d)] {
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
            PositionInTag::First,
            PositionInTag::Last,
            PositionInTag::Absolute(0),
            PositionInTag::Absolute(9),
        ] {
            let mut members = TaggedAssets::default();
            members.insert(a, position)?;
            assert_eq!(members.assets.into_iter().collect::<Vec<_>>(), vec![a]);
        }
        assert_eq!(
            PositionInTag::<UntypedAssetId>::default(),
            PositionInTag::Last
        );
        Ok(())
    }
}
