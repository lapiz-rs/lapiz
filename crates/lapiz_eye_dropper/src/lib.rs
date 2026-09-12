use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use glam::{IVec2, Vec2};
use iced_core::{Background, Element, Length, Theme, keyboard::Modifiers};
use iced_runtime::Task;
use iced_widget::{Space, container, row};
use lapiz_canvas::{CanvasAppExt, CanvasId};
use lapiz_color::{Color, ForegroundBackgroundColorExt, ForegroundColorChanged, model::rgb::Rgb};
use lapiz_i18n::t;
use lapiz_image::{
    CImage,
    layer::{
        Layer, LayerId, group_layer::GroupLayer, pixel_layer::PixelLayer,
        properties::LayerTexelTypePropertyExt,
    },
    tile::{GpuTileStorage, TileStorageAppExt},
};
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{
    Application, Renderer, Services, event::Event, plugin::Plugin, service::Service,
};
use lapiz_tools::{ToolFunction, ToolId, ToolsAppExt};
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{
    fluent_builder::When, form::Form, icon, label::Label, panel::Panel,
    segmented_control::SegmentedControl, spin_slider::SpinSlider,
};
use wgpu::{Device, Queue};

use crate::builtin::{GroupLayerEyeDropperTarget, PixelLayerEyeDropperTarget};

pub mod builtin;

lapiz_i18n::define_i18n!("eye_dropper");

pub struct EyeDropperPlugin;

impl Plugin for EyeDropperPlugin {
    fn build(&self, app: &mut Application) {
        i18n::init();
        app.runtime_mut()
            .services_mut()
            .add_tool_function::<EyeDropperTool>();

        let mut registry = EyeDropperTargetRegistry::default();
        registry.register::<PixelLayer, PixelLayerEyeDropperTarget>();
        registry.register::<GroupLayer, GroupLayerEyeDropperTarget>();
        app.runtime_mut().services_mut().insert_service(registry);
    }
}

#[async_trait]
pub trait EyeDropperTarget: Send + Sync + 'static {
    async fn sample(
        &self,
        layer_id: LayerId,
        pixel: IVec2,
        mode: EyeDropperSampleMode,
        tiles: &GpuTileStorage,
        device: &Device,
        queue: &Queue,
    ) -> Result<Color>;
}

#[derive(Default)]
pub struct EyeDropperTargetRegistry {
    inner: HashMap<u32, Arc<dyn EyeDropperTarget>>,
}

impl Service for EyeDropperTargetRegistry {}

impl EyeDropperTargetRegistry {
    pub fn register<L: Layer + Default, T: EyeDropperTarget + Default>(&mut self) {
        self.inner
            .insert(L::default().layer_type(), Arc::new(T::default()));
    }

    pub fn get(&self, layer: &dyn Layer) -> Option<Arc<dyn EyeDropperTarget>> {
        self.inner.get(&layer.layer_type()).cloned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EyeDropperSampleMode {
    Single,
    Average { radius: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EyeDropperTargetMode {
    Current,
    Merged,
}

#[derive(Clone, Copy)]
struct SampleRequest {
    canvas_id: CanvasId,
    layer_id: LayerId,
    pixel: IVec2,
    mode: EyeDropperSampleMode,
}

pub struct EyeDropperTool {
    sample_mode: EyeDropperSampleMode,
    target_mode: EyeDropperTargetMode,
    sampled_color: Option<Color>,
    sample_in_flight: bool,
    pending_sample: Option<SampleRequest>,
}

impl Default for EyeDropperTool {
    fn default() -> Self {
        Self {
            sample_mode: EyeDropperSampleMode::Single,
            target_mode: EyeDropperTargetMode::Merged,
            sampled_color: None,
            sample_in_flight: false,
            pending_sample: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EyeDropperToolMessage {
    SampleModeChanged(EyeDropperSampleMode),
    RadiusChanged(u32),
    TargetModeChanged(EyeDropperTargetMode),
    Sampled(std::result::Result<Color, String>),
}

impl EyeDropperTool {
    fn sample(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &Services,
    ) -> Task<EyeDropperToolMessage> {
        self.pending_sample = None;

        let Some(canvas) = services.current_canvas() else {
            return Task::none();
        };
        let position = canvas
            .transform
            .window_to_pixel(Vec2::new(mouse.position.x, mouse.position.y));

        let target_mode = if keyboard.modifiers().contains(Modifiers::ALT) {
            if keyboard.modifiers().contains(Modifiers::CTRL) {
                EyeDropperTargetMode::Current
            } else {
                EyeDropperTargetMode::Merged
            }
        } else {
            self.target_mode
        };

        let layer_id = match target_mode {
            EyeDropperTargetMode::Merged => Some(*canvas.image.layer_stack().root_id()),
            EyeDropperTargetMode::Current => canvas
                .active_layer_node()
                .properties()
                .get_texel_type()
                .map(|_| canvas.active_layer_id()),
        };
        let Some(layer_id) = layer_id else {
            return Task::none();
        };

        let request = SampleRequest {
            canvas_id: canvas.id(),
            layer_id,
            pixel: position.as_ivec2(),
            mode: self.sample_mode,
        };
        if self.sample_in_flight {
            self.pending_sample = Some(request);
            Task::none()
        } else {
            self.start_sample(request, &canvas.image, services)
        }
    }

    fn start_sample(
        &mut self,
        request: SampleRequest,
        image: &CImage,
        services: &Services,
    ) -> Task<EyeDropperToolMessage> {
        let registry = services.service::<EyeDropperTargetRegistry>();
        let layer = image.layer_stack().get_layer(&request.layer_id).unwrap();
        let Some(target) = registry.get(layer.instance()) else {
            return Task::none();
        };

        let tiles = services.tile_storage().clone();
        let device = services.render_device().clone();
        let queue = services.render_queue().clone();

        self.sample_in_flight = true;

        Task::future(async move {
            EyeDropperToolMessage::Sampled(
                target
                    .sample(
                        request.layer_id,
                        request.pixel,
                        request.mode,
                        &tiles,
                        &device,
                        &queue,
                    )
                    .await
                    .map_err(|error| error.to_string()),
            )
        })
    }
}

impl ToolFunction for EyeDropperTool {
    type Message = EyeDropperToolMessage;

    fn id() -> ToolId {
        ToolId::new("eye_dropper_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::eyedropper()
    }

    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        self.sample(keyboard, mouse, services)
    }

    fn update(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        self.sample(keyboard, mouse, services)
    }

    fn handle_message(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        match message {
            EyeDropperToolMessage::SampleModeChanged(mode) => self.sample_mode = mode,
            EyeDropperToolMessage::RadiusChanged(radius) => {
                self.sample_mode = EyeDropperSampleMode::Average { radius };
            }
            EyeDropperToolMessage::TargetModeChanged(mode) => self.target_mode = mode,
            EyeDropperToolMessage::Sampled(result) => {
                self.sample_in_flight = false;
                if let Ok(color) = result.logged_err() {
                    let old = services.foreground_color().get();
                    services.foreground_color_mut().set(color);
                    ForegroundColorChanged::broadcast(ForegroundColorChanged::new(old, color));
                    self.sampled_color = Some(color);
                }

                if let Some(pending) = self.pending_sample.take()
                    && let Some(canvas) = services.canvas(&pending.canvas_id)
                {
                    return self.start_sample(pending, &canvas.image, services);
                }
            }
        }

        Task::none()
    }

    fn tool_option_widget<'a>(
        &'a self,
        services: &'a Services,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let radius = match self.sample_mode {
            EyeDropperSampleMode::Single => 1,
            EyeDropperSampleMode::Average { radius } => radius,
        };
        let sampled_rgb = self.sampled_color.map(|color| {
            let profile = services
                .current_canvas()
                .expect("Tool options should only be shown for the current canvas")
                .image
                .profile();
            color.into_rgb(profile.rgb_to_xyz_matrix().to_f32().inverse())
        });
        let color = sampled_rgb.unwrap_or(Rgb::new(0.0, 0.0, 0.0));
        let preview_color = iced_core::Color::from_rgb(color.r, color.g, color.b);
        let color_text = sampled_rgb.map_or_else(
            || "—".to_owned(),
            |color| {
                format!(
                    "#{:02X}{:02X}{:02X}",
                    (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
                    (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
                    (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
                )
            },
        );
        let preview = row![
            container(Space::new().width(Length::Fill).height(32)).style(move |_| {
                container::Style {
                    background: Some(Background::Color(preview_color)),
                    ..Default::default()
                }
            }),
            Label::new(color_text),
        ]
        .spacing(8)
        .align_y(iced_core::Alignment::Center);

        let fields = Form::new()
            .push(
                t!("sample_mode"),
                SegmentedControl::new()
                    .push(
                        Label::new(t!("single")),
                        matches!(self.sample_mode, EyeDropperSampleMode::Single),
                        EyeDropperToolMessage::SampleModeChanged(EyeDropperSampleMode::Single),
                    )
                    .push(
                        Label::new(t!("average")),
                        matches!(self.sample_mode, EyeDropperSampleMode::Average { .. }),
                        EyeDropperToolMessage::SampleModeChanged(EyeDropperSampleMode::Average {
                            radius,
                        }),
                    ),
            )
            .when(
                matches!(self.sample_mode, EyeDropperSampleMode::Average { .. }),
                |form| {
                    form.push(
                        t!("radius"),
                        SpinSlider::new(1..=64, radius)
                            .on_confirm(EyeDropperToolMessage::RadiusChanged),
                    )
                },
            )
            .push(
                t!("sample_target"),
                SegmentedControl::new()
                    .push(
                        Label::new(t!("current_layer")),
                        self.target_mode == EyeDropperTargetMode::Current,
                        EyeDropperToolMessage::TargetModeChanged(EyeDropperTargetMode::Current),
                    )
                    .push(
                        Label::new(t!("all_layers")),
                        self.target_mode == EyeDropperTargetMode::Merged,
                        EyeDropperToolMessage::TargetModeChanged(EyeDropperTargetMode::Merged),
                    ),
            )
            .push(t!("sampled_color"), preview);

        Some(Panel::new(fields).padding(8).width(Length::Fill).into())
    }
}
