use std::sync::{Arc, LazyLock};

use lapiz_image::blend_modes::BlendMode;
use lapiz_shader_graph::{
    graph::{
        GraphData, GraphResources,
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeRegistry, GraphNodeUpdateContext,
            GraphNodeUpdateSignatureContext, GraphNodeViewContext, StatelessCommonGraphNode,
            stateless,
        },
        slot::{ErasedGraphLiteralUpdateMessage, GraphDefaultInputSlot, GraphDefaultOutputSlot},
        variable::GraphTypeRegistry,
    },
    wgsl_std::{
        builtin_nodes, builtin_types,
        types::{ColorType, F32Type, RectType, Vec2FType},
    },
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::combo_box::ComboBox;
use serde::{Deserialize, Serialize};

#[derive(Default, Clone)]
pub struct FilterGraphData {
    pub resources: GraphResources<FilterGraphData>,
}

impl GraphData for FilterGraphData {}

#[derive(Default, Clone)]
pub struct PixelPositionNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for PixelPositionNode {
    fn id(&self) -> &'static str {
        "pixel_position_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PixelPositionNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("position".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!("let {} = pixel_posf;", ctx.get_output(0)?))
    }
}

#[derive(Default, Clone)]
pub struct InputColorNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for InputColorNode {
    fn id(&self) -> &'static str {
        "input_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InputColorNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = load_input_color(pixel_pos);",
            ctx.get_output(0)?
        ))
    }
}

#[derive(Default, Clone)]
pub struct SampleInputColorNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for SampleInputColorNode {
    fn id(&self) -> &'static str {
        "sample_input_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SampleInputColorNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("position".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let position = ctx.get_input(0)?;
        Ok(format!(
            "let {} = load_input_color(vec2i({}));",
            ctx.get_output(0)?,
            position
        ))
    }
}

#[derive(Default, Clone)]
pub struct InputBoundsNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for InputBoundsNode {
    fn id(&self) -> &'static str {
        "input_bounds_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InputBoundsNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<RectType>("rectangle".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!("let {} = input_bounds;", ctx.get_output(0)?))
    }
}

#[derive(Default, Clone)]
pub struct OutputColorNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for OutputColorNode {
    fn id(&self) -> &'static str {
        "output_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(OutputColorNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<ColorType>("color".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![]
    }

    fn generate_code(
        &self,
        ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        // set_output_color is a no-op during bounds evaluation (see template).
        Ok(format!(
            "set_output_color(pixel_pos, {});\n",
            ctx.get_input(0)?
        ))
    }
}

#[derive(Default, Clone)]
pub struct OutputBoundsNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for OutputBoundsNode {
    fn id(&self) -> &'static str {
        "output_bounds_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(OutputBoundsNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<RectType>("bounds".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![]
    }

    fn update_signature(&self, mut ctx: GraphNodeUpdateSignatureContext<'_, Data>) {
        ctx.require_input_slot_as_graph_output(0, "Bounds".to_string());
    }

    fn generate_code(
        &self,
        ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        // set_output_bounds is a no-op outside bounds evaluation (see template).
        Ok(format!("set_output_bounds({});", ctx.get_input(0)?))
    }
}

#[derive(Default, Clone)]
pub struct SelectionMaskNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for SelectionMaskNode {
    fn id(&self) -> &'static str {
        "selection_mask_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(SelectionMaskNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("position".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("value".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input = ctx.get_input(0)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {output} = load_selection_mask_value(vec2i({input}));\n"
        ))
    }
}

#[derive(Default, Clone)]
pub struct LayerPixelColorNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for LayerPixelColorNode {
    fn id(&self) -> &'static str {
        "layer_pixel_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(LayerPixelColorNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("position".into())]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input = ctx.get_input(0)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {output} = target_layer_color(vec2i({input}));\n"
        ))
    }
}

#[derive(Default, Clone)]
pub struct BlendWithLayerNode;

#[derive(Clone, Serialize, Deserialize)]
pub struct BlendWithLayerNodeState {
    pub blend_mode: BlendMode,
}

#[derive(Clone)]
pub enum BlendWithLayerNodeMessage {
    ModeChanged(BlendMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for BlendWithLayerNode {
    type State = BlendWithLayerNodeState;
    type Message = BlendWithLayerNodeMessage;

    fn id(&self) -> &'static str {
        "blend_with_layer_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        BlendWithLayerNodeState {
            blend_mode: BlendMode::Normal,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BlendWithLayerNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("color".into()),
            GraphDefaultInputSlot::new::<F32Type>("opacity".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> lapiz_shader_graph::GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
                BlendMode::ALL,
                Some(state.blend_mode),
                BlendWithLayerNodeMessage::ModeChanged,
            )
            .width(iced_core::Length::Fill),
            BlendWithLayerNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            BlendWithLayerNodeMessage::ModeChanged(mode) => state.blend_mode = mode,
            BlendWithLayerNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let opacity = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {output} = image::blend_modes::{}(vec4f({color}.rgb, {color}.a * {opacity}), target_layer_color(pixel_pos));\n",
            state.blend_mode.shader_func()
        ))
    }
}

#[derive(Default, Clone)]
pub struct BlendWithInputNode;

#[derive(Clone, Serialize, Deserialize)]
pub struct BlendWithInputNodeState {
    pub blend_mode: BlendMode,
}

#[derive(Clone)]
pub enum BlendWithInputNodeMessage {
    ModeChanged(BlendMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for BlendWithInputNode {
    type State = BlendWithInputNodeState;
    type Message = BlendWithInputNodeMessage;

    fn id(&self) -> &'static str {
        "blend_with_input_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        BlendWithInputNodeState {
            blend_mode: BlendMode::Normal,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BlendWithInputNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("color".into()),
            GraphDefaultInputSlot::new::<F32Type>("opacity".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>("color".into())]
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_, Data>,
    ) -> lapiz_shader_graph::GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
                BlendMode::ALL,
                Some(state.blend_mode),
                BlendWithInputNodeMessage::ModeChanged,
            )
            .width(iced_core::Length::Fill),
            BlendWithInputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            BlendWithInputNodeMessage::ModeChanged(mode) => state.blend_mode = mode,
            BlendWithInputNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let opacity = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {output} = image::blend_modes::{}(vec4f({color}.rgb, {color}.a * {opacity}), current_input_color(pixel_pos));\n",
            state.blend_mode.shader_func()
        ))
    }
}

pub static FILTER_GRAPH_TYPES: LazyLock<Arc<GraphTypeRegistry>> =
    LazyLock::new(|| Arc::new(filter_graph_types()));
pub static FILTER_GRAPH_NODES: LazyLock<Arc<GraphNodeRegistry<FilterGraphData>>> =
    LazyLock::new(|| Arc::new(filter_graph_nodes()));

fn filter_graph_types() -> GraphTypeRegistry {
    let mut types = GraphTypeRegistry::default();
    types.merge(builtin_types());
    types
}

fn filter_graph_nodes() -> GraphNodeRegistry<FilterGraphData> {
    let mut nodes = GraphNodeRegistry::with_capacity();
    nodes.merge(builtin_nodes());

    nodes.register::<PixelPositionNode>();
    nodes.register::<InputColorNode>();
    nodes.register::<SampleInputColorNode>();
    nodes.register::<InputBoundsNode>();
    nodes.register::<OutputColorNode>();
    nodes.register::<OutputBoundsNode>();
    nodes.register::<SelectionMaskNode>();
    nodes.register::<LayerPixelColorNode>();
    nodes.register::<BlendWithLayerNode>();
    nodes.register::<BlendWithInputNode>();

    nodes
}
