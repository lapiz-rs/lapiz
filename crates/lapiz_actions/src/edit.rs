use std::{any::TypeId, sync::Arc};

use iced_core::clipboard::Content;
use iced_runtime::{Task, clipboard};
use lapiz_canvas::{CanvasAppExt, CanvasUndoStackAppExt, command::InsertLayerCommand};
use lapiz_image::{
    layer::{LayerPosition, pixel_layer::PixelLayer},
    tile::TileStorageAppExt,
};
use lapiz_runtime::Services;
use lapiz_undo::{BatchedUndoCommand, UndoStacks};
use lapiz_utils::log_err::LogErr;

use crate::{ActionFunction, ActionId};

#[derive(Default)]
pub struct UndoAction;

impl ActionFunction for UndoAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("undo_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };
        services.service_scope::<UndoStacks, _>(|stacks, services| {
            if let Some(stack) = stacks.get_mut(&*canvas_id) {
                stack.undo(services).log_err();
            }
        });
        Task::none()
    }
}

#[derive(Default)]
pub struct RedoAction;

impl ActionFunction for RedoAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("redo_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };
        services.service_scope::<UndoStacks, _>(|stacks, services| {
            if let Some(stack) = stacks.get_mut(&*canvas_id) {
                stack.redo(services).log_err();
            }
        });
        Task::none()
    }
}

#[derive(Default)]
pub struct PasteIntoNewLayerAction;

pub enum PasteMessage {
    Clipboard(Result<Arc<Content>, iced_core::clipboard::Error>),
}

impl ActionFunction for PasteIntoNewLayerAction {
    type Message = PasteMessage;

    fn id(&self) -> ActionId {
        ActionId::new("paste_into_new_layer_action".into())
    }

    fn trigger(&self, _services: &mut Services) -> Task<Self::Message> {
        clipboard::read(iced_core::clipboard::Kind::Files).map(PasteMessage::Clipboard)
    }

    fn handle_message(
        &self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
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

        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };

        let (parent, position, profile) = services
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
                PixelLayer::from_path(path, services.tile_storage(), &profile).logged_err()
            else {
                continue;
            };
            layers.push(layer);
        }
        if layers.is_empty() {
            return Task::none();
        }

        let commands = services
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

        services
            .push_undo_command(
                &canvas_id,
                BatchedUndoCommand::new("Paste Images".into(), commands),
            )
            .log_err();

        Task::none()
    }
}
