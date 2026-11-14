use anyhow::Result;
use encase::ShaderType;
use glam::{IVec2, IVec3, IVec4, UVec2, UVec3, UVec4, Vec2, Vec3, Vec4};
use iced_core::{Element, Length};
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::DynamicBindGroupLayoutEntries,
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::{flex::Flex, spin_slider};
use serde::Deserialize as _;
use wesl::syntax::{Expression, ExpressionNode, FunctionCall, Ident, TypeExpression};
use wgpu::{Buffer, Device, Queue};

use super::{prepare_uniform_storage, push_buffer_binding, push_storage_layout};
use crate::{
    GraphRenderer, GraphTheme,
    graph::slot::{GraphShaderStage, GraphValueType},
    save::GraphValueTypeId,
};

/// UI message updating a single component at `index` of a vector literal.
#[derive(Clone)]
pub struct VectorMessage<T> {
    index: usize,
    value: T,
}

macro_rules! vector_type {
    (
        $(#[$meta:meta])*
        $name:ident,
        $literal:ty,
        $scalar:ty,
        $len:tt,
        $id:literal,
        $wgsl:literal,
        $zero:expr,
        $range:expr,
        $component:ident
    ) => {
        $(#[$meta])*
        #[derive(Default, Clone)]
        pub struct $name;

        impl GraphValueType for $name {
            type AssociatedLiteralType = $literal;
            type MetadataType = ();
            type PreparedShaderType = Buffer;
            type Message = VectorMessage<$scalar>;

            fn hue_chroma(&self) -> (f32, f32) {
                random_oklch_hue_chroma!($name)
            }

            fn id(&self) -> GraphValueTypeId {
                GraphValueTypeId::new($id)
            }

            fn default_literal(&self) -> Self::AssociatedLiteralType {
                $zero
            }

            fn default_metadata(&self) -> Self::MetadataType {}

            fn wgsl_type_name(&self) -> Option<&'static str> {
                Some($wgsl)
            }
            fn wgsl_array_element_stride(&self) -> Option<u64> {
                Some(<Vec<$literal> as ShaderType>::METADATA.stride().get())
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
                push_storage_layout(stage, $wgsl, name, group, binding, bindings, shader)
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

            fn prepare_to_shader(
                &self,
                data: &Self::AssociatedLiteralType,
                device: &Device,
                _queue: &Queue,
            ) -> Result<Buffer> {
                prepare_uniform_storage(data, device, concat!("graph ", $wgsl, " literal"))
            }

            fn view_literal(
                &self,
                data: &Self::AssociatedLiteralType,
                _metadata: &Self::MetadataType,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
                let controls = (0..$len)
                    .map(|index| {
                        spin_slider($range, data[index])
                            .on_change(move |value| VectorMessage { index, value })
                            .into()
                    })
                    .collect::<Vec<Element<'static, Self::Message, GraphTheme, GraphRenderer>>>();
                Flex::column(controls).width(Length::Fill).padding(2).into()
            }

            fn update_literal(
                &self,
                data: &mut Self::AssociatedLiteralType,
                _metadata: &mut Self::MetadataType,
                message: Self::Message,
            ) {
                data[message.index] = message.value;
            }

            fn literal_to_code(&self, data: &Self::AssociatedLiteralType) -> Option<Expression> {
                let constructor = FunctionCall {
                    ty: TypeExpression::from(Ident::new($wgsl.to_string())),
                    arguments: (0..$len)
                        .map(|index| {
                            let component: Expression = data[index].into();
                            ExpressionNode::from(component)
                        })
                        .collect(),
                };
                Some(Expression::FunctionCall(constructor))
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

vector_type!(
    /// Shader graph value type representing a 2D 32-bit floating-point vector (`vec2f`).
    Vec2FType,
    Vec2,
    f32,
    2,
    "vec2f",
    "vec2f",
    Vec2::ZERO,
    0.0..=1.0,
    f
);
vector_type!(
    /// Shader graph value type representing a 3D 32-bit floating-point vector (`vec3f`).
    Vec3FType,
    Vec3,
    f32,
    3,
    "vec3f",
    "vec3f",
    Vec3::ZERO,
    0.0..=1.0,
    f
);
vector_type!(
    /// Shader graph value type representing a 4D 32-bit floating-point vector (`vec4f`).
    Vec4FType,
    Vec4,
    f32,
    4,
    "vec4f",
    "vec4f",
    Vec4::ZERO,
    0.0..=1.0,
    f
);

vector_type!(
    /// Shader graph value type representing a 2D 32-bit signed integer vector (`vec2i`).
    Vec2IType,
    IVec2,
    i32,
    2,
    "vec2i",
    "vec2i",
    IVec2::ZERO,
    -10..=10,
    i
);
vector_type!(
    /// Shader graph value type representing a 3D 32-bit signed integer vector (`vec3i`).
    Vec3IType,
    IVec3,
    i32,
    3,
    "vec3i",
    "vec3i",
    IVec3::ZERO,
    -10..=10,
    i
);
vector_type!(
    /// Shader graph value type representing a 4D 32-bit signed integer vector (`vec4i`).
    Vec4IType,
    IVec4,
    i32,
    4,
    "vec4i",
    "vec4i",
    IVec4::ZERO,
    -10..=10,
    i
);

vector_type!(
    /// Shader graph value type representing a 2D 32-bit unsigned integer vector (`vec2u`).
    Vec2UType,
    UVec2,
    u32,
    2,
    "vec2u",
    "vec2u",
    UVec2::ZERO,
    0..=10,
    u
);
vector_type!(
    /// Shader graph value type representing a 3D 32-bit unsigned integer vector (`vec3u`).
    Vec3UType,
    UVec3,
    u32,
    3,
    "vec3u",
    "vec3u",
    UVec3::ZERO,
    0..=10,
    u
);
vector_type!(
    /// Shader graph value type representing a 4D 32-bit unsigned integer vector (`vec4u`).
    Vec4UType,
    UVec4,
    u32,
    4,
    "vec4u",
    "vec4u",
    UVec4::ZERO,
    0..=10,
    u
);
