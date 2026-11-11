use std::collections::BTreeSet;

use lapiz_utils::wrapper;
use parse_display::Display;
use rusqlite::types::{self, FromSql, ToSql};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::bundle::BundleId;

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

impl From<Tag> for TagFile {
    fn from(tag: Tag) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
            asset_ty: tag.asset_ty,
        }
    }
}

pub const TAG_EXT: &str = "tag";

/// On-disk sidecar representation (`.tags`) storing tag associations for an asset.
///
/// Persisted as TOML alongside asset files to record the set of [`TagId`]s assigned to that asset.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct AssetTags {
    pub tags: BTreeSet<TagId>,
}

pub const ASSET_TAGS_EXT: &str = "tags";
