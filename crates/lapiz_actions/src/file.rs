use std::{ffi::OsStr, path::PathBuf};

use iced_runtime::Task;
use lapiz_canvas::{CCanvas, CanvasAppExt, event::CanvasCreated};
use lapiz_config::Config;
use lapiz_image::{
    CImage,
    texel::TexelType,
    tile::{GpuLayerInfo, TileStorageAppExt},
};
use lapiz_image_adapter::{
    EXPORT_DIALOG_VIEW_ID, ImageAdapterConfig, ImageFormatAdapterRegistry, PendingExport,
    SilentSaveCanvases,
};
use lapiz_runtime::{
    Services,
    event::Event,
    windows::{OpenWindowViewCommand, WindowCommandBuffer, WindowViewId},
};
use lapiz_tools::{ToolFunctionRegistry, ToolProxies, ToolProxy};
use lapiz_undo::{UndoStack, UndoStacks};
use lapiz_utils::log_err::LogErr;
use rfd::AsyncFileDialog;

use crate::{ActionFunction, ActionId};

#[derive(Default)]
pub struct OpenFileAction;

pub enum OpenFileMessage {
    Opened(PathBuf),
    Canceled,
}

impl ActionFunction for OpenFileAction {
    type Message = OpenFileMessage;

    fn id(&self) -> ActionId {
        ActionId::new("open_file_action".into())
    }

    fn trigger(&self, _services: &mut Services) -> Task<Self::Message> {
        Task::future(async {
            let Some(file) = AsyncFileDialog::new().pick_file().await else {
                log::error!("Unable to get selected file path.");
                return OpenFileMessage::Canceled;
            };
            OpenFileMessage::Opened(file.path().to_path_buf())
        })
    }

    fn handle_message(
        &self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let OpenFileMessage::Opened(path) = message else {
            return Task::none();
        };

        let Ok((image, archive)) = CImage::from_file(&path, services).logged_err() else {
            return Task::none();
        };
        log::info!("Opened image from file {:?}.", path);

        let canvas = CCanvas::new(path, image, archive);
        let canvas_id = canvas.id();
        let tool_proxy = ToolProxy::new(services.service::<ToolFunctionRegistry>());
        services
            .service_mut::<ToolProxies>()
            .insert(*canvas_id, tool_proxy);
        let undo_stack = UndoStack::new(*canvas_id, 200);
        services
            .service_mut::<UndoStacks>()
            .insert(*canvas_id, undo_stack);

        // TODO this should not be done here
        let tiles = services.tile_storage();
        for layer in canvas.image.layer_stack().iter_layers() {
            tiles.declare_layer(
                *layer.id(),
                GpuLayerInfo {
                    // TODO
                    texel_type: TexelType::RGBA8,
                },
            );
        }
        tiles.declare_layer(
            canvas.image.selection_layer(),
            GpuLayerInfo {
                // TODO This will change when image depth is not 8 bit
                texel_type: TexelType::A8,
            },
        );

        services.add_canvas(canvas);
        CanvasCreated::broadcast(CanvasCreated { id: canvas_id });

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
        let Some(path) = services
            .canvas(&canvas_id)
            .map(|canvas| canvas.file_path().clone())
        else {
            return Task::none();
        };

        // TODO incremental saving for lazuli file. Saving should happen at every canvas command.

        start_export(services, true, path);
        Task::none()
    }
}

#[derive(Default)]
pub struct ExportFileAction;

pub enum ExportFileMessage {
    PathChosen(Option<PathBuf>),
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
        let file_name = canvas
            .file_path()
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned());

        let mut dialog = AsyncFileDialog::new();
        for format in services
            .service::<ImageFormatAdapterRegistry>()
            .iter_formats()
        {
            let mut extensions = vec![format.extension];
            extensions.extend(format.aliases);
            dialog = dialog.add_filter(&format.description, &extensions);
        }
        if let Some(file_name) = file_name {
            dialog = dialog.set_file_name(file_name);
        }

        Task::future(async move {
            ExportFileMessage::PathChosen(
                dialog
                    .save_file()
                    .await
                    .map(|file| file.path().to_path_buf()),
            )
        })
    }

    fn handle_message(
        &self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let ExportFileMessage::PathChosen(Some(path)) = message else {
            return Task::none();
        };
        start_export(services, false, path);
        Task::none()
    }
}

fn start_export(services: &mut Services, allow_silent_export: bool, path: PathBuf) {
    let Some(canvas) = services.current_canvas() else {
        return;
    };
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

    let config = Config::<ImageAdapterConfig>::read_or_init_or_fallback();
    let Some(adapter) = adapters.create_with_saved_settings(extension, &config.get()) else {
        return;
    };

    let can_silent_export = services
        .service::<SilentSaveCanvases>()
        .contains(canvas.id());
    if adapter.has_export_options() && !(allow_silent_export && can_silent_export) {
        // This is consumed by the export dialog view.
        // TODO: use more reliable param passing
        services.insert_service(PendingExport {
            path,
            allow_silent_export,
            canvas_id: canvas.id(),
        });

        services
            .service_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new(WindowViewId::new(
                EXPORT_DIALOG_VIEW_ID,
            )));
    } else {
        // TODO nonononono use async
        futures::executor::block_on(adapter.export(services, &path)).log_err();
    }
}
