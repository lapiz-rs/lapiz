use std::{any::TypeId, fs::File, io::BufReader, path::Path};

use anyhow::Result;
use glam::UVec3;
use imagers::DynamicImage;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{
        BindGroupLayoutEntries, DynamicBindGroupLayoutEntries, binding_types,
    },
    buffer::DynamicBuffer,
    wesl_jit,
};
use moxcms::ColorProfile;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupLayoutDescriptor, BufferUsages, ComputePass,
    ComputePipeline, ComputePipelineDescriptor, Device, PipelineLayoutDescriptor, Queue,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StorageTextureAccess,
};

use crate::{
    CImage,
    composite::{
        BlendFunctionId, BlendFunctionRegistry, BlendLayerParams, ImageCompositor,
        LayerPreviewOverriders, PixelPreviewOverrider,
    },
    copy_layer::{CopyLayerPipeline, PreparedCopyLayerPipeline},
    layer::{
        Layer, LayerId, LayerStackNode,
        properties::{
            BlendFunctionProp, BlendFunctionPropertyExt, DisabledChannelsProp,
            DisabledChannelsPropertyExt, EncodedLayerProperties, HasLayerProperties,
            LayerProperties, LayerPropertiesDeclaration, LayerTexelTypeProp, LockedChannelsProp,
            LockedProp, NameProp, NamePropertyExt, OpacityProp, OpacityPropertyExt, VisibleProp,
            VisiblePropertyExt,
        },
    },
    texel::TexelType,
    tile::{GpuTileInfo, GpuTileStorage, LayerBinding},
};

#[derive(Debug, Default, Clone)]
pub struct PixelLayer;

impl Layer for PixelLayer {
    fn can_have_children_of(&self, _: TypeId) -> bool {
        false
    }

    fn layer_type(&self) -> u32 {
        0
    }

    fn create_blend_cache(
        &self,
        compositor: &mut ImageCompositor,
        _: &mut LayerPreviewOverriders,
        image: &CImage,
        layer_id: LayerId,
        tiles: &GpuTileStorage,
        blend_funcs: &BlendFunctionRegistry,
        device: &Device,
        _: &Queue,
    ) {
        let node = image.layer_stack().get_layer(&layer_id).unwrap();
        let props = node.properties();
        let layer_info = tiles.get_layer_info(layer_id).unwrap();
        let blend_func_id = props.blend_function();

        if let Some(cache) = compositor.get_blend_cache_mut::<PixelBlendCache>(&layer_id)
            && cache.blend_func_name == *blend_func_id
            && cache.layer_texel_type == layer_info.texel_type
            && cache.image_texel_type == image.texel_type()
        {
            return;
        }

        let blend_func = blend_funcs
            .get(blend_func_id)
            .unwrap_or_else(|| panic!("Blend function {} not found", blend_func_id));
        let shader = include_str!("../blend_layers.wesl").replace(
            "//CODEGEN_BLEND_FUNC",
            &blend_func.wgsl_function_call("src", "dst"),
        );

        let without_overrider_shader = wesl_jit::compile_wesl_with_config(
            shader.clone(),
            &[&crate::image::PACKAGE],
            |compiler| {
                compiler.set_feature("OVERRIDER", false);
            },
        )
        .unwrap();

        let with_overrider_shader =
            wesl_jit::compile_wesl_with_config(shader, &[&crate::image::PACKAGE], |compiler| {
                compiler.set_feature("OVERRIDER", true);
            })
            .unwrap();

        let with_overrider_shader_module = device.create_shader_module(ShaderModuleDescriptor {
            label: "pixel layer blend shader".into(),
            source: ShaderSource::Wgsl(with_overrider_shader.into()),
        });
        let without_overrider_shader_module = device.create_shader_module(ShaderModuleDescriptor {
            label: "pixel layer blend shader".into(),
            source: ShaderSource::Wgsl(without_overrider_shader.into()),
        });

        let mut entries = DynamicBindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                binding_types::uniform_buffer::<BlendLayerParams>(false),
                binding_types::texture_storage_2d_array(
                    layer_info.texel_type.wgpu_format(),
                    StorageTextureAccess::ReadOnly,
                ),
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                binding_types::texture_storage_2d_array(
                    layer_info.texel_type.wgpu_format(),
                    StorageTextureAccess::ReadOnly,
                ),
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                binding_types::texture_storage_2d_array(
                    image.texel_type().wgpu_format(),
                    StorageTextureAccess::WriteOnly,
                ),
                binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
            ),
        )
        .to_vec();
        let without_overrider_layout =
            device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: "pixel layer blend bind group layout without overrider".into(),
                entries: &entries,
            });

        entries.extend(
            BindGroupLayoutEntries::with_indices(
                ShaderStages::COMPUTE,
                (
                    (
                        7,
                        binding_types::texture_storage_2d_array(
                            image.texel_type().wgpu_format(),
                            StorageTextureAccess::ReadOnly,
                        ),
                    ),
                    (
                        8,
                        binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    ),
                ),
            )
            .as_ref(),
        );
        let with_overrider_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: "pixel layer blend bind group layout with overrider".into(),
            entries: &entries,
        });

        let with_overrider_pipeline_layout =
            device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: "pixel layer blend pipeline layout with overrider".into(),
                bind_group_layouts: &[Some(&with_overrider_layout)],
                ..Default::default()
            });

        let without_overrider_pipeline_layout =
            device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: "pixel layer blend pipeline layout without overrider".into(),
                bind_group_layouts: &[Some(&without_overrider_layout)],
                ..Default::default()
            });

        let with_overrider_pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: "pixel layer blend pipeline with overrider".into(),
            layout: Some(&with_overrider_pipeline_layout),
            module: &with_overrider_shader_module,
            entry_point: "main".into(),
            compilation_options: Default::default(),
            cache: None,
        });

        let without_overrider_pipeline =
            device.create_compute_pipeline(&ComputePipelineDescriptor {
                label: "pixel layer blend pipeline without overrider".into(),
                layout: Some(&without_overrider_pipeline_layout),
                module: &without_overrider_shader_module,
                entry_point: "main".into(),
                compilation_options: Default::default(),
                cache: None,
            });

        let params_buffer = DynamicBuffer::new(
            Some("pixel layer blend params buffer".into()),
            BufferUsages::UNIFORM,
        );

        let copy_pipeline = CopyLayerPipeline::new(device, image.texel_type);

        let cache = PixelBlendCache {
            blend_func_name: blend_func_id.clone(),
            layer_texel_type: layer_info.texel_type,
            image_texel_type: image.texel_type(),
            params_buffer,
            with_overrider_pipeline,
            without_overrider_pipeline,
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
        let Some(cache) = compositor.get_blend_cache_mut::<PixelBlendCache>(&layer_id) else {
            log::error!("BlendCache is not created for layer {:?}", layer_id);
            return;
        };

        let node = image.layer_stack().get_layer(&layer_id).unwrap();
        let props = node.properties();

        if !props.visible() {
            cache.copy_prepared = Some(cache.copy_pipeline.prepare(device, dst_layer, output));
            return;
        }

        let src = tiles.get_layer_binding_or_empty(layer_id).unwrap();

        cache.params_buffer.clear();
        cache.params_buffer.push(&BlendLayerParams {
            src_opacity: props.opacity(),
            src_disabled_channels: props.disabled_channels().0,
            _pad: Default::default(),
        });
        cache.params_buffer.write_buffer(device, queue);

        let mut entries = DynamicBindGroupEntries::sequential((
            cache.params_buffer.binding().unwrap(),
            &src.texture,
            src.tile_info_buffer.as_entire_binding(),
            &dst_layer.texture,
            dst_layer.tile_info_buffer.as_entire_binding(),
            &output.texture,
            output.tile_info_buffer.as_entire_binding(),
        ));

        let pipeline =
            if let Some(overrider) = overriders.get_overrider::<PixelPreviewOverrider>(&layer_id) {
                entries = entries.extend_sequential((
                    &overrider.texture,
                    overrider.tile_info_buffer.as_entire_binding(),
                ));

                cache.with_overrider_pipeline.clone()
            } else {
                cache.without_overrider_pipeline.clone()
            };

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: "pixel layer blend bind group".into(),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });

        let workgroup_count =
            UVec3::new(image.size().x.div_ceil(16), image.size().y.div_ceil(16), 1);

        cache.dispatch = Some((pipeline, bind_group, workgroup_count));
    }

    fn dispatch_blend(
        &self,
        compositor: &ImageCompositor,
        pass: &mut ComputePass,
        image: &CImage,
        layer_id: LayerId,
        _: &GpuTileStorage,
    ) {
        let Some(cache) = compositor.get_blend_cache::<PixelBlendCache>(&layer_id) else {
            log::error!("BlendCache is not created for layer {:?}", layer_id);
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

        let Some((pipeline, bind_group, workgroup_count)) = &cache.dispatch else {
            log::error!("BlendCache bind group is not prepared");
            return;
        };

        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(workgroup_count.x, workgroup_count.y, workgroup_count.z);
    }
}

pub struct PixelBlendCache {
    blend_func_name: BlendFunctionId,
    layer_texel_type: TexelType,
    image_texel_type: TexelType,
    params_buffer: DynamicBuffer<BlendLayerParams>,
    with_overrider_pipeline: ComputePipeline,
    without_overrider_pipeline: ComputePipeline,
    dispatch: Option<(ComputePipeline, BindGroup, UVec3)>,
    copy_pipeline: CopyLayerPipeline,
    copy_prepared: Option<PreparedCopyLayerPipeline>,
}

impl PixelLayer {
    pub fn from_path(
        path: impl AsRef<Path>,
        tiles: &GpuTileStorage,
        dst_profile: &ColorProfile,
    ) -> Result<LayerStackNode> {
        let path = path.as_ref();
        let filename = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let (image, profile) = CImage::load_image_with_profile(BufReader::new(File::open(path)?))?;

        let mut layer = Self::from_image(image, tiles);
        layer.properties_mut().set_name(filename);
        let layer_storage = tiles.get_layer(layer.id).unwrap();
        layer_storage.convert_color_space(&profile, dst_profile, Default::default())?;

        Ok(layer)
    }

    pub fn from_image(img: DynamicImage, tiles: &GpuTileStorage) -> LayerStackNode {
        let id = LayerId::random();
        tiles.upload_image(id, img);
        LayerStackNode::without_parent(id, Box::new(Self), LayerProperties::new::<Self>())
    }
}

impl HasLayerProperties for PixelLayer {
    fn new_properties() -> LayerPropertiesDeclaration {
        let mut decl = LayerPropertiesDeclaration::default();
        decl.create_default::<NameProp>();
        decl.create_default::<VisibleProp>();
        decl.create_default::<BlendFunctionProp>();
        decl.create_default::<OpacityProp>();
        decl.create_default::<LockedProp>();
        decl.create_default::<LockedChannelsProp>();
        decl.create_default::<DisabledChannelsProp>();
        decl.create(LayerTexelTypeProp(TexelType::RGBA8));
        decl
    }

    fn decode_properties(mut data: EncodedLayerProperties) -> Result<LayerPropertiesDeclaration> {
        let mut decl = LayerPropertiesDeclaration::default();
        data.decode::<NameProp>(&mut decl)?;
        data.decode::<VisibleProp>(&mut decl)?;
        data.decode::<BlendFunctionProp>(&mut decl)?;
        data.decode::<OpacityProp>(&mut decl)?;
        data.decode::<LockedProp>(&mut decl)?;
        data.decode::<LockedChannelsProp>(&mut decl)?;
        data.decode::<DisabledChannelsProp>(&mut decl)?;
        data.decode::<LayerTexelTypeProp>(&mut decl)?;
        Ok(decl)
    }
}
