use glam::{Vec2, Vec4};
use iced_core::{Element, Length, Theme};
use iced_runtime::Task;
use iced_wgpu::Renderer;
use iced_widget::row;
use lapiz_bucket_tool::{
    BucketTool,
    bucket::{Bucket, BucketAntialiasApproach, BucketParams},
};
use lapiz_canvas::{CanvasAppExt, CanvasUndoStackAppExt, command::TileReplaceCommand};
use lapiz_image::tile::TileStorageAppExt;
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::Services;
use lapiz_tools::{ToolFunction, ToolId};
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{
    button::Button, fluent_builder::When, form::Form, icon, label::Label, panel::Panel,
    spin_slider::SpinSlider,
};

use crate::render::{SelectionOperation, SelectionPipeline};

pub struct MagicWandSelectionTool {
    threshold: f32,
    alpha_threshold: f32,
    grow: i32,
    close_gap: u32,
    cached_feather: u32,
    aa_approach: BucketAntialiasApproach,
}

impl Default for MagicWandSelectionTool {
    fn default() -> Self {
        let b = BucketTool::default();
        Self {
            threshold: b.threshold,
            alpha_threshold: b.alpha_threshold,
            grow: b.grow,
            close_gap: b.close_gap,
            cached_feather: b.cached_feather,
            aa_approach: b.aa_approach,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MagicWandSelectionToolMessage {
    ThresholdChanged(f32),
    AlphaThresholdChanged(f32),
    GrowChanged(i32),
    CloseGapChanged(u32),
    FeatherChanged(u32),
    AaApproachSelected(BucketAntialiasApproach),
}

impl ToolFunction for MagicWandSelectionTool {
    type Message = MagicWandSelectionToolMessage;

    fn id() -> ToolId {
        ToolId::new("magic_wand_selection_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::magic_wand()
    }

    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };
        let canvas_id = canvas.id();

        let position_ws = Vec2::new(mouse.position.x, mouse.position.y);
        let position_ps = canvas.transform.window_to_pixel(position_ws);
        if position_ps.x < 0.0
            || position_ps.y < 0.0
            || position_ps.x > canvas.image.size().x as f32
            || position_ps.y > canvas.image.size().y as f32
        {
            return Task::none();
        }

        let tiles = services.tile_storage();
        let render_context = services.render_context();
        // TODO Reference other layers
        let ref_layer_id = canvas.active_layer_id();
        let ref_layer_info = tiles.get_layer_tiles(ref_layer_id).unwrap();
        let ref_layer_info_buffer = tiles.get_layer_info(ref_layer_id).unwrap();
        let ref_layer = tiles.get_layer_binding_or_empty(ref_layer_id).unwrap();

        let image_size = canvas.image.size();
        let params = BucketParams {
            seed: position_ps.as_uvec2(),
            fill_color: Vec4::ZERO,
            threshold: self.threshold,
            alpha_threshold: self.alpha_threshold,
            contiguous: true,
            close_gap: self.close_gap,
            grow: self.grow,
            aa_approach: match self.aa_approach {
                BucketAntialiasApproach::Feather(_) => {
                    BucketAntialiasApproach::Feather(self.cached_feather)
                }
                _ => self.aa_approach,
            },
            image_size,
        };

        let bucket = Bucket::new(
            &render_context.device,
            ref_layer_info_buffer.texel_type,
            // This won't be used
            ref_layer_info_buffer.texel_type,
        );
        let Some(mask) = bucket.dispatch_mask(
            &render_context.device,
            &render_context.queue,
            &params,
            &ref_layer,
            ref_layer_info.into_iter().collect(),
        ) else {
            return Task::none();
        };

        let selection_layer_id = canvas.image.selection_layer();
        let selection_layer = tiles.get_layer(selection_layer_id).unwrap();
        let selection_layer_info = selection_layer.layer_info();
        let selection_layer_binding = tiles
            .get_layer_binding_or_empty(selection_layer_id)
            .unwrap();

        let selection_pipeline =
            SelectionPipeline::new(&render_context.device, selection_layer_info.texel_type);
        let selection = selection_pipeline.composite_with_tight_input(
            &render_context.device,
            &render_context.queue,
            SelectionOperation::from_modifiers(keyboard.modifiers()),
            &mask,
            &selection_layer,
            &selection_layer_binding,
        );

        let cmd = if let Some(selection) = selection {
            TileReplaceCommand::new(
                "Magic Wand".into(),
                canvas_id,
                &render_context.device,
                &render_context.queue,
                selection_layer_id,
                &selection_layer,
                selection.iter_tiles().map(|(i, _, _)| i).collect(),
                selection.texture_view().unwrap().texture().clone(),
            )
        } else {
            TileReplaceCommand::new_clear(
                "Magic Wand".into(),
                canvas_id,
                &render_context.device,
                &render_context.queue,
                selection_layer_id,
                &selection_layer,
            )
        };
        drop(selection_layer);
        services.push_undo_command_to_current(cmd).log_err();

        Task::none()
    }

    fn handle_message(&mut self, message: Self::Message, _: &mut Services) -> Task<Self::Message> {
        match message {
            MagicWandSelectionToolMessage::ThresholdChanged(value) => self.threshold = value,
            MagicWandSelectionToolMessage::AlphaThresholdChanged(value) => {
                self.alpha_threshold = value
            }
            MagicWandSelectionToolMessage::GrowChanged(value) => self.grow = value,
            MagicWandSelectionToolMessage::CloseGapChanged(value) => self.close_gap = value,
            MagicWandSelectionToolMessage::FeatherChanged(value) => {
                self.cached_feather = value;
                if matches!(self.aa_approach, BucketAntialiasApproach::Feather(_)) {
                    self.aa_approach = BucketAntialiasApproach::Feather(value);
                }
            }
            MagicWandSelectionToolMessage::AaApproachSelected(approach) => {
                self.aa_approach = match approach {
                    BucketAntialiasApproach::Feather(_) => {
                        BucketAntialiasApproach::Feather(self.cached_feather)
                    }
                    other => other,
                };
            }
        }

        Task::none()
    }

    fn tool_option_widget<'a>(
        &'a self,
        _: &'a Services,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let fields = Form::new()
            .push(
                "Threshold",
                SpinSlider::new_01(self.threshold)
                    .on_confirm(MagicWandSelectionToolMessage::ThresholdChanged),
            )
            .push(
                "Alpha Threshold",
                SpinSlider::new_01(self.alpha_threshold)
                    .on_confirm(MagicWandSelectionToolMessage::AlphaThresholdChanged),
            )
            .push(
                "Grow",
                SpinSlider::new(-64..=64, self.grow)
                    .on_confirm(MagicWandSelectionToolMessage::GrowChanged),
            )
            .push(
                "Close Gap",
                SpinSlider::new(0..=64, self.close_gap)
                    .on_confirm(MagicWandSelectionToolMessage::CloseGapChanged),
            )
            .push(
                "Antialiasing Approach",
                row![
                    Button::new(Label::new("None"))
                        .on_press(MagicWandSelectionToolMessage::AaApproachSelected(
                            BucketAntialiasApproach::None
                        ))
                        .activated(matches!(self.aa_approach, BucketAntialiasApproach::None)),
                    Button::new(Label::new("FXAA"))
                        .on_press(MagicWandSelectionToolMessage::AaApproachSelected(
                            BucketAntialiasApproach::Fxaa
                        ))
                        .activated(matches!(self.aa_approach, BucketAntialiasApproach::Fxaa)),
                    Button::new(Label::new("Feather"))
                        .on_press(MagicWandSelectionToolMessage::AaApproachSelected(
                            BucketAntialiasApproach::Feather(self.cached_feather)
                        ))
                        .activated(matches!(
                            self.aa_approach,
                            BucketAntialiasApproach::Feather(_)
                        )),
                ],
            )
            .when(
                matches!(self.aa_approach, BucketAntialiasApproach::Feather(_)),
                |form| {
                    form.push(
                        "Feather",
                        SpinSlider::new(0..=64, self.cached_feather)
                            .on_confirm(MagicWandSelectionToolMessage::FeatherChanged),
                    )
                },
            );

        Some(Panel::new(fields).padding(8).width(Length::Fill).into())
    }
}
