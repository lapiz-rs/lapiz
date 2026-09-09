use std::{borrow::Cow, sync::OnceLock, time::Instant};

use bevy_math::IRect;
use encase::ShaderType;
use glam::{IVec2, Mat3, Vec2};
use iced_core::{
    Element, Length, Rectangle, Size, Widget, keyboard::Modifiers, layout, pointer::mouse,
    renderer, widget,
};
use iced_wgpu::{Primitive, graphics::Viewport};
use iced_widget::shader::Pipeline;
use indexmap::IndexSet;
use lapiz_anti_aliasing::fxaa::{FxaaParams, FxaaPipeline};
use lapiz_canvas::{CanvasAppExt, command::TileReplaceCommand, control::CanvasTransform};
use lapiz_image::{
    texel::TexelType,
    tile::{
        DynamicLayerStorage, GpuLayerInfo, GpuTileInfo, GpuTileStorage, LayerBinding,
        TileStorageAppExt,
    },
};
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    render_context::RenderContextAppExt,
};
use lapiz_runtime::Renderer;
use lapiz_runtime::Services;
use lyon::{
    path::Path,
    tessellation::{
        BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
    },
};
use wesl::include_wesl;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, BindingResource,
    BufferUsages, ColorTargetState, ColorWrites, CommandEncoder, ComputePassDescriptor,
    ComputePipeline, ComputePipelineDescriptor, Device, FragmentState, IndexFormat, LoadOp,
    Operations, PipelineLayoutDescriptor, Queue, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipeline, RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StorageTextureAccess, StoreOp, TextureFormat, TextureView, VertexAttribute, VertexBufferLayout,
    VertexFormat, VertexState, VertexStepMode,
    util::{BufferInitDescriptor, DeviceExt},
};

pub fn generate_cmd(
    label: Cow<'static, str>,
    vertices: &[Vec2],
    indices: &[u32],
    aabb_ps: IRect,
    op: SelectionOperation,
    services: &mut Services,
) -> Option<TileReplaceCommand> {
    let canvas = services.current_canvas()?;
    let canvas_id = canvas.id();

    let tiles = services.tile_storage();
    let device = services.render_device();
    let queue = services.render_queue();

    let selection_layer_id = canvas.image.selection_layer();

    let affected_tiles = GpuTileStorage::pixel_rect_to_tile(aabb_ps);

    let selection_layer = tiles.get_layer(selection_layer_id).unwrap();
    let selection_layer_format = selection_layer.layer_info().texel_type;
    let selection_layer_binding = selection_layer.binding_or_empty();

    let mut pipeline = SelectionPipeline::new(device, selection_layer_format);
    unsafe {
        device.start_graphics_debugger_capture();
    };
    let selection = pipeline.draw(
        device,
        queue,
        affected_tiles,
        vertices,
        indices,
        op,
        selection_layer_binding,
        selection_layer.iter_tiles().map(|(i, _, _)| i).collect(),
    )?;
    unsafe {
        device.stop_graphics_debugger_capture();
    };

    Some(TileReplaceCommand::new(
        label,
        canvas_id,
        device,
        queue,
        selection_layer_id,
        &selection_layer,
        selection.iter_tiles().map(|(i, _, _)| i).collect(),
        selection.texture().unwrap().clone(),
    ))
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum SelectionOperation {
    Replace,
    Intersect,
    Union,
    Subtract,
    SymmetricDifference,
}

impl SelectionOperation {
    pub fn from_modifiers(modifiers: Modifiers) -> Self {
        if modifiers == MODIFIERS_INTERSECT {
            return Self::Intersect;
        }
        if modifiers == MODIFIERS_UNION {
            return Self::Union;
        }
        if modifiers == MODIFIERS_SUBTRACT {
            return Self::Subtract;
        }
        if modifiers == MODIFIERS_SYMMETRIC_DIFFERENCE {
            return Self::SymmetricDifference;
        }
        Self::Replace
    }
}

pub const MODIFIERS_INTERSECT: Modifiers = Modifiers::ALT.union(Modifiers::SHIFT);
pub const MODIFIERS_UNION: Modifiers = Modifiers::SHIFT;
pub const MODIFIERS_SUBTRACT: Modifiers = Modifiers::ALT;
pub const MODIFIERS_SYMMETRIC_DIFFERENCE: Modifiers = Modifiers::CTRL.union(Modifiers::ALT);

#[derive(ShaderType, Debug, Clone, Copy)]
struct SelectionParams {
    operation_ty: u32,
}

pub struct SelectionPipeline {
    render_layout: BindGroupLayout,
    render_pipeline: RenderPipeline,
    fxaa_pipeline: FxaaPipeline,
    composite_layout: BindGroupLayout,
    composite_pipeline: ComputePipeline,
    layer_format: TexelType,
}

impl SelectionPipeline {
    pub fn new(device: &Device, layer_format: TexelType) -> Self {
        let render_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("selection_render_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX,
                (binding_types::uniform_buffer::<IVec2>(true),),
            ),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("selection_render_pipeline_layout"),
            bind_group_layouts: &[Some(&render_layout)],
            immediate_size: 0,
        });

        let render_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("selection_render_shader"),
            source: ShaderSource::Wgsl(include_wesl!("render").into()),
        });

        let render_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("selection_render_pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: VertexState {
                module: &render_shader,
                entry_point: "vertex".into(),
                compilation_options: Default::default(),
                buffers: &[VertexBufferLayout {
                    array_stride: 8,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
                        // Position in pixel space
                        VertexAttribute {
                            shader_location: 0,
                            format: VertexFormat::Float32x2,
                            offset: 0,
                        },
                    ],
                }],
            },
            fragment: Some(FragmentState {
                module: &render_shader,
                entry_point: "fragment".into(),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format: layer_format.wgpu_format(),
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let fxaa_pipeline = FxaaPipeline::new(device, layer_format.wgpu_format());

        let composite_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("selection_composite_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::texture_storage_2d_array(
                        layer_format.wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::texture_storage_2d_array(
                        layer_format.wgpu_format(),
                        StorageTextureAccess::ReadWrite,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::uniform_buffer::<SelectionParams>(false),
                ),
            ),
        });

        let composite_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("selection_composite_shader"),
            source: ShaderSource::Wgsl(include_wesl!("composite").into()),
        });

        let composite_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("selection_composite_pipeline_layout"),
            bind_group_layouts: &[Some(&composite_layout)],
            immediate_size: 0,
        });

        let composite_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("selection_composite_pipeline"),
            layout: Some(&composite_pipeline_layout),
            module: &composite_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            render_layout,
            render_pipeline,
            fxaa_pipeline,
            composite_layout,
            composite_pipeline,
            layer_format,
        }
    }

    #[tracing::instrument(skip_all)]
    pub fn draw(
        &mut self,
        device: &Device,
        queue: &Queue,
        tile_aabb: IRect,
        vertices: &[Vec2],
        // Must be in counter-clockwise order
        indices: &[u32],
        op: SelectionOperation,
        target_selection: LayerBinding,
        target_selection_tile_indices: IndexSet<IVec2>,
    ) -> Option<DynamicLayerStorage> {
        let selection = self.render_with_target_selection_reserved_output(
            device,
            queue,
            tile_aabb,
            vertices,
            indices,
            target_selection_tile_indices,
        )?;
        self.composite_with_target_selection_reserved_input(
            device,
            queue,
            op,
            &selection,
            &target_selection,
        )
    }

    /// Render the selection mesh with only affected tiles.
    #[tracing::instrument(skip_all)]
    pub fn render_with_tight_output(
        &mut self,
        device: &Device,
        queue: &Queue,
        tile_aabb: IRect,
        vertices: &[Vec2],
        // Must be in counter-clockwise order
        indices: &[u32],
    ) -> Option<DynamicLayerStorage> {
        self.render_internal(device, queue, tile_aabb, vertices, indices, IndexSet::new())
    }

    /// Render the selection mesh with affected tiles and reserve tiles that exist in target selection.
    #[tracing::instrument(skip_all)]
    pub fn render_with_target_selection_reserved_output(
        &mut self,
        device: &Device,
        queue: &Queue,
        tile_aabb: IRect,
        vertices: &[Vec2],
        // Must be in counter-clockwise order
        indices: &[u32],
        reserved_output_tiles: IndexSet<IVec2>,
    ) -> Option<DynamicLayerStorage> {
        self.render_internal(
            device,
            queue,
            tile_aabb,
            vertices,
            indices,
            reserved_output_tiles,
        )
    }

    fn render_internal(
        &mut self,
        device: &Device,
        queue: &Queue,
        tile_aabb: IRect,
        vertices: &[Vec2],
        // Must be in counter-clockwise order
        indices: &[u32],
        reserved_output_tiles: IndexSet<IVec2>,
    ) -> Option<DynamicLayerStorage> {
        if indices.is_empty() || vertices.is_empty() || tile_aabb.is_empty() {
            return None;
        }

        assert_eq!(indices.len() % 3, 0);

        let vertices_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("selection_vertices_buffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: BufferUsages::VERTEX,
        });

        let indices_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("selection_indices_buffer"),
            contents: bytemuck::cast_slice(indices),
            usage: BufferUsages::INDEX,
        });

        let n_render_tiles = (tile_aabb.max - tile_aabb.min).element_product() as usize;

        let mut cur_rendering_indices = Vec::with_capacity(n_render_tiles);
        let mut cur_rendering_index_buffer =
            DynamicBuffer::new(Some("cur_tile_index_buffer".into()), BufferUsages::UNIFORM);
        let mut cur_rendering_index_offsets = Vec::with_capacity(n_render_tiles);

        let mut output_tiles = reserved_output_tiles;

        for x in tile_aabb.min.x..tile_aabb.max.x {
            for y in tile_aabb.min.y..tile_aabb.max.y {
                let index = IVec2 { x, y };

                cur_rendering_indices.push(index);
                let offset = cur_rendering_index_buffer.push(&index);
                cur_rendering_index_offsets.push(offset);
                output_tiles.insert(index);
            }
        }

        cur_rendering_index_buffer.write_buffer(device, queue);

        let mut selection = DynamicLayerStorage::new(
            device.clone(),
            queue.clone(),
            GpuLayerInfo {
                texel_type: self.layer_format,
            },
        );
        selection.allocate_tiles_batch(output_tiles);

        let render_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("selection_render_bind_group"),
            layout: &self.render_layout,
            entries: &BindGroupEntries::single(cur_rendering_index_buffer.binding().unwrap()),
        });

        let mut ec = device.create_command_encoder(&Default::default());

        for (tile_index, index_buffer_offset) in cur_rendering_indices
            .into_iter()
            .zip(cur_rendering_index_offsets)
        {
            let target_view = selection.get_tile(tile_index).unwrap();

            let mut pass = ec.begin_render_pass(&RenderPassDescriptor {
                label: Some(&format!(
                    "selection_render_pass_x{}_y{}",
                    tile_index.x, tile_index.y
                )),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.render_pipeline);
            pass.set_bind_group(0, &render_bind_group, &[index_buffer_offset as u32]);
            pass.set_vertex_buffer(0, vertices_buffer.slice(..));
            pass.set_index_buffer(indices_buffer.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
        }

        queue.submit([ec.finish()]);

        let smoothed_selection = selection.create_allocated_empty_sibling();

        self.fxaa_pipeline.dispatch(
            device,
            queue,
            &FxaaParams::default(),
            selection.binding().unwrap(),
            smoothed_selection.binding().unwrap(),
        );

        Some(smoothed_selection)
    }

    /// Composite the input selection with the target selection. The input selection is allowed to unable
    /// to cover the target selection.
    ///
    /// The returned mask has no empty tiles.
    #[tracing::instrument(skip_all)]
    pub fn composite_with_tight_input(
        &self,
        device: &Device,
        queue: &Queue,
        op: SelectionOperation,
        input_selection: &DynamicLayerStorage,
        target_selection: &DynamicLayerStorage,
        target_selection_binding: &LayerBinding,
    ) -> Option<DynamicLayerStorage> {
        let mut output_tiles = input_selection.deep_clone();
        output_tiles.allocate_tiles_batch(target_selection.iter_tile_indices());

        self.composite_with_target_selection_reserved_input(
            device,
            queue,
            op,
            &output_tiles,
            target_selection_binding,
        )
    }

    /// Blend the selection from inout texture with target selection, then write it back to inout_texture.
    /// The inout texture must ensure it contains the area of target selection. Otherwise the result will
    /// be incomplete.
    ///
    /// The returned mask has no empty tiles.
    #[tracing::instrument(skip_all)]
    pub fn composite_with_target_selection_reserved_input(
        &self,
        device: &Device,
        queue: &Queue,
        op: SelectionOperation,
        input_selection: &DynamicLayerStorage,
        target_selection: &LayerBinding,
    ) -> Option<DynamicLayerStorage> {
        let output_selection = input_selection.deep_clone();

        let composite_params_buffer = {
            let mut b = DynamicBuffer::new(
                Some("selection_params_buffer".into()),
                BufferUsages::UNIFORM,
            );
            b.push(&SelectionParams {
                operation_ty: op as u32,
            });
            b.write_buffer(device, queue);
            b
        };

        let composite_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("selection_composite_bind_group"),
            layout: &self.composite_layout,
            entries: &BindGroupEntries::sequential((
                BindingResource::TextureView(&target_selection.texture),
                target_selection.tile_info_buffer.as_entire_binding(),
                BindingResource::TextureView(output_selection.texture_view().unwrap()),
                output_selection
                    .tile_info_buffer()
                    .unwrap()
                    .as_entire_binding(),
                composite_params_buffer.binding().unwrap(),
            )),
        });

        let mut ec = device.create_command_encoder(&Default::default());

        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("selection_composite_pass"),
                ..Default::default()
            });

            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &composite_bind_group, &[]);
            pass.dispatch_workgroups(
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                output_selection.len() as u32,
            );
        }

        queue.submit([ec.finish()]);

        Some(output_selection)
    }
}

#[derive(ShaderType, Debug, Clone, Copy)]
pub struct CanvasUniform {
    pub pixel_to_widget: Mat3,
    pub widget_min: Vec2,
    pub screen_size: Vec2,
    pub time: f32,
}

pub struct SelectionPreviewLayer<'a> {
    pub line_vertices_ps: Cow<'a, [Vec2]>,
    pub canvas_transform: &'a CanvasTransform,
}

impl<'a, Message, Theme> Widget<Message, Theme, Renderer> for SelectionPreviewLayer<'a> {
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
        _layout: layout::Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        iced_wgpu::primitive::Renderer::draw_primitive(
            renderer,
            *viewport,
            SelectionPreviewPrimitive {
                line_vertices_ps: self.line_vertices_ps.clone().into_owned(),
                canvas_transform: self.canvas_transform.clone(),
            },
        );
    }
}

impl<'a, Message, Theme> From<SelectionPreviewLayer<'a>> for Element<'a, Message, Theme, Renderer> {
    fn from(val: SelectionPreviewLayer<'a>) -> Element<'a, Message, Theme, Renderer> {
        Element::new(val)
    }
}

struct PreparedSelectionPreviewPipeline {
    bind_group: BindGroup,
    n_vertices: u32,
}

pub struct SelectionPreviewPipeline {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
    prepared: Option<PreparedSelectionPreviewPipeline>,
}

impl Pipeline for SelectionPreviewPipeline {
    fn new(device: &Device, _queue: &Queue, format: TextureFormat) -> Self
    where
        Self: Sized,
    {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("selection_preview_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                (
                    binding_types::uniform_buffer::<CanvasUniform>(false)
                        .visibility(ShaderStages::VERTEX | ShaderStages::FRAGMENT),
                    binding_types::storage_buffer_read_only::<Vec2>(false)
                        .visibility(ShaderStages::VERTEX),
                ),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("selection_preview_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("selection_preview_shader"),
            source: ShaderSource::Wgsl(include_wesl!("preview").into()),
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("selection_preview_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },

            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            layout,
            pipeline,
            prepared: None,
        }
    }
}

impl SelectionPreviewPipeline {
    pub fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        line_vertices_ps: &[Vec2],
        screen_size: Vec2,
        canvas_transform: &CanvasTransform,
    ) {
        if line_vertices_ps.len() < 2 {
            return;
        }

        let vertices_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("selection_preview_vertices"),
            contents: bytemuck::cast_slice(line_vertices_ps),
            usage: BufferUsages::STORAGE,
        });

        static FIRST_DRAW: OnceLock<Instant> = OnceLock::new();
        let canvas_params_buffer = {
            let mut b =
                DynamicBuffer::new(Some("canvas_params_buffer".into()), BufferUsages::UNIFORM);
            b.push(&CanvasUniform {
                pixel_to_widget: canvas_transform.pixel_to_widget,
                widget_min: canvas_transform.widget_bounds.min,
                screen_size,
                time: FIRST_DRAW.get_or_init(Instant::now).elapsed().as_secs_f32(),
            });
            b.write_buffer(device, queue);
            b.into_inner_buffer().unwrap()
        };

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("canvas_params_bind_group"),
            layout: &self.layout,
            entries: &BindGroupEntries::sequential((
                canvas_params_buffer.as_entire_binding(),
                vertices_buffer.as_entire_binding(),
            )),
        });

        self.prepared = Some(PreparedSelectionPreviewPipeline {
            bind_group,
            n_vertices: line_vertices_ps.len() as u32,
        });
    }

    pub fn draw(
        &self,
        ec: &mut CommandEncoder,
        canvas_surface: &TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(prepared) = &self.prepared else {
            return;
        };

        {
            let mut pass = ec.begin_render_pass(&RenderPassDescriptor {
                label: Some("selection_preview_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: canvas_surface,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &prepared.bind_group, &[]);
            pass.set_scissor_rect(
                clip_bounds.x,
                clip_bounds.y,
                clip_bounds.width,
                clip_bounds.height,
            );
            pass.draw(0..(prepared.n_vertices - 1) * 6, 0..1);
        }
    }
}

pub(crate) fn indices_from_vertices(
    vertices: &[Vec2],
    fill_rule: FillRule,
) -> VertexBuffers<Vec2, u32> {
    let mut builder = Path::builder();
    builder.begin(lyon::geom::point(vertices[0].x, vertices[0].y));
    for v in &vertices[1..] {
        builder.line_to(lyon::geom::point(v.x, v.y));
    }
    builder.end(true);
    let path = builder.build();

    let mut geometry = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();

    let options = FillOptions::default().with_fill_rule(fill_rule);

    tessellator
        .tessellate_path(
            &path,
            &options,
            &mut BuffersBuilder::new(&mut geometry, |v: FillVertex| {
                Vec2::new(v.position().x, v.position().y)
            }),
        )
        .unwrap();

    geometry
}

#[derive(Debug)]
pub struct SelectionPreviewPrimitive {
    pub line_vertices_ps: Vec<Vec2>,
    pub canvas_transform: CanvasTransform,
}

impl Primitive for SelectionPreviewPrimitive {
    type Pipeline = SelectionPreviewPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &Device,
        queue: &Queue,
        _bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let screen_size = viewport.logical_size();
        pipeline.prepare(
            device,
            queue,
            &self.line_vertices_ps,
            Vec2::new(screen_size.width, screen_size.height),
            &self.canvas_transform,
        );
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        pipeline.draw(encoder, target, clip_bounds);
    }
}
