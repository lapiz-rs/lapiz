use std::{
    any::Any,
    collections::{HashMap, HashSet, hash_map::Entry},
    future::Future,
    iter,
    path::Path,
    pin::Pin,
    sync::Arc,
};

use anyhow::Result;
use iced_core::Element;
use iced_runtime::Task;
use lapiz_canvas::{CCanvas, CanvasId};
use lapiz_file_dialog::LocalFile;
use lapiz_runtime::{
    Renderer, Runtime, Theme,
    global::{Global, Globals},
    plugin::Plugin,
};

use crate::{
    adapter::{
        avif::AvifExporter,
        jpg::JpgExporter,
        lazuli::LazuliExporter,
        png::PngExporter,
        simple::{
            BmpExporter, FarbfeldExporter, GifExporter, HdrExporter, IcoExporter, OpenExrExporter,
            PnmExporter, QoiExporter, TgaExporter, TiffExporter, WebPExporter,
        },
    },
    config::ImageExporterConfig,
    export_dialog::ExportDialogView,
};

lapiz_i18n::define_i18n!("image_exporter");

pub mod adapter;
pub mod config;
pub mod export_dialog;

/// Plugin that initializes the image export subsystem, registers built-in [`ImageFormatExporter`]s, and sets up the [`ExportDialogView`].
pub struct ImageExporterPlugin;

impl Plugin for ImageExporterPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();

        app.add_global::<ImageFormatAdapterRegistry>()
            .add_global::<SilentSaveCanvases>()
            .register_view::<ExportDialogView>();

        app.globals_mut()
            .global_mut::<ImageFormatAdapterRegistry>()
            .register::<PngExporter>()
            .register::<JpgExporter>()
            .register::<WebPExporter>()
            .register::<AvifExporter>()
            .register::<LazuliExporter>()
            .register::<GifExporter>()
            .register::<BmpExporter>()
            .register::<TiffExporter>()
            .register::<TgaExporter>()
            .register::<QoiExporter>()
            .register::<FarbfeldExporter>()
            .register::<IcoExporter>()
            .register::<HdrExporter>()
            .register::<OpenExrExporter>()
            .register::<PnmExporter>();
    }
}

pub type ErasedExportDialogMessage = Box<dyn Any + Send>;

pub(crate) fn default_embed_profile() -> bool {
    true
}

/// Defines a format-specific exporter capable of saving a [`CCanvas`] to disk.
///
/// Implementations provide format metadata (primary file extension, extension aliases, and
/// localized description), an optional configuration UI rendered inside [`ExportDialogView`],
/// TOML settings persistence, and an asynchronous [`export`](Self::export) routine.
///
/// # Lifecycle
///
/// 1. Registered into [`ImageFormatAdapterRegistry`] at startup.
/// 2. Instantiated and populated with persisted settings from [`ImageExporterConfig`] when an
///    export is requested.
/// 3. Displayed in [`ExportDialogView`] if [`has_options`](Self::has_options) returns `true`.
/// 4. Executed via [`export`](Self::export) to write the canvas contents to the target path.
pub trait ImageFormatExporter: 'static {
    type DialogMessage: Send + 'static;

    fn extension() -> &'static str;

    fn aliases() -> &'static [&'static str] {
        &[]
    }

    fn description() -> String;

    fn has_options() -> bool {
        true
    }

    fn dialog_view<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Self::DialogMessage, Theme, Renderer>;

    fn dialog_update(
        &mut self,
        message: Self::DialogMessage,
        globals: &mut Globals,
    ) -> Task<Self::DialogMessage>;

    #[allow(
        async_fn_in_trait,
        reason = "callers await this method directly; the erased trait boxes the future"
    )]
    async fn export(&self, globals: &Globals, canvas: &CCanvas, path: &Path) -> Result<()>;

    fn to_toml(&self) -> Result<toml::Value>;

    #[allow(
        clippy::wrong_self_convention,
        reason = "pairs with to_toml and fills an existing exporter instead of converting from a value"
    )]
    fn from_toml(&mut self, value: toml::Value) -> Result<()>;
}

/// Type-erased wrapper trait for [`ImageFormatExporter`] used for dynamic storage and dispatch.
pub trait ErasedImageFormatAdapter: Send + Sync + 'static {
    fn extension(&self) -> &'static str;

    fn description(&self) -> String;

    fn has_options(&self) -> bool;

    fn dialog_view<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, ErasedExportDialogMessage, Theme, Renderer>;

    fn dialog_update(
        &mut self,
        message: ErasedExportDialogMessage,
        globals: &mut Globals,
    ) -> Task<ErasedExportDialogMessage>;

    fn export<'a>(
        &'a self,
        globals: &'a Globals,
        canvas: &'a CCanvas,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

    fn to_toml(&self) -> Result<toml::Value>;

    #[allow(
        clippy::wrong_self_convention,
        reason = "pairs with to_toml and fills an existing exporter instead of converting from a value"
    )]
    fn from_toml(&mut self, value: toml::Value) -> Result<()>;
}

impl<T> ErasedImageFormatAdapter for T
where
    T: ImageFormatExporter + Send + Sync,
{
    fn extension(&self) -> &'static str {
        T::extension()
    }

    fn description(&self) -> String {
        T::description()
    }

    fn has_options(&self) -> bool {
        T::has_options()
    }

    fn dialog_view<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, ErasedExportDialogMessage, Theme, Renderer> {
        self.dialog_view(globals)
            .map(|message| Box::new(message) as ErasedExportDialogMessage)
    }

    fn dialog_update(
        &mut self,
        message: ErasedExportDialogMessage,
        globals: &mut Globals,
    ) -> Task<ErasedExportDialogMessage> {
        let message = *message
            .downcast::<T::DialogMessage>()
            .expect("Invalid export dialog message type");
        self.dialog_update(message, globals)
            .map(|message| Box::new(message) as ErasedExportDialogMessage)
    }

    fn export<'a>(
        &'a self,
        globals: &'a Globals,
        canvas: &'a CCanvas,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        Box::pin(ImageFormatExporter::export(self, globals, canvas, path))
    }

    fn to_toml(&self) -> Result<toml::Value> {
        self.to_toml()
    }

    fn from_toml(&mut self, value: toml::Value) -> Result<()> {
        self.from_toml(value)
    }
}

/// Metadata describing a registered export format for file dialogs and format pickers.
pub struct ImageFormatInfo {
    pub extension: &'static str,
    pub aliases: &'static [&'static str],
    pub description: String,
}

struct AdapterEntry {
    extension: &'static str,
    aliases: &'static [&'static str],
    description: fn() -> String,
    construct: fn() -> Box<dyn ErasedImageFormatAdapter>,
}

/// Global registry of available [`ImageFormatExporter`] implementations indexed by file extension.
///
/// Maps primary extensions and aliases (case-insensitively) to factory functions that construct
/// type-erased [`ErasedImageFormatAdapter`] instances, optionally hydrated with saved user
/// preferences from [`ImageExporterConfig`].
#[derive(Default)]
pub struct ImageFormatAdapterRegistry {
    entries: Vec<AdapterEntry>,
    lookup: HashMap<String, usize>,
}

impl Global for ImageFormatAdapterRegistry {}

impl ImageFormatAdapterRegistry {
    pub fn register<A: ImageFormatExporter + Send + Sync + Default>(&mut self) -> &mut Self {
        let entry = AdapterEntry {
            extension: A::extension(),
            aliases: A::aliases(),
            description: A::description,
            construct: || Box::new(A::default()) as Box<dyn ErasedImageFormatAdapter>,
        };
        let index = self.entries.len();
        for key in iter::once(entry.extension)
            .chain(entry.aliases.iter().copied())
            .map(str::to_ascii_lowercase)
        {
            match self.lookup.entry(key) {
                Entry::Vacant(vacant) => {
                    vacant.insert(index);
                }
                Entry::Occupied(_) => {
                    log::error!(
                        "Image format adapter '{}' is already registered",
                        entry.extension
                    );
                    return self;
                }
            }
        }
        self.entries.push(entry);
        self
    }

    pub fn find_extension(&self, extension: &str) -> Option<&'static str> {
        let index = self.lookup.get(&extension.to_ascii_lowercase())?;
        Some(self.entries[*index].extension)
    }

    pub fn create(&self, extension: &str) -> Option<Box<dyn ErasedImageFormatAdapter>> {
        let index = *self.lookup.get(&extension.to_ascii_lowercase())?;
        Some((self.entries[index].construct)())
    }

    pub fn create_with_saved_settings(
        &self,
        extension: &str,
        config: &ImageExporterConfig,
    ) -> Option<Box<dyn ErasedImageFormatAdapter>> {
        let extension = self.find_extension(extension)?;
        let mut adapter = self.create(extension)?;
        if let Some(value) = config.adapters.get(extension)
            && let Err(error) = adapter.from_toml(value.clone())
        {
            log::warn!("Failed to restore {} export settings: {error}", extension);
        }
        Some(adapter)
    }

    pub fn iter_formats(&self) -> impl Iterator<Item = ImageFormatInfo> + '_ {
        self.entries.iter().map(|entry| ImageFormatInfo {
            extension: entry.extension,
            aliases: entry.aliases,
            description: (entry.description)(),
        })
    }
}

/// Parameters passed when opening [`ExportDialogView`] for a specific canvas and target file.
pub struct PendingExport {
    pub local_file: Arc<LocalFile>,
    pub allow_silent_export: bool,
    pub canvas_id: CanvasId,
}

/// Global set of [`CanvasId`]s that have opted into silent subsequent saves without re-opening the export dialog.
#[derive(Default)]
pub struct SilentSaveCanvases {
    canvases: HashSet<CanvasId>,
}

impl Global for SilentSaveCanvases {}

impl SilentSaveCanvases {
    pub fn insert(&mut self, canvas: CanvasId) {
        self.canvases.insert(canvas);
    }

    pub fn remove(&mut self, canvas: CanvasId) {
        self.canvases.remove(&canvas);
    }

    pub fn contains(&self, canvas: CanvasId) -> bool {
        self.canvases.contains(&canvas)
    }
}
