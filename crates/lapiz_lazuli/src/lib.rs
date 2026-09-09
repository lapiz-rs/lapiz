use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, bail};
use parking_lot::{Mutex, MutexGuard, RawMutex, lock_api::MappedMutexGuard};
use rusqlite::{Connection, MAIN_DB, OpenFlags};

pub mod image_props;
pub mod layer_tree;
pub mod metadata;
pub mod tile_data;
pub use image_props::ImageProperties;
pub use layer_tree::LayerNode;
pub use metadata::Metadata;

pub const VERSION: u32 = 0;

struct Inner {
    path: Option<PathBuf>,
    conn: Connection,
}

pub struct LazuliArchive {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for LazuliArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazuliArchive").finish()
    }
}

impl LazuliArchive {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }

        File::create_new(path)?;

        let conn = Connection::open(path)?;
        let archive = Self {
            inner: Arc::new(Mutex::new(Inner {
                path: Some(path.to_path_buf()),
                conn,
            })),
        };
        archive.initialize_tables()?;
        archive.write_metadata(Metadata { version: VERSION })?;

        Ok(archive)
    }

    pub fn new_in_memory() -> Result<Self> {
        let archive = Self {
            inner: Arc::new(Mutex::new(Inner {
                path: None,
                conn: Connection::open_in_memory()?,
            })),
        };
        archive.initialize_tables()?;
        archive.write_metadata(Metadata { version: VERSION })?;

        Ok(archive)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let archive = Self {
            inner: Arc::new(Mutex::new(Inner {
                path: Some(path.to_path_buf()),
                conn,
            })),
        };
        let metadata = archive.read_metadata()?;
        if metadata.version != VERSION {
            bail!("unsupported lazuli archive version: {}", metadata.version);
        }

        Ok(archive)
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.inner.lock().path.clone()
    }

    pub fn set_path(&mut self, path: PathBuf) -> Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }

        let mut inner = self.inner.lock();
        if inner.path.as_deref() == Some(path.as_path()) {
            return Ok(());
        }

        inner.conn.backup(MAIN_DB, &path, None)?;
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        inner.conn = conn;
        inner.path = Some(path);

        Ok(())
    }

    pub(crate) fn conn(&'_ self) -> MappedMutexGuard<'_, RawMutex, Connection> {
        let lock = self.inner.lock();
        MutexGuard::map(lock, |g| &mut g.conn)
    }

    fn initialize_tables(&self) -> Result<()> {
        {
            let conn = self.conn();
            metadata::initialize_table(&conn)?;
            image_props::initialize_table(&conn)?;
            layer_tree::initialize_table(&conn)?;
            tile_data::initialize_table(&conn)?;
        }
        self.write_metadata(Metadata { version: VERSION })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    #[test]
    fn set_path_saves_an_in_memory_archive_and_switches_to_the_disk_database() {
        let directory =
            std::env::temp_dir().join(format!("lapiz_lazuli_set_path_{}", Uuid::new_v4()));
        let path = directory.join("archive.lazuli");
        let mut archive = LazuliArchive::new_in_memory().unwrap();
        let root_layer = Uuid::new_v4();

        archive
            .write_image_properties(&ImageProperties {
                width: 128,
                height: 256,
                tile_size: 256,
                color_profile: vec![0, 1, 2, 3],
                root_layer,
                texel_type: 8,
            })
            .unwrap();
        archive.set_path(path.clone()).unwrap();

        assert_eq!(archive.path(), Some(path.clone()));
        assert!(path.is_file());

        archive
            .write_image_properties(&ImageProperties {
                width: 512,
                height: 256,
                tile_size: 256,
                color_profile: vec![4, 5, 6, 7],
                root_layer,
                texel_type: 10,
            })
            .unwrap();
        drop(archive);

        let archive = LazuliArchive::open(&path).unwrap();
        let properties = archive.read_image_properties().unwrap();
        assert_eq!(properties.width, 512);
        assert_eq!(properties.height, 256);
        assert_eq!(properties.tile_size, 256);
        assert_eq!(properties.color_profile, [4, 5, 6, 7]);
        assert_eq!(properties.root_layer, root_layer);
        assert_eq!(properties.texel_type, 10);
        drop(archive);

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
