use std::sync::Arc;

use anyhow::Result;
use futures::executor::block_on;
use iced_core::{Alignment, Element, Length, Size, Theme, window};
use iced_runtime::{
    Task,
    window::{close, open},
};
use iced_widget::{Space, column, row};
use lapiz_canvas::{CanvasAppExt as _, CanvasId};
use lapiz_config::Config;
use lapiz_file_dialog::LocalFile;
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer, Services,
    windows::{WindowView, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    button::Button, checkbox::Checkbox, fluent_builder::When as _, label::Label, panel::Panel,
};

use crate::{
    ErasedExportDialogMessage, ImageExporterConfig, ImageFormatAdapterRegistry, PendingExport,
    SilentSaveCanvases,
};

pub const EXPORT_DIALOG_VIEW_ID: &str = "export_dialog";

pub struct ExportDialogView {
    window: window::Id,
    windows: Arc<[window::Id]>,
    adapter: Box<dyn crate::ErasedImageFormatAdapter>,
    extension: &'static str,
    local_file: Arc<LocalFile>,
    canvas_id: CanvasId,
    allow_silent_export: bool,
    dont_ask_again: bool,
}

pub enum ExportDialogMessage {
    Adapter(ErasedExportDialogMessage),
    DontAskAgainToggled(bool),
    Confirm,
    Cancel,
}

impl WindowView for ExportDialogView {
    type Message = ExportDialogMessage;

    type BootParams = PendingExport;

    fn id() -> WindowViewId {
        WindowViewId::new(EXPORT_DIALOG_VIEW_ID)
    }

    fn boot(
        params: Option<Self::BootParams>,
        services: &mut Services,
    ) -> Result<(Self, Task<Self::Message>)> {
        let pending = params.ok_or(anyhow::anyhow!("No pending export"))?;
        let registry = services.service::<ImageFormatAdapterRegistry>();
        let extension = pending
            .local_file
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| registry.find_extension(extension))
            .expect("Export dialog opened for a path without a registered format");
        let config = Config::<ImageExporterConfig>::read_or_init_or_fallback();
        let adapter = registry
            .create_with_saved_settings(extension, &config.get())
            .expect("Export dialog opened for a path without a registered format");
        let dont_ask_again = services
            .service::<SilentSaveCanvases>()
            .contains(pending.canvas_id);

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
                adapter,
                extension,
                local_file: pending.local_file,
                canvas_id: pending.canvas_id,
                allow_silent_export: pending.allow_silent_export,
                dont_ask_again,
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
            .adapter
            .dialog_view(services)
            .map(ExportDialogMessage::Adapter);
        let footer = row![]
            .when(self.allow_silent_export, |r| {
                r.push(
                    Checkbox::new(self.dont_ask_again)
                        .label(t!("dont_ask_again"))
                        .on_toggle(ExportDialogMessage::DontAskAgainToggled),
                )
            })
            .extend([
                Space::new().width(Length::Fill).into(),
                Button::new(Label::new(t!("cancel")))
                    .on_press(ExportDialogMessage::Cancel)
                    .into(),
                Button::new(Label::new(t!("export")))
                    .primary()
                    .on_press(ExportDialogMessage::Confirm)
                    .into(),
            ])
            .align_y(Alignment::Center)
            .spacing(10);

        column![
            Panel::new(
                column![
                    Label::new(self.local_file.path().display().to_string()).muted(),
                    options,
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
            ExportDialogMessage::Adapter(message) => self
                .adapter
                .dialog_update(message, services)
                .map(ExportDialogMessage::Adapter),
            ExportDialogMessage::DontAskAgainToggled(checked) => {
                self.dont_ask_again = checked;
                Task::none()
            }
            ExportDialogMessage::Confirm => {
                let Some(canvas) = services.canvas(&self.canvas_id) else {
                    return Task::done(ExportDialogMessage::Cancel);
                };

                // TODO use async
                if block_on(
                    self.adapter
                        .export(services, canvas, self.local_file.path()),
                )
                .and_then(|_| self.local_file.commit())
                .logged_err()
                .is_err()
                {
                    return Task::none();
                }

                let silent_saves = services.service_mut::<SilentSaveCanvases>();
                if self.dont_ask_again {
                    silent_saves.insert(self.canvas_id);
                } else {
                    silent_saves.remove(self.canvas_id);
                }
                Config::<ImageExporterConfig>::read_or_init_or_fallback()
                    .update(|config| match self.adapter.to_toml() {
                        Ok(value) => {
                            config.adapters.insert(self.extension.to_string(), value);
                        }
                        Err(error) => {
                            log::error!("Failed to serialize export settings: {error}")
                        }
                    })
                    .log_err();
                close(self.window)
            }
            ExportDialogMessage::Cancel => close(self.window),
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
