use iced_runtime::Task;
use lapiz_canvas::{CanvasAppExt, CanvasUndoStackAppExt, command::TileReplaceCommand};
use lapiz_image::tile::TileStorageAppExt;
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::Services;
use lapiz_utils::log_err::LogErr;

use crate::{ActionFunction, ActionId};

#[derive(Default)]
pub struct DeleteSelectionAction;

impl ActionFunction for DeleteSelectionAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("delete_selection_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };

        let cmd = {
            let tiles = services.tile_storage();
            let selection_layer_id = canvas.image.selection_layer();
            let selection_layer = tiles.get_layer(selection_layer_id).unwrap();

            TileReplaceCommand::new_clear(
                "Delete Selection".into(),
                canvas.id(),
                services.render_device(),
                services.render_queue(),
                selection_layer_id,
                &selection_layer,
            )
        };

        services.push_undo_command_to_current(cmd).log_err();

        Task::none()
    }
}
