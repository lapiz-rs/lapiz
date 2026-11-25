use std::collections::HashMap;

use anyhow::Result;
use glam::IVec2;
use rusqlite::{Connection, OptionalExtension as _, params};
use uuid::Uuid;

use crate::LazuliArchive;

pub(crate) fn initialize_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE tile_data (
    layer_id BLOB NOT NULL CHECK (length(layer_id) = 16),
    tile_x   INTEGER NOT NULL,
    tile_y   INTEGER NOT NULL,
    data     BLOB NOT NULL,
    PRIMARY KEY (layer_id, tile_x, tile_y)
) WITHOUT ROWID;
        "#,
    )?;

    Ok(())
}

impl LazuliArchive {
    pub fn read_layer_data(&self, layer_id: Uuid) -> Result<HashMap<IVec2, Vec<u8>>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            r#"
SELECT tile_x, tile_y, data
FROM tile_data
WHERE layer_id = ?1
            "#,
        )?;
        let rows = statement.query_map(params![&layer_id.as_bytes()[..]], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let mut tiles = HashMap::new();

        for row in rows {
            let (tile_x, tile_y, data) = row?;
            tiles.insert(IVec2::new(tile_x, tile_y), data);
        }

        Ok(tiles)
    }

    pub fn write_tile_data(
        &self,
        layer_id: Uuid,
        tile_x: i32,
        tile_y: i32,
        data: impl AsRef<[u8]>,
    ) -> Result<()> {
        self.conn().execute(
            r#"
INSERT INTO tile_data (layer_id, tile_x, tile_y, data)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (layer_id, tile_x, tile_y) DO UPDATE SET
    data = excluded.data
            "#,
            params![&layer_id.as_bytes()[..], tile_x, tile_y, data.as_ref()],
        )?;

        Ok(())
    }

    pub fn apply_tile_changes(
        &self,
        layer_id: Uuid,
        writes: impl IntoIterator<Item = (IVec2, Vec<u8>)>,
        deletes: impl IntoIterator<Item = IVec2>,
    ) -> Result<()> {
        let id = layer_id.as_bytes().as_slice();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        {
            let mut delete = tx.prepare(
                "DELETE FROM tile_data WHERE layer_id = ?1 AND tile_x = ?2 AND tile_y = ?3",
            )?;
            for tile in deletes {
                delete.execute(params![id, tile.x, tile.y])?;
            }
            let mut write = tx.prepare(
                r#"
INSERT INTO tile_data (layer_id, tile_x, tile_y, data)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (layer_id, tile_x, tile_y) DO UPDATE SET
    data = excluded.data
            "#,
            )?;
            for (tile, data) in writes {
                write.execute(params![id, tile.x, tile.y, data])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn read_tile_data(
        &self,
        layer_id: Uuid,
        tile_x: i32,
        tile_y: i32,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn()
            .query_row(
                r#"
SELECT data
FROM tile_data
WHERE layer_id = ?1 AND tile_x = ?2 AND tile_y = ?3
                "#,
                params![&layer_id.as_bytes()[..], tile_x, tile_y],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::tile_data::*;

    #[test]
    fn tile_data_table_has_the_requested_columns() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let conn = archive.conn();
        let mut statement = conn.prepare("PRAGMA table_info(tile_data)").unwrap();
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(
            columns,
            [
                ("layer_id".into(), "BLOB".into(), true),
                ("tile_x".into(), "INTEGER".into(), true),
                ("tile_y".into(), "INTEGER".into(), true),
                ("data".into(), "BLOB".into(), true),
            ]
        );
    }

    #[test]
    fn compressed_tile_data_round_trips_without_transformation() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let layer_id = Uuid::new_v4();
        let compressed_data = [120, 156, 99, 96, 100, 98, 6, 0, 0, 14, 0, 7];

        archive
            .write_tile_data(layer_id, -2, 3, compressed_data)
            .unwrap();

        assert_eq!(
            archive.read_tile_data(layer_id, -2, 3).unwrap(),
            Some(compressed_data.to_vec())
        );
    }

    #[test]
    fn writing_the_same_tile_replaces_its_data() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let layer_id = Uuid::new_v4();

        archive.write_tile_data(layer_id, 4, 5, [1, 2]).unwrap();
        archive.write_tile_data(layer_id, 4, 5, [3, 4]).unwrap();

        assert_eq!(
            archive.read_tile_data(layer_id, 4, 5).unwrap(),
            Some(vec![3, 4])
        );
    }

    #[test]
    fn applies_tile_additions_replacements_and_deletions() -> Result<()> {
        let archive = LazuliArchive::new_in_memory()?;
        let layer = Uuid::new_v4();
        let removed = IVec2::ZERO;
        let replaced = IVec2::X;
        let added = IVec2::Y;
        let unchanged = IVec2::ONE;
        archive.apply_tile_changes(
            layer,
            [
                (removed, vec![1]),
                (replaced, vec![2]),
                (unchanged, vec![3]),
            ],
            [],
        )?;

        archive.apply_tile_changes(layer, [(replaced, vec![4]), (added, vec![5])], [removed])?;
        assert_eq!(
            archive.read_layer_data(layer)?,
            HashMap::from([(replaced, vec![4]), (added, vec![5]), (unchanged, vec![3])]),
        );
        Ok(())
    }

    #[test]
    fn reads_all_tiles_from_one_layer() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let layer_id = Uuid::new_v4();
        let other_layer_id = Uuid::new_v4();

        archive.write_tile_data(layer_id, -1, 2, [1, 2]).unwrap();
        archive.write_tile_data(layer_id, 3, 4, [3, 4]).unwrap();
        archive
            .write_tile_data(other_layer_id, 5, 6, [5, 6])
            .unwrap();

        assert_eq!(
            archive.read_layer_data(layer_id).unwrap(),
            HashMap::from([
                (IVec2::new(-1, 2), vec![1, 2]),
                (IVec2::new(3, 4), vec![3, 4]),
            ])
        );
    }
}
