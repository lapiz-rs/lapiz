use std::{any::Any, collections::HashMap, future::Future, iter, path::Path, pin::Pin};

use anyhow::Result;
use futures::executor::block_on;
use iced_core::Element;
use iced_runtime::Task;
use lapiz_canvas::{CCanvas, CanvasAppExt as _};
use lapiz_config::Config;
use lapiz_file_dialog::LocalFile;
use lapiz_image::CImage;
use lapiz_lazuli::LazuliArchive;
use lapiz_runtime::{
    Application, Renderer, Services, Theme,
    plugin::Plugin,
    service::Service,
    windows::{OpenWindowViewCommand, WindowCommandBuffer, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;

use crate::{
    config::ImageImporterConfig,
    import_dialog::{IMPORT_DIALOG_VIEW_ID, ImportDialogView},
    importer::{
        lazuli::LazuliImporter,
        simple::{
            AvifImporter, BmpImporter, FarbfeldImporter, GifImporter, HdrImporter, IcoImporter,
            JpgImporter, OpenExrImporter, PngImporter, PnmImporter, QoiImporter, TgaImporter,
            TiffImporter, WebPImporter,
        },
    },
};

lapiz_i18n::define_i18n!("image_importer");

pub mod config;
pub mod import_dialog;
pub mod importer;

pub struct ImageImporterPlugin;

impl Plugin for ImageImporterPlugin {
    fn build(&self, app: &mut Application) {
        i18n::init();

        let mut runtime = app.runtime_mut();
        runtime.add_service::<ImageImporterRegistry>();
        runtime
            .window_manager_mut()
            .register_view::<ImportDialogView>();

        let services = runtime.services_mut();
        services
            .service_mut::<ImageImporterRegistry>()
            .register::<PngImporter>()
            .register::<JpgImporter>()
            .register::<WebPImporter>()
            .register::<AvifImporter>()
            .register::<LazuliImporter>()
            .register::<GifImporter>()
            .register::<BmpImporter>()
            .register::<TiffImporter>()
            .register::<TgaImporter>()
            .register::<QoiImporter>()
            .register::<FarbfeldImporter>()
            .register::<IcoImporter>()
            .register::<HdrImporter>()
            .register::<OpenExrImporter>()
            .register::<PnmImporter>();
    }
}

pub type ErasedImportDialogMessage = Box<dyn Any + Send>;

pub trait ImageFormatImporter: 'static {
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
        services: &'a Services,
    ) -> Element<'a, Self::DialogMessage, Theme, Renderer>;

    fn dialog_update(
        &mut self,
        message: Self::DialogMessage,
        services: &mut Services,
    ) -> Task<Self::DialogMessage>;

    #[allow(
        async_fn_in_trait,
        reason = "callers await this method directly; the erased trait boxes the future"
    )]
    async fn import(&self, services: &Services, path: &Path) -> Result<LazuliArchive>;

    fn to_toml(&self) -> Result<toml::Value>;

    #[allow(
        clippy::wrong_self_convention,
        reason = "pairs with to_toml and fills an existing importer instead of converting from a value"
    )]
    fn from_toml(&mut self, value: toml::Value) -> Result<()>;
}

pub trait ErasedImageFormatImporter: Send + Sync + 'static {
    fn extension(&self) -> &'static str;
    fn description(&self) -> String;
    fn has_options(&self) -> bool;

    fn dialog_view<'a>(
        &'a self,
        services: &'a Services,
    ) -> Element<'a, ErasedImportDialogMessage, Theme, Renderer>;

    fn dialog_update(
        &mut self,
        message: ErasedImportDialogMessage,
        services: &mut Services,
    ) -> Task<ErasedImportDialogMessage>;

    fn import<'a>(
        &'a self,
        services: &'a Services,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<LazuliArchive>> + 'a>>;

    fn to_toml(&self) -> Result<toml::Value>;

    #[allow(
        clippy::wrong_self_convention,
        reason = "pairs with to_toml and fills an existing importer instead of converting from a value"
    )]
    fn from_toml(&mut self, value: toml::Value) -> Result<()>;
}

impl<T> ErasedImageFormatImporter for T
where
    T: ImageFormatImporter + Send + Sync,
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
        services: &'a Services,
    ) -> Element<'a, ErasedImportDialogMessage, Theme, Renderer> {
        self.dialog_view(services)
            .map(|message| Box::new(message) as ErasedImportDialogMessage)
    }

    fn dialog_update(
        &mut self,
        message: ErasedImportDialogMessage,
        services: &mut Services,
    ) -> Task<ErasedImportDialogMessage> {
        let message = *message
            .downcast::<T::DialogMessage>()
            .expect("Invalid import dialog message type");
        self.dialog_update(message, services)
            .map(|message| Box::new(message) as ErasedImportDialogMessage)
    }

    fn import<'a>(
        &'a self,
        services: &'a Services,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<LazuliArchive>> + 'a>> {
        Box::pin(ImageFormatImporter::import(self, services, path))
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

struct ImporterEntry {
    extension: &'static str,
    aliases: &'static [&'static str],
    description: fn() -> String,
    construct: fn() -> Box<dyn ErasedImageFormatImporter>,
}

#[derive(Default)]
pub struct ImageImporterRegistry {
    entries: Vec<ImporterEntry>,
    lookup: HashMap<String, usize>,
}

impl Service for ImageImporterRegistry {}

impl ImageImporterRegistry {
    pub fn register<A: ImageFormatImporter + Send + Sync + Default>(&mut self) -> &mut Self {
        let entry = ImporterEntry {
            extension: A::extension(),
            aliases: A::aliases(),
            description: A::description,
            construct: || Box::new(A::default()) as Box<dyn ErasedImageFormatImporter>,
        };
        let index = self.entries.len();
        let keys = iter::once(entry.extension)
            .chain(entry.aliases.iter().copied())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        if keys.iter().any(|key| self.lookup.contains_key(key)) {
            log::error!("Image importer '{}' is already registered", entry.extension);
            return self;
        }
        self.entries.push(entry);
        for key in keys {
            self.lookup.insert(key, index);
        }
        self
    }

    pub fn find_extension(&self, extension: &str) -> Option<&'static str> {
        let index = self.lookup.get(&extension.to_ascii_lowercase())?;
        Some(self.entries[*index].extension)
    }

    pub fn create(&self, extension: &str) -> Option<Box<dyn ErasedImageFormatImporter>> {
        let index = *self.lookup.get(&extension.to_ascii_lowercase())?;
        Some((self.entries[index].construct)())
    }

    pub fn create_with_saved_settings(
        &self,
        extension: &str,
        config: &ImageImporterConfig,
    ) -> Option<Box<dyn ErasedImageFormatImporter>> {
        let extension = self.find_extension(extension)?;
        let mut importer = self.create(extension)?;
        if let Some(value) = config.importers.get(extension)
            && let Err(error) = importer.from_toml(value.clone())
        {
            log::warn!("Failed to restore {extension} import settings: {error}");
        }
        Some(importer)
    }

    pub fn iter_formats(&self) -> impl Iterator<Item = ImageFormatInfo> + '_ {
        self.entries.iter().map(|entry| ImageFormatInfo {
            extension: entry.extension,
            aliases: entry.aliases,
            description: (entry.description)(),
        })
    }
}

pub struct PendingImport {
    pub local_file: LocalFile,
}

pub fn start_import(services: &mut Services, local_file: LocalFile) {
    let path = local_file.path();
    let Some(path_extension) = path.extension().and_then(|extension| extension.to_str()) else {
        log::warn!(
            "Cannot import a path without an extension: {}",
            path.display()
        );
        return;
    };
    let registry = services.service::<ImageImporterRegistry>();
    let Some(extension) = registry.find_extension(path_extension) else {
        log::warn!("No image importer matches {}", path.display());
        return;
    };
    let config = Config::<ImageImporterConfig>::read_or_init_or_fallback();
    let Some(importer) = registry.create_with_saved_settings(extension, &config.get()) else {
        return;
    };

    if importer.has_options() {
        services
            .service_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new_with_params(
                WindowViewId::new(IMPORT_DIALOG_VIEW_ID),
                PendingImport { local_file },
            ));
    } else {
        let archive = block_on(importer.import(services, path)).logged_err();
        if let Ok(archive) = archive
            && let Ok(image) = CImage::from_lazuli(&archive, services).logged_err()
        {
            services.add_canvas(CCanvas::new(local_file, image, archive));
        }
    }
}
