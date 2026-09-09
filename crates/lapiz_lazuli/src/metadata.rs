use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, params};

use crate::{LazuliArchive, VERSION};

pub(crate) fn initialize_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        r#"
CREATE TABLE metadata (
    version INTEGER NOT NULL DEFAULT {VERSION}
);

CREATE UNIQUE INDEX metadata_singleton ON metadata ((1));
        "#,
    ))?;

    Ok(())
}

pub struct Metadata {
    pub version: u32,
}

impl LazuliArchive {
    pub fn read_metadata(&self) -> Result<Metadata> {
        let conn = self.conn();
        let mut statement = conn.prepare("SELECT version FROM metadata")?;
        let mut rows = statement.query([])?;
        let row = rows
            .next()?
            .ok_or_else(|| anyhow!("archive does not contain metadata"))?;
        let version = u32::try_from(row.get::<_, i64>(0)?)?;

        if rows.next()?.is_some() {
            bail!("archive contains more than one metadata row");
        }

        Ok(Metadata { version })
    }

    pub fn write_metadata(&self, metadata: Metadata) -> Result<()> {
        let mut conn = self.conn();
        let transaction = conn.transaction()?;
        transaction.execute("DELETE FROM metadata", [])?;
        transaction.execute(
            "INSERT INTO metadata (version) VALUES (?1)",
            params![metadata.version],
        )?;
        transaction.commit()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::LazuliArchive;

    #[test]
    fn metadata_table_contains_the_current_version() {
        let archive = LazuliArchive::new_in_memory().unwrap();
        let conn = archive.conn();
        let mut statement = conn.prepare("PRAGMA table_info(metadata)").unwrap();
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(
            columns,
            [(
                "version".into(),
                "INTEGER".into(),
                true,
                Some(VERSION.to_string())
            )]
        );

        drop(statement);
        drop(conn);
        assert_eq!(archive.read_metadata().unwrap().version, VERSION);
    }

    #[test]
    fn opening_an_archive_with_an_unsupported_version_fails() {
        let directory =
            std::env::temp_dir().join(format!("lapiz_lazuli_metadata_{}", Uuid::new_v4()));
        let path = directory.join("archive.lazuli");
        let archive = LazuliArchive::new(&path).unwrap();
        archive
            .write_metadata(Metadata {
                version: VERSION + 1,
            })
            .unwrap();
        drop(archive);

        let error = LazuliArchive::open(&path).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported lazuli archive version")
        );

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
