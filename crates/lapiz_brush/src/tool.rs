use std::collections::HashMap;

use iced_core::{Element, Length, Theme};
use iced_runtime::Task;
use iced_widget::column;
use lapiz_assets::asset::AssetHandle;
use lapiz_canvas::{
    CanvasAppExt, CanvasUndoStackAppExt, command::TileReplaceCommand, event::CanvasUpdated,
};
use lapiz_i18n::t;
use lapiz_image::{composite::LayerPreviewOverriders, tile::TileStorageAppExt};
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{Renderer, Services, event::Event, service::Service};
use lapiz_shader_graph::graph::{
    external::ExternalVariableId, function::ASSET_GRAPH_FUNCTION_STORAGE,
    slot::ErasedGraphLiteralUpdateMessage, texture::ASSET_GRAPH_TEXTURE_STORAGE,
};
use lapiz_tools::{ToolFunction, ToolId};
use lapiz_undo::QueuedUndoCommand;
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{icon, label::Label, panel::Panel};
use log::error;

use crate::{
    asset::BrushPreset,
    input_processing::{BasicStabilizer, InputProcessor},
    instance::BrushPresetInstance,
    render::{BrushStrokePreview, BrushStrokeResult, CanvasBrushPresetOperator},
};

pub struct CurrentBrushPreset(pub CanvasBrushPresetOperator);

impl Service for CurrentBrushPreset {}

#[derive(Clone)]
pub struct CurrentBrushPresetHandle(pub AssetHandle<BrushPreset>);

impl Service for CurrentBrushPresetHandle {}

pub trait BrushServicesExt {
    fn set_current_brush_preset(&mut self, handle: AssetHandle<BrushPreset>);
}

impl BrushServicesExt for Services {
    fn set_current_brush_preset(&mut self, handle: AssetHandle<BrushPreset>) {
        let (instance, errors) = BrushPresetInstance::from_asset(
            &handle,
            ASSET_GRAPH_TEXTURE_STORAGE.clone(),
            ASSET_GRAPH_FUNCTION_STORAGE.clone(),
        );
        for error in errors {
            error!("{error}");
        }

        let Some(instance) = instance else {
            return;
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
        self.insert_service(CurrentBrushPreset(operator));
        self.insert_service(CurrentBrushPresetHandle(handle));
    }
}

#[derive(Default)]
pub struct BrushTool {
    next_stroke_id: u64,
    active_stroke_id: Option<u64>,
    queued_commands: HashMap<u64, QueuedUndoCommand>,
    request_preview_when_preview_ongoing: bool,
    preview_ongoing: bool,
}

// TODO
#[allow(clippy::large_enum_variant)]
pub enum BrushToolMessage {
    StrokePreview(Option<BrushStrokePreview>),
    StrokeResult(BrushStrokeResult),
    UpdateExternalVariable {
        id: ExternalVariableId,
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
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };
        if services.get_service::<CurrentBrushPreset>().is_none() {
            return Task::none();
        }

        self.next_stroke_id += 1;
        let stroke_id = self.next_stroke_id;
        self.active_stroke_id = Some(stroke_id);
        self.queued_commands
            .insert(stroke_id, services.queue_undo_command(&canvas_id).unwrap());

        services
            .try_service_scope::<CurrentBrushPreset, _>(|brush, services| {
                brush
                    .0
                    .begin_stroke(mouse, stroke_id, canvas_id, services)
                    .discard()
            })
            .unwrap_or_else(Task::none)
    }

    #[tracing::instrument(skip_all, name = "brush_tool_update")]
    fn update(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        services
            .try_service_scope::<CurrentBrushPreset, _>(|brush, services| {
                let render = brush.0.update_stroke(mouse, services).discard();
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
        services: &mut Services,
    ) -> Task<Self::Message> {
        services
            .try_service_scope::<CurrentBrushPreset, _>(|brush, services| {
                brush
                    .0
                    .end_stroke(mouse, services)
                    .map(BrushToolMessage::StrokeResult)
            })
            .unwrap_or_else(Task::none)
    }

    fn handle_message(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        match message {
            BrushToolMessage::StrokePreview(maybe_preview) => {
                self.preview_ongoing = false;
                let Some(BrushStrokePreview {
                    stroke_id,
                    canvas_id,
                    target_layer_id,
                    overrider,
                    dirty_tiles,
                }) = maybe_preview
                else {
                    return Task::none();
                };

                if self.active_stroke_id != Some(stroke_id) {
                    return Task::none();
                }

                services
                    .service_mut::<LayerPreviewOverriders>()
                    .insert_overrider(target_layer_id, overrider);
                CanvasUpdated::broadcast(CanvasUpdated {
                    id: canvas_id,
                    dirty_tiles,
                });

                if !self.request_preview_when_preview_ongoing {
                    return Task::none();
                }

                self.request_preview_when_preview_ongoing = false;
                self.preview_ongoing = true;

                services
                    .try_service_scope::<CurrentBrushPreset, _>(|brush, _services| {
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
                    services
                        .service_mut::<LayerPreviewOverriders>()
                        .remove_overrider(&target_layer_id);
                }

                let Some(result_texture) = result.texture().cloned() else {
                    return Task::none();
                };
                let cmd = {
                    let layer_storage = services
                        .tile_storage()
                        .get_layer(target_layer_id)
                        .expect("Brush target layer should exist");
                    TileReplaceCommand::new(
                        "Brush stroke".into(),
                        canvas_id,
                        services.render_device(),
                        services.render_queue(),
                        target_layer_id,
                        &layer_storage,
                        result.iter_tile_indices().collect(),
                        result_texture,
                    )
                };
                command.send(Box::new(cmd), services).log_err();

                Task::none()
            }
            BrushToolMessage::UpdateExternalVariable { id, message } => {
                services.service_scope::<CurrentBrushPreset, _>(|brush, _| {
                    brush.0.instance_mut().update_external_var(&id, message);
                });

                Task::none()
            }
        }
    }

    fn tool_option_widget<'a>(
        &'a self,
        services: &'a Services,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let brush = services.get_service::<CurrentBrushPreset>()?;

        let variables = brush
            .0
            .instance()
            .iter_external_vars()
            .map(|(id, variable)| {
                column![
                    Label::new(variable.name),
                    variable
                        .value
                        .ty()
                        .view_literal((*id).into(), variable.value.value())
                        .map(move |message| BrushToolMessage::UpdateExternalVariable {
                            id,
                            message
                        }),
                ]
                .spacing(4)
                .into()
            })
            .collect::<Vec<_>>();

        Some(
            Panel::new(
                column(variables)
                    .spacing(8)
                    .push(Label::new(t!("variables")).strong()),
            )
            .padding(8)
            .width(Length::Fill)
            .into(),
        )
    }
}
