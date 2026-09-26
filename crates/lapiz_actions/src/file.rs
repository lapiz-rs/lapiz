use std::{ffi::OsStr, iter, sync::Arc};

use futures::executor::block_on;
use iced_runtime::Task;
use lapiz_canvas::CanvasAppExt as _;
use lapiz_config::Config;
use lapiz_file_dialog::{FileDialog, LocalFile};
use lapiz_i18n::t;
use lapiz_image_exporter::{
    ImageFormatAdapterRegistry, PendingExport, SilentSaveCanvases, config::ImageExporterConfig,
    export_dialog::EXPORT_DIALOG_VIEW_ID,
};
use lapiz_image_importer::{ImageImporterRegistry, start_import};
use lapiz_runtime::{
    Services,
    windows::{OpenWindowViewCommand, WindowCommandBuffer, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;

use crate::{ActionFunction, ActionId};

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

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let mut dialog = FileDialog::new_maybe_from_service(services);
        let formats = services
            .service::<ImageImporterRegistry>()
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

    fn handle_message(
        &self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let OpenFileMessage::Opened(local_file) = message else {
            return Task::none();
        };

        start_import(services, local_file);

        Task::none()
    }
}

#[derive(Default)]
pub struct SaveFileAction;

impl ActionFunction for SaveFileAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("save_file_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };
        let Some(canvas) = services.canvas(&canvas_id) else {
            return Task::none();
        };

        // TODO incremental saving for lazuli file. Saving should happen at every canvas command.
        start_export(services, true, canvas.local_file().clone());

        Task::none()
    }
}

#[derive(Default)]
pub struct ExportFileAction;

pub enum ExportFileMessage {
    PathChosen(Option<LocalFile>),
}

impl ActionFunction for ExportFileAction {
    type Message = ExportFileMessage;

    fn id(&self) -> ActionId {
        ActionId::new("export_file_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };
        let mut dialog = FileDialog::new_maybe_from_service(services);
        let name = canvas.local_file().name();
        if !name.is_empty() {
            dialog = dialog.set_file_name(name);
        } else {
            #[cfg(target_os = "android")]
            {
                dialog = dialog.set_file_name("Untitled.png");
            }
        }
        for format in services
            .service::<ImageFormatAdapterRegistry>()
            .iter_formats()
        {
            let mut extensions = vec![format.extension];
            extensions.extend(format.aliases);
            dialog = dialog.add_filter(format.description, &extensions);
        }

        Task::future(async move {
            match dialog.save_file().await {
                Ok(file) => ExportFileMessage::PathChosen(file),
                Err(error) => {
                    log::error!("Unable to create document: {error}");
                    ExportFileMessage::PathChosen(None)
                }
            }
        })
    }

    fn handle_message(
        &self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let ExportFileMessage::PathChosen(Some(local_file)) = message else {
            return Task::none();
        };
        start_export(services, false, local_file.into());
        Task::none()
    }
}

fn start_export(services: &mut Services, allow_silent_export: bool, local_file: Arc<LocalFile>) {
    let Some(canvas) = services.current_canvas() else {
        return;
    };
    let path = local_file.path();
    let Some(path_extension) = path.extension().and_then(OsStr::to_str) else {
        return;
    };

    let adapters = services.service::<ImageFormatAdapterRegistry>();
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

    let can_silent_export = services
        .service::<SilentSaveCanvases>()
        .contains(canvas.id());
    if adapter.has_options() && !(allow_silent_export && can_silent_export) {
        let params = PendingExport {
            local_file,
            allow_silent_export,
            canvas_id: canvas.id(),
        };
        services
            .service_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new_with_params(
                WindowViewId::new(EXPORT_DIALOG_VIEW_ID),
                params,
            ));
    } else {
        // TODO nonononono use async
        block_on(adapter.export(services, canvas, local_file.path()))
            .and_then(|_| local_file.commit())
            .log_err();
    }
}
