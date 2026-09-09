use std::{sync::OnceLock, time::Instant};

use anyhow::Result;
use bevy_math::IRect;
use encase::ShaderType;
use glam::{IVec2, Mat3, UVec2, UVec3};
use iced_core::Rectangle;
use iced_widget::shader;
use lapiz_color::shader::IccTransformShader;
use lapiz_image::{
    layer::LayerId,
    texel::TexelType,
    tile::{GpuTileInfo, GpuTileStorage},
};
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    resources::{FullscreenVertex, GlobalSamplers},
    wesl_jit::compile_wesl,
};
use moxcms::{ColorProfile, Layout};
use wesl::include_wesl;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, BufferUsages, ColorTargetState,
    ColorWrites, CommandEncoder, ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor,
    Device, Extent3d, FilterMode, FragmentState, LoadOp, Operations, PipelineLayoutDescriptor,
    Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StorageTextureAccess, StoreOp,
    TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureView, TextureViewDescriptor, TextureViewDimension,
};

use crate::control::CanvasTransform;

pub const INTERMEDIATE_BUFFER_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
pub const ICC_TRANSFORM_SHADER_IDENT: &str = "calibrate_color";

#[derive(Debug)]
pub struct CanvasRenderer {
    texture: Option<TextureView>,
    render_pipeline: CanvasRenderPipeline,
    present_pipeline: CanvasPresentPipeline,
    root_texel_type: TexelType,
    selection_texel_type: TexelType,
    window_id: u64,
    monitor_name: String,
}

impl CanvasRenderer {
    fn ensure_pipeline(
        &mut self,
        device: &Device,
        root_texel_type: TexelType,
        selection_texel_type: TexelType,
        src_pr: &ColorProfile,
        window_id: u64,
        monitor_name: &str,
    ) -> Result<()> {
        if self.root_texel_type == root_texel_type
            && self.selection_texel_type == selection_texel_type
            && self.window_id == window_id
            && self.monitor_name == monitor_name
        {
            return Ok(());
        }

        let dst_pr = lapiz_color::platform::get_window_color_profile(window_id)?;
        let icc_transform = IccTransformShader::new(
            ICC_TRANSFORM_SHADER_IDENT,
            src_pr,
            Layout::Rgb,
            &dst_pr,
            Layout::Rgb,
            Default::default(),
        )?;
        self.render_pipeline = CanvasRenderPipeline::new(
            device,
            root_texel_type,
            selection_texel_type,
            &icc_transform,
        );
        self.root_texel_type = root_texel_type;
        self.selection_texel_type = selection_texel_type;
        self.window_id = window_id;
        self.monitor_name = monitor_name.to_owned();
        Ok(())
    }

    fn resize_output_buffer(&mut self, device: &Device, size: UVec2) {
        if size.x == 0 || size.y == 0 {
            return;
        }
        if self.texture.as_ref().is_some_and(|texture| {
            texture.texture().width() == size.x && texture.texture().height() == size.y
        }) {
            return;
        }

        let texture = device.create_texture(&TextureDescriptor {
            label: Some("canvas render buffer"),
            size: Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: INTERMEDIATE_BUFFER_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.texture = Some(texture.create_view(&TextureViewDescriptor::default()));
    }

    fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        canvas_transform: &CanvasTransform,
        image_size: UVec2,
        tile_storage: &GpuTileStorage,
        root_layer_id: LayerId,
        selection_layer_id: LayerId,
    ) {
        let Some(texture) = &self.texture else {
            return;
        };
        let tile_rect = GpuTileStorage::pixel_rect_to_tile(IRect {
            min: IVec2::ZERO,
            max: image_size.as_ivec2(),
        });
        static FIRST_DRAW: OnceLock<Instant> = OnceLock::new();
        self.render_pipeline.prepare(
            device,
            queue,
            CanvasUniform {
                transform: canvas_transform.pixel_to_widget,
                inv_transform: canvas_transform.pixel_to_widget.inverse(),
                size: image_size,
                total_tile_count: tile_rect.size().as_uvec2(),
                tile_size: GpuTileStorage::TILE_SIZE,
                time: FIRST_DRAW.get_or_init(Instant::now).elapsed().as_secs_f32(),
            },
            texture,
            tile_storage,
            root_layer_id,
            selection_layer_id,
        );
        self.present_pipeline.prepare(device, texture);
    }

    fn dispatch(&self, device: &Device, queue: &Queue) {
        let mut encoder = device.create_command_encoder(&Default::default());
        self.render_pipeline.draw(&mut encoder);
        queue.submit([encoder.finish()]);
    }

    fn draw(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        self.present_pipeline.present(encoder, target, clip_bounds);
    }
}

impl shader::Pipeline for CanvasRenderer {
    fn new(device: &Device, _: &Queue, format: TextureFormat) -> Self {
        let fullscreen_vertex = FullscreenVertex::new(device);
        let global_samplers = GlobalSamplers::new(device);
        let render_pipeline = CanvasRenderPipeline::new(
            device,
            TexelType::RGBA8,
            TexelType::A8,
            &IccTransformShader::unmanaged(ICC_TRANSFORM_SHADER_IDENT),
        );
        let present_pipeline =
            CanvasPresentPipeline::new(device, format, &fullscreen_vertex, &global_samplers);
        Self {
            texture: None,
            render_pipeline,
            present_pipeline,
            root_texel_type: TexelType::RGBA8,
            selection_texel_type: TexelType::A8,
            window_id: u64::MAX,
            monitor_name: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CanvasPrimitive {
    pub image_size: UVec2,
    pub root_layer: LayerId,
    pub selection_layer: LayerId,
    pub root_texel_type: TexelType,
    pub selection_texel_type: TexelType,
    pub transform: CanvasTransform,
    pub tile_storage: GpuTileStorage,
    pub color_profile: ColorProfile,
    pub window_id: u64,
    pub monitor_name: String,
}

impl shader::Primitive for CanvasPrimitive {
    type Pipeline = CanvasRenderer;

    fn prepare(
        &self,
        renderer: &mut Self::Pipeline,
        device: &Device,
        queue: &Queue,
        bounds: &Rectangle,
        _: &shader::Viewport,
    ) {
        renderer
            .ensure_pipeline(
                device,
                self.root_texel_type,
                self.selection_texel_type,
                &self.color_profile,
                self.window_id,
                &self.monitor_name,
            )
            .ok();
        renderer.resize_output_buffer(
            device,
            UVec2::new(bounds.width.max(0.0) as u32, bounds.height.max(0.0) as u32),
        );
        renderer.prepare(
            device,
            queue,
            &self.transform,
            self.image_size,
            &self.tile_storage,
            self.root_layer,
            self.selection_layer,
        );
        renderer.dispatch(device, queue);
    }

    fn render(
        &self,
        renderer: &Self::Pipeline,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        renderer.draw(encoder, target, clip_bounds);
    }
}

#[derive(Debug)]
struct CanvasRenderPipeline {
    pipeline: ComputePipeline,
    main_layout: BindGroupLayout,
    uniform_buffer: DynamicBuffer<CanvasUniform>,
    dispatch: Option<(BindGroup, UVec3)>,
}

#[derive(Debug, Clone, Copy, ShaderType)]
struct CanvasUniform {
    transform: Mat3,
    inv_transform: Mat3,
    size: UVec2,
    total_tile_count: UVec2,
    tile_size: u32,
    time: f32,
}

impl CanvasRenderPipeline {
    fn new(
        device: &Device,
        root_texel_type: TexelType,
        selection_texel_type: TexelType,
        icc_transform: &IccTransformShader,
    ) -> Self {
        let main_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("canvas main layout"),
            entries: BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::texture_storage_2d_array(
                        root_texel_type.wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::uniform_buffer::<CanvasUniform>(false),
                    binding_types::texture_storage_2d(
                        INTERMEDIATE_BUFFER_FORMAT,
                        StorageTextureAccess::WriteOnly,
                    ),
                    binding_types::texture_storage_2d_array(
                        selection_texel_type.wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                ),
            )
            .as_ref(),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("canvas pipeline layout"),
            bind_group_layouts: &[Some(&main_layout)],
            immediate_size: 0,
        });
        let shader = include_str!("shaders/canvas_render.wesl")
            .replace("//CODEGEN_FLAG_CALIBRATE_COLOR", &icc_transform.function);
        let shader_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("canvas shader"),
            source: ShaderSource::Wgsl(
                compile_wesl(shader, &[&lapiz_image::image::PACKAGE])
                    .unwrap()
                    .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("canvas pipeline"),
            layout: Some(&pipeline_layout),
            entry_point: Some("main"),
            module: &shader_module,
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            pipeline,
            main_layout,
            uniform_buffer: DynamicBuffer::new(
                Some("canvas uniform buffer".into()),
                BufferUsages::UNIFORM,
            ),
            dispatch: None,
        }
    }

    fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        uniform: CanvasUniform,
        target: &TextureView,
        tile_storage: &GpuTileStorage,
        root_layer_id: LayerId,
        selection_layer_id: LayerId,
    ) {
        self.uniform_buffer.clear();
        self.uniform_buffer.push(&uniform);
        self.uniform_buffer.write_buffer(device, queue);
        let Some(uniform_buffer) = self.uniform_buffer.binding() else {
            return;
        };
        let root_layer = tile_storage
            .get_layer_binding_or_empty(root_layer_id)
            .unwrap();
        let selection_layer = tile_storage
            .get_layer_binding_or_empty(selection_layer_id)
            .unwrap();
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("canvas render bind group"),
            layout: &self.main_layout,
            entries: BindGroupEntries::sequential((
                BindingResource::TextureView(&root_layer.texture),
                root_layer.tile_info_buffer.as_entire_binding(),
                uniform_buffer,
                BindingResource::TextureView(target),
                BindingResource::TextureView(&selection_layer.texture),
                selection_layer.tile_info_buffer.as_entire_binding(),
            ))
            .as_ref(),
        });
        let target_size = target.texture().size();
        self.dispatch = Some((
            bind_group,
            UVec3::new(
                target_size.width.div_ceil(16),
                target_size.height.div_ceil(16),
                1,
            ),
        ));
    }

    fn draw(&self, encoder: &mut CommandEncoder) {
        let Some((bind_group, workgroups)) = &self.dispatch else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("canvas render pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(workgroups.x, workgroups.y, workgroups.z);
    }
}

#[derive(Debug)]
struct CanvasPresentPipeline {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    sampler: Sampler,
    bind_group: Option<BindGroup>,
}

impl CanvasPresentPipeline {
    fn new(
        device: &Device,
        format: TextureFormat,
        fullscreen_vertex: &FullscreenVertex,
        _: &GlobalSamplers,
    ) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("canvas present shader"),
            source: ShaderSource::Wgsl(include_wesl!("canvas_present").into()),
        });
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("canvas present bind group layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("canvas present pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("canvas present pipeline"),
            layout: Some(&pipeline_layout),
            vertex: fullscreen_vertex.fullscreen_vertex_state(),
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("canvas present sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            bind_group: None,
        }
    }

    fn prepare(&mut self, device: &Device, src: &TextureView) {
        self.bind_group = Some(device.create_bind_group(&BindGroupDescriptor {
            label: Some("canvas present bind group"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(src),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    fn present(
        &self,
        encoder: &mut CommandEncoder,
        dst: &TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("canvas present pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: dst,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_viewport(
            clip_bounds.x as f32,
            clip_bounds.y as f32,
            clip_bounds.width as f32,
            clip_bounds.height as f32,
            0.0,
            1.0,
        );
        pass.draw(0..3, 0..1);
    }
}
