use std::{
    any::Any,
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
};

use anyhow::Result;
use iced_core::Element;
use iced_runtime::Task;
use lapiz_canvas::CanvasId;
use lapiz_runtime::{Renderer, Services, Theme, plugin::Plugin, service::Service};

lapiz_i18n::define_i18n!("image_adapter");

pub mod config;
pub mod export_dialog;
mod jpg;
mod lazuli;
mod pixels;
mod png;

pub use config::ImageAdapterConfig;
pub use export_dialog::{EXPORT_DIALOG_VIEW_ID, ExportDialogView};
pub use jpg::JpgAdapter;
pub use lazuli::LazuliAdapter;
pub use png::PngAdapter;

pub struct ImageAdapterPlugin;

impl Plugin for ImageAdapterPlugin {
    fn build(&self, app: &mut lapiz_runtime::Application) {
        let mut runtime = app.runtime_mut();
        runtime.add_service::<ImageFormatAdapterRegistry>();
        runtime.add_service::<SilentSaveCanvases>();
        let services = runtime.services_mut();
        services
            .service_mut::<ImageFormatAdapterRegistry>()
            .register::<PngAdapter>()
            .register::<JpgAdapter>()
            .register::<LazuliAdapter>();
        i18n::init();
    }
}

pub type ErasedExportDialogMessage = Box<dyn Any + Send>;

pub trait ImageFormatAdapter: 'static {
    type ExportDialogMessage: Send + 'static;

    fn extension() -> &'static str;

    fn aliases() -> &'static [&'static str] {
        &[]
    }

    fn description() -> String;

    fn has_export_options() -> bool {
        true
    }

    fn export_dialog_view<'a>(
        &'a self,
        services: &'a Services,
    ) -> Element<'a, Self::ExportDialogMessage, Theme, Renderer>;

    fn export_dialog_update(
        &mut self,
        message: Self::ExportDialogMessage,
        services: &mut Services,
    ) -> Task<Self::ExportDialogMessage>;

    #[allow(async_fn_in_trait)]
    async fn export(&self, services: &Services, path: &Path) -> Result<()>;

    fn to_toml(&self) -> Result<toml::Value>;

    #[allow(clippy::wrong_self_convention)]
    fn from_toml(&mut self, value: toml::Value) -> Result<()>;
}

pub trait ErasedImageFormatAdapter: Send + Sync + 'static {
    fn extension(&self) -> &'static str;

    fn description(&self) -> String;

    fn has_export_options(&self) -> bool;

    fn export_dialog_view<'a>(
        &'a self,
        services: &'a Services,
    ) -> Element<'a, ErasedExportDialogMessage, Theme, Renderer>;

    fn export_dialog_update(
        &mut self,
        message: ErasedExportDialogMessage,
        services: &mut Services,
    ) -> Task<ErasedExportDialogMessage>;

    fn export<'a>(
        &'a self,
        services: &'a Services,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>>;

    fn to_toml(&self) -> Result<toml::Value>;

    #[allow(clippy::wrong_self_convention)]
    fn from_toml(&mut self, value: toml::Value) -> Result<()>;
}

impl<T> ErasedImageFormatAdapter for T
where
    T: ImageFormatAdapter + Send + Sync,
{
    fn extension(&self) -> &'static str {
        T::extension()
    }

    fn description(&self) -> String {
        T::description()
    }

    fn has_export_options(&self) -> bool {
        T::has_export_options()
    }

    fn export_dialog_view<'a>(
        &'a self,
        services: &'a Services,
    ) -> Element<'a, ErasedExportDialogMessage, Theme, Renderer> {
        self.export_dialog_view(services)
            .map(|message| Box::new(message) as ErasedExportDialogMessage)
    }

    fn export_dialog_update(
        &mut self,
        message: ErasedExportDialogMessage,
        services: &mut Services,
    ) -> Task<ErasedExportDialogMessage> {
        let message = *message
            .downcast::<T::ExportDialogMessage>()
            .expect("Invalid export dialog message type");
        self.export_dialog_update(message, services)
            .map(|message| Box::new(message) as ErasedExportDialogMessage)
    }

    fn export<'a>(
        &'a self,
        services: &'a Services,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        Box::pin(ImageFormatAdapter::export(self, services, path))
    }

    fn to_toml(&self) -> Result<toml::Value> {
        self.to_toml()
    }

    fn from_toml(&mut self, value: toml::Value) -> Result<()> {
        self.from_toml(value)
    }
}

pub struct ImageFormatInfo {
    pub extension: &'static str,
    pub aliases: &'static [&'static str],
    pub description: String,
}

struct AdapterEntry {
    extension: &'static str,
    aliases: &'static [&'static str],
    description: String,
    construct: fn() -> Box<dyn ErasedImageFormatAdapter>,
}

#[derive(Default)]
pub struct ImageFormatAdapterRegistry {
    entries: Vec<AdapterEntry>,
    lookup: HashMap<String, usize>,
}

impl Service for ImageFormatAdapterRegistry {}

impl ImageFormatAdapterRegistry {
    pub fn register<A: ImageFormatAdapter + Send + Sync + Default>(&mut self) -> &mut Self {
        let entry = AdapterEntry {
            extension: A::extension(),
            aliases: A::aliases(),
            description: A::description(),
            construct: || Box::new(A::default()) as Box<dyn ErasedImageFormatAdapter>,
        };
        let index = self.entries.len();
        for key in std::iter::once(entry.extension)
            .chain(entry.aliases.iter().copied())
            .map(str::to_ascii_lowercase)
        {
            match self.lookup.entry(key) {
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(index);
                }
                std::collections::hash_map::Entry::Occupied(_) => {
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
        config: &ImageAdapterConfig,
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
            description: entry.description.clone(),
        })
    }
}

pub struct PendingExport {
    pub path: PathBuf,
    pub allow_silent_export: bool,
    pub canvas_id: CanvasId,
}

impl Service for PendingExport {}

#[derive(Default)]
pub struct SilentSaveCanvases {
    canvases: HashSet<CanvasId>,
}

impl Service for SilentSaveCanvases {}

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
