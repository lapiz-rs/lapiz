use std::{ffi::OsStr, iter, sync::Arc};

use futures::executor::block_on;
use iced_runtime::Task;
use lapiz_canvas::{CanvasAppExt as _, CanvasId, recent::RecentFiles};
use lapiz_config::Config;
use lapiz_file_dialog::{FileDialog, LocalFile};
use lapiz_i18n::t;
use lapiz_image_exporter::{
    ImageFormatAdapterRegistry, PendingExport, SilentSaveCanvases, config::ImageExporterConfig,
    export_dialog::EXPORT_DIALOG_VIEW_ID,
};
use lapiz_image_importer::{ImageImporterRegistry, start_import};
use lapiz_new_canvas_dialog::NEW_CANVAS_DIALOG_VIEW_ID;
use lapiz_runtime::{
    global::Globals,
    windows::{OpenWindowViewCommand, WindowCommandBuffer, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;

use crate::{ActionFunction, ActionId};

/// Action that opens the dialog for creating a new canvas.
#[derive(Default)]
pub struct NewFileAction;

impl ActionFunction for NewFileAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("new_file_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        globals
            .global_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new(WindowViewId::new(
                NEW_CANVAS_DIALOG_VIEW_ID,
            )));
        Task::none()
    }
}

/// Action that prompts the user to pick an image file and imports it as a canvas.
#[derive(Default)]
pub struct OpenFileAction;

pub enum OpenFileMessage {
    Opened(LocalFile),
    Canceled,
}

impl ActionFunction for OpenFileAction {
    type Message = OpenFileMessage;

    fn id(&self) -> ActionId {
        ActionId::new("open_file_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        let mut dialog = FileDialog::new_maybe_from_global(globals);
        let formats = globals
            .global::<ImageImporterRegistry>()
            .iter_formats()
            .collect::<Vec<_>>();
        let all_extensions = formats
            .iter()
            .flat_map(|format| iter::once(format.extension).chain(format.aliases.iter().copied()))
            .collect::<Vec<_>>();
        dialog = dialog.add_filter(t!("all_formats"), &all_extensions);
        for format in formats {
            let mut extensions = vec![format.extension];
            extensions.extend(format.aliases);
            dialog = dialog.add_filter(format.description, &extensions);
        }
        Task::future(async move {
            match dialog.pick_file().await {
                Ok(Some(file)) => OpenFileMessage::Opened(file),
                Ok(None) => OpenFileMessage::Canceled,
                Err(error) => {
                    log::error!("Unable to open document: {error}");
                    OpenFileMessage::Canceled
                }
            }
        })
    }

    fn handle_message(&self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        let OpenFileMessage::Opened(local_file) = message else {
            return Task::none();
        };

        start_import(globals, local_file);

        Task::none()
    }
}

/// Action that saves the active canvas to its associated file or prompts for a destination path.
#[derive(Default)]
pub struct SaveFileAction;

impl ActionFunction for SaveFileAction {
    type Message = ExportFileMessage;

    fn id(&self) -> ActionId {
        ActionId::new("save_file_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        let Some(canvas_id) = globals.current_canvas_id() else {
            return Task::none();
        };
        let Some(canvas) = globals.canvas(&canvas_id) else {
            return Task::none();
        };

        // TODO incremental saving for lazuli file. Saving should happen at every canvas command.
        if let Some(file) = canvas.local_file().cloned() {
            start_export(globals, canvas_id, true, file);
            Task::none()
        } else {
            // The canvas has not saved yet and only exists in memory.
            ExportFileAction.trigger(globals)
        }
    }

    fn handle_message(&self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        let ExportFileMessage::PathChosen(canvas_id, Some(local_file)) = message else {
            return Task::none();
        };
        let Some(canvas) = globals.canvas_mut(&canvas_id) else {
            return Task::none();
        };

        canvas.set_local_file(local_file.into());
        let local_file = canvas.local_file().unwrap().clone();
        Config::<RecentFiles>::read_or_init_or_fallback()
            .update(|c| c.update(local_file.source(), local_file.name().to_owned()))
            .log_err();
        start_export(globals, canvas_id, false, local_file);
        Task::none()
    }
}

/// Action that prompts for a destination file path and exports the active canvas.
#[derive(Default)]
pub struct ExportFileAction;

pub enum ExportFileMessage {
    PathChosen(CanvasId, Option<LocalFile>),
}

impl ActionFunction for ExportFileAction {
    type Message = ExportFileMessage;

    fn id(&self) -> ActionId {
        ActionId::new("export_file_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        let Some(canvas) = globals.current_canvas() else {
            return Task::none();
        };
        let mut dialog = FileDialog::new_maybe_from_global(globals);
        let canvas_id = canvas.id();
        let name = canvas
            .local_file()
            .map(|file| file.name())
            .filter(|name| !name.is_empty());
        dialog = dialog.set_file_name(name.unwrap_or("Untitled.png"));
        for format in globals
            .global::<ImageFormatAdapterRegistry>()
            .iter_formats()
        {
            let mut extensions = vec![format.extension];
            extensions.extend(format.aliases);
            dialog = dialog.add_filter(format.description, &extensions);
        }

        Task::future(async move {
            match dialog.save_file().await {
                Ok(file) => ExportFileMessage::PathChosen(canvas_id, file),
                Err(error) => {
                    log::error!("Unable to create document: {error}");
                    ExportFileMessage::PathChosen(canvas_id, None)
                }
            }
        })
    }

    fn handle_message(&self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        let ExportFileMessage::PathChosen(canvas_id, Some(local_file)) = message else {
            return Task::none();
        };
        start_export(globals, canvas_id, false, local_file.into());
        Task::none()
    }
}

fn start_export(
    globals: &mut Globals,
    canvas_id: CanvasId,
    allow_silent_export: bool,
    local_file: Arc<LocalFile>,
) {
    let Some(canvas) = globals.canvas(&canvas_id) else {
        return;
    };
    let path = local_file.path();
    let Some(path_extension) = path.extension().and_then(OsStr::to_str) else {
        return;
    };

    let adapters = globals.global::<ImageFormatAdapterRegistry>();
    let Some(extension) = adapters.find_extension(path_extension) else {
        log::warn!(
            "No image format adapter matches {}, cannot export",
            path.display()
        );
        return;
    };

    let config = Config::<ImageExporterConfig>::read_or_init_or_fallback();
    let Some(adapter) = adapters.create_with_saved_settings(extension, &config.get()) else {
        return;
    };

    let can_silent_export = globals.global::<SilentSaveCanvases>().contains(canvas.id());
    if adapter.has_options() && !(allow_silent_export && can_silent_export) {
        let params = PendingExport {
            local_file,
            allow_silent_export,
            canvas_id: canvas.id(),
        };
        globals
            .global_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new_with_params(
                WindowViewId::new(EXPORT_DIALOG_VIEW_ID),
                params,
            ));
    } else {
        // TODO nonononono use async
        block_on(adapter.export(globals, canvas, local_file.path()))
            .and_then(|_| local_file.commit())
            .log_err();
    }
}
