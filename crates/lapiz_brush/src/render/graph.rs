use bevy_math::IRect;
use encase::ShaderType;
use glam::Vec4;
use iced_core::Length;
use lapiz_i18n::{Translated, t};
use lapiz_image::blend_modes::BlendMode;
use lapiz_shader_graph::{
    GraphElement,
    graph::{
        GraphData,
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeUpdateContext, GraphNodeUpdateSignatureContext,
            GraphNodeViewContext, StatelessCommonGraphNode, stateless,
        },
        slot::{ErasedGraphLiteralUpdateMessage, GraphDefaultInputSlot, GraphDefaultOutputSlot},
    },
    wgsl_std::{
        nodes::{GraphDataWithTime, GraphTimes},
        types::{ColorType, F32Type, I32Type, RectType, TextureType, Vec2FType},
    },
};
use lapiz_utils::random_oklch_hue_chroma;
use lapiz_widgets::combo_box::ComboBox;
use parse_display::Display;
use serde::{Deserialize, Serialize};

use crate::render::{ComputedPenInput, Time};

// TODO We may move to another crate.
#[derive(Debug, Default, Clone, ShaderType)]
pub struct CanvasResources {
    pub foreground_color: Vec4,
    pub background_color: Vec4,
}

pub trait GraphDataWithBrushResource: GraphData {
    fn canvas_resources_field() -> String;
}

#[derive(Default, Clone)]
pub struct BrushStrokePostprocessGraphData {
    pub accumulated_pixel_bounds: IRect,
    pub time: Time,
    pub resources: CanvasResources,
}

impl GraphData for BrushStrokePostprocessGraphData {}

pub struct BrushRequiredSpacingGraphData {
    pub pen_input: ComputedPenInput,
    pub resources: CanvasResources,
}

impl GraphData for BrushRequiredSpacingGraphData {}

#[derive(Debug, Default, Clone, ShaderType)]
pub struct BrushMainGraphData {
    pub pen_input: ComputedPenInput,
    pub initial_pen_input: ComputedPenInput,
    pub resources: CanvasResources,
}

impl GraphData for BrushMainGraphData {}

pub trait GraphDataWithPenInput: GraphData {
    fn pen_input_field() -> String;
}

pub trait GraphDataWithInitialPenInput: GraphData {
    fn initial_pen_input_field() -> String;
}

// TODO This is kinda mess
impl GraphDataWithPenInput for BrushMainGraphData {
    fn pen_input_field() -> String {
        "graph_input".into()
    }
}

impl GraphDataWithPenInput for BrushRequiredSpacingGraphData {
    fn pen_input_field() -> String {
        "graph_input".into()
    }
}

impl GraphDataWithInitialPenInput for BrushMainGraphData {
    fn initial_pen_input_field() -> String {
        "initial_pen_input".into()
    }
}

impl GraphDataWithInitialPenInput for BrushRequiredSpacingGraphData {
    fn initial_pen_input_field() -> String {
        "initial_pen_input".into()
    }
}

impl GraphDataWithTime for BrushMainGraphData {
    fn time(&self) -> GraphTimes {
        GraphTimes {
            now: self.pen_input.time.now,
            stroke_begin: self.pen_input.time.stroke_begin,
        }
    }

    fn wgsl_variable() -> String {
        "graph_input.time".into()
    }
}

impl GraphDataWithTime for BrushRequiredSpacingGraphData {
    fn time(&self) -> GraphTimes {
        GraphTimes {
            now: self.pen_input.time.now,
            stroke_begin: self.pen_input.time.stroke_begin,
        }
    }

    fn wgsl_variable() -> String {
        "graph_input.time".into()
    }
}

impl GraphDataWithTime for BrushStrokePostprocessGraphData {
    fn time(&self) -> GraphTimes {
        GraphTimes {
            now: self.time.now,
            stroke_begin: self.time.stroke_begin,
        }
    }

    fn wgsl_variable() -> String {
        "graph_input.time".into()
    }
}

impl GraphDataWithBrushResource for BrushRequiredSpacingGraphData {
    fn canvas_resources_field() -> String {
        "canvas_resources".into()
    }
}

impl GraphDataWithBrushResource for BrushMainGraphData {
    fn canvas_resources_field() -> String {
        "canvas_resources".into()
    }
}

impl GraphDataWithBrushResource for BrushStrokePostprocessGraphData {
    fn canvas_resources_field() -> String {
        "canvas_resources".into()
    }
}

#[derive(Default, Clone)]
pub struct PenPositionNode;

#[stateless]
impl<Data: GraphDataWithPenInput> StatelessCommonGraphNode<Data> for PenPositionNode {
    fn id(&self) -> &'static str {
        "pen_position_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PenPositionNode)
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
        Ok(format!(
            "let {} = {}.position;",
            ctx.get_output(0)?,
            Data::pen_input_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct DrawDirectionNode;

#[stateless]
impl<Data: GraphDataWithPenInput> StatelessCommonGraphNode<Data> for DrawDirectionNode {
    fn id(&self) -> &'static str {
        "draw_direction_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(DrawDirectionNode)
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
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("angle".into()),
            GraphDefaultOutputSlot::new::<Vec2FType>("direction".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let pen_input = Data::pen_input_field();
        Ok(format!(
            "let {} = {pen_input}.draw_direction_angle;\nlet {} = {pen_input}.draw_direction_vec;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

#[derive(Default, Clone)]
pub struct PenPressureNode;

#[stateless]
impl<Data: GraphDataWithPenInput> StatelessCommonGraphNode<Data> for PenPressureNode {
    fn id(&self) -> &'static str {
        "pen_pressure_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PenPressureNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("pressure".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = {}.pressure;\n",
            ctx.get_output(0)?,
            Data::pen_input_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct PenTiltNode;

#[stateless]
impl<Data: GraphDataWithPenInput> StatelessCommonGraphNode<Data> for PenTiltNode {
    fn id(&self) -> &'static str {
        "pen_tilt_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PenTiltNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("tilt".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = {}.tilt;\n",
            ctx.get_output(0)?,
            Data::pen_input_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct PenAngleNode;

#[stateless]
impl<Data: GraphDataWithPenInput> StatelessCommonGraphNode<Data> for PenAngleNode {
    fn id(&self) -> &'static str {
        "pen_angle_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PenAngleNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("altitude".into()),
            GraphDefaultOutputSlot::new::<F32Type>("azimuth".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let pen_input = Data::pen_input_field();
        Ok(format!(
            "let {} = {pen_input}.angle.x;\nlet {} = {pen_input}.angle.y;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

#[derive(Default, Clone)]
pub struct DabIndexNode;

#[stateless]
impl<Data: GraphDataWithPenInput> StatelessCommonGraphNode<Data> for DabIndexNode {
    fn id(&self) -> &'static str {
        "dab_index_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(DabIndexNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<I32Type>("dab_index".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = i32({}.dab_index);",
            ctx.get_output(0)?,
            Data::pen_input_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct StrokeDistanceNode;

#[stateless]
impl<Data: GraphDataWithPenInput> StatelessCommonGraphNode<Data> for StrokeDistanceNode {
    fn id(&self) -> &'static str {
        "stroke_distance_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(StrokeDistanceNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>(
            "stroke_distance".into(),
        )]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = {}.stroke_distance;",
            ctx.get_output(0)?,
            Data::pen_input_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct InitialPenPositionNode;

#[stateless]
impl<Data: GraphDataWithInitialPenInput> StatelessCommonGraphNode<Data> for InitialPenPositionNode {
    fn id(&self) -> &'static str {
        "initial_pen_position_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialPenPositionNode)
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
        Ok(format!(
            "let {} = {}.position;",
            ctx.get_output(0)?,
            Data::initial_pen_input_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct InitialDrawDirectionNode;

#[stateless]
impl<Data: GraphDataWithInitialPenInput> StatelessCommonGraphNode<Data>
    for InitialDrawDirectionNode
{
    fn id(&self) -> &'static str {
        "initial_draw_direction_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialDrawDirectionNode)
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
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("angle".into()),
            GraphDefaultOutputSlot::new::<Vec2FType>("direction".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let pen_input = Data::initial_pen_input_field();
        Ok(format!(
            "let {} = {pen_input}.draw_direction_angle;\nlet {} = {pen_input}.draw_direction_vec;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

#[derive(Default, Clone)]
pub struct InitialPenPressureNode;

#[stateless]
impl<Data: GraphDataWithInitialPenInput> StatelessCommonGraphNode<Data> for InitialPenPressureNode {
    fn id(&self) -> &'static str {
        "initial_pen_pressure_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialPenPressureNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("pressure".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = {}.pressure;\n",
            ctx.get_output(0)?,
            Data::initial_pen_input_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct InitialPenTiltNode;

#[stateless]
impl<Data: GraphDataWithInitialPenInput> StatelessCommonGraphNode<Data> for InitialPenTiltNode {
    fn id(&self) -> &'static str {
        "initial_pen_tilt_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialPenTiltNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<Vec2FType>("tilt".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = {}.tilt;\n",
            ctx.get_output(0)?,
            Data::initial_pen_input_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct InitialPenAngleNode;

#[stateless]
impl<Data: GraphDataWithInitialPenInput> StatelessCommonGraphNode<Data> for InitialPenAngleNode {
    fn id(&self) -> &'static str {
        "initial_pen_angle_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(InitialPenAngleNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("altitude".into()),
            GraphDefaultOutputSlot::new::<F32Type>("azimuth".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let pen_input = Data::initial_pen_input_field();
        Ok(format!(
            "let {} = {pen_input}.angle.x;\nlet {} = {pen_input}.angle.y;\n",
            ctx.get_output(0)?,
            ctx.get_output(1)?
        ))
    }
}

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
pub struct FilterWithinMaskNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for FilterWithinMaskNode {
    fn id(&self) -> &'static str {
        "filter_within_mask_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(FilterWithinMaskNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("color".into()),
            GraphDefaultInputSlot::new::<TextureType>("mask".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("translation".into()),
            GraphDefaultInputSlot::new::<F32Type>("rotation".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("scale".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("anchor".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<ColorType>("color".into()),
            GraphDefaultOutputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let mask = ctx.get_input(1)?;
        let translation = ctx.get_input(2)?;
        let rotation = ctx.get_input(3)?;
        let scale = ctx.get_input(4)?;
        let anchor = ctx.get_input(5)?;

        Ok(format!(
            "let {} = filter_within_mask(pixel_pos, {}, {}, {}, {}, {}, {});\nlet {} = filter_within_mask_bounds({}, {}, {}, {}, {});\n",
            ctx.get_output(0)?,
            color,
            mask,
            scale,
            rotation,
            translation,
            anchor,
            ctx.get_output(1)?,
            mask,
            scale,
            rotation,
            translation,
            anchor,
        ))
    }
}

#[derive(Default, Clone)]
pub struct FilterWithinBoundsNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for FilterWithinBoundsNode {
    fn id(&self) -> &'static str {
        "filter_within_bounds_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(FilterWithinBoundsNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("color".into()),
            GraphDefaultInputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<ColorType>("color".into()),
            GraphDefaultOutputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let bounds = ctx.get_input(1)?;

        Ok(format!(
            "let {} = filter_within_bounds(pixel_pos, {}, {});\nlet {} = {};\n",
            ctx.get_output(0)?,
            color,
            bounds,
            ctx.get_output(1)?,
            bounds
        ))
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
        Ok(format!("set_output_pixel_bounds({});", ctx.get_input(0)?))
    }
}

#[derive(Default, Clone)]
pub struct PasteTextureNode;

#[derive(Clone, PartialEq)]
struct BlendModeOption(BlendMode);

impl BlendModeOption {
    fn into_inner(self) -> BlendMode {
        self.0
    }
}

impl std::fmt::Display for BlendModeOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&t!(&format!("blend_{}", self.0)))
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Display)]
#[display(style = "snake_case")]
pub enum PasteTextureMode {
    Clamp,
    #[default]
    Wrap,
}

impl PasteTextureMode {
    const ALL: [PasteTextureMode; 2] = [PasteTextureMode::Clamp, PasteTextureMode::Wrap];
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct PasteTextureNodeState {
    pub mode: PasteTextureMode,
}

#[derive(Clone)]
pub enum PasteTextureNodeMessage {
    ModeChanged(PasteTextureMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for PasteTextureNode {
    type State = PasteTextureNodeState;
    type Message = PasteTextureNodeMessage;

    fn id(&self) -> &'static str {
        "paste_texture_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
        Default::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(PasteTextureNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<TextureType>("texture".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("translation".into()),
            GraphDefaultInputSlot::new::<F32Type>("rotation".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("scale".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("anchor".into()),
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
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
                PasteTextureMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(Translated)
                    .collect::<Vec<_>>(),
                Some(Translated(state.mode)),
                |option| PasteTextureNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(Length::Fill),
            PasteTextureNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            PasteTextureNodeMessage::ModeChanged(mode) => state.mode = mode,
            PasteTextureNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let tex = ctx.get_input(0)?;
        let translation = ctx.get_input(1)?;
        let rotation = ctx.get_input(2)?;
        let scale = ctx.get_input(3)?;
        let anchor = ctx.get_input(4)?;
        let output = ctx.get_output(0)?;
        let fn_name = match state.mode {
            PasteTextureMode::Clamp => "sample_transformed_local_texture_clamp",
            PasteTextureMode::Wrap => "sample_transformed_local_texture_wrap",
        };
        Ok(format!(
            "let {} = {}({}, pixel_posf, {}, {}, {}, {});\n",
            output, fn_name, tex, scale, rotation, translation, anchor
        ))
    }
}

#[derive(Default, Clone)]
pub struct CurrentPixelColorNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for CurrentPixelColorNode {
    fn id(&self) -> &'static str {
        "current_pixel_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CurrentPixelColorNode)
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
        Ok(format!(
            "let {} = current_input_color(vec2i({}));\n",
            ctx.get_output(0)?,
            ctx.get_input(0)?
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
        Ok(format!(
            "let {} = target_layer_color(vec2i({}));\n",
            ctx.get_output(0)?,
            ctx.get_input(0)?
        ))
    }
}

#[derive(Default, Clone)]
pub struct BlendColorNode;

#[derive(Clone, Serialize, Deserialize)]
pub struct BlendColorNodeState {
    pub blend_mode: BlendMode,
}

#[derive(Clone)]
pub enum BlendModeNodeMessage {
    ModeChanged(BlendMode),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl<Data: GraphData> GraphNode<Data> for BlendColorNode {
    type State = BlendColorNodeState;
    type Message = BlendModeNodeMessage;

    fn id(&self) -> &'static str {
        "blend_color_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
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
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<ColorType>("src_color".into()),
            GraphDefaultInputSlot::new::<ColorType>("dst_color".into()),
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
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
                BlendMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(BlendModeOption)
                    .collect::<Vec<_>>(),
                Some(BlendModeOption(state.blend_mode)),
                |option| BlendModeNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(Length::Fill),
            BlendModeNodeMessage::LiteralUpdate,
        )
    }
    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            BlendModeNodeMessage::ModeChanged(mode) => state.blend_mode = mode,
            BlendModeNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let src = ctx.get_input(0)?;
        let dst = ctx.get_input(1)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {} = image::blend_modes::{}({}, {});\n",
            output,
            state.blend_mode.shader_func(),
            src,
            dst
        ))
    }
}

#[derive(Default, Clone)]
pub struct StrokeBoundsNode;

#[stateless]
impl StatelessCommonGraphNode<BrushStrokePostprocessGraphData> for StrokeBoundsNode {
    fn id(&self) -> &'static str {
        "stroke_bounds_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(StrokeBoundsNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, BrushStrokePostprocessGraphData>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, BrushStrokePostprocessGraphData>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<RectType>("bounds".into())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, BrushStrokePostprocessGraphData>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = Rect(vec2f(graph_input.accumulated_pixel_bound.min), vec2f(graph_input.accumulated_pixel_bound.max));",
            ctx.get_output(0)?
        ))
    }
}

#[derive(Default, Clone)]
pub struct EllipticalMaskNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for EllipticalMaskNode {
    fn id(&self) -> &'static str {
        "elliptical_mask_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(EllipticalMaskNode)
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![
            GraphDefaultInputSlot::new::<Vec2FType>("sample_position".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("center".into()),
            GraphDefaultInputSlot::new::<Vec2FType>("radii".into()),
            GraphDefaultInputSlot::new::<F32Type>("rotation".into()),
        ]
    }

    fn create_outputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![
            GraphDefaultOutputSlot::new::<F32Type>("mask_value".into()),
            GraphDefaultOutputSlot::new::<RectType>("bounds".into()),
        ]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
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

#[derive(Default, Clone)]
pub struct BlendWithInputNode;

#[derive(Clone, Serialize, Deserialize)]
pub struct BlendWithBufferNodeState {
    pub blend_mode: BlendMode,
}

impl<Data: GraphData> GraphNode<Data> for BlendWithInputNode {
    type State = BlendWithBufferNodeState;
    type Message = BlendModeNodeMessage;

    fn id(&self) -> &'static str {
        "blend_with_input_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_, Data>) -> Self::State {
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
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
                BlendMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(BlendModeOption)
                    .collect::<Vec<_>>(),
                Some(BlendModeOption(state.blend_mode)),
                |option| BlendModeNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(Length::Fill),
            BlendModeNodeMessage::LiteralUpdate,
        )
    }
    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            BlendModeNodeMessage::ModeChanged(mode) => state.blend_mode = mode,
            BlendModeNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let opacity = ctx.get_input(1)?;
        Ok(format!(
            "let {} = image::blend_modes::{}(vec4f({color}.rgb, {color}.a * {opacity}), current_input_color(pixel_pos));\n",
            ctx.get_output(0)?,
            state.blend_mode.shader_func()
        ))
    }
}

#[derive(Default, Clone)]
pub struct BlendWithLayerNode;

#[derive(Clone, Serialize, Deserialize)]
pub struct BlendWithLayerNodeState {
    pub blend_mode: BlendMode,
}

impl<Data: GraphData> GraphNode<Data> for BlendWithLayerNode {
    type State = BlendWithLayerNodeState;
    type Message = BlendModeNodeMessage;

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
    ) -> GraphElement<'static, Self::Message> {
        ctx.view_all_slots_with_header(
            ComboBox::new(
                BlendMode::ALL
                    .to_vec()
                    .into_iter()
                    .map(BlendModeOption)
                    .collect::<Vec<_>>(),
                Some(BlendModeOption(state.blend_mode)),
                |option| BlendModeNodeMessage::ModeChanged(option.into_inner()),
            )
            .width(Length::Fill),
            BlendModeNodeMessage::LiteralUpdate,
        )
    }
    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_, Data>,
    ) {
        match message {
            BlendModeNodeMessage::ModeChanged(mode) => state.blend_mode = mode,
            BlendModeNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let color = ctx.get_input(0)?;
        let opacity = ctx.get_input(1)?;
        Ok(format!(
            "let {} = image::blend_modes::{}(vec4f({color}.rgb, {color}.a * {opacity}), target_layer_color(pixel_pos));\n",
            ctx.get_output(0)?,
            state.blend_mode.shader_func()
        ))
    }
}

#[derive(Default, Clone)]
pub struct OutputSpacingNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for OutputSpacingNode {
    fn id(&self) -> &'static str {
        "output_spacing_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(OutputSpacingNode)
    }

    fn update_signature(&self, mut ctx: GraphNodeUpdateSignatureContext<'_, Data>) {
        ctx.require_input_slot_as_graph_output(0, "Spacing".to_string());
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<F32Type>("spacing".into())]
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
        Ok(format!("return {};\n", ctx.get_input(0)?))
    }
}

#[derive(Default, Clone)]
pub struct OutputRequiredSpacingNode;

#[stateless]
impl<Data: GraphData> StatelessCommonGraphNode<Data> for OutputRequiredSpacingNode {
    fn id(&self) -> &'static str {
        "output_required_spacing_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(OutputRequiredSpacingNode)
    }

    fn update_signature(&self, mut ctx: GraphNodeUpdateSignatureContext<'_, Data>) {
        ctx.require_input_slot_as_graph_output(0, "Required Spacing".to_string());
    }

    fn create_inputs(
        &self,
        _: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<F32Type>(
            "required_spacing".into(),
        )]
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
        Ok(format!("return {};\n", ctx.get_input(0)?))
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
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![GraphDefaultInputSlot::new::<Vec2FType>("position".into())]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<F32Type>("value".to_string())]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let input = ctx.get_input(0)?;
        let output = ctx.get_output(0)?;
        Ok(format!(
            "let {} = load_selection_mask_value(vec2i({}));\n",
            output, input
        ))
    }
}

#[derive(Default, Clone)]
pub struct ForegroundColorNode;

#[stateless]
impl<Data: GraphDataWithBrushResource> StatelessCommonGraphNode<Data> for ForegroundColorNode {
    fn id(&self) -> &'static str {
        "foreground_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(ForegroundColorNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>(
            "color".to_string(),
        )]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = {}.foreground_color;\n",
            ctx.get_output(0)?,
            Data::canvas_resources_field()
        ))
    }
}

#[derive(Default, Clone)]
pub struct BackgroundColorNode;

#[stateless]
impl<Data: GraphDataWithBrushResource> StatelessCommonGraphNode<Data> for BackgroundColorNode {
    fn id(&self) -> &'static str {
        "background_color_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(BackgroundColorNode)
    }

    fn create_inputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultInputSlot> {
        vec![]
    }

    fn create_outputs(
        &self,
        _ctx: GraphNodeCreateSlotsContext<'_, Data>,
    ) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<ColorType>(
            "color".to_string(),
        )]
    }

    fn generate_code(
        &self,
        mut ctx: GraphNodeCodeGenContext<'_, Data>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(format!(
            "let {} = {}.background_color;\n",
            ctx.get_output(0)?,
            Data::canvas_resources_field()
        ))
    }
}
