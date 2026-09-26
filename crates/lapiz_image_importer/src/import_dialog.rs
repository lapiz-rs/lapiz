use std::sync::Arc;

use anyhow::Result;
use futures::executor::block_on;
use iced_core::{Alignment, Element, Length, Size, Theme, window};
use iced_runtime::{
    Task,
    window::{close, open},
};
use iced_widget::{Space, column, row};
use lapiz_canvas::{CCanvas, CanvasAppExt as _};
use lapiz_config::Config;
use lapiz_file_dialog::LocalFile;
use lapiz_i18n::t;
use lapiz_image::CImage;
use lapiz_runtime::{
    Renderer, Services,
    windows::{WindowView, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{button::Button, label::Label, panel::Panel};

use crate::{
    ErasedImportDialogMessage, ImageImporterRegistry, PendingImport, config::ImageImporterConfig,
};

pub const IMPORT_DIALOG_VIEW_ID: &str = "import_dialog";

pub struct ImportDialogView {
    window: window::Id,
    windows: Arc<[window::Id]>,
    importer: Box<dyn crate::ErasedImageFormatImporter>,
    extension: &'static str,
    local_file: Option<LocalFile>,
}

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
        services: &mut Services,
    ) -> Result<(Self, Task<Self::Message>)> {
        let pending = params.ok_or(anyhow::anyhow!("No pending import"))?;
        let registry = services.service::<ImageImporterRegistry>();
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
        services: &'a Services,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let options = self
            .importer
            .dialog_view(services)
            .map(ImportDialogMessage::Importer);
        let footer = row![
            Space::new().width(Length::Fill),
            Button::new(Label::new(t!("cancel"))).on_press(ImportDialogMessage::Cancel),
            Button::new(Label::new(t!("import")))
                .primary()
                .on_press(ImportDialogMessage::Confirm),
        ]
        .align_y(Alignment::Center)
        .spacing(10);

        column![
            Panel::new(
                column![
                    Label::new(
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
                .spacing(10),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(12),
            footer,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .spacing(8)
        .padding(8)
    }

    fn update(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            ImportDialogMessage::Importer(message) => self
                .importer
                .dialog_update(message, services)
                .map(ImportDialogMessage::Importer),
            ImportDialogMessage::Confirm => {
                let Some(local_file) = self.local_file.as_ref() else {
                    return close(self.window);
                };
                let archive =
                    block_on(self.importer.import(services, local_file.path())).logged_err();
                if let Ok(archive) = archive
                    && let Ok(image) = CImage::from_lazuli(&archive, services).logged_err()
                {
                    services.add_canvas(CCanvas::new(
                        self.local_file.take().unwrap(),
                        image,
                        archive,
                    ));
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

    fn close(self, _: &mut Services) -> Task<()> {
        close(self.window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.window)
    }
}
