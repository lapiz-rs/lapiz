use anyhow::Result;
use encase::StorageBuffer;
use iced_core::Element;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{DynamicBindGroupLayoutEntries, binding_types},
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::space;
use serde::Deserialize as _;
use wesl::syntax::{
    AccessMode, AddressSpace, Attribute, CompoundStatement, Declaration, DeclarationKind,
    Expression, FunctionCall, FunctionCallStatement, GlobalDeclaration, Ident, IndexingExpression,
    Span, Spanned, Statement, TemplateArg, TypeExpression, UnaryExpression, UnaryOperator,
};
use wesl_quote::{quote_declaration, quote_expression, quote_statement};
use wgpu::{Buffer, Device, Queue};

use super::{
    prepare_uniform_storage,
    primitive::{I32Type, U32Type},
    push_buffer_binding, write_storage_buffer,
};
use crate::{
    GraphRenderer, GraphTheme,
    graph::{
        node::GraphNodeCodeGenContext,
        slot::{GraphDefaultInputSlot, GraphDefaultOutputSlot, GraphShaderStage, GraphValueType},
    },
    save::GraphValueTypeId,
};

/// A GPU storage [`Buffer`] and element count prepared for an atomic array shader binding.
#[derive(Clone)]
pub struct PreparedAtomicArray {
    pub buffer: Buffer,
    pub len: u32,
}

macro_rules! atomic_type {
    ($(#[$meta:meta])* $name:ident, $value:ty, $plain:ty, $id:literal, $wgsl:literal) => {
        $(#[$meta])*
        #[derive(Default, Clone)]
        pub struct $name;

        impl GraphValueType for $name {
            type AssociatedLiteralType = $value;
            type PreparedShaderType = Buffer;
            type Message = ();

            fn id(&self) -> GraphValueTypeId {
                GraphValueTypeId::new($id)
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
                let value_type = Ident::new($wgsl.into());
                let declaration = match stage {
                    GraphShaderStage::Input => quote_declaration! {
                        @group(#group) @binding(#binding) var<storage, read> #name: atomic<#value_type>;
                    },
                    GraphShaderStage::Eval | GraphShaderStage::Main => quote_declaration! {
                        @group(#group) @binding(#binding) var<storage, read_write> #name: atomic<#value_type>;
                    },
                };
                shader.push_str(&declaration.to_string());
                shader.push('\n');
                let entry = match stage {
                    GraphShaderStage::Input => binding_types::storage_buffer_read_only_sized(false, None),
                    GraphShaderStage::Eval | GraphShaderStage::Main => binding_types::storage_buffer_sized(false, None),
                };
                Ok((binding + 1, bindings.extend_with_indices(((binding, entry),)), shader))
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

            fn prepare_to_shader(&self, data: &$value, device: &Device, _queue: &Queue) -> Result<Buffer> {
                prepare_uniform_storage(data, device, concat!("graph ", $id, " literal"))
            }

            fn push_input_slots(&self) -> Vec<GraphDefaultInputSlot> {
                vec![GraphDefaultInputSlot::new::<$plain>("value".into())]
            }

            fn push_output_slots(&self) -> Vec<GraphDefaultOutputSlot> {
                vec![GraphDefaultOutputSlot::new::<$plain>("value".into())]
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
                    quote_expression! { atomicLoad(&#name) },
                );
                Ok(String::new())
            }

            fn handle_output_values(
                &self,
                output_name: &str,
                base_index: usize,
                ctx: &GraphNodeCodeGenContext,
            ) -> Result<String> {
                let value = ctx.get_input(base_index)?;
                let output = Ident::new(output_name.to_string());
                Ok(quote_statement! { @if(!EVAL) { atomicAdd(&#output, #value); } }.to_string())
            }

            fn default_literal(&self) -> Self::AssociatedLiteralType {
                0
            }

            fn wgsl_type_name(&self) -> Option<&'static str> {
                None
            }

            fn hue_chroma(&self) -> (f32, f32) {
                random_oklch_hue_chroma!($name)
            }

            fn view_literal(
                &self,
                _data: &Self::AssociatedLiteralType,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
                Element::new(space())
            }

            fn update_literal(&self, _data: &mut Self::AssociatedLiteralType, _message: Self::Message) {}

            fn literal_to_code(&self, _data: &Self::AssociatedLiteralType) -> Option<Expression> {
                None
            }

            fn serialize_literal(
                &self,
                data: &Self::AssociatedLiteralType,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Result<toml::Value> {
                Ok(toml::Value::try_from(data)?)
            }

            fn deserialize_literal(
                &self,
                value: toml::Value,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Result<Self::AssociatedLiteralType> {
                Ok(Self::AssociatedLiteralType::deserialize(value)?)
            }
        }
    };
}

macro_rules! atomic_array_type {
    ($(#[$meta:meta])* $name:ident, $value:ty, $plain:ty, $prefix:literal, $wgsl:literal) => {
        $(#[$meta])*
        #[derive(Clone)]
        pub struct $name {
            pub len: u32,
        }

        impl GraphValueType for $name {
            type AssociatedLiteralType = Vec<$value>;
            type PreparedShaderType = PreparedAtomicArray;
            type Message = ();

            fn id(&self) -> GraphValueTypeId {
                GraphValueTypeId::new(format!("{}_{}", $prefix, self.len))
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
                let value_type = Ident::new($wgsl.into());
                let declaration = match stage {
                    GraphShaderStage::Input => quote_declaration! {
                        @group(#group) @binding(#binding) var<storage, read> #name: array<atomic<#value_type>>;
                    },
                    GraphShaderStage::Eval | GraphShaderStage::Main => quote_declaration! {
                        @group(#group) @binding(#binding) var<storage, read_write> #name: array<atomic<#value_type>>;
                    },
                };
                shader.push_str(&declaration.to_string());
                shader.push('\n');
                let entry = match stage {
                    GraphShaderStage::Input => binding_types::storage_buffer_read_only_sized(false, None),
                    GraphShaderStage::Eval | GraphShaderStage::Main => binding_types::storage_buffer_sized(false, None),
                };
                Ok((binding + 1, bindings.extend_with_indices(((binding, entry),)), shader))
            }

            fn push_shader_binding<'a>(
                &self,
                _stage: GraphShaderStage,
                value: &'a PreparedAtomicArray,
                binding: u32,
                bindings: DynamicBindGroupEntries<'a>,
            ) -> Result<(u32, DynamicBindGroupEntries<'a>)> {
                push_buffer_binding(&value.buffer, binding, bindings)
            }

            fn prepare_to_shader(&self, data: &Vec<$value>, device: &Device, _queue: &Queue) -> Result<PreparedAtomicArray> {
                let mut values = data.clone();
                values.resize(self.len as usize, 0);
                let mut bytes = StorageBuffer::new(Vec::new());
                bytes.write(&values)?;
                let buffer = write_storage_buffer(device, bytes.as_ref(), concat!("graph ", $prefix, " literal"))?;
                Ok(PreparedAtomicArray { buffer, len: self.len })
            }

            fn push_input_slots(&self) -> Vec<GraphDefaultInputSlot> {
                vec![
                    GraphDefaultInputSlot::new::<I32Type>("index".into()),
                    GraphDefaultInputSlot::new::<$plain>("value".into()),
                ]
            }

            fn push_output_slots(&self) -> Vec<GraphDefaultOutputSlot> {
                vec![GraphDefaultOutputSlot::new::<$plain>("value".into())]
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
                    quote_expression! { atomicLoad(&#name[dispatch_index]) },
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
                let output = Ident::new(output_name.to_string());
                Ok(
                    quote_statement! { @if(!EVAL) { atomicAdd(&#output[#index], #value); } }
                        .to_string(),
                )
            }

            fn default_literal(&self) -> Self::AssociatedLiteralType {
                vec![0; self.len as usize]
            }

            fn wgsl_type_name(&self) -> Option<&'static str> {
                None
            }

            fn hue_chroma(&self) -> (f32, f32) {
                random_oklch_hue_chroma!($name)
            }

            fn view_literal(
                &self,
                _data: &Self::AssociatedLiteralType,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
                Element::new(space())
            }

            fn update_literal(&self, _data: &mut Self::AssociatedLiteralType, _message: Self::Message) {}

            fn literal_to_code(&self, _data: &Self::AssociatedLiteralType) -> Option<Expression> {
                None
            }

            fn serialize_literal(
                &self,
                data: &Self::AssociatedLiteralType,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Result<toml::Value> {
                Ok(toml::Value::try_from(data)?)
            }

            fn deserialize_literal(
                &self,
                value: toml::Value,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Result<Self::AssociatedLiteralType> {
                Ok(Self::AssociatedLiteralType::deserialize(value)?)
            }
        }
    };
}

atomic_type!(
    /// Shader graph value type representing a 32-bit signed atomic integer (`atomic<i32>`).
    AtomicI32Type,
    i32,
    I32Type,
    "atomic_i32",
    "i32"
);
atomic_type!(
    /// Shader graph value type representing a 32-bit unsigned atomic integer (`atomic<u32>`).
    AtomicU32Type,
    u32,
    U32Type,
    "atomic_u32",
    "u32"
);
atomic_array_type!(
    /// Shader graph value type representing a fixed-length storage array of 32-bit signed atomic integers (`array<atomic<i32>>`).
    ArrayAtomicI32Type,
    i32,
    I32Type,
    "array_atomic_i32",
    "i32"
);
atomic_array_type!(
    /// Shader graph value type representing a fixed-length storage array of 32-bit unsigned atomic integers (`array<atomic<u32>>`).
    ArrayAtomicU32Type,
    u32,
    U32Type,
    "array_atomic_u32",
    "u32"
);
