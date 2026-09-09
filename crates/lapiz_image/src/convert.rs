use anyhow::Result;
use lapiz_color::shader::IccTransformShader;
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    wesl_jit::compile_wesl,
};
use moxcms::{ColorProfile, Layout, TransformOptions};
use wgpu::{
    BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, ComputePassDescriptor,
    ComputePipeline, ComputePipelineDescriptor, Device, PipelineLayoutDescriptor, Queue,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StorageTextureAccess, TextureView,
};

use crate::{texel::TexelType, tile::GpuTileStorage};

pub struct ColorProfileConvertPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl ColorProfileConvertPipeline {
    pub fn new(
        device: &Device,
        texel_type: TexelType,
        src_pr: &ColorProfile,
        src_layout: Layout,
        dst_pr: &ColorProfile,
        dst_layout: Layout,
        options: TransformOptions,
    ) -> Result<Self> {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("color_profile_convert_layout"),
            entries: &BindGroupLayoutEntries::single(
                ShaderStages::COMPUTE,
                binding_types::texture_storage_2d_array(
                    texel_type.wgpu_format(),
                    StorageTextureAccess::ReadWrite,
                ),
            ),
        });

        let icc_transform = IccTransformShader::new(
            "transform_color",
            src_pr,
            src_layout,
            dst_pr,
            dst_layout,
            options,
        )?;
        let shader = include_str!("color_space_convert.wesl")
            .replace("//CODEGEN_FLAG_TRANSFORM_COLOR", &icc_transform.function);

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("color_space_convert_shader"),
            source: ShaderSource::Wgsl(compile_wesl(shader, &[&crate::image::PACKAGE])?.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("color_space_convert_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("color_space_convert_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Ok(Self { layout, pipeline })
    }

    pub fn convert(&self, device: &Device, queue: &Queue, tiles: &TextureView) {
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("color_space_convert_bind_group"),
            layout: &self.layout,
            entries: &BindGroupEntries::single(tiles),
        });

        let tile_count = tiles.texture().depth_or_array_layers();

        let mut ec = device.create_command_encoder(&Default::default());

        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("color_space_convert_pass"),
                ..Default::default()
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                tile_count,
            );
        }

        queue.submit([ec.finish()]);
    }
}
