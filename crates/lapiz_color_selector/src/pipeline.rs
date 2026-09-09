use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use encase::ShaderType;
use glam::{UVec2, Vec2, Vec3, Vec4};
use lapiz_color::{
    model::rgb::Rgb,
    shader::{IccInputTransformShader, IccOutputTransformShader},
};
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    readback::{
        AsyncBufferReadback, create_readback_buffer_and_schedule_copy_buffer,
        readback_buffer_on_submit_async,
    },
    wesl_jit::{compile_wesl, compile_wesl_with_config},
};
use moxcms::{ColorProfile, Layout};
use wgpu::{
    BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, BlendState, Buffer,
    BufferUsages, ColorTargetState, ColorWrites, ComputePassDescriptor, ComputePipeline,
    ComputePipelineDescriptor, Device, FragmentState, PipelineLayoutDescriptor, Queue,
    RenderPipeline, RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    TextureFormat, VertexAttribute, VertexBufferLayout, VertexFormat, VertexState, VertexStepMode,
    util::{BufferInitDescriptor, DeviceExt},
};

use crate::{
    GradientPlaneShape,
    config::{GradientBarConfig, GradientPlaneConfig},
};

const COMPUTE_BOUNDS_RESOLUTION: u32 = 256;
const COMPUTE_BOUNDS_PADDING: u32 = 5;

fn compile_gradient_shader(
    profile: &ColorProfile,
    output_profile: &ColorProfile,
    srgb_output: bool,
) -> Result<String> {
    let input = IccInputTransformShader::new("picker_to_pcs", profile, Layout::Rgb)?;
    let image = IccOutputTransformShader::new("pcs_to_image", profile, Layout::Rgb)?;
    let output = IccOutputTransformShader::new("pcs_to_output", output_profile, Layout::Rgb)?;

    compile_wesl_with_config(
        include_str!("../shader/gradient.wesl")
            .replace("//CODEGEN_FLAG_PICKER_TO_PCS", &input.function)
            .replace("//CODEGEN_FLAG_PCS_TO_IMAGE", &image.function)
            .replace("//CODEGEN_FLAG_PCS_TO_OUTPUT", &output.function),
        &[&lapiz_color::color::PACKAGE],
        |wesl| {
            wesl.set_feature("IS_SRGB_OUTPUT", srgb_output);
        },
    )
}

fn compile_compute_bounds_shader(profile: &ColorProfile) -> Result<String> {
    let input = IccInputTransformShader::new("picker_to_pcs", profile, Layout::Rgb)?;
    let image = IccOutputTransformShader::new("pcs_to_image", profile, Layout::Rgb)?;

    compile_wesl(
        include_str!("../shader/compute_bounds.wesl")
            .replace("//CODEGEN_FLAG_PICKER_TO_PCS", &input.function)
            .replace("//CODEGEN_FLAG_PCS_TO_IMAGE", &image.function),
        &[&lapiz_color::color::PACKAGE],
    )
}

pub struct ComputeBoundsPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl ComputeBoundsPipeline {
    pub fn new(device: &Device, profile: &ColorProfile) -> Self {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("compute_bounds_layout"),
            entries: BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::uniform_buffer::<GradientSettings>(false),
                    binding_types::uniform_buffer::<ComputeBoundsParams>(false),
                    binding_types::storage_buffer::<OutputBounds>(false),
                ),
            )
            .as_ref(),
        });
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("compute_bounds_shader"),
            source: ShaderSource::Wgsl(compile_compute_bounds_shader(profile).unwrap().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("compute_bounds_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("compute_bounds_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self { layout, pipeline }
    }

    pub fn compute(
        &self,
        device: &Device,
        queue: &Queue,
        settings: &GradientSettings,
    ) -> AsyncBufferReadback<OutputBounds> {
        let mut settings_buffer = DynamicBuffer::new(
            Some("compute_bounds_settings_buffer".into()),
            BufferUsages::UNIFORM,
        );
        settings_buffer.push(settings);
        settings_buffer.write_buffer(device, queue);

        let mut params_buffer = DynamicBuffer::new(
            Some("compute_bounds_params_buffer".into()),
            BufferUsages::UNIFORM,
        );
        params_buffer.push(&ComputeBoundsParams {
            resolution: UVec2::splat(COMPUTE_BOUNDS_RESOLUTION),
        });
        params_buffer.write_buffer(device, queue);

        let mut output_buffer = DynamicBuffer::new(
            Some("compute_bounds_output_buffer".into()),
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        );
        output_buffer.push(&OutputBounds::empty());
        output_buffer.write_buffer(device, queue);

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("compute_bounds_bind_group"),
            layout: &self.layout,
            entries: BindGroupEntries::sequential((
                settings_buffer.binding().unwrap(),
                params_buffer.binding().unwrap(),
                output_buffer.binding().unwrap(),
            ))
            .as_ref(),
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("compute_bounds_pass"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                COMPUTE_BOUNDS_RESOLUTION.div_ceil(16),
                COMPUTE_BOUNDS_RESOLUTION.div_ceil(16),
                1,
            );
        }

        let readback_buffer = create_readback_buffer_and_schedule_copy_buffer(
            device,
            &mut encoder,
            output_buffer.inner_buffer().unwrap(),
        );
        let readback =
            readback_buffer_on_submit_async::<OutputBounds, _>(&mut encoder, &readback_buffer, ..);
        queue.submit([encoder.finish()]);
        readback
    }
}

#[derive(ShaderType)]
struct ComputeBoundsParams {
    resolution: UVec2,
}

#[derive(Debug, Clone, Copy, ShaderType)]
pub struct OutputBounds {
    x_min: u32,
    y_min: u32,
    x_max: u32,
    y_max: u32,
}

impl OutputBounds {
    fn empty() -> Self {
        Self {
            x_min: COMPUTE_BOUNDS_RESOLUTION - 1,
            y_min: COMPUTE_BOUNDS_RESOLUTION - 1,
            x_max: 0,
            y_max: 0,
        }
    }

    pub fn normalized_ranges(self) -> Option<(Vec2, Vec2)> {
        if self.x_min > self.x_max || self.y_min > self.y_max {
            return None;
        }

        let max_index = COMPUTE_BOUNDS_RESOLUTION - 1;
        let x_min = self.x_min.saturating_sub(COMPUTE_BOUNDS_PADDING);
        let y_min = self.y_min.saturating_sub(COMPUTE_BOUNDS_PADDING);
        let x_max = self
            .x_max
            .saturating_add(COMPUTE_BOUNDS_PADDING)
            .min(max_index);
        let y_max = self
            .y_max
            .saturating_add(COMPUTE_BOUNDS_PADDING)
            .min(max_index);
        let scale = 1.0 / max_index as f32;
        Some((
            Vec2::new(x_min as f32, x_max as f32) * scale,
            Vec2::new(y_min as f32, y_max as f32) * scale,
        ))
    }
}

pub struct GradientPipeline {
    pub(crate) pipeline: RenderPipeline,
}

impl GradientPipeline {
    pub fn new(
        device: &Device,
        layout: &BindGroupLayout,
        profile: &ColorProfile,
        output_profile: &ColorProfile,
        format: TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("gradient_shader"),
            source: ShaderSource::Wgsl(
                compile_gradient_shader(profile, output_profile, format.is_srgb())
                    .unwrap()
                    .into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("gradient_pipeline_layout"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("gradient_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[VertexBufferLayout {
                    array_stride: 16,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                }],
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
            depth_stencil: Default::default(),
            multisample: Default::default(),
            multiview_mask: None,
            cache: Default::default(),
        });

        Self { pipeline }
    }
}

pub struct GradientRingPipeline {
    pub(crate) pipeline: RenderPipeline,
    pub(crate) mesh: GradientMesh,
}

impl GradientRingPipeline {
    pub fn new(
        device: &Device,
        layout: &BindGroupLayout,
        profile: &ColorProfile,
        output_profile: &ColorProfile,
        format: TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("gradient_ring_shader"),
            source: ShaderSource::Wgsl(
                compile_gradient_shader(profile, output_profile, format.is_srgb())
                    .unwrap()
                    .into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("gradient_ring_pipeline_layout"),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("gradient_ring_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("ring_vertex"),
                compilation_options: Default::default(),
                buffers: &[VertexBufferLayout {
                    array_stride: 16,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("ring_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: Default::default(),
            multisample: Default::default(),
            multiview_mask: None,
            cache: Default::default(),
        });
        let mesh = GradientMesh::new_plane(device, GradientPlaneShape::Square, 1.0);

        Self { pipeline, mesh }
    }
}

#[derive(Pod, Zeroable, Clone, Copy)]
#[repr(C)]
struct Vertex {
    position: Vec2,
    uv: Vec2,
}

#[derive(Debug)]
pub struct GradientMesh {
    pub(crate) n_indices: u32,
    pub(crate) vertices: Buffer,
    pub(crate) indices: Buffer,
}

impl GradientMesh {
    pub fn new_bar(device: &Device) -> Self {
        Self::from_vertices(
            device,
            vec![
                vtx(-1.0, -1.0, 0.0, 0.0),
                vtx(1.0, -1.0, 1.0, 0.0),
                vtx(1.0, 1.0, 1.0, 0.0),
                vtx(-1.0, 1.0, 0.0, 0.0),
            ],
            vec![0, 1, 2, 2, 3, 0],
        )
    }

    pub fn new_plane(device: &Device, shape: GradientPlaneShape, scale: f32) -> Self {
        let (mut vertices, indices) = match shape {
            GradientPlaneShape::Square => (
                vec![
                    vtx(-1.0, -1.0, 0.0, 0.0),
                    vtx(1.0, -1.0, 1.0, 0.0),
                    vtx(1.0, 1.0, 1.0, 1.0),
                    vtx(-1.0, 1.0, 0.0, 1.0),
                ],
                vec![0, 1, 2, 2, 3, 0],
            ),
            GradientPlaneShape::Triangle => (
                vec![
                    vtx(0.0, 1.0, 0.5, 0.0),
                    vtx(-3.0_f32.sqrt() * 0.5, -0.5, 0.0, 1.0),
                    vtx(3.0_f32.sqrt() * 0.5, -0.5, 1.0, 1.0),
                ],
                vec![0, 1, 2],
            ),
        };

        for vertex in &mut vertices {
            vertex.position *= scale;
        }
        Self::from_vertices(device, vertices, indices)
    }

    fn from_vertices(device: &Device, vertices: Vec<Vertex>, indices: Vec<u16>) -> Self {
        Self {
            n_indices: indices.len() as u32,
            vertices: device.create_buffer_init(&BufferInitDescriptor {
                label: Some("gradient_vertex_buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&BufferInitDescriptor {
                label: Some("gradient_index_buffer"),
                contents: bytemuck::cast_slice(&indices),
                usage: BufferUsages::INDEX,
            }),
        }
    }
}

#[inline]
fn vtx(px: f32, py: f32, uvx: f32, uvy: f32) -> Vertex {
    Vertex {
        position: Vec2::new(px, py),
        uv: Vec2::new(uvx, uvy),
    }
}

#[derive(Debug, Clone, ShaderType)]
pub struct GradientSettings {
    pub channel_ranges: [Vec4; 3],
    pub base_channels: Vec3,
    pub out_of_gamut_color: Vec3,
    pub use_out_of_gamut_color: u32,
    pub color_model: u32,
    pub variable_channels: u32,
    pub plane_shape: u32,
    pub rotation: f32,
    pub flip_axis: u32,
    pub primary_channel: u32,
    pub ring_rotation: f32,
    pub reversed_ring: u32,
    pub saturate_primary_channel: u32,
    pub ring_width: f32,
    pub texture_size: f32,
    pub x_range: Vec2,
    pub y_range: Vec2,
}

impl GradientSettings {
    pub fn new_plane(
        out_of_gamut_color: Rgb,
        use_out_of_gamut_color: bool,
        base_channels: Vec3,
        config: &GradientPlaneConfig,
        primary_channel_override: Option<u8>,
        texture_size: f32,
    ) -> Self {
        let variable_channels = primary_channel_override
            .map_or(config.variable_channels, |channel| 0b111 & !(1 << channel));
        let primary_channel = (0..3)
            .find(|channel| variable_channels & (1 << channel) == 0)
            .unwrap_or(0);

        Self {
            channel_ranges: config
                .model
                .channel_ranges()
                .map(|range| Vec4::new(range.x, range.y, 0.0, 0.0)),
            base_channels,
            out_of_gamut_color: Vec3::new(
                out_of_gamut_color.r,
                out_of_gamut_color.g,
                out_of_gamut_color.b,
            ),
            use_out_of_gamut_color: u32::from(use_out_of_gamut_color),
            color_model: config.model as u32,
            variable_channels: u32::from(variable_channels),
            plane_shape: config.shape as u32,
            rotation: config.rotation,
            flip_axis: u32::from(config.flip_axis.bits()),
            primary_channel,
            ring_rotation: config.ring_rotation,
            reversed_ring: u32::from(config.reversed_ring),
            saturate_primary_channel: u32::from(
                config.ring_bar_saturated_hue_channel
                    && config.model.hue_channel() == Some(primary_channel as u8),
            ),
            ring_width: config.primary_channel_ring_width,
            texture_size,
            x_range: Vec2::new(0.0, 1.0),
            y_range: Vec2::new(0.0, 1.0),
        }
    }

    pub fn new_bar(
        out_of_gamut_color: Rgb,
        use_out_of_gamut_color: bool,
        base_channels: Vec3,
        config: &GradientBarConfig,
        saturate_primary_channel: bool,
    ) -> Self {
        Self {
            channel_ranges: config
                .model
                .channel_ranges()
                .map(|range| Vec4::new(range.x, range.y, 0.0, 0.0)),
            base_channels,
            out_of_gamut_color: Vec3::new(
                out_of_gamut_color.r,
                out_of_gamut_color.g,
                out_of_gamut_color.b,
            ),
            use_out_of_gamut_color: u32::from(use_out_of_gamut_color),
            color_model: config.model as u32,
            variable_channels: 1 << config.channel,
            plane_shape: GradientPlaneShape::Square as u32,
            rotation: 0.0,
            flip_axis: 0,
            primary_channel: 0,
            ring_rotation: 0.0,
            reversed_ring: 0,
            saturate_primary_channel: u32::from(saturate_primary_channel),
            ring_width: 0.0,
            texture_size: 1.0,
            x_range: Vec2::new(0.0, 1.0),
            y_range: Vec2::new(0.0, 1.0),
        }
    }
}
