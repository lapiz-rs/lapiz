use std::{collections::HashMap, f32::consts::TAU};

use bevy_math::IRect;
use encase::ShaderType;
use glam::{IVec2, Vec2};
use iced_core::{
    Color, Element, Length, Point, Rectangle, Size, Theme, Vector, Widget, layout, pointer::mouse,
    renderer, widget,
};
use iced_runtime::{Task, futures::Subscription};
use iced_widget::{
    canvas::{Frame, Path, Stroke},
    column, row, space,
};
use lapiz_canvas::{
    CanvasAppExt, CanvasId, CanvasUndoStackAppExt,
    command::TileReplaceCommand,
    control::CanvasTransform,
    event::{CanvasActiveLayerChanged, CanvasUpdated},
};
use lapiz_i18n::t;
use lapiz_image::{
    composite::{LayerPreviewOverriders, PixelPreviewOverrider},
    layer::LayerId,
    scan_pixels::ScanPixelsPipeline,
    texel::TexelType,
    tile::{
        DynamicLayerStorage, GpuLayerInfo, GpuTileInfo, GpuTileStorage, LayerBinding,
        TileStorageAppExt,
    },
};
use lapiz_input::{
    key::KeyboardState,
    mouse::{HoverMouseState, PressedMouseState},
};
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{DynamicBindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    render_context::RenderContextAppExt,
    wesl_jit,
};
use lapiz_runtime::Renderer;
use lapiz_runtime::{Services, event::Event};
use lapiz_tools::{ToolFunction, ToolId};
use lapiz_undo::BatchedUndoCommand;
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{
    button::Button, combo_box::ComboBox, form::Form, icon, label::Label, panel::Panel,
    spin_slider::SpinSlider,
};
use parse_display::Display;
use tracing::warn;
use wgpu::{
    BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, BufferUsages,
    ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor, Device,
    PipelineLayoutDescriptor, Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StorageTextureAccess, TextureFormat,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display)]
pub enum LiquifyMode {
    #[default]
    Move,
    Scale,
    Rotate,
    Offset,
    Undo,
}

impl LiquifyMode {
    fn shader_mode(self) -> u32 {
        match self {
            Self::Move => 0,
            Self::Scale => 1,
            Self::Rotate => 2,
            Self::Offset => 3,
            Self::Undo => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LiquifyProperties {
    pub mode: LiquifyMode,
    pub size: f32,
    pub amount: f32,
    pub spacing: f32,
    pub reverse: bool,
}

impl Default for LiquifyProperties {
    fn default() -> Self {
        Self {
            mode: LiquifyMode::Move,
            size: 60.0,
            amount: 0.05,
            spacing: 0.2,
            reverse: false,
        }
    }
}

#[derive(ShaderType, Debug, Clone, Copy)]
pub struct LiquifyDabParams {
    pub center: Vec2,
    pub dir: Vec2,
    pub mode: u32,
    pub sigma: f32,
    pub magnitude: f32,
}

pub struct LiquifySession {
    pub canvas_id: CanvasId,
    pub target_layers: Vec<LayerId>,
    pub selection_layer_id: LayerId,
    pub has_selection: Buffer,
    pub disp: DynamicLayerStorage,
    pub disp_back: DynamicLayerStorage,
    pub result_buffers: HashMap<LayerId, DynamicLayerStorage>,
    pub render_pipelines: HashMap<TexelType, LiquifyPipeline>,
    pub dab_pipeline: LiquifyPipeline,
    pub last_dab: Vec2,
    pub stroking: bool,
}

impl LiquifySession {
    pub fn apply_stroke(
        &mut self,
        props: &LiquifyProperties,
        cursor_ps: Vec2,
        services: &mut Services,
    ) {
        if !self.stroking {
            return;
        }

        let spacing = (props.spacing * props.size).max(1.0);
        let mut last = self.last_dab;
        loop {
            let delta = cursor_ps - last;
            let dist = delta.length();
            if dist < spacing {
                break;
            }
            let dir = delta / dist;
            let next = last + dir * spacing;
            let seg = next - last;
            self.apply_dab(props, services, next, seg, dir);
            last = next;
        }
        self.last_dab = last;

        self.render_preview(services);
    }

    fn apply_dab(
        &mut self,
        props: &LiquifyProperties,
        services: &Services,
        center: Vec2,
        drag_delta: Vec2,
        stroke_dir: Vec2,
    ) {
        let device = services.render_device();
        let queue = services.render_queue();
        let sigma = props.size * 0.5;

        let sign = if props.reverse { -1.0 } else { 1.0 };
        let (magnitude, dir, drag_vec, gradient) = match props.mode {
            LiquifyMode::Move => {
                let len = drag_delta.length();
                (0.0, Vec2::ZERO, sign * drag_delta, 1.22 * len / props.size)
            }
            LiquifyMode::Scale => (
                sign * props.amount,
                Vec2::ZERO,
                Vec2::ZERO,
                1.2 * props.amount,
            ),
            LiquifyMode::Rotate => (
                sign * TAU * props.amount,
                Vec2::ZERO,
                Vec2::ZERO,
                TAU * props.amount,
            ),
            LiquifyMode::Offset => (
                sign * props.size * props.amount,
                stroke_dir,
                Vec2::ZERO,
                1.22 * props.amount,
            ),
            LiquifyMode::Undo => (props.amount, Vec2::ZERO, Vec2::ZERO, 0.0),
        };

        // Keep the per-substep displacement gradient below 1 so that every
        // substep `id + d` stays a diffeomorphism and the field never folds.
        let substeps = ((gradient / 0.5).ceil() as u32).clamp(1, 64);

        let cutoff = (sigma * 3.0).ceil() as i32;
        let center_i = center.as_ivec2();
        let dab_rect = IRect::new(
            center_i.x - cutoff,
            center_i.y - cutoff,
            center_i.x + cutoff,
            center_i.y + cutoff,
        );
        let tile_rect = GpuTileStorage::pixel_rect_to_tile(dab_rect);
        let dirty = (tile_rect.min.y..tile_rect.max.y)
            .flat_map(|y| (tile_rect.min.x..tile_rect.max.x).map(move |x| IVec2::new(x, y)))
            .collect::<Vec<_>>();

        self.disp.allocate_tiles_batch(dirty.iter().copied());
        self.disp_back
            .allocate_tiles_batch(self.disp.iter_tile_indices());

        for _ in 0..substeps {
            let params = LiquifyDabParams {
                mode: props.mode.shader_mode(),
                center,
                sigma,
                magnitude: magnitude / substeps as f32,
                dir: if props.mode == LiquifyMode::Move {
                    drag_vec / substeps as f32
                } else {
                    dir
                },
            };

            let src = self.disp.binding_or_empty();
            let dst = self.disp_back.binding_or_empty();
            self.dab_pipeline.dispatch_dab(
                device,
                queue,
                &params,
                &src,
                &dst,
                self.disp_back.len() as u32,
            );
            std::mem::swap(&mut self.disp, &mut self.disp_back);
        }
    }

    fn render_preview(&mut self, services: &mut Services) {
        let device = services.render_device().clone();
        let queue = services.render_queue().clone();
        let tiles = services.tile_storage().clone();
        let dirty_tiles = self.disp.compute_tile_bounds();

        let selection_binding = tiles
            .get_layer_binding_or_empty(self.selection_layer_id)
            .unwrap();

        for (layer_id, result) in &mut self.result_buffers {
            result.allocate_tiles(dirty_tiles);

            let layer_binding = tiles.get_layer_binding_or_empty(*layer_id).unwrap();
            let pipeline = self
                .render_pipelines
                .get(&result.layer_info().texel_type)
                .unwrap();

            pipeline.dispatch_render(
                &device,
                &queue,
                &layer_binding,
                &result.binding_or_empty(),
                result.len() as u32,
                &self.disp.binding_or_empty(),
                &self.has_selection,
                &selection_binding,
            );

            services
                .service_mut::<LayerPreviewOverriders>()
                .insert_overrider(*layer_id, PixelPreviewOverrider::from_layer_storage(result));
        }

        CanvasUpdated::broadcast(CanvasUpdated {
            id: self.canvas_id,
            dirty_tiles,
        });
    }
}

#[derive(Debug, Clone)]
pub enum LiquifyToolMessage {
    RequestInit,
    ModeChanged(LiquifyMode),
    SizeChanged(f32),
    AmountChanged(f32),
    SpacingChanged(f32),
    ReverseToggled,
    Confirm,
    Cancel,
}

// TODO Undoing stroke
#[derive(Default)]
pub struct LiquifyTransformTool {
    props: LiquifyProperties,
    scan_pixels: HashMap<TexelType, ScanPixelsPipeline>,
    session: Option<LiquifySession>,
    hover_ps: Option<Vec2>,
}

impl ToolFunction for LiquifyTransformTool {
    type Message = LiquifyToolMessage;

    fn id() -> ToolId {
        ToolId::new("liquify_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::smudge()
    }

    fn activate(&mut self, _: &mut Services) -> Task<Self::Message> {
        Task::done(LiquifyToolMessage::RequestInit)
    }

    fn hover(
        &mut self,
        _: &KeyboardState,
        mouse: &HoverMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        self.hover_ps = services.current_canvas().map(|canvas| {
            canvas
                .transform
                .window_to_pixel(Vec2::new(mouse.position.x, mouse.position.y))
        });
        Task::none()
    }

    fn begin(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(session) = self.session.as_mut() else {
            return Task::none();
        };
        let Some(canvas) = services.canvas(&session.canvas_id) else {
            return Task::none();
        };
        let cursor_ps = canvas
            .transform
            .window_to_pixel(Vec2::new(mouse.position.x, mouse.position.y));

        session.last_dab = cursor_ps;
        session.stroking = true;
        Task::none()
    }

    fn update(
        &mut self,
        _: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(cursor_ps) = services.current_canvas().map(|canvas| {
            canvas
                .transform
                .window_to_pixel(Vec2::new(mouse.position.x, mouse.position.y))
        }) else {
            return Task::none();
        };
        self.hover_ps = Some(cursor_ps);

        let Some(session) = self.session.as_mut() else {
            return Task::none();
        };
        session.apply_stroke(&self.props, cursor_ps, services);
        Task::none()
    }

    fn end(
        &mut self,
        _: &KeyboardState,
        _: &PressedMouseState,
        _: &mut Services,
    ) -> Task<Self::Message> {
        if let Some(session) = self.session.as_mut() {
            session.stroking = false;
        }
        Task::none()
    }

    fn deactivate(&mut self, services: &mut Services) -> Task<Self::Message> {
        if let Some(session) = self.session.take() {
            commit_liquify(session, services);
        }
        Task::none()
    }

    fn handle_message(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        match message {
            LiquifyToolMessage::RequestInit => {
                if let Some(session) = self.session.take() {
                    commit_liquify(session, services);
                }
                self.session = init_liquify_session(&mut self.scan_pixels, services);
                Task::none()
            }
            LiquifyToolMessage::ModeChanged(mode) => {
                self.props.mode = mode;
                Task::none()
            }
            LiquifyToolMessage::SizeChanged(size) => {
                self.props.size = size;
                Task::none()
            }
            LiquifyToolMessage::AmountChanged(amount) => {
                self.props.amount = amount;
                Task::none()
            }
            LiquifyToolMessage::SpacingChanged(spacing) => {
                self.props.spacing = spacing;
                Task::none()
            }
            LiquifyToolMessage::ReverseToggled => {
                self.props.reverse = !self.props.reverse;
                Task::none()
            }
            LiquifyToolMessage::Confirm => Task::done(LiquifyToolMessage::RequestInit),
            LiquifyToolMessage::Cancel => {
                if let Some(session) = self.session.take() {
                    cancel_liquify(session, services);
                }
                Task::done(LiquifyToolMessage::RequestInit)
            }
        }
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        CanvasActiveLayerChanged::listen_to().map(|_| LiquifyToolMessage::RequestInit)
    }

    fn tool_option_widget<'a>(
        &'a self,
        _: &'a Services,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let fields = Form::new()
            .push(
                t!("mode"),
                ComboBox::new(
                    vec![
                        LiquifyMode::Move,
                        LiquifyMode::Scale,
                        LiquifyMode::Rotate,
                        LiquifyMode::Offset,
                        LiquifyMode::Undo,
                    ],
                    Some(self.props.mode),
                    LiquifyToolMessage::ModeChanged,
                )
                .width(Length::Fill),
            )
            .push(
                t!("size"),
                SpinSlider::new(1.0..=2048.0, self.props.size)
                    .step(1.0)
                    .precision(0)
                    .suffix(" px")
                    .on_confirm(LiquifyToolMessage::SizeChanged),
            )
            .push(
                t!("amount"),
                SpinSlider::new_01(self.props.amount).on_confirm(LiquifyToolMessage::AmountChanged),
            )
            .push(
                t!("spacing"),
                SpinSlider::new(0.01..=2.0, self.props.spacing)
                    .on_confirm(LiquifyToolMessage::SpacingChanged),
            )
            .push(
                t!("reverse_direction"),
                Button::new(Label::new(t!("toggle")))
                    .activated(self.props.reverse)
                    .on_press(LiquifyToolMessage::ReverseToggled)
                    .width(Length::Fill),
            );

        let actions = row![
            Button::new(Label::new(t!("cancel")))
                .on_press(LiquifyToolMessage::Cancel)
                .danger()
                .width(Length::Fill),
            Button::new(Label::new(t!("confirm")))
                .on_press(LiquifyToolMessage::Confirm)
                .primary()
                .width(Length::Fill),
        ]
        .spacing(4);

        Some(
            Panel::new(column![fields, actions].spacing(8))
                .padding(8)
                .width(Length::Fill)
                .into(),
        )
    }

    fn canvas_overlay<'a>(
        &'a self,
        services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let Some(canvas) = services.current_canvas() else {
            return space().into();
        };
        let Some(hover_ps) = self.hover_ps else {
            return space().into();
        };

        Element::new(LiquifyBrushOverlay {
            canvas_transform: &canvas.transform,
            center_ps: hover_ps,
            radius_ps: self.props.size,
        })
    }
}

fn init_liquify_session(
    scan_pixels: &mut HashMap<TexelType, ScanPixelsPipeline>,
    services: &Services,
) -> Option<LiquifySession> {
    let canvas = services.current_canvas()?;
    let canvas_id = canvas.id();
    let selection_layer_id = canvas.image.selection_layer();

    let target_layers = canvas
        .selected_layer_ids()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    if target_layers.is_empty() {
        warn!("Unable to liquify: no layer selected.");
        return None;
    }

    let device = services.render_device();
    let queue = services.render_queue();
    let tiles = services.tile_storage();

    let selection_layer = tiles.get_layer(selection_layer_id).unwrap();
    let selection_texel_type = selection_layer.layer_info().texel_type;
    let scan_pixels = scan_pixels
        .entry(selection_texel_type)
        .or_insert_with(|| ScanPixelsPipeline::new(device, selection_texel_type));
    let has_selection =
        scan_pixels.scan_to_binary_buffer(device, queue, &selection_layer.binding_or_empty());

    let mut render_pipelines = HashMap::new();
    let mut result_buffers = HashMap::new();
    for layer_id in &target_layers {
        let texel_type = tiles.get_layer_info(*layer_id).unwrap().texel_type;
        render_pipelines
            .entry(texel_type)
            .or_insert_with(|| LiquifyPipeline::new_render(device, texel_type));
        result_buffers.insert(
            *layer_id,
            DynamicLayerStorage::new(device.clone(), queue.clone(), GpuLayerInfo { texel_type }),
        );
    }

    let dab_pipeline = LiquifyPipeline::new_dab(device);
    let disp_info = GpuLayerInfo {
        texel_type: TexelType::RGBA8,
    };
    let disp = DynamicLayerStorage::new(device.clone(), queue.clone(), disp_info);
    let disp_back = DynamicLayerStorage::new(device.clone(), queue.clone(), disp_info);

    Some(LiquifySession {
        canvas_id,
        target_layers,
        selection_layer_id,
        has_selection,
        disp,
        disp_back,
        result_buffers,
        render_pipelines,
        dab_pipeline,
        last_dab: Vec2::ZERO,
        stroking: false,
    })
}

fn commit_liquify(session: LiquifySession, services: &mut Services) {
    let replace_commands = session
        .result_buffers
        .into_iter()
        .filter_map(|(layer_id, result_buffer)| {
            services
                .service_mut::<LayerPreviewOverriders>()
                .remove_overrider(&layer_id);

            let result_texture = result_buffer.texture()?;
            let target_layer = services.tile_storage().get_layer(layer_id)?;
            Some(TileReplaceCommand::new(
                "Liquify".into(),
                session.canvas_id,
                services.render_device(),
                services.render_queue(),
                layer_id,
                &target_layer,
                result_buffer.iter_tile_indices().collect(),
                result_texture.clone(),
            ))
        })
        .collect::<Vec<_>>();

    if replace_commands.is_empty() {
        return;
    }

    let cmd = BatchedUndoCommand::new("Liquify".into(), replace_commands);
    services
        .push_undo_command(&session.canvas_id, cmd)
        .log_err();
}

fn cancel_liquify(session: LiquifySession, services: &mut Services) {
    for layer_id in &session.target_layers {
        services
            .service_mut::<LayerPreviewOverriders>()
            .remove_overrider(layer_id);
    }

    let mut bounds = IRect::EMPTY;
    for result in session.result_buffers.values() {
        for tile in result.iter_tile_indices() {
            bounds = bounds.union(GpuTileStorage::tile_to_pixel_rect(tile));
        }
    }

    CanvasUpdated::broadcast(CanvasUpdated {
        id: session.canvas_id,
        dirty_tiles: GpuTileStorage::pixel_rect_to_tile(bounds),
    });
}

pub struct LiquifyBrushOverlay<'a> {
    pub canvas_transform: &'a CanvasTransform,
    pub center_ps: Vec2,
    pub radius_ps: f32,
}

impl<'a> Widget<LiquifyToolMessage, Theme, Renderer> for LiquifyBrushOverlay<'a> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut widget::Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: layout::Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let center = self.canvas_transform.pixel_to_window(self.center_ps);
        let edge = self
            .canvas_transform
            .pixel_to_window(self.center_ps + Vec2::new(self.radius_ps, 0.0));
        let radius = (edge - center).length().max(1.0);

        let mut frame = Frame::with_bounds(renderer, layout.bounds());

        for (color, translation) in [
            (Color::WHITE, Vector::ZERO),
            (Color::BLACK, Vector::new(1.0, 1.0)),
        ] {
            frame.push_transform();
            frame.translate(translation);
            frame.stroke(
                &Path::circle(Point::new(center.x, center.y), radius),
                Stroke {
                    style: color.into(),
                    width: 1.0,
                    ..Default::default()
                },
            );
            frame.pop_transform();
        }

        iced_core::Renderer::with_layer(renderer, layout.bounds(), |renderer| {
            iced_graphics::geometry::Renderer::draw_geometry(renderer, frame.into_geometry());
        });
    }

    fn mouse_interaction(
        &self,
        _tree: &widget::Tree,
        _layout: layout::Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        mouse::Interaction::Crosshair
    }
}

pub struct LiquifyPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl LiquifyPipeline {
    pub fn new_dab(device: &Device) -> Self {
        let shader = wesl_jit::compile_wesl_with_config(
            include_str!("liquify_dab.wesl").into(),
            &[&lapiz_image::image::PACKAGE],
            |_| {},
        )
        .unwrap();

        let entries = DynamicBindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                binding_types::texture_storage_2d_array(
                    TextureFormat::R32Uint,
                    StorageTextureAccess::ReadOnly,
                ),
                binding_types::texture_storage_2d_array(
                    TextureFormat::R32Uint,
                    StorageTextureAccess::WriteOnly,
                ),
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                binding_types::uniform_buffer::<LiquifyDabParams>(false),
            ),
        )
        .to_vec();

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("liquify dab bind group layout"),
            entries: entries.as_ref(),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("liquify dab pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });

        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("liquify dab shader module"),
            source: ShaderSource::Wgsl(shader.into()),
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("liquify dab pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("dab"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self { layout, pipeline }
    }

    pub fn new_render(device: &Device, format: TexelType) -> Self {
        let shader = wesl_jit::compile_wesl_with_config(
            include_str!("liquify_render.wesl").into(),
            &[&lapiz_image::image::PACKAGE],
            |compiler| {
                compiler.set_feature(format.shader_def(), true);
            },
        )
        .unwrap();

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("liquify render bind group layout"),
            entries: DynamicBindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::texture_storage_2d_array(
                        format.wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::texture_storage_2d_array(
                        format.wgpu_format(),
                        StorageTextureAccess::WriteOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::texture_storage_2d_array(
                        TextureFormat::R32Uint,
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::storage_buffer_read_only::<u32>(false),
                    binding_types::texture_storage_2d_array(
                        TextureFormat::R8Unorm,
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                ),
            )
            .as_ref(),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("liquify render pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });

        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("liquify render shader module"),
            source: ShaderSource::Wgsl(shader.into()),
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("liquify render pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("render"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self { layout, pipeline }
    }

    pub fn dispatch_dab(
        &self,
        device: &Device,
        queue: &Queue,
        params: &LiquifyDabParams,
        src: &LayerBinding,
        dst: &LayerBinding,
        dst_tile_count: u32,
    ) {
        if dst_tile_count == 0 {
            return;
        }

        let mut params_buffer = DynamicBuffer::new(
            Some("liquify_dab_params_buffer".into()),
            BufferUsages::UNIFORM,
        );
        params_buffer.push(params);
        params_buffer.write_buffer(device, queue);

        let entries = DynamicBindGroupEntries::sequential((
            &src.texture,
            &dst.texture,
            src.tile_info_buffer.as_entire_binding(),
            dst.tile_info_buffer.as_entire_binding(),
            params_buffer.binding().unwrap(),
        ));

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("liquify dab bind group"),
            layout: &self.layout,
            entries: entries.as_ref(),
        });

        let mut ec = device.create_command_encoder(&Default::default());
        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("liquify dab pass"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                dst_tile_count,
            );
        }
        queue.submit([ec.finish()]);
    }

    pub fn dispatch_render(
        &self,
        device: &Device,
        queue: &Queue,
        layer: &LayerBinding,
        output: &LayerBinding,
        output_tile_count: u32,
        disp: &LayerBinding,
        has_selection: &Buffer,
        selection: &LayerBinding,
    ) {
        if output_tile_count == 0 {
            return;
        }

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("liquify render bind group"),
            layout: &self.layout,
            entries: DynamicBindGroupEntries::sequential((
                &layer.texture,
                &output.texture,
                layer.tile_info_buffer.as_entire_binding(),
                output.tile_info_buffer.as_entire_binding(),
                &disp.texture,
                disp.tile_info_buffer.as_entire_binding(),
                has_selection.as_entire_binding(),
                &selection.texture,
                selection.tile_info_buffer.as_entire_binding(),
            ))
            .as_ref(),
        });

        let mut ec = device.create_command_encoder(&Default::default());
        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("liquify render pass"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                output_tile_count,
            );
        }
        queue.submit([ec.finish()]);
    }
}
