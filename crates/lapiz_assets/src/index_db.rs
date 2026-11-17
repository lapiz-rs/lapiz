use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::File,
    marker::PhantomData,
    path::Path,
};

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use indexmap::IndexSet;
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension as _, params};
use uuid::Uuid;

use crate::{
    asset::{Asset, AssetMetadata, UntypedAssetId},
    bundle::{AssetBundleMetadata, BundleId, BundleSnapshot},
    tag::{Tag, TagId, TaggedAssets, TaggedTags},
};

/// Synchronization status of an indexed database record after an upsert operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    UpToDate,
    Outdated,
}

/// A strongly-typed filter used to query assets of type `T` from the [`AssetIndexDb`].
///
/// Allows constraining queries by assigned [`TagId`], containing [`BundleId`], and deletion status.
pub struct AssetFilter<T: Asset> {
    tag: Option<TagId>,
    include_descendants: bool,
    bundle: Option<BundleId>,
    is_deleted: bool,
    _marker: PhantomData<T>,
}

impl<T: Asset> Default for AssetFilter<T> {
    fn default() -> Self {
        Self {
            tag: Default::default(),
            include_descendants: false,
            bundle: Default::default(),
            is_deleted: false,
            _marker: PhantomData,
        }
    }
}

impl<T: Asset> AssetFilter<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tag(mut self, tag: TagId) -> Self {
        self.tag = Some(tag);
        self
    }

    pub fn with_bundle(mut self, bundle: BundleId) -> Self {
        self.bundle = Some(bundle);
        self
    }

    /// Include assets assigned to any descendant of the selected tag.
    pub fn with_descendants(mut self, include: bool) -> Self {
        self.include_descendants = include;
        self
    }

    pub fn with_deleted(mut self, is_deleted: bool) -> Self {
        self.is_deleted = is_deleted;
        self
    }

    pub fn into_untyped(self) -> UntypedAssetFilter {
        UntypedAssetFilter {
            ty: Some(T::TYPE_NAME.to_string()),
            tag: self.tag,
            include_descendants: self.include_descendants,
            bundle: self.bundle,
            is_deleted: self.is_deleted,
        }
    }
}

/// Untyped query parameters for searching asset records in the [`AssetIndexDb`].
#[derive(Default)]
pub struct UntypedAssetFilter {
    pub ty: Option<String>,
    pub tag: Option<TagId>,
    pub include_descendants: bool,
    pub bundle: Option<BundleId>,
    pub is_deleted: bool,
}

/// Query parameters for filtering tag records in the [`AssetIndexDb`].
#[derive(Default)]
pub struct TagFilter {
    pub asset_ty: Option<Option<String>>,
    pub is_deleted: bool,
}

/// A SQLite-backed persistent index database for asset and bundle metadata.
///
/// Maintains relational tables for bundles, assets, asset revisions, tags, and asset-tag associations.
///
/// # Responsibilities
///
/// - **Synchronization**: Reconciles scanned bundle snapshots against existing database records upon launch.
/// - **Revision Tracking**: Tracks multi-version revision histories for modified assets.
/// - **Fast Lookups**: Enables indexed filtering and querying of assets by type, bundle, and assigned tags.
/// - **Rollback**: Cleans up uncommitted in-memory revisions upon startup via `revert_all_assets`.
pub struct AssetIndexDb {
    conn: Mutex<Connection>,
}

impl AssetIndexDb {
    pub fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            File::create(path)?;
        }

        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self { conn: conn.into() };
        db.initialize_tables()?;
        db.revert_all_assets()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self { conn: conn.into() };
        db.initialize_tables()?;
        db.revert_all_assets()?;
        Ok(db)
    }

    fn initialize_tables(&self) -> Result<()> {
        let conn = self.conn.lock();

        let tag_columns = {
            let mut statement = conn.prepare("PRAGMA table_info(tags)")?;
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !tag_columns.is_empty()
            && (!tag_columns.iter().any(|column| column == "bundle_id")
                || !tag_columns.iter().any(|column| column == "relative_path"))
        {
            conn.execute_batch(
                r#"
DROP TABLE IF EXISTS tag_children;
DROP TABLE IF EXISTS asset_tags;
DROP TABLE tags;
                "#,
            )?;
        }

        conn.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS bundles (
    bundle_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    last_modified TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS assets (
    asset_id TEXT PRIMARY KEY,
    ty TEXT NOT NULL,
    bundle_id TEXT NOT NULL,
    is_deleted INTEGER NOT NULL,

    FOREIGN KEY (bundle_id) REFERENCES bundles(bundle_id) ON DELETE CASCADE,
    CHECK (is_deleted IN (0, 1))
);

CREATE TABLE IF NOT EXISTS asset_revisions (
    asset_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    relative_path TEXT,
    last_modified TEXT NOT NULL,
    in_memory INTEGER NOT NULL,

    PRIMARY KEY (asset_id, revision),
    FOREIGN KEY (asset_id) REFERENCES assets(asset_id) ON DELETE CASCADE,

    CHECK (
        (in_memory = 1 AND relative_path IS NULL)
        OR
        (in_memory = 0 AND relative_path IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS tags (
    id TEXT PRIMARY KEY,
    bundle_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    name TEXT NOT NULL,
    asset_ty TEXT,
    last_modified TEXT NOT NULL,
    is_deleted INTEGER NOT NULL,

    FOREIGN KEY (bundle_id) REFERENCES bundles(bundle_id) ON DELETE CASCADE,
    UNIQUE (bundle_id, relative_path),
    CHECK (is_deleted IN (0, 1))
);

CREATE TABLE IF NOT EXISTS asset_tags (
    asset_id TEXT NOT NULL,
    tag_id TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (asset_id, tag_id),
    FOREIGN KEY (asset_id) REFERENCES assets(asset_id) ON DELETE CASCADE,
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS tag_children (
    parent_id TEXT NOT NULL,
    child_id TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY (parent_id, child_id),
    FOREIGN KEY (parent_id) REFERENCES tags(id) ON DELETE CASCADE,
    FOREIGN KEY (child_id) REFERENCES tags(id) ON DELETE CASCADE,
    CHECK (parent_id <> child_id)
);

CREATE INDEX IF NOT EXISTS idx_tag_children_child
    ON tag_children(child_id);
            "#,
        )?;

        for table in ["asset_tags", "tag_children"] {
            let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if !columns.iter().any(|column| column == "position") {
                conn.execute_batch(&format!(
                    r#"
ALTER TABLE {table}
ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
                    "#
                ))?;
            }
        }

        Ok(())
    }

    pub(crate) fn sync_bundles(&self, bundles: &[BundleSnapshot]) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        for bundle in bundles {
            tx.execute(
                r#"
INSERT INTO bundles (bundle_id, name, last_modified)
VALUES (?1, ?2, ?3)
ON CONFLICT(bundle_id) DO UPDATE SET
    name = excluded.name,
    last_modified = excluded.last_modified;
                "#,
                params![
                    bundle.metadata.bundle_id,
                    bundle.metadata.name,
                    bundle.metadata.last_modified,
                ],
            )?;

            tx.execute(
                r#"
DELETE FROM asset_revisions
WHERE asset_id IN (
    SELECT asset_id FROM assets WHERE bundle_id = ?1
);
                "#,
                params![bundle.metadata.bundle_id],
            )?;

            let mut scanned_asset_ids = BTreeSet::new();
            for asset in &bundle.assets {
                if scanned_asset_ids.insert(asset.asset_id) {
                    tx.execute(
                        r#"
INSERT INTO assets (asset_id, ty, bundle_id, is_deleted)
VALUES (?1, ?2, ?3, 0)
ON CONFLICT(asset_id) DO UPDATE SET
    ty = excluded.ty,
    bundle_id = excluded.bundle_id;
                        "#,
                        params![asset.asset_id, asset.ty, asset.bundle_id],
                    )?;
                }
                tx.execute(
                    r#"
INSERT INTO asset_revisions (asset_id, revision, relative_path, last_modified, in_memory)
VALUES (?1, ?2, ?3, ?4, ?5);
                    "#,
                    params![
                        asset.asset_id,
                        asset.revision,
                        asset.relative_path,
                        asset.last_modified,
                        asset.in_memory as i64,
                    ],
                )?;
            }

            let stored_asset_ids = {
                let mut statement =
                    tx.prepare("SELECT asset_id FROM assets WHERE bundle_id = ?1")?;
                statement
                    .query_map(params![bundle.metadata.bundle_id], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<UntypedAssetId>>>()?
            };
            for asset_id in stored_asset_ids {
                if !scanned_asset_ids.contains(&asset_id) {
                    tx.execute("DELETE FROM assets WHERE asset_id = ?1", params![asset_id])?;
                }
            }

            let stored_tag_ids = {
                let mut statement = tx.prepare("SELECT id FROM tags WHERE bundle_id = ?1")?;
                statement
                    .query_map(params![bundle.metadata.bundle_id], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<TagId>>>()?
            };
            for tag_id in stored_tag_ids {
                if !bundle.manifest.tags.contains_key(&tag_id) {
                    tx.execute("DELETE FROM tags WHERE id = ?1", params![tag_id])?;
                }
            }
        }

        for bundle in bundles {
            for tag in &bundle.tags {
                let existing_bundle_id = tx
                    .query_row(
                        "SELECT bundle_id FROM tags WHERE id = ?1",
                        params![tag.id],
                        |row| row.get::<_, BundleId>(0),
                    )
                    .optional()?;
                if let Some(existing_bundle_id) = existing_bundle_id
                    && existing_bundle_id != tag.bundle_id
                {
                    bail!(
                        "Tag {} is defined by multiple bundles: {} and {}",
                        tag.id,
                        existing_bundle_id,
                        tag.bundle_id
                    );
                }

                tx.execute(
                    r#"
INSERT INTO tags (
    id, bundle_id, relative_path, name, asset_ty, last_modified, is_deleted
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)
ON CONFLICT(id) DO UPDATE SET
    relative_path = excluded.relative_path,
    name = excluded.name,
    asset_ty = excluded.asset_ty,
    last_modified = excluded.last_modified;
                    "#,
                    params![
                        tag.id,
                        tag.bundle_id,
                        tag.relative_path,
                        tag.name,
                        tag.asset_ty,
                        bundle.metadata.last_modified,
                    ],
                )?;
            }
        }

        for bundle in bundles {
            for (parent_id, children) in &bundle.tag_children {
                replace_tagged_tags(&tx, parent_id, children)?;
            }
        }
        validate_tag_hierarchy(&tx)?;

        for bundle in bundles {
            for (tag_id, tagged_assets) in &bundle.tagged_assets {
                replace_tagged_assets(&tx, tag_id, tagged_assets)?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub(crate) fn remove_unloaded_bundles(
        &self,
        loaded_bundle_ids: &HashSet<BundleId>,
    ) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let stored_bundle_ids = {
            let mut statement = tx.prepare("SELECT bundle_id FROM bundles")?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<BundleId>>>()?
        };
        for bundle_id in stored_bundle_ids {
            if !loaded_bundle_ids.contains(&bundle_id) {
                tx.execute(
                    "DELETE FROM bundles WHERE bundle_id = ?1",
                    params![bundle_id],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_bundle(&self, bundle: &AssetBundleMetadata) -> Result<ItemStatus> {
        let conn = self.conn.lock();
        let result = conn.query_row(
            r#"
INSERT INTO bundles (bundle_id, name, last_modified)
VALUES (?1, ?2, ?3)
ON CONFLICT(bundle_id) DO UPDATE SET
    name = excluded.name,
    last_modified = excluded.last_modified
WHERE bundles.last_modified IS NOT excluded.last_modified
RETURNING 0;
            "#,
            params![bundle.bundle_id, bundle.name, bundle.last_modified,],
            |_| Ok(()),
        );

        match result {
            Ok(_) => Ok(ItemStatus::Outdated),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(ItemStatus::UpToDate),
            Err(e) => Err(e.into()),
        }
    }

    pub fn upsert_tag(&self, tag: &Tag, last_modified: DateTime<Utc>) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        let stored_tag = tx
            .query_row(
                r#"
SELECT bundle_id, asset_ty, last_modified, is_deleted
FROM tags
WHERE id = ?1;
                "#,
                params![tag.id],
                |row| {
                    Ok((
                        row.get::<_, BundleId>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, DateTime<Utc>>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;

        match stored_tag {
            None => {
                tx.execute(
                    r#"
INSERT INTO tags (
    id, bundle_id, relative_path, name, asset_ty, last_modified, is_deleted
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0);
                    "#,
                    params![
                        tag.id,
                        tag.bundle_id,
                        tag.relative_path,
                        tag.name,
                        tag.asset_ty,
                        last_modified,
                    ],
                )?;
            }
            Some((stored_bundle_id, stored_asset_ty, stored_last_modified, is_deleted)) => {
                if stored_bundle_id != tag.bundle_id {
                    bail!(
                        "Tag {} is defined by multiple bundles: {} and {}",
                        tag.id,
                        stored_bundle_id,
                        tag.bundle_id
                    );
                }

                if is_deleted {
                    tx.commit()?;
                    return Ok(());
                }

                if stored_asset_ty != tag.asset_ty {
                    bail!(
                        "Asset type restriction for tag {} cannot be changed from {:?} to {:?}",
                        tag.id,
                        stored_asset_ty,
                        tag.asset_ty
                    );
                }

                if stored_last_modified == last_modified {
                    tx.commit()?;
                    return Ok(());
                }

                tx.execute(
                    r#"
UPDATE tags
SET relative_path = ?2, name = ?3, last_modified = ?4
WHERE id = ?1;
                    "#,
                    params![tag.id, tag.relative_path, tag.name, last_modified],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn get_tag(&self, tag_id: TagId) -> Result<Tag> {
        let conn = self.conn.lock();
        let tag = conn.query_row(
            r#"
SELECT id, bundle_id, relative_path, name, asset_ty
FROM tags
WHERE id = ?1 AND is_deleted = 0;
            "#,
            params![tag_id],
            |row| {
                Ok(Tag {
                    id: row.get(0)?,
                    bundle_id: row.get(1)?,
                    relative_path: row.get(2)?,
                    name: row.get(3)?,
                    asset_ty: row.get(4)?,
                })
            },
        )?;
        Ok(tag)
    }

    pub fn get_tags(&self, filter: TagFilter) -> Result<Vec<Tag>> {
        let (filter_kind, asset_ty) = match filter.asset_ty {
            None => (0, None),
            Some(None) => (1, None),
            Some(Some(asset_ty)) => (2, Some(asset_ty)),
        };

        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
SELECT id, bundle_id, relative_path, name, asset_ty
FROM tags
WHERE is_deleted = ?3
    AND (
        ?1 = 0
        OR (?1 = 1 AND asset_ty IS NULL)
        OR (?1 = 2 AND asset_ty = ?2)
    )
ORDER BY name ASC, id ASC;
            "#,
        )?;
        let rows = stmt.query_map(params![filter_kind, asset_ty, filter.is_deleted], |row| {
            Ok(Tag {
                id: row.get(0)?,
                bundle_id: row.get(1)?,
                relative_path: row.get(2)?,
                name: row.get(3)?,
                asset_ty: row.get(4)?,
            })
        })?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_child_tags(&self, parent_id: &TagId) -> Result<Vec<Tag>> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare(
            r#"
SELECT
    t.id,
    t.bundle_id,
    t.relative_path,
    t.name,
    t.asset_ty
FROM tags t
JOIN tag_children c ON c.child_id = t.id
JOIN tags p ON p.id = c.parent_id
WHERE p.id = ?1
    AND p.is_deleted = 0
    AND t.is_deleted = 0
ORDER BY
    c.position,
    t.id
            "#,
        )?;
        let rows = statement.query_map(params![parent_id], |row| {
            Ok(Tag {
                id: row.get(0)?,
                bundle_id: row.get(1)?,
                relative_path: row.get(2)?,
                name: row.get(3)?,
                asset_ty: row.get(4)?,
            })
        })?;

        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_root_tags(&self) -> Result<Vec<Tag>> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare(
            r#"
SELECT
    t.id,
    t.bundle_id,
    t.relative_path,
    t.name,
    t.asset_ty
FROM tags t
WHERE t.is_deleted = 0 AND NOT EXISTS (
    SELECT 1 FROM tag_children c
    JOIN tags p ON p.id = c.parent_id
    WHERE c.child_id = t.id
        AND p.is_deleted = 0
)
ORDER BY t.name, t.id
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Tag {
                id: row.get(0)?,
                bundle_id: row.get(1)?,
                relative_path: row.get(2)?,
                name: row.get(3)?,
                asset_ty: row.get(4)?,
            })
        })?;

        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_tag(&self, tag: Tag) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
INSERT INTO tags (
    id, bundle_id, relative_path, name, asset_ty, last_modified, is_deleted
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0);
            "#,
            params![
                tag.id,
                tag.bundle_id,
                tag.relative_path,
                tag.name,
                tag.asset_ty,
                Utc::now(),
            ],
        )?;
        Ok(())
    }

    pub fn update_tag(&self, tag: Tag) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            r#"
UPDATE tags
SET relative_path = ?2, name = ?3, asset_ty = ?4, last_modified = ?5
WHERE id = ?1 AND is_deleted = 0;
            "#,
            params![
                tag.id,
                tag.relative_path,
                tag.name,
                tag.asset_ty,
                Utc::now(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_tag(&self, tag_id: &TagId) -> Result<()> {
        let conn = self.conn.lock();
        let deleted = conn.execute(
            "UPDATE tags SET is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
            params![tag_id],
        )?;
        if deleted == 0 {
            bail!("Tag not found for tag ID: {tag_id}");
        }

        Ok(())
    }

    pub fn add_asset(&self, asset: &AssetMetadata) -> Result<UntypedAssetId> {
        let asset_id = asset.asset_id;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        tx.execute(
            r#"
DELETE FROM asset_revisions
WHERE asset_id = ?1
    AND EXISTS (
        SELECT 1 FROM assets
        WHERE asset_id = ?1 AND is_deleted = 1
    );
            "#,
            params![asset.asset_id],
        )?;

        tx.execute(
            r#"
INSERT INTO assets (asset_id, ty, bundle_id, is_deleted)
VALUES (?1, ?2, ?3, 0)
ON CONFLICT(asset_id) DO UPDATE SET
    ty = excluded.ty,
    bundle_id = excluded.bundle_id,
    is_deleted = 0;
            "#,
            params![asset.asset_id, asset.ty, asset.bundle_id,],
        )?;

        tx.execute(
            r#"
INSERT INTO asset_revisions (asset_id, revision, relative_path, last_modified, in_memory)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(asset_id, revision) DO UPDATE SET
    relative_path = excluded.relative_path,
    last_modified = excluded.last_modified,
    in_memory = excluded.in_memory;
            "#,
            params![
                asset.asset_id,
                asset.revision,
                asset.relative_path,
                asset.last_modified,
                asset.in_memory as i64,
            ],
        )?;

        tx.commit()?;
        Ok(asset_id)
    }

    pub fn get_asset(&self, id: &UntypedAssetId) -> Result<AssetMetadata> {
        let conn = self.conn.lock();
        let asset = conn.query_row(
            r#"
SELECT
    r.asset_id,
    a.ty,
    a.bundle_id,
    r.relative_path,
    r.revision,
    r.last_modified,
    r.in_memory
FROM asset_revisions r
JOIN assets a USING (asset_id)
WHERE r.asset_id = ?1
    AND a.is_deleted = 0
ORDER BY r.revision DESC
LIMIT 1;
            "#,
            params![id],
            |row| {
                Ok(AssetMetadata {
                    asset_id: row.get(0)?,
                    ty: row.get(1)?,
                    bundle_id: row.get(2)?,
                    relative_path: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    revision: row.get(4)?,
                    last_modified: row.get(5)?,
                    in_memory: row.get::<_, i64>(6)? == 1,
                })
            },
        )?;

        Ok(asset)
    }

    pub fn get_assets(&self, filter: UntypedAssetFilter) -> Result<Vec<AssetMetadata>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
WITH RECURSIVE selected_tags(id) AS (
    SELECT id FROM tags WHERE id = ?2 AND (is_deleted = 0 OR ?4 = 1)
    UNION
    SELECT c.child_id FROM tag_children c
    JOIN selected_tags s ON c.parent_id = s.id
    JOIN tags t ON t.id = c.child_id
    WHERE ?5 = 1 AND (t.is_deleted = 0 OR ?4 = 1)
), latest AS (
    SELECT
        r.asset_id,
        r.relative_path,
        r.revision,
        r.last_modified,
        r.in_memory,
        ROW_NUMBER() OVER (PARTITION BY r.asset_id ORDER BY r.revision DESC) AS ord
    FROM asset_revisions r
)
SELECT
    l.asset_id,
    a.ty,
    a.bundle_id,
    l.relative_path,
    l.revision,
    l.last_modified,
    l.in_memory
FROM latest l
JOIN assets a ON a.asset_id = l.asset_id
WHERE l.ord = 1
    AND a.is_deleted = ?4
    AND (?1 IS NULL OR a.ty = ?1)
    AND (
        ?2 IS NULL
        OR EXISTS (
            SELECT 1
            FROM asset_tags at
            WHERE at.asset_id = a.asset_id
                AND at.tag_id IN (SELECT id FROM selected_tags)
        )
    )
    AND (?3 IS NULL OR a.bundle_id = ?3)
ORDER BY
    CASE WHEN ?5 = 0 THEN (
        SELECT at.position
        FROM asset_tags at
        WHERE at.asset_id = a.asset_id
            AND at.tag_id = ?2
    ) END ASC,
    l.relative_path ASC;
            "#,
        )?;

        let rows = stmt.query_map(
            params![
                filter.ty,
                filter.tag.as_ref(),
                filter.bundle.as_ref(),
                filter.is_deleted,
                filter.include_descendants,
            ],
            |row| {
                Ok(AssetMetadata {
                    asset_id: row.get(0)?,
                    ty: row.get(1)?,
                    bundle_id: row.get(2)?,
                    relative_path: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    revision: row.get(4)?,
                    last_modified: row.get(5)?,
                    in_memory: row.get::<_, i64>(6)? == 1,
                })
            },
        )?;

        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn update_asset(&self, id: &UntypedAssetId) -> Result<u32> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let (revision, in_memory) = tx.query_row(
            r#"
SELECT r.revision, r.in_memory
FROM asset_revisions r
JOIN assets a USING (asset_id)
WHERE r.asset_id = ?1
    AND a.is_deleted = 0
ORDER BY r.revision DESC
LIMIT 1;
            "#,
            params![id],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, i64>(1)? == 1)),
        )?;

        if in_memory {
            tx.commit()?;
            return Ok(revision);
        }

        let revision = tx.query_one(
            r#"
INSERT INTO asset_revisions (
    asset_id,
    relative_path,
    revision,
    last_modified,
    in_memory
)
VALUES (?1, NULL, ?2, ?3, 1)
RETURNING revision;
            "#,
            params![id, revision + 1, Utc::now()],
            |row| row.get::<_, u32>(0),
        )?;

        tx.commit()?;
        Ok(revision)
    }

    pub fn write_asset(
        &self,
        id: &UntypedAssetId,
        new_path: &str,
        last_modified: DateTime<Utc>,
    ) -> Result<u32> {
        let conn = self.conn.lock();
        let revision = conn.query_row(
            r#"
WITH latest AS (
    SELECT r.revision, r.in_memory
    FROM asset_revisions r
    JOIN assets a USING (asset_id)
    WHERE r.asset_id = ?1
      AND a.is_deleted = 0
    ORDER BY r.revision DESC
    LIMIT 1
)
UPDATE asset_revisions
SET in_memory = 0, relative_path = ?2, last_modified = ?3
WHERE asset_id = ?1
  AND revision = (SELECT revision FROM latest)
  AND (SELECT in_memory FROM latest) = 1
RETURNING revision;
            "#,
            params![id, new_path, last_modified],
            |row| row.get::<_, u32>(0),
        )?;

        Ok(revision)
    }

    pub fn delete_asset(&self, asset_id: &UntypedAssetId) -> Result<()> {
        let conn = self.conn.lock();
        let deleted = conn.execute(
            "UPDATE assets SET is_deleted = 1 WHERE asset_id = ?1 AND is_deleted = 0",
            params![asset_id],
        )?;
        if deleted == 0 {
            bail!("Asset not found for asset ID: {asset_id}");
        }

        Ok(())
    }

    pub fn restore_tag(&self, tag_id: &TagId) -> Result<()> {
        let conn = self.conn.lock();
        let restored = conn.execute(
            "UPDATE tags SET is_deleted = 0 WHERE id = ?1 AND is_deleted = 1",
            params![tag_id],
        )?;
        if restored == 0 {
            bail!("Tag not found for tag ID: {tag_id}");
        }

        Ok(())
    }

    pub fn restore_asset(&self, asset_id: &UntypedAssetId) -> Result<()> {
        let conn = self.conn.lock();
        let restored = conn.execute(
            "UPDATE assets SET is_deleted = 0 WHERE asset_id = ?1 AND is_deleted = 1",
            params![asset_id],
        )?;
        if restored == 0 {
            bail!("Asset not found for asset ID: {asset_id}");
        }

        Ok(())
    }

    pub(crate) fn validate_tagged_tags(&self, tag_id: &TagId, tags: &TaggedTags) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        update_tagged_tags(&tx, tag_id, tags)?;
        tx.rollback()?;
        Ok(())
    }

    pub fn edit_tagged_tags(&self, tag_id: &TagId, tags: &TaggedTags) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        update_tagged_tags(&tx, tag_id, tags)?;
        tx.commit()?;
        Ok(())
    }

    pub fn edit_tagged_assets(&self, tag_id: &TagId, assets: &TaggedAssets) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        let tag_exists = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE id = ?1 AND is_deleted = 0)",
            params![tag_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !tag_exists {
            bail!("Tag not found for tag ID: {tag_id}");
        }

        let mut statement = tx.prepare(
            "SELECT EXISTS(SELECT 1 FROM assets WHERE asset_id = ?1 AND is_deleted = 1)",
        )?;
        for asset_id in &assets.assets {
            if statement.query_row(params![asset_id], |row| row.get::<_, bool>(0))? {
                bail!("Asset not found for asset ID: {asset_id}");
            }
        }
        drop(statement);

        replace_tagged_assets(&tx, tag_id, assets)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_asset_tags(&self, asset_id: &UntypedAssetId) -> Result<IndexSet<TagId>> {
        let conn = self.conn.lock();
        let mut statement = conn.prepare(
            r#"
SELECT t.id
FROM tags t
JOIN asset_tags at ON at.tag_id = t.id
JOIN assets a ON a.asset_id = at.asset_id
WHERE at.asset_id = ?1
    AND t.is_deleted = 0
    AND a.is_deleted = 0
ORDER BY
    t.name,
    t.id;
            "#,
        )?;
        Ok(statement
            .query_map(params![asset_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn revert_asset(&self, id: &Uuid) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM asset_revisions WHERE in_memory = 1 AND asset_id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn revert_all_assets(&self) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM asset_revisions WHERE in_memory = 1", [])?;
        Ok(())
    }

    pub fn get_bundle(&self, id: &BundleId) -> Result<AssetBundleMetadata> {
        let conn = self.conn.lock();
        let bundle = conn.query_row(
            r#"
SELECT bundle_id, name, last_modified
FROM bundles
WHERE bundle_id = ?1;
            "#,
            params![id],
            |row| {
                Ok(AssetBundleMetadata {
                    bundle_id: row.get(0)?,
                    name: row.get(1)?,
                    last_modified: row.get(2)?,
                })
            },
        )?;

        Ok(bundle)
    }
}

fn update_tagged_tags(conn: &Connection, tag_id: &TagId, tags: &TaggedTags) -> Result<()> {
    let exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM tags WHERE id = ?1 AND is_deleted = 0)",
        params![tag_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        bail!("Tag not found for tag ID: {tag_id}");
    }

    for child_id in &tags.tags {
        let deleted = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE id = ?1 AND is_deleted = 1)",
            params![child_id],
            |row| row.get::<_, bool>(0),
        )?;
        if deleted {
            bail!("Tag not found for tag ID: {child_id}");
        }
    }

    replace_tagged_tags(conn, tag_id, &tags.tags)?;
    validate_tag_hierarchy(conn)
}

fn replace_tagged_tags(conn: &Connection, tag_id: &TagId, tags: &IndexSet<TagId>) -> Result<()> {
    conn.execute(
        "DELETE FROM tag_children WHERE parent_id = ?1",
        params![tag_id],
    )?;
    let mut insert = conn.prepare(
        r#"
INSERT INTO tag_children (parent_id, child_id, position)
SELECT ?1, ?2, ?3
WHERE EXISTS (SELECT 1 FROM tags WHERE id = ?2);
        "#,
    )?;
    for (position, child_id) in tags.iter().enumerate() {
        if insert.execute(params![tag_id, child_id, position as i64])? == 0 {
            log::warn!("Skipped missing child tag {child_id} for parent tag {tag_id}");
        }
    }
    Ok(())
}

fn replace_tagged_assets(conn: &Connection, tag_id: &TagId, assets: &TaggedAssets) -> Result<()> {
    conn.execute(
        r#"
DELETE FROM asset_tags
WHERE tag_id = ?1;
        "#,
        params![tag_id],
    )?;
    let mut insert = conn.prepare(
        r#"
INSERT INTO asset_tags (asset_id, tag_id, position)
SELECT ?1, ?2, ?3
WHERE EXISTS (
    SELECT 1
    FROM assets
    WHERE asset_id = ?1
);
        "#,
    )?;
    for (position, asset_id) in assets.assets.iter().enumerate() {
        if insert.execute(params![asset_id, tag_id, position as i64])? == 0 {
            log::warn!("Skipped missing asset {asset_id} for tag {tag_id}");
        }
    }
    Ok(())
}

fn validate_tag_hierarchy(conn: &Connection) -> Result<()> {
    let mut children = HashMap::<TagId, Vec<TagId>>::new();
    let mut indegree = HashMap::<TagId, usize>::new();
    let mut statement = conn.prepare("SELECT parent_id, child_id FROM tag_children")?;
    let edges = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    for edge in edges {
        let (parent, child) = edge?;
        children.entry(parent).or_default().push(child);
        indegree.entry(parent).or_default();
        *indegree.entry(child).or_default() += 1;
    }
    let mut roots = indegree
        .iter()
        .filter_map(|(&id, &degree)| (degree == 0).then_some(id))
        .collect::<Vec<_>>();
    let mut visited = 0;
    while let Some(parent) = roots.pop() {
        visited += 1;
        if let Some(children) = children.get(&parent) {
            for child in children {
                let degree = indegree
                    .get_mut(child)
                    .expect("every child has an indegree");
                *degree -= 1;
                if *degree == 0 {
                    roots.push(*child);
                }
            }
        }
    }
    if visited != indegree.len() {
        bail!("Tag hierarchy contains a cycle");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    struct TestAsset;

    impl Asset for TestAsset {
        const TYPE_NAME: &'static str = "test_asset";
    }

    struct OtherAsset;

    impl Asset for OtherAsset {
        const TYPE_NAME: &'static str = "other_asset";
    }

    fn sourced_tag(
        bundle_id: BundleId,
        relative_path: &str,
        name: &str,
        asset_ty: Option<String>,
    ) -> Tag {
        Tag {
            id: TagId::new(Uuid::new_v4()),
            bundle_id,
            relative_path: relative_path.to_string(),
            name: name.to_string(),
            asset_ty,
        }
    }

    #[test]
    fn legacy_relation_tables_gain_positions_without_losing_rows() -> Result<()> {
        let (db, bundle_id, [root, left, right, _]) = hierarchy_db()?;
        let asset_id = UntypedAssetId::new(Uuid::new_v4());
        db.add_asset(&AssetMetadata {
            asset_id,
            ty: TestAsset::TYPE_NAME.into(),
            bundle_id,
            relative_path: "sample.asset".into(),
            revision: 0,
            last_modified: Utc::now(),
            in_memory: false,
        })?;
        add_indexed_member(&db, &asset_id, &root)?;
        db.conn.lock().execute_batch(
            r#"
ALTER TABLE asset_tags
DROP COLUMN position;

ALTER TABLE tag_children
DROP COLUMN position;
            "#,
        )?;
        db.initialize_tables()?;
        db.initialize_tables()?;
        assert!(db.get_asset_tags(&asset_id)?.contains(&root));
        assert_eq!(db.get_child_tags(&root)?.len(), 2);
        db.edit_tagged_tags(
            &root,
            &TaggedTags {
                tags: IndexSet::from([right, left]),
            },
        )?;
        assert_eq!(
            db.get_child_tags(&root)?
                .into_iter()
                .map(|tag| tag.id)
                .collect::<Vec<_>>(),
            vec![right, left]
        );
        Ok(())
    }

    #[test]
    fn bundle_upsert_and_get() -> Result<()> {
        let db = AssetIndexDb::open_in_memory()?;
        let bundle_id = BundleId::new(Uuid::from_u128(1));
        let mut bundle = AssetBundleMetadata {
            bundle_id,
            name: "Original".to_string(),
            last_modified: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        };

        assert_eq!(db.upsert_bundle(&bundle)?, ItemStatus::Outdated);

        let stored = db.get_bundle(&bundle_id)?;
        assert_eq!(stored.bundle_id, bundle_id);
        assert_eq!(stored.name, "Original");
        assert_eq!(stored.last_modified, bundle.last_modified);

        bundle.name = "Ignored".to_string();
        assert_eq!(db.upsert_bundle(&bundle)?, ItemStatus::UpToDate);
        assert_eq!(db.get_bundle(&bundle_id)?.name, "Original");

        bundle.name = "Updated".to_string();
        bundle.last_modified = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
        assert_eq!(db.upsert_bundle(&bundle)?, ItemStatus::Outdated);

        let stored = db.get_bundle(&bundle_id)?;
        assert_eq!(stored.name, "Updated");
        assert_eq!(stored.last_modified, bundle.last_modified);

        Ok(())
    }

    fn add_indexed_member(
        db: &AssetIndexDb,
        asset_id: &UntypedAssetId,
        tag_id: &TagId,
    ) -> Result<()> {
        let mut assets = TaggedAssets {
            assets: db
                .get_assets(UntypedAssetFilter {
                    tag: Some(*tag_id),
                    ..Default::default()
                })?
                .into_iter()
                .map(|asset| asset.asset_id)
                .collect(),
        };
        assets.assets.insert(*asset_id);
        db.edit_tagged_assets(tag_id, &assets)
    }

    fn hierarchy_db() -> Result<(AssetIndexDb, BundleId, [TagId; 4])> {
        let db = AssetIndexDb::open_in_memory()?;
        let bundle_id = BundleId::new(Uuid::new_v4());
        db.upsert_bundle(&AssetBundleMetadata {
            bundle_id,
            name: "Hierarchy".into(),
            last_modified: Utc::now(),
        })?;
        let tags = ["root", "left", "right", "leaf"]
            .map(|name| sourced_tag(bundle_id, &format!("{name}.tag"), name, None));
        for tag in &tags {
            db.add_tag(tag.clone())?;
        }
        let ids = tags.map(|tag| tag.id);
        for (parent, children) in [
            (0, vec![ids[1], ids[2]]),
            (1, vec![ids[3]]),
            (2, vec![ids[3]]),
        ] {
            db.edit_tagged_tags(
                &ids[parent],
                &TaggedTags {
                    tags: children.into_iter().collect(),
                },
            )?;
        }
        Ok((db, bundle_id, ids))
    }

    #[test]
    fn topological_check_handles_long_chains_and_disconnected_cycles() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("CREATE TABLE tag_children (parent_id TEXT, child_id TEXT);")?;
        validate_tag_hierarchy(&conn)?;
        let ids = (1..=10_000)
            .map(|id| TagId::new(Uuid::from_u128(id)))
            .collect::<Vec<_>>();
        let tx = conn.transaction()?;
        {
            let mut insert = tx.prepare("INSERT INTO tag_children VALUES (?1, ?2)")?;
            for pair in ids.windows(2) {
                insert.execute(params![pair[0], pair[1]])?;
            }
        }
        tx.commit()?;
        validate_tag_hierarchy(&conn)?;
        // A separate cyclic component must be detected even after the long chain
        // has been processed successfully.
        let a = TagId::new(Uuid::from_u128(10_001));
        let b = TagId::new(Uuid::from_u128(10_002));
        conn.execute(
            "INSERT INTO tag_children VALUES (?1, ?2), (?2, ?1)",
            params![a, b],
        )?;
        assert!(
            validate_tag_hierarchy(&conn)
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );
        Ok(())
    }

    #[test]
    fn tag_relationships_support_shared_children_and_soft_deletion() -> Result<()> {
        let (db, _, [root, left, right, leaf]) = hierarchy_db()?;
        assert_eq!(db.get_root_tags()?[0].id, root);
        assert_eq!(db.get_child_tags(&root)?.len(), 2);
        assert_eq!(db.get_child_tags(&left)?[0].id, leaf);
        assert_eq!(db.get_child_tags(&right)?[0].id, leaf);
        assert!(
            db.edit_tagged_tags(
                &root,
                &TaggedTags {
                    tags: IndexSet::from([root])
                }
            )
            .is_err()
        );
        assert_eq!(db.get_child_tags(&root)?.len(), 2);
        assert!(
            db.edit_tagged_tags(&TagId::new(Uuid::new_v4()), &TaggedTags::default())
                .is_err()
        );

        db.delete_tag(&left)?;
        db.delete_tag(&right)?;
        assert!(db.get_child_tags(&left)?.is_empty());
        assert!(db.get_child_tags(&right)?.is_empty());
        assert_eq!(db.get_root_tags()?.len(), 2);
        // Hidden paths still count when checking cycles for future restoration.
        assert!(
            db.edit_tagged_tags(
                &leaf,
                &TaggedTags {
                    tags: IndexSet::from([root])
                }
            )
            .is_err()
        );
        assert!(
            db.edit_tagged_tags(
                &right,
                &TaggedTags {
                    tags: IndexSet::from([leaf])
                }
            )
            .is_err()
        );
        db.restore_tag(&left)?;
        db.restore_tag(&right)?;
        assert_eq!(db.get_root_tags()?.len(), 1);
        db.edit_tagged_tags(&left, &TaggedTags::default())?;
        assert!(db.get_child_tags(&left)?.is_empty());
        assert_eq!(db.get_child_tags(&right)?[0].id, leaf);
        db.edit_tagged_tags(&right, &TaggedTags::default())?;
        assert_eq!(db.get_root_tags()?.len(), 2);
        Ok(())
    }

    #[test]
    fn descendant_assets_are_deduplicated_and_keep_existing_filters() -> Result<()> {
        let (db, bundle_id, [root, left, right, leaf]) = hierarchy_db()?;
        let other_bundle = BundleId::new(Uuid::new_v4());
        db.upsert_bundle(&AssetBundleMetadata {
            bundle_id: other_bundle,
            name: "Other".into(),
            last_modified: Utc::now(),
        })?;
        let ids = [0, 1, 2, 3].map(|_| UntypedAssetId::new(Uuid::new_v4()));
        for (index, id) in ids.iter().enumerate() {
            db.add_asset(&AssetMetadata {
                asset_id: *id,
                ty: if index == 2 {
                    OtherAsset::TYPE_NAME
                } else {
                    TestAsset::TYPE_NAME
                }
                .into(),
                bundle_id: if index == 3 { other_bundle } else { bundle_id },
                relative_path: format!("{index}.asset"),
                revision: 0,
                last_modified: Utc::now(),
                in_memory: false,
            })?;
            add_indexed_member(&db, id, if index == 0 { &root } else { &leaf })?;
        }
        add_indexed_member(&db, &ids[1], &right)?;
        let query = || {
            AssetFilter::<TestAsset>::new()
                .with_tag(root)
                .with_descendants(true)
        };
        assert_eq!(
            db.get_assets(
                AssetFilter::<TestAsset>::new()
                    .with_tag(root)
                    .into_untyped()
            )?
            .len(),
            1
        );
        assert_eq!(db.get_assets(query().into_untyped())?.len(), 3);
        assert_eq!(
            db.get_assets(query().with_bundle(bundle_id).into_untyped())?
                .len(),
            2
        );
        // The second path still reaches the leaf when the left branch is hidden.
        db.delete_tag(&left)?;
        assert_eq!(db.get_assets(query().into_untyped())?.len(), 3);
        db.delete_tag(&right)?;
        assert_eq!(db.get_assets(query().into_untyped())?.len(), 1);
        db.restore_tag(&left)?;
        assert_eq!(db.get_assets(query().into_untyped())?.len(), 3);
        db.delete_asset(&ids[1])?;
        db.delete_tag(&root)?;
        assert!(db.get_assets(query().into_untyped())?.is_empty());
        let deleted = db.get_assets(query().with_deleted(true).into_untyped())?;
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].asset_id, ids[1]);
        Ok(())
    }

    #[test]
    fn add_get_and_filter_assets() -> Result<()> {
        let db = AssetIndexDb::open_in_memory()?;
        let first_bundle_id = BundleId::new(Uuid::from_u128(10));
        let second_bundle_id = BundleId::new(Uuid::from_u128(11));
        let last_modified = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();

        for bundle in [
            AssetBundleMetadata {
                bundle_id: first_bundle_id,
                name: "First".to_string(),
                last_modified,
            },
            AssetBundleMetadata {
                bundle_id: second_bundle_id,
                name: "Second".to_string(),
                last_modified,
            },
        ] {
            db.upsert_bundle(&bundle)?;
        }

        let first_id = UntypedAssetId::new(Uuid::from_u128(20));
        let second_id = UntypedAssetId::new(Uuid::from_u128(21));
        let third_id = UntypedAssetId::new(Uuid::from_u128(22));
        let first = AssetMetadata {
            asset_id: first_id,
            ty: TestAsset::TYPE_NAME.to_string(),
            bundle_id: first_bundle_id,
            relative_path: "zeta.asset".to_string(),
            revision: 0,
            last_modified,
            in_memory: false,
        };
        let second = AssetMetadata {
            asset_id: second_id,
            ty: OtherAsset::TYPE_NAME.to_string(),
            bundle_id: first_bundle_id,
            relative_path: "alpha.asset".to_string(),
            revision: 0,
            last_modified,
            in_memory: false,
        };
        let third = AssetMetadata {
            asset_id: third_id,
            ty: TestAsset::TYPE_NAME.to_string(),
            bundle_id: second_bundle_id,
            relative_path: "middle.asset".to_string(),
            revision: 4,
            last_modified,
            in_memory: false,
        };

        assert_eq!(db.add_asset(&first)?, first_id);
        assert_eq!(db.add_asset(&second)?, second_id);
        assert_eq!(db.add_asset(&third)?, third_id);

        let stored = db.get_asset(&third_id)?;
        assert_eq!(stored.asset_id, third_id);
        assert_eq!(stored.ty, TestAsset::TYPE_NAME);
        assert_eq!(stored.bundle_id, second_bundle_id);
        assert_eq!(stored.relative_path, "middle.asset");
        assert_eq!(stored.revision, 4);
        assert_eq!(stored.last_modified, last_modified);
        assert!(!stored.in_memory);

        let all = db.get_assets(UntypedAssetFilter::default())?;
        assert_eq!(
            all.iter().map(|asset| asset.asset_id).collect::<Vec<_>>(),
            vec![second_id, third_id, first_id]
        );

        let typed = db.get_assets(AssetFilter::<TestAsset>::new().into_untyped())?;
        assert_eq!(
            typed.iter().map(|asset| asset.asset_id).collect::<Vec<_>>(),
            vec![third_id, first_id]
        );

        let in_first_bundle = db.get_assets(
            AssetFilter::<TestAsset>::new()
                .with_bundle(first_bundle_id)
                .into_untyped(),
        )?;
        assert_eq!(in_first_bundle.len(), 1);
        assert_eq!(in_first_bundle[0].asset_id, first_id);

        let untyped_in_first_bundle = db.get_assets(UntypedAssetFilter {
            bundle: Some(first_bundle_id),
            ..Default::default()
        })?;
        assert_eq!(
            untyped_in_first_bundle
                .iter()
                .map(|asset| asset.asset_id)
                .collect::<Vec<_>>(),
            vec![second_id, first_id]
        );

        Ok(())
    }

    #[test]
    fn tags_are_upserted_queried_and_filtered() -> Result<()> {
        let db = AssetIndexDb::open_in_memory()?;
        let last_modified = Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
        let bundle_id = BundleId::new(Uuid::from_u128(60));
        db.upsert_bundle(&AssetBundleMetadata {
            bundle_id,
            name: "Tags".to_string(),
            last_modified,
        })?;
        let mut tag = sourced_tag(
            bundle_id,
            "original.tag",
            "Original",
            Some(TestAsset::TYPE_NAME.to_string()),
        );

        db.upsert_tag(&tag, last_modified)?;

        let stored = db.conn.lock().query_row(
            "SELECT name, asset_ty, last_modified, is_deleted FROM tags WHERE id = ?1",
            params![tag.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, DateTime<Utc>>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )?;
        assert_eq!(stored.0, "Original");
        assert_eq!(stored.1.as_deref(), Some(TestAsset::TYPE_NAME));
        assert_eq!(stored.2, last_modified);
        assert!(!stored.3);

        tag.name = "Ignored".to_string();
        db.upsert_tag(&tag, last_modified)?;
        let stored_name = db.conn.lock().query_row(
            "SELECT name FROM tags WHERE id = ?1",
            params![tag.id],
            |row| row.get::<_, String>(0),
        )?;
        assert_eq!(stored_name, "Original");

        let updated_at = Utc.with_ymd_and_hms(2026, 4, 2, 0, 0, 0).unwrap();
        db.upsert_tag(&tag, updated_at)?;
        let stored = db.conn.lock().query_row(
            "SELECT name, asset_ty, last_modified FROM tags WHERE id = ?1",
            params![tag.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, DateTime<Utc>>(2)?,
                ))
            },
        )?;
        assert_eq!(stored.0, "Ignored");
        assert_eq!(stored.1.as_deref(), Some(TestAsset::TYPE_NAME));
        assert_eq!(stored.2, updated_at);

        let changed_asset_ty_tag = Tag {
            name: "Changed type".to_string(),
            asset_ty: Some(OtherAsset::TYPE_NAME.to_string()),
            ..tag.clone()
        };
        assert!(
            db.upsert_tag(
                &changed_asset_ty_tag,
                Utc.with_ymd_and_hms(2026, 4, 3, 0, 0, 0).unwrap(),
            )
            .is_err()
        );

        let stored = db.conn.lock().query_row(
            "SELECT name, asset_ty, last_modified FROM tags WHERE id = ?1",
            params![tag.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, DateTime<Utc>>(2)?,
                ))
            },
        )?;
        assert_eq!(stored.0, "Ignored");
        assert_eq!(stored.1.as_deref(), Some(TestAsset::TYPE_NAME));
        assert_eq!(stored.2, updated_at);

        let untyped_tag = sourced_tag(bundle_id, "all.tag", "All assets", None);
        let other_asset_tag = sourced_tag(
            bundle_id,
            "other.tag",
            "Other assets",
            Some(OtherAsset::TYPE_NAME.to_string()),
        );
        db.upsert_tag(&untyped_tag, updated_at)?;
        db.upsert_tag(&other_asset_tag, updated_at)?;

        let queried = db.get_tag(tag.id)?;
        assert_eq!(queried.id, tag.id);
        assert_eq!(queried.name, "Ignored");
        assert_eq!(queried.asset_ty.as_deref(), Some(TestAsset::TYPE_NAME));

        assert_eq!(
            db.get_tags(TagFilter::default())?
                .iter()
                .map(|tag| tag.name.as_str())
                .collect::<Vec<_>>(),
            vec!["All assets", "Ignored", "Other assets"]
        );

        let untyped = db.get_tags(TagFilter {
            asset_ty: Some(None),
            ..Default::default()
        })?;
        assert_eq!(untyped.len(), 1);
        assert_eq!(untyped[0].id, untyped_tag.id);

        let typed = db.get_tags(TagFilter {
            asset_ty: Some(Some(TestAsset::TYPE_NAME.to_string())),
            ..Default::default()
        })?;
        assert_eq!(typed.len(), 1);
        assert_eq!(typed[0].id, tag.id);

        assert!(
            db.get_tags(TagFilter {
                asset_ty: Some(Some("missing_asset".to_string())),
                ..Default::default()
            })?
            .is_empty()
        );

        Ok(())
    }

    #[test]
    fn tagged_tag_edits_validate_and_roll_back_without_losing_order() -> Result<()> {
        let (db, _, [root, left, right, _]) = hierarchy_db()?;
        let members = TaggedTags {
            tags: IndexSet::from([right, left]),
        };
        db.validate_tagged_tags(&root, &members)?;
        assert_eq!(
            db.get_child_tags(&root)?
                .into_iter()
                .map(|tag| tag.id)
                .collect::<Vec<_>>(),
            vec![left, right]
        );
        db.edit_tagged_tags(&root, &members)?;
        db.conn.lock().execute_batch(
            r#"
CREATE TEMP TRIGGER fail_tagged_tag_insert
BEFORE INSERT ON tag_children
BEGIN
    SELECT RAISE(ABORT, 'write failed');
END;
            "#,
        )?;
        assert!(
            db.edit_tagged_tags(
                &root,
                &TaggedTags {
                    tags: IndexSet::from([left])
                }
            )
            .unwrap_err()
            .to_string()
            .contains("write failed")
        );
        assert_eq!(
            db.get_child_tags(&root)?
                .into_iter()
                .map(|tag| tag.id)
                .collect::<Vec<_>>(),
            vec![right, left]
        );
        db.conn
            .lock()
            .execute_batch("DROP TRIGGER fail_tagged_tag_insert;")?;
        let missing = TagId::new(Uuid::new_v4());
        db.edit_tagged_tags(
            &root,
            &TaggedTags {
                tags: IndexSet::from([missing, left]),
            },
        )?;
        assert_eq!(
            db.get_child_tags(&root)?
                .into_iter()
                .map(|tag| tag.id)
                .collect::<Vec<_>>(),
            vec![left]
        );
        db.delete_tag(&left)?;
        assert!(
            db.edit_tagged_tags(
                &root,
                &TaggedTags {
                    tags: IndexSet::from([left])
                }
            )
            .is_err()
        );
        db.restore_tag(&left)?;
        assert_eq!(db.get_child_tags(&root)?[0].id, left);
        db.edit_tagged_tags(&root, &TaggedTags::default())?;
        assert!(db.get_child_tags(&root)?.is_empty());
        Ok(())
    }

    #[test]
    fn tagged_asset_edits_roll_back_on_persistence_failure() -> Result<()> {
        let (db, bundle_id, [tag_id, _, _, _]) = hierarchy_db()?;
        let asset_id = UntypedAssetId::new(Uuid::new_v4());
        db.add_asset(&AssetMetadata {
            asset_id,
            ty: TestAsset::TYPE_NAME.into(),
            bundle_id,
            relative_path: "asset.test".into(),
            revision: 0,
            last_modified: Utc::now(),
            in_memory: false,
        })?;
        let members = TaggedAssets {
            assets: IndexSet::from([asset_id]),
        };
        db.edit_tagged_assets(&tag_id, &members)?;
        db.conn.lock().execute_batch(
            r#"
CREATE TEMP TRIGGER fail_tagged_asset_insert
BEFORE INSERT ON asset_tags
BEGIN
    SELECT RAISE(ABORT, 'write failed');
END;
            "#,
        )?;
        let error = db.edit_tagged_assets(&tag_id, &members).unwrap_err();
        assert!(error.to_string().contains("write failed"));
        assert!(db.get_asset_tags(&asset_id)?.contains(&tag_id));
        db.conn
            .lock()
            .execute_batch("DROP TRIGGER fail_tagged_asset_insert;")?;
        db.edit_tagged_assets(&tag_id, &members)?;
        assert_eq!(
            db.get_asset_tags(&asset_id)?
                .into_iter()
                .collect::<Vec<_>>(),
            vec![tag_id]
        );
        db.delete_tag(&tag_id)?;
        assert!(db.get_asset_tags(&asset_id)?.is_empty());
        assert!(db.edit_tagged_assets(&tag_id, &members).is_err());
        db.restore_tag(&tag_id)?;
        assert!(db.get_asset_tags(&asset_id)?.contains(&tag_id));
        Ok(())
    }

    #[test]
    fn deleted_assets_are_hidden_and_can_be_restored() -> Result<()> {
        let db = AssetIndexDb::open_in_memory()?;
        let bundle_id = BundleId::new(Uuid::from_u128(68));
        let last_modified = Utc.with_ymd_and_hms(2026, 4, 7, 0, 0, 0).unwrap();
        db.upsert_bundle(&AssetBundleMetadata {
            bundle_id,
            name: "Deleted assets".to_string(),
            last_modified,
        })?;

        let asset_id = UntypedAssetId::new(Uuid::from_u128(69));
        let asset = AssetMetadata {
            asset_id,
            ty: TestAsset::TYPE_NAME.to_string(),
            bundle_id,
            relative_path: "original.asset".to_string(),
            revision: 5,
            last_modified,
            in_memory: false,
        };
        db.add_asset(&asset)?;
        let tag = sourced_tag(
            bundle_id,
            "restored.tag",
            "Restored tag",
            Some(TestAsset::TYPE_NAME.to_string()),
        );
        db.upsert_tag(&tag, last_modified)?;
        add_indexed_member(&db, &asset_id, &tag.id)?;

        db.delete_asset(&asset_id)?;

        assert!(db.get_asset(&asset_id).is_err());
        assert!(db.get_assets(UntypedAssetFilter::default())?.is_empty());
        assert!(
            db.get_assets(UntypedAssetFilter {
                tag: Some(tag.id),
                ..Default::default()
            })?
            .is_empty()
        );
        assert!(db.update_asset(&asset_id).is_err());
        assert!(add_indexed_member(&db, &asset_id, &tag.id).is_err());
        assert!(db.delete_asset(&asset_id).is_err());

        let deleted = db.get_assets(UntypedAssetFilter {
            is_deleted: true,
            ..Default::default()
        })?;
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].asset_id, asset_id);
        assert_eq!(deleted[0].relative_path, "original.asset");
        assert_eq!(deleted[0].revision, 5);
        assert_eq!(
            db.get_assets(UntypedAssetFilter {
                tag: Some(tag.id),
                is_deleted: true,
                ..Default::default()
            })?[0]
                .asset_id,
            asset_id
        );
        assert!(
            db.get_assets(UntypedAssetFilter {
                ty: Some(OtherAsset::TYPE_NAME.to_string()),
                is_deleted: true,
                ..Default::default()
            })?
            .is_empty()
        );

        let association_count = db.conn.lock().query_row(
            "SELECT COUNT(*) FROM asset_tags WHERE asset_id = ?1 AND tag_id = ?2",
            params![asset_id, tag.id],
            |row| row.get::<_, u32>(0),
        )?;
        assert_eq!(association_count, 1);

        db.restore_asset(&asset_id)?;
        let restored_in_place = db.get_asset(&asset_id)?;
        assert_eq!(restored_in_place.relative_path, "original.asset");
        assert_eq!(restored_in_place.revision, 5);
        assert!(
            db.get_assets(UntypedAssetFilter {
                is_deleted: true,
                ..Default::default()
            })?
            .is_empty()
        );
        assert!(db.restore_asset(&asset_id).is_err());
        let tagged = db.get_assets(UntypedAssetFilter {
            tag: Some(tag.id),
            ..Default::default()
        })?;
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].asset_id, asset_id);

        db.delete_asset(&asset_id)?;
        let restored = AssetMetadata {
            relative_path: "restored.asset".to_string(),
            revision: 0,
            ..asset
        };
        db.add_asset(&restored)?;

        let stored = db.get_asset(&asset_id)?;
        assert_eq!(stored.revision, 0);
        assert_eq!(stored.relative_path, "restored.asset");
        assert!(
            db.get_assets(UntypedAssetFilter {
                is_deleted: true,
                ..Default::default()
            })?
            .is_empty()
        );
        let tagged = db.get_assets(UntypedAssetFilter {
            tag: Some(tag.id),
            ..Default::default()
        })?;
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].asset_id, asset_id);

        Ok(())
    }

    #[test]
    fn asset_revision_lifecycle() -> Result<()> {
        let db = AssetIndexDb::open_in_memory()?;
        let bundle_id = BundleId::new(Uuid::from_u128(70));
        let initial_last_modified = Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap();
        db.upsert_bundle(&AssetBundleMetadata {
            bundle_id,
            name: "Revisions".to_string(),
            last_modified: initial_last_modified,
        })?;

        let first_id = UntypedAssetId::new(Uuid::from_u128(80));
        let second_id = UntypedAssetId::new(Uuid::from_u128(81));
        db.add_asset(&AssetMetadata {
            asset_id: first_id,
            ty: TestAsset::TYPE_NAME.to_string(),
            bundle_id,
            relative_path: "first.asset".to_string(),
            revision: 7,
            last_modified: initial_last_modified,
            in_memory: false,
        })?;
        db.add_asset(&AssetMetadata {
            asset_id: second_id,
            ty: TestAsset::TYPE_NAME.to_string(),
            bundle_id,
            relative_path: "second.asset".to_string(),
            revision: 0,
            last_modified: initial_last_modified,
            in_memory: false,
        })?;

        assert_eq!(db.update_asset(&first_id)?, 8);
        assert_eq!(db.update_asset(&first_id)?, 8);
        let first = db.get_asset(&first_id)?;
        assert_eq!(first.revision, 8);
        assert!(first.in_memory);
        assert_eq!(first.relative_path, "");

        let revision_count = db.conn.lock().query_row(
            "SELECT COUNT(*) FROM asset_revisions WHERE asset_id = ?1",
            params![first_id],
            |row| row.get::<_, u32>(0),
        )?;
        assert_eq!(revision_count, 2);

        let written_at = Utc.with_ymd_and_hms(2026, 5, 2, 0, 0, 0).unwrap();
        assert_eq!(
            db.write_asset(&first_id, "first.rev8.asset", written_at)?,
            8
        );
        let written = db.get_asset(&first_id)?;
        assert_eq!(written.revision, 8);
        assert_eq!(written.relative_path, "first.rev8.asset");
        assert_eq!(written.last_modified, written_at);
        assert!(!written.in_memory);
        assert_eq!(
            db.conn.lock().query_row(
                "SELECT COUNT(*) FROM asset_revisions WHERE asset_id = ?1",
                params![first_id],
                |row| row.get::<_, u32>(0),
            )?,
            2
        );

        assert_eq!(db.update_asset(&first_id)?, 9);
        db.revert_asset(&first_id)?;
        let reverted = db.get_asset(&first_id)?;
        assert_eq!(reverted.revision, 8);
        assert_eq!(reverted.relative_path, "first.rev8.asset");
        assert!(!reverted.in_memory);

        assert_eq!(db.update_asset(&first_id)?, 9);
        assert_eq!(db.update_asset(&second_id)?, 1);
        db.revert_all_assets()?;

        let first = db.get_asset(&first_id)?;
        let second = db.get_asset(&second_id)?;
        assert_eq!(first.revision, 8);
        assert!(!first.in_memory);
        assert_eq!(second.revision, 0);
        assert!(!second.in_memory);

        assert_eq!(db.update_asset(&second_id)?, 1);
        db.conn.lock().execute(
            r#"
INSERT INTO asset_revisions (asset_id, revision, relative_path, last_modified, in_memory)
VALUES (?1, 2, ?2, ?3, 0);
            "#,
            params![second_id, "second.rev2.asset", written_at],
        )?;
        assert!(
            db.write_asset(&second_id, "second.rev1.asset", written_at)
                .is_err()
        );
        let latest = db.get_asset(&second_id)?;
        assert_eq!(latest.revision, 2);
        assert_eq!(latest.relative_path, "second.rev2.asset");
        assert!(!latest.in_memory);
        let older_in_memory = db.conn.lock().query_row(
            "SELECT in_memory FROM asset_revisions WHERE asset_id = ?1 AND revision = 1",
            params![second_id],
            |row| row.get::<_, i64>(0),
        )?;
        assert_eq!(older_in_memory, 1);

        Ok(())
    }
}
