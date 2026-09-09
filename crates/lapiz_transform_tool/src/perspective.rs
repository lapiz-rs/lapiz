use std::collections::{HashMap, hash_map::Entry};

use anyhow::Result;
use bevy_math::{IRect, Rect};
use encase::ShaderType;
use glam::{Mat3, Vec2};
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
    layer_bounds::LayerBoundsPipeline,
    texel::TexelType,
    tile::{
        DynamicLayerStorage, GpuLayerInfo, GpuTileInfo, GpuTileStorage, LayerBinding,
        TileStorageAppExt,
    },
};
use lapiz_input::{key::KeyboardState, mouse::PressedMouseState};
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{DynamicBindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    readback::{create_readback_buffer_and_schedule_copy_buffer, readback_buffer_on_submit_async},
    render_context::RenderContextAppExt,
    util::DevicePollExt,
    wesl_jit,
};
use lapiz_runtime::{Renderer, Services, event::Event};
use lapiz_tools::{ToolFunction, ToolId};
use lapiz_undo::BatchedUndoCommand;
use lapiz_utils::log_err::LogErr;
use lapiz_widgets::{button::Button, icon, label::Label, panel::Panel};
use tracing::warn;
use wgpu::{
    BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, BufferUsages,
    ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor, Device,
    PipelineLayoutDescriptor, Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StorageTextureAccess,
};

const HANDLE_HIT_RADIUS: f32 = 20.0;
const HANDLE_SIZE: f32 = 10.0;
const VANISHING_HANDLE_RADIUS: f32 = 8.0;

#[derive(Debug, Clone, Copy)]
pub enum PerspectiveHandle {
    Translate,
    Corner(usize),
    Midpoint(usize),
    XVanishing,
    YVanishing,
}

#[derive(Debug, Clone, Copy)]
pub struct OngoingPerspectiveTransform {
    pub cursor_origin_ps: Vec2,
    pub handle: PerspectiveHandle,
    pub base_quad: [Vec2; 4],
}

#[derive(Debug, Clone)]
pub struct InitPerspectiveTransform {
    pub canvas_id: CanvasId,
    pub target_layers: Vec<LayerId>,
    pub selection_layer_id: LayerId,
    pub selection_bounds: IRect,
    pub pixel_bounds: IRect,
}

pub struct PerspectiveSession {
    pub canvas_id: CanvasId,
    pub target_layers: Vec<(LayerId, TexelType)>,
    pub selection_layer_id: LayerId,
    pub selection_bounds: IRect,
    pub src_quad: [Vec2; 4],
    pub dst_quad: [Vec2; 4],
    pub matrix: Mat3,
    pub tile_bounds: IRect,
    pub pixel_bounds: IRect,
    pub result_buffers: HashMap<LayerId, DynamicLayerStorage>,
    pub transform_pipelines: HashMap<TexelType, PerspectiveTransformPipeline>,
    pub ongoing_transform: Option<OngoingPerspectiveTransform>,
}

impl PerspectiveSession {
    pub fn new(init: InitPerspectiveTransform, services: &Services) -> Self {
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
                        e.insert(PerspectiveTransformPipeline::new(
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

        let src_quad = rect_to_quad(init.pixel_bounds.as_rect());

        Self {
            canvas_id: init.canvas_id,
            target_layers,
            selection_layer_id: init.selection_layer_id,
            selection_bounds: init.selection_bounds,
            src_quad,
            dst_quad: src_quad,
            matrix: Mat3::IDENTITY,
            tile_bounds: GpuTileStorage::pixel_rect_to_tile(init.pixel_bounds),
            pixel_bounds: init.pixel_bounds,
            result_buffers,
            transform_pipelines,
            ongoing_transform: None,
        }
    }

    pub fn quad_ps(&self) -> [Vec2; 4] {
        self.dst_quad
    }

    pub fn transformed_aabb_ps(&self) -> Rect {
        let quad = self.dst_quad;
        let mut min = quad[0];
        let mut max = quad[0];
        for p in &quad[1..] {
            min = min.min(*p);
            max = max.max(*p);
        }
        Rect::from_corners(min, max)
    }

    pub fn update(&mut self, cursor_ps: Vec2) {
        let Some(ongoing) = self.ongoing_transform else {
            return;
        };

        let Some(new_quad) = compute_candidate_quad(ongoing, cursor_ps, self.dst_quad) else {
            return;
        };

        if !is_convex_quad(new_quad) || is_quad_too_big(new_quad) {
            return;
        }

        let Some(matrix) = homography(self.src_quad, new_quad) else {
            return;
        };

        self.dst_quad = new_quad;
        self.matrix = matrix;
    }

    pub fn midpoints_ps(&self) -> [Vec2; 4] {
        quad_midpoints(self.dst_quad)
    }

    pub fn vanishing_points_ps(&self) -> (Option<Vec2>, Option<Vec2>) {
        vanishing_points(self.dst_quad)
    }
}

#[derive(Debug, Clone)]
pub enum PerspectiveTransformToolMessage {
    RequestInit,
    InitTransform(InitPerspectiveTransform),
    Cancel,
    Confirm,
}

#[derive(Default)]
pub struct PerspectiveTransformTool {
    session: Option<PerspectiveSession>,
    bounds_pipelines: HashMap<TexelType, LayerBoundsPipeline>,
}

impl ToolFunction for PerspectiveTransformTool {
    type Message = PerspectiveTransformToolMessage;

    fn id() -> ToolId {
        ToolId::new("perspective_transform_tool".into())
    }

    fn icon() -> icon::Icon<'static> {
        icon::perspective()
    }

    fn activate(&mut self, _services: &mut Services) -> Task<Self::Message> {
        Task::done(PerspectiveTransformToolMessage::RequestInit)
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
        let Some(handle) = hit_test(session.dst_quad, cursor_ps) else {
            return Task::none();
        };

        session.ongoing_transform = Some(OngoingPerspectiveTransform {
            cursor_origin_ps: cursor_ps,
            handle,
            base_quad: session.dst_quad,
        });

        Task::none()
    }

    fn update(
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

        if session.ongoing_transform.is_none() {
            return Task::none();
        }

        session.update(cursor_ps);
        render_transform_preview(session, services);

        Task::none()
    }

    fn end(
        &mut self,
        _keyboard: &KeyboardState,
        _mouse: &PressedMouseState,
        _services: &mut Services,
    ) -> Task<Self::Message> {
        if let Some(session) = &mut self.session {
            session.ongoing_transform = None;
        }
        Task::none()
    }

    fn handle_message(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> Task<Self::Message> {
        match message {
            PerspectiveTransformToolMessage::RequestInit => {
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
                    let init = InitPerspectiveTransform {
                        canvas_id,
                        target_layers,
                        selection_layer_id,
                        selection_bounds: selection_layer_bounds,
                        pixel_bounds: selection_layer_bounds,
                    };
                    return Task::done(PerspectiveTransformToolMessage::InitTransform(init));
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

                    Task::done(PerspectiveTransformToolMessage::InitTransform(
                        InitPerspectiveTransform {
                            canvas_id,
                            selection_layer_id,
                            target_layers: target_layers.clone(),
                            selection_bounds: selection_layer_bounds,
                            pixel_bounds: bounds,
                        },
                    ))
                });

                Task::batch([init_task, poll_task.discard()])
            }
            PerspectiveTransformToolMessage::InitTransform(init) => {
                if init.pixel_bounds.is_empty() {
                    warn!("Unable to transform on empty layer.");
                    self.session = None;
                } else {
                    self.session = Some(PerspectiveSession::new(init, services));
                }
                Task::none()
            }
            PerspectiveTransformToolMessage::Confirm => {
                Task::done(PerspectiveTransformToolMessage::RequestInit)
            }
            PerspectiveTransformToolMessage::Cancel => {
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
                Task::done(PerspectiveTransformToolMessage::RequestInit)
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

        Element::new(PerspectiveTransformToolOverlay {
            canvas_transform: &canvas.transform,
            session,
        })
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        CanvasActiveLayerChanged::listen_to().map(|_| PerspectiveTransformToolMessage::RequestInit)
    }

    fn tool_option_widget<'a>(
        &'a self,
        _services: &'a Services,
    ) -> Option<Element<'a, Self::Message, Theme, Renderer>> {
        let actions = row![
            Button::new(Label::new(t!("cancel")))
                .on_press(PerspectiveTransformToolMessage::Cancel)
                .danger()
                .width(Length::Fill),
            Button::new(Label::new(t!("confirm")))
                .on_press(PerspectiveTransformToolMessage::Confirm)
                .primary()
                .width(Length::Fill),
        ]
        .spacing(4);

        Some(
            Panel::new(column![actions].spacing(8))
                .padding(8)
                .width(Length::Fill)
                .into(),
        )
    }
}

pub fn hit_test(quad: [Vec2; 4], p: Vec2) -> Option<PerspectiveHandle> {
    let default = if point_in_quad(quad, p) {
        Some(PerspectiveHandle::Translate)
    } else {
        None
    };

    let mut best = default;
    let mut best_d2 = f32::MAX;

    let (vx, vy) = vanishing_points(quad);

    let mut consider_handle = |point: Vec2, handle: PerspectiveHandle| {
        let d2 = (point - p).length_squared();
        if d2 < HANDLE_HIT_RADIUS * HANDLE_HIT_RADIUS && d2 < best_d2 {
            best_d2 = d2;
            best = Some(handle);
        }
    };

    if let Some(point) = vx {
        consider_handle(point, PerspectiveHandle::XVanishing);
    }
    if let Some(point) = vy {
        consider_handle(point, PerspectiveHandle::YVanishing);
    }

    for (i, point) in quad.iter().enumerate() {
        consider_handle(*point, PerspectiveHandle::Corner(i));
    }

    for (i, point) in quad_midpoints(quad).iter().enumerate() {
        consider_handle(*point, PerspectiveHandle::Midpoint(i));
    }

    best
}

fn rect_to_quad(rect: Rect) -> [Vec2; 4] {
    let min = rect.min;
    let max = rect.max;
    [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
}

fn quad_midpoints(quad: [Vec2; 4]) -> [Vec2; 4] {
    [
        (quad[0] + quad[1]) * 0.5,
        (quad[2] + quad[3]) * 0.5,
        (quad[0] + quad[3]) * 0.5,
        (quad[1] + quad[2]) * 0.5,
    ]
}

fn point_in_quad(quad: [Vec2; 4], p: Vec2) -> bool {
    let [tl, tr, br, bl] = quad;
    let s1 = cross(tr - tl, p - tl);
    let s2 = cross(br - tr, p - tr);
    let s3 = cross(bl - br, p - br);
    let s4 = cross(tl - bl, p - bl);

    (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0 && s4 >= 0.0)
        || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0 && s4 <= 0.0)
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn is_convex_quad(quad: [Vec2; 4]) -> bool {
    let [tl, tr, br, bl] = quad;
    let c1 = cross(tr - tl, br - tr);
    let c2 = cross(br - tr, bl - br);
    let c3 = cross(bl - br, tl - bl);
    let c4 = cross(tl - bl, tr - tl);

    (c1 > 0.0 && c2 > 0.0 && c3 > 0.0 && c4 > 0.0) || (c1 < 0.0 && c2 < 0.0 && c3 < 0.0 && c4 < 0.0)
}

fn is_quad_too_big(quad: [Vec2; 4]) -> bool {
    for i in 0..4 {
        let pt = quad[i];
        let prev = quad[(i + 3) % 4];
        let next = quad[(i + 1) % 4];
        let other = quad[(i + 2) % 4];

        let Some(intersection) = line_intersection(pt, other, prev, next) else {
            return true;
        };

        let max_distance = pt.distance_squared(other);
        if pt.distance_squared(intersection) > max_distance
            || other.distance_squared(intersection) > max_distance
        {
            return true;
        }

        let l2 = next - prev;
        let l2_len = l2.length();
        if distance_to_line(pt, prev, next) < 0.02 * l2_len {
            return true;
        }
    }

    false
}

fn distance_to_line(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len = ab.length();
    if len < 1.0e-9 {
        return p.distance(a);
    }
    ((p.x - a.x) * ab.y - (p.y - a.y) * ab.x).abs() / len
}

fn compute_candidate_quad(
    ongoing: OngoingPerspectiveTransform,
    cursor_ps: Vec2,
    current_quad: [Vec2; 4],
) -> Option<[Vec2; 4]> {
    let base = ongoing.base_quad;
    let delta = cursor_ps - ongoing.cursor_origin_ps;

    match ongoing.handle {
        PerspectiveHandle::Translate => Some(base.map(|p| p + delta)),
        PerspectiveHandle::Corner(index) => {
            let mut quad = base;
            quad[index] = cursor_ps;
            Some(quad)
        }
        PerspectiveHandle::Midpoint(index) => {
            let mid = quad_midpoints(base)[index];
            let d = cursor_ps - mid;
            let mut quad = base;
            match index {
                0 => {
                    quad[0] += d;
                    quad[1] += d;
                }
                1 => {
                    quad[2] += d;
                    quad[3] += d;
                }
                2 => {
                    quad[0] += d;
                    quad[3] += d;
                }
                3 => {
                    quad[1] += d;
                    quad[2] += d;
                }
                _ => return None,
            }
            Some(quad)
        }
        PerspectiveHandle::XVanishing | PerspectiveHandle::YVanishing => {
            compute_vanishing_candidate(current_quad, ongoing.handle, cursor_ps)
        }
    }
}

fn compute_vanishing_candidate(
    quad: [Vec2; 4],
    handle: PerspectiveHandle,
    cursor_ps: Vec2,
) -> Option<[Vec2; 4]> {
    let (vx, vy) = vanishing_points(quad);
    let [tl, tr, br, bl] = quad;

    match handle {
        PerspectiveHandle::XVanishing => {
            let v = vx?;
            let other_v = vy?;
            if v.distance_squared(tl) > v.distance_squared(tr) {
                let new_tr = line_intersection(tl, cursor_ps, other_v, tr)?;
                let new_br = line_intersection(bl, cursor_ps, other_v, tr)?;
                Some([tl, new_tr, new_br, bl])
            } else {
                let new_tl = line_intersection(tr, cursor_ps, other_v, tl)?;
                let new_bl = line_intersection(br, cursor_ps, other_v, tl)?;
                Some([new_tl, tr, br, new_bl])
            }
        }
        PerspectiveHandle::YVanishing => {
            let v = vy?;
            let other_v = vx?;
            if v.distance_squared(tl) > v.distance_squared(bl) {
                let new_bl = line_intersection(tl, cursor_ps, other_v, bl)?;
                let new_br = line_intersection(tr, cursor_ps, other_v, bl)?;
                Some([tl, tr, new_br, new_bl])
            } else {
                let new_tl = line_intersection(bl, cursor_ps, other_v, tl)?;
                let new_tr = line_intersection(br, cursor_ps, other_v, tl)?;
                Some([new_tl, new_tr, br, bl])
            }
        }
        _ => None,
    }
}

fn vanishing_points(quad: [Vec2; 4]) -> (Option<Vec2>, Option<Vec2>) {
    let [tl, tr, br, bl] = quad;
    (
        line_intersection(tl, tr, bl, br),
        line_intersection(tl, bl, tr, br),
    )
}

fn line_intersection(a1: Vec2, a2: Vec2, b1: Vec2, b2: Vec2) -> Option<Vec2> {
    let d1 = a2 - a1;
    let d2 = b2 - b1;
    let denom = d1.x * d2.y - d1.y * d2.x;
    if denom.abs() < 1.0e-6 {
        return None;
    }

    let t = ((b1.x - a1.x) * d2.y - (b1.y - a1.y) * d2.x) / denom;
    Some(a1 + d1 * t)
}

fn transition_matrix(tl: Vec2, tr: Vec2, bl: Vec2, br: Vec2) -> Option<Mat3> {
    let tlh = tl.extend(1.0);
    let trh = tr.extend(1.0);
    let blh = bl.extend(1.0);
    let brh = br.extend(1.0);

    let m = Mat3::from_cols(tlh, trh, blh);
    if m.determinant().abs() < 1.0e-6 {
        return None;
    }

    let coeffs = m.inverse() * brh;
    Some(Mat3::from_cols(
        tlh * coeffs.x,
        trh * coeffs.y,
        blh * coeffs.z,
    ))
}

fn homography(src: [Vec2; 4], dst: [Vec2; 4]) -> Option<Mat3> {
    let a = transition_matrix(src[0], src[1], src[3], src[2])?;
    let b = transition_matrix(dst[0], dst[1], dst[3], dst[2])?;
    if a.determinant().abs() < 1.0e-6 {
        return None;
    }
    Some(b * a.inverse())
}

fn render_transform_preview(session: &mut PerspectiveSession, services: &mut Services) {
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
            &PerspectiveTransformParams {
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

fn commit_transform(session: PerspectiveSession, services: &mut Services) {
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
                "Perspective Transform".into(),
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

    let cmd = BatchedUndoCommand::new("Perspective Transform".into(), replace_commands);
    services
        .push_undo_command(&session.canvas_id, cmd)
        .log_err();
}

pub struct PerspectiveTransformToolOverlay<'a> {
    pub canvas_transform: &'a CanvasTransform,
    pub session: &'a PerspectiveSession,
}

impl<'a> Widget<PerspectiveTransformToolMessage, Theme, Renderer>
    for PerspectiveTransformToolOverlay<'a>
{
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
        let midpoints = self
            .session
            .midpoints_ps()
            .map(|p| self.canvas_transform.pixel_to_window(p));
        let [tl, tr, br, bl] = corners;
        let [mt, mb, ml, mr] = midpoints;
        let x = tr - tl;
        let y = bl - tl;

        let mut frame = Frame::with_bounds(renderer, layout.bounds());

        for (color, translation) in [
            (Color::WHITE, Vector::ZERO),
            (Color::BLACK, Vector::new(1.0, 1.0)),
        ] {
            frame.push_transform();
            frame.translate(translation);

            for point in [tl, tr, bl, br, mt, mb, ml, mr] {
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
                let hx = x.try_normalize().unwrap_or(Vec2::ZERO) * HANDLE_SIZE;
                let hy = y.try_normalize().unwrap_or(Vec2::ZERO) * HANDLE_SIZE;

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

            frame.pop_transform();
        }

        // Vanishing point handles, drawn like Krita's red perspective handles.
        let (vx, vy) = self.session.vanishing_points_ps();
        for point in [vx, vy].into_iter().flatten() {
            let center = self.canvas_transform.pixel_to_window(point);
            frame.stroke(
                &Path::circle(Point::new(center.x, center.y), VANISHING_HANDLE_RADIUS),
                Stroke {
                    style: Color::from_rgb(1.0, 0.0, 0.0).into(),
                    width: 1.5,
                    ..Default::default()
                },
            );
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

        let handle = self
            .session
            .ongoing_transform
            .as_ref()
            .map(|t| t.handle)
            .or_else(|| hit_test(self.session.quad_ps(), cursor_ps));

        match handle {
            Some(PerspectiveHandle::Translate) => mouse::Interaction::Move,
            Some(
                PerspectiveHandle::Corner(_)
                | PerspectiveHandle::Midpoint(_)
                | PerspectiveHandle::XVanishing
                | PerspectiveHandle::YVanishing,
            ) => mouse::Interaction::Pointer,
            None => mouse::Interaction::None,
        }
    }
}

pub struct PerspectiveTransformPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
    with_selection: bool,
}

impl PerspectiveTransformPipeline {
    pub fn new(
        device: &Device,
        format: TexelType,
        with_selection: bool,
        is_target_selection: bool,
    ) -> Self {
        let shader = wesl_jit::compile_wesl_with_config(
            include_str!("perspective.wesl").into(),
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
                binding_types::storage_buffer_read_only::<PerspectiveTransformParams>(false),
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
            label: Some("perspective transform bind group layout"),
            entries: entries.as_ref(),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("perspective transform pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });

        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("perspective transform shader module"),
            source: ShaderSource::Wgsl(shader.into()),
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("perspective transform pipeline"),
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
        params: &PerspectiveTransformParams,
        layer: LayerBinding,
        output: LayerBinding,
        selection: Option<LayerBinding>,
    ) {
        let mut params_buffer = DynamicBuffer::new(
            Some("perspective_transform_params_buffer".into()),
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
            label: Some("perspective transform bind group"),
            layout: &self.layout,
            entries: entries.as_ref(),
        });

        let mut ec = device.create_command_encoder(&Default::default());
        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("perspective transform pass"),
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
pub struct PerspectiveTransformParams {
    pub mat_inv: Mat3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perspective_shader_compiles_for_all_formats() {
        for format in TexelType::ALL_POSSIBLE_FORMATS {
            for with_selection in [false, true] {
                wesl_jit::compile_wesl_with_config(
                    include_str!("perspective.wesl").into(),
                    &[&lapiz_image::image::PACKAGE],
                    |compiler| {
                        compiler.set_feature(format.shader_def(), true);
                        compiler.set_feature("WITH_SELECTION", with_selection);
                    },
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn homography_maps_src_corners_to_dst_corners() {
        let src = rect_to_quad(Rect::new(0.0, 0.0, 100.0, 80.0));
        let dst = [
            Vec2::new(10.0, 10.0),
            Vec2::new(130.0, 20.0),
            Vec2::new(120.0, 110.0),
            Vec2::new(5.0, 95.0),
        ];

        let matrix = homography(src, dst).unwrap();
        for (src_point, dst_point) in src.iter().zip(dst.iter()) {
            let h = matrix * src_point.extend(1.0);
            let mapped = Vec2::new(h.x / h.z, h.y / h.z);
            assert!(
                mapped.distance(*dst_point) < 1.0e-3,
                "{mapped:?} != {dst_point:?}"
            );
        }
    }
}
