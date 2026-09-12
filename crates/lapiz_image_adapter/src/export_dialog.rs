use std::{path::PathBuf, sync::Arc};

use iced_core::{Alignment, Element, Length, Size, Theme, window};
use iced_runtime::Task;
use iced_widget::{Space, column, row};
use lapiz_canvas::{CanvasAppExt, CanvasId};
use lapiz_config::Config;
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer, Services,
    windows::{WindowView, WindowViewId},
};
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{
    button::Button, checkbox::Checkbox, fluent_builder::When, label::Label, panel::Panel,
};

use crate::{
    ErasedExportDialogMessage, ImageAdapterConfig, ImageFormatAdapterRegistry, PendingExport,
    SilentSaveCanvases,
};

pub const EXPORT_DIALOG_VIEW_ID: &str = "export_dialog";

pub struct ExportDialogView {
    window: window::Id,
    windows: Arc<[window::Id]>,
    adapter: Box<dyn crate::ErasedImageFormatAdapter>,
    extension: &'static str,
    path: PathBuf,
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

    fn id() -> WindowViewId {
        WindowViewId::new(EXPORT_DIALOG_VIEW_ID)
    }

    fn boot(services: &mut Services) -> (Self, Task<Self::Message>) {
        // TODO don't panic
        let pending = services.remove_service::<PendingExport>();
        let registry = services.service::<ImageFormatAdapterRegistry>();
        let extension = pending
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| registry.find_extension(extension))
            .expect("Export dialog opened for a path without a registered format");
        let config = Config::<ImageAdapterConfig>::read_or_init_or_fallback();
        let adapter = registry
            .create_with_saved_settings(extension, &config.get())
            .expect("Export dialog opened for a path without a registered format");
        let dont_ask_again = services
            .service::<SilentSaveCanvases>()
            .contains(pending.canvas_id);

        let (window, open) = iced_runtime::window::open(window::Settings {
            size: Size {
                width: 420.0,
                height: 300.0,
            },
            ..Default::default()
        });
        (
            Self {
                window,
                windows: Arc::from([window]),
                adapter,
                extension,
                path: pending.path,
                canvas_id: pending.canvas_id,
                allow_silent_export: pending.allow_silent_export,
                dont_ask_again,
            },
            open.discard(),
        )
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        services: &'a Services,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let options = self
            .adapter
            .export_dialog_view(services)
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
                column![Label::new(self.path.display().to_string()).muted(), options,].spacing(10),
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
                .export_dialog_update(message, services)
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
                futures::executor::block_on(self.adapter.export(services, canvas, &self.path))
                    .log_err();
                let silent_saves = services.service_mut::<SilentSaveCanvases>();
                if self.dont_ask_again {
                    silent_saves.insert(self.canvas_id);
                } else {
                    silent_saves.remove(self.canvas_id);
                }
                Config::<ImageAdapterConfig>::read_or_init_or_fallback()
                    .update(|config| match self.adapter.to_toml() {
                        Ok(value) => {
                            config.adapters.insert(self.extension.to_string(), value);
                        }
                        Err(error) => {
                            log::error!("Failed to serialize export settings: {error}")
                        }
                    })
                    .log_err();
                iced_runtime::window::close(self.window)
            }
            ExportDialogMessage::Cancel => iced_runtime::window::close(self.window),
        }
    }

    fn close(self, _: &mut Services) -> Task<()> {
        iced_runtime::window::close(self.window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.window)
    }
}
