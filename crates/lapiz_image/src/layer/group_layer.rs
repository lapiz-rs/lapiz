use std::any::TypeId;

use anyhow::Result;
use bevy_math::IRect;
use glam::{IVec2, UVec3};
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    wesl_jit,
};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, BufferUsages,
    ComputePass, ComputePipeline, ComputePipelineDescriptor, Device, PipelineLayoutDescriptor,
    Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages, StorageTextureAccess,
};

use crate::{
    CImage,
    composite::{
        BlendFunctionId, BlendFunctionRegistry, BlendLayerParams, ImageCompositor,
        LayerPreviewOverriders,
    },
    copy_layer::{CopyLayerPipeline, PreparedCopyLayerPipeline},
    dynamic_intermediate_buffer::IntermediateBuffer,
    layer::{
        Layer, LayerId,
        properties::{
            BlendFunctionProp, BlendFunctionPropertyExt, DisabledChannelsProp,
            DisabledChannelsPropertyExt, EncodedLayerProperties, HasLayerProperties,
            LayerPropertiesDeclaration, LockedProp, NameProp, OpacityProp, OpacityPropertyExt,
            VisibleProp, VisiblePropertyExt,
        },
    },
    tile::{GpuTileInfo, GpuTileStorage, LayerBinding},
};

#[derive(Debug, Default, Clone)]
pub struct GroupLayer;

impl Layer for GroupLayer {
    fn can_have_children_of(&self, _: TypeId) -> bool {
        true
    }

    fn layer_type(&self) -> u32 {
        1
    }

    fn create_blend_cache(
        &self,
        compositor: &mut ImageCompositor,
        overriders: &mut LayerPreviewOverriders,
        image: &CImage,
        layer_id: LayerId,
        tiles: &GpuTileStorage,
        blend_funcs: &BlendFunctionRegistry,
        device: &Device,
        queue: &Queue,
    ) {
        let node = image.layer_stack().get_layer(&layer_id).unwrap();
        for child_id in node.iter_children_composite_order() {
            let child_layer = image.layer_stack().get_layer(child_id).unwrap();
            child_layer.create_blend_cache(
                compositor,
                overriders,
                image,
                tiles,
                blend_funcs,
                device,
                queue,
            );
        }

        let props = node.properties();
        let blend_func_id = props.blend_function();

        let tile_rect = GpuTileStorage::pixel_rect_to_tile(IRect {
            min: IVec2::ZERO,
            max: image.size().as_ivec2(),
        });

        if let Some(cache) = compositor.get_blend_cache::<GroupBlendCache>(&layer_id)
            && cache.blend_func_name == *blend_func_id
            && cache.intermediate.texel_type() == image.texel_type()
            && cache.intermediate.tile_rect() == tile_rect
        {
            return;
        }

        let blend_func = blend_funcs
            .get(blend_func_id)
            .unwrap_or_else(|| panic!("Blend function '{}' not found", blend_func_id));

        let shader = wesl_jit::compile_wesl(
            include_str!("../blend_layers.wesl").replace(
                "//CODEGEN_BLEND_FUNC",
                &blend_func.wgsl_function_call("src", "dst"),
            ),
            &[&crate::image::PACKAGE],
        )
        .unwrap();

        let shader_module = device.create_shader_module(ShaderModuleDescriptor {
            label: "layer blend shader".into(),
            source: ShaderSource::Wgsl(shader.into()),
        });

        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "layer blend bind group layout".into(),
            entries: &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::uniform_buffer::<GpuTileInfo>(false),
                    binding_types::texture_storage_2d_array(
                        image.texel_type().wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::texture_storage_2d_array(
                        image.texel_type().wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::texture_storage_2d_array(
                        image.texel_type().wgpu_format(),
                        StorageTextureAccess::WriteOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                ),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: "layer blend pipeline layout".into(),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });

        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: "layer blend pipeline".into(),
            layout: Some(&pipeline_layout),
            module: &shader_module,
            entry_point: "main".into(),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buffer = DynamicBuffer::new(
            Some("group layer blend params buffer".into()),
            BufferUsages::UNIFORM,
        );

        let copy_pipeline = CopyLayerPipeline::new(device, image.texel_type);

        let cache = GroupBlendCache {
            blend_func_name: blend_func_id.clone(),
            intermediate: IntermediateBuffer::new(device, queue, tile_rect, image.texel_type()),
            params_buffer,
            layout,
            pipeline,
            dispatch: None,
            copy_pipeline,
            copy_prepared: None,
        };
        compositor.insert_blend_cache(layer_id, cache);
    }

    fn prepare_blend_cache(
        &self,
        compositor: &mut ImageCompositor,
        overriders: &LayerPreviewOverriders,
        image: &CImage,
        layer_id: LayerId,
        tiles: &GpuTileStorage,
        dst_layer: &LayerBinding,
        output: &LayerBinding,
        device: &Device,
        queue: &Queue,
    ) {
        let Some(cache) = compositor.get_blend_cache_mut::<GroupBlendCache>(&layer_id) else {
            log::error!("BlendCache is not created for layer {}", layer_id);
            return;
        };

        let node = image.layer_stack().get_layer(&layer_id).unwrap();
        let props = node.properties();

        if !props.visible() {
            cache.copy_prepared = Some(cache.copy_pipeline.prepare(device, dst_layer, output));
            return;
        }

        cache.params_buffer.clear();
        cache.params_buffer.push(&BlendLayerParams {
            src_opacity: props.opacity(),
            src_disabled_channels: props.disabled_channels().0,
            _pad: Default::default(),
        });
        cache.params_buffer.write_buffer(device, queue);

        cache.intermediate.clear(device, queue);

        let mut next_output = 1;
        let bindings = [
            LayerBinding {
                texture: cache.intermediate.textures()[0].clone(),
                tile_info_buffer: cache.intermediate.tile_info_buffer().clone(),
            },
            LayerBinding {
                texture: cache.intermediate.textures()[1].clone(),
                tile_info_buffer: cache.intermediate.tile_info_buffer().clone(),
            },
        ];
        let node = image.layer_stack().get_layer(&layer_id).unwrap();

        for child_node in node.iter_children_composite_order() {
            let child_layer = image.layer_stack().get_layer(child_node).unwrap();

            child_layer.prepare_blend_cache(
                compositor,
                overriders,
                image,
                tiles,
                &bindings[1 - next_output],
                &bindings[next_output],
                device,
                queue,
            );
            next_output = 1 - next_output;
        }

        let cache = compositor
            .get_blend_cache_mut::<GroupBlendCache>(&layer_id)
            .unwrap();

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: "layer blend bind group".into(),
            layout: &cache.layout,
            entries: BindGroupEntries::sequential((
                cache.params_buffer.binding().unwrap(),
                &cache.intermediate.textures()[1 - next_output],
                cache.intermediate.tile_info_buffer().as_entire_binding(),
                &dst_layer.texture,
                dst_layer.tile_info_buffer.as_entire_binding(),
                &output.texture,
                output.tile_info_buffer.as_entire_binding(),
            ))
            .as_ref(),
        });

        let workgroup_count =
            UVec3::new(image.size().x.div_ceil(16), image.size().y.div_ceil(16), 1);

        cache.dispatch = Some((bind_group, workgroup_count));
    }

    fn dispatch_blend(
        &self,
        compositor: &ImageCompositor,
        pass: &mut ComputePass,
        image: &CImage,
        layer_id: LayerId,
        tiles: &GpuTileStorage,
    ) {
        let Some(cache) = compositor.get_blend_cache::<GroupBlendCache>(&layer_id) else {
            log::error!("BlendCache is not created for layer {}", layer_id);
            return;
        };

        let node = image.layer_stack().get_layer(&layer_id).unwrap();
        let props = node.properties();

        if !props.visible() {
            if let Some(prepared) = &cache.copy_prepared {
                cache.copy_pipeline.dispatch(pass, prepared);
            }
            return;
        }

        for child_node in node.iter_children_composite_order() {
            let child_layer = image.layer_stack().get_layer(child_node).unwrap();
            child_layer.dispatch_blend(compositor, pass, image, tiles);
        }

        let Some((bind_group, workgroup_count)) = &cache.dispatch else {
            log::error!("BlendCache bind group is not prepared");
            return;
        };

        pass.set_pipeline(&cache.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(workgroup_count.x, workgroup_count.y, workgroup_count.z);
    }
}

pub struct GroupBlendCache {
    blend_func_name: BlendFunctionId,
    intermediate: IntermediateBuffer,
    params_buffer: DynamicBuffer<BlendLayerParams>,
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
    dispatch: Option<(BindGroup, UVec3)>,
    copy_pipeline: CopyLayerPipeline,
    copy_prepared: Option<PreparedCopyLayerPipeline>,
}

impl HasLayerProperties for GroupLayer {
    fn new_properties() -> LayerPropertiesDeclaration {
        let mut decl = LayerPropertiesDeclaration::default();
        decl.create_default::<NameProp>();
        decl.create_default::<VisibleProp>();
        decl.create_default::<BlendFunctionProp>();
        decl.create_default::<OpacityProp>();
        decl.create_default::<LockedProp>();
        decl.create_default::<DisabledChannelsProp>();
        decl
    }

    fn decode_properties(mut data: EncodedLayerProperties) -> Result<LayerPropertiesDeclaration> {
        let mut decl = LayerPropertiesDeclaration::default();
        data.decode::<NameProp>(&mut decl)?;
        data.decode::<VisibleProp>(&mut decl)?;
        data.decode::<BlendFunctionProp>(&mut decl)?;
        data.decode::<OpacityProp>(&mut decl)?;
        data.decode::<LockedProp>(&mut decl)?;
        data.decode::<DisabledChannelsProp>(&mut decl)?;
        Ok(decl)
    }
}
