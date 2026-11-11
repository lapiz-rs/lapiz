use std::{collections::HashMap, result, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use glam::{IVec2, Vec2};
use iced_core::{
    Background, Element, Length, Padding, Point, Theme, keyboard::Modifiers, widget::Void,
};
use iced_runtime::Task;
use lapiz_canvas::{CanvasAppExt as _, CanvasId};
use lapiz_color::{
    Color, ForegroundBackgroundColorExt as _, ForegroundColorChanged, model::rgb::Rgb,
};
use lapiz_i18n::t;
use lapiz_image::{
    CImage,
    layer::{
        Layer, LayerId, group_layer::GroupLayer, pixel_layer::PixelLayer,
        properties::builtin::LayerTexelTypePropertyExt as _,
    },
    tile::{GpuTileStorage, TileStorageAppExt as _},
};
use lapiz_input::{
    key::KeyboardState,
    mouse::{HoverMouseState, PressedMouseState},
};
use lapiz_render::render_context::RenderContextAppExt as _;
use lapiz_runtime::{
    Renderer, Runtime,
    event::Event as _,
    global::{Global, Globals},
    plugin::Plugin,
};
use lapiz_tools::{ToolFunction, ToolId, ToolsAppExt as _};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    container, fluent_builder::When as _, form, icon, label, panel, row, segmented_control, space,
    spin_slider,
};
use wgpu::{Device, Queue};

use crate::builtin::{GroupLayerEyeDropperTarget, PixelLayerEyeDropperTarget};

pub mod builtin;

lapiz_i18n::define_i18n!("eye_dropper");

/// A [`Plugin`] that registers [`EyeDropperTool`] and built-in [`EyeDropperTarget`] implementations.
pub struct EyeDropperPlugin;

impl Plugin for EyeDropperPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.globals_mut().add_tool_function::<EyeDropperTool>();

        let mut registry = EyeDropperTargetRegistry::default();
        registry.register::<PixelLayer, PixelLayerEyeDropperTarget>();
        registry.register::<GroupLayer, GroupLayerEyeDropperTarget>();
        app.add_global_instance(registry);
    }
}

/// Asynchronous color-sampling backend for a specific [`Layer`] type.
///
/// Implementations read back pixel data from [`GpuTileStorage`] around the requested canvas pixel
/// coordinate according to the [`EyeDropperSampleMode`] and return the sampled [`Color`].
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

/// Global registry mapping [`Layer`] type IDs to their corresponding [`EyeDropperTarget`] samplers.
#[derive(Default)]
pub struct EyeDropperTargetRegistry {
    inner: HashMap<u32, Arc<dyn EyeDropperTarget>>,
}

impl Global for EyeDropperTargetRegistry {}

impl EyeDropperTargetRegistry {
    pub fn register<L: Layer + Default, T: EyeDropperTarget + Default>(&mut self) {
        self.inner
            .insert(L::default().layer_type(), Arc::new(T::default()));
    }

    pub fn get(&self, layer: &dyn Layer) -> Option<Arc<dyn EyeDropperTarget>> {
        self.inner.get(&layer.layer_type()).cloned()
    }
}

/// Specifies whether the eyedropper samples a single pixel or averages a square neighborhood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EyeDropperSampleMode {
    Single,
    Average { radius: u32 },
}

/// Specifies whether the eyedropper samples only the active layer or the composited root image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EyeDropperTargetMode {
    Current,
    Merged,
}

/// Queued parameters for an asynchronous eyedropper pixel readback operation.
#[derive(Clone, Copy)]
struct SampleRequest {
    canvas_id: CanvasId,
    layer_id: LayerId,
    pixel: IVec2,
    mode: EyeDropperSampleMode,
}

/// Interactive canvas tool for sampling colors from layers into the global foreground color.
///
/// `EyeDropperTool` implements [`ToolFunction`] and dispatches asynchronous GPU tile readbacks
/// through [`EyeDropperTargetRegistry`] when pressing or dragging on the canvas. To avoid
/// overlapping GPU readbacks during rapid drags, at most one sample request is kept in flight at
/// a time while the latest cursor position is queued in `pending_sample`. When a sample completes,
/// it updates the global foreground color, broadcasts [`ForegroundColorChanged`], and renders a
/// floating color swatch above the cursor.
pub struct EyeDropperTool {
    sample_mode: EyeDropperSampleMode,
    target_mode: EyeDropperTargetMode,
    sampled_color: Option<Color>,
    cursor_position: Option<Point>,
    sample_in_flight: bool,
    pending_sample: Option<SampleRequest>,
}

impl Default for EyeDropperTool {
    fn default() -> Self {
        Self {
            sample_mode: EyeDropperSampleMode::Single,
            target_mode: EyeDropperTargetMode::Merged,
            sampled_color: None,
            cursor_position: None,
            sample_in_flight: false,
            pending_sample: None,
        }
    }
}

/// Messages handled by [`EyeDropperTool`] for option changes and completed GPU color readbacks.
#[derive(Debug, Clone)]
pub enum EyeDropperToolMessage {
    SampleModeChanged(EyeDropperSampleMode),
    RadiusChanged(u32),
    TargetModeChanged(EyeDropperTargetMode),
    Sampled(result::Result<Color, String>),
}

impl EyeDropperTool {
    fn sample(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &Globals,
    ) -> Task<EyeDropperToolMessage> {
        self.pending_sample = None;
        self.cursor_position = Some(mouse.position);

        let Some(canvas) = globals.current_canvas() else {
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
            self.start_sample(request, &canvas.image, globals)
        }
    }

    fn sampled_rgb(&self, globals: &Globals) -> Option<Rgb> {
        let color = self.sampled_color?;
        let profile = globals.current_canvas()?.image.profile();
        Some(color.into_rgb(profile.rgb_to_xyz_matrix().to_f32().inverse()))
    }

    fn start_sample(
        &mut self,
        request: SampleRequest,
        image: &CImage,
        globals: &Globals,
    ) -> Task<EyeDropperToolMessage> {
        let registry = globals.global::<EyeDropperTargetRegistry>();
        let layer = image.layer_stack().get_layer(&request.layer_id).unwrap();
        let Some(target) = registry.get(layer.instance()) else {
            return Task::none();
        };

        let tiles = globals.tile_storage().clone();
        let device = globals.render_device().clone();
        let queue = globals.render_queue().clone();

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

    fn hover(
        &mut self,
        _: &KeyboardState,
        mouse: &HoverMouseState,
        _: &mut Globals,
    ) -> Task<Self::Message> {
        self.cursor_position = Some(mouse.position);
        Task::none()
    }

    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        self.sample(keyboard, mouse, globals)
    }

    fn update(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        self.sample(keyboard, mouse, globals)
    }

    fn end(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        _: &mut Globals,
    ) -> Task<Self::Message> {
        self.cursor_position = Some(mouse.position);
        Task::none()
    }

    fn deactivate(&mut self, _: &mut Globals) -> Task<Self::Message> {
        self.cursor_position = None;
        Task::none()
    }

    fn handle_message(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
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
                    let old = globals.foreground_color().get();
                    globals.foreground_color_mut().set(color);
                    ForegroundColorChanged::broadcast(ForegroundColorChanged::new(old, color));
                    self.sampled_color = Some(color);
                }

                if let Some(pending) = self.pending_sample.take()
                    && let Some(canvas) = globals.canvas(&pending.canvas_id)
                {
                    return self.start_sample(pending, &canvas.image, globals);
                }
            }
        }

        Task::none()
    }

    fn tool_option_widget<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let radius = match self.sample_mode {
            EyeDropperSampleMode::Single => 1,
            EyeDropperSampleMode::Average { radius } => radius,
        };
        let sampled_rgb = self.sampled_rgb(globals);
        let color = sampled_rgb.unwrap_or(Rgb::new(0.0, 0.0, 0.0));
        let preview_color = iced_core::Color::from_rgb(
            color.r.clamp(0.0, 1.0),
            color.g.clamp(0.0, 1.0),
            color.b.clamp(0.0, 1.0),
        );
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
            container(space().width(Length::Fill).height(32)).style(move |_| {
                container::Style {
                    background: Some(Background::Color(preview_color)),
                    ..Default::default()
                }
            }),
            label(color_text),
        ]
        .gap(8.0);

        let fields = form()
            .push(
                t!("sample_mode"),
                segmented_control()
                    .push(
                        label(t!("single")),
                        matches!(self.sample_mode, EyeDropperSampleMode::Single),
                        EyeDropperToolMessage::SampleModeChanged(EyeDropperSampleMode::Single),
                    )
                    .push(
                        label(t!("average")),
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
                        spin_slider(1..=64, radius)
                            .on_confirm(EyeDropperToolMessage::RadiusChanged),
                    )
                },
            )
            .push(
                t!("sample_target"),
                segmented_control()
                    .push(
                        label(t!("current_layer")),
                        self.target_mode == EyeDropperTargetMode::Current,
                        EyeDropperToolMessage::TargetModeChanged(EyeDropperTargetMode::Current),
                    )
                    .push(
                        label(t!("all_layers")),
                        self.target_mode == EyeDropperTargetMode::Merged,
                        EyeDropperToolMessage::TargetModeChanged(EyeDropperTargetMode::Merged),
                    ),
            )
            .push(t!("sampled_color"), preview);

        Some(panel(fields).padding(8).width(Length::Fill).into())
    }

    fn canvas_overlay<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        const SWATCH_SIZE: f32 = 60.0;

        let (Some(cursor), Some(color), Some(canvas)) = (
            self.cursor_position,
            self.sampled_rgb(globals),
            globals.current_canvas(),
        ) else {
            return Void.into();
        };
        let Some(position) = canvas
            .transform
            .window_to_in_widget(Vec2::new(cursor.x, cursor.y))
        else {
            return Void.into();
        };

        let preview_color = iced_core::Color::from_rgb(
            color.r.clamp(0.0, 1.0),
            color.g.clamp(0.0, 1.0),
            color.b.clamp(0.0, 1.0),
        );
        let swatch = container(space().width(SWATCH_SIZE).height(SWATCH_SIZE))
            .width(SWATCH_SIZE)
            .height(SWATCH_SIZE)
            .style(move |_| container::Style {
                background: Some(Background::Color(preview_color)),
                ..Default::default()
            });

        container(swatch)
            .padding(Padding {
                top: position.y - SWATCH_SIZE - 40.0,
                right: 0.0,
                bottom: 0.0,
                left: position.x - SWATCH_SIZE * 0.5,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
