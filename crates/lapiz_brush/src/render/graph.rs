use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, LazyLock},
};

use anyhow::{Result, bail};
use glam::Vec4;
use iced_core::widget::Void;
use lapiz_assets::store::AssetRegistry;
use lapiz_effect::nodes::effect_nodes;
use lapiz_image::{blend_modes::BlendMode, texel::TexelType};
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::{
        DynamicBindGroupLayoutEntries, binding_types::storage_buffer_read_only_sized,
    },
};
use lapiz_shader_graph::{
    GraphElement,
    graph::{
        GraphResources,
        node::{
            ExtraShaderBody, GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError,
            GraphNodeCreateSlotsContext, GraphNodeDefaultStateContext,
            GraphNodeInjectShaderBodyContext, GraphNodeRegistry, GraphNodeUpdateContext,
            GraphNodeViewContext, StatelessCommonGraphNode, stateless,
        },
        slot::{
            ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphDefaultInputSlot,
            GraphDefaultOutputSlot, GraphShaderStage, GraphValueType,
        },
        variable::GraphTypeRegistry,
    },
    save::GraphValueTypeId,
    wgsl_std::{
        builtin_types,
        types::{
            compound::{ColorType, RectType},
            handle::LayerType,
            layer_load_ident,
            primitive::{F32Type, I32Type, U32Type},
            vector::Vec2FType,
        },
    },
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::combo_box;
use serde::{Deserialize, Serialize};
use wesl::syntax::{
    BinaryExpression, BinaryOperator, Declaration, DeclarationKind, Expression, FunctionCall,
    Ident, ModulePath, NamedComponentExpression, PathOrigin, Span, Spanned, Statement,
    TypeExpression,
};
use wesl_quote::quote_statement;
use wgpu::{Buffer, Device, Queue, util::DeviceExt as _};

use super::ComputedPenInput;

/// Foreground and background RGBA colors supplied as built-in uniforms when rendering a brush stroke.
#[derive(Debug, Default, Clone, Copy)]
pub struct CanvasResources {
    pub foreground_color: Vec4,
    pub background_color: Vec4,
}

pub const SPACING_OUTPUT: &str = "spacing";
pub const MAIN_DAB_BUFFER: &str = "main_dab";
pub const STROKE_RESULT: &str = "stroke_result";

pub const FOREGROUND_COLOR_BUILTIN: &str = "foreground_color";
pub const BACKGROUND_COLOR_BUILTIN: &str = "background_color";
pub const TARGET_LAYER_BUILTIN: &str = "target_layer";
pub const SELECTION_BUILTIN: &str = "selection";
pub const HAS_SELECTION_BUILTIN: &str = "has_selection";
pub const BRUSH_SAMPLE_BUILTIN: &str = "brush_sample";
pub const INITIAL_PEN_INPUT_BUILTIN: &str = "initial_pen_input";
pub const MAIN_ACCUMULATE_BUFFER: &str = "main_accumulate";

/// Shader graph value type representing a [`ComputedPenInput`] storage buffer binding.
#[derive(Default, Clone)]
pub struct ComputedPenInputValueType;

impl GraphValueType for ComputedPenInputValueType {
    type AssociatedLiteralType = ComputedPenInput;
    type PreparedShaderType = Buffer;
    type Message = ();

    fn id(&self) -> GraphValueTypeId {
        GraphValueTypeId::new("brush_computed_pen_input")
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
            bail!("computed pen input can only be a shader input");
        }
        shader.push_str(&format!(
            "@group({group}) @binding({binding}) var<storage, read> {name}: brush::brush_types::ComputedPenInput;\n"
        ));
        let bindings =
            bindings.extend_with_indices(((binding, storage_buffer_read_only_sized(false, None)),));
        Ok((binding + 1, bindings, shader))
    }

    fn push_shader_binding<'a>(
        &self,
        _stage: GraphShaderStage,
        value: &'a wgpu::Buffer,
        binding: u32,
        bindings: DynamicBindGroupEntries<'a>,
    ) -> Result<(u32, DynamicBindGroupEntries<'a>)> {
        Ok((
            binding + 1,
            bindings.extend_with_indices(((binding, value.as_entire_binding()),)),
        ))
    }

    fn prepare_to_shader(
        &self,
        data: &ComputedPenInput,
        device: &Device,
        _queue: &Queue,
    ) -> Result<Buffer> {
        let mut storage = encase::StorageBuffer::new(Vec::new());
        storage.write(data)?;
        Ok(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("brush computed pen input"),
                contents: storage.as_ref(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            }),
        )
    }

    fn default_literal(&self) -> ComputedPenInput {
        ComputedPenInput::default()
    }

    fn wgsl_type_name(&self) -> Option<&'static str> {
        None
    }
    fn hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ComputedPenInputValueType)
    }
    fn view_literal(
        &self,
        _: &ComputedPenInput,
        _assets: &AssetRegistry,
    ) -> GraphElement<'static, ()> {
        Void.into()
    }
    fn update_literal(&self, _: &mut ComputedPenInput, _: ()) {}
    fn literal_to_code(&self, _: &ComputedPenInput) -> Option<Expression> {
        None
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

struct BrushNodeShaderBody {
    node: &'static str,
    source: &'static str,
}

impl ExtraShaderBody for BrushNodeShaderBody {
    fn key(&self) -> String {
        self.node.into()
    }

    fn inject_body(&self, shader: &mut String) {
        shader.push('\n');
        shader.push_str(self.source);
    }
}

/// Shader graph node that outputs the current dab's canvas pixel position (`brush_sample.position`).
#[derive(Default, Clone)]
pub struct PenPositionNode;

#[stateless]
impl StatelessCommonGraphNode for PenPositionNode {
    fn id(&self) -> &'static str {
        "pen_position_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PenPositionNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("position".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = brush_sample.position;\n",
            ctx.get_output(0)?
        ))
    }
}

/// Shader graph node that outputs the current dab's normalized pen pressure (`brush_sample.pressure`).
#[derive(Default, Clone)]
pub struct PenPressureNode;

#[stateless]
impl StatelessCommonGraphNode for PenPressureNode {
    fn id(&self) -> &'static str {
        "pen_pressure_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PenPressureNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("pressure".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = brush_sample.pressure;\n",
            ctx.get_output(0)?
        ))
    }
}

/// Shader graph node that outputs the current dab's pen tilt vector (`brush_sample.tilt`).
#[derive(Default, Clone)]
pub struct PenTiltNode;

#[stateless]
impl StatelessCommonGraphNode for PenTiltNode {
    fn id(&self) -> &'static str {
        "pen_tilt_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PenTiltNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("tilt".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!("let {} = brush_sample.tilt;\n", ctx.get_output(0)?))
    }
}

/// Shader graph node that outputs the current dab's pen altitude and azimuth angles.
#[derive(Default, Clone)]
pub struct PenAngleNode;

#[stateless]
impl StatelessCommonGraphNode for PenAngleNode {
    fn id(&self) -> &'static str {
        "pen_angle_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PenAngleNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("altitude".into()),
            GraphDefaultOutputSlot::new::<F32Type>("azimuth".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = brush_sample.angle.x;\nlet {} = brush_sample.angle.y;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

/// Shader graph node that outputs the current dab's stroke tangent angle and unit direction vector.
#[derive(Default, Clone)]
pub struct DrawDirectionNode;

#[stateless]
impl StatelessCommonGraphNode for DrawDirectionNode {
    fn id(&self) -> &'static str {
        "draw_direction_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(DrawDirectionNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("angle".into()),
            GraphDefaultOutputSlot::new::<Vec2FType>("direction".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = brush_sample.draw_direction_angle;\nlet {} = brush_sample.draw_direction_vec;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

/// Shader graph node that outputs the sequential index of the current dab within the stroke.
#[derive(Default, Clone)]
pub struct DabIndexNode;

#[stateless]
impl StatelessCommonGraphNode for DabIndexNode {
    fn id(&self) -> &'static str {
        "dab_index_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(DabIndexNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<I32Type>("index".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = i32(brush_sample.dab_index);",
            ctx.get_output(0)?
        ))
    }
}

/// Shader graph node that outputs the cumulative arc-length distance traveled along the stroke.
#[derive(Default, Clone)]
pub struct StrokeDistanceNode;

#[stateless]
impl StatelessCommonGraphNode for StrokeDistanceNode {
    fn id(&self) -> &'static str {
        "stroke_distance_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(StrokeDistanceNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("distance".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = brush_sample.stroke_distance;",
            ctx.get_output(0)?
        ))
    }
}

/// Shader graph node that outputs the pen position recorded at the start of the stroke.
#[derive(Default, Clone)]
pub struct InitialPenPositionNode;

#[stateless]
impl StatelessCommonGraphNode for InitialPenPositionNode {
    fn id(&self) -> &'static str {
        "initial_pen_position_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialPenPositionNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("position".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = initial_pen_input.position;",
            ctx.get_output(0)?
        ))
    }
}

/// Shader graph node that outputs the pen pressure recorded at the start of the stroke.
#[derive(Default, Clone)]
pub struct InitialPenPressureNode;

#[stateless]
impl StatelessCommonGraphNode for InitialPenPressureNode {
    fn id(&self) -> &'static str {
        "initial_pen_pressure_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialPenPressureNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("pressure".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = initial_pen_input.pressure;\n",
            ctx.get_output(0)?
        ))
    }
}

/// Shader graph node that outputs the pen tilt vector recorded at the start of the stroke.
#[derive(Default, Clone)]
pub struct InitialPenTiltNode;

#[stateless]
impl StatelessCommonGraphNode for InitialPenTiltNode {
    fn id(&self) -> &'static str {
        "initial_pen_tilt_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialPenTiltNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("tilt".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = initial_pen_input.tilt;\n",
            ctx.get_output(0)?
        ))
    }
}

/// Shader graph node that outputs the pen altitude and azimuth angles recorded at the start of the stroke.
#[derive(Default, Clone)]
pub struct InitialPenAngleNode;

#[stateless]
impl StatelessCommonGraphNode for InitialPenAngleNode {
    fn id(&self) -> &'static str {
        "initial_pen_angle_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialPenAngleNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("altitude".into()),
            GraphDefaultOutputSlot::new::<F32Type>("azimuth".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = initial_pen_input.angle.x;\nlet {} = initial_pen_input.angle.y;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

/// Shader graph node that outputs the initial stroke tangent angle and direction vector.
#[derive(Default, Clone)]
pub struct InitialDrawDirectionNode;

#[stateless]
impl StatelessCommonGraphNode for InitialDrawDirectionNode {
    fn id(&self) -> &'static str {
        "initial_draw_direction_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialDrawDirectionNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("angle".into()),
            GraphDefaultOutputSlot::new::<Vec2FType>("direction".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = initial_pen_input.draw_direction_angle;\nlet {} = initial_pen_input.draw_direction_vec;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

/// Shader graph node that outputs the current sample timestamp and stroke start timestamp.
#[derive(Default, Clone)]
pub struct TimeNode;

#[stateless]
impl StatelessCommonGraphNode for TimeNode {
    fn id(&self) -> &'static str {
        "time_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(TimeNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("now".into()),
            GraphDefaultOutputSlot::new::<F32Type>("stroke_begin".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = brush_sample.time.now;\nlet {} = brush_sample.time.stroke_begin;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

/// Shader graph node that outputs the 2D canvas pixel coordinate of the current compute invocation.
#[derive(Default, Clone)]
pub struct PixelPositionNode;

#[stateless]
impl StatelessCommonGraphNode for PixelPositionNode {
    fn id(&self) -> &'static str {
        "pixel_position_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PixelPositionNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("position".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = vec2f(dispatch_index);",
            ctx.get_output(0)?
        ))
    }
}

/// Shader graph node that masks a color to zero outside a given bounding rectangle.
#[derive(Default, Clone)]
pub struct FilterWithinBoundsNode;

#[stateless]
impl StatelessCommonGraphNode for FilterWithinBoundsNode {
    fn extra_shader_body(&self, mut ctx: GraphNodeInjectShaderBodyContext<'_>) {
        ctx.inject_extra_body(BrushNodeShaderBody {
            node: StatelessCommonGraphNode::id(self),
            source: include_str!("graph/filter_within_bounds.wesl"),
        });
    }

    fn id(&self) -> &'static str {
        "filter_within_bounds_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(FilterWithinBoundsNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("color".into()),
            GraphDefaultInputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<ColorType>("color".into()),
            GraphDefaultOutputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let bounds = ctx.get_input(1)?;
        let output_color = ctx.get_output(0)?;
        let output_bounds = ctx.get_output(1)?;
        Ok(format!(
            "{}\n{}\n",
            quote_statement! {
                let #output_color = filter_within_bounds(dispatch_index, #color, #bounds);
            },
            quote_statement! {
                let #output_bounds = #bounds;
            },
        ))
    }
}

/// Shader graph node that blends a source and destination color using a selectable [`BlendMode`].
#[derive(Default, Clone)]
pub struct BlendColorNode;

/// Persisted state for [`BlendColorNode`], storing the selected [`BlendMode`].
#[derive(Clone, Serialize, Deserialize)]
pub struct BlendColorNodeState {
    pub blend_mode: BlendMode,
}

/// UI messages emitted by blend-mode shader graph nodes.
#[derive(Clone)]
pub enum BlendModeNodeMessage {
    ModeChanged(BlendMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for BlendColorNode {
    type State = BlendColorNodeState;
    type Message = BlendModeNodeMessage;

    fn id(&self) -> &'static str {
        "blend_color_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        BlendColorNodeState {
            blend_mode: BlendMode::Normal,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BlendColorNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("src_color".into()),
            GraphDefaultInputSlot::new::<ColorType>("dst_color".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        ctx.view_all_slots_with_header(
            combo_box(
                BlendMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(BlendModeOption)
                    .collect::<Vec<_>>(),
                Some(BlendModeOption(state.blend_mode)),
                |option| BlendModeNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(iced_core::Length::Fill),
            BlendModeNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            BlendModeNodeMessage::ModeChanged(mode) => state.blend_mode = mode,
            BlendModeNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let src = ctx.get_input(0)?;
        let dst = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {output} = image::blend_modes::{}({src}, {dst});\n",
            state.blend_mode.shader_func(),
        ))
    }
}

/// Shader graph node that blends an input color and opacity over the current pixel in the stroke accumulation buffer.
#[derive(Default, Clone)]
pub struct BlendWithInputNode;

/// Persisted state for [`BlendWithInputNode`] and [`BlendWithLayerNode`], storing the selected [`BlendMode`].
#[derive(Clone, Serialize, Deserialize)]
pub struct BlendWithBufferNodeState {
    pub blend_mode: BlendMode,
}

impl GraphNode for BlendWithInputNode {
    type State = BlendWithBufferNodeState;
    type Message = BlendModeNodeMessage;

    fn id(&self) -> &'static str {
        "blend_with_input_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        BlendWithBufferNodeState {
            blend_mode: BlendMode::Normal,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BlendWithInputNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("color".into()),
            GraphDefaultInputSlot::new::<F32Type>("opacity".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        ctx.view_all_slots_with_header(
            combo_box(
                BlendMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(BlendModeOption)
                    .collect::<Vec<_>>(),
                Some(BlendModeOption(state.blend_mode)),
                |option| BlendModeNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(iced_core::Length::Fill),
            BlendModeNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            BlendModeNodeMessage::ModeChanged(mode) => state.blend_mode = mode,
            BlendModeNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let opacity = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {output} = image::blend_modes::{}(vec4f({color}.rgb, {color}.a * {opacity}), main_accumulate_load(dispatch_index));\n",
            state.blend_mode.shader_func(),
        ))
    }
}

/// Shader graph node that blends an input color and opacity over the current pixel of the target canvas layer.
#[derive(Default, Clone)]
pub struct BlendWithLayerNode;

impl GraphNode for BlendWithLayerNode {
    type State = BlendWithBufferNodeState;
    type Message = BlendModeNodeMessage;

    fn id(&self) -> &'static str {
        "blend_with_layer_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        BlendWithBufferNodeState {
            blend_mode: BlendMode::Normal,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BlendWithLayerNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("color".into()),
            GraphDefaultInputSlot::new::<F32Type>("opacity".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'a>,
    ) -> GraphElement<'a, Self::Message> {
        ctx.view_all_slots_with_header(
            combo_box(
                BlendMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(BlendModeOption)
                    .collect::<Vec<_>>(),
                Some(BlendModeOption(state.blend_mode)),
                |option| BlendModeNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(iced_core::Length::Fill),
            BlendModeNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            BlendModeNodeMessage::ModeChanged(mode) => state.blend_mode = mode,
            BlendModeNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let opacity = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;
        let blend = Ident::new(state.blend_mode.shader_func().to_string());
        let target_layer_load = Ident::new(layer_load_ident(TARGET_LAYER_BUILTIN));
        Ok(quote_statement! {
            let #output = image::blend_modes::#blend(
                vec4f(#color.rgb, #color.a * #opacity),
                #target_layer_load(dispatch_index),
            );
        }
        .to_string())
    }
}

/// Display wrapper around [`BlendMode`] for combo box selection widgets in blend nodes.
#[derive(Clone)]
pub struct BlendModeOption(BlendMode);

impl BlendModeOption {
    fn into_inner(self) -> BlendMode {
        self.0
    }
}

impl fmt::Display for BlendModeOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl PartialEq for BlendModeOption {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// Shader graph node that samples a pixel color from the target canvas layer at a given position.
#[derive(Default, Clone)]
pub struct LayerPixelColorNode;

#[stateless]
impl StatelessCommonGraphNode for LayerPixelColorNode {
    fn id(&self) -> &'static str {
        "layer_pixel_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(LayerPixelColorNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("position".into())]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let output = ctx.get_output(0)?;
        let load = Ident::new(layer_load_ident(TARGET_LAYER_BUILTIN));
        let position = ctx.get_input(0)?;
        Ok(quote_statement! {
            let #output = #load(vec2i(#position));
        }
        .to_string())
    }
}

/// Shader graph node that samples a pixel color from the stroke accumulation buffer at a given position.
#[derive(Default, Clone)]
pub struct CurrentPixelColorNode;

#[stateless]
impl StatelessCommonGraphNode for CurrentPixelColorNode {
    fn id(&self) -> &'static str {
        "current_pixel_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CurrentPixelColorNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("position".into())]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = main_accumulate_load(vec2i({}));\n",
            ctx.get_output(0)?,
            ctx.get_input(0)?
        ))
    }
}

/// Shader graph node that outputs the tile-aligned pixel bounding rectangle of the stroke accumulation buffer.
#[derive(Default, Clone)]
pub struct StrokeBoundsNode;

#[stateless]
impl StatelessCommonGraphNode for StrokeBoundsNode {
    fn id(&self) -> &'static str {
        "stroke_bounds_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(StrokeBoundsNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<RectType>("bounds".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {output}_tile_size = f32(image::image_tiling::TILE_SIZE);\n\
             let {output} = render::math::Rect(\n\
                 floor(vec2f(main_accumulate_bounds.xy) / {output}_tile_size) * {output}_tile_size,\n\
                 ceil(vec2f(main_accumulate_bounds.zw) / {output}_tile_size) * {output}_tile_size,\n\
             );"
        ))
    }
}

/// Shader graph node that unions a dab bounding rectangle with the current stroke accumulation bounds.
#[derive(Default, Clone)]
pub struct AccumulateBoundsNode;

#[stateless]
impl StatelessCommonGraphNode for AccumulateBoundsNode {
    fn id(&self) -> &'static str {
        "accumulate_bounds_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(AccumulateBoundsNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<RectType>("dab bounds".into())]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<RectType>(
            "stroke bounds".into(),
        )]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let bounds = ctx.get_input(0)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {output} = render::math::Rect(min(({bounds}).min, vec2f(main_accumulate_bounds.xy)), max(({bounds}).max, vec2f(main_accumulate_bounds.zw)));"
        ))
    }
}

/// Shader graph node that evaluates a rotated elliptical falloff mask and its bounding rectangle.
#[derive(Default, Clone)]
pub struct EllipticalMaskNode;

#[stateless]
impl StatelessCommonGraphNode for EllipticalMaskNode {
    fn extra_shader_body(&self, mut ctx: GraphNodeInjectShaderBodyContext<'_>) {
        ctx.inject_extra_body(BrushNodeShaderBody {
            node: StatelessCommonGraphNode::id(self),
            source: include_str!("graph/elliptical_mask.wesl"),
        });
    }

    fn id(&self) -> &'static str {
        "elliptical_mask_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(EllipticalMaskNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<Vec2FType>("sample_position".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("center".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("radii".into()),
            GraphDefaultInputSlot::new::<F32Type>("rotation".into()),
        ]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("mask_value".into()),
            GraphDefaultOutputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let mask = ctx.ident_generator.next_output();
        Ok(format!(
            "let {mask} = elliptical_mask({}, {}, {}, {});\nlet {} = {mask}.value;\nlet {} = {mask}.bounds;\n",
            ctx.get_input(0)?,
            ctx.get_input(1)?,
            ctx.get_input(2)?,
            ctx.get_input(3)?,
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

/// Shader graph node that samples the canvas selection mask value at a given pixel position.
#[derive(Default, Clone)]
pub struct SelectionMaskNode;

#[stateless]
impl StatelessCommonGraphNode for SelectionMaskNode {
    fn id(&self) -> &'static str {
        "selection_mask_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SelectionMaskNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("position".into())]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("value".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input = ctx.get_input(0)?;
        let output = ctx.get_output(0)?;
        let load = Ident::new(layer_load_ident(SELECTION_BUILTIN));
        Ok(quote_statement! {
            let #output = #load(vec2i(#input));
        }
        .to_string())
    }
}

/// Shader graph node that outputs the active foreground color uniform.
#[derive(Default, Clone)]
pub struct ForegroundColorNode;

#[stateless]
impl StatelessCommonGraphNode for ForegroundColorNode {
    fn id(&self) -> &'static str {
        "foreground_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ForegroundColorNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!("let {} = foreground_color;\n", ctx.get_output(0)?))
    }
}

/// Shader graph node that outputs the active background color uniform.
#[derive(Default, Clone)]
pub struct BackgroundColorNode;

#[stateless]
impl StatelessCommonGraphNode for BackgroundColorNode {
    fn id(&self) -> &'static str {
        "background_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BackgroundColorNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!("let {} = background_color;\n", ctx.get_output(0)?))
    }
}

pub static BRUSH_GRAPH_TYPES: LazyLock<Arc<GraphTypeRegistry>> =
    LazyLock::new(|| Arc::new(brush_graph_types()));

fn brush_graph_types() -> GraphTypeRegistry {
    let mut types = builtin_types();
    types.register_type::<ComputedPenInputValueType>();
    types
}

pub fn spacing_graph_nodes() -> GraphNodeRegistry {
    let mut nodes = effect_nodes();
    nodes.register::<PenPositionNode>();
    nodes.register::<PenPressureNode>();
    nodes.register::<PenAngleNode>();
    nodes.register::<PenTiltNode>();
    nodes.register::<DrawDirectionNode>();
    nodes.register::<DabIndexNode>();
    nodes.register::<InitialPenPositionNode>();
    nodes.register::<InitialPenPressureNode>();
    nodes.register::<InitialPenAngleNode>();
    nodes.register::<InitialPenTiltNode>();
    nodes.register::<InitialDrawDirectionNode>();
    nodes.register::<TimeNode>();
    nodes.register::<ForegroundColorNode>();
    nodes.register::<BackgroundColorNode>();
    nodes
}

pub fn main_graph_nodes() -> GraphNodeRegistry {
    let mut nodes = effect_nodes();
    nodes.register::<PenPositionNode>();
    nodes.register::<PenPressureNode>();
    nodes.register::<PenAngleNode>();
    nodes.register::<PenTiltNode>();
    nodes.register::<DrawDirectionNode>();
    nodes.register::<DabIndexNode>();
    nodes.register::<StrokeDistanceNode>();
    nodes.register::<InitialPenPositionNode>();
    nodes.register::<InitialPenPressureNode>();
    nodes.register::<InitialPenAngleNode>();
    nodes.register::<InitialPenTiltNode>();
    nodes.register::<InitialDrawDirectionNode>();
    nodes.register::<TimeNode>();
    nodes.register::<PixelPositionNode>();
    nodes.register::<FilterWithinBoundsNode>();
    nodes.register::<BlendColorNode>();
    nodes.register::<LayerPixelColorNode>();
    nodes.register::<CurrentPixelColorNode>();
    nodes.register::<StrokeBoundsNode>();
    nodes.register::<AccumulateBoundsNode>();
    nodes.register::<EllipticalMaskNode>();
    nodes.register::<BlendWithInputNode>();
    nodes.register::<BlendWithLayerNode>();
    nodes.register::<SelectionMaskNode>();
    nodes.register::<ForegroundColorNode>();
    nodes.register::<BackgroundColorNode>();
    nodes
}

pub fn postprocess_graph_nodes() -> GraphNodeRegistry {
    let mut nodes = effect_nodes();
    nodes.register::<PixelPositionNode>();
    nodes.register::<FilterWithinBoundsNode>();
    nodes.register::<BlendColorNode>();
    nodes.register::<LayerPixelColorNode>();
    nodes.register::<CurrentPixelColorNode>();
    nodes.register::<StrokeBoundsNode>();
    nodes.register::<EllipticalMaskNode>();
    nodes.register::<BlendWithInputNode>();
    nodes.register::<BlendWithLayerNode>();
    nodes.register::<SelectionMaskNode>();
    nodes.register::<ForegroundColorNode>();
    nodes.register::<BackgroundColorNode>();
    nodes
}

pub fn brush_builtin_types(
    target_layer_format: TexelType,
    selection_layer_format: TexelType,
) -> BTreeMap<String, Arc<dyn ErasedGraphValueType>> {
    BTreeMap::from([
        (
            FOREGROUND_COLOR_BUILTIN.to_string(),
            Arc::new(ColorType) as Arc<dyn ErasedGraphValueType>,
        ),
        (BACKGROUND_COLOR_BUILTIN.to_string(), Arc::new(ColorType)),
        (HAS_SELECTION_BUILTIN.to_string(), Arc::new(U32Type)),
        (
            SELECTION_BUILTIN.to_string(),
            Arc::new(LayerType {
                texel_type: selection_layer_format,
            }),
        ),
        (
            TARGET_LAYER_BUILTIN.to_string(),
            Arc::new(LayerType {
                texel_type: target_layer_format,
            }),
        ),
    ])
}

pub fn main_builtin_types(
    target_layer_format: TexelType,
    selection_layer_format: TexelType,
) -> BTreeMap<String, Arc<dyn ErasedGraphValueType>> {
    let mut types = brush_builtin_types(target_layer_format, selection_layer_format);
    types.insert(
        BRUSH_SAMPLE_BUILTIN.into(),
        Arc::new(ComputedPenInputValueType),
    );
    types.insert(
        INITIAL_PEN_INPUT_BUILTIN.into(),
        Arc::new(ComputedPenInputValueType),
    );
    types.insert(
        MAIN_ACCUMULATE_BUFFER.into(),
        Arc::new(LayerType {
            texel_type: target_layer_format,
        }),
    );
    types
}

pub fn postprocess_builtin_types(
    target_layer_format: TexelType,
    selection_layer_format: TexelType,
) -> BTreeMap<String, Arc<dyn ErasedGraphValueType>> {
    let mut types = brush_builtin_types(target_layer_format, selection_layer_format);
    types.insert(
        MAIN_ACCUMULATE_BUFFER.into(),
        Arc::new(LayerType {
            texel_type: target_layer_format,
        }),
    );
    types
}

pub fn brush_graph_resources(
    registry: Arc<GraphNodeRegistry>,
    assets: AssetRegistry,
) -> GraphResources {
    GraphResources {
        type_registry: BRUSH_GRAPH_TYPES.clone(),
        node_registry: registry,
        assets,
    }
}
