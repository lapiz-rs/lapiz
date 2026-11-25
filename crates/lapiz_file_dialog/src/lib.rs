use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
#[cfg(target_os = "android")]
use std::{
    fs::File,
    sync::{Mutex, OnceLock},
};

#[cfg(target_os = "android")]
use anyhow::Context as _;
use anyhow::Result;
#[cfg(target_os = "android")]
use lapiz_runtime::android::AndroidApp;
use lapiz_runtime::global::Globals;
#[cfg(not(target_os = "android"))]
use rfd::AsyncFileDialog;

#[cfg(target_os = "android")]
mod android;

/// Cross-platform asynchronous file open/save dialog builder.
///
/// Wraps `rfd::AsyncFileDialog` on desktop platforms and bridges to the native Android Storage
/// Access Framework document picker on Android, returning a [`LocalFile`] handle when a file is
/// selected.
pub struct FileDialog {
    #[cfg(not(target_os = "android"))]
    dialog: AsyncFileDialog,

    #[cfg(target_os = "android")]
    app: AndroidApp,
    #[cfg(target_os = "android")]
    file_name: Option<String>,
}

impl FileDialog {
    // quick helper
    pub fn new_maybe_from_global(globals: &Globals) -> Self {
        #[cfg(target_os = "android")]
        {
            use lapiz_runtime::android::AndroidAppExt as _;

            FileDialog::new_android(globals.android_app().clone())
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = globals;
            FileDialog::new()
        }
    }

    #[cfg(not(target_os = "android"))]
    #[allow(
        clippy::new_without_default,
        reason = "Parameter of `new` various between platforms"
    )]
    pub fn new() -> Self {
        Self {
            dialog: AsyncFileDialog::new(),
        }
    }

    #[cfg(target_os = "android")]
    pub fn new_android(app: AndroidApp) -> Self {
        Self {
            app,
            file_name: None,
        }
    }

    #[must_use]
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        #[cfg(not(target_os = "android"))]
        {
            self.dialog = self.dialog.set_file_name(name);
        }
        #[cfg(target_os = "android")]
        {
            self.file_name = Some(name);
        }
        self
    }

    #[must_use]
    pub fn add_filter(self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        #[cfg(not(target_os = "android"))]
        {
            Self {
                dialog: self.dialog.add_filter(name, extensions),
            }
        }
        #[cfg(target_os = "android")]
        {
            let _ = (name, extensions);
            self
        }
    }

    pub async fn pick_file(self) -> Result<Option<LocalFile>> {
        #[cfg(not(target_os = "android"))]
        {
            Ok(self
                .dialog
                .pick_file()
                .await
                .map(|file| LocalFile::native(file.path().to_path_buf())))
        }
        #[cfg(target_os = "android")]
        {
            let Some(document) = android::pick(&self.app, false, "").await? else {
                return Ok(None);
            };
            Ok(Some(LocalFile::open_android(
                self.app,
                document.uri,
                document.name,
            )))
        }
    }

    pub async fn save_file(self) -> Result<Option<LocalFile>> {
        #[cfg(not(target_os = "android"))]
        {
            Ok(self
                .dialog
                .save_file()
                .await
                .map(|file| LocalFile::native(file.path().to_path_buf())))
        }
        #[cfg(target_os = "android")]
        {
            let name = self.file_name.context("Document name is required")?;
            let Some(document) = android::pick(&self.app, true, &name).await? else {
                return Ok(None);
            };
            Ok(Some(LocalFile {
                name: document.name,
                app: self.app,
                uri: document.uri,
                existing: false,
                staged: OnceLock::new(),
                stage_lock: Mutex::new(()),
            }))
        }
    }
}

#[cfg(target_os = "android")]
struct StagedFile {
    path: PathBuf,
    _temp: tempfile::TempDir,
}

/// A local filesystem handle to a file chosen through [`FileDialog`].
///
/// On desktop platforms this points directly to the selected path. On Android, it holds the
/// Storage Access Framework content URI and supports either direct file-descriptor access via
/// [`open_read_write`](LocalFile::open_read_write) or lazy temporary staging via
/// [`materialize_path`](LocalFile::materialize_path) and [`commit`](LocalFile::commit).
pub struct LocalFile {
    #[cfg(not(target_os = "android"))]
    path: PathBuf,
    name: String,
    #[cfg(target_os = "android")]
    app: AndroidApp,
    #[cfg(target_os = "android")]
    uri: String,
    #[cfg(target_os = "android")]
    existing: bool,
    #[cfg(target_os = "android")]
    staged: OnceLock<StagedFile>,
    #[cfg(target_os = "android")]
    stage_lock: Mutex<()>,
}

impl LocalFile {
    #[cfg(not(target_os = "android"))]
    pub fn native(path: PathBuf) -> Self {
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        LocalFile { path, name }
    }

    // TODO: maybe query the name on the fly?
    #[cfg(target_os = "android")]
    pub fn open_android(app: AndroidApp, uri: String, name: String) -> Self {
        LocalFile {
            name,
            app,
            uri,
            existing: true,
            staged: OnceLock::new(),
            stage_lock: Mutex::new(()),
        }
    }

    #[cfg(not(target_os = "android"))]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn materialize_path(&self) -> Result<&Path> {
        #[cfg(not(target_os = "android"))]
        {
            Ok(&self.path)
        }
        #[cfg(target_os = "android")]
        {
            if let Some(staged) = self.staged.get() {
                return Ok(&staged.path);
            }

            let _guard = self.stage_lock.lock().expect("Stage lock poisoned");
            if let Some(staged) = self.staged.get() {
                return Ok(&staged.path);
            }

            let (path, directory) = android::temp_file(&self.name)?;
            if self.existing {
                android::copy_file(&self.app, &self.uri, &path)?;
            }
            let staged = self.staged.get_or_init(|| StagedFile {
                path,
                _temp: directory,
            });
            Ok(&staged.path)
        }
    }

    #[cfg(target_os = "android")]
    pub fn open_read_write(&self) -> Result<File> {
        android::open_file(&self.app, &self.uri, "rw")
    }

    pub fn extension(&self) -> Option<&str> {
        #[cfg(not(target_os = "android"))]
        {
            self.path.extension().and_then(OsStr::to_str)
        }
        #[cfg(target_os = "android")]
        {
            Path::new(&self.name).extension().and_then(OsStr::to_str)
        }
    }

    pub fn file_name(&self) -> Option<&str> {
        #[cfg(not(target_os = "android"))]
        {
            self.path.file_name().and_then(OsStr::to_str)
        }
        #[cfg(target_os = "android")]
        {
            Path::new(&self.name).file_name().and_then(OsStr::to_str)
        }
    }

    pub fn display_path(&self) -> String {
        #[cfg(not(target_os = "android"))]
        {
            self.path.display().to_string()
        }
        #[cfg(target_os = "android")]
        {
            self.name.clone()
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source(&self) -> String {
        #[cfg(target_os = "android")]
        {
            self.uri.clone()
        }
        #[cfg(not(target_os = "android"))]
        {
            self.path.to_string_lossy().into_owned()
        }
    }

    pub fn commit(&self) -> Result<()> {
        #[cfg(target_os = "android")]
        if let Some(staged) = self.staged.get() {
            android::write_file(&self.app, &self.uri, &staged.path)?;
        }

        Ok(())
    }
}
