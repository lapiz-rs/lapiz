use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use iced_core::Point;
use lapiz_assets::{asset::AssetId, store::AssetRegistry};
use lapiz_brush::{
    instance::{main_effect_resources, postprocess_effect_resources, spacing_effect_resources},
    render::graph::{
        BackgroundColorNode, CurrentPixelColorNode, DabIndexNode, DrawDirectionNode,
        ForegroundColorNode, InitialDrawDirectionNode, LayerPixelColorNode, MAIN_DAB_BUFFER,
        PenAngleNode, PenPositionNode, PenPressureNode, PenTiltNode, PixelPositionNode,
        SPACING_OUTPUT, STROKE_RESULT, StrokeBoundsNode, StrokeDistanceNode, TimeNode,
    },
};
use lapiz_effect::{
    asset::{
        EffectAsset, EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy,
        EffectPassId, EffectPassOutputSlotId, SerializableEffectInputSlot,
        SerializableEffectOutputSlot, SerializableEffectPass,
    },
    nodes::{PassInput, PassInputNode, PassOutput, PassOutputNode},
};
use lapiz_image::{blend_modes::BlendMode, texel::TexelType};
use lapiz_render::texture::Image;
use lapiz_shader_graph::{
    graph::{
        Graph,
        node::{GraphNode, GraphNodeId},
        slot::{ErasedGraphValueType, GraphValueType},
    },
    save::{SerializableGraph, SerializableGraphLiteral},
    wgsl_std::{
        nodes::{CustomExpressionNode, CustomExpressionNodeState},
        types::{
            compound::{ColorType, RectType},
            handle::{LayerType, TextureType},
            primitive::{F32Type, I32Type},
            vector::Vec2FType,
        },
    },
};
use toml::map::Map;
use uuid::Uuid;

use crate::desc::wgsl::{
    AZIMUTH_INPUT, BrushPose, BrushTexture, ColorAdjustment, DAB_INDEX_INPUT, DIRECTION_INPUT,
    DUAL_TIP_TEXTURE_INPUT, DualBrush, Dynamics, INITIAL_DIRECTION_INPUT,
    MAIN_BACKGROUND_COLOR_INPUT, MAIN_BOUNDS_OUTPUT, MAIN_COLOR_OUTPUT, MAIN_CURRENT_COLOR_INPUT,
    MAIN_FOREGROUND_COLOR_INPUT, MAIN_PATTERN_TEXTURE_INPUT, MAIN_PEN_POSITION_INPUT,
    MAIN_PIXEL_POSITION_INPUT, MAIN_TIP_TEXTURE_INPUT, POSTPROCESS_INPUT_COLOR,
    POSTPROCESS_STROKE_BOUNDS_INPUT, POSTPROCESS_TARGET_COLOR_INPUT, PRESSURE_INPUT,
    REQUIRED_SPACING_OUTPUT, STROKE_BEGIN_INPUT, STROKE_DISTANCE_INPUT, Scatter, TILT_INPUT,
    USER_FLOW, USER_OPACITY, USER_SIZE, computed_main, computed_required_spacing,
    opacity_postprocess, sampled_main, sampled_required_spacing,
};

pub fn add_stateful_node<T>(
    graph: &mut Graph,
    position: Point,
    node: T,
    state: T::State,
) -> GraphNodeId
where
    T: GraphNode,
{
    let node_id = graph.add_node(position, node);
    graph.update_node_state::<T>(node_id, |current| *current = state);
    node_id
}

fn add_mask_input_slot(state: &mut CustomExpressionNodeState, name: &str) {
    state.add_input_non_default(
        name,
        TextureType {
            texel_type: TexelType::A8,
        },
    );
}

fn add_dynamics_input_slots(state: &mut CustomExpressionNodeState) {
    state.add_input::<F32Type>(PRESSURE_INPUT);
    state.add_input::<Vec2FType>(TILT_INPUT);
    state.add_input::<F32Type>(AZIMUTH_INPUT);
    state.add_input::<F32Type>(DIRECTION_INPUT);
    state.add_input::<F32Type>(INITIAL_DIRECTION_INPUT);
    state.add_input::<I32Type>(DAB_INDEX_INPUT);
}

fn connect_dynamics_input_nodes(graph: &mut Graph, expression: GraphNodeId, input_offset: usize) {
    let pressure = graph.add_node(Point::new(0.0, 350.0), PenPressureNode);
    let tilt = graph.add_node(Point::new(0.0, 400.0), PenTiltNode);
    let angle = graph.add_node(Point::new(0.0, 450.0), PenAngleNode);
    let direction = graph.add_node(Point::new(0.0, 500.0), DrawDirectionNode);
    let initial_direction = graph.add_node(Point::new(0.0, 550.0), InitialDrawDirectionNode);
    let dab_index = graph.add_node(Point::new(0.0, 600.0), DabIndexNode);

    graph.connect_slots_by_index(pressure, 0, expression, input_offset);
    graph.connect_slots_by_index(tilt, 0, expression, input_offset + 1);
    graph.connect_slots_by_index(angle, 1, expression, input_offset + 2);
    graph.connect_slots_by_index(direction, 0, expression, input_offset + 3);
    graph.connect_slots_by_index(initial_direction, 0, expression, input_offset + 4);
    graph.connect_slots_by_index(dab_index, 0, expression, input_offset + 5);
}

#[derive(Clone, Copy)]
pub struct MainGraphOptions {
    pub flow: f32,
    pub size_dynamics: Option<Dynamics>,
    pub opacity_dynamics: Option<Dynamics>,
    pub flow_dynamics: Option<Dynamics>,
    pub angle_dynamics: Option<Dynamics>,
    pub roundness_dynamics: Option<Dynamics>,
    pub tilt_scale: f32,
    pub flip_x_jitter: bool,
    pub flip_y_jitter: bool,
    pub pose: BrushPose,
    pub color_adjustment: Option<ColorAdjustment>,
    pub scatter: Option<Scatter>,
    pub brush_texture: Option<BrushTexture>,
    pub pattern_asset: Option<AssetId<Image>>,
    pub noise: bool,
    pub dual_brush: Option<DualBrush>,
    pub dual_sample_asset: Option<AssetId<Image>>,
}

#[derive(Clone, Copy)]
pub struct ComputedMainTip {
    pub diameter: f32,
    pub hardness: f32,
    pub angle: f32,
    pub roundness: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub spacing: f32,
}

#[derive(Clone, Copy)]
pub struct SampledMainTip {
    pub sample_asset: AssetId<Image>,
    pub diameter: f32,
    pub angle: f32,
    pub roundness: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub spacing: f32,
}

#[derive(Clone, Copy)]
pub enum MainTip {
    Computed(ComputedMainTip),
    Sampled(SampledMainTip),
}

pub struct BrushInputSlot {
    pub id: EffectInputSlotId,
    pub name: String,
    pub ty: Arc<dyn ErasedGraphValueType>,
    pub value: SerializableGraphLiteral,
}

pub struct BrushInputs {
    pub size: EffectInputSlotId,
    pub opacity: EffectInputSlotId,
    pub flow: EffectInputSlotId,
    pub tip_texture: Option<EffectInputSlotId>,
    pub pattern_texture: Option<EffectInputSlotId>,
    pub dual_tip_texture: Option<EffectInputSlotId>,
    pub slots: Vec<BrushInputSlot>,
}

impl BrushInputs {
    pub fn new(
        size: f32,
        tip_texture: Option<AssetId<Image>>,
        pattern_texture: Option<AssetId<Image>>,
        dual_tip_texture: Option<AssetId<Image>>,
    ) -> Result<Self> {
        let size_id = EffectInputSlotId::new(Uuid::new_v4());
        let opacity_id = EffectInputSlotId::new(Uuid::new_v4());
        let flow_id = EffectInputSlotId::new(Uuid::new_v4());
        let mut inputs = vec![
            f32_input(size_id, "Size", size.clamp(0.1, 1000.0))?,
            f32_input(opacity_id, "Opacity", 1.0)?,
            f32_input(flow_id, "Flow", 1.0)?,
        ];
        let tip_texture = tip_texture
            .map(|asset| {
                let id = EffectInputSlotId::new(Uuid::new_v4());
                inputs.push(mask_input(id, "Tip Texture", asset)?);
                Ok::<_, anyhow::Error>(id)
            })
            .transpose()?;
        let pattern_texture = pattern_texture
            .map(|asset| {
                let id = EffectInputSlotId::new(Uuid::new_v4());
                inputs.push(mask_input(id, "Pattern Texture", asset)?);
                Ok::<_, anyhow::Error>(id)
            })
            .transpose()?;
        let dual_tip_texture = dual_tip_texture
            .map(|asset| {
                let id = EffectInputSlotId::new(Uuid::new_v4());
                inputs.push(mask_input(id, "Dual Tip Texture", asset)?);
                Ok::<_, anyhow::Error>(id)
            })
            .transpose()?;

        Ok(Self {
            size: size_id,
            opacity: opacity_id,
            flow: flow_id,
            tip_texture,
            pattern_texture,
            dual_tip_texture,
            slots: inputs,
        })
    }

    fn add_input_nodes_into(&self, graph: &mut Graph) -> HashMap<EffectInputSlotId, GraphNodeId> {
        self.slots
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let node = graph.add_node(Point::new(-200.0, index as f32 * 50.0), PassInputNode);
                graph.update_node_state::<PassInputNode>(node, |state| {
                    state.input = Some(PassInput::Effect(input.id));
                    state.cached_ty = Some(input.ty.clone());
                });
                (input.id, node)
            })
            .collect()
    }
}

fn f32_input(id: EffectInputSlotId, name: &str, value: f32) -> Result<BrushInputSlot> {
    let ty = Arc::new(F32Type);
    Ok(BrushInputSlot {
        id,
        name: name.into(),
        value: SerializableGraphLiteral {
            ty: GraphValueType::id(ty.as_ref()).id,
            value: toml::Value::try_from(value)?,
        },
        ty,
    })
}

fn mask_input(id: EffectInputSlotId, name: &str, asset: AssetId<Image>) -> Result<BrushInputSlot> {
    let ty = Arc::new(TextureType {
        texel_type: TexelType::A8,
    });
    let mut value = Map::new();
    value.insert("asset".into(), toml::Value::try_from(asset)?);
    Ok(BrushInputSlot {
        id,
        name: name.into(),
        value: SerializableGraphLiteral {
            ty: GraphValueType::id(ty.as_ref()).id,
            value: toml::Value::Table(value),
        },
        ty,
    })
}

fn add_effect_output_node(
    graph: &mut Graph,
    position: Point,
    output: EffectOutputSlotId,
    ty: Arc<dyn ErasedGraphValueType>,
) -> (GraphNodeId, EffectPassOutputSlotId) {
    let node = graph.add_node(position, PassOutputNode);
    let port = graph
        .get_node(&node)
        .expect("newly added output node exists")
        .data
        .state::<PassOutputNode>()
        .expect("output node has output state")
        .id;
    graph.update_node_state::<PassOutputNode>(node, |state| {
        state.output = Some(PassOutput::Effect(output));
        state.cached_ty = Some(ty);
    });
    (node, port)
}

fn effect_asset(
    name: &str,
    pass_name: &str,
    graph: SerializableGraph,
    dispatch_strategy: EffectPassDispatchStrategy,
    inputs: &BrushInputs,
    outputs: Vec<SerializableEffectOutputSlot>,
) -> EffectAsset {
    EffectAsset {
        name: name.into(),
        passes: vec![SerializableEffectPass {
            id: EffectPassId::new(Uuid::new_v4()),
            name: pass_name.into(),
            graph,
            dispatch_strategy,
        }],
        inputs: {
            let this = &inputs;
            this.slots
                .iter()
                .map(|input| SerializableEffectInputSlot {
                    name: input.name.clone(),
                    id: input.id,
                    ty: input.ty.id().id,
                })
                .collect()
        },
        outputs,
    }
}

pub fn computed_graphs(
    tip: ComputedMainTip,
    options: MainGraphOptions,
    inputs: &BrushInputs,
) -> Result<(EffectAsset, EffectAsset)> {
    let spacing_effect = required_spacing_effect(
        inputs,
        tip.spacing,
        options.size_dynamics,
        options.pose,
        false,
    )?;
    let main_effect = build_main_effect(MainTip::Computed(tip), options, inputs)?;
    Ok((spacing_effect, main_effect))
}

pub fn sampled_graphs(
    tip: SampledMainTip,
    options: MainGraphOptions,
    inputs: &BrushInputs,
) -> Result<(EffectAsset, EffectAsset)> {
    let spacing_effect = required_spacing_effect(
        inputs,
        tip.spacing,
        options.size_dynamics,
        options.pose,
        true,
    )?;
    let main_effect = build_main_effect(MainTip::Sampled(tip), options, inputs)?;
    Ok((spacing_effect, main_effect))
}

fn required_spacing_effect(
    inputs: &BrushInputs,
    spacing: f32,
    size_dynamics: Option<Dynamics>,
    pose: BrushPose,
    sampled: bool,
) -> Result<EffectAsset> {
    let mut graph = Graph::new(spacing_effect_resources(AssetRegistry::new_in_memory(
        Default::default(),
    )));
    let input_nodes = inputs.add_input_nodes_into(&mut graph);
    let mut state = CustomExpressionNodeState::default();
    let input_offset = if sampled {
        add_mask_input_slot(&mut state, MAIN_TIP_TEXTURE_INPUT);
        1
    } else {
        0
    };
    state.add_input::<F32Type>(PRESSURE_INPUT);
    state.add_input::<Vec2FType>(TILT_INPUT);
    state.add_input::<F32Type>(AZIMUTH_INPUT);
    state.add_input::<F32Type>(DIRECTION_INPUT);
    state.add_input::<F32Type>(INITIAL_DIRECTION_INPUT);
    state.add_input::<I32Type>(DAB_INDEX_INPUT);
    state.add_input::<F32Type>(USER_SIZE);
    state.add_output::<F32Type>(REQUIRED_SPACING_OUTPUT);
    state.set_code(if sampled {
        sampled_required_spacing(spacing, size_dynamics, pose)
    } else {
        computed_required_spacing(spacing, size_dynamics, pose)
    });

    let expression = add_stateful_node(
        &mut graph,
        Point::new(100.0, 100.0),
        CustomExpressionNode,
        state,
    );
    if sampled {
        graph.connect_slots_by_index(input_nodes[&inputs.tip_texture.unwrap()], 0, expression, 0);
    }
    connect_dynamics_input_nodes(&mut graph, expression, input_offset);
    graph.connect_slots_by_index(input_nodes[&inputs.size], 0, expression, input_offset + 6);

    let output_id = EffectOutputSlotId::new(Uuid::new_v4());
    let (output, _) = add_effect_output_node(
        &mut graph,
        Point::new(300.0, 100.0),
        output_id,
        Arc::new(F32Type),
    );
    graph.connect_slots_by_index(expression, 0, output, 0);

    Ok(effect_asset(
        "Brush Spacing",
        "Spacing",
        graph.as_serialized()?,
        EffectPassDispatchStrategy::Once,
        inputs,
        vec![SerializableEffectOutputSlot {
            name: SPACING_OUTPUT.into(),
            id: output_id,
            ty: GraphValueType::id(&F32Type).id,
        }],
    ))
}

fn build_main_effect(
    tip: MainTip,
    options: MainGraphOptions,
    inputs: &BrushInputs,
) -> Result<EffectAsset> {
    let mut graph = Graph::new(main_effect_resources(AssetRegistry::new_in_memory(
        Default::default(),
    )));
    let input_nodes = inputs.add_input_nodes_into(&mut graph);
    let pixel_position = graph.add_node(Point::new(0.0, 0.0), PixelPositionNode);
    let pen_position = graph.add_node(Point::new(0.0, 100.0), PenPositionNode);
    let foreground_color = graph.add_node(Point::new(0.0, 200.0), ForegroundColorNode);
    let tip_texture = inputs.tip_texture.map(|id| input_nodes[&id]);
    let background_color = graph.add_node(Point::new(0.0, 350.0), BackgroundColorNode);
    let current_color = graph.add_node(Point::new(0.0, 400.0), CurrentPixelColorNode);
    let pattern_texture = inputs.pattern_texture.map(|id| input_nodes[&id]);
    let stroke_distance = options
        .dual_brush
        .map(|_| graph.add_node(Point::new(0.0, 600.0), StrokeDistanceNode));
    let stroke_time = graph.add_node(Point::new(0.0, 650.0), TimeNode);
    let dual_texture = inputs.dual_tip_texture.map(|id| input_nodes[&id]);

    let mut state = CustomExpressionNodeState::default();
    state.add_input::<Vec2FType>(MAIN_PIXEL_POSITION_INPUT);
    state.add_input::<Vec2FType>(MAIN_PEN_POSITION_INPUT);
    state.add_input::<ColorType>(MAIN_FOREGROUND_COLOR_INPUT);
    if tip_texture.is_some() {
        add_mask_input_slot(&mut state, MAIN_TIP_TEXTURE_INPUT);
    }
    state.add_input::<ColorType>(MAIN_BACKGROUND_COLOR_INPUT);
    state.add_input::<ColorType>(MAIN_CURRENT_COLOR_INPUT);
    if pattern_texture.is_some() {
        add_mask_input_slot(&mut state, MAIN_PATTERN_TEXTURE_INPUT);
    }
    add_dynamics_input_slots(&mut state);
    if stroke_distance.is_some() {
        state.add_input::<F32Type>(STROKE_DISTANCE_INPUT);
    }
    state.add_input::<F32Type>(STROKE_BEGIN_INPUT);
    if dual_texture.is_some() {
        add_mask_input_slot(&mut state, DUAL_TIP_TEXTURE_INPUT);
    }
    state.add_input::<F32Type>(USER_SIZE);
    state.add_input::<F32Type>(USER_FLOW);
    state.add_output::<ColorType>(MAIN_COLOR_OUTPUT);
    state.add_output::<RectType>(MAIN_BOUNDS_OUTPUT);
    state.set_code(match tip {
        MainTip::Computed(tip) => computed_main(tip, options),
        MainTip::Sampled(tip) => sampled_main(tip, options),
    });

    let expression = add_stateful_node(
        &mut graph,
        Point::new(300.0, 150.0),
        CustomExpressionNode,
        state,
    );
    let output_id = EffectOutputSlotId::new(Uuid::new_v4());
    let layer_ty = Arc::new(LayerType {
        texel_type: TexelType::RGBA8,
    });
    let (output, output_port) = add_effect_output_node(
        &mut graph,
        Point::new(550.0, 100.0),
        output_id,
        layer_ty.clone(),
    );

    let tip_input_offset = usize::from(tip_texture.is_some());
    let background_input = 3 + tip_input_offset;
    let current_color_input = background_input + 1;
    let pattern_input = current_color_input + 1;
    let dynamics_input = pattern_input + usize::from(pattern_texture.is_some());
    let stroke_distance_input = dynamics_input + 6;
    let stroke_time_input = stroke_distance_input + usize::from(stroke_distance.is_some());
    let user_size_input = stroke_time_input + 1 + usize::from(dual_texture.is_some());

    graph.connect_slots_by_index(pixel_position, 0, expression, 0);
    graph.connect_slots_by_index(pen_position, 0, expression, 1);
    graph.connect_slots_by_index(foreground_color, 0, expression, 2);
    if let Some(tip_texture) = tip_texture {
        graph.connect_slots_by_index(tip_texture, 0, expression, 3);
    }
    graph.connect_slots_by_index(background_color, 0, expression, background_input);
    graph.connect_slots_by_index(pixel_position, 0, current_color, 0);
    graph.connect_slots_by_index(current_color, 0, expression, current_color_input);
    if let Some(pattern_texture) = pattern_texture {
        graph.connect_slots_by_index(pattern_texture, 0, expression, pattern_input);
    }
    connect_dynamics_input_nodes(&mut graph, expression, dynamics_input);
    if let Some(stroke_distance) = stroke_distance {
        graph.connect_slots_by_index(stroke_distance, 0, expression, stroke_distance_input);
    }
    graph.connect_slots_by_index(stroke_time, 1, expression, stroke_time_input);
    if let Some(dual_texture) = dual_texture {
        graph.connect_slots_by_index(dual_texture, 0, expression, stroke_time_input + 1);
    }
    graph.connect_slots_by_index(input_nodes[&inputs.size], 0, expression, user_size_input);
    graph.connect_slots_by_index(
        input_nodes[&inputs.flow],
        0,
        expression,
        user_size_input + 1,
    );

    graph.connect_slots_by_index(expression, 0, output, 0);
    graph.connect_slots_by_index(expression, 1, output, 1);

    Ok(effect_asset(
        "Brush Main",
        "Dab",
        graph.as_serialized()?,
        EffectPassDispatchStrategy::EveryOutputLayerPixel(output_port),
        inputs,
        vec![SerializableEffectOutputSlot {
            name: MAIN_DAB_BUFFER.into(),
            id: output_id,
            ty: GraphValueType::id(layer_ty.as_ref()).id,
        }],
    ))
}

pub fn opacity_postprocess_effect(
    opacity: f32,
    blend_mode: BlendMode,
    inputs: &BrushInputs,
) -> Result<EffectAsset> {
    let mut graph = Graph::new(postprocess_effect_resources(AssetRegistry::new_in_memory(
        Default::default(),
    )));
    let input_nodes = inputs.add_input_nodes_into(&mut graph);
    let pixel_position = graph.add_node(Point::new(0.0, 0.0), PixelPositionNode);
    let current_color = graph.add_node(Point::new(200.0, 0.0), CurrentPixelColorNode);
    let target_color = graph.add_node(Point::new(200.0, 75.0), LayerPixelColorNode);
    let stroke_bounds = graph.add_node(Point::new(200.0, 150.0), StrokeBoundsNode);

    let mut state = CustomExpressionNodeState::default();
    state.add_input::<ColorType>(POSTPROCESS_INPUT_COLOR);
    state.add_input::<RectType>(POSTPROCESS_STROKE_BOUNDS_INPUT);
    state.add_input::<ColorType>(POSTPROCESS_TARGET_COLOR_INPUT);
    state.add_input::<F32Type>(USER_OPACITY);
    state.add_output::<ColorType>(MAIN_COLOR_OUTPUT);
    state.add_output::<RectType>(MAIN_BOUNDS_OUTPUT);
    state.set_code(opacity_postprocess(opacity, blend_mode));

    let expression = add_stateful_node(
        &mut graph,
        Point::new(400.0, 75.0),
        CustomExpressionNode,
        state,
    );
    graph.connect_slots_by_index(pixel_position, 0, current_color, 0);
    graph.connect_slots_by_index(pixel_position, 0, target_color, 0);
    graph.connect_slots_by_index(current_color, 0, expression, 0);
    graph.connect_slots_by_index(stroke_bounds, 0, expression, 1);
    graph.connect_slots_by_index(target_color, 0, expression, 2);
    graph.connect_slots_by_index(input_nodes[&inputs.opacity], 0, expression, 3);

    let output_id = EffectOutputSlotId::new(Uuid::new_v4());
    let layer_ty = Arc::new(LayerType {
        texel_type: TexelType::RGBA8,
    });
    let (output, output_port) = add_effect_output_node(
        &mut graph,
        Point::new(650.0, 75.0),
        output_id,
        layer_ty.clone(),
    );
    graph.connect_slots_by_index(expression, 0, output, 0);
    graph.connect_slots_by_index(expression, 1, output, 1);

    Ok(effect_asset(
        "Brush Postprocess",
        "Opacity and Blend",
        graph.as_serialized()?,
        EffectPassDispatchStrategy::EveryOutputLayerPixel(output_port),
        inputs,
        vec![SerializableEffectOutputSlot {
            name: STROKE_RESULT.into(),
            id: output_id,
            ty: GraphValueType::id(layer_ty.as_ref()).id,
        }],
    ))
}
