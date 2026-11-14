use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use bevy_math::{IRect, IVec2, IVec4};
use encase::{ShaderType as _, StorageBuffer};
use iced_core::Element;
use lapiz_assets::{
    asset::{AssetHandle, AssetId},
    store::AssetRegistry,
};
use lapiz_i18n::t;
use lapiz_image::{
    texel::TexelType,
    tile::{DynamicLayerStorage, GpuLayerInfo, GpuTileInfo, LayerBinding},
};
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{DynamicBindGroupLayoutEntries, binding_types},
    readback::{create_readback_buffer_and_schedule_copy_buffer, readback_buffer_on_submit_async},
    texture::{GpuImage, Image, ImageListItem},
    util::DevicePollExt as _,
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::{combo_box::ComboBox, flex::Flex, label, row, space};
use serde::{Deserialize, Serialize};
use toml::map::Map;
use uuid::Uuid;
use wesl::syntax::{
    AccessMode, AddressSpace, AssignmentOperator, AssignmentStatement, Attribute, BinaryExpression,
    BinaryOperator, BreakStatement, CompoundStatement, Declaration, DeclarationKind, Expression,
    ForStatement, FormalParameter, Function, FunctionCall, FunctionCallStatement,
    GlobalDeclaration, Ident, IfClause, IfStatement, IndexingExpression, LiteralExpression,
    ModulePath, NamedComponentExpression, ParenthesizedExpression, PathOrigin, ReturnStatement,
    Span, Spanned, Statement, TemplateArg, TypeExpression, UnaryExpression, UnaryOperator,
};
use wesl_quote::{quote_declaration, quote_expression, quote_statement};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Device, Extent3d, Queue,
    TextureDescriptor, TextureDimension, TextureUsages, TextureViewDescriptor,
    TextureViewDimension, util::DeviceExt as _,
};

use super::{
    compound::{ColorType, RectType},
    layer_bounds_ident, layer_load_ident, layer_store_ident, layer_tile_info_ident,
    primitive::{F32Type, I32Type},
    push_buffer_binding, push_storage_layout,
};
use crate::{
    GraphRenderer, GraphTheme,
    graph::{
        node::GraphNodeCodeGenContext,
        slot::{
            ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphDefaultInputSlot,
            GraphDefaultOutputSlot, GraphInputSlotId, GraphShaderStage, GraphValueType,
        },
        variable::GraphLiteral,
    },
    save::GraphValueTypeId,
};

/// UI message emitted when a user selects a new [`Image`] asset for a [`TextureType`] literal.
#[derive(Clone)]
pub struct TextureChanged(pub AssetHandle<Image>);

/// Shader graph value type representing a 2D sampled/loaded texture (`texture_2d<T>`) with a specific [`TexelType`].
#[derive(Clone)]
pub struct TextureType {
    pub texel_type: TexelType,
}

impl Default for TextureType {
    fn default() -> Self {
        Self {
            texel_type: TexelType::RGBA8,
        }
    }
}

/// CPU-side literal value for [`TextureType`], holding an optional [`AssetHandle<Image>`].
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct TextureReference {
    pub texture: Option<AssetHandle<Image>>,
}

impl TextureReference {
    pub const NULL: Self = Self { texture: None };

    pub fn from_asset(asset: AssetHandle<Image>) -> Self {
        Self {
            texture: Some(asset),
        }
    }
}

impl Serialize for TextureReference {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.texture {
            Some(handle) => serializer.serialize_some(&handle.id()),
            None => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for TextureReference {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<AssetId<Image>>::deserialize(deserializer)?;
        Ok(Self::NULL)
    }
}

impl GraphValueType for TextureType {
    type ValueType = TextureReference;
    type MetadataType = ();
    type PreparedShaderType = wgpu::TextureView;
    type Message = TextureChanged;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(TextureType)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new(match self.texel_type {
            TexelType::RGBA8 => "texture_rgba8",
            TexelType::A8 => "texture_a8",
        })
    }

    fn push_shader_layout(
        &self,
        name: &str,
        stage: GraphShaderStage,
        group: u32,
        binding: u32,
        bindings: DynamicBindGroupLayoutEntries,
        mut shader: String,
    ) -> Result<(u32, DynamicBindGroupLayoutEntries, String)> {
        if stage != GraphShaderStage::Input {
            bail!("Texture values cannot be shader outputs");
        }
        let scalar = match self.texel_type.sample_type() {
            wgpu::TextureSampleType::Uint => "u32",
            wgpu::TextureSampleType::Sint => "i32",
            _ => "f32",
        };
        let scalar = Ident::new(scalar.to_string());
        shader.push_str(
            &quote_declaration! {
                @group(#group) @binding(#binding) var #name: texture_2d<#scalar>;
            }
            .to_string(),
        );
        shader.push('\n');
        let bindings = bindings.extend_with_indices(((
            binding,
            binding_types::texture_2d(self.texel_type.sample_type()),
        ),));
        Ok((binding + 1, bindings, shader))
    }

    fn push_shader_binding<'a>(
        &self,
        _stage: GraphShaderStage,
        value: &'a wgpu::TextureView,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)> {
        let bindings = bindings.extend_with_indices(((binding, value),));
        Ok((binding + 1, bindings))
    }

    fn prepare_to_shader(
        &self,
        data: &Self::ValueType,
        device: &Device,
        queue: &Queue,
    ) -> Result<Self::PreparedShaderType> {
        // TODO Validate texture format matches current type declaration.
        //      We should prevent dismatch at UI level?
        if let Some(handle) = &data.texture {
            let image = handle.get().context("texture asset is not loaded")?;
            let gpu = GpuImage::from_asset(device, queue, &image, TextureUsages::TEXTURE_BINDING);
            return Ok(gpu.texture.create_view(&TextureViewDescriptor::default()));
        }
        // The NULL reference still binds an empty placeholder.
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("graph dummy texture"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: self.texel_type.wgpu_format(),
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Ok(texture.create_view(&Default::default()))
    }

    fn serialize_literal(
        &self,
        data: &Self::ValueType,
        _assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        let mut table = Map::new();
        if let Some(handle) = &data.texture {
            table.insert("asset".into(), toml::Value::try_from(handle.id())?);
        }
        Ok(toml::Value::Table(table))
    }

    fn deserialize_literal(
        &self,
        deserializer: toml::Value,
        assets: &AssetRegistry,
    ) -> Result<Self::ValueType> {
        let Some(asset) = deserializer.get("asset") else {
            return Ok(TextureReference::NULL);
        };
        let id = AssetId::<Image>::deserialize(asset.clone())?;
        let handle = assets
            .handle(id)
            .map_err(|e| anyhow::anyhow!("texture asset {id} is unavailable: {e:?}"))?;
        Ok(TextureReference {
            texture: Some(handle),
        })
    }

    fn default_literal(&self) -> Self::ValueType {
        TextureReference::NULL
    }

    fn default_metadata(&self) -> Self::MetadataType {}

    fn wgsl_type_name(&self) -> Option<&'static str> {
        None
    }

    fn view_literal(
        &self,
        data: &Self::ValueType,
        _metadata: &Self::MetadataType,
        assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        let options = assets.all_handles_of::<Image>().unwrap_or_default();
        let selected = data
            .texture
            .as_ref()
            .and_then(|handle| options.iter().find(|option| *option == handle).cloned());

        ComboBox::new_with(options, selected, TextureChanged, |handle| {
            ImageListItem::new(handle.clone()).into()
        })
        .placeholder(t!("select_texture"))
        .width(iced_core::Length::Fill)
        .into()
    }

    fn update_literal(
        &self,
        data: &mut Self::ValueType,
        _metadata: &mut Self::MetadataType,
        message: Self::Message,
    ) {
        data.texture = Some(message.0);
    }

    fn literal_to_code(&self, _data: &Self::ValueType) -> Option<Expression> {
        None
    }
}

/// Shader graph value type representing a tiled 2D image layer (`layer_rgba8` or `layer_a8`).
///
/// Unlike [`TextureType`], `LayerType` represents a dynamically tiled canvas layer backed by a
/// `texture_storage_2d_array`, a tile metadata buffer, and an integer bounding box. In effect
/// passes, it exposes both texel color/alpha and spatial bounds slots, and participates in the
/// two-stage `Eval` (bounding-box calculation and tile allocation) and `Main` (texel read/write)
/// execution pipeline.
#[derive(Clone)]
pub struct LayerType {
    pub texel_type: TexelType,
}

/// Unit literal value placeholder for [`LayerType`], which cannot be specified as an inline constant.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct LayerReference;

/// Backing GPU pixel storage for a [`PreparedLayer`], either dynamically allocated read-write tiles or a borrowed read-only [`LayerBinding`].
#[allow(
    clippy::large_enum_variant,
    reason = "callers move DynamicLayerStorage out of the writable variant"
)]
pub enum PreparedLayerPixels {
    ReadWrite {
        storage: DynamicLayerStorage,
        // TODO can we optimize this out?
        empty_binding: LayerBinding,
    },
    ReadOnly(LayerBinding),
}

/// GPU-prepared representation of a [`LayerType`] value, bundling tile storage, CPU-readback pixel bounds, and a GPU bounds buffer.
pub struct PreparedLayer {
    pub pixels: PreparedLayerPixels,
    pub pixel_bounds: Option<IRect>,
    pub bounds: Buffer,
}

impl PreparedLayer {
    pub fn from_binding(binding: LayerBinding, bounds: Buffer) -> Self {
        Self {
            pixels: PreparedLayerPixels::ReadOnly(binding),
            pixel_bounds: None,
            bounds,
        }
    }

    pub fn deep_clone(&self) -> Self {
        let pixels = match &self.pixels {
            PreparedLayerPixels::ReadWrite {
                storage,
                empty_binding,
            } => PreparedLayerPixels::ReadWrite {
                storage: storage.deep_clone(),
                empty_binding: empty_binding.clone(),
            },
            PreparedLayerPixels::ReadOnly(binding) => {
                PreparedLayerPixels::ReadOnly(binding.clone())
            }
        };
        Self {
            pixels,
            pixel_bounds: self.pixel_bounds,
            bounds: self.bounds.clone(),
        }
    }
}

impl GraphValueType for LayerType {
    type ValueType = LayerReference;
    type MetadataType = ();
    type PreparedShaderType = PreparedLayer;
    type Message = ();

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(LayerType)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new(match self.texel_type {
            TexelType::RGBA8 => "layer_rgba8",
            TexelType::A8 => "layer_a8",
        })
    }

    fn push_input_slots(&self) -> Vec<GraphDefaultInputSlot> {
        let color = match self.texel_type {
            TexelType::RGBA8 => GraphDefaultInputSlot::new::<ColorType>("color".into()),
            TexelType::A8 => GraphDefaultInputSlot::new::<F32Type>("alpha".into()),
        };
        vec![
            color,
            GraphDefaultInputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn push_output_slots(&self) -> Vec<GraphDefaultOutputSlot> {
        let color = match self.texel_type {
            TexelType::RGBA8 => GraphDefaultOutputSlot::new::<ColorType>("color".into()),
            TexelType::A8 => GraphDefaultOutputSlot::new::<F32Type>("alpha".into()),
        };
        vec![
            color,
            GraphDefaultOutputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn handle_input_values(
        &self,
        input_name: &str,
        base_index: usize,
        ctx: &mut GraphNodeCodeGenContext,
    ) -> Result<String> {
        let color = ctx.get_output(base_index)?;
        let bounds = ctx.get_output(base_index + 1)?;
        ctx.output_slot_idents
            .insert(ctx.outputs[base_index], color.clone().into());
        ctx.output_slot_idents
            .insert(ctx.outputs[base_index + 1], bounds.clone().into());
        let load = Ident::new(layer_load_ident(input_name));
        let input_bounds = Ident::new(layer_bounds_ident(input_name));
        let color_stmt = quote_statement! {
            let #color = #load(dispatch_index);
        };
        let bounds_stmt = quote_statement! {
            let #bounds = render::math::Rect(vec2f(#input_bounds.xy), vec2f(#input_bounds.zw));
        };
        Ok(format!("{}\n{}", color_stmt, bounds_stmt))
    }

    fn handle_output_values(
        &self,
        output_name: &str,
        base_index: usize,
        ctx: &GraphNodeCodeGenContext,
    ) -> Result<String> {
        let color = ctx.get_input(base_index)?;
        let bounds = ctx.get_input(base_index + 1)?;
        let output_bounds = Ident::new(layer_bounds_ident(output_name));
        let store = Ident::new(layer_store_ident(output_name));
        let eval_stmt = quote_statement! {
            @if(EVAL) { #output_bounds = vec4i(vec2i(floor((#bounds).min)), vec2i(ceil((#bounds).max))); }
        };
        let main_stmt = quote_statement! {
            @if(!EVAL) {
                if all(dispatch_index >= vec2i(floor((#bounds).min))) && all(dispatch_index < vec2i(ceil((#bounds).max))) {
                    #store(dispatch_index, #color);
                }
            }
        };
        Ok(format!("{}\n{}", eval_stmt, main_stmt))
    }

    fn push_shader_layout(
        &self,
        name: &str,
        stage: GraphShaderStage,
        group: u32,
        binding: u32,
        bindings: DynamicBindGroupLayoutEntries,
        mut shader: String,
    ) -> Result<(u32, DynamicBindGroupLayoutEntries, String)> {
        match stage {
            GraphShaderStage::Input | GraphShaderStage::Main => {
                let (access, access_ident) = if stage == GraphShaderStage::Input {
                    (
                        wgpu::StorageTextureAccess::ReadOnly,
                        Ident::new("read".to_string()),
                    )
                } else {
                    (
                        wgpu::StorageTextureAccess::WriteOnly,
                        Ident::new("write".to_string()),
                    )
                };
                let texel = Ident::new(self.texel_type.wgpu_texel());
                shader.push_str(
                    &quote_declaration! {
                        @group(#group) @binding(#binding) var #name: texture_storage_2d_array<#texel, #access_ident>;
                    }
                    .to_string(),
                );
                shader.push('\n');
                let bindings = bindings.extend_with_indices(((
                    binding,
                    binding_types::texture_storage_2d_array(self.texel_type.wgpu_format(), access),
                ),));
                let tile_info_binding = binding + 1;
                let tile_info = Ident::new(layer_tile_info_ident(name));
                shader.push_str(
                    &quote_declaration! {
                        @group(#group) @binding(#tile_info_binding) var<storage, read> #tile_info: array<image::image_tiling::TileInfo>;
                    }
                    .to_string(),
                );
                shader.push('\n');
                let bindings = bindings.extend_with_indices(((
                    tile_info_binding,
                    binding_types::storage_buffer_read_only_sized(false, None),
                ),));
                if stage == GraphShaderStage::Input {
                    let bounds_binding = binding + 2;
                    let bounds = Ident::new(layer_bounds_ident(name));
                    shader.push_str(
                        &quote_declaration! {
                            @group(#group) @binding(#bounds_binding) var<storage, read> #bounds: vec4i;
                        }
                        .to_string(),
                    );
                    shader.push('\n');
                    let bindings = bindings.extend_with_indices(((
                        bounds_binding,
                        binding_types::storage_buffer_read_only_sized(false, None),
                    ),));
                    Ok((binding + 3, bindings, shader))
                } else {
                    Ok((binding + 2, bindings, shader))
                }
            }
            GraphShaderStage::Eval => {
                let bounds = Ident::new(layer_bounds_ident(name));
                shader.push_str(
                    &quote_declaration! {
                        @group(#group) @binding(#binding) var<storage, read_write> #bounds: vec4i;
                    }
                    .to_string(),
                );
                shader.push('\n');
                let bindings = bindings.extend_with_indices(((
                    binding,
                    binding_types::storage_buffer_sized(false, None),
                ),));
                Ok((binding + 1, bindings, shader))
            }
        }
    }

    fn push_shader_binding<'a>(
        &self,
        stage: GraphShaderStage,
        value: &'a PreparedLayer,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)> {
        match stage {
            GraphShaderStage::Input | GraphShaderStage::Main => {
                let (texture, tile_info) = match &value.pixels {
                    PreparedLayerPixels::ReadWrite {
                        storage,
                        empty_binding,
                    } => (
                        storage.texture_view().unwrap_or(&empty_binding.texture),
                        storage
                            .tile_info_buffer()
                            .unwrap_or(&empty_binding.tile_info_buffer),
                    ),
                    PreparedLayerPixels::ReadOnly(binding) => {
                        if stage == GraphShaderStage::Main {
                            bail!("read-only layer cannot be a shader output");
                        }
                        (&binding.texture, &binding.tile_info_buffer)
                    }
                };
                let bindings = bindings.extend_with_indices(((binding, texture),));
                let bindings =
                    bindings.extend_with_indices(((binding + 1, tile_info.as_entire_binding()),));
                if stage == GraphShaderStage::Input {
                    let bindings = bindings
                        .extend_with_indices(((binding + 2, value.bounds.as_entire_binding()),));
                    Ok((binding + 3, bindings))
                } else {
                    Ok((binding + 2, bindings))
                }
            }
            GraphShaderStage::Eval => {
                let bindings =
                    bindings.extend_with_indices(((binding, value.bounds.as_entire_binding()),));
                Ok((binding + 1, bindings))
            }
        }
    }

    fn prepare_to_shader(
        &self,
        _data: &Self::ValueType,
        device: &Device,
        queue: &Queue,
    ) -> Result<Self::PreparedShaderType> {
        let dummy_texture = device.create_texture(&TextureDescriptor {
            label: Some("graph empty layer texture"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: self.texel_type.wgpu_format(),
            usage: TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let mut initial_bounds = StorageBuffer::new(Vec::new());
        initial_bounds.write(&IVec4::new(i32::MAX, i32::MAX, i32::MIN, i32::MIN))?;
        Ok(PreparedLayer {
            pixels: PreparedLayerPixels::ReadWrite {
                storage: DynamicLayerStorage::new(
                    device.clone(),
                    queue.clone(),
                    GpuLayerInfo {
                        texel_type: self.texel_type,
                    },
                ),
                empty_binding: LayerBinding {
                    texture: dummy_texture.create_view(&TextureViewDescriptor {
                        dimension: Some(TextureViewDimension::D2Array),
                        ..Default::default()
                    }),
                    tile_info_buffer: device.create_buffer(&BufferDescriptor {
                        label: Some("graph empty layer tile info"),
                        size: u64::from(GpuTileInfo::min_size()),
                        usage: BufferUsages::STORAGE,
                        mapped_at_creation: false,
                    }),
                },
            },
            pixel_bounds: Some(IRect::EMPTY),
            bounds: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("graph layer bounds"),
                contents: initial_bounds.as_ref(),
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            }),
        })
    }

    fn requires_eval(&self) -> bool {
        true
    }

    fn post_eval(&self, value: &mut PreparedLayer, device: &Device, queue: &Queue) -> Result<()> {
        // TODO: async readback; block for now.
        let mut encoder = device.create_command_encoder(&Default::default());
        let staging =
            create_readback_buffer_and_schedule_copy_buffer(device, &mut encoder, &value.bounds);
        let readback = readback_buffer_on_submit_async::<IVec4, _>(&mut encoder, &staging, ..);
        let submission = queue.submit([encoder.finish()]);
        device.poll_indefinitely_for(submission)?;
        let mut readback = readback.into_inner();
        let bounds = loop {
            if let Some(bounds) = readback.try_recv()? {
                break bounds?;
            }
            device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })?;
        };
        let pixel_bounds = IRect {
            min: IVec2::new(bounds.x, bounds.y),
            max: IVec2::new(bounds.z, bounds.w),
        };
        value.pixel_bounds = Some(pixel_bounds);
        let PreparedLayerPixels::ReadWrite { storage, .. } = &mut value.pixels else {
            bail!("read-only layer cannot be allocated as a shader output");
        };
        storage.allocate_pixels(pixel_bounds);
        Ok(())
    }

    fn literal_to_code(&self, _data: &Self::ValueType) -> Option<Expression> {
        None
    }

    fn serialize_literal(
        &self,
        _data: &Self::ValueType,
        _assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        Ok(toml::Value::Table(Default::default()))
    }

    fn deserialize_literal(
        &self,
        _deserializer: toml::Value,
        _assets: &AssetRegistry,
    ) -> Result<Self::ValueType> {
        Ok(LayerReference)
    }

    fn generate_extra_shader_body(&self, stage: GraphShaderStage, name: &str) -> Option<String> {
        let load = Ident::new(layer_load_ident(name));
        let store = Ident::new(layer_store_ident(name));
        let texture = Ident::new(name.to_string());
        let tile_info = Ident::new(layer_tile_info_ident(name));

        let (pack, unpack, default_value, color_ty) = match self.texel_type {
            TexelType::RGBA8 => (
                quote_expression! {
                    image::texture_unpack::pack_rgba8_texel(color)
                },
                quote_expression! {
                    image::texture_unpack::unpack_rgba8_texel(texel)
                },
                quote_expression! {
                    image::texture_unpack::pack_rgba8_texel(vec4f(0.0))
                },
                Ident::new("vec4f".into()),
            ),
            TexelType::A8 => (
                quote_expression! {
                    image::texture_unpack::pack_a8_texel(color)
                },
                quote_expression! {
                    image::texture_unpack::unpack_a8_texel(texel)
                },
                quote_expression! {
                    image::texture_unpack::pack_a8_texel(0.0)
                },
                Ident::new("f32".into()),
            ),
        };

        let mut helpers = String::new();
        match stage {
            GraphShaderStage::Input =>
            {
                helpers.push_str(
                                &quote_declaration! {
                                    fn #load(pixel: vec2i) -> #color_ty {
                                        var texel = #default_value;
                                        for (var tile = 0u; tile < arrayLength(&#tile_info); tile += 1u) {
                                            let info = #tile_info[tile];
                                            if (pixel.x >= info.origin.x) && (pixel.x < info.origin.x + i32(image::image_tiling::TILE_SIZE))
                                                && (pixel.y >= info.origin.y) && (pixel.y < info.origin.y + i32(image::image_tiling::TILE_SIZE)) {
                                                texel = textureLoad(#texture, pixel - info.origin, tile);
                                                break;
                                            }
                                        }
                                        return #unpack;
                                    }
                                }
                                .to_string(),
                            )
            }
,
            GraphShaderStage::Eval =>{},
            GraphShaderStage::Main =>{

                helpers.push_str(
                    &quote_declaration! {
                        @if(!EVAL) fn #store(pixel: vec2i, color: #color_ty) {
                            for (var tile = 0u; tile < arrayLength(&#tile_info); tile += 1u) {
                                let info = #tile_info[tile];
                                if (pixel.x >= info.origin.x) && (pixel.x < info.origin.x + i32(image::image_tiling::TILE_SIZE))
                                    && (pixel.y >= info.origin.y) && (pixel.y < info.origin.y + i32(image::image_tiling::TILE_SIZE)) {
                                    textureStore(#texture, pixel - info.origin, tile, #pack);
                                    return;
                                }
                            }
                        }
                    }
                    .to_string(),
                );
            },
        }
        helpers.push('\n');
        Some(helpers)
    }

    fn default_literal(&self) -> Self::ValueType {
        LayerReference
    }

    fn default_metadata(&self) -> Self::MetadataType {}

    fn wgsl_type_name(&self) -> Option<&'static str> {
        None
    }

    fn view_literal(
        &self,
        _data: &Self::ValueType,
        _metadata: &Self::MetadataType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        Element::new(space())
    }

    fn update_literal(
        &self,
        _data: &mut Self::ValueType,
        _metadata: &mut Self::MetadataType,
        _message: Self::Message,
    ) {
    }
}

/// Shader graph value type representing a fixed-length storage buffer array (`array<T>`) of a primitive, vector, or compound element type.
#[derive(Clone)]
pub struct ArrayType {
    pub element_type: Arc<dyn ErasedGraphValueType>,
    pub len: u32,
}

/// CPU-side literal value for [`ArrayType`], storing a vector of per-element [`GraphLiteral`]s.
#[derive(Clone)]
pub struct ArrayLiteral {
    pub elements: Vec<GraphLiteral>,
}

/// UI message updating the element at `index` within an [`ArrayLiteral`].
#[derive(Clone)]
pub struct ArrayLiteralUpdateMessage {
    index: usize,
    message: ErasedGraphLiteralUpdateMessage,
}

/// GPU-prepared storage [`Buffer`] and element count for an [`ArrayType`] binding.
#[derive(Clone)]
pub struct PreparedArray {
    pub buffer: Buffer,
    pub len: u32,
}

impl GraphValueType for ArrayType {
    type ValueType = ArrayLiteral;
    type MetadataType = ();
    type PreparedShaderType = PreparedArray;
    type Message = ArrayLiteralUpdateMessage;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ArrayType)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new(format!("array_{}_{}", self.element_type.id().id, self.len))
    }

    fn push_input_slots(&self) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<I32Type>("index".into()),
            GraphDefaultInputSlot::new_boxed("value".into(), self.element_type.clone()),
        ]
    }

    fn push_output_slots(&self) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new_boxed(
            "value".into(),
            self.element_type.clone(),
        )]
    }

    fn handle_input_values(
        &self,
        input_name: &str,
        base_index: usize,
        ctx: &mut GraphNodeCodeGenContext,
    ) -> Result<String> {
        ctx.get_output(base_index)?;
        let name = Ident::new(input_name.to_string());
        ctx.output_slot_idents.insert(
            ctx.outputs[base_index],
            quote_expression! { #name[dispatch_index] },
        );
        Ok(String::new())
    }

    fn handle_output_values(
        &self,
        output_name: &str,
        base_index: usize,
        ctx: &GraphNodeCodeGenContext,
    ) -> Result<String> {
        let index = ctx.get_input(base_index)?;
        let value = ctx.get_input(base_index + 1)?;
        Ok(format!(
            "@if(!EVAL) {{ {output_name}[{index}] = {value}; }}\n"
        ))
    }

    fn push_shader_layout(
        &self,
        name: &str,
        stage: GraphShaderStage,
        group: u32,
        binding: u32,
        bindings: DynamicBindGroupLayoutEntries,
        shader: String,
    ) -> Result<(u32, DynamicBindGroupLayoutEntries, String)> {
        let element = self
            .element_type
            .wgsl_type_name()
            .context("Array element type has no GPU representation")?
            .to_string();
        push_storage_layout(
            stage,
            &format!("array<{element}>"),
            name,
            group,
            binding,
            bindings,
            shader,
        )
    }

    fn push_shader_binding<'a>(
        &self,
        _stage: GraphShaderStage,
        value: &'a PreparedArray,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)> {
        push_buffer_binding(&value.buffer, binding, bindings)
    }

    fn prepare_to_shader(
        &self,
        data: &Self::ValueType,
        device: &Device,
        queue: &Queue,
    ) -> Result<Self::PreparedShaderType> {
        if data.elements.len() != self.len as usize {
            bail!(
                "Array literal has {} elements, expected {}",
                data.elements.len(),
                self.len
            );
        }
        let stride = self
            .element_type
            .wgsl_array_element_stride()
            .context("Array element type has no GPU representation")?;
        let size = u64::from(self.len)
            .checked_mul(stride)
            .context("Array too large")?;
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("graph array literal"),
            size: size.max(4),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let prepared_elements = data
            .elements
            .iter()
            .map(|element| {
                if element.ty().id() != self.element_type.id() {
                    bail!(
                        "Array literal element type is {}, expected {}",
                        element.ty().id().id,
                        self.element_type.id().id
                    );
                }
                self.element_type
                    .prepare_to_shader(element.value(), device, queue)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("graph array literal upload"),
        });
        // TODO better way to avoid cloning?
        for (index, prepared) in prepared_elements.iter().enumerate() {
            let source = prepared
                .downcast_ref::<Buffer>()
                .context("Array element type did not prepare to a buffer")?;
            encoder.copy_buffer_to_buffer(
                source,
                0,
                &buffer,
                index as u64 * stride,
                source.size().min(stride),
            );
        }
        queue.submit([encoder.finish()]);

        Ok(PreparedArray {
            buffer,
            len: self.len,
        })
    }

    fn default_literal(&self) -> Self::ValueType {
        ArrayLiteral {
            elements: vec![
                GraphLiteral::new_boxed_default(self.element_type.clone());
                self.len as usize
            ],
        }
    }

    fn default_metadata(&self) -> Self::MetadataType {}

    fn wgsl_type_name(&self) -> Option<&'static str> {
        None
    }

    fn view_literal<'a>(
        &self,
        data: &'a Self::ValueType,
        _metadata: &'a Self::MetadataType,
        assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        let elements = data
            .elements
            .iter()
            .enumerate()
            .map(|(index, element)| {
                row![
                    label(index.to_string()).width(24).muted(),
                    element
                        .ty()
                        .view_literal(
                            GraphInputSlotId::new(Uuid::nil()),
                            element.value(),
                            element.metadata(),
                            assets
                        )
                        .map(move |message| ArrayLiteralUpdateMessage { index, message }),
                ]
                .gap(4.0)
                .into()
            })
            .collect::<Vec<_>>();
        Flex::column(elements).gap(4.0).into()
    }

    fn update_literal(
        &self,
        data: &mut Self::ValueType,
        _metadata: &mut Self::MetadataType,
        message: Self::Message,
    ) {
        data.elements
            .get_mut(message.index)
            .expect("array literal update index is valid")
            .update(message.message);
    }

    fn literal_to_code(&self, _data: &Self::ValueType) -> Option<Expression> {
        None
    }

    fn serialize_literal(
        &self,
        data: &Self::ValueType,
        assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        if data.elements.len() != self.len as usize {
            bail!(
                "Array literal has {} elements, expected {}",
                data.elements.len(),
                self.len
            );
        }
        data.elements
            .iter()
            .map(|element| {
                if element.ty().id() != self.element_type.id() {
                    bail!(
                        "Array literal element type is {}, expected {}",
                        element.ty().id().id,
                        self.element_type.id().id
                    );
                }
                self.element_type.serialize_literal(element.value(), assets)
            })
            .collect::<Result<Vec<_>>>()
            .map(toml::Value::Array)
    }

    fn deserialize_literal(
        &self,
        deserializer: toml::Value,
        assets: &AssetRegistry,
    ) -> Result<Self::ValueType> {
        let values = match deserializer {
            toml::Value::Array(values) => values,
            _ => bail!("Array literal must be an array"),
        };
        if values.len() != self.len as usize {
            bail!(
                "Array literal has {} elements, expected {}",
                values.len(),
                self.len
            );
        }
        let elements = values
            .into_iter()
            .map(|value| {
                self.element_type
                    .deserialize_literal(value, assets)
                    .map(|value| {
                        GraphLiteral::new_boxed(
                            value,
                            self.element_type.default_metadata(),
                            self.element_type.clone(),
                        )
                    })
            })
            .collect::<Result<_>>()?;
        Ok(ArrayLiteral { elements })
    }
}
