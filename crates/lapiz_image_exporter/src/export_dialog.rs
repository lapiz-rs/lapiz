use std::sync::Arc;

use anyhow::Result;
use futures::executor::block_on;
use iced_core::{Element, Length, Size, Theme, window};
use iced_runtime::{
    Task,
    window::{close, open},
};
use lapiz_canvas::{CanvasAppExt as _, CanvasId};
use lapiz_config::Config;
use lapiz_file_dialog::LocalFile;
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer,
    global::Globals,
    windows::{WindowView, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    button, checkbox, column, fluent_builder::When as _, label, panel, row, space,
};

use crate::{
    ErasedExportDialogMessage, ImageExporterConfig, ImageFormatAdapterRegistry, PendingExport,
    SilentSaveCanvases,
};

pub const EXPORT_DIALOG_VIEW_ID: &str = "export_dialog";

/// Modal window view that presents format-specific export options before committing an image save.
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

/// UI messages handled by [`ExportDialogView`].
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
        globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)> {
        let pending = params.ok_or(anyhow::anyhow!("No pending export"))?;
        let registry = globals.global::<ImageFormatAdapterRegistry>();
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
        let dont_ask_again = globals
            .global::<SilentSaveCanvases>()
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
        globals: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let options = self
            .adapter
            .dialog_view(globals)
            .map(ExportDialogMessage::Adapter);
        let footer = row![]
            .when(self.allow_silent_export, |r| {
                r.push(
                    checkbox(self.dont_ask_again)
                        .label(t!("dont_ask_again"))
                        .on_toggle(ExportDialogMessage::DontAskAgainToggled),
                )
            })
            .extend([
                space().width(Length::Fill).into(),
                button(label(t!("cancel")))
                    .on_press(ExportDialogMessage::Cancel)
                    .into(),
                button(label(t!("export")))
                    .primary()
                    .on_press(ExportDialogMessage::Confirm)
                    .into(),
            ])
            .gap(10.0);

        column![
            panel(
                column![
                    label(self.local_file.path().display().to_string()).muted(),
                    options,
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
            ExportDialogMessage::Adapter(message) => self
                .adapter
                .dialog_update(message, globals)
                .map(ExportDialogMessage::Adapter),
            ExportDialogMessage::DontAskAgainToggled(checked) => {
                self.dont_ask_again = checked;
                Task::none()
            }
            ExportDialogMessage::Confirm => {
                let Some(canvas) = globals.canvas(&self.canvas_id) else {
                    return Task::done(ExportDialogMessage::Cancel);
                };

                // TODO use async
                if block_on(self.adapter.export(globals, canvas, self.local_file.path()))
                    .and_then(|_| self.local_file.commit())
                    .logged_err()
                    .is_err()
                {
                    return Task::none();
                }

                let silent_saves = globals.global_mut::<SilentSaveCanvases>();
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
