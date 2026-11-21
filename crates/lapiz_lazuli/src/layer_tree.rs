use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension as _, params};
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

impl LayerNode {
    pub fn with_parent(mut self, parent_id: Uuid, sort_order: u32) -> Self {
        self.parent_id = Some(parent_id);
        self.sort_order = Some(sort_order);
        self
    }
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

    pub fn insert_layer_node(&self, layer: &LayerNode) -> Result<()> {
        let parent = layer
            .parent_id
            .ok_or_else(|| anyhow!("Missing parent layer"))?;
        let index = layer
            .sort_order
            .ok_or_else(|| anyhow!("Missing layer order"))?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE layer_tree SET sort_order = sort_order + 1 WHERE parent_id = ?1 AND sort_order >= ?2",
            params![parent.as_bytes().as_slice(), index],
        )?;
        tx.execute(
            "INSERT INTO layer_tree (id, parent_id, sort_order, layer_type, properties) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![layer.id.as_bytes().as_slice(), parent.as_bytes().as_slice(), index, layer.layer_type, layer.properties],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_layer_hierarchy(&self, layer_id: Uuid) -> Result<()> {
        let id = layer_id.as_bytes().as_slice();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let position = tx
            .query_row(
                "SELECT parent_id, sort_order FROM layer_tree WHERE id = ?1 AND parent_id IS NOT NULL",
                params![id],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()?;
        let (parent, index) =
            position.ok_or_else(|| anyhow!("Layer {layer_id} is missing or is the root"))?;

        tx.execute(
            r#"
WITH RECURSIVE subtree(id) AS (
    SELECT ?1
    UNION ALL
    SELECT layer_tree.id FROM layer_tree
        JOIN subtree ON layer_tree.parent_id = subtree.id
)
DELETE FROM tile_data WHERE layer_id IN (SELECT id FROM subtree)
"#,
            params![id],
        )?;
        tx.execute(
            r#"
WITH RECURSIVE subtree(id) AS (
    SELECT ?1
    UNION ALL
    SELECT layer_tree.id FROM layer_tree
        JOIN subtree ON layer_tree.parent_id = subtree.id
)
DELETE FROM layer_tree WHERE id IN (SELECT id FROM subtree)
"#,
            params![id],
        )?;
        tx.execute(
            "UPDATE layer_tree SET sort_order = sort_order - 1 WHERE parent_id = ?1 AND sort_order > ?2",
            params![parent, index],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Detaches all listed nodes, then inserts them in input order at the specified indices.
    /// Descendants and tile data stay attached to their original node IDs.
    pub fn move_layer_nodes(&self, moves: &[(Uuid, Uuid, usize)]) -> Result<()> {
        if moves.is_empty() {
            return Ok(());
        }

        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for &(layer, _, _) in moves {
            let id = layer.as_bytes().as_slice();
            let position = tx.query_row(
                "SELECT parent_id, sort_order FROM layer_tree WHERE id = ?1 AND parent_id IS NOT NULL",
                params![id],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, u32>(1)?)),
            ).optional()?;
            let (parent, index) = position.ok_or_else(|| {
                anyhow!("Layer {layer} is missing, is the root, or was already detached")
            })?;
            tx.execute(
                "UPDATE layer_tree SET parent_id = NULL, sort_order = NULL WHERE id = ?1",
                params![id],
            )?;
            tx.execute(
                "UPDATE layer_tree SET sort_order = sort_order - 1 WHERE parent_id = ?1 AND sort_order > ?2",
                params![parent, index],
            )?;
        }

        for &(layer, parent, index) in moves {
            let id = layer.as_bytes().as_slice();
            let parent = parent.as_bytes().as_slice();
            let index = u32::try_from(index)?;
            let child_count = tx.query_row(
                "SELECT (SELECT COUNT(*) FROM layer_tree WHERE parent_id = ?1) FROM layer_tree WHERE id = ?1",
                params![parent],
                |row| row.get::<_, u32>(0),
            ).optional()?.ok_or_else(|| anyhow!("Destination layer is missing"))?;
            if index > child_count {
                bail!("Layer insertion index is out of bounds");
            }

            tx.execute(
                "UPDATE layer_tree SET sort_order = sort_order + 1 WHERE parent_id = ?1 AND sort_order >= ?2",
                params![parent, index],
            )?;
            tx.execute(
                "UPDATE layer_tree SET parent_id = ?2, sort_order = ?3 WHERE id = ?1",
                params![id, parent, index],
            )?;
        }

        for &(layer, _, _) in moves {
            let cyclic = tx.query_row(
                r#"
WITH RECURSIVE ancestors(id) AS (
    SELECT parent_id FROM layer_tree WHERE id = ?1
    UNION
    SELECT layer_tree.parent_id FROM layer_tree JOIN ancestors ON layer_tree.id = ancestors.id
)
SELECT EXISTS (SELECT 1 FROM ancestors WHERE id = ?1)
"#,
                params![layer.as_bytes().as_slice()],
                |row| row.get::<_, bool>(0),
            )?;
            if cyclic {
                bail!("Cannot move layer {layer} into its own subtree");
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn write_layer_properties(&self, layer: Uuid, properties: &[u8]) -> Result<()> {
        let updated = self.conn().execute(
            "UPDATE layer_tree SET properties = ?2 WHERE id = ?1",
            params![layer.as_bytes().as_slice(), properties],
        )?;
        if updated == 0 {
            bail!("Layer {layer} is missing");
        }
        Ok(())
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
    fn inserting_and_removing_a_leaf_updates_sibling_order() -> Result<()> {
        let archive = LazuliArchive::new_in_memory()?;
        let root = LayerNode {
            id: Uuid::new_v4(),
            parent_id: None,
            sort_order: None,
            layer_type: 1,
            properties: vec![],
        };
        let sibling = LayerNode {
            id: Uuid::new_v4(),
            parent_id: Some(root.id),
            sort_order: Some(0),
            layer_type: 0,
            properties: vec![],
        };
        let inserted = LayerNode {
            id: Uuid::new_v4(),
            ..sibling.clone()
        };
        archive.write_layer_node(&root)?;
        archive.write_layer_node(&sibling)?;

        archive.insert_layer_node(&inserted)?;
        archive.write_tile_data(inserted.id, 0, 0, [1])?;
        assert_eq!(archive.read_layer_node(inserted.id)?, inserted);
        assert_eq!(archive.read_layer_node(sibling.id)?.sort_order, Some(1));

        archive.remove_layer_hierarchy(inserted.id)?;
        assert_eq!(archive.read_all_layer_nodes()?, [root, sibling]);
        assert_eq!(archive.read_tile_data(inserted.id, 0, 0)?, None);
        Ok(())
    }

    #[test]
    fn removing_a_hierarchy_preserves_other_layers_and_tiles() -> Result<()> {
        let archive = LazuliArchive::new_in_memory()?;
        let root = LayerNode {
            id: Uuid::new_v4(),
            parent_id: None,
            sort_order: None,
            layer_type: 1,
            properties: vec![],
        };
        archive.write_layer_node(&root)?;

        let add_layer = |parent_id, sort_order| -> Result<_> {
            let node = LayerNode {
                id: Uuid::new_v4(),
                parent_id: Some(parent_id),
                sort_order: Some(sort_order),
                layer_type: 1,
                properties: vec![],
            };
            archive.write_layer_node(&node)?;
            archive.write_tile_data(node.id, 0, 0, [1])?;
            Ok(node)
        };
        let before = add_layer(root.id, 0)?;
        let group = add_layer(root.id, 1)?;
        let after = add_layer(root.id, 2)?;
        let child = add_layer(group.id, 0)?;
        let grandchild = add_layer(child.id, 0)?;

        assert!(archive.remove_layer_hierarchy(root.id).is_err());
        assert_eq!(archive.read_all_layer_nodes()?.len(), 6);

        archive.remove_layer_hierarchy(group.id)?;

        assert_eq!(
            archive.read_all_layer_nodes()?,
            [
                root,
                before.clone(),
                LayerNode {
                    sort_order: Some(1),
                    ..after.clone()
                }
            ]
        );
        for node in [group, child, grandchild] {
            assert_eq!(archive.read_tile_data(node.id, 0, 0)?, None);
        }
        for node in [before, after] {
            assert_eq!(archive.read_tile_data(node.id, 0, 0)?, Some(vec![1]));
        }
        Ok(())
    }

    #[test]
    fn cyclic_move_leaves_layer_positions_unchanged() -> Result<()> {
        let archive = LazuliArchive::new_in_memory()?;
        let root = Uuid::new_v4();
        let group = Uuid::new_v4();
        let child = Uuid::new_v4();
        let sibling = Uuid::new_v4();
        for (id, parent_id, sort_order) in [
            (root, None, None),
            (group, Some(root), Some(0)),
            (child, Some(group), Some(0)),
            (sibling, Some(root), Some(1)),
        ] {
            archive.write_layer_node(&LayerNode {
                id,
                parent_id,
                sort_order,
                layer_type: 1,
                properties: vec![],
            })?;
        }
        let before = archive.read_all_layer_nodes()?;

        assert!(
            archive
                .move_layer_nodes(&[(sibling, root, 0), (group, child, 0)])
                .is_err()
        );
        assert_eq!(archive.read_all_layer_nodes()?, before);
        Ok(())
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
