use std::{any::TypeId, sync::Arc};

use iced_core::clipboard::{Content, Error, Kind};
use iced_runtime::{Task, clipboard};
use lapiz_canvas::{
    CanvasAppExt as _, CanvasToolProxyAppExt as _, CanvasUndoStackAppExt as _,
    command::InsertLayerCommand,
};
use lapiz_image::{
    layer::{LayerPosition, pixel_layer::PixelLayer},
    tile::TileStorageAppExt as _,
};
use lapiz_runtime::global::Globals;
use lapiz_undo::{BatchedUndoCommand, UndoStacks};
use lapiz_utils::log_err::LogErr as _;

use crate::{ActionFunction, ActionId};

/// Action that undoes the most recent operation on the active tool or canvas undo stack.
#[derive(Default)]
pub struct UndoAction;

impl ActionFunction for UndoAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("undo_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        if globals.update_current_tool_proxy(|proxy, globals| proxy.undo(globals)) == Some(true) {
            return Task::none();
        }

        let Some(canvas_id) = globals.current_canvas_id() else {
            return Task::none();
        };
        globals.update_global::<UndoStacks, _>(|stacks, globals| {
            if let Some(stack) = stacks.get_mut(&*canvas_id) {
                stack.undo(globals).log_err();
            }
        });
        Task::none()
    }
}

/// Action that redoes the most recently undone operation on the active tool or canvas undo stack.
#[derive(Default)]
pub struct RedoAction;

impl ActionFunction for RedoAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("redo_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        if globals.update_current_tool_proxy(|proxy, globals| proxy.redo(globals)) == Some(true) {
            return Task::none();
        }

        let Some(canvas_id) = globals.current_canvas_id() else {
            return Task::none();
        };
        globals.update_global::<UndoStacks, _>(|stacks, globals| {
            if let Some(stack) = stacks.get_mut(&*canvas_id) {
                stack.redo(globals).log_err();
            }
        });
        Task::none()
    }
}

/// Action that reads image files from the system clipboard and inserts them as new pixel layers.
#[derive(Default)]
pub struct PasteIntoNewLayerAction;

/// Message returned after reading clipboard contents for [`PasteIntoNewLayerAction`].
pub enum PasteMessage {
    Clipboard(Result<Arc<Content>, Error>),
}

impl ActionFunction for PasteIntoNewLayerAction {
    type Message = PasteMessage;

    fn id(&self) -> ActionId {
        ActionId::new("paste_into_new_layer_action".into())
    }

    fn trigger(&self, _globals: &mut Globals) -> Task<Self::Message> {
        clipboard::read(Kind::Files).map(PasteMessage::Clipboard)
    }

    fn handle_message(&self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        let PasteMessage::Clipboard(Ok(content)) = message else {
            return Task::none();
        };

        let paths = match content.as_ref() {
            Content::Files(path_bufs) => path_bufs
                .iter()
                .filter(|path| path.exists())
                .cloned()
                .collect::<Vec<_>>(),
            _ => return Task::none(),
        };

        if paths.is_empty() {
            return Task::none();
        }

        let Some(canvas_id) = globals.current_canvas_id() else {
            return Task::none();
        };

        let (parent, position, profile) = globals
            .update_canvas(&canvas_id, |canvas, _| {
                let (parent, position) = {
                    let mut cur_parent = canvas.active_layer_node();
                    let mut cur_position = LayerPosition::foreground();
                    while !cur_parent
                        .instance()
                        .can_have_children_of(TypeId::of::<PixelLayer>())
                    {
                        let cur_parent_id = cur_parent.parent()?;
                        let parent_id =
                            canvas.image.layer_stack().get_layer(cur_parent_id).unwrap();
                        cur_position = LayerPosition::above(*cur_parent.id());
                        cur_parent = canvas
                            .image
                            .layer_stack()
                            .get_layer(parent_id.id())
                            .unwrap();
                    }
                    (*cur_parent.id(), cur_position)
                };
                Some((parent, position, canvas.image.profile().clone()))
            })
            .flatten()
            .unwrap();

        let mut layers = Vec::new();
        for path in &paths {
            let Ok(layer) =
                PixelLayer::from_path(path, globals.tile_storage(), &profile).logged_err()
            else {
                continue;
            };
            layers.push(layer);
        }
        if layers.is_empty() {
            return Task::none();
        }

        let commands = globals
            .update_canvas(&canvas_id, |canvas, _| {
                let mut commands = Vec::new();
                let mut cur_position = position;
                for layer in layers {
                    let layer_id = *layer.id();
                    commands.push(InsertLayerCommand::new(canvas, layer, parent, cur_position));
                    cur_position = LayerPosition::above(layer_id);
                }
                commands
            })
            .unwrap();

        globals
            .push_undo_command(
                &canvas_id,
                BatchedUndoCommand::new("Paste Images".into(), commands),
            )
            .log_err();

        Task::none()
    }
}
