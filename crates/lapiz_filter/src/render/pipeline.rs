use std::borrow::Cow;

use bevy_math::URect;
use lapiz_image::tile::{DynamicLayerStorage, GpuTileInfo, GpuTileStorage, LayerBinding};
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{DynamicBindGroupLayoutEntries, binding_types},
};
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindingResource, Buffer, ComputePass, ComputePipeline, ComputePipelineDescriptor, Device,
    PipelineLayoutDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StorageTextureAccess, TextureSampleType,
};

use crate::render::FilterResources;

fn filter_common_layout_entries(resources: &FilterResources) -> DynamicBindGroupLayoutEntries {
    DynamicBindGroupLayoutEntries::new_with_indices(
        ShaderStages::COMPUTE,
        (
            (
                0,
                binding_types::texture_storage_2d_array(
                    resources.target_layer_format.wgpu_format(),
                    StorageTextureAccess::ReadOnly,
                ),
            ),
            (
                1,
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
            ),
            (
                4,
                binding_types::texture_storage_2d_array(
                    resources.selection_layer_format.wgpu_format(),
                    StorageTextureAccess::ReadOnly,
                ),
            ),
            (
                5,
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
            ),
            (6, binding_types::storage_buffer_read_only::<u32>(false)),
            (7, binding_types::storage_buffer_read_only::<URect>(false)),
            (
                11,
                binding_types::texture_2d(TextureSampleType::Float { filterable: false }),
            ),
            (12, binding_types::storage_buffer_read_only::<URect>(false)),
        ),
    )
}

fn filter_common_entries<'a>(
    resources: &'a FilterResources,
    input: &'a LayerBinding,
    selection: &'a LayerBinding,
    has_selection: &'a Buffer,
    bounds_input: &'a Buffer,
) -> Vec<BindGroupEntry<'a>> {
    let mut entries = DynamicBindGroupEntries::new_with_indices((
        (0, BindingResource::TextureView(&input.texture)),
        (1, input.tile_info_buffer.as_entire_binding()),
        (4, BindingResource::TextureView(&selection.texture)),
        (5, selection.tile_info_buffer.as_entire_binding()),
        (6, has_selection.as_entire_binding()),
        (7, bounds_input.as_entire_binding()),
        (
            11,
            BindingResource::TextureView(resources.texture_atlas.texture_view()),
        ),
        (12, resources.texture_atlas.atlas_bounds_buffer_binding()),
    ))
    .to_vec();
    entries.extend(resources.external_var_bindings());
    entries
}

#[derive(Clone)]
pub struct FilterMainPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl FilterMainPipeline {
    pub fn new(
        device: &Device,
        resources: &FilterResources,
        compiled_shader: Cow<'_, str>,
    ) -> Self {
        let layout_entries = filter_common_layout_entries(resources).extend_with_indices((
            (
                2,
                binding_types::texture_storage_2d_array(
                    resources.target_layer_format.wgpu_format(),
                    StorageTextureAccess::WriteOnly,
                ),
            ),
            (
                3,
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
            ),
            (
                9,
                binding_types::texture_storage_2d_array(
                    resources.target_layer_format.wgpu_format(),
                    StorageTextureAccess::ReadOnly,
                ),
            ),
            (
                10,
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
            ),
        ));

        let mut layout_entries = layout_entries.to_vec();
        layout_entries.extend(resources.external_var_layouts.clone());
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("filter main layout"),
            entries: &layout_entries,
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("filter main shader"),
            source: ShaderSource::Wgsl(compiled_shader),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("filter main pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("filter main pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("filter_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self { layout, pipeline }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dispatch(
        &self,
        device: &Device,
        pass: &mut ComputePass,
        input: &LayerBinding,
        output: &DynamicLayerStorage,
        selection: &LayerBinding,
        has_selection: &Buffer,
        bounds_input: &Buffer,
        original: &LayerBinding,
        resources: &FilterResources,
    ) {
        let mut entries =
            filter_common_entries(resources, input, selection, has_selection, bounds_input);
        entries.extend(
            DynamicBindGroupEntries::new_with_indices((
                (
                    2,
                    BindingResource::TextureView(output.texture_view().unwrap()),
                ),
                (3, output.tile_info_buffer().unwrap().as_entire_binding()),
                (9, BindingResource::TextureView(&original.texture)),
                (10, original.tile_info_buffer.as_entire_binding()),
            ))
            .to_vec(),
        );

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("filter main bind group"),
            layout: &self.layout,
            entries: &entries,
        });

        let n_tiles = output
            .texture_view()
            .unwrap()
            .texture()
            .depth_or_array_layers();

        pass.push_debug_group("filter main");
        {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                n_tiles,
            );
        }
        pass.pop_debug_group();
    }
}

#[derive(Clone)]
pub struct FilterBoundsEvalPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl FilterBoundsEvalPipeline {
    pub fn new(
        device: &Device,
        resources: &FilterResources,
        compiled_shader: Cow<'_, str>,
    ) -> Self {
        let layout_entries = filter_common_layout_entries(resources)
            .extend_with_indices(((8, binding_types::storage_buffer::<URect>(false)),));
        let mut layout_entries = layout_entries.to_vec();
        layout_entries.extend(resources.external_var_layouts.clone());

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("filter bounds eval layout"),
            entries: &layout_entries,
        });
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("filter bounds eval shader"),
            source: ShaderSource::Wgsl(compiled_shader),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("filter bounds eval pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("filter bounds eval pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("filter_bounds_eval"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self { pipeline, layout }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn dispatch(
        &self,
        device: &Device,
        pass: &mut ComputePass,
        input: &LayerBinding,
        selection: &LayerBinding,
        has_selection: &Buffer,
        bounds_input: &Buffer,
        bounds_output: &Buffer,
        resources: &FilterResources,
    ) {
        let mut entries =
            filter_common_entries(resources, input, selection, has_selection, bounds_input);
        entries.extend(
            DynamicBindGroupEntries::new_with_indices(((8, bounds_output.as_entire_binding()),))
                .to_vec(),
        );

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("filter bounds eval bind group"),
            layout: &self.layout,
            entries: &entries,
        });

        pass.push_debug_group("filter bounds eval");
        {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        pass.pop_debug_group();
    }
}
