use lapiz_image::texel::TexelType;

use crate::shader_graph::graph::{node::GraphNodeRegistry, variable::GraphTypeRegistry};

pub mod casters;
pub mod nodes;
pub mod types;

#[macro_export]
macro_rules! random_oklch_hue_chroma {
    ($struct_name:ty) => {{
        const CH: (f32, f32) = {
            let name = stringify!($struct_name).as_bytes();
            let mut i = 0;
            let mut hash = 0u32;
            while i < name.len() {
                hash = (hash << 5).wrapping_sub(hash) + name[i] as u32;
                i += 1;
            }

            const MIN_C: f32 = 0.05;
            const MAX_C: f32 = 0.15;
            let h = (hash % 360) as f32;
            let c = MIN_C + (hash as f32 / u32::MAX as f32) * (MAX_C - MIN_C);
            (h, c)
        };

        CH
    }};
}

pub fn builtin_nodes() -> GraphNodeRegistry {
    use nodes::{
        components::{
            CombineColorComponentsNode, CombineComponentsNode, SplitColorComponentsNode,
            SplitComponentsNode,
        },
        curve::CurveNode,
        custom_expression::CustomExpressionNode,
        function::GraphFunctionNode,
        image::{ColorMixNode, GetPixelColorNode, TextureSizeNode},
        math::{ClampNode, RectMathNode, ScalarMathNode, SmoothStepNode, StepNode, VectorMathNode},
        random::RandomNode,
        repeat::RepeatNode,
        selection::{CompareNode, ScalarSelectNode, VectorSelectNode},
    };

    let mut nodes = GraphNodeRegistry::with_capacity();

    nodes.register::<ScalarMathNode>();
    nodes.register::<VectorMathNode>();
    nodes.register::<RectMathNode>();
    nodes.register::<CompareNode>();
    nodes.register::<ScalarSelectNode>();
    nodes.register::<VectorSelectNode>();
    nodes.register::<ClampNode>();
    nodes.register::<RandomNode>();
    nodes.register::<StepNode>();
    nodes.register::<SmoothStepNode>();
    nodes.register::<SplitComponentsNode>();
    nodes.register::<CombineComponentsNode>();
    nodes.register::<SplitColorComponentsNode>();
    nodes.register::<CombineColorComponentsNode>();
    nodes.register::<GetPixelColorNode>();
    nodes.register::<ColorMixNode>();
    nodes.register::<TextureSizeNode>();
    nodes.register::<GraphFunctionNode>();
    nodes.register::<CurveNode>();
    nodes.register::<RepeatNode>();
    nodes.register::<CustomExpressionNode>();

    nodes
}

pub fn builtin_types() -> GraphTypeRegistry {
    use casters::{
        BoolToI32Caster, F32ToI32Caster, F32ToVec2FCaster, I32ToBoolCaster, I32ToF32Caster,
        I32ToVec2FCaster, Vec2FToF32Caster, Vec2FToI32Caster,
    };
    use types::{
        atomic::{AtomicI32Type, AtomicU32Type},
        compound::{ColorType, RectType},
        handle::{LayerType, TextureType},
        primitive::{BoolType, F32Type, I32Type, U32Type},
        vector::{
            Vec2FType, Vec2IType, Vec2UType, Vec3FType, Vec3IType, Vec3UType, Vec4FType, Vec4IType,
            Vec4UType,
        },
    };

    let mut types = GraphTypeRegistry::default();

    types.register_type::<F32Type>();
    types.register_type::<I32Type>();
    types.register_type::<U32Type>();
    types.register_type::<BoolType>();
    types.register_type::<AtomicI32Type>();
    types.register_type::<AtomicU32Type>();
    types.register_type::<Vec2FType>();
    types.register_type::<Vec3FType>();
    types.register_type::<Vec4FType>();
    types.register_type::<Vec2IType>();
    types.register_type::<Vec3IType>();
    types.register_type::<Vec4IType>();
    types.register_type::<Vec2UType>();
    types.register_type::<Vec3UType>();
    types.register_type::<Vec4UType>();
    types.register_type::<ColorType>();
    types.register_type::<RectType>();

    for texel_type in TexelType::ALL_POSSIBLE_FORMATS {
        types.register_type_value(TextureType { texel_type });
        types.register_type_value(LayerType { texel_type });
    }

    types.register_caster::<F32ToVec2FCaster>();
    types.register_caster::<Vec2FToF32Caster>();
    types.register_caster::<BoolToI32Caster>();
    types.register_caster::<I32ToBoolCaster>();
    types.register_caster::<F32ToI32Caster>();
    types.register_caster::<I32ToF32Caster>();
    types.register_caster::<Vec2FToI32Caster>();
    types.register_caster::<I32ToVec2FCaster>();

    types
}
