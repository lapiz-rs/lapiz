use glam::UVec3;
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    wesl_jit,
};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, ComputePass,
    ComputePipeline, ComputePipelineDescriptor, Device, PipelineLayoutDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StorageTextureAccess,
};

use crate::{
    texel::TexelType,
    tile::{GpuTileInfo, GpuTileStorage, LayerBinding},
};

pub struct PreparedCopyLayerPipeline {
    bind_group: BindGroup,
    workgroup_count: UVec3,
}

pub struct CopyLayerPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl CopyLayerPipeline {
    pub fn new(device: &Device, format: TexelType) -> Self {
        let shader = wesl_jit::compile_wesl_with_config(
            include_str!("copy_layer.wesl").into(),
            &[&crate::image::PACKAGE],
            |compiler| {
                compiler.set_feature(format.shader_def(), true);
            },
        )
        .unwrap();

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("copy layer bind group layout"),
            entries: &BindGroupLayoutEntries::sequential(
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
                ),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("copy layer pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });

        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("copy layer shader module"),
            source: ShaderSource::Wgsl(shader.into()),
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("copy layer pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self { layout, pipeline }
    }

    #[must_use]
    pub fn prepare(
        &self,
        device: &Device,
        src_layer: &LayerBinding,
        dst_layer: &LayerBinding,
    ) -> PreparedCopyLayerPipeline {
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("copy layer bind group"),
            layout: &self.layout,
            entries: &BindGroupEntries::sequential((
                &src_layer.texture,
                &dst_layer.texture,
                src_layer.tile_info_buffer.as_entire_binding(),
                dst_layer.tile_info_buffer.as_entire_binding(),
            )),
        });

        let workgroup_count = UVec3::new(
            GpuTileStorage::TILE_SIZE.div_ceil(16),
            GpuTileStorage::TILE_SIZE.div_ceil(16),
            src_layer.texture.texture().depth_or_array_layers(),
        );

        PreparedCopyLayerPipeline {
            bind_group,
            workgroup_count,
        }
    }

    // dst layer must ensure that the texture is already contains src layer
    pub fn dispatch(&self, pass: &mut ComputePass, prepared: &PreparedCopyLayerPipeline) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &prepared.bind_group, &[]);
        pass.dispatch_workgroups(
            prepared.workgroup_count.x,
            prepared.workgroup_count.y,
            prepared.workgroup_count.z,
        );
    }
}
