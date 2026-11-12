use anyhow::Result;
use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::LazuliArchive;

/// A serialized layer tree record stored in the `layer_tree` table of a [`LazuliArchive`].
///
/// Represents a single node in the persisted layer hierarchy. The root layer has both
/// `parent_id` and `sort_order` set to `None`, whereas all child layers specify their parent
/// UUID and sibling sort order alongside their layer type tag and serialized property blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerNode {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub sort_order: Option<u32>,
    pub layer_type: u32,
    pub properties: Vec<u8>,
}

pub(crate) fn initialize_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
CREATE TABLE layer_tree (
    id         BLOB PRIMARY KEY NOT NULL CHECK (length(id) = 16),
    parent_id  BLOB CHECK (parent_id IS NULL OR length(parent_id) = 16),
    sort_order INTEGER,
    layer_type INTEGER NOT NULL,
    properties BLOB NOT NULL,
    CHECK (
        (parent_id IS NULL AND sort_order IS NULL)
        OR
        (parent_id IS NOT NULL AND sort_order IS NOT NULL)
    )
) WITHOUT ROWID;

CREATE INDEX layer_tree_parent ON layer_tree (parent_id, sort_order);
        "#,
    )?;

    Ok(())
}

impl LazuliArchive {
    pub fn read_layer_node(&self, layer_id: Uuid) -> Result<LayerNode> {
        let (id, parent_id, sort_order, layer_type, properties) = self.conn().query_row(
            r#"
SELECT id, parent_id, sort_order, layer_type, properties
FROM layer_tree
WHERE id = ?1
            "#,
            params![&layer_id.as_bytes()[..]],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            },
        )?;

        Ok(LayerNode {
            id: Uuid::from_slice(&id)?,
            parent_id: parent_id
                .map(|parent_id| Uuid::from_slice(&parent_id))
                .transpose()?,
            sort_order: sort_order.map(u32::try_from).transpose()?,
            layer_type: u32::try_from(layer_type)?,
            properties,
        })
    }

    pub fn read_all_layer_nodes(&self) -> Result<Vec<LayerNode>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            r#"
SELECT id, parent_id, sort_order, layer_type, properties
FROM layer_tree
ORDER BY parent_id, sort_order, id
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Option<Vec<u8>>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        })?;
        let mut layers = Vec::new();

        for row in rows {
            let (id, parent_id, sort_order, layer_type, properties) = row?;
            layers.push(LayerNode {
                id: Uuid::from_slice(&id)?,
                parent_id: parent_id
                    .map(|parent_id| Uuid::from_slice(&parent_id))
                    .transpose()?,
                sort_order: sort_order.map(u32::try_from).transpose()?,
                layer_type: u32::try_from(layer_type)?,
                properties,
            });
        }

        Ok(layers)
    }

    pub fn write_layer_node(&self, layer: &LayerNode) -> Result<()> {
        let parent_id = layer
            .parent_id
            .as_ref()
            .map(|parent_id| &parent_id.as_bytes()[..]);

        self.conn().execute(
            r#"
INSERT INTO layer_tree (id, parent_id, sort_order, layer_type, properties)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT (id) DO UPDATE SET
    parent_id = excluded.parent_id,
    sort_order = excluded.sort_order,
    layer_type = excluded.layer_type,
    properties = excluded.properties
            "#,
            params![
                &layer.id.as_bytes()[..],
                parent_id,
                layer.sort_order,
                layer.layer_type,
                layer.properties,
            ],
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_tree_has_the_requested_columns() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let conn = archive.conn();
        let mut statement = conn.prepare("PRAGMA table_info(layer_tree)").unwrap();
        let columns = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(
            columns,
            [
                ("id".into(), "BLOB".into()),
                ("parent_id".into(), "BLOB".into()),
                ("sort_order".into(), "INTEGER".into()),
                ("layer_type".into(), "INTEGER".into()),
                ("properties".into(), "BLOB".into()),
            ]
        );
    }

    #[test]
    fn property_bytes_round_trip() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let root_id = Uuid::new_v4();
        let pixel_id = Uuid::new_v4();

        archive
            .write_layer_node(&LayerNode {
                id: root_id,
                parent_id: None,
                sort_order: None,
                layer_type: 1,
                properties: vec![0, 1, 2, 3],
            })
            .unwrap();
        archive
            .write_layer_node(&LayerNode {
                id: pixel_id,
                parent_id: Some(root_id),
                sort_order: Some(0),
                layer_type: 0,
                properties: vec![4, 5, 6, 7],
            })
            .unwrap();

        assert_eq!(
            archive.read_layer_node(root_id).unwrap(),
            LayerNode {
                id: root_id,
                parent_id: None,
                sort_order: None,
                layer_type: 1,
                properties: vec![0, 1, 2, 3],
            }
        );
        assert_eq!(
            archive.read_layer_node(pixel_id).unwrap(),
            LayerNode {
                id: pixel_id,
                parent_id: Some(root_id),
                sort_order: Some(0),
                layer_type: 0,
                properties: vec![4, 5, 6, 7],
            }
        );
    }

    #[test]
    fn parent_and_sort_order_must_both_be_null_or_non_null() {
        let archive = LazuliArchive::new_in_memory().unwrap();

        assert!(
            archive
                .write_layer_node(&LayerNode {
                    id: Uuid::new_v4(),
                    parent_id: Some(Uuid::new_v4()),
                    sort_order: None,
                    layer_type: 0,
                    properties: vec![],
                })
                .is_err()
        );
        assert!(
            archive
                .write_layer_node(&LayerNode {
                    id: Uuid::new_v4(),
                    parent_id: None,
                    sort_order: Some(0),
                    layer_type: 0,
                    properties: vec![],
                })
                .is_err()
        );
    }

    #[test]
    fn writing_the_same_layer_replaces_it() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let id = Uuid::new_v4();

        archive
            .write_layer_node(&LayerNode {
                id,
                parent_id: None,
                sort_order: None,
                layer_type: 1,
                properties: vec![0, 1],
            })
            .unwrap();
        archive
            .write_layer_node(&LayerNode {
                id,
                parent_id: None,
                sort_order: None,
                layer_type: 1,
                properties: vec![2, 3],
            })
            .unwrap();

        assert_eq!(archive.read_layer_node(id).unwrap().properties, [2, 3]);
    }

    #[test]
    fn reads_all_layer_nodes() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let root_id = Uuid::new_v4();
        let first_child_id = Uuid::new_v4();
        let second_child_id = Uuid::new_v4();
        let nodes = [
            LayerNode {
                id: root_id,
                parent_id: None,
                sort_order: None,
                layer_type: 1,
                properties: vec![0],
            },
            LayerNode {
                id: first_child_id,
                parent_id: Some(root_id),
                sort_order: Some(0),
                layer_type: 0,
                properties: vec![1],
            },
            LayerNode {
                id: second_child_id,
                parent_id: Some(root_id),
                sort_order: Some(1),
                layer_type: 0,
                properties: vec![2],
            },
        ];

        archive.write_layer_node(&nodes[2]).unwrap();
        archive.write_layer_node(&nodes[0]).unwrap();
        archive.write_layer_node(&nodes[1]).unwrap();

        assert_eq!(archive.read_all_layer_nodes().unwrap(), nodes);
    }
}
