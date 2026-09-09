use encase::ShaderType;
use lapiz_image::tile::{GpuTileInfo, LayerBinding};
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
};
use wesl::include_wesl;
use wgpu::{
    BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, BufferUsages,
    ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor, Device,
    PipelineLayoutDescriptor, Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StorageTextureAccess, TextureFormat,
};

#[derive(ShaderType, Debug, Clone, Copy)]
pub struct FxaaParams {
    pub edge_threshold_min: f32,
    pub edge_threshold_max: f32,
    pub iterations: u32,
    pub subpixel_quality: f32,
}

impl Default for FxaaParams {
    fn default() -> Self {
        Self::HIGH
    }
}

impl FxaaParams {
    pub const LOW: Self = Self {
        edge_threshold_min: 0.0833,
        edge_threshold_max: 0.250,
        iterations: 12,
        subpixel_quality: 0.75,
    };

    pub const MEDIUM: Self = Self {
        edge_threshold_min: 0.0625,
        edge_threshold_max: 0.166,
        iterations: 12,
        subpixel_quality: 0.75,
    };

    pub const HIGH: Self = Self {
        edge_threshold_min: 0.0312,
        edge_threshold_max: 0.125,
        iterations: 12,
        subpixel_quality: 0.75,
    };

    pub const ULTRA: Self = Self {
        edge_threshold_min: 0.0156,
        edge_threshold_max: 0.063,
        iterations: 12,
        subpixel_quality: 0.75,
    };

    pub const EXTREME: Self = Self {
        edge_threshold_min: 0.0078,
        edge_threshold_max: 0.031,
        iterations: 12,
        subpixel_quality: 0.75,
    };
}

pub struct FxaaPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl FxaaPipeline {
    pub fn new(device: &Device, texture_format: TextureFormat) -> Self {
        let fxaa_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("fxaa_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::texture_storage_2d_array(
                        texture_format,
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::texture_storage_2d_array(
                        texture_format,
                        StorageTextureAccess::WriteOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::uniform_buffer::<FxaaParams>(false),
                ),
            ),
        });

        let fxaa_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("fxaa_pipeline_layout"),
            bind_group_layouts: &[Some(&fxaa_layout)],
            ..Default::default()
        });
        let fxaa_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("fxaa_shader"),
            source: ShaderSource::Wgsl(include_wesl!("fxaa").into()),
        });
        let fxaa_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("fxaa_pipeline"),
            layout: Some(&fxaa_pipeline_layout),
            module: &fxaa_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            layout: fxaa_layout,
            pipeline: fxaa_pipeline,
        }
    }

    pub fn dispatch(
        &self,
        device: &Device,
        queue: &Queue,
        params: &FxaaParams,
        src: LayerBinding,
        dst: LayerBinding,
    ) {
        let mut fxaa_params_buffer =
            DynamicBuffer::new(Some("fxaa_params_buffer".into()), BufferUsages::UNIFORM);
        fxaa_params_buffer.push(params);
        fxaa_params_buffer.write_buffer(device, queue);

        let fxaa_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("fxaa_bind_group"),
            layout: &self.layout,
            entries: &BindGroupEntries::sequential((
                &src.texture,
                dst.tile_info_buffer.as_entire_binding(),
                &dst.texture,
                dst.tile_info_buffer.as_entire_binding(),
                fxaa_params_buffer.binding().unwrap(),
            )),
        });

        let mut ec = device.create_command_encoder(&Default::default());
        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("fxaa_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &fxaa_bind_group, &[]);
            pass.dispatch_workgroups(
                dst.texture.texture().width().div_ceil(16),
                dst.texture.texture().height().div_ceil(16),
                dst.texture.texture().depth_or_array_layers(),
            );
        }
        queue.submit([ec.finish()]);
    }
}
