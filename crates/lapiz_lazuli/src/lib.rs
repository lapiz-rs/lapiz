use std::{
    fmt,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use parking_lot::{Mutex, MutexGuard, RawMutex, lock_api::MappedMutexGuard};
use rusqlite::{
    Connection, MAIN_DB, OpenFlags,
    backup::{Backup, NO_PROGRESS},
};

pub mod image_props;
pub mod layer_tree;
pub mod metadata;
pub mod tile_data;
mod vfs;
use crate::metadata::Metadata;

pub const EXTENSION: &str = "lazuli";
pub const VERSION: u32 = 0;

struct Inner {
    path: Option<PathBuf>,
    conn: Connection,
}

/// A SQLite-backed `.lazuli` project archive storing document metadata, layer hierarchy, and tile data.
///
/// `LazuliArchive` serves as the native project persistence format. Internally, it manages a
/// thread-safe SQLite connection (either backed by a file on disk or held in memory) divided
/// into four logical tables:
///
/// - **`metadata`**: Format schema version ([`Metadata`]).
/// - **`image`**: Singleton document properties such as dimensions, tile size, ICC profile, and root layer ([`ImageProperties`](crate::image_props::ImageProperties)).
/// - **`layer_tree`**: Hierarchical layer nodes, ordering, types, and serialized properties ([`LayerNode`](crate::layer_tree::LayerNode)).
/// - **`tile_data`**: Compressed or raw pixel tile blobs indexed by layer UUID and 2D tile coordinates.
///
/// An archive created with [`new_in_memory`](Self::new_in_memory) can later be backed up and
/// transitioned to a file on disk via [`set_path`](Self::set_path).
#[derive(Clone)]
pub struct LazuliArchive {
    inner: Arc<Mutex<Inner>>,
}

impl fmt::Debug for LazuliArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            fs::create_dir_all(parent)?;
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

    /// Creates a new archive backed by an already-opened read-write [`File`] handle, truncating it first.
    ///
    /// Intended for handle-only storage without a filesystem path (such as Android SAF). Uses an
    /// internal file-handle SQLite VFS with an in-memory journal and no OS file locking. Prefer
    /// [`new`](Self::new) when a path is available.
    pub fn new_file(file: File) -> Result<Self> {
        file.set_len(0)?;
        let conn = vfs::open_connection(file)?;
        let archive = Self {
            inner: Arc::new(Mutex::new(Inner { path: None, conn })),
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

    /// Opens an existing archive from an already-opened read-write [`File`] handle.
    ///
    /// Intended for handle-only storage without a filesystem path (such as Android SAF). Uses an
    /// internal file-handle SQLite VFS with an in-memory journal and no OS file locking. Prefer
    /// [`open`](Self::open) when a path is available.
    pub fn open_file(file: File) -> Result<Self> {
        let conn = vfs::open_connection(file)?;
        let archive = Self {
            inner: Arc::new(Mutex::new(Inner { path: None, conn })),
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
            fs::create_dir_all(parent)?;
        }

        let mut inner = self.inner.lock();
        if inner.path.as_deref() == Some(path.as_path()) {
            return Ok(());
        }

        File::create_new(&path)?;
        let conn = inner
            .conn
            .backup(MAIN_DB, &path, NO_PROGRESS)
            .and_then(|_| Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE));
        let conn = match conn {
            Ok(conn) => conn,
            Err(error) => {
                let _ = fs::remove_file(&path);
                return Err(error.into());
            }
        };
        inner.conn = conn;
        inner.path = Some(path);

        Ok(())
    }

    pub fn set_file(&mut self, file: File) -> Result<()> {
        file.set_len(0)?;
        let mut dst = vfs::open_connection(file)?;
        let mut inner = self.inner.lock();
        {
            let backup = Backup::new(&inner.conn, &mut dst)?;
            backup.run_to_completion(100, Duration::ZERO, None)?;
        }
        inner.conn = dst;
        inner.path = None;

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
    use std::{env, fs};

    use uuid::Uuid;

    use crate::{image_props::ImageProperties, *};

    #[test]
    fn set_path_saves_an_in_memory_archive_and_switches_to_the_disk_database() {
        let directory = env::temp_dir().join(format!("lapiz_lazuli_set_path_{}", Uuid::new_v4()));
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
        let queued_write_archive = archive.clone();
        archive.set_path(path.clone()).unwrap();

        assert_eq!(queued_write_archive.path(), Some(path.clone()));
        assert!(path.is_file());

        queued_write_archive
            .write_image_properties(&ImageProperties {
                width: 512,
                height: 256,
                tile_size: 256,
                color_profile: vec![4, 5, 6, 7],
                root_layer,
                texel_type: 10,
            })
            .unwrap();
        drop(queued_write_archive);
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

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn set_path_does_not_overwrite_an_existing_archive() {
        let directory = env::temp_dir().join(format!("lapiz_lazuli_set_path_{}", Uuid::new_v4()));
        let path = directory.join("archive.lazuli");
        let existing = LazuliArchive::new(&path).unwrap();
        drop(existing);
        let contents = fs::read(&path).unwrap();
        let mut archive = LazuliArchive::new_in_memory().unwrap();

        assert!(archive.set_path(path.clone()).is_err());
        assert_eq!(archive.path(), None);
        assert_eq!(fs::read(&path).unwrap(), contents);

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn set_file_saves_in_memory_archive_and_persists_incremental_writes() {
        let directory = env::temp_dir().join(format!("lapiz_lazuli_set_file_{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("archive.lazuli");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();

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
        let queued_write_archive = archive.clone();
        archive.set_file(file).unwrap();

        assert_eq!(queued_write_archive.path(), None);

        queued_write_archive
            .write_image_properties(&ImageProperties {
                width: 512,
                height: 256,
                tile_size: 256,
                color_profile: vec![4, 5, 6, 7],
                root_layer,
                texel_type: 10,
            })
            .unwrap();
        drop(queued_write_archive);
        drop(archive);

        let reopened = LazuliArchive::open(&path).unwrap();
        let properties = reopened.read_image_properties().unwrap();
        assert_eq!(properties.width, 512);
        assert_eq!(properties.height, 256);
        assert_eq!(properties.color_profile, [4, 5, 6, 7]);
        assert_eq!(properties.root_layer, root_layer);
        assert_eq!(properties.texel_type, 10);
        drop(reopened);

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn open_file_reads_and_updates_existing_archive_in_place() {
        let directory = env::temp_dir().join(format!("lapiz_lazuli_open_file_{}", Uuid::new_v4()));
        let path = directory.join("archive.lazuli");
        let root_layer = Uuid::new_v4();

        let archive = LazuliArchive::new(&path).unwrap();
        archive
            .write_image_properties(&ImageProperties {
                width: 64,
                height: 64,
                tile_size: 256,
                color_profile: vec![1, 2],
                root_layer,
                texel_type: 8,
            })
            .unwrap();
        drop(archive);

        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let archive = LazuliArchive::open_file(file).unwrap();
        assert_eq!(archive.read_image_properties().unwrap().width, 64);

        archive
            .write_tile_data(root_layer, 1, -2, vec![9, 8, 7])
            .unwrap();
        drop(archive);

        let reopened = LazuliArchive::open(&path).unwrap();
        assert_eq!(
            reopened.read_tile_data(root_layer, 1, -2).unwrap(),
            Some(vec![9, 8, 7])
        );
        drop(reopened);

        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
