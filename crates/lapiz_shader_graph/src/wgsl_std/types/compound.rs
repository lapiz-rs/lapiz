use anyhow::Result;
use bevy_math::Rect;
use encase::ShaderType;
use glam::Vec4;
use iced_core::Element;
use lapiz_assets::store::AssetRegistry;
use lapiz_i18n::t;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::DynamicBindGroupLayoutEntries,
};
use lapiz_utils::random_oklch_hue_chroma;
use serde::Deserialize as _;
use wesl::syntax::{
    Expression, FunctionCall, Ident, ModulePath, PathOrigin, Span, Spanned, TypeExpression,
};
use wesl_quote::quote_expression;
use wgpu::{Buffer, Device, Queue};

use super::{
    prepare_uniform_storage,
    primitive::{F32Message, F32Metadata, F32Type},
    push_buffer_binding, push_storage_layout,
    vector::{ComponentMessage, ComponentMetadata},
};
use crate::{
    GraphRenderer, GraphTheme,
    graph::slot::{GraphShaderStage, GraphValueType},
    save::GraphValueTypeId,
};

/// Shader graph value type representing an RGBA color (`vec4f`).
#[derive(Default, Clone)]
pub struct ColorType;

/// Per-channel range and editing settings in RGBA order.
pub type ColorMetadata = ComponentMetadata<F32Metadata, 4>;

pub type ColorMessage = ComponentMessage<F32Message>;

impl GraphValueType for ColorType {
    type AssociatedLiteralType = Vec4;
    type MetadataType = ColorMetadata;
    type PreparedShaderType = Buffer;

    type Message = ColorMessage;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ColorType)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("color")
    }

    fn wgsl_array_element_stride(&self) -> Option<u64> {
        Some(<Vec<Vec4> as ShaderType>::METADATA.stride().get())
    }

    fn default_literal(&self) -> Self::AssociatedLiteralType {
        Vec4::ZERO
    }

    fn default_metadata(&self) -> Self::MetadataType {
        ColorMetadata::new(F32Metadata::zero_to_one())
    }

    fn wgsl_type_name(&self) -> Option<&'static str> {
        Some("vec4f")
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
        push_storage_layout(stage, "vec4f", name, group, binding, bindings, shader)
    }

    fn push_shader_binding<'a>(
        &self,
        _stage: GraphShaderStage,
        value: &'a Buffer,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)> {
        push_buffer_binding(value, binding, bindings)
    }

    fn prepare_to_shader(&self, data: &Vec4, device: &Device, _queue: &Queue) -> Result<Buffer> {
        prepare_uniform_storage(data, device, "graph vec4f literal")
    }

    fn view_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        metadata: &Self::MetadataType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        metadata.view(&data.to_array(), ["R", "G", "B", "A"].map(str::to_owned))
    }

    fn update_literal(
        &self,
        data: &mut Self::AssociatedLiteralType,
        metadata: &mut Self::MetadataType,
        message: Self::Message,
    ) {
        let mut components = data.to_array();
        metadata.update(&F32Type, &mut components, message);
        *data = Vec4::from_array(components);
    }

    fn literal_to_code(&self, data: &Self::AssociatedLiteralType) -> Option<Expression> {
        let (x, y, z, w) = (
            Expression::from(data.x),
            Expression::from(data.y),
            Expression::from(data.z),
            Expression::from(data.w),
        );
        Some(quote_expression! { vec4f(#x, #y, #z, #w) })
    }

    fn serialize_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        _assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        Ok(toml::Value::try_from(data)?)
    }

    fn deserialize_literal(
        &self,
        value: toml::Value,
        _assets: &AssetRegistry,
    ) -> Result<Self::AssociatedLiteralType> {
        Ok(Self::AssociatedLiteralType::deserialize(value)?)
    }
}

/// Shader graph value type representing a 2D axis-aligned bounding rectangle (`render::math::Rect`).
#[derive(Default, Clone)]
pub struct RectType;

/// Per-coordinate settings in min-x, min-y, max-x, max-y order.
pub type RectMetadata = ComponentMetadata<F32Metadata, 4>;

pub type RectMessage = ComponentMessage<F32Message>;

impl GraphValueType for RectType {
    type AssociatedLiteralType = Rect;
    type MetadataType = RectMetadata;
    type PreparedShaderType = Buffer;

    type Message = RectMessage;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RectType)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("rect")
    }

    fn wgsl_array_element_stride(&self) -> Option<u64> {
        Some(<Vec<Rect> as ShaderType>::METADATA.stride().get())
    }

    fn default_literal(&self) -> Self::AssociatedLiteralType {
        Rect::default()
    }

    fn default_metadata(&self) -> Self::MetadataType {
        RectMetadata::new(F32Metadata::zero_to_one())
    }

    fn wgsl_type_name(&self) -> Option<&'static str> {
        Some("render::math::Rect")
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
        push_storage_layout(
            stage,
            "render::math::Rect",
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
        value: &'a Buffer,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)> {
        push_buffer_binding(value, binding, bindings)
    }

    fn prepare_to_shader(&self, data: &Rect, device: &Device, _queue: &Queue) -> Result<Buffer> {
        prepare_uniform_storage(data, device, "graph rect literal")
    }

    fn view_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        metadata: &Self::MetadataType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        metadata.view(
            &[data.min.x, data.min.y, data.max.x, data.max.y],
            [t!("min_x"), t!("min_y"), t!("max_x"), t!("max_y")],
        )
    }

    fn update_literal(
        &self,
        data: &mut Self::AssociatedLiteralType,
        metadata: &mut Self::MetadataType,
        message: Self::Message,
    ) {
        let mut components = [data.min.x, data.min.y, data.max.x, data.max.y];
        metadata.update(&F32Type, &mut components, message);
        [data.min.x, data.min.y, data.max.x, data.max.y] = components;
    }

    fn literal_to_code(&self, data: &Self::AssociatedLiteralType) -> Option<Expression> {
        let (min_x, min_y, max_x, max_y) = (
            Expression::from(data.min.x),
            Expression::from(data.min.y),
            Expression::from(data.max.x),
            Expression::from(data.max.y),
        );
        Some(quote_expression! {
            render::math::Rect(vec2f(#min_x, #min_y), vec2f(#max_x, #max_y))
        })
    }

    fn serialize_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        _assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        Ok(toml::Value::try_from(data)?)
    }

    fn deserialize_literal(
        &self,
        value: toml::Value,
        _assets: &AssetRegistry,
    ) -> Result<Self::AssociatedLiteralType> {
        Ok(Self::AssociatedLiteralType::deserialize(value)?)
    }
}
