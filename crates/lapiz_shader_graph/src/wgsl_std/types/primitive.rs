use std::convert::identity;

use anyhow::Result;
use encase::ShaderType;
use iced_core::Element;
use lapiz_assets::store::AssetRegistry;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::DynamicBindGroupLayoutEntries,
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::{checkbox, spin_slider};
use serde::Deserialize as _;
use wesl::syntax::{
    BinaryExpression, BinaryOperator, Expression, Ident, LiteralExpression, Span, Spanned,
};
use wesl_quote::quote_expression;
use wgpu::{Buffer, Device, Queue};

use super::{prepare_uniform_storage, push_buffer_binding, push_storage_layout};
use crate::{
    GraphRenderer, GraphTheme,
    graph::{
        node::GraphNodeCodeGenContext,
        slot::{GraphShaderStage, GraphValueType},
    },
    save::GraphValueTypeId,
};

/// Shader graph value type representing a 32-bit floating-point scalar (`f32`).
#[derive(Default, Clone)]
pub struct F32Type;

impl GraphValueType for F32Type {
    type AssociatedLiteralType = f32;
    type PreparedShaderType = Buffer;

    type Message = f32;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(F32Type)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("f32")
    }

    fn default_literal(&self) -> Self::AssociatedLiteralType {
        0.0
    }

    fn wgsl_type_name(&self) -> Option<&'static str> {
        Some("f32")
    }
    fn wgsl_array_element_stride(&self) -> Option<u64> {
        Some(<Vec<f32> as ShaderType>::METADATA.stride().get())
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
        push_storage_layout(stage, "f32", name, group, binding, bindings, shader)
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

    fn prepare_to_shader(&self, data: &f32, device: &Device, _queue: &Queue) -> Result<Buffer> {
        prepare_uniform_storage(data, device, "graph f32 literal")
    }

    fn view_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        spin_slider(0.0..=1.0, *data)
            .on_change(identity)
            .step(0.01)
            .into()
    }

    fn update_literal(&self, data: &mut Self::AssociatedLiteralType, message: Self::Message) {
        *data = message;
    }

    fn literal_to_code(&self, data: &Self::AssociatedLiteralType) -> Option<Expression> {
        Some((*data).into())
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

/// Shader graph value type representing a 32-bit signed integer scalar (`i32`).
#[derive(Default, Clone)]
pub struct I32Type;

impl GraphValueType for I32Type {
    type AssociatedLiteralType = i32;
    type PreparedShaderType = Buffer;

    type Message = i32;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(I32Type)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("i32")
    }

    fn default_literal(&self) -> Self::AssociatedLiteralType {
        0
    }

    fn wgsl_type_name(&self) -> Option<&'static str> {
        Some("i32")
    }
    fn wgsl_array_element_stride(&self) -> Option<u64> {
        Some(<Vec<i32> as ShaderType>::METADATA.stride().get())
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
        push_storage_layout(stage, "i32", name, group, binding, bindings, shader)
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

    fn prepare_to_shader(&self, data: &i32, device: &Device, _queue: &Queue) -> Result<Buffer> {
        prepare_uniform_storage(data, device, "graph i32 literal")
    }

    fn literal_to_code(&self, data: &Self::AssociatedLiteralType) -> Option<Expression> {
        Some((*data).into())
    }

    fn view_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        spin_slider(-10..=10, *data).on_change(identity).into()
    }

    fn update_literal(&self, data: &mut Self::AssociatedLiteralType, message: Self::Message) {
        *data = message;
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

/// Shader graph value type representing a 32-bit unsigned integer scalar (`u32`).
#[derive(Default, Clone)]
pub struct U32Type;

impl GraphValueType for U32Type {
    type AssociatedLiteralType = u32;
    type PreparedShaderType = Buffer;
    type Message = u32;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(U32Type)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("u32")
    }

    fn default_literal(&self) -> Self::AssociatedLiteralType {
        0
    }

    fn wgsl_type_name(&self) -> Option<&'static str> {
        Some("u32")
    }
    fn wgsl_array_element_stride(&self) -> Option<u64> {
        Some(<Vec<u32> as ShaderType>::METADATA.stride().get())
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
        push_storage_layout(stage, "u32", name, group, binding, bindings, shader)
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

    fn prepare_to_shader(&self, data: &u32, device: &Device, _queue: &Queue) -> Result<Buffer> {
        prepare_uniform_storage(data, device, "graph u32 literal")
    }

    fn literal_to_code(&self, data: &Self::AssociatedLiteralType) -> Option<Expression> {
        Some((*data).into())
    }

    fn view_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        spin_slider(0..=10, *data).on_change(identity).into()
    }

    fn update_literal(&self, data: &mut Self::AssociatedLiteralType, message: Self::Message) {
        *data = message;
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

/// Shader graph value type representing a boolean scalar (`bool`, stored as `u32` in GPU buffers).
#[derive(Default, Clone)]
pub struct BoolType;

impl GraphValueType for BoolType {
    type AssociatedLiteralType = bool;
    type PreparedShaderType = Buffer;

    type Message = bool;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BoolType)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("bool")
    }

    fn default_literal(&self) -> Self::AssociatedLiteralType {
        false
    }

    fn wgsl_type_name(&self) -> Option<&'static str> {
        Some("bool")
    }
    // Bools are host-shared as u32, so their array stride matches u32.
    fn wgsl_array_element_stride(&self) -> Option<u64> {
        Some(<Vec<u32> as ShaderType>::METADATA.stride().get())
    }

    // FIXME This is not working is some expr references bool type,
    //       because it generates exprs like `if 1u { .. }` which is not valid.
    fn push_shader_layout(
        &self,
        name: &str,
        stage: GraphShaderStage,
        group: u32,
        binding: u32,
        bindings: DynamicBindGroupLayoutEntries,
        shader: String,
    ) -> Result<(u32, DynamicBindGroupLayoutEntries, String)> {
        push_storage_layout(stage, "u32", name, group, binding, bindings, shader)
    }

    fn handle_input_values(
        &self,
        input_name: &str,
        base_index: usize,
        ctx: &mut GraphNodeCodeGenContext,
    ) -> Result<String> {
        ctx.get_output(base_index)?;
        let name = Ident::new(input_name.to_string());
        ctx.output_slot_idents
            .insert(ctx.outputs[base_index], quote_expression! { #name != 0u });
        Ok(String::new())
    }

    fn handle_output_values(
        &self,
        output_name: &str,
        base_index: usize,
        ctx: &GraphNodeCodeGenContext,
    ) -> Result<String> {
        let value = ctx.get_input(base_index)?;
        Ok(format!(
            "@if(!EVAL) {{ {output_name} = select(0u, 1u, {value}); }}\n"
        ))
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

    fn prepare_to_shader(&self, data: &bool, device: &Device, _queue: &Queue) -> Result<Buffer> {
        prepare_uniform_storage(&u32::from(*data), device, "graph bool literal")
    }

    fn literal_to_code(&self, data: &Self::AssociatedLiteralType) -> Option<Expression> {
        Some((*data).into())
    }

    fn view_literal(
        &self,
        data: &Self::AssociatedLiteralType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        checkbox(*data).on_toggle(identity).into()
    }

    fn update_literal(&self, data: &mut Self::AssociatedLiteralType, message: Self::Message) {
        *data = message;
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
