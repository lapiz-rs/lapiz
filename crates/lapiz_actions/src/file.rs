use std::path::PathBuf;

use iced_runtime::Task;
use lapiz_canvas::{CCanvas, CanvasAppExt, event::CanvasCreated};
use lapiz_image::{
    CImage,
    texel::TexelType,
    tile::{GpuLayerInfo, TileStorageAppExt},
};
use lapiz_runtime::{Services, event::Event};
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
                // TODO This should change when image depth is not 8 bit
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
        services.update_canvas(&canvas_id, |canvas, services| {
            if canvas.archive.path().is_none()
                && canvas
                    .set_file_path(canvas.file_path().with_extension("lazuli"))
                    .logged_err()
                    .is_err()
            {
                return;
            }

            // TODO nonononono use async
            futures::executor::block_on(canvas.image.write_archive(&canvas.archive, services))
                .log_err()
        });
        Task::none()
    }
}
