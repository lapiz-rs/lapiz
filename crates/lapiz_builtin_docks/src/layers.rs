use std::{mem, sync::LazyLock};

use iced_core::{
    Element, Theme,
    keyboard::{self, Modifiers},
    widget::Void,
    window,
};
use iced_futures::{Subscription, event::listen_with};
use iced_runtime::Task;
use lapiz_canvas::{
    CanvasAppExt as _, CanvasUndoStackAppExt as _,
    command::{LayerPropertyChangeCommand, MoveLayersCommand},
    widget::layer_stack::{DropInfo, LayerStackMessage, LayerStackView},
};
use lapiz_dock::dock::{Dock, DockId};
use lapiz_image::{
    composite::BlendFunctionRegistry,
    layer::{
        LayerId,
        properties::{LayerProperties, builtin::NamePropertyExt as _},
    },
    tile::TileStorageAppExt as _,
};
use lapiz_input::key::KeyboardState;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_utils::log_err::LogErr as _;

pub static LAYER_DOCK_ID: LazyLock<DockId> = LazyLock::new(|| DockId::new("layer_dock".into()));

/// Workspace [`Dock`] that renders the active canvas's layer hierarchy via [`LayerStackView`] and dispatches undoable layer property, selection, and reordering commands.
#[derive(Default)]
pub struct LayersDock {
    renaming_layer: Option<LayerId>,
    rename_value: String,
    drop_preview: Option<DropInfo>,
}

impl LayersDock {
    fn push_property_change(
        globals: &mut Globals,
        layer_id: LayerId,
        apply: impl FnOnce(&mut LayerProperties),
    ) {
        let Some(canvas_id) = globals.current_canvas_id() else {
            return;
        };
        let cmd = globals.update_canvas(&canvas_id, |canvas, _globals| {
            let layer = canvas.image.layer_stack().get_layer(&layer_id)?;
            let old = layer.properties().clone();
            let new = {
                let mut props = old.clone();
                apply(&mut props);
                props
            };
            Some(LayerPropertyChangeCommand {
                canvas: canvas_id,
                layer_id,
                old,
                new,
            })
        });
        if let Some(cmd) = cmd.flatten() {
            globals.push_undo_command(&canvas_id, cmd).log_err();
        }
    }
}

/// Messages handled by [`LayersDock`].
#[derive(Debug, Clone)]
pub enum LayersDockMessage {
    Layer(LayerStackMessage),
    EscapePressed,
}

impl Dock for LayersDock {
    type Message = LayersDockMessage;

    fn id(&self) -> DockId {
        LAYER_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let Some(canvas) = globals.current_canvas() else {
            return Void.into();
        };
        let blend_functions = globals.global::<BlendFunctionRegistry>();
        let tile_storage = globals.tile_storage();
        LayerStackView::new(
            canvas,
            blend_functions,
            tile_storage,
            self.renaming_layer,
            &self.rename_value,
            self.drop_preview.clone(),
            &|m| LayersDockMessage::Layer(m),
        )
        .into()
    }

    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        match message {
            LayersDockMessage::EscapePressed => {
                if self.renaming_layer.is_some() {
                    self.renaming_layer = None;
                    self.rename_value.clear();
                }
            }
            LayersDockMessage::Layer(LayerStackMessage::LayerPropertyChanged(command)) => {
                let canvas_id = command.canvas;
                globals.push_undo_command(&canvas_id, command).log_err();
            }
            LayersDockMessage::Layer(LayerStackMessage::DropPreview(drop_preview)) => {
                self.drop_preview = drop_preview;
            }
            LayersDockMessage::Layer(LayerStackMessage::SelectLayer(layer_id)) => {
                let Some(canvas_id) = globals.current_canvas_id() else {
                    return Task::none();
                };
                let modifiers = globals.global::<KeyboardState>().modifiers();
                globals.update_canvas(&canvas_id, |canvas, _| {
                    if modifiers.contains(Modifiers::CTRL) {
                        canvas.toggle_layer_selection_and_active(layer_id);
                    } else if modifiers.contains(Modifiers::SHIFT) {
                        let active_layer = canvas.active_layer_id();
                        if layer_id == active_layer {
                            return;
                        }
                        let tree = canvas
                            .image
                            .layer_stack()
                            .iter_layers_dfs_display_order_without_root()
                            .map(|(n, _)| *n.id())
                            .collect::<Vec<_>>();
                        let mut on_select = false;
                        for layer in tree {
                            if on_select {
                                canvas.select_layer(layer);
                            }
                            if layer == layer_id || layer == active_layer {
                                on_select = !on_select;
                            }
                        }
                        canvas.set_active_layer(layer_id);
                    } else if !canvas.selected_layer_ids().contains(&layer_id) {
                        canvas.set_active_layer_and_clear_select(layer_id);
                    } else {
                        canvas.set_active_layer(layer_id);
                    }
                });
            }
            LayersDockMessage::Layer(LayerStackMessage::MoveLayers {
                layer_ids,
                new_parent,
                new_position,
            }) => {
                self.drop_preview = None;
                let Some(canvas_id) = globals.current_canvas_id() else {
                    return Task::none();
                };
                let cmd = globals.update_canvas(&canvas_id, |canvas, _globals| {
                    let dragged = layer_ids.first()?;
                    let original_parent = canvas
                        .image
                        .layer_stack()
                        .get_layer(dragged)
                        .and_then(|n| n.parent().copied())?;
                    let original_index = canvas
                        .image
                        .layer_stack()
                        .get_layer(&original_parent)
                        .and_then(|p| p.child_index(dragged))
                        .unwrap_or(0);
                    let resolved_index = canvas
                        .image
                        .layer_stack()
                        .get_layer(&new_parent)
                        .and_then(|p| p.resolve_index(new_position));
                    if let Some(resolved_index) = resolved_index
                        && original_parent == new_parent
                        && original_index == resolved_index
                    {
                        return None;
                    }
                    Some(MoveLayersCommand::new(
                        canvas,
                        layer_ids.iter().copied(),
                        new_parent,
                        new_position,
                    ))
                });
                if let Some(cmd) = cmd.flatten() {
                    globals.push_undo_command(&canvas_id, cmd).log_err();
                }
            }
            LayersDockMessage::Layer(LayerStackMessage::RenameLayer(layer_id)) => {
                let name = globals.current_canvas().and_then(|canvas| {
                    canvas
                        .image
                        .layer_stack()
                        .get_layer(&layer_id)
                        .and_then(|layer| layer.properties().get_name())
                        .map(ToOwned::to_owned)
                });
                if let Some(name) = name {
                    self.renaming_layer = Some(layer_id);
                    self.rename_value = name;
                }
            }
            LayersDockMessage::Layer(LayerStackMessage::RenameChanged(value)) => {
                if self.renaming_layer.is_some() {
                    self.rename_value = value;
                }
            }
            LayersDockMessage::Layer(LayerStackMessage::RenameCommit(layer_id)) => {
                if self.renaming_layer != Some(layer_id) {
                    return Task::none();
                }
                let name = mem::take(&mut self.rename_value);
                self.renaming_layer = None;
                Self::push_property_change(globals, layer_id, move |props| {
                    props.set_name(name);
                });
            }
        }
        Task::none()
    }

    fn subscription(&self, _globals: &Globals) -> Subscription<Self::Message> {
        listen_with(|event, _status, _window| match event {
            iced_core::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => Some(LayersDockMessage::EscapePressed),
            _ => None,
        })
    }
}
