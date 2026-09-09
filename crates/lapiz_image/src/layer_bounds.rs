use glam::Vec4;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{DynamicBindGroupLayoutEntries, binding_types},
    wesl_jit,
};
use wgpu::{
    BindGroupLayout, BindGroupLayoutDescriptor, Buffer, BufferUsages, CommandEncoder,
    ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor, Device,
    PipelineLayoutDescriptor, Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StorageTextureAccess,
    util::{BufferInitDescriptor, DeviceExt},
};

use crate::{
    scan_pixels::ScanPixelsPipeline,
    texel::TexelType,
    tile::{GpuTileInfo, GpuTileStorage, LayerBinding},
};

pub struct LayerBoundsPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
    scan_pipeline: ScanPixelsPipeline,
}

impl LayerBoundsPipeline {
    pub fn new(device: &Device, format: TexelType, with_selection: bool) -> Self {
        let shader = wesl_jit::compile_wesl_with_config(
            include_str!("layer_bounds.wesl").into(),
            &[&crate::image::PACKAGE],
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
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                binding_types::storage_buffer::<Vec4>(false),
            ),
        );
        if with_selection {
            entries = entries.extend_sequential((
                binding_types::storage_buffer_read_only::<u32>(false),
                binding_types::texture_storage_2d_array(
                    wgpu::TextureFormat::R8Unorm,
                    StorageTextureAccess::ReadOnly,
                ),
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
            ));
        }

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("layer bounds bind group layout"),
            entries: entries.as_ref(),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("layer bounds pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });

        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("layer bounds shader module"),
            source: ShaderSource::Wgsl(shader.into()),
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("layer bounds pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            layout,
            pipeline,
            scan_pipeline: ScanPixelsPipeline::new(device, TexelType::A8),
        }
    }

    pub fn dispatch(
        &self,
        device: &Device,
        queue: &Queue,
        ec: &mut CommandEncoder,
        layer: &LayerBinding,
        selection: Option<&LayerBinding>,
    ) -> Buffer {
        let result_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("layer bounds result"),
            contents: bytemuck::bytes_of(&[
                i32::MAX as u32,
                i32::MAX as u32,
                i32::MIN as u32,
                i32::MIN as u32,
            ]),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        });

        let has_selection = selection.as_ref().map(|selection_binding| {
            self.scan_pipeline
                .scan_to_binary_buffer(device, queue, selection_binding)
        });

        let mut entries = DynamicBindGroupEntries::sequential((
            &layer.texture,
            layer.tile_info_buffer.as_entire_binding(),
            result_buffer.as_entire_binding(),
        ));

        if let (Some(selection_binding), Some(has_selection)) =
            (selection.as_ref(), has_selection.as_ref())
        {
            entries = entries.extend_sequential((
                has_selection.as_entire_binding(),
                &selection_binding.texture,
                selection_binding.tile_info_buffer.as_entire_binding(),
            ));
        }

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer bounds bind group"),
            layout: &self.layout,
            entries: entries.as_ref(),
        });

        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("layer bounds pass"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                layer.texture.texture().depth_or_array_layers(),
            );
        }

        result_buffer
    }
}
