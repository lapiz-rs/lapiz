use std::convert::identity;

use anyhow::Result;
use encase::ShaderType;
use iced_core::Element;
use lapiz_assets::store::AssetRegistry;
use lapiz_i18n::t;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::DynamicBindGroupLayoutEntries,
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::{
    checkbox, context_menu, label, menu, menu::Menu, row, spin_box, spin_slider,
    spin_slider::SliderScale,
};
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

pub(crate) trait ScalarMetadata: Clone {
    type Value: Copy;
    type Message: Clone;

    fn view<Message: Clone + 'static>(
        &self,
        value: Self::Value,
        on_message: impl Fn(Self::Message) -> Message + Copy + 'static,
        menu: Menu<'static, Message>,
    ) -> Element<'static, Message, GraphTheme, GraphRenderer>;

    fn is_metadata_message(message: &Self::Message) -> bool;
}

macro_rules! scalar_metadata_editor {
    ($metadata:ident, $message:ident, $value:ty, $lower:expr, $upper:expr, $min_step:expr, $max_step:expr) => {
        impl ScalarMetadata for $metadata {
            type Value = $value;
            type Message = $message;

            fn view<Message: Clone + 'static>(
                &self,
                value: $value,
                on_message: impl Fn($message) -> Message + Copy + 'static,
                menu: Menu<'static, Message>,
            ) -> Element<'static, Message, GraphTheme, GraphRenderer> {
                context_menu(
                    spin_slider(self.min..=self.max, value)
                        .allow_beyond_range(self.allow_beyond_range)
                        .on_change(move |value| on_message($message::Value(value)))
                        .scale(if self.logarithmic {
                            SliderScale::Logarithmic
                        } else {
                            SliderScale::Linear
                        })
                        .step(self.step),
                    menu.element(
                        "min",
                        row![
                            label(t!("min")),
                            spin_box(&self.min, $lower..=self.max, move |value| on_message(
                                $message::Min(value)
                            )),
                        ]
                        .gap(8),
                    )
                    .element(
                        "max",
                        row![
                            label(t!("max")),
                            spin_box(&self.max, self.min..=$upper, move |value| on_message(
                                $message::Max(value)
                            )),
                        ]
                        .gap(8),
                    )
                    .element(
                        "step",
                        row![
                            label(t!("step")),
                            spin_box(&self.step, $min_step..=$max_step, move |value| on_message(
                                $message::Step(value)
                            )),
                        ]
                        .gap(8),
                    )
                    .element(
                        "logarithmic",
                        checkbox(self.logarithmic)
                            .on_toggle(move |value| on_message($message::Logarithmic(value)))
                            .label(t!("logarithmic")),
                    )
                    .element(
                        "allow_beyond_range",
                        checkbox(self.allow_beyond_range)
                            .on_toggle(move |value| on_message($message::AllowBeyondRange(value)))
                            .label(t!("allow_beyond_range")),
                    )
                    .separator()
                    .item(
                        t!("use_percentage_preset"),
                        None,
                        on_message($message::UsePercentagePreset),
                    )
                    .item(
                        t!("use_zero_to_one_preset"),
                        None,
                        on_message($message::UseZeroToOnePreset),
                    ),
                )
                .into()
            }

            fn is_metadata_message(message: &$message) -> bool {
                match message {
                    $message::Value(_) => false,
                    $message::Min(_)
                    | $message::Max(_)
                    | $message::Step(_)
                    | $message::Logarithmic(_)
                    | $message::AllowBeyondRange(_)
                    | $message::UsePercentagePreset
                    | $message::UseZeroToOnePreset => true,
                }
            }
        }
    };
}

#[derive(Clone)]
pub struct F32Metadata {
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub logarithmic: bool,
    pub allow_beyond_range: bool,
}

impl Default for F32Metadata {
    fn default() -> Self {
        Self::percentage()
    }
}

impl F32Metadata {
    pub fn new(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            step: 0.01,
            logarithmic: false,
            allow_beyond_range: false,
        }
    }

    pub fn percentage() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            logarithmic: false,
            allow_beyond_range: false,
        }
    }

    pub fn zero_to_one() -> Self {
        Self {
            min: 0.0,
            max: 1.0,
            step: 0.01,
            logarithmic: false,
            allow_beyond_range: false,
        }
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    pub fn allow_beyond_range(mut self) -> Self {
        self.allow_beyond_range = true;
        self
    }

    pub fn logarithmic(mut self) -> Self {
        self.logarithmic = true;
        self
    }
}

#[derive(Clone)]
pub enum F32Message {
    Value(f32),
    Min(f32),
    Max(f32),
    Step(f32),
    Logarithmic(bool),
    AllowBeyondRange(bool),
    UsePercentagePreset,
    UseZeroToOnePreset,
}

scalar_metadata_editor!(F32Metadata, F32Message, f32, -1e5, 1e5, 1e-5, 1e5);

/// Shader graph value type representing a 32-bit floating-point scalar (`f32`).
#[derive(Default, Clone)]
pub struct F32Type;

impl GraphValueType for F32Type {
    type ValueType = f32;
    type MetadataType = F32Metadata;
    type PreparedShaderType = Buffer;
    type Message = F32Message;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(F32Type)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("f32")
    }

    fn default_literal(&self) -> Self::ValueType {
        0.0
    }

    fn default_metadata(&self) -> Self::MetadataType {
        F32Metadata::default()
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
        data: &Self::ValueType,
        metadata: &Self::MetadataType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        metadata.view(*data, identity, menu())
    }

    fn update_literal(
        &self,
        data: &mut Self::ValueType,
        metadata: &mut Self::MetadataType,
        message: Self::Message,
    ) {
        match message {
            F32Message::Value(value) => *data = value,
            F32Message::Min(min) => metadata.min = min,
            F32Message::Max(max) => metadata.max = max,
            F32Message::Step(step) => metadata.step = step,
            F32Message::Logarithmic(log) => metadata.logarithmic = log,
            F32Message::AllowBeyondRange(allow) => metadata.allow_beyond_range = allow,
            F32Message::UsePercentagePreset => {
                *metadata = F32Metadata::percentage();
            }
            F32Message::UseZeroToOnePreset => {
                *metadata = F32Metadata::zero_to_one();
            }
        }
    }

    fn literal_to_code(&self, data: &Self::ValueType) -> Option<Expression> {
        Some((*data).into())
    }

    fn serialize_literal(
        &self,
        data: &Self::ValueType,
        _assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        Ok(toml::Value::try_from(data)?)
    }

    fn deserialize_literal(
        &self,
        value: toml::Value,
        _assets: &AssetRegistry,
    ) -> Result<Self::ValueType> {
        Ok(Self::ValueType::deserialize(value)?)
    }
}

macro_rules! integer_metadata {
    ($metadata:ident, $message:ident, $value:ty, $min:expr, $max:expr) => {
        #[derive(Clone)]
        pub struct $metadata {
            pub min: $value,
            pub max: $value,
            pub step: $value,
            pub logarithmic: bool,
            pub allow_beyond_range: bool,
        }

        impl Default for $metadata {
            fn default() -> Self {
                Self::new($min, $max)
            }
        }

        impl $metadata {
            pub fn new(min: $value, max: $value) -> Self {
                Self {
                    min,
                    max,
                    step: 1,
                    logarithmic: false,
                    allow_beyond_range: false,
                }
            }

            pub fn percentage() -> Self {
                Self::new(0, 100)
            }

            pub fn zero_to_one() -> Self {
                Self::new(0, 1)
            }

            pub fn step(mut self, step: $value) -> Self {
                self.step = step;
                self
            }

            pub fn allow_beyond_range(mut self) -> Self {
                self.allow_beyond_range = true;
                self
            }

            pub fn logarithmic(mut self) -> Self {
                self.logarithmic = true;
                self
            }

            fn update(&mut self, data: &mut $value, message: $message) {
                match message {
                    $message::Value(value) => *data = value,
                    $message::Min(min) => self.min = min,
                    $message::Max(max) => self.max = max,
                    $message::Step(step) => self.step = step,
                    $message::Logarithmic(logarithmic) => self.logarithmic = logarithmic,
                    $message::AllowBeyondRange(allow) => self.allow_beyond_range = allow,
                    $message::UsePercentagePreset => *self = Self::percentage(),
                    $message::UseZeroToOnePreset => *self = Self::zero_to_one(),
                }
            }
        }

        #[derive(Clone)]
        pub enum $message {
            Value($value),
            Min($value),
            Max($value),
            Step($value),
            Logarithmic(bool),
            AllowBeyondRange(bool),
            UsePercentagePreset,
            UseZeroToOnePreset,
        }

        scalar_metadata_editor!(
            $metadata,
            $message,
            $value,
            <$value>::MIN,
            <$value>::MAX,
            1,
            <$value>::MAX
        );
    };
}

integer_metadata!(I32Metadata, I32Message, i32, -10, 10);
integer_metadata!(U32Metadata, U32Message, u32, 0, 10);

/// Shader graph value type representing a 32-bit signed integer scalar (`i32`).
#[derive(Default, Clone)]
pub struct I32Type;

impl GraphValueType for I32Type {
    type ValueType = i32;
    type MetadataType = I32Metadata;
    type PreparedShaderType = Buffer;
    type Message = I32Message;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(I32Type)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("i32")
    }

    fn default_literal(&self) -> Self::ValueType {
        0
    }

    fn default_metadata(&self) -> Self::MetadataType {
        I32Metadata::default()
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

    fn literal_to_code(&self, data: &Self::ValueType) -> Option<Expression> {
        Some((*data).into())
    }

    fn view_literal(
        &self,
        data: &Self::ValueType,
        metadata: &Self::MetadataType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        metadata.view(*data, identity, menu())
    }

    fn update_literal(
        &self,
        data: &mut Self::ValueType,
        metadata: &mut Self::MetadataType,
        message: Self::Message,
    ) {
        metadata.update(data, message);
    }

    fn serialize_literal(
        &self,
        data: &Self::ValueType,
        _assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        Ok(toml::Value::try_from(data)?)
    }

    fn deserialize_literal(
        &self,
        value: toml::Value,
        _assets: &AssetRegistry,
    ) -> Result<Self::ValueType> {
        Ok(Self::ValueType::deserialize(value)?)
    }
}

/// Shader graph value type representing a 32-bit unsigned integer scalar (`u32`).
#[derive(Default, Clone)]
pub struct U32Type;

impl GraphValueType for U32Type {
    type ValueType = u32;
    type MetadataType = U32Metadata;
    type PreparedShaderType = Buffer;
    type Message = U32Message;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(U32Type)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("u32")
    }

    fn default_literal(&self) -> Self::ValueType {
        0
    }

    fn default_metadata(&self) -> Self::MetadataType {
        U32Metadata::default()
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

    fn literal_to_code(&self, data: &Self::ValueType) -> Option<Expression> {
        Some((*data).into())
    }

    fn view_literal(
        &self,
        data: &Self::ValueType,
        metadata: &Self::MetadataType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        metadata.view(*data, identity, menu())
    }

    fn update_literal(
        &self,
        data: &mut Self::ValueType,
        metadata: &mut Self::MetadataType,
        message: Self::Message,
    ) {
        metadata.update(data, message);
    }

    fn serialize_literal(
        &self,
        data: &Self::ValueType,
        _assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        Ok(toml::Value::try_from(data)?)
    }

    fn deserialize_literal(
        &self,
        value: toml::Value,
        _assets: &AssetRegistry,
    ) -> Result<Self::ValueType> {
        Ok(Self::ValueType::deserialize(value)?)
    }
}

/// Shader graph value type representing a boolean scalar (`bool`, stored as `u32` in GPU buffers).
#[derive(Default, Clone)]
pub struct BoolType;

impl GraphValueType for BoolType {
    type ValueType = bool;
    type MetadataType = ();
    type PreparedShaderType = Buffer;

    type Message = bool;

    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BoolType)
    }

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("bool")
    }

    fn default_literal(&self) -> Self::ValueType {
        false
    }

    fn default_metadata(&self) -> Self::MetadataType {}

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

    fn literal_to_code(&self, data: &Self::ValueType) -> Option<Expression> {
        Some((*data).into())
    }

    fn view_literal(
        &self,
        data: &Self::ValueType,
        _metadata: &Self::MetadataType,
        _assets: &AssetRegistry,
    ) -> Element<'static, Self::Message, GraphTheme, GraphRenderer> {
        checkbox(*data).on_toggle(identity).into()
    }

    fn update_literal(
        &self,
        data: &mut Self::ValueType,
        _metadata: &mut Self::MetadataType,
        message: Self::Message,
    ) {
        *data = message;
    }

    fn serialize_literal(
        &self,
        data: &Self::ValueType,
        _assets: &AssetRegistry,
    ) -> Result<toml::Value> {
        Ok(toml::Value::try_from(data)?)
    }

    fn deserialize_literal(
        &self,
        value: toml::Value,
        _assets: &AssetRegistry,
    ) -> Result<Self::ValueType> {
        Ok(Self::ValueType::deserialize(value)?)
    }
}
