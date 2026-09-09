use std::{
    collections::{BTreeSet, HashSet},
    fs::File,
    marker::PhantomData,
    path::Path,
};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{
    asset::{Asset, AssetMetadata, UntypedAssetId},
    bundle::{AssetBundleMetadata, BundleId, BundleSnapshot},
    error::{AssetErrorKind, AssetResult},
    tag::{Tag, TagId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    UpToDate,
    Outdated,
}

pub struct AssetFilter<T: Asset> {
    tag: Option<TagId>,
    bundle: Option<BundleId>,
    is_deleted: bool,
    _marker: PhantomData<T>,
}

impl<T: Asset> Default for AssetFilter<T> {
    fn default() -> Self {
        Self {
            tag: Default::default(),
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

    pub fn with_deleted(mut self, is_deleted: bool) -> Self {
        self.is_deleted = is_deleted;
        self
    }

    pub fn into_untyped(self) -> UntypedAssetFilter {
        UntypedAssetFilter {
            ty: Some(T::TYPE_NAME.to_string()),
            tag: self.tag,
            bundle: self.bundle,
            is_deleted: self.is_deleted,
        }
    }
}

#[derive(Default)]
pub struct UntypedAssetFilter {
    pub ty: Option<String>,
    pub tag: Option<TagId>,
    pub bundle: Option<BundleId>,
    pub is_deleted: bool,
}

#[derive(Default)]
pub struct TagFilter {
    pub asset_ty: Option<Option<String>>,
    pub is_deleted: bool,
}

pub struct AssetIndexDb {
    conn: Mutex<Connection>,
}

impl AssetIndexDb {
    pub fn connect(path: impl AsRef<Path>) -> AssetResult<Self> {
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

    pub fn open_in_memory() -> AssetResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Self { conn: conn.into() };
        db.initialize_tables()?;
        db.revert_all_assets()?;
        Ok(db)
    }

    fn initialize_tables(&self) -> AssetResult<()> {
        let conn = self.conn.lock();

        let tag_columns = {
            let mut statement = conn.prepare("PRAGMA table_info(tags)")?;
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?
        };
        if !tag_columns.is_empty()
            && (!tag_columns.iter().any(|column| column == "bundle_id")
                || !tag_columns.iter().any(|column| column == "relative_path"))
        {
            conn.execute_batch(
                r#"
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
    PRIMARY KEY (asset_id, tag_id),
    FOREIGN KEY (asset_id) REFERENCES assets(asset_id) ON DELETE CASCADE,
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
);
            "#,
        )?;

        Ok(())
    }

    pub(crate) fn sync_bundles(&self, bundles: &[BundleSnapshot]) -> AssetResult<()> {
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
                    .collect::<Result<Vec<UntypedAssetId>, _>>()?
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
                    .collect::<Result<Vec<TagId>, _>>()?
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
                    return Err(AssetErrorKind::DuplicateTagDefinition {
                        tag_id: tag.id,
                        first_bundle_id: existing_bundle_id,
                        second_bundle_id: tag.bundle_id,
                    }
                    .into());
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
            tx.execute(
                r#"
DELETE FROM asset_tags
WHERE asset_id IN (
    SELECT asset_id FROM assets WHERE bundle_id = ?1
);
                "#,
                params![bundle.metadata.bundle_id],
            )?;
            for (asset_id, asset_tags) in &bundle.asset_tags {
                for tag_id in &asset_tags.tags {
                    tx.execute(
                        "INSERT INTO asset_tags (asset_id, tag_id) VALUES (?1, ?2)",
                        params![asset_id, tag_id],
                    )?;
                }
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub(crate) fn remove_unloaded_bundles(
        &self,
        loaded_bundle_ids: &HashSet<BundleId>,
    ) -> AssetResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let stored_bundle_ids = {
            let mut statement = tx.prepare("SELECT bundle_id FROM bundles")?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<BundleId>, _>>()?
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

    pub fn upsert_bundle(&self, bundle: &AssetBundleMetadata) -> AssetResult<ItemStatus> {
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

    pub fn upsert_tag(&self, tag: &Tag, last_modified: DateTime<Utc>) -> AssetResult<()> {
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
                    return Err(AssetErrorKind::DuplicateTagDefinition {
                        tag_id: tag.id,
                        first_bundle_id: stored_bundle_id,
                        second_bundle_id: tag.bundle_id,
                    }
                    .into());
                }

                if is_deleted {
                    tx.commit()?;
                    return Ok(());
                }

                if stored_asset_ty != tag.asset_ty {
                    return Err(AssetErrorKind::TagAssetTypeChanged {
                        tag_id: tag.id,
                        current_asset_ty: stored_asset_ty,
                        new_asset_ty: tag.asset_ty.clone(),
                    }
                    .into());
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

    pub fn get_tag(&self, tag_id: TagId) -> AssetResult<Tag> {
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

    pub fn get_tags(&self, filter: TagFilter) -> AssetResult<Vec<Tag>> {
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

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn add_tag(&self, tag: Tag) -> AssetResult<()> {
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

    pub fn delete_tag(&self, tag_id: &TagId) -> AssetResult<()> {
        let conn = self.conn.lock();
        let deleted = conn.execute(
            "UPDATE tags SET is_deleted = 1 WHERE id = ?1 AND is_deleted = 0",
            params![tag_id],
        )?;
        if deleted == 0 {
            return Err(AssetErrorKind::TagNotFound(*tag_id).into());
        }

        Ok(())
    }

    pub fn add_asset(&self, asset: &AssetMetadata) -> AssetResult<UntypedAssetId> {
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

    pub fn get_asset(&self, id: &UntypedAssetId) -> AssetResult<AssetMetadata> {
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

    pub fn get_assets(&self, filter: UntypedAssetFilter) -> AssetResult<Vec<AssetMetadata>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            r#"
WITH latest AS (
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
            JOIN tags t ON t.id = at.tag_id
            WHERE at.asset_id = a.asset_id
                AND at.tag_id = ?2
                AND (t.is_deleted = 0 OR ?4 = 1)
        )
    )
    AND (?3 IS NULL OR a.bundle_id = ?3)
ORDER BY l.relative_path ASC;
            "#,
        )?;

        let rows = stmt.query_map(
            params![
                filter.ty,
                filter.tag.as_ref(),
                filter.bundle.as_ref(),
                filter.is_deleted,
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

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn update_asset(&self, id: &UntypedAssetId) -> AssetResult<u32> {
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
    ) -> AssetResult<u32> {
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

    pub fn delete_asset(&self, asset_id: &UntypedAssetId) -> AssetResult<()> {
        let conn = self.conn.lock();
        let deleted = conn.execute(
            "UPDATE assets SET is_deleted = 1 WHERE asset_id = ?1 AND is_deleted = 0",
            params![asset_id],
        )?;
        if deleted == 0 {
            return Err(AssetErrorKind::AssetNotFound(*asset_id).into());
        }

        Ok(())
    }

    pub fn restore_tag(&self, tag_id: &TagId) -> AssetResult<()> {
        let conn = self.conn.lock();
        let restored = conn.execute(
            "UPDATE tags SET is_deleted = 0 WHERE id = ?1 AND is_deleted = 1",
            params![tag_id],
        )?;
        if restored == 0 {
            return Err(AssetErrorKind::TagNotFound(*tag_id).into());
        }

        Ok(())
    }

    pub fn restore_asset(&self, asset_id: &UntypedAssetId) -> AssetResult<()> {
        let conn = self.conn.lock();
        let restored = conn.execute(
            "UPDATE assets SET is_deleted = 0 WHERE asset_id = ?1 AND is_deleted = 1",
            params![asset_id],
        )?;
        if restored == 0 {
            return Err(AssetErrorKind::AssetNotFound(*asset_id).into());
        }

        Ok(())
    }

    pub fn add_tag_to_asset(&self, asset_id: &UntypedAssetId, tag_id: &TagId) -> AssetResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        let asset_ty = tx
            .query_row(
                "SELECT ty FROM assets WHERE asset_id = ?1 AND is_deleted = 0",
                params![asset_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| AssetErrorKind::AssetNotFound(*asset_id))?;
        let tag_asset_ty = tx
            .query_row(
                "SELECT asset_ty FROM tags WHERE id = ?1 AND is_deleted = 0",
                params![tag_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| AssetErrorKind::TagNotFound(*tag_id))?;

        if let Some(expected_ty) = tag_asset_ty
            && asset_ty != expected_ty
        {
            return Err(AssetErrorKind::InvalidTagAssetType {
                tag_id: *tag_id,
                asset_id: *asset_id,
                asset_ty,
                expected_ty,
            }
            .into());
        }

        let inserted = tx.execute(
            r#"
INSERT INTO asset_tags (asset_id, tag_id)
VALUES (?1, ?2)
ON CONFLICT DO NOTHING;
            "#,
            params![asset_id, tag_id],
        )?;
        if inserted == 0 {
            return Err(AssetErrorKind::TagAlreadyAssigned {
                asset_id: *asset_id,
                tag_id: *tag_id,
            }
            .into());
        }

        tx.commit()?;
        Ok(())
    }

    pub fn remove_tag_from_asset(
        &self,
        asset_id: &UntypedAssetId,
        tag_id: &TagId,
    ) -> AssetResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        let tag_exists = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE id = ?1 AND is_deleted = 0)",
            params![tag_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !tag_exists {
            return Err(AssetErrorKind::TagNotFound(*tag_id).into());
        }

        let deleted = tx.execute(
            "DELETE FROM asset_tags WHERE asset_id = ?1 AND tag_id = ?2",
            params![asset_id, tag_id],
        )?;
        if deleted == 0 {
            return Err(AssetErrorKind::TagNotAssigned {
                asset_id: *asset_id,
                tag_id: *tag_id,
            }
            .into());
        }

        tx.commit()?;
        Ok(())
    }

    pub fn revert_asset(&self, id: &Uuid) -> AssetResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM asset_revisions WHERE in_memory = 1 AND asset_id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn revert_all_assets(&self) -> AssetResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM asset_revisions WHERE in_memory = 1", [])?;
        Ok(())
    }

    pub fn get_bundle(&self, id: &BundleId) -> AssetResult<AssetBundleMetadata> {
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

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

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
    fn bundle_upsert_and_get() -> AssetResult<()> {
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

    #[test]
    fn add_get_and_filter_assets() -> AssetResult<()> {
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
    fn tags_are_upserted_queried_and_filtered() -> AssetResult<()> {
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
    fn tag_asset_association_lifecycle() -> AssetResult<()> {
        let db = AssetIndexDb::open_in_memory()?;
        let bundle_id = BundleId::new(Uuid::from_u128(63));
        let last_modified = Utc.with_ymd_and_hms(2026, 4, 5, 0, 0, 0).unwrap();
        db.upsert_bundle(&AssetBundleMetadata {
            bundle_id,
            name: "Tagged assets".to_string(),
            last_modified,
        })?;

        let test_asset_id = UntypedAssetId::new(Uuid::from_u128(64));
        let other_asset_id = UntypedAssetId::new(Uuid::from_u128(65));
        for (asset_id, ty) in [
            (test_asset_id, TestAsset::TYPE_NAME),
            (other_asset_id, OtherAsset::TYPE_NAME),
        ] {
            db.add_asset(&AssetMetadata {
                asset_id,
                ty: ty.to_string(),
                bundle_id,
                relative_path: format!("{asset_id}.asset"),
                revision: 0,
                last_modified,
                in_memory: false,
            })?;
        }

        let typed_tag = sourced_tag(
            bundle_id,
            "test.tag",
            "Test assets",
            Some(TestAsset::TYPE_NAME.to_string()),
        );
        let untyped_tag = sourced_tag(bundle_id, "any.tag", "Any assets", None);
        db.upsert_tag(&typed_tag, last_modified)?;
        db.upsert_tag(&untyped_tag, last_modified)?;

        db.add_tag_to_asset(&test_asset_id, &typed_tag.id)?;
        assert!(db.add_tag_to_asset(&test_asset_id, &typed_tag.id).is_err());
        assert!(db.add_tag_to_asset(&other_asset_id, &typed_tag.id).is_err());
        db.add_tag_to_asset(&other_asset_id, &untyped_tag.id)?;

        let tagged = db.get_assets(UntypedAssetFilter {
            tag: Some(typed_tag.id),
            ..Default::default()
        })?;
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].asset_id, test_asset_id);

        db.remove_tag_from_asset(&test_asset_id, &typed_tag.id)?;
        assert!(
            db.remove_tag_from_asset(&test_asset_id, &typed_tag.id)
                .is_err()
        );
        assert!(
            db.get_assets(UntypedAssetFilter {
                tag: Some(typed_tag.id),
                ..Default::default()
            })?
            .is_empty()
        );

        db.delete_tag(&untyped_tag.id)?;

        assert!(db.get_tag(untyped_tag.id).is_err());
        assert!(
            db.get_tags(TagFilter::default())?
                .iter()
                .all(|tag| tag.id != untyped_tag.id)
        );
        assert_eq!(
            db.get_tags(TagFilter {
                is_deleted: true,
                ..Default::default()
            })?
            .iter()
            .map(|tag| tag.id)
            .collect::<Vec<_>>(),
            vec![untyped_tag.id]
        );
        assert_eq!(
            db.get_tags(TagFilter {
                asset_ty: Some(None),
                is_deleted: true,
            })?[0]
                .id,
            untyped_tag.id
        );
        assert!(
            db.get_tags(TagFilter {
                asset_ty: Some(Some(TestAsset::TYPE_NAME.to_string())),
                is_deleted: true,
            })?
            .is_empty()
        );
        assert!(
            db.get_assets(UntypedAssetFilter {
                tag: Some(untyped_tag.id),
                ..Default::default()
            })?
            .is_empty()
        );
        db.delete_asset(&other_asset_id)?;
        assert_eq!(
            db.get_assets(UntypedAssetFilter {
                tag: Some(untyped_tag.id),
                is_deleted: true,
                ..Default::default()
            })?[0]
                .asset_id,
            other_asset_id
        );
        assert!(
            db.add_tag_to_asset(&test_asset_id, &untyped_tag.id)
                .is_err()
        );
        assert!(
            db.remove_tag_from_asset(&other_asset_id, &untyped_tag.id)
                .is_err()
        );
        assert!(db.delete_tag(&untyped_tag.id).is_err());

        let mut rediscovered_tag = untyped_tag.clone();
        rediscovered_tag.name = "Rediscovered tag".to_string();
        db.upsert_tag(
            &rediscovered_tag,
            Utc.with_ymd_and_hms(2026, 4, 6, 0, 0, 0).unwrap(),
        )?;
        assert!(db.get_tag(untyped_tag.id).is_err());

        let (association_count, is_deleted) = db.conn.lock().query_row(
            r#"
SELECT
    (SELECT COUNT(*) FROM asset_tags WHERE tag_id = ?1),
    (SELECT is_deleted FROM tags WHERE id = ?1);
            "#,
            params![untyped_tag.id],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, bool>(1)?)),
        )?;
        assert_eq!(association_count, 1);
        assert!(is_deleted);

        db.restore_tag(&untyped_tag.id)?;
        assert_eq!(db.get_tag(untyped_tag.id)?.id, untyped_tag.id);
        assert!(
            db.get_tags(TagFilter {
                is_deleted: true,
                ..Default::default()
            })?
            .is_empty()
        );
        assert!(db.restore_tag(&untyped_tag.id).is_err());

        db.restore_asset(&other_asset_id)?;
        let tagged = db.get_assets(UntypedAssetFilter {
            tag: Some(untyped_tag.id),
            ..Default::default()
        })?;
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].asset_id, other_asset_id);

        Ok(())
    }

    #[test]
    fn deleted_assets_are_hidden_and_can_be_restored() -> AssetResult<()> {
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
        db.add_tag_to_asset(&asset_id, &tag.id)?;

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
        assert!(db.add_tag_to_asset(&asset_id, &tag.id).is_err());
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
    fn asset_revision_lifecycle() -> AssetResult<()> {
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
