use std::array::from_fn;

use anyhow::Result;
use encase::ShaderType;
use glam::{IVec2, IVec3, IVec4, UVec2, UVec3, UVec4, Vec2, Vec3, Vec4};
use iced_core::{Element, Length};
use lapiz_assets::store::AssetRegistry;
use lapiz_i18n::t;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::DynamicBindGroupLayoutEntries,
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::{checkbox, flex::Flex, label, menu, row};
use serde::Deserialize as _;
use wesl::syntax::{Expression, ExpressionNode, FunctionCall, Ident, TypeExpression};
use wgpu::{Buffer, Device, Queue};

use super::{
    prepare_uniform_storage,
    primitive::{F32Metadata, F32Type, I32Metadata, I32Type, ScalarMetadata, U32Metadata, U32Type},
    push_buffer_binding, push_storage_layout,
};
use crate::{
    GraphRenderer, GraphTheme,
    graph::slot::{GraphShaderStage, GraphValueType},
    save::GraphValueTypeId,
};

/// Independent component settings and an optional shared metadata editing mode.
#[derive(Clone)]
pub struct ComponentMetadata<M, const N: usize> {
    pub components: [M; N],
    pub edit_together: bool,
}

impl<M: Default, const N: usize> Default for ComponentMetadata<M, N> {
    fn default() -> Self {
        Self {
            components: from_fn(|_| M::default()),
            edit_together: false,
        }
    }
}

impl<M: Clone, const N: usize> ComponentMetadata<M, N> {
    pub fn new(component: M) -> Self {
        Self {
            components: from_fn(|_| component.clone()),
            edit_together: false,
        }
    }

    pub fn edit_together(mut self) -> Self {
        self.edit_together = true;
        self
    }

    pub(crate) fn view(
        &self,
        data: &[M::Value; N],
        names: [String; N],
    ) -> Element<'static, ComponentMessage<M::Message>, GraphTheme, GraphRenderer>
    where
        M: ScalarMetadata,
        M::Message: 'static,
    {
        let controls = names.into_iter().enumerate().map(|(index, name)| {
            row![
                label(name),
                self.components[index].view(
                    data[index],
                    move |message| ComponentMessage::Component { index, message },
                    menu()
                        .element(
                            "edit_all_components",
                            checkbox(self.edit_together)
                                .label(t!("edit_all_components"))
                                .on_toggle(ComponentMessage::EditTogether),
                        )
                        .separator(),
                ),
            ]
            .gap(4)
            .into()
        });

        Flex::column(controls).width(Length::Fill).padding(2).into()
    }

    pub(crate) fn update<T>(
        &mut self,
        ty: &T,
        data: &mut [T::ValueType; N],
        message: ComponentMessage<T::Message>,
    ) where
        T: GraphValueType<MetadataType = M>,
        T::Message: Clone,
        M: ScalarMetadata<Message = T::Message>,
    {
        match message {
            ComponentMessage::EditTogether(edit_together) => self.edit_together = edit_together,
            ComponentMessage::Component { index, message } => {
                if self.edit_together && M::is_metadata_message(&message) {
                    for (value, metadata) in data.iter_mut().zip(&mut self.components) {
                        ty.update_literal(value, metadata, message.clone());
                    }
                } else {
                    ty.update_literal(&mut data[index], &mut self.components[index], message);
                }
            }
        }
    }
}

#[derive(Clone)]
pub enum ComponentMessage<M> {
    Component { index: usize, message: M },
    EditTogether(bool),
}

macro_rules! vector_type {
    (
        $(#[$meta:meta])*
        $name:ident,
        $literal:ty,
        $len:tt,
        $id:literal,
        $wgsl:literal,
        $zero:expr,
        $metadata:expr,
        $component:ty
    ) => {
        $(#[$meta])*
        #[derive(Default, Clone)]
        pub struct $name;

        impl GraphValueType for $name {
            type ValueType = $literal;
            type MetadataType = ComponentMetadata<<$component as GraphValueType>::MetadataType, $len>;
            type PreparedShaderType = Buffer;
            type Message = ComponentMessage<<$component as GraphValueType>::Message>;

            fn hue_chroma(&self) -> (f32, f32) {
                random_oklch_hue_chroma!($name)
            }

            fn id(&self) -> GraphValueTypeId {
                GraphValueTypeId::new($id)
            }

            fn default_literal(&self) -> Self::ValueType {
                $zero
            }

            fn default_metadata(&self) -> Self::MetadataType {
                ComponentMetadata::new($metadata)
            }

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
                data: &Self::ValueType,
                device: &Device,
                _queue: &Queue,
            ) -> Result<Buffer> {
                prepare_uniform_storage(data, device, concat!("graph ", $wgsl, " literal"))
            }

            fn view_literal(
                &self,
                data: &Self::ValueType,
                metadata: &Self::MetadataType,
                _assets: &AssetRegistry,
            ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
                let names = from_fn(|index| ["X", "Y", "Z", "W"][index].to_string());
                metadata.view(&data.to_array(), names)
            }

            fn update_literal(
                &self,
                data: &mut Self::ValueType,
                metadata: &mut Self::MetadataType,
                message: Self::Message,
            ) {
                let mut components = data.to_array();
                metadata.update(&<$component>::default(), &mut components, message);
                *data = <$literal>::from_array(components);
            }

            fn literal_to_code(&self, data: &Self::ValueType) -> Option<Expression> {
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
                data: &Self::ValueType,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Result<toml::Value> {
                Ok(toml::Value::try_from(data)?)
            }

            fn deserialize_literal(
                &self,
                value: toml::Value,
                _assets: &lapiz_assets::store::AssetRegistry,
            ) -> Result<Self::ValueType> {
                Ok(Self::ValueType::deserialize(value)?)
            }
        }
    };
}

vector_type!(
    /// Shader graph value type representing a 2D 32-bit floating-point vector (`vec2f`).
    Vec2FType,
    Vec2,
    2,
    "vec2f",
    "vec2f",
    Vec2::ZERO,
    F32Metadata::zero_to_one(),
    F32Type
);
vector_type!(
    /// Shader graph value type representing a 3D 32-bit floating-point vector (`vec3f`).
    Vec3FType,
    Vec3,
    3,
    "vec3f",
    "vec3f",
    Vec3::ZERO,
    F32Metadata::zero_to_one(),
    F32Type
);
vector_type!(
    /// Shader graph value type representing a 4D 32-bit floating-point vector (`vec4f`).
    Vec4FType,
    Vec4,
    4,
    "vec4f",
    "vec4f",
    Vec4::ZERO,
    F32Metadata::zero_to_one(),
    F32Type
);

vector_type!(
    /// Shader graph value type representing a 2D 32-bit signed integer vector (`vec2i`).
    Vec2IType,
    IVec2,
    2,
    "vec2i",
    "vec2i",
    IVec2::ZERO,
    I32Metadata::default(),
    I32Type
);
vector_type!(
    /// Shader graph value type representing a 3D 32-bit signed integer vector (`vec3i`).
    Vec3IType,
    IVec3,
    3,
    "vec3i",
    "vec3i",
    IVec3::ZERO,
    I32Metadata::default(),
    I32Type
);
vector_type!(
    /// Shader graph value type representing a 4D 32-bit signed integer vector (`vec4i`).
    Vec4IType,
    IVec4,
    4,
    "vec4i",
    "vec4i",
    IVec4::ZERO,
    I32Metadata::default(),
    I32Type
);

vector_type!(
    /// Shader graph value type representing a 2D 32-bit unsigned integer vector (`vec2u`).
    Vec2UType,
    UVec2,
    2,
    "vec2u",
    "vec2u",
    UVec2::ZERO,
    U32Metadata::default(),
    U32Type
);
vector_type!(
    /// Shader graph value type representing a 3D 32-bit unsigned integer vector (`vec3u`).
    Vec3UType,
    UVec3,
    3,
    "vec3u",
    "vec3u",
    UVec3::ZERO,
    U32Metadata::default(),
    U32Type
);
vector_type!(
    /// Shader graph value type representing a 4D 32-bit unsigned integer vector (`vec4u`).
    Vec4UType,
    UVec4,
    4,
    "vec4u",
    "vec4u",
    UVec4::ZERO,
    U32Metadata::default(),
    U32Type
);
