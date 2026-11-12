use std::collections::HashMap;

use iced_core::{Element, Theme};
use iced_runtime::Task;
use lapiz_assets::{AssetAppExt as _, asset::AssetHandle};
use lapiz_canvas::{
    CanvasAppExt as _, CanvasUndoStackAppExt as _, command::TileReplaceCommand,
    event::CanvasUpdated,
};
use lapiz_effect::asset::EffectInputSlotId;
use lapiz_i18n::t;
use lapiz_image::{composite::LayerPreviewOverriders, tile::TileStorageAppExt as _};
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{
    Renderer,
    event::Event as _,
    global::{Global, Globals},
};
use lapiz_shader_graph::graph::slot::{ErasedGraphLiteralUpdateMessage, GraphInputSlotId};
use lapiz_tools::{ToolFunction, ToolId};
use lapiz_undo::QueuedUndoCommand;
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{column, flex::Flex, icon, label, panel};
use log::error;

use crate::{
    asset::BrushPreset,
    input_processing::{BasicStabilizer, InputProcessor},
    instance::BrushPresetInstance,
    render::{BrushStrokePreview, BrushStrokeResult, CanvasBrushPresetOperator},
};

/// Global resource holding the active [`CanvasBrushPresetOperator`] used by [`BrushTool`].
pub struct CurrentBrushPreset(pub CanvasBrushPresetOperator);

impl Global for CurrentBrushPreset {}

/// Global resource holding the [`AssetHandle`] of the currently selected [`BrushPreset`].
#[derive(Clone)]
pub struct CurrentBrushPresetHandle(pub AssetHandle<BrushPreset>);

impl Global for CurrentBrushPresetHandle {}

/// Extension trait on [`Globals`] for switching the active [`BrushPreset`].
pub trait BrushglobalsExt {
    fn set_current_brush_preset(&mut self, handle: AssetHandle<BrushPreset>);
}

impl BrushglobalsExt for Globals {
    fn set_current_brush_preset(&mut self, handle: AssetHandle<BrushPreset>) {
        let instance = match BrushPresetInstance::from_asset(&handle, self.assets().clone()) {
            Ok(instance) => instance,
            Err(error) => {
                error!("{error:#}");
                return;
            }
        };
        let operator = CanvasBrushPresetOperator::new(
            instance,
            self.render_device().clone(),
            self.render_queue().clone(),
            InputProcessor::new(256, Box::new(BasicStabilizer)),
        );
        log::info!(
            "Loaded brush preset {} {:?}",
            operator.instance().metadata().name,
            operator.instance().asset_id()
        );
        self.insert_global(CurrentBrushPreset(operator));
        self.insert_global(CurrentBrushPresetHandle(handle));
    }
}

/// Interactive painting [`ToolFunction`] that drives brush strokes on the active canvas layer.
///
/// `BrushTool` coordinates stroke lifecycle events (`begin`, `update`, `end`) with the global
/// [`CurrentBrushPreset`] operator, reserves [`QueuedUndoCommand`] slots in the canvas undo stack
/// so asynchronous GPU strokes commit in order, updates live [`LayerPreviewOverriders`] during
/// painting, and commits completed strokes via [`TileReplaceCommand`].
#[derive(Default)]
pub struct BrushTool {
    next_stroke_id: u64,
    active_stroke_id: Option<u64>,
    queued_commands: HashMap<u64, QueuedUndoCommand>,
    request_preview_when_preview_ongoing: bool,
    preview_ongoing: bool,
}

// TODO
/// Asynchronous stroke results and parameter update messages handled by [`BrushTool`].
#[allow(
    clippy::large_enum_variant,
    reason = "stroke results own a DynamicLayerStorage and are matched by value"
)]
pub enum BrushToolMessage {
    StrokePreview(Option<BrushStrokePreview>),
    StrokeResult(BrushStrokeResult),
    UpdateParameter {
        id: EffectInputSlotId,
        message: ErasedGraphLiteralUpdateMessage,
    },
}

impl ToolFunction for BrushTool {
    type Message = BrushToolMessage;

    fn id() -> ToolId {
        ToolId::new("brush_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::brush()
    }

    #[tracing::instrument(skip_all, name = "brush_tool_begin")]
    fn begin(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        let Some(canvas_id) = globals.current_canvas_id() else {
            return Task::none();
        };
        if globals.get_global::<CurrentBrushPreset>().is_none() {
            return Task::none();
        }

        self.next_stroke_id += 1;
        let stroke_id = self.next_stroke_id;
        self.active_stroke_id = Some(stroke_id);
        self.queued_commands
            .insert(stroke_id, globals.queue_undo_command(&canvas_id).unwrap());

        globals
            .try_update_scope::<CurrentBrushPreset, _>(|brush, globals| {
                brush
                    .0
                    .begin_stroke(mouse, stroke_id, canvas_id, globals)
                    .discard()
            })
            .unwrap_or_else(Task::none)
    }

    #[tracing::instrument(skip_all, name = "brush_tool_update")]
    fn update(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        globals
            .try_update_scope::<CurrentBrushPreset, _>(|brush, globals| {
                let render = brush.0.update_stroke(mouse, globals).discard();
                if self.preview_ongoing {
                    self.request_preview_when_preview_ongoing = true;
                    return render;
                }

                let preview = brush.0.preview().map(BrushToolMessage::StrokePreview);
                self.preview_ongoing = true;

                render.chain(preview)
            })
            .unwrap_or_else(Task::none)
    }

    #[tracing::instrument(skip_all, name = "brush_tool_end")]
    fn end(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        globals
            .try_update_scope::<CurrentBrushPreset, _>(|brush, globals| {
                brush
                    .0
                    .end_stroke(mouse, globals)
                    .map(BrushToolMessage::StrokeResult)
            })
            .unwrap_or_else(Task::none)
    }

    fn handle_message(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        match message {
            BrushToolMessage::StrokePreview(maybe_preview) => {
                self.preview_ongoing = false;
                if let Some(BrushStrokePreview {
                    stroke_id,
                    canvas_id,
                    target_layer_id,
                    overrider,
                    dirty_tiles,
                }) = maybe_preview
                    && self.active_stroke_id == Some(stroke_id)
                {
                    globals
                        .global_mut::<LayerPreviewOverriders>()
                        .insert_overrider(target_layer_id, overrider);
                    CanvasUpdated::broadcast(CanvasUpdated {
                        id: canvas_id,
                        dirty_tiles,
                    });
                }

                if !self.request_preview_when_preview_ongoing || self.active_stroke_id.is_none() {
                    return Task::none();
                }

                self.request_preview_when_preview_ongoing = false;
                self.preview_ongoing = true;

                globals
                    .try_update_scope::<CurrentBrushPreset, _>(|brush, _globals| {
                        brush.0.preview().map(BrushToolMessage::StrokePreview)
                    })
                    .unwrap_or_else(Task::none)
            }
            BrushToolMessage::StrokeResult(BrushStrokeResult {
                stroke_id,
                canvas_id,
                target_layer_id,
                result,
            }) => {
                let command = self.queued_commands.remove(&stroke_id).unwrap();
                if self
                    .active_stroke_id
                    .take_if(|active_id| *active_id == stroke_id)
                    .is_some()
                {
                    globals
                        .global_mut::<LayerPreviewOverriders>()
                        .remove_overrider(&target_layer_id);
                }

                let Some(result_texture) = result.texture().cloned() else {
                    return Task::none();
                };
                let cmd = {
                    let layer_storage = globals
                        .tile_storage()
                        .get_layer(target_layer_id)
                        .expect("Brush target layer should exist");
                    TileReplaceCommand::new(
                        "Brush stroke".into(),
                        canvas_id,
                        globals.render_device(),
                        globals.render_queue(),
                        target_layer_id,
                        &layer_storage,
                        result.iter_tile_indices().collect(),
                        result_texture,
                    )
                };
                command.send(Box::new(cmd), globals).log_err();

                Task::none()
            }
            BrushToolMessage::UpdateParameter { id, message } => {
                globals.update_global::<CurrentBrushPreset, _>(|brush, _| {
                    brush.0.instance_mut().update_parameter(&id, message);
                });

                Task::none()
            }
        }
    }

    fn tool_option_widget<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let brush = globals.get_global::<CurrentBrushPreset>()?;

        let parameters = brush
            .0
            .instance()
            .parameters()
            .iter()
            .map(|(id, parameter)| {
                let slot_id = GraphInputSlotId::new(id.into_inner());
                column![
                    label(parameter.name.clone()),
                    parameter
                        .value
                        .ty()
                        .view_literal(slot_id, parameter.value.value(), globals.assets())
                        .map(move |message| BrushToolMessage::UpdateParameter { id: *id, message }),
                ]
                .gap(4.0)
                .into()
            })
            .collect::<Vec<_>>();

        Some(
            panel(
                Flex::column(parameters)
                    .gap(8.0)
                    .push(label(t!("variables")).strong()),
            )
            .padding(8)
            .width(iced_core::Length::Fill)
            .into(),
        )
    }
}
