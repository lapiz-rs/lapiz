use encase::ShaderType;
use glam::{IVec2, UVec2, Vec4};
use indexmap::IndexSet;
use lapiz_anti_aliasing::fxaa::{FxaaParams, FxaaPipeline};
use lapiz_image::{
    composite::BlendFunction,
    scan_pixels::ScanPixelsPipeline,
    texel::{TexelFormat, TexelType},
    tile::{DynamicLayerStorage, GpuLayerInfo, GpuTileInfo, GpuTileStorage, LayerBinding},
};
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    readback::{create_readback_buffer_and_schedule_copy_buffer, readback_buffer_on_submit_async},
    util::DevicePollExt,
    wesl_jit,
};
use tracing::{error, info};
use wesl::include_wesl;
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindingResource, BufferUsages, ComputePassDescriptor, ComputePipeline,
    ComputePipelineDescriptor, Device, Extent3d, Origin3d, PipelineLayoutDescriptor, Queue,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StorageTextureAccess, TexelCopyTextureInfo,
    TextureAspect, TextureDimension, TextureFormat, TextureUsages, TextureViewDimension,
    wgt::{BufferDescriptor, TextureDescriptor, TextureViewDescriptor},
};

#[derive(Debug, Clone, Copy)]
pub enum BucketAntialiasApproach {
    None,
    Fxaa,
    Feather(u32),
}

#[derive(Debug, Clone, Copy)]
pub struct BucketParams {
    pub seed: UVec2,
    pub fill_color: Vec4,
    pub threshold: f32,
    pub alpha_threshold: f32,
    pub contiguous: bool,
    pub close_gap: u32,
    pub grow: i32,
    pub aa_approach: BucketAntialiasApproach,
    pub image_size: UVec2,
}

#[derive(ShaderType, Debug, Clone, Copy)]
struct BucketParamsInner {
    pub seed: UVec2,
    pub fill_color: Vec4,
    pub threshold: f32,
    pub alpha_threshold: f32,
    pub close_gap: u32,
    pub grow: i32,
    pub feather: u32,
    pub image_size: UVec2,
    pub transparent_mode: u32,
}

#[derive(ShaderType, Debug, Clone, Copy)]
pub struct JumpParams {
    pub jump: u32,
}

struct BucketResultInternal {
    bucket_params_buffer: DynamicBuffer<BucketParamsInner>,
    mask: DynamicLayerStorage,
}

pub struct Bucket {
    seed_mode_layout: BindGroupLayout,
    seed_mode_pipeline: ComputePipeline,
    thresholding_layout: BindGroupLayout,
    thresholding_pipeline: ComputePipeline,
    close_gap_resolve_pipeline: ComputePipeline,
    ccl_layout: BindGroupLayout,
    ccl_init_pipeline: ComputePipeline,
    ccl_merge_pipeline: ComputePipeline,
    ccl_compress_pipeline: ComputePipeline,
    ccl_extract_pipeline: ComputePipeline,
    grow_layout: BindGroupLayout,
    grow_pipeline: ComputePipeline,
    fxaa_pipeline: FxaaPipeline,
    close_gap_and_feather_layout: BindGroupLayout,
    close_gap_and_feather_seed_pipeline: ComputePipeline,
    close_gap_and_feather_jump_pipeline: ComputePipeline,
    feather_resolve_pipeline: ComputePipeline,
    composite_layout: BindGroupLayout,
    composite_pipeline: Option<ComputePipeline>,
    scan_pixels_pipeline: ScanPixelsPipeline,

    output_layer_format: TexelType,
    mask_format: TexelType,
}

impl std::fmt::Debug for Bucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bucket").finish()
    }
}

impl Bucket {
    pub fn new(device: &Device, ref_texel_type: TexelType, output_texel_type: TexelType) -> Self {
        let mask_format = TexelType {
            format: TexelFormat::Alpha,
            depth: ref_texel_type.depth,
        };

        let mask_texture_format = mask_format.wgpu_format();

        let seed_mode_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("seed_mode_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::uniform_buffer::<BucketParamsInner>(false),
                    binding_types::texture_storage_2d_array(
                        ref_texel_type.wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer::<u32>(false),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                ),
            ),
        });

        let seed_mode_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("seed_mode_pipeline_layout"),
            bind_group_layouts: &[Some(&seed_mode_layout)],
            ..Default::default()
        });
        let seed_mode_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("seed_mode_shader_module"),
            source: ShaderSource::Wgsl(include_wesl!("seed_mode").into()),
        });
        let seed_mode_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("seed_mode_pipeline"),
            layout: Some(&seed_mode_pipeline_layout),
            module: &seed_mode_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let thresholding_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("thresholding_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::uniform_buffer::<BucketParamsInner>(false),
                    binding_types::texture_storage_2d_array(
                        ref_texel_type.wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::texture_storage_2d_array(
                        mask_texture_format,
                        StorageTextureAccess::WriteOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                ),
            ),
        });

        let thresholding_pipeline_layout =
            device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("thresholding_pipeline_layout"),
                bind_group_layouts: &[Some(&thresholding_layout)],
                ..Default::default()
            });
        let thresholding_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("thresholding_shader_module"),
            source: ShaderSource::Wgsl(include_wesl!("thresholding").into()),
        });
        let thresholding_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("thresholding_pipeline"),
            layout: Some(&thresholding_pipeline_layout),
            module: &thresholding_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let ccl_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("ccl_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::uniform_buffer::<BucketParamsInner>(false),
                    binding_types::texture_storage_2d_array(
                        mask_texture_format,
                        StorageTextureAccess::ReadWrite,
                    ),
                    binding_types::storage_buffer::<u32>(false),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                ),
            ),
        });

        let ccl_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("ccl_pipeline_layout"),
            bind_group_layouts: &[Some(&ccl_layout)],
            ..Default::default()
        });

        let ccl_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("ccl_shader"),
            source: ShaderSource::Wgsl(include_wesl!("ccl").into()),
        });

        let ccl_init_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("ccl_init_pipeline"),
            layout: Some(&ccl_pipeline_layout),
            module: &ccl_shader,
            entry_point: Some("ccl_init"),
            compilation_options: Default::default(),
            cache: None,
        });

        let ccl_merge_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("ccl_merge_pipeline"),
            layout: Some(&ccl_pipeline_layout),
            module: &ccl_shader,
            entry_point: Some("ccl_merge"),
            compilation_options: Default::default(),
            cache: None,
        });

        let ccl_compress_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("ccl_compress_pipeline"),
            layout: Some(&ccl_pipeline_layout),
            module: &ccl_shader,
            entry_point: Some("ccl_compress"),
            compilation_options: Default::default(),
            cache: None,
        });

        let ccl_extract_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("ccl_extract_pipeline"),
            layout: Some(&ccl_pipeline_layout),
            module: &ccl_shader,
            entry_point: Some("ccl_extract"),
            compilation_options: Default::default(),
            cache: None,
        });

        let grow_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("grow_shader"),
            source: ShaderSource::Wgsl(include_wesl!("grow").into()),
        });

        let grow_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("grow_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::uniform_buffer::<BucketParamsInner>(false),
                    binding_types::texture_storage_2d_array(
                        mask_texture_format,
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::texture_storage_2d_array(
                        mask_texture_format,
                        StorageTextureAccess::WriteOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                ),
            ),
        });

        let grow_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("grow_pipeline_layout"),
            bind_group_layouts: &[Some(&grow_layout)],
            ..Default::default()
        });

        let grow_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("grow_pipeline"),
            layout: Some(&grow_pipeline_layout),
            module: &grow_shader,
            entry_point: Some("grow"),
            compilation_options: Default::default(),
            cache: None,
        });

        let fxaa_pipeline = FxaaPipeline::new(device, mask_texture_format);

        let close_gap_and_feather_layout =
            device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("close_gap_and_feather_layout"),
                entries: &BindGroupLayoutEntries::sequential(
                    ShaderStages::COMPUTE,
                    (
                        binding_types::uniform_buffer::<BucketParamsInner>(false),
                        binding_types::texture_storage_2d_array(
                            TextureFormat::R8Unorm,
                            StorageTextureAccess::ReadWrite,
                        ),
                        binding_types::texture_storage_2d_array(
                            TextureFormat::Rg32Float,
                            StorageTextureAccess::ReadOnly,
                        ),
                        binding_types::texture_storage_2d_array(
                            TextureFormat::Rg32Float,
                            StorageTextureAccess::WriteOnly,
                        ),
                        binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                        binding_types::uniform_buffer::<JumpParams>(true),
                    ),
                ),
            });

        let close_gap_and_feather_pipeline_layout =
            device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("close_gap_and_feather_pipeline_layout"),
                bind_group_layouts: &[Some(&close_gap_and_feather_layout)],
                ..Default::default()
            });
        let close_gap_and_feather_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("close_gap_and_feather_shader"),
            source: ShaderSource::Wgsl(include_wesl!("close_gap_and_feather").into()),
        });
        let close_gap_and_feather_seed_pipeline =
            device.create_compute_pipeline(&ComputePipelineDescriptor {
                label: Some("close_gap_and_feather_seed_pipeline"),
                layout: Some(&close_gap_and_feather_pipeline_layout),
                module: &close_gap_and_feather_shader,
                entry_point: Some("seed_edges"),
                compilation_options: Default::default(),
                cache: None,
            });
        let close_gap_and_feather_jump_pipeline =
            device.create_compute_pipeline(&ComputePipelineDescriptor {
                label: Some("close_gap_and_feather_jump_pipeline"),
                layout: Some(&close_gap_and_feather_pipeline_layout),
                module: &close_gap_and_feather_shader,
                entry_point: Some("jfa_jump"),
                compilation_options: Default::default(),
                cache: None,
            });
        let feather_resolve_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("feather_resolve_pipeline"),
            layout: Some(&close_gap_and_feather_pipeline_layout),
            module: &close_gap_and_feather_shader,
            entry_point: Some("feather_resolve_alpha"),
            compilation_options: Default::default(),
            cache: None,
        });
        let close_gap_resolve_pipeline =
            device.create_compute_pipeline(&ComputePipelineDescriptor {
                label: Some("close_gap_resolve_pipeline"),
                layout: Some(&close_gap_and_feather_pipeline_layout),
                module: &close_gap_and_feather_shader,
                entry_point: Some("close_gap_resolve"),
                compilation_options: Default::default(),
                cache: None,
            });

        let composite_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("composite_layout"),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::uniform_buffer::<BucketParamsInner>(false),
                    binding_types::texture_storage_2d_array(
                        TextureFormat::R8Unorm,
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::texture_storage_2d_array(
                        output_texel_type.wgpu_format(),
                        StorageTextureAccess::WriteOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::texture_storage_2d_array(
                        output_texel_type.wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::storage_buffer_read_only::<u32>(false),
                    binding_types::texture_storage_2d_array(
                        mask_texture_format,
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                ),
            ),
        });

        let scan_pixels_pipeline = ScanPixelsPipeline::new(device, mask_format);

        Self {
            seed_mode_layout,
            seed_mode_pipeline,
            thresholding_layout,
            thresholding_pipeline,
            close_gap_resolve_pipeline,
            ccl_layout,
            ccl_init_pipeline,
            ccl_merge_pipeline,
            ccl_compress_pipeline,
            ccl_extract_pipeline,
            grow_layout,
            grow_pipeline,
            fxaa_pipeline,
            close_gap_and_feather_layout,
            close_gap_and_feather_seed_pipeline,
            close_gap_and_feather_jump_pipeline,
            feather_resolve_pipeline,
            composite_layout,
            composite_pipeline: None,
            scan_pixels_pipeline,
            output_layer_format: output_texel_type,
            mask_format,
        }
    }

    pub fn set_blend_function(&mut self, device: &Device, blend_function: &dyn BlendFunction) {
        let composite_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("composite_pipeline_layout"),
            bind_group_layouts: &[Some(&self.composite_layout)],
            ..Default::default()
        });

        let shader = wesl_jit::compile_wesl_with_config_and_include(
            include_str!("composite.wesl").replace(
                "//CODEGEN_FLAG_BLEND_FUNCTION",
                &blend_function.wgsl_function_call("src", "dst"),
            ),
            &[&lapiz_image::image::PACKAGE, &lapiz_render::render::PACKAGE],
            |resolver| {
                resolver.add_module(
                    "package::types".parse().unwrap(),
                    include_str!("../shaders/types.wesl").into(),
                );
            },
            |_| {},
        )
        .unwrap();

        let composite_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("composite_shader"),
            source: ShaderSource::Wgsl(shader.into()),
        });

        let composite_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("composite_pipeline"),
            layout: Some(&composite_pipeline_layout),
            module: &composite_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        self.composite_pipeline = Some(composite_pipeline);
    }

    /// Generate a mask with parameters then blend it with the dst_layer, outputting the result
    /// with no empty tiles.
    #[tracing::instrument(skip_all)]
    pub fn dispatch_composite(
        &self,
        device: &Device,
        queue: &Queue,
        bucket_params: &BucketParams,
        ref_layer: &LayerBinding,
        ref_layer_tile_info: IndexSet<IVec2>,
        dst_layer: &LayerBinding,
        selection: &LayerBinding,
    ) -> Option<DynamicLayerStorage> {
        let Some(composite_pipeline) = &self.composite_pipeline else {
            error!("composite_pipeline is not initialized");
            return None;
        };

        unsafe { device.start_graphics_debugger_capture() };

        let BucketResultInternal {
            bucket_params_buffer,
            mask,
        } = self.dispatch_mask_internal(
            device,
            queue,
            bucket_params,
            ref_layer,
            ref_layer_tile_info,
        )?;

        unsafe { device.stop_graphics_debugger_capture() };

        let output_tile_indices = self.scan_pixels_pipeline.scan(device, queue, &mask);
        if output_tile_indices.is_empty() {
            return None;
        }

        let mut output_tiles = DynamicLayerStorage::new(
            device.clone(),
            queue.clone(),
            GpuLayerInfo {
                texel_type: self.output_layer_format,
            },
        );
        output_tiles.allocate_tiles_batch(output_tile_indices);

        let has_selection_buffer = self
            .scan_pixels_pipeline
            .scan_to_binary_buffer(device, queue, selection);

        let mut ec = device.create_command_encoder(&Default::default());

        let composite_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("composite_bind_group"),
            layout: &self.composite_layout,
            entries: &BindGroupEntries::sequential((
                bucket_params_buffer.binding().unwrap(),
                BindingResource::TextureView(mask.texture_view().unwrap()),
                mask.tile_info_buffer().unwrap().as_entire_binding(),
                BindingResource::TextureView(output_tiles.texture_view().unwrap()),
                output_tiles.tile_info_buffer().unwrap().as_entire_binding(),
                BindingResource::TextureView(&dst_layer.texture),
                dst_layer.tile_info_buffer.as_entire_binding(),
                has_selection_buffer.as_entire_binding(),
                BindingResource::TextureView(&selection.texture),
                selection.tile_info_buffer.as_entire_binding(),
            )),
        });

        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("composite_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(composite_pipeline);
            pass.set_bind_group(0, &composite_bind_group, &[]);
            pass.dispatch_workgroups(
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                GpuTileStorage::TILE_SIZE.div_ceil(16),
                mask.len() as u32,
            );
        }

        queue.submit([ec.finish()]);

        info!(
            "Filled {} tiles: {:?}",
            output_tiles.len(),
            output_tiles
                .iter_tiles()
                .map(|(i, _, _)| i)
                .collect::<Vec<_>>()
        );

        Some(output_tiles)
    }

    /// Generate a mask texture with parameters and shrink the mask to no empty tiles.
    #[tracing::instrument(skip_all)]
    pub fn dispatch_mask(
        &self,
        device: &Device,
        queue: &Queue,
        bucket_params: &BucketParams,
        ref_layer: &LayerBinding,
        ref_layer_tile_info: IndexSet<IVec2>,
    ) -> Option<DynamicLayerStorage> {
        let BucketResultInternal { mask, .. } = self.dispatch_mask_internal(
            device,
            queue,
            bucket_params,
            ref_layer,
            ref_layer_tile_info,
        )?;

        let output_tiles = self.scan_pixels_pipeline.scan(device, queue, &mask);
        if output_tiles.is_empty() {
            return None;
        }

        let mut output_texture = DynamicLayerStorage::new(
            device.clone(),
            queue.clone(),
            GpuLayerInfo {
                texel_type: self.mask_format,
            },
        );
        output_texture.allocate_tiles_batch(output_tiles);

        let mut ec = device.create_command_encoder(&Default::default());
        for (dst_layer, tile) in output_texture.iter_tile_indices().enumerate() {
            let src_layer = mask.get_tile_layer(tile).unwrap();
            ec.copy_texture_to_texture(
                TexelCopyTextureInfo {
                    texture: mask.texture_view().unwrap().texture(),
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: src_layer,
                    },
                    aspect: TextureAspect::All,
                },
                TexelCopyTextureInfo {
                    texture: output_texture.texture_view().unwrap().texture(),
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: dst_layer as u32,
                    },
                    aspect: TextureAspect::All,
                },
                GpuTileStorage::TILE_COPY_SIZE,
            );
        }
        queue.submit([ec.finish()]);

        Some(output_texture)
    }

    /// Generate a mask with parameters. Result can contain empty tiles.
    #[tracing::instrument(skip_all)]
    fn dispatch_mask_internal(
        &self,
        device: &Device,
        queue: &Queue,
        bucket_params: &BucketParams,
        ref_layer: &LayerBinding,
        ref_layer_tile_info: IndexSet<IVec2>,
    ) -> Option<BucketResultInternal> {
        let dispatch_xy = GpuTileStorage::TILE_SIZE.div_ceil(16);

        let mut inner_params = BucketParamsInner {
            seed: bucket_params.seed,
            fill_color: bucket_params.fill_color,
            threshold: bucket_params.threshold,
            alpha_threshold: bucket_params.alpha_threshold,
            close_gap: bucket_params.close_gap,
            grow: bucket_params.grow,
            feather: match bucket_params.aa_approach {
                BucketAntialiasApproach::Feather(f) => f,
                _ => 0,
            },
            image_size: bucket_params.image_size,
            transparent_mode: 0,
        };

        let mut seed_params_buffer = DynamicBuffer::new(
            Some("bucket_seed_params_buffer".into()),
            BufferUsages::UNIFORM,
        );
        seed_params_buffer.push(&inner_params);
        seed_params_buffer.write_buffer(device, queue);

        let seed_transparent_mode =
            self.classify_seed_mode(device, queue, &seed_params_buffer, ref_layer);
        inner_params.transparent_mode = u32::from(seed_transparent_mode);

        let mut bucket_params_buffer =
            DynamicBuffer::new(Some("bucket_params_buffer".into()), BufferUsages::UNIFORM);
        bucket_params_buffer.push(&inner_params);
        bucket_params_buffer.write_buffer(device, queue);

        let mask_tile_indices = if seed_transparent_mode {
            let image_tile_count = GpuTileStorage::calc_tile_count(inner_params.image_size);
            let mut mask_tile_indices =
                IndexSet::with_capacity(image_tile_count.element_product() as usize);

            for y in 0..image_tile_count.y {
                for x in 0..image_tile_count.x {
                    let index = IVec2::new(x as i32, y as i32);
                    mask_tile_indices.insert(index);
                }
            }

            mask_tile_indices
        } else {
            ref_layer_tile_info.clone()
        };

        if mask_tile_indices.is_empty() {
            return None;
        }

        let mut mask = DynamicLayerStorage::new(
            device.clone(),
            queue.clone(),
            GpuLayerInfo {
                texel_type: self.mask_format,
            },
        );
        mask.allocate_tiles_batch(mask_tile_indices);

        let mut ec = device.create_command_encoder(&Default::default());

        let thresholding_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("thresholding_bind_group"),
            layout: &self.thresholding_layout,
            entries: &BindGroupEntries::sequential((
                bucket_params_buffer.binding().unwrap(),
                BindingResource::TextureView(&ref_layer.texture),
                BindingResource::TextureView(mask.texture_view().unwrap()),
                ref_layer.tile_info_buffer.as_entire_binding(),
                mask.tile_info_buffer().unwrap().as_entire_binding(),
            )),
        });

        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("thresholding_pass"),
                timestamp_writes: None,
            });

            pass.set_pipeline(&self.thresholding_pipeline);
            pass.set_bind_group(0, &thresholding_bind_group, &[]);
            pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
        }

        queue.submit([ec.finish()]);

        let existing_mask_tiles = self.scan_pixels_pipeline.scan(device, queue, &mask);
        let grown_mask_tiles = existing_mask_tiles.into_iter().flat_map(|t| {
            (-1..=1).flat_map(move |dx| (-1..=1).map(move |dy| t + IVec2::new(dx, dy)))
        });
        mask.allocate_tiles_batch(grown_mask_tiles);

        let seed_texture_a_view = {
            let t = device.create_texture(&TextureDescriptor {
                label: Some("feather_seed_texture_a"),
                size: Extent3d {
                    width: GpuTileStorage::TILE_SIZE,
                    height: GpuTileStorage::TILE_SIZE,
                    depth_or_array_layers: mask.len() as u32,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rg32Float,
                usage: TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            t.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let seed_texture_b_view = {
            let seed_texture_b = device.create_texture(&TextureDescriptor {
                label: Some("feather_seed_texture_b"),
                size: Extent3d {
                    width: GpuTileStorage::TILE_SIZE,
                    height: GpuTileStorage::TILE_SIZE,
                    depth_or_array_layers: mask.len() as u32,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rg32Float,
                usage: TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            seed_texture_b.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            })
        };

        if inner_params.close_gap > 0 {
            let max_jump = inner_params.close_gap.next_power_of_two();
            let (jump_params_buffer, jump_params_offsets) =
                create_jfa_params(device, queue, max_jump, "close_gap_jump_params");
            let jump_iterations = jump_params_offsets.len();

            let common_entries = vec![
                BindGroupEntry {
                    binding: 0,
                    resource: bucket_params_buffer.binding().unwrap(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(mask.texture_view().unwrap()),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: mask.tile_info_buffer().unwrap().as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: jump_params_buffer.binding().unwrap(),
                },
            ];

            let bind_groups = {
                let a_to_b_entries = common_entries
                    .clone()
                    .into_iter()
                    .chain([
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::TextureView(&seed_texture_a_view),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: BindingResource::TextureView(&seed_texture_b_view),
                        },
                    ])
                    .collect::<Vec<_>>();
                let b_to_a_entries = common_entries
                    .into_iter()
                    .chain([
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::TextureView(&seed_texture_b_view),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: BindingResource::TextureView(&seed_texture_a_view),
                        },
                    ])
                    .collect::<Vec<_>>();

                let a_to_b_group = device.create_bind_group(&BindGroupDescriptor {
                    label: Some("close_gap_seed_a_to_b_bind_group"),
                    layout: &self.close_gap_and_feather_layout,
                    entries: &a_to_b_entries,
                });
                let b_to_a_group = device.create_bind_group(&BindGroupDescriptor {
                    label: Some("close_gap_seed_b_to_a_bind_group"),
                    layout: &self.close_gap_and_feather_layout,
                    entries: &b_to_a_entries,
                });

                [a_to_b_group, b_to_a_group]
            };

            let mut ec = device.create_command_encoder(&Default::default());
            {
                let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("close_gap_seed_edges_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.close_gap_and_feather_seed_pipeline);
                pass.set_bind_group(0, &bind_groups[1], &[jump_params_offsets[0]]);
                pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
            }

            {
                let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("close_gap_jfa_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.close_gap_and_feather_jump_pipeline);
                for i in 0..jump_iterations {
                    pass.set_bind_group(0, &bind_groups[i % 2], &[jump_params_offsets[i]]);
                    pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
                }
            }

            {
                let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("close_gap_resolve_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.close_gap_resolve_pipeline);
                pass.set_bind_group(
                    0,
                    &bind_groups[jump_iterations % 2],
                    &[jump_params_offsets[jump_iterations - 1]],
                );
                pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
            }

            queue.submit([ec.finish()]);
        }

        if bucket_params.contiguous {
            let total_pixels =
                mask.len() as u32 * GpuTileStorage::TILE_SIZE * GpuTileStorage::TILE_SIZE;

            let labels_buffer = device.create_buffer(&BufferDescriptor {
                label: Some("labels_buffer"),
                size: (total_pixels * 4) as u64,
                usage: BufferUsages::STORAGE,
                mapped_at_creation: false,
            });

            let ccl_bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("ccl_bind_group"),
                layout: &self.ccl_layout,
                entries: &BindGroupEntries::sequential((
                    bucket_params_buffer.binding().unwrap(),
                    BindingResource::TextureView(mask.texture_view().unwrap()),
                    labels_buffer.as_entire_binding(),
                    mask.tile_info_buffer().unwrap().as_entire_binding(),
                )),
            });

            let mut ec = device.create_command_encoder(&Default::default());

            {
                let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("ccl_pass"),
                    timestamp_writes: None,
                });

                pass.push_debug_group("ccl_init_pass");
                pass.set_pipeline(&self.ccl_init_pipeline);
                pass.set_bind_group(0, &ccl_bind_group, &[]);
                pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
                pass.pop_debug_group();

                pass.push_debug_group("ccl_merge_pass");
                pass.set_pipeline(&self.ccl_merge_pipeline);
                pass.set_bind_group(0, &ccl_bind_group, &[]);
                let max_distance = (total_pixels as f32).sqrt().ceil() as u32;
                let ccl_iterations = (max_distance as f32).log2().ceil() as u32 + 1;
                for _ in 0..ccl_iterations {
                    pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
                }
                pass.pop_debug_group();

                pass.push_debug_group("ccl_compress_pass");
                pass.set_pipeline(&self.ccl_compress_pipeline);
                pass.set_bind_group(0, &ccl_bind_group, &[]);
                pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
                pass.pop_debug_group();

                pass.push_debug_group("ccl_extract_pass");
                pass.set_pipeline(&self.ccl_extract_pipeline);
                pass.set_bind_group(0, &ccl_bind_group, &[]);
                pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
                pass.pop_debug_group();
            }

            queue.submit([ec.finish()]);
        }

        if inner_params.grow > 0 {
            let grown_mask = mask.create_allocated_empty_sibling();

            let grow_bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("grow_bind_group"),
                layout: &self.grow_layout,
                entries: &BindGroupEntries::sequential((
                    bucket_params_buffer.binding().unwrap(),
                    BindingResource::TextureView(mask.texture_view().unwrap()),
                    BindingResource::TextureView(grown_mask.texture_view().unwrap()),
                    mask.tile_info_buffer().unwrap().as_entire_binding(),
                )),
            });

            let mut ec = device.create_command_encoder(&Default::default());

            {
                let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                    label: Some("grow_pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.grow_pipeline);
                pass.set_bind_group(0, &grow_bind_group, &[]);
                pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
            }

            queue.submit([ec.finish()]);

            mask = grown_mask;
        }

        match bucket_params.aa_approach {
            BucketAntialiasApproach::None => {}
            BucketAntialiasApproach::Fxaa => {
                let smoothed_mask = mask.create_allocated_empty_sibling();
                self.fxaa_pipeline.dispatch(
                    device,
                    queue,
                    &FxaaParams::default(),
                    mask.binding().unwrap(),
                    smoothed_mask.binding().unwrap(),
                );
                mask = smoothed_mask;
            }
            BucketAntialiasApproach::Feather(radius) => 'a: {
                if radius == 0 {
                    break 'a;
                }

                let max_jump = radius.next_power_of_two();
                let (jump_params_buffer, jump_params_offsets) =
                    create_jfa_params(device, queue, max_jump, "feather_jump_params");
                let jump_iterations = jump_params_offsets.len();

                let common_entries = vec![
                    BindGroupEntry {
                        binding: 0,
                        resource: bucket_params_buffer.binding().unwrap(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(mask.texture_view().unwrap()),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: mask.tile_info_buffer().unwrap().as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: jump_params_buffer.binding().unwrap(),
                    },
                ];

                let bind_groups = {
                    let a_to_b_entries = common_entries
                        .clone()
                        .into_iter()
                        .chain([
                            BindGroupEntry {
                                binding: 2,
                                resource: BindingResource::TextureView(&seed_texture_a_view),
                            },
                            BindGroupEntry {
                                binding: 3,
                                resource: BindingResource::TextureView(&seed_texture_b_view),
                            },
                        ])
                        .collect::<Vec<_>>();
                    let b_to_a_entries = common_entries
                        .into_iter()
                        .chain([
                            BindGroupEntry {
                                binding: 2,
                                resource: BindingResource::TextureView(&seed_texture_b_view),
                            },
                            BindGroupEntry {
                                binding: 3,
                                resource: BindingResource::TextureView(&seed_texture_a_view),
                            },
                        ])
                        .collect::<Vec<_>>();

                    let a_to_b_group = device.create_bind_group(&BindGroupDescriptor {
                        label: Some("feather_seed_a_to_b_bind_group"),
                        layout: &self.close_gap_and_feather_layout,
                        entries: &a_to_b_entries,
                    });
                    let b_to_a_group = device.create_bind_group(&BindGroupDescriptor {
                        label: Some("feather_seed_b_to_a_bind_group"),
                        layout: &self.close_gap_and_feather_layout,
                        entries: &b_to_a_entries,
                    });

                    [a_to_b_group, b_to_a_group]
                };

                let mut ec = device.create_command_encoder(&Default::default());
                {
                    let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                        label: Some("feather_seed_edges_pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.close_gap_and_feather_seed_pipeline);
                    pass.set_bind_group(0, &bind_groups[1], &[jump_params_offsets[0]]);
                    pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
                }

                {
                    let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                        label: Some("feather_jfa_pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.close_gap_and_feather_jump_pipeline);
                    for i in 0..jump_iterations {
                        pass.set_bind_group(0, &bind_groups[i % 2], &[jump_params_offsets[i]]);
                        pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
                    }
                }

                {
                    let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                        label: Some("feather_resolve_pass"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&self.feather_resolve_pipeline);
                    pass.set_bind_group(
                        0,
                        &bind_groups[jump_iterations % 2],
                        &[jump_params_offsets[jump_iterations - 1]],
                    );
                    pass.dispatch_workgroups(dispatch_xy, dispatch_xy, mask.len() as u32);
                }
                queue.submit([ec.finish()]);
            }
        };

        dbg!();
        Some(BucketResultInternal {
            bucket_params_buffer,
            mask,
        })
    }

    fn classify_seed_mode(
        &self,
        device: &Device,
        queue: &Queue,
        params_buffer: &DynamicBuffer<BucketParamsInner>,
        ref_layer: &LayerBinding,
    ) -> bool {
        let seed_mode_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("seed_mode_buffer"),
            size: 4,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let seed_mode_bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("seed_mode_bind_group"),
            layout: &self.seed_mode_layout,
            entries: &BindGroupEntries::sequential((
                params_buffer.binding().unwrap(),
                BindingResource::TextureView(&ref_layer.texture),
                seed_mode_buffer.as_entire_binding(),
                ref_layer.tile_info_buffer.as_entire_binding(),
            )),
        });

        let mut ec = device.create_command_encoder(&Default::default());
        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("seed_mode_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.seed_mode_pipeline);
            pass.set_bind_group(0, &seed_mode_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        let seed_mode_readback_buffer =
            create_readback_buffer_and_schedule_copy_buffer(device, &mut ec, &seed_mode_buffer);
        let seed_mode_readback =
            readback_buffer_on_submit_async::<u32, _>(&mut ec, &seed_mode_readback_buffer, ..);

        let si = queue.submit([ec.finish()]);
        device.poll_indefinitely_for(si).unwrap();

        let seed_mode = seed_mode_readback.block_on().unwrap();

        seed_mode == 1
    }
}

fn create_jfa_params(
    device: &Device,
    queue: &Queue,
    max_jump: u32,
    label: &'static str,
) -> (DynamicBuffer<JumpParams>, Vec<u32>) {
    let mut jump_params_buffer = DynamicBuffer::new(Some(label.into()), BufferUsages::UNIFORM);
    let mut jump_params_offsets = Vec::new();
    let mut jump = max_jump.max(1);
    while jump > 0 {
        let offset = jump_params_buffer.push(&JumpParams { jump });
        jump_params_offsets.push(offset as u32);
        jump /= 2;
    }
    jump_params_buffer.write_buffer(device, queue);

    (jump_params_buffer, jump_params_offsets)
}
