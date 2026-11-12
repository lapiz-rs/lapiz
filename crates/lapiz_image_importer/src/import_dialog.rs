use std::sync::Arc;

use anyhow::Result;
use futures::executor::block_on;
use iced_core::{Element, Length, Size, Theme, window};
use iced_runtime::{
    Task,
    window::{close, open},
};
use lapiz_canvas::{CCanvas, CanvasAppExt as _};
use lapiz_config::Config;
use lapiz_file_dialog::LocalFile;
use lapiz_i18n::t;
use lapiz_image::CImage;
use lapiz_runtime::{
    Renderer,
    global::Globals,
    windows::{WindowView, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{button, column, label, panel, row, space};

use crate::{
    ErasedImportDialogMessage, ImageImporterRegistry, PendingImport, config::ImageImporterConfig,
};

pub const IMPORT_DIALOG_VIEW_ID: &str = "import_dialog";

/// Modal window view that presents format-specific import options before opening an image as a [`CCanvas`].
pub struct ImportDialogView {
    window: window::Id,
    windows: Arc<[window::Id]>,
    importer: Box<dyn crate::ErasedImageFormatImporter>,
    extension: &'static str,
    local_file: Option<LocalFile>,
}

/// UI messages handled by [`ImportDialogView`].
pub enum ImportDialogMessage {
    Importer(ErasedImportDialogMessage),
    Confirm,
    Cancel,
}

impl WindowView for ImportDialogView {
    type Message = ImportDialogMessage;
    type BootParams = PendingImport;

    fn id() -> WindowViewId {
        WindowViewId::new(IMPORT_DIALOG_VIEW_ID)
    }

    fn boot(
        params: Option<Self::BootParams>,
        globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)> {
        let pending = params.ok_or(anyhow::anyhow!("No pending import"))?;
        let registry = globals.global::<ImageImporterRegistry>();
        let extension = pending
            .local_file
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| registry.find_extension(extension))
            .expect("Import dialog opened for a path without a registered format");
        let config = Config::<ImageImporterConfig>::read_or_init_or_fallback();
        let importer = registry
            .create_with_saved_settings(extension, &config.get())
            .expect("Import dialog opened for a path without a registered format");
        let (window, open) = open(window::Settings {
            size: Size {
                width: 420.0,
                height: 300.0,
            },
            ..Default::default()
        });

        Ok((
            Self {
                window,
                windows: Arc::from([window]),
                importer,
                extension,
                local_file: Some(pending.local_file),
            },
            open.discard(),
        ))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        globals: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let options = self
            .importer
            .dialog_view(globals)
            .map(ImportDialogMessage::Importer);
        let footer = row![
            space().width(Length::Fill),
            button(label(t!("cancel"))).on_press(ImportDialogMessage::Cancel),
            button(label(t!("import")))
                .primary()
                .on_press(ImportDialogMessage::Confirm),
        ]
        .gap(10.0);

        column![
            panel(
                column![
                    label(
                        self.local_file
                            .as_ref()
                            .expect("import is active")
                            .path()
                            .display()
                            .to_string()
                    )
                    .muted(),
                    options
                ]
                .gap(10.0),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(12),
            footer,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .gap(8.0)
        .padding(8)
    }

    fn update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            ImportDialogMessage::Importer(message) => self
                .importer
                .dialog_update(message, globals)
                .map(ImportDialogMessage::Importer),
            ImportDialogMessage::Confirm => {
                let Some(local_file) = self.local_file.as_ref() else {
                    return close(self.window);
                };
                let archive =
                    block_on(self.importer.import(globals, local_file.path())).logged_err();
                if let Ok(archive) = archive
                    && let Ok(image) = CImage::from_lazuli(&archive, globals).logged_err()
                {
                    globals.add_canvas(CCanvas::new(self.local_file.take(), image, archive));
                }

                Config::<ImageImporterConfig>::read_or_init_or_fallback()
                    .update(|config| match self.importer.to_toml() {
                        Ok(value) => {
                            config.importers.insert(self.extension.to_string(), value);
                        }
                        Err(error) => log::error!("Failed to serialize import settings: {error}"),
                    })
                    .log_err();
                close(self.window)
            }
            ImportDialogMessage::Cancel => close(self.window),
        }
    }

    fn close(self, _: &mut Globals) -> Task<()> {
        close(self.window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.window)
    }
}
