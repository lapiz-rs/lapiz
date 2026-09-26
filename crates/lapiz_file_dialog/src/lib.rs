use std::path::{Path, PathBuf};

#[cfg(target_os = "android")]
use anyhow::Context as _;
use anyhow::Result;
use lapiz_runtime::Services;
#[cfg(target_os = "android")]
use lapiz_runtime::android::AndroidApp;
#[cfg(not(target_os = "android"))]
use rfd::AsyncFileDialog;

#[cfg(target_os = "android")]
mod android;

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
    pub fn new_maybe_from_service(services: &Services) -> Self {
        #[cfg(target_os = "android")]
        {
            use lapiz_runtime::android::AndroidAppExt as _;

            FileDialog::new_android(services.android_app().clone())
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = services;
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
            LocalFile::open_android(self.app, document.uri, document.name).map(Some)
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
            let (path, directory) = android::temp_file(&document.name)?;
            Ok(Some(LocalFile {
                path,
                name: document.name,
                app: self.app,
                uri: document.uri,
                _temp: directory,
            }))
        }
    }
}

pub struct LocalFile {
    path: PathBuf,
    name: String,
    #[cfg(target_os = "android")]
    app: AndroidApp,
    #[cfg(target_os = "android")]
    uri: String,
    #[cfg(target_os = "android")]
    _temp: tempfile::TempDir,
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
    pub fn open_android(app: AndroidApp, uri: String, name: String) -> Result<Self> {
        let (path, directory) = android::temp_file(&name)?;
        android::copy_file(&app, &uri, &path)?;
        Ok(LocalFile {
            path,
            name,
            app,
            uri,
            _temp: directory,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
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
        android::write_file(&self.app, &self.uri, &self.path)?;

        Ok(())
    }
}
