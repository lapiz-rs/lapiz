use glam::{Vec2, Vec4};
use iced_core::{
    Element, Length, Theme,
    keyboard::{self, key},
};
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_wgpu::Renderer;
use iced_widget::row;
use lapiz_canvas::{CanvasAppExt, CanvasUndoStackAppExt, command::TileReplaceCommand};
use lapiz_color::ForegroundBackgroundColorExt;
use lapiz_i18n::{Translated, t};
use lapiz_image::{
    blend_modes::BlendMode,
    composite::{BlendFunction, BlendFunctionId, BlendFunctionRegistry},
    tile::TileStorageAppExt,
};
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{Application, Services, plugin::Plugin};
use lapiz_tools::{ToolFunction, ToolId, ToolsAppExt};
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{
    button::Button, checkbox::Checkbox, combo_box::ComboBox, fluent_builder::When, form::Form,
    icon, label::Label, panel::Panel, spin_slider::SpinSlider,
};
use tracing::error;

use crate::bucket::{Bucket, BucketAntialiasApproach, BucketParams};

lapiz_i18n::define_i18n!("bucket_tool");

pub mod bucket;

pub struct BucketPlugin;

impl Plugin for BucketPlugin {
    fn build(&self, app: &mut Application) {
        crate::i18n::init();
        app.runtime_mut()
            .services_mut()
            .add_tool_function::<BucketTool>();
    }
}

pub struct BucketTool {
    pub threshold: f32,
    pub alpha_threshold: f32,
    pub grow: i32,
    pub contiguous: bool,
    pub close_gap: u32,
    pub cached_feather: u32,
    pub blend_function: BlendFunctionId,
    pub aa_approach: BucketAntialiasApproach,
}

impl Default for BucketTool {
    fn default() -> Self {
        Self {
            threshold: 0.08,
            alpha_threshold: 0.02,
            grow: 0,
            contiguous: true,
            close_gap: 0,
            cached_feather: 0,
            blend_function: BlendMode::Normal.id(),
            aa_approach: BucketAntialiasApproach::Fxaa,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BucketToolMessage {
    ThresholdChanged(f32),
    AlphaThresholdChanged(f32),
    GrowChanged(i32),
    ContiguousChanged(bool),
    CloseGapChanged(u32),
    FeatherChanged(u32),
    BlendFunctionChanged(BlendFunctionId),
    AaApproachSelected(BucketAntialiasApproach),
}

impl ToolFunction for BucketTool {
    type Message = BucketToolMessage;

    fn id() -> ToolId {
        ToolId::new("bucket_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::fill()
    }

    fn end(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(canvas_id) = services.current_canvas_id() else {
            return Task::none();
        };
        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };

        let position_ws = Vec2::new(mouse.position.x, mouse.position.y);
        let position_ps = canvas.transform.window_to_pixel(position_ws);
        if position_ps.x < 0.0
            || position_ps.y < 0.0
            || position_ps.x > canvas.image.size().x as f32
            || position_ps.y > canvas.image.size().y as f32
        {
            return Task::none();
        }

        let profile = canvas.image.profile();
        let fg_color = services
            .foreground_color()
            .get()
            .into_rgb(profile.rgb_to_xyz_matrix().to_f32().inverse());

        let tiles = services.tile_storage();
        // TODO Reference other layers
        let ref_layer_id = canvas.active_layer_id();
        let ref_layer_info = tiles.get_layer_tiles(ref_layer_id).unwrap();
        let ref_layer_info_buffer = tiles.get_layer_info(ref_layer_id).unwrap();
        let ref_layer = tiles.get_layer_binding_or_empty(ref_layer_id).unwrap();

        let output_layer_id = canvas.active_layer_id();
        let output_layer_info = tiles.get_layer_info(output_layer_id).unwrap();
        let output_layer = tiles.get_layer_binding_or_empty(output_layer_id).unwrap();

        let selection_layer = canvas.image.selection_layer();
        let selection_layer = tiles.get_layer_binding_or_empty(selection_layer).unwrap();

        let image_size = canvas.image.size();
        let params = BucketParams {
            seed: position_ps.as_uvec2(),
            fill_color: Vec4::new(fg_color.r, fg_color.g, fg_color.b, 1.0),
            threshold: self.threshold,
            alpha_threshold: self.alpha_threshold,
            contiguous: self.contiguous,
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

        let device = services.render_device();
        let queue = services.render_queue();
        let Some(blend_function) = services
            .service::<BlendFunctionRegistry>()
            .get(&self.blend_function)
        else {
            error!("Failed to get blend function: {}", self.blend_function);
            return Task::none();
        };

        let mut bucket = Bucket::new(
            device,
            ref_layer_info_buffer.texel_type,
            output_layer_info.texel_type,
        );
        bucket.set_blend_function(device, blend_function.as_ref());
        let result = bucket.dispatch_composite(
            device,
            queue,
            &params,
            &ref_layer,
            ref_layer_info.into_iter().collect(),
            &output_layer,
            &selection_layer,
        );

        if let Some(new_tiles) = result {
            let output_layer = tiles.get_layer(output_layer_id).unwrap();
            let cmd = TileReplaceCommand::new(
                "Bucket Fill".into(),
                canvas_id,
                device,
                queue,
                output_layer_id,
                &output_layer,
                new_tiles.iter_tile_indices().collect(),
                new_tiles.texture().unwrap().clone(),
            );
            drop(output_layer);
            services.push_undo_command_to_current(cmd).log_err();
        }

        Task::none()
    }

    fn handle_message(&mut self, message: Self::Message, _: &mut Services) -> Task<Self::Message> {
        match message {
            BucketToolMessage::ThresholdChanged(value) => self.threshold = value,
            BucketToolMessage::AlphaThresholdChanged(value) => self.alpha_threshold = value,
            BucketToolMessage::GrowChanged(value) => self.grow = value,
            BucketToolMessage::ContiguousChanged(value) => self.contiguous = value,
            BucketToolMessage::CloseGapChanged(value) => self.close_gap = value,
            BucketToolMessage::FeatherChanged(value) => {
                self.cached_feather = value;
                if matches!(self.aa_approach, BucketAntialiasApproach::Feather(_)) {
                    self.aa_approach = BucketAntialiasApproach::Feather(value);
                }
            }
            BucketToolMessage::BlendFunctionChanged(value) => self.blend_function = value,
            BucketToolMessage::AaApproachSelected(approach) => {
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
        services: &'a Services,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let blend_functions = services.service::<BlendFunctionRegistry>();

        let fields = Form::new()
            .push(
                t!("threshold"),
                SpinSlider::new_01(self.threshold).on_confirm(BucketToolMessage::ThresholdChanged),
            )
            .push(
                t!("alpha_threshold"),
                SpinSlider::new_01(self.alpha_threshold)
                    .on_confirm(BucketToolMessage::AlphaThresholdChanged),
            )
            .push(
                t!("grow"),
                SpinSlider::new(-64..=64, self.grow).on_confirm(BucketToolMessage::GrowChanged),
            )
            .push(
                t!("contiguous"),
                Checkbox::new(self.contiguous).on_toggle(BucketToolMessage::ContiguousChanged),
            )
            .push(
                t!("close_gap"),
                SpinSlider::new(0..=64, self.close_gap)
                    .on_confirm(BucketToolMessage::CloseGapChanged),
            )
            .push(
                t!("blend_function"),
                ComboBox::new(
                    blend_functions
                        .all_ids()
                        .cloned()
                        .map(Translated)
                        .collect::<Vec<_>>(),
                    Some(Translated(self.blend_function.clone())),
                    |option| BucketToolMessage::BlendFunctionChanged(option.into_inner()),
                ),
            )
            .push(
                t!("antialiasing_approach"),
                row![
                    Button::new(Label::new(t!("none")))
                        .on_press(BucketToolMessage::AaApproachSelected(
                            BucketAntialiasApproach::None
                        ))
                        .activated(matches!(self.aa_approach, BucketAntialiasApproach::None)),
                    Button::new(Label::new(t!("fxaa")))
                        .on_press(BucketToolMessage::AaApproachSelected(
                            BucketAntialiasApproach::Fxaa
                        ))
                        .activated(matches!(self.aa_approach, BucketAntialiasApproach::Fxaa)),
                    Button::new(Label::new(t!("feather")))
                        .on_press(BucketToolMessage::AaApproachSelected(
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
                        t!("feather"),
                        SpinSlider::new(0..=64, self.cached_feather)
                            .on_confirm(BucketToolMessage::FeatherChanged)
                            .precision(0),
                    )
                },
            );

        Some(Panel::new(fields).padding(8).width(Length::Fill).into())
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        iced_futures::keyboard::listen().filter_map(|event| match event {
            keyboard::Event::KeyPressed {
                physical_key: key::Physical::Code(key::Code::ShiftLeft),
                repeat: false,
                ..
            } => Some(BucketToolMessage::ContiguousChanged(false)),
            keyboard::Event::KeyReleased {
                physical_key: key::Physical::Code(key::Code::ShiftLeft),
                ..
            } => Some(BucketToolMessage::ContiguousChanged(true)),
            _ => None,
        })
    }
}
