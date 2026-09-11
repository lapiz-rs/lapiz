use std::future::Future;

use anyhow::{Context, Result, anyhow};
use glam::{IVec2, UVec2, Vec2};
use iced_core::{Background, Element, Length, Theme, keyboard::Modifiers};
use iced_runtime::Task;
use iced_widget::{Space, container, row};
use lapiz_canvas::{CCanvas, CanvasAppExt};
use lapiz_color::{Color, ForegroundBackgroundColorExt, ForegroundColorChanged, model::rgb::Rgb};
use lapiz_i18n::t;
use lapiz_image::{
    layer::{LayerId, properties::LayerTexelTypePropertyExt},
    tile::{GpuTileStorage, TileStorageAppExt},
};
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{Application, Renderer, Services, event::Event, plugin::Plugin};
use lapiz_tools::{ToolFunction, ToolId, ToolsAppExt};
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{
    button::Button, fluent_builder::When, form::Form, icon, label::Label, panel::Panel,
    segmented_control::SegmentedControl, spin_slider::SpinSlider,
};
use wgpu::{Device, Queue};

lapiz_i18n::define_i18n!("eye_dropper");

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

pub trait EyeDropperTarget: Send + 'static {
    fn sample(
        &self,
        pixel: IVec2,
        mode: EyeDropperSampleMode,
        tiles: &GpuTileStorage,
        device: &Device,
        queue: &Queue,
    ) -> impl Future<Output = Result<Rgb>> + Send;
}

struct StoredLayerTarget {
    layer_id: LayerId,
}

impl StoredLayerTarget {
    fn new(layer_id: LayerId) -> Self {
        Self { layer_id }
    }
}

impl EyeDropperTarget for StoredLayerTarget {
    async fn sample(
        &self,
        pixel: IVec2,
        mode: EyeDropperSampleMode,
        tiles: &GpuTileStorage,
        device: &Device,
        queue: &Queue,
    ) -> Result<Rgb> {
        let radius = match mode {
            EyeDropperSampleMode::Single => 0,
            EyeDropperSampleMode::Average { radius } => radius as i32,
        };
        let min = pixel - IVec2::splat(radius);
        let max = pixel + IVec2::splat(radius) + IVec2::ONE;
        let min_tile = min / GpuTileStorage::TILE_SIZE as i32;
        let max_tile = (max - IVec2::ONE) / GpuTileStorage::TILE_SIZE as i32;
        let requested_tiles = (min_tile.y..=max_tile.y)
            .flat_map(|y| (min_tile.x..=max_tile.x).map(move |x| IVec2::new(x, y)));

        let layer = tiles
            .get_layer(self.layer_id)
            .ok_or_else(|| anyhow!("Eye dropper target layer {} is unavailable", self.layer_id))?;
        let buffers = layer.readback(device, queue, requested_tiles).await?;
        drop(layer);

        let mut sum = [0.0; 3];
        let mut count = 0;
        for y in min.y..max.y {
            for x in min.x..max.x {
                let position = IVec2::new(x, y);
                let tile = position / GpuTileStorage::TILE_SIZE as i32;
                if let Some(buffer) = buffers.get(&tile) {
                    let local = position - tile * GpuTileStorage::TILE_SIZE as i32;
                    let offset = ((local.y as u32 * GpuTileStorage::TILE_SIZE + local.x as u32) * 4)
                        as usize;
                    sum[0] += buffer[offset] as f32 / 255.0;
                    sum[1] += buffer[offset + 1] as f32 / 255.0;
                    sum[2] += buffer[offset + 2] as f32 / 255.0;
                }
                count += 1;
            }
        }

        let scale = 1.0 / count as f32;
        Ok(Rgb::new(sum[0] * scale, sum[1] * scale, sum[2] * scale))
    }
}

pub struct EyeDropperPlugin;

impl Plugin for EyeDropperPlugin {
    fn build(&self, app: &mut Application) {
        i18n::init();
        app.runtime_mut()
            .services_mut()
            .add_tool_function::<EyeDropperTool>();
    }
}

pub struct EyeDropperTool {
    sample_mode: EyeDropperSampleMode,
    target_mode: EyeDropperTargetMode,
    sampled_color: Option<Rgb>,
    latest_request: u64,
}

impl Default for EyeDropperTool {
    fn default() -> Self {
        Self {
            sample_mode: EyeDropperSampleMode::Single,
            target_mode: EyeDropperTargetMode::Merged,
            sampled_color: None,
            latest_request: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EyeDropperToolMessage {
    SampleModeChanged(EyeDropperSampleMode),
    RadiusChanged(u32),
    TargetModeChanged(EyeDropperTargetMode),
    Sampled {
        request: u64,
        result: std::result::Result<Rgb, String>,
    },
}

impl EyeDropperTool {
    fn sample(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &Services,
    ) -> Task<EyeDropperToolMessage> {
        self.latest_request += 1;
        let request = self.latest_request;

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

        let target = StoredLayerTarget::new(layer_id);
        let pixel = position.as_ivec2();
        let sample_mode = self.sample_mode;
        let tiles = services.tile_storage().clone();
        let device = services.render_device().clone();
        let queue = services.render_queue().clone();

        Task::future(async move {
            EyeDropperToolMessage::Sampled {
                request,
                result: target
                    .sample(pixel, sample_mode, &tiles, &device, &queue)
                    .await
                    .map_err(|error| error.to_string()),
            }
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
            EyeDropperToolMessage::Sampled { request, result } => {
                if request != self.latest_request {
                    return Task::none();
                }
                let Ok(color) = result.logged_err() else {
                    return Task::none();
                };
                let old = services.foreground_color().get();
                let new = Color::Rgb(color);
                services.foreground_color_mut().set(new);
                ForegroundColorChanged::broadcast(ForegroundColorChanged::new(old, new));
                self.sampled_color = Some(color);
            }
        }

        Task::none()
    }

    fn tool_option_widget<'a>(
        &'a self,
        _: &'a Services,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let radius = match self.sample_mode {
            EyeDropperSampleMode::Single => 1,
            EyeDropperSampleMode::Average { radius } => radius,
        };
        let color = self.sampled_color.unwrap_or(Rgb::new(0.0, 0.0, 0.0));
        let preview_color = iced_core::Color::from_rgb(color.r, color.g, color.b);
        let color_text = self.sampled_color.map_or_else(
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
