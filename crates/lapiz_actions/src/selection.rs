use iced_runtime::Task;
use lapiz_canvas::{CanvasAppExt as _, CanvasUndoStackAppExt as _, command::TileReplaceCommand};
use lapiz_image::tile::TileStorageAppExt as _;
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::global::Globals;
use lapiz_utils::log_err::LogErr as _;

use crate::{ActionFunction, ActionId};

/// Action that clears the current selection mask on the active canvas.
#[derive(Default)]
pub struct DeleteSelectionAction;

impl ActionFunction for DeleteSelectionAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("delete_selection_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        let Some(canvas) = globals.current_canvas() else {
            return Task::none();
        };

        let cmd = {
            let tiles = globals.tile_storage();
            let selection_layer_id = canvas.image.selection_layer();
            let selection_layer = tiles.get_layer(selection_layer_id).unwrap();

            TileReplaceCommand::new_clear(
                "Delete Selection".into(),
                canvas.id(),
                globals.render_device(),
                globals.render_queue(),
                selection_layer_id,
                &selection_layer,
            )
        };

        globals.push_undo_command_to_current(cmd).log_err();

        Task::none()
    }
}
