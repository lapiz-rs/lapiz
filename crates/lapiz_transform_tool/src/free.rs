use std::{
    collections::{HashMap, hash_map::Entry},
    fmt,
};

use anyhow::Result;
use bevy_math::{IRect, Rect};
use encase::ShaderType;
use glam::{Mat3, Vec2};
use iced_core::{
    Alignment, Color, Element, Length, Point, Rectangle, Size, Theme, Vector, Widget,
    keyboard::Modifiers, layout, mouse, renderer, widget,
};
use iced_runtime::{Task, futures::Subscription};
use iced_wgpu::Renderer;
use iced_widget::{
    canvas::{Frame, Path, Stroke},
    column, row, space, text,
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
    layer_bounds::LayerBoundsPipeline,
    texel::TexelType,
    tile::{
        DynamicLayerStorage, GpuLayerInfo, GpuTileInfo, GpuTileStorage, LayerBinding,
        TileStorageAppExt,
    },
};
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_math::rect_transform::RectTransform;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{DynamicBindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    readback::{create_readback_buffer_and_schedule_copy_buffer, readback_buffer_on_submit_async},
    render_context::RenderContextAppExt,
    util::DevicePollExt,
    wesl_jit,
};
use lapiz_runtime::{Services, event::Event};
use lapiz_tools::{ToolFunction, ToolId};
use lapiz_undo::BatchedUndoCommand;
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{
    button::Button, combo_box::ComboBox, form::Form, icon, label::Label, panel::Panel,
    spin_box::SpinBox,
};
use tracing::warn;
use wgpu::{
    BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, BufferUsages,
    ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor, Device,
    PipelineLayoutDescriptor, Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StorageTextureAccess,
};

pub struct TransformSession {
    pub canvas_id: CanvasId,
    pub target_layers: Vec<(LayerId, TexelType)>,
    pub selection_layer_id: LayerId,
    pub selection_bounds: IRect,
    pub matrix: Mat3,
    pub translate: Vec2,
    pub rotate: f32,
    pub scale: Vec2,
    pub shear: f32,
    pub last_shear: Option<ShearType>,
    pub pivot: Vec2,
    pub anchor: Vec2,
    pub tile_bounds: IRect,
    pub pixel_bounds: IRect,
    pub result_buffers: HashMap<LayerId, DynamicLayerStorage>,
    pub transform_pipelines: HashMap<TexelType, FreeTransformPipeline>,
    pub ongoing_transform: Option<OngoingTransform>,
}

impl TransformSession {
    pub fn new(init: InitTransform, services: &Services) -> Self {
        let device = services.render_device();
        let queue = services.render_queue();
        let tiles = services.tile_storage();

        let target_layers = init
            .target_layers
            .into_iter()
            .map(|layer_id| (layer_id, tiles.get_layer_info(layer_id).unwrap().texel_type))
            .collect::<Vec<_>>();

        let transform_pipelines =
            target_layers
                .iter()
                .fold(HashMap::new(), |mut acc, (layer_id, texel_type)| {
                    if let Entry::Vacant(e) = acc.entry(*texel_type) {
                        e.insert(FreeTransformPipeline::new(
                            device,
                            *texel_type,
                            !init.selection_bounds.is_empty(),
                            *layer_id == init.selection_layer_id,
                        ));
                    }

                    acc
                });

        let result_buffers = target_layers
            .iter()
            .map(|(layer_id, texel_type)| {
                (
                    *layer_id,
                    DynamicLayerStorage::new(
                        device.clone(),
                        queue.clone(),
                        GpuLayerInfo {
                            texel_type: *texel_type,
                        },
                    ),
                )
            })
            .collect();

        Self {
            canvas_id: init.canvas_id,
            target_layers,
            selection_layer_id: init.selection_layer_id,
            selection_bounds: init.selection_bounds,
            matrix: Mat3::IDENTITY,
            translate: Vec2::ZERO,
            rotate: 0.0,
            scale: Vec2::ONE,
            shear: 0.0,
            last_shear: None,
            pivot: init.pixel_bounds.as_rect().center(),
            anchor: Vec2::splat(0.5),
            tile_bounds: GpuTileStorage::pixel_rect_to_tile(init.pixel_bounds),
            pixel_bounds: init.pixel_bounds,
            result_buffers,
            transform_pipelines,
            ongoing_transform: None,
        }
    }

    pub fn update(&mut self, cursor_ps: Vec2, modifiers: Modifiers) {
        let Some(ongoing) = self.ongoing_transform else {
            return;
        };

        if matches!(ongoing.ty, InteractionType::Anchor) {
            let size = self.pixel_bounds.size().as_vec2();
            let delta = ongoing
                .base_matrix
                .inverse()
                .transform_vector2(cursor_ps - ongoing.cursor_origin_ps);
            self.anchor = ongoing.base_anchor + delta / size;
            return;
        }

        let pivot = self.op_pivot_ps(ongoing.ty, modifiers.alt());
        let origin = ongoing.base_matrix.transform_point2(Vec2::ZERO);
        let base_translate = origin - pivot + ongoing.base_matrix.transform_vector2(pivot);

        self.pivot = pivot;
        self.translate = base_translate;
        self.rotate = ongoing.base_rotate;
        self.scale = ongoing.base_scale;
        self.shear = ongoing.base_shear;
        self.last_shear = ongoing.base_last_shear;

        let delta = cursor_ps - ongoing.cursor_origin_ps;
        match ongoing.ty {
            InteractionType::Anchor => unreachable!(),
            InteractionType::Translate => {
                let d = if modifiers.shift() {
                    if delta.x.abs() > delta.y.abs() {
                        Vec2::new(delta.x, 0.0)
                    } else {
                        Vec2::new(0.0, delta.y)
                    }
                } else {
                    delta
                };
                self.translate = base_translate + d;
            }
            InteractionType::Rotate(_) => {
                let center = base_translate + pivot;
                let from = ongoing.cursor_origin_ps - center;
                let to = cursor_ps - center;
                self.rotate = ongoing.base_rotate + (to.y.atan2(to.x) - from.y.atan2(from.x));
                if modifiers.shift() {
                    let step = 15f32.to_radians();
                    self.rotate = (self.rotate / step).round() * step;
                }
            }
            InteractionType::Scale(ty) => {
                let anchor_ps = base_translate + pivot;
                let inv_rot = Mat3::from_angle(-ongoing.base_rotate);
                let u = inv_rot.transform_vector2(cursor_ps - anchor_ps);
                let w = inv_rot.transform_vector2(ongoing.cursor_origin_ps - anchor_ps)
                    / ongoing.base_scale;
                self.scale = if modifiers.shift() {
                    let from = ongoing.cursor_origin_ps - anchor_ps;
                    let to = cursor_ps - anchor_ps;
                    let s = if from.length_squared() > f32::EPSILON {
                        (to.length() / from.length()).copysign(to.dot(from))
                    } else {
                        1.0
                    };
                    ongoing.base_scale * s
                } else {
                    let sx = if w.x.abs() > f32::EPSILON {
                        u.x / w.x
                    } else {
                        ongoing.base_scale.x
                    };
                    let sy = if w.y.abs() > f32::EPSILON {
                        u.y / w.y
                    } else {
                        ongoing.base_scale.y
                    };
                    match ty {
                        ScaleType::Left | ScaleType::Right => Vec2::new(sx, ongoing.base_scale.y),
                        ScaleType::Top | ScaleType::Bottom => Vec2::new(ongoing.base_scale.x, sy),
                        _ => Vec2::new(sx, sy),
                    }
                };
                const MIN_SCALE: f32 = 0.001;
                if self.scale.x.abs() <= MIN_SCALE {
                    self.scale.x = MIN_SCALE.copysign(self.scale.x);
                }
                if self.scale.y.abs() <= MIN_SCALE {
                    self.scale.y = MIN_SCALE.copysign(self.scale.y);
                }
            }
            InteractionType::Shear(ty) => {
                let b = self.pixel_bounds;
                let proj = Mat3::from_angle(-ongoing.base_rotate).transform_vector2(delta);
                let (extent, projection) = match ty {
                    ShearType::Top => (ongoing.base_scale.x * (pivot.y - b.min.y as f32), proj.x),
                    ShearType::Bottom => {
                        (ongoing.base_scale.x * (b.max.y as f32 - pivot.y), proj.x)
                    }
                    ShearType::Left => (ongoing.base_scale.y * (pivot.x - b.min.x as f32), proj.y),
                    ShearType::Right => (ongoing.base_scale.y * (b.max.x as f32 - pivot.x), proj.y),
                };
                let sign = match ty {
                    ShearType::Top | ShearType::Left => -1.0,
                    ShearType::Bottom | ShearType::Right => 1.0,
                };
                let extent = if extent >= 0.0 {
                    extent.max(1.0)
                } else {
                    extent.min(-1.0)
                };
                self.shear = ongoing.base_shear + sign * projection / extent;
                self.last_shear = Some(ty);
            }
        }
        self.update_matrix();
    }

    pub fn transformed_aabb_ps(&self) -> Rect {
        self.pixel_bounds.as_rect().transformed(&self.matrix)
    }

    pub fn quad_ps(&self) -> [Vec2; 4] {
        let b = self.pixel_bounds.as_rect();
        [
            self.matrix.transform_point2(b.min),
            self.matrix.transform_point2(Vec2::new(b.max.x, b.min.y)),
            self.matrix.transform_point2(b.max),
            self.matrix.transform_point2(Vec2::new(b.min.x, b.max.y)),
        ]
    }

    pub fn update_matrix(&mut self) {
        let pivot = self.pivot;
        let mut m = Mat3::from_translation(self.translate);
        m *= Mat3::from_translation(pivot);
        m *= Mat3::from_angle(self.rotate);
        m *= Mat3::from_scale(self.scale);
        if let Some(shear) = self.active_shear() {
            m *= shear;
        }
        m *= Mat3::from_translation(-pivot);
        self.matrix = m;
    }

    fn pixel_bounds_center_ps(&self) -> Vec2 {
        let b = self.pixel_bounds;
        Vec2::new(
            (b.min.x + b.max.x) as f32 * 0.5,
            (b.min.y + b.max.y) as f32 * 0.5,
        )
    }

    fn scale_anchor_ps(&self, ty: ScaleType) -> Vec2 {
        let b = self.pixel_bounds;
        let min = b.min.as_vec2();
        let max = b.max.as_vec2();
        let cx = (b.min.x + b.max.x) as f32 * 0.5;
        let cy = (b.min.y + b.max.y) as f32 * 0.5;
        match ty {
            ScaleType::Left => Vec2::new(max.x, cy),
            ScaleType::Right => Vec2::new(min.x, cy),
            ScaleType::Top => Vec2::new(cx, max.y),
            ScaleType::Bottom => Vec2::new(cx, min.y),
            ScaleType::TopLeft => max,
            ScaleType::TopRight => Vec2::new(min.x, max.y),
            ScaleType::BottomLeft => Vec2::new(max.x, min.y),
            ScaleType::BottomRight => min,
        }
    }

    fn op_pivot_ps(&self, ty: InteractionType, symmetric: bool) -> Vec2 {
        let b = self.pixel_bounds;
        let anchor = b.min.as_vec2() + self.anchor * b.size().as_vec2();
        match ty {
            InteractionType::Anchor | InteractionType::Rotate(_) => anchor,
            InteractionType::Translate => self.pixel_bounds_center_ps(),
            InteractionType::Scale(ty) => {
                if symmetric {
                    anchor
                } else {
                    self.scale_anchor_ps(ty)
                }
            }
            InteractionType::Shear(ty) => {
                if symmetric {
                    anchor
                } else {
                    self.shear_pivot_ps(ty)
                }
            }
        }
    }

    fn shear_pivot_ps(&self, ty: ShearType) -> Vec2 {
        let b = self.pixel_bounds;
        let cx = (b.min.x + b.max.x) as f32 * 0.5;
        let cy = (b.min.y + b.max.y) as f32 * 0.5;
        match ty {
            ShearType::Left => Vec2::new(b.max.x as f32, cy),
            ShearType::Right => Vec2::new(b.min.x as f32, cy),
            ShearType::Top => Vec2::new(cx, b.max.y as f32),
            ShearType::Bottom => Vec2::new(cx, b.min.y as f32),
        }
    }

    fn reorient_shear(&mut self, ty: ShearType) {
        let Some(last_shear) = self.last_shear else {
            return;
        };
        let vertical = matches!(ty, ShearType::Left | ShearType::Right);
        if vertical == matches!(last_shear, ShearType::Left | ShearType::Right) {
            return;
        }

        let x = self.matrix.transform_vector2(Vec2::X);
        let y = self.matrix.transform_vector2(Vec2::Y);
        if vertical {
            let sy = y.length();
            let rotate = (-y.x).atan2(y.y);
            let local_x = Mat3::from_angle(-rotate).transform_vector2(x);
            self.rotate = rotate;
            self.scale = Vec2::new(local_x.x, sy);
            self.shear = local_x.y / sy;
        } else {
            let sx = x.length();
            let rotate = x.y.atan2(x.x);
            let local_y = Mat3::from_angle(-rotate).transform_vector2(y);
            self.rotate = rotate;
            self.scale = Vec2::new(sx, local_y.y);
            self.shear = local_y.x / sx;
        }
        self.last_shear = Some(ty);
        self.update_matrix();
    }

    fn active_shear(&self) -> Option<Mat3> {
        match self.last_shear {
            Some(ShearType::Left | ShearType::Right) => Some(Mat3::from_cols_array(&[
                1.0, self.shear, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0,
            ])),
            Some(ShearType::Top | ShearType::Bottom) => Some(Mat3::from_cols_array(&[
                1.0, 0.0, 0.0, self.shear, 1.0, 0.0, 0.0, 0.0, 1.0,
            ])),
            None => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OngoingTransform {
    pub cursor_origin_ps: Vec2,
    pub ty: InteractionType,
    pub base_matrix: Mat3,
    pub base_rotate: f32,
    pub base_scale: Vec2,
    pub base_shear: f32,
    pub base_last_shear: Option<ShearType>,
    pub base_anchor: Vec2,
}

pub fn hit_test(quad: [Vec2; 4], p: Vec2, anchor: Vec2) -> Option<InteractionType> {
    const ANCHOR_HIT_RADIUS: f32 = 20.0;
    const EDGE_HIT_RADIUS: f32 = 10.0;
    const SHEAR_HIT_MAX_DISTANCE: f32 = 30.0;
    const ROTATE_HIT_RADIUS: f32 = 40.0;

    let [tl, tr, br, bl] = quad;
    let x = tr - tl;
    let y = bl - tl;
    let det = x.x * y.y - x.y * y.x;
    if det.abs() < f32::EPSILON {
        return None;
    }

    let anchor = tl + anchor.x * x + anchor.y * y;
    if anchor.distance(p) < ANCHOR_HIT_RADIUS {
        return Some(InteractionType::Anchor);
    }

    let d = p - tl;
    let u = (d.x * y.y - d.y * y.x) / det;
    let v = (x.x * d.y - x.y * d.x) / det;

    let w_abs = x.length();
    let h_abs = y.length();
    let area = det.abs();

    let left_distance = u * area / h_abs;
    let right_distance = (1.0 - u) * area / h_abs;
    let top_distance = v * area / w_abs;
    let bottom_distance = (1.0 - v) * area / w_abs;

    let left_edge_t = (p - tl).dot(y) / y.length_squared();
    let right_edge_t = (p - tr).dot(y) / y.length_squared();
    let top_edge_t = (p - tl).dot(x) / x.length_squared();
    let bottom_edge_t = (p - bl).dot(x) / x.length_squared();

    if left_distance > EDGE_HIT_RADIUS
        && right_distance > EDGE_HIT_RADIUS
        && top_distance > EDGE_HIT_RADIUS
        && bottom_distance > EDGE_HIT_RADIUS
    {
        return Some(InteractionType::Translate);
    }

    if p.distance(tl) < EDGE_HIT_RADIUS {
        return Some(InteractionType::Scale(ScaleType::TopLeft));
    }
    if p.distance(br) < EDGE_HIT_RADIUS {
        return Some(InteractionType::Scale(ScaleType::BottomRight));
    }
    if p.distance(tr) < EDGE_HIT_RADIUS {
        return Some(InteractionType::Scale(ScaleType::TopRight));
    }
    if p.distance(bl) < EDGE_HIT_RADIUS {
        return Some(InteractionType::Scale(ScaleType::BottomLeft));
    }

    if 0.0 < left_edge_t && left_edge_t < 1.0 && left_distance.abs() < EDGE_HIT_RADIUS {
        return Some(InteractionType::Scale(ScaleType::Left));
    }
    if 0.0 < right_edge_t && right_edge_t < 1.0 && right_distance.abs() < EDGE_HIT_RADIUS {
        return Some(InteractionType::Scale(ScaleType::Right));
    }
    if 0.0 < top_edge_t && top_edge_t < 1.0 && top_distance.abs() < EDGE_HIT_RADIUS {
        return Some(InteractionType::Scale(ScaleType::Top));
    }
    if 0.0 < bottom_edge_t && bottom_edge_t < 1.0 && bottom_distance.abs() < EDGE_HIT_RADIUS {
        return Some(InteractionType::Scale(ScaleType::Bottom));
    }

    if ((left_edge_t - 0.5) * h_abs).abs() < h_abs.min(EDGE_HIT_RADIUS)
        && (-EDGE_HIT_RADIUS..-SHEAR_HIT_MAX_DISTANCE).contains(&left_distance)
    {
        return Some(InteractionType::Shear(ShearType::Left));
    }
    if ((right_edge_t - 0.5) * h_abs).abs() < h_abs.min(EDGE_HIT_RADIUS)
        && (-EDGE_HIT_RADIUS..-SHEAR_HIT_MAX_DISTANCE).contains(&right_distance)
    {
        return Some(InteractionType::Shear(ShearType::Right));
    }
    if ((top_edge_t - 0.5) * w_abs).abs() < w_abs.min(EDGE_HIT_RADIUS)
        && (-EDGE_HIT_RADIUS..-SHEAR_HIT_MAX_DISTANCE).contains(&top_distance)
    {
        return Some(InteractionType::Shear(ShearType::Top));
    }
    if ((bottom_edge_t - 0.5) * w_abs).abs() < w_abs.min(EDGE_HIT_RADIUS)
        && (-EDGE_HIT_RADIUS..-SHEAR_HIT_MAX_DISTANCE).contains(&bottom_distance)
    {
        return Some(InteractionType::Shear(ShearType::Bottom));
    }

    if u < 0.0 && v < 0.0 && p.distance(tl) < ROTATE_HIT_RADIUS {
        return Some(InteractionType::Rotate(RotateType::TopLeft));
    }
    if u > 1.0 && v < 0.0 && p.distance(tr) < ROTATE_HIT_RADIUS {
        return Some(InteractionType::Rotate(RotateType::TopRight));
    }
    if u > 1.0 && v > 1.0 && p.distance(br) < ROTATE_HIT_RADIUS {
        return Some(InteractionType::Rotate(RotateType::BottomRight));
    }
    if u < 0.0 && v > 1.0 && p.distance(bl) < ROTATE_HIT_RADIUS {
        return Some(InteractionType::Rotate(RotateType::BottomLeft));
    }

    None
}

#[derive(Debug, Clone, Copy)]
pub enum InteractionType {
    Anchor,
    Translate,
    Rotate(RotateType),
    Scale(ScaleType),
    Shear(ShearType),
    // TODO rotate transform bounds
}

#[derive(Debug, Clone, Copy)]
pub enum RotateType {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy)]
pub enum ScaleType {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy)]
pub enum ShearType {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone)]
pub struct InitTransform {
    pub canvas_id: CanvasId,
    pub target_layers: Vec<LayerId>,
    pub selection_layer_id: LayerId,
    pub selection_bounds: IRect,
    pub pixel_bounds: IRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingMethod {
    NearestNeighbor,
}

impl fmt::Display for SamplingMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Nearest Neighbor")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShearAxis {
    Horizontal,
    Vertical,
}

impl fmt::Display for ShearAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Horizontal => f.write_str("Horizontal"),
            Self::Vertical => f.write_str("Vertical"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FreeTransformToolMessage {
    RequestInit,
    InitTransform(InitTransform),
    SamplingChanged(SamplingMethod),
    TranslationXChanged(f32),
    TranslationYChanged(f32),
    RotationChanged(f32),
    ScaleXChanged(f32),
    ScaleYChanged(f32),
    ShearChanged(f32),
    ShearAxisChanged(ShearAxis),
    MirrorHorizontally,
    MirrorVertically,
    Cancel,
    Confirm,
}

#[derive(Default)]
pub struct FreeTransformTool {
    session: Option<TransformSession>,
    bounds_pipelines: HashMap<TexelType, LayerBoundsPipeline>,
}

impl ToolFunction for FreeTransformTool {
    type Message = FreeTransformToolMessage;

    fn id() -> ToolId {
        ToolId::new("free_transform_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::transform()
    }

    fn activate(&mut self, _services: &mut Services) -> Task<Self::Message> {
        Task::done(FreeTransformToolMessage::RequestInit)
    }

    fn begin(
        &mut self,
        _keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(session) = &mut self.session else {
            return Task::none();
        };
        let Some(canvas) = services.canvas(&session.canvas_id) else {
            return Task::none();
        };
        let cursor_ps = canvas
            .transform
            .window_to_pixel(Vec2::new(mouse.position.x, mouse.position.y));
        let Some(ty) = hit_test(session.quad_ps(), cursor_ps, session.anchor) else {
            return Task::none();
        };

        if let InteractionType::Shear(shear) = ty {
            session.reorient_shear(shear);
        }
        session.ongoing_transform = Some(OngoingTransform {
            cursor_origin_ps: cursor_ps,
            ty,
            base_matrix: session.matrix,
            base_rotate: session.rotate,
            base_scale: session.scale,
            base_shear: session.shear,
            base_last_shear: session.last_shear,
            base_anchor: session.anchor,
        });

        Task::none()
    }

    fn update(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(session) = &mut self.session else {
            return Task::none();
        };
        let Some(canvas) = services.canvas(&session.canvas_id) else {
            return Task::none();
        };
        let cursor_ps = canvas
            .transform
            .window_to_pixel(Vec2::new(mouse.position.x, mouse.position.y));

        if session.ongoing_transform.is_none() {
            return Task::none();
        }

        session.update(cursor_ps, keyboard.modifiers());
        render_transform_preview(session, services);

        Task::none()
    }

    fn end(
        &mut self,
        _keyboard: &KeyboardState,
        _mouse: &PressedMouseState,
        _services: &mut Services,
    ) -> Task<Self::Message> {
        let Some(session) = &mut self.session else {
            return Task::none();
        };

        session.ongoing_transform = None;
        Task::none()
    }

    fn handle_message(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        match message {
            FreeTransformToolMessage::RequestInit => {
                if let Some(session) = self.session.take() {
                    commit_transform(session, services);
                }

                let Some(canvas) = services.current_canvas() else {
                    return Task::none();
                };

                let device = services.render_device();
                let queue = services.render_queue();

                let canvas_id = canvas.id();
                let tiles = services.tile_storage();

                let selection_layer_id = canvas.image.selection_layer();
                let selection_layer = tiles.get_layer(selection_layer_id).unwrap();
                let selection_layer_binding = selection_layer.binding_or_empty();
                let selection_layer_texel = selection_layer.layer_info().texel_type;

                let selection_layer_bounds_pipeline = self
                    .bounds_pipelines
                    .entry(selection_layer_texel)
                    .or_insert_with(|| {
                        LayerBoundsPipeline::new(device, selection_layer_texel, false)
                    });
                let mut ec = device.create_command_encoder(&Default::default());
                let selection_layer_bounds_buffer = selection_layer_bounds_pipeline.dispatch(
                    device,
                    queue,
                    &mut ec,
                    &selection_layer_binding,
                    None,
                );
                let selection_layer_bounds_staging =
                    create_readback_buffer_and_schedule_copy_buffer(
                        device,
                        &mut ec,
                        &selection_layer_bounds_buffer,
                    );
                let selection_layer_bounds_readback = readback_buffer_on_submit_async::<IRect, _>(
                    &mut ec,
                    &selection_layer_bounds_staging,
                    ..,
                );
                let si = queue.submit([ec.finish()]);
                device.poll_indefinitely_for(si).unwrap();
                let selection_layer_bounds = selection_layer_bounds_readback.block_on().unwrap();

                let target_layers = canvas
                    .selected_layer_ids()
                    .iter()
                    .copied()
                    .chain([canvas.image.selection_layer()])
                    .collect::<Vec<_>>();

                if !selection_layer_bounds.is_empty() {
                    let init = InitTransform {
                        canvas_id,
                        target_layers,
                        selection_layer_id,
                        selection_bounds: selection_layer_bounds,
                        pixel_bounds: selection_layer_bounds,
                    };
                    return Task::done(FreeTransformToolMessage::InitTransform(init));
                }

                let mut ec = device.create_command_encoder(&Default::default());
                let mut readback_tasks = Vec::with_capacity(target_layers.len());

                for target_layer_id in &target_layers {
                    let target_layer = tiles.get_layer(*target_layer_id).unwrap();
                    let target_layer_binding = target_layer.binding_or_empty();
                    let target_layer_texel = target_layer.layer_info().texel_type;

                    let bounds_pipeline = self
                        .bounds_pipelines
                        .entry(target_layer_texel)
                        .or_insert_with(|| {
                            LayerBoundsPipeline::new(device, target_layer_texel, false)
                        });

                    let bounds_buffer = bounds_pipeline.dispatch(
                        device,
                        queue,
                        &mut ec,
                        &target_layer_binding,
                        None,
                    );

                    let bounds_buffer_staging = create_readback_buffer_and_schedule_copy_buffer(
                        device,
                        &mut ec,
                        &bounds_buffer,
                    );
                    let bounds_buffer_readback = readback_buffer_on_submit_async::<IRect, _>(
                        &mut ec,
                        &bounds_buffer_staging,
                        ..,
                    )
                    .into_task();

                    readback_tasks.push(bounds_buffer_readback);
                }

                let si = queue.submit([ec.finish()]);

                let poll_task = Task::future({
                    let device = device.clone();
                    async move {
                        let _ = device.poll_indefinitely_for(si);
                    }
                });

                let init_task = Task::batch(readback_tasks).collect().then(move |bounds| {
                    let bounds = bounds
                        .into_iter()
                        .try_fold(IRect::EMPTY, |acc, b| Result::<IRect>::Ok(acc.union(b?)))
                        .logged_err()
                        .unwrap_or(IRect::EMPTY);

                    Task::done(FreeTransformToolMessage::InitTransform(InitTransform {
                        canvas_id,
                        selection_layer_id,
                        target_layers: target_layers.clone(),
                        selection_bounds: selection_layer_bounds,
                        pixel_bounds: bounds,
                    }))
                });

                Task::batch([init_task, poll_task.discard()])
            }
            FreeTransformToolMessage::InitTransform(init) => {
                if init.pixel_bounds.is_empty() {
                    warn!("Unable to transform on empty layer.");
                    self.session = None;
                } else {
                    self.session = Some(TransformSession::new(init, services));
                }
                Task::none()
            }
            FreeTransformToolMessage::SamplingChanged(_) => Task::none(),
            FreeTransformToolMessage::Confirm => Task::done(FreeTransformToolMessage::RequestInit),
            FreeTransformToolMessage::Cancel => {
                if let Some(session) = self.session.take() {
                    for (layer_id, _) in &session.target_layers {
                        services
                            .service_mut::<LayerPreviewOverriders>()
                            .remove_overrider(layer_id);
                    }
                    CanvasUpdated::broadcast(CanvasUpdated {
                        id: session.canvas_id,
                        dirty_tiles: GpuTileStorage::pixel_rect_to_tile(
                            session.transformed_aabb_ps().as_irect(),
                        )
                        .union(session.tile_bounds),
                    });
                }
                Task::done(FreeTransformToolMessage::RequestInit)
            }
            message => {
                let Some(session) = &mut self.session else {
                    return Task::none();
                };
                session.ongoing_transform = None;

                if matches!(
                    &message,
                    FreeTransformToolMessage::RotationChanged(_)
                        | FreeTransformToolMessage::ScaleXChanged(_)
                        | FreeTransformToolMessage::ScaleYChanged(_)
                        | FreeTransformToolMessage::ShearChanged(_)
                        | FreeTransformToolMessage::MirrorHorizontally
                        | FreeTransformToolMessage::MirrorVertically
                ) {
                    let b = session.pixel_bounds;
                    let pivot = b.min.as_vec2() + session.anchor * b.size().as_vec2();
                    let origin = session.matrix.transform_point2(Vec2::ZERO);
                    session.translate = origin - pivot + session.matrix.transform_vector2(pivot);
                    session.pivot = pivot;
                }

                match message {
                    FreeTransformToolMessage::TranslationXChanged(value) => {
                        session.translate.x = value;
                    }
                    FreeTransformToolMessage::TranslationYChanged(value) => {
                        session.translate.y = value;
                    }
                    FreeTransformToolMessage::RotationChanged(value) => {
                        session.rotate = value;
                    }
                    FreeTransformToolMessage::ScaleXChanged(value) => {
                        session.scale.x = if value.abs() <= 0.001 {
                            (0.001 + f32::EPSILON).copysign(value)
                        } else {
                            value
                        };
                    }
                    FreeTransformToolMessage::ScaleYChanged(value) => {
                        session.scale.y = if value.abs() <= 0.001 {
                            (0.001 + f32::EPSILON).copysign(value)
                        } else {
                            value
                        };
                    }
                    FreeTransformToolMessage::ShearChanged(value) => {
                        session.shear = value;
                        session.last_shear.get_or_insert(ShearType::Top);
                    }
                    FreeTransformToolMessage::ShearAxisChanged(axis) => {
                        let shear = match axis {
                            ShearAxis::Horizontal => ShearType::Top,
                            ShearAxis::Vertical => ShearType::Left,
                        };
                        session.reorient_shear(shear);
                        session.last_shear = Some(shear);
                    }
                    FreeTransformToolMessage::MirrorHorizontally
                    | FreeTransformToolMessage::MirrorVertically => match message {
                        FreeTransformToolMessage::MirrorHorizontally => {
                            session.scale.x = -session.scale.x;
                        }
                        FreeTransformToolMessage::MirrorVertically => {
                            session.scale.y = -session.scale.y;
                        }
                        _ => unreachable!(),
                    },
                    _ => unreachable!(),
                }

                session.update_matrix();
                render_transform_preview(session, services);
                Task::none()
            }
        }
    }

    fn deactivate(&mut self, services: &mut Services) -> Task<Self::Message> {
        if let Some(session) = self.session.take() {
            commit_transform(session, services);
        }

        Task::none()
    }

    fn canvas_overlay<'a>(
        &'a self,
        services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let Some(session) = &self.session else {
            return space().into();
        };
        let Some(canvas) = services.current_canvas() else {
            return space().into();
        };

        Element::new(FreeTransformToolOverlay {
            canvas_transform: &canvas.transform,
            session,
        })
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        CanvasActiveLayerChanged::listen_to().map(|_| FreeTransformToolMessage::RequestInit)
    }

    fn tool_option_widget<'a>(
        &'a self,
        _services: &'a Services,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let session = self.session.as_ref()?;
        let shear_axis = match session.last_shear {
            Some(ShearType::Left | ShearType::Right) => ShearAxis::Vertical,
            _ => ShearAxis::Horizontal,
        };

        let fields = Form::new()
            .push(
                t!("sampling"),
                ComboBox::new(
                    vec![SamplingMethod::NearestNeighbor],
                    Some(SamplingMethod::NearestNeighbor),
                    FreeTransformToolMessage::SamplingChanged,
                )
                .width(Length::Fill),
            )
            .push(
                t!("translation"),
                row![
                    text(t!("x")),
                    SpinBox::new(
                        &session.translate.x,
                        f32::MIN..=f32::MAX,
                        FreeTransformToolMessage::TranslationXChanged,
                    )
                    .step(1.0)
                    .width(Length::FillPortion(1)),
                    text(t!("y")),
                    SpinBox::new(
                        &session.translate.y,
                        f32::MIN..=f32::MAX,
                        FreeTransformToolMessage::TranslationYChanged,
                    )
                    .step(1.0)
                    .width(Length::FillPortion(1)),
                ]
                .align_y(Alignment::Center)
                .spacing(4),
            )
            .push(
                t!("rotation"),
                SpinBox::new(
                    &session.rotate,
                    f32::MIN..=f32::MAX,
                    FreeTransformToolMessage::RotationChanged,
                )
                .step(0.01)
                .width(Length::Fill),
            )
            .push(
                t!("scale"),
                row![
                    text(t!("x")),
                    SpinBox::new(
                        &session.scale.x,
                        f32::MIN..=f32::MAX,
                        FreeTransformToolMessage::ScaleXChanged,
                    )
                    .step(0.01)
                    .width(Length::FillPortion(1)),
                    text(t!("y")),
                    SpinBox::new(
                        &session.scale.y,
                        f32::MIN..=f32::MAX,
                        FreeTransformToolMessage::ScaleYChanged,
                    )
                    .step(0.01)
                    .width(Length::FillPortion(1)),
                ]
                .align_y(Alignment::Center)
                .spacing(4),
            )
            .push(
                t!("shear"),
                SpinBox::new(
                    &session.shear,
                    f32::MIN..=f32::MAX,
                    FreeTransformToolMessage::ShearChanged,
                )
                .step(0.01)
                .width(Length::Fill),
            )
            .push(
                t!("shear_direction"),
                ComboBox::new(
                    vec![ShearAxis::Horizontal, ShearAxis::Vertical],
                    Some(shear_axis),
                    FreeTransformToolMessage::ShearAxisChanged,
                )
                .width(Length::Fill),
            );

        let mirrors = row![
            Button::new(Label::new(t!("mirror_horizontally")))
                .on_press(FreeTransformToolMessage::MirrorHorizontally)
                .width(Length::Fill),
            Button::new(Label::new(t!("mirror_vertically")))
                .on_press(FreeTransformToolMessage::MirrorVertically)
                .width(Length::Fill),
        ]
        .spacing(4);
        let actions = row![
            Button::new(Label::new(t!("cancel")))
                .on_press(FreeTransformToolMessage::Cancel)
                .danger()
                .width(Length::Fill),
            Button::new(Label::new(t!("confirm")))
                .on_press(FreeTransformToolMessage::Confirm)
                .primary()
                .width(Length::Fill),
        ]
        .spacing(4);

        Some(
            Panel::new(column![fields, mirrors, actions].spacing(8))
                .padding(8)
                .width(Length::Fill)
                .into(),
        )
    }
}

fn render_transform_preview(session: &mut TransformSession, services: &mut Services) {
    let transformed_tiles =
        GpuTileStorage::pixel_rect_to_tile(session.transformed_aabb_ps().as_irect());

    for (layer_id, result_buffer) in &mut session.result_buffers {
        result_buffer.allocate_tiles(session.tile_bounds);
        result_buffer.allocate_tiles(transformed_tiles);

        let device = services.render_device();
        let queue = services.render_queue();
        let tiles = services.tile_storage();
        let target_layer = tiles.get_layer_binding_or_empty(*layer_id).unwrap();
        let selection_layer = (!session.selection_bounds.is_empty()).then(|| {
            tiles
                .get_layer_binding_or_empty(session.selection_layer_id)
                .unwrap()
        });

        let transform_pipeline = session
            .transform_pipelines
            .get(&result_buffer.layer_info().texel_type)
            .unwrap();
        transform_pipeline.dispatch(
            device,
            queue,
            &FreeTransformParams {
                mat_inv: session.matrix.inverse(),
            },
            target_layer,
            result_buffer.binding_or_empty(),
            selection_layer,
        );

        services
            .service_mut::<LayerPreviewOverriders>()
            .insert_overrider(
                *layer_id,
                PixelPreviewOverrider::from_layer_storage(result_buffer),
            );
    }

    CanvasUpdated::broadcast(CanvasUpdated {
        id: session.canvas_id,
        dirty_tiles: transformed_tiles.union(session.tile_bounds),
    });
}

fn commit_transform(session: TransformSession, services: &mut Services) {
    let replace_commands = session
        .result_buffers
        .into_iter()
        .filter_map(|(layer_id, result_buffer)| {
            let result_texture = result_buffer.texture()?;

            services
                .service_mut::<LayerPreviewOverriders>()
                .remove_overrider(&layer_id);

            let tiles = services.tile_storage();
            let target_layer = tiles.get_layer(layer_id).unwrap();
            let cmd = TileReplaceCommand::new(
                "Free Transform".into(),
                session.canvas_id,
                services.render_device(),
                services.render_queue(),
                layer_id,
                &target_layer,
                result_buffer.iter_tile_indices().collect(),
                result_texture.clone(),
            );

            Some(cmd)
        })
        .collect();

    let cmd = BatchedUndoCommand::new("Free Transform".into(), replace_commands);
    services
        .push_undo_command(&session.canvas_id, cmd)
        .log_err();
}

pub struct FreeTransformToolOverlay<'a> {
    pub canvas_transform: &'a CanvasTransform,
    pub session: &'a TransformSession,
}

impl<'a> Widget<FreeTransformToolMessage, Theme, Renderer> for FreeTransformToolOverlay<'a> {
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
        let corners = self
            .session
            .quad_ps()
            .map(|p| self.canvas_transform.pixel_to_window(p));
        let [tl, tr, br, bl] = corners;
        let x = tr - tl;
        let y = bl - tl;

        let mut frame = Frame::with_bounds(renderer, layout.bounds());

        for (color, translation) in [
            (Color::WHITE, Vector::ZERO),
            (Color::BLACK, Vector::new(1.0, 1.0)),
        ] {
            frame.push_transform();
            frame.translate(translation);

            const HANDLE_SIZE: f32 = 10.0;
            for point in [tl, tr, bl, br] {
                let origin = Point::new(point.x - HANDLE_SIZE * 0.5, point.y - HANDLE_SIZE * 0.5);
                frame.stroke_rectangle(
                    origin,
                    Size::new(HANDLE_SIZE, HANDLE_SIZE),
                    Stroke {
                        style: color.into(),
                        width: 1.0,
                        ..Default::default()
                    },
                );
            }

            let path = Path::new(|b| {
                let hx = x.normalize() * HANDLE_SIZE;
                let hy = y.normalize() * HANDLE_SIZE;

                for seg in [
                    (tl + hx, tr - hx),
                    (tr + hy, br - hy),
                    (br - hx, bl + hx),
                    (bl - hy, tl + hy),
                ] {
                    let (p1, p2) = seg;
                    b.move_to(Point::new(p1.x, p1.y));
                    b.line_to(Point::new(p2.x, p2.y));
                }
            });
            frame.stroke(
                &path,
                Stroke {
                    style: color.into(),
                    width: 1.0,
                    ..Default::default()
                },
            );

            const ANCHOR_SIZE: f32 = 20.0;
            let anchor = self.session.anchor.x * x + self.session.anchor.y * y + tl;
            let anchor_mark = Path::new(|b| {
                let h = ANCHOR_SIZE * 0.5;
                b.move_to(Point::new(anchor.x - h, anchor.y));
                b.line_to(Point::new(anchor.x + h, anchor.y));
                b.move_to(Point::new(anchor.x, anchor.y - h));
                b.line_to(Point::new(anchor.x, anchor.y + h));
            });
            let stroke = Stroke {
                style: color.into(),
                width: 1.0,
                ..Default::default()
            };
            frame.stroke(
                &Path::circle(Point::new(anchor.x, anchor.y), ANCHOR_SIZE * 0.5 * 0.7),
                stroke,
            );
            frame.stroke(&anchor_mark, stroke);

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
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let Some(cursor_ps) = cursor
            .position()
            .map(|p| self.canvas_transform.window_to_pixel(Vec2::new(p.x, p.y)))
        else {
            return mouse::Interaction::None;
        };

        let Some(ty) = self
            .session
            .ongoing_transform
            .as_ref()
            .map(|t| t.ty)
            .or_else(|| hit_test(self.session.quad_ps(), cursor_ps, self.session.anchor))
        else {
            return mouse::Interaction::None;
        };

        match ty {
            InteractionType::Anchor => mouse::Interaction::Pointer,
            InteractionType::Translate => mouse::Interaction::Move,
            // TODO
            InteractionType::Rotate(_ty) => mouse::Interaction::None,
            InteractionType::Scale(ty) => match ty {
                ScaleType::Left | ScaleType::Right => mouse::Interaction::ResizingColumn,
                ScaleType::Top | ScaleType::Bottom => mouse::Interaction::ResizingRow,
                ScaleType::TopLeft | ScaleType::BottomRight => {
                    mouse::Interaction::ResizingDiagonallyDown
                }
                ScaleType::TopRight | ScaleType::BottomLeft => {
                    mouse::Interaction::ResizingDiagonallyUp
                }
            },
            // TODO
            InteractionType::Shear(_ty) => mouse::Interaction::None,
        }
    }
}

pub struct FreeTransformPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
    with_selection: bool,
}

impl FreeTransformPipeline {
    pub fn new(
        device: &Device,
        format: TexelType,
        with_selection: bool,
        is_target_selection: bool,
    ) -> Self {
        let shader = wesl_jit::compile_wesl_with_config(
            include_str!("free.wesl").into(),
            &[&lapiz_image::image::PACKAGE],
            |compiler| {
                compiler.set_feature(format.shader_def(), true);
                compiler.set_feature("WITH_SELECTION", with_selection);
            },
        )
        .unwrap();

        let mut entries = DynamicBindGroupLayoutEntries::sequential(
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
                binding_types::storage_buffer_read_only::<FreeTransformParams>(false),
            ),
        );

        if with_selection {
            entries = entries.extend_sequential((
                binding_types::texture_storage_2d_array(
                    wgpu::TextureFormat::R8Unorm,
                    StorageTextureAccess::ReadOnly,
                ),
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
            ));
        }

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("free transform bind group layout"),
            entries: entries.as_ref(),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("free transform pipeline layout"),
            bind_group_layouts: &[&layout],
            ..Default::default()
        });

        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("free transform shader module"),
            source: ShaderSource::Wgsl(shader.into()),
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("free transform pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some(if is_target_selection {
                "main_selection"
            } else {
                "main"
            }),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            layout,
            pipeline,
            with_selection,
        }
    }

    pub fn dispatch(
        &self,
        device: &Device,
        queue: &Queue,
        params: &FreeTransformParams,
        layer: LayerBinding,
        output: LayerBinding,
        selection: Option<LayerBinding>,
    ) {
        let mut params_buffer = DynamicBuffer::new(
            Some("free_transform_params_buffer".into()),
            BufferUsages::STORAGE,
        );
        params_buffer.push(params);
        params_buffer.write_buffer(device, queue);

        let mut entries = DynamicBindGroupEntries::sequential((
            &layer.texture,
            &output.texture,
            layer.tile_info_buffer.as_entire_binding(),
            output.tile_info_buffer.as_entire_binding(),
            params_buffer.binding().unwrap(),
        ));

        if self.with_selection {
            let selection = selection
                .as_ref()
                .expect("selection is required when with_selection is true");
            entries = entries.extend_sequential((
                &selection.texture,
                selection.tile_info_buffer.as_entire_binding(),
            ));
        }

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("free transform bind group"),
            layout: &self.layout,
            entries: entries.as_ref(),
        });

        let mut ec = device.create_command_encoder(&Default::default());
        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("free transform pass"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                output.texture.texture().depth_or_array_layers(),
            );
        }
        queue.submit([ec.finish()]);
    }
}

#[derive(ShaderType, Debug)]
pub struct FreeTransformParams {
    pub mat_inv: Mat3,
}
