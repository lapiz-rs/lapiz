use std::sync::Arc;

use anyhow::{Context as _, Result};
use iced_core::{Element, Length, Point, Theme};
use iced_runtime::Task;
use indexmap::IndexMap;
use lapiz_assets::{AssetAppExt as _, asset::AssetId, store::AssetRegistry};
use lapiz_effect::{
    asset::{EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy, EffectPassId},
    editor::{EffectEditor, EffectEditorMessage},
    instance::{EffectInputSlot, EffectInstance, EffectOutputSlot, EffectPass},
    shader_graph::{
        GraphElement,
        builtin::types::{handle::LayerType, primitive::F32Type},
        editor::{
            GraphEditor,
            parameters::{ParametersEditor, ParametersEditorMessage},
        },
        graph::{Graph, GraphResources, slot::ErasedGraphValueType},
    },
};
use lapiz_i18n::t;
use lapiz_image::texel::TexelType;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::{column, component::component, label, panel, row, tab_bar, text_input};
use uuid::Uuid;

use crate::{
    asset::{BrushPreset, BrushPresetMetadata, BrushPresetSerializer},
    engine::{BrushEngine, BrushEngineId, BrushPresetEntry},
    instance::{
        BrushParameter, BrushPresetInstance, main_effect_resources, postprocess_effect_resources,
        spacing_effect_resources,
    },
    render::graph::{
        MAIN_DAB_BUFFER, OutputMainDabBufferNode, OutputSpacingNode, SPACING_OUTPUT, STROKE_RESULT,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrushEffectSlot {
    Spacing,
    Main,
    Postprocess,
}

impl BrushEffectSlot {
    pub const ALL: [BrushEffectSlot; 3] = [
        BrushEffectSlot::Spacing,
        BrushEffectSlot::Main,
        BrushEffectSlot::Postprocess,
    ];

    fn label(self) -> String {
        match self {
            BrushEffectSlot::Spacing => t!("spacing_effect"),
            BrushEffectSlot::Main => t!("main_effect"),
            BrushEffectSlot::Postprocess => t!("postprocess_effect"),
        }
    }
}

pub struct LapizBrushEngine {
    selected: SelectedBrush,
    active_slot: BrushEffectSlot,
    name_buffer: String,
    dirty: bool,
}

struct SelectedBrush {
    instance: BrushPresetInstance,
    resources: BrushEffectResources,
}

struct BrushEffectResources {
    spacing: GraphResources,
    main: GraphResources,
    postprocess: GraphResources,
}

impl BrushEffectResources {
    fn new(assets: &AssetRegistry) -> Self {
        Self {
            spacing: spacing_effect_resources(assets.clone()),
            main: main_effect_resources(assets.clone()),
            postprocess: postprocess_effect_resources(assets.clone()),
        }
    }

    fn get(&self, slot: BrushEffectSlot) -> &GraphResources {
        match slot {
            BrushEffectSlot::Spacing => &self.spacing,
            BrushEffectSlot::Main => &self.main,
            BrushEffectSlot::Postprocess => &self.postprocess,
        }
    }
}

#[derive(Clone)]
pub enum LapizBrushEngineMessage {
    BrushNameChanged(String),
    SelectEffectSlot(BrushEffectSlot),
    Effect(EffectEditorMessage),
    Parameters(ParametersEditorMessage),
}

impl BrushEngine for LapizBrushEngine {
    type Message = LapizBrushEngineMessage;
    type Brush = BrushPreset;
    type BrushSerializer = BrushPresetSerializer;

    fn id() -> BrushEngineId {
        BrushEngineId::new("lapiz_brush_engine")
    }

    fn label() -> String {
        t!("lapiz_brush_engine")
    }

    fn query_brushes(assets: &AssetRegistry) -> Result<Vec<BrushPresetEntry<Self::Brush>>> {
        Ok(assets
            .all_handles_of::<BrushPreset>()?
            .into_iter()
            .map(|handle| BrushPresetEntry {
                id: handle.id(),
                name: handle
                    .get()
                    .map(|preset| preset.metadata.name.clone())
                    .unwrap_or_else(|_| "<loading>".to_string()),
            })
            .collect())
    }

    fn load_brush(
        brush: &Self::Brush,
        asset_id: Option<AssetId<Self::Brush>>,
        assets: &AssetRegistry,
    ) -> Result<Self>
    where
        Self: Sized,
    {
        let instance =
            BrushPresetInstance::new(brush, asset_id.map(|id| id.into_untyped()), assets.clone())?;
        let resources = BrushEffectResources::new(assets);

        Ok(Self {
            name_buffer: instance.metadata().name.clone(),
            selected: SelectedBrush {
                instance,
                resources,
            },
            active_slot: BrushEffectSlot::Main,
            dirty: false,
        })
    }

    fn new_brush(assets: &AssetRegistry) -> Result<Self::Brush> {
        Ok(BrushPreset {
            metadata: BrushPresetMetadata {
                name: "[Unnamed Brush]".into(),
            },
            spacing_effect: new_spacing_effect(assets.clone())?.as_asset()?,
            main_effect: new_main_effect(assets.clone())?.as_asset()?,
            postprocess_effect: new_postprocess_effect().as_asset()?,
            parameters: Default::default(),
        })
    }

    fn brush_asset_id(&self) -> Option<AssetId<Self::Brush>> {
        self.selected.instance.asset_id().map(|id| id.into_typed())
    }

    fn compile_brush(&self, globals: &Globals) -> Result<BrushPreset> {
        self.selected.instance.as_asset(globals.assets())
    }

    fn editor_view<'a>(
        &'a self,
        _globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        row![
            self.view_selected(&self.selected),
            self.view_parameters(&self.selected)
        ]
        .gap(8.0)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn editor_update(
        &mut self,
        message: Self::Message,
        _globals: &mut Globals,
    ) -> Task<Self::Message> {
        match message {
            LapizBrushEngineMessage::BrushNameChanged(name) => {
                self.name_buffer = name.clone();
                self.selected.instance.metadata_mut().name = name;
                self.dirty = true;
            }
            LapizBrushEngineMessage::SelectEffectSlot(slot) => {
                self.active_slot = slot;
            }
            LapizBrushEngineMessage::Effect(message) => {
                effect_mut(&mut self.selected.instance, self.active_slot)
                    .update(message, self.selected.resources.get(self.active_slot));
            }
            LapizBrushEngineMessage::Parameters(message) => {
                apply_parameter_message(&mut self.selected.instance, message);
                self.dirty = true;
            }
        }
        Task::none()
    }

    fn editor_dirty(&self) -> bool {
        self.dirty
    }

    fn editor_save(&mut self, globals: &Globals) -> Result<()> {
        let asset_id = self
            .selected
            .instance
            .asset_id()
            .context("Brush has no asset ID")?;
        let preset = self.compile_brush(globals)?;
        let assets = globals.assets();
        let handle = assets.handle(asset_id.into_typed())?;
        handle.update(preset)?;
        handle.write()?;
        self.dirty = false;
        Ok(())
    }
}

type EditorElement<'a> = GraphElement<'a, LapizBrushEngineMessage>;

impl LapizBrushEngine {
    fn view_selected<'a>(&'a self, selected: &'a SelectedBrush) -> EditorElement<'a> {
        let naming = row![
            label(t!("name")),
            text_input("", &self.name_buffer)
                .on_input(LapizBrushEngineMessage::BrushNameChanged)
                .width(Length::Fill),
        ]
        .gap(6.0)
        .height(Length::Shrink);

        let effect_editor = match self.active_slot {
            BrushEffectSlot::Spacing | BrushEffectSlot::Main => {
                let instance = effect(&selected.instance, self.active_slot);
                let pass_id = instance.single_pass_id().unwrap();
                let pass = instance.single_pass().unwrap();

                component(GraphEditor::new(&pass.graph, move |message| {
                    LapizBrushEngineMessage::Effect(EffectEditorMessage::Graph(pass_id, message))
                }))
            }
            BrushEffectSlot::Postprocess => component(EffectEditor::new(
                effect(&selected.instance, self.active_slot),
                LapizBrushEngineMessage::Effect,
            )),
        };

        let selected_slot = BrushEffectSlot::ALL
            .iter()
            .position(|slot| *slot == self.active_slot)
            .unwrap();
        let slots = tab_bar(
            BrushEffectSlot::ALL.into_iter().map(BrushEffectSlot::label),
            selected_slot,
        )
        .on_select(|index| LapizBrushEngineMessage::SelectEffectSlot(BrushEffectSlot::ALL[index]));

        column![naming, effect_editor, slots]
            .gap(6.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_parameters<'a>(&'a self, selected: &'a SelectedBrush) -> EditorElement<'a> {
        let resources = selected.resources.get(self.active_slot);
        let parameters = component(ParametersEditor::new(
            selected
                .instance
                .parameters()
                .values()
                .map(|parameter| (parameter.name.as_str(), &parameter.value)),
            &resources.type_registry,
            &resources.assets,
            LapizBrushEngineMessage::Parameters,
        ));

        panel(column![label(t!("parameters")).strong(), parameters].gap(6.0))
            .padding(8)
            .width(320)
            .height(Length::Fill)
            .into()
    }
}

fn apply_parameter_message(instance: &mut BrushPresetInstance, message: ParametersEditorMessage) {
    match message {
        ParametersEditorMessage::Add { name, value } => {
            instance.parameters_mut().insert(
                EffectInputSlotId::new(Uuid::new_v4()),
                BrushParameter { name, value },
            );
        }
        ParametersEditorMessage::Remove { index } => {
            instance.parameters_mut().shift_remove_index(index);
        }
        ParametersEditorMessage::Rename { index, name } => {
            if let Some((_, parameter)) = instance.parameters_mut().get_index_mut(index) {
                parameter.name = name;
            }
        }
        ParametersEditorMessage::UpdateLiteral { index, message } => {
            if let Some((_, parameter)) = instance.parameters_mut().get_index_mut(index) {
                parameter.value.update(message);
            }
        }
    }

    let parameters = instance
        .parameters()
        .iter()
        .map(|(id, parameter)| (*id, parameter.name.clone(), parameter.value.ty().clone()))
        .collect::<Vec<_>>();
    sync_brush_effect_parameters(instance.spacing_effect_mut(), &parameters);
    sync_brush_effect_parameters(instance.main_effect_mut(), &parameters);
    sync_brush_effect_parameters(instance.postprocess_effect_mut(), &parameters);
}

fn sync_brush_effect_parameters(
    effect: &mut EffectInstance,
    parameters: &[(EffectInputSlotId, String, Arc<dyn ErasedGraphValueType>)],
) {
    effect.inputs = parameters
        .iter()
        .map(|(id, name, ty)| {
            (
                *id,
                EffectInputSlot {
                    name: name.clone(),
                    id: *id,
                    ty: ty.clone(),
                },
            )
        })
        .collect();
    effect
        .sync_pass_graph_effect_properties()
        .expect("brush parameter inputs always sync");
}

fn effect(instance: &BrushPresetInstance, slot: BrushEffectSlot) -> &EffectInstance {
    match slot {
        BrushEffectSlot::Spacing => instance.spacing_effect(),
        BrushEffectSlot::Main => instance.main_effect(),
        BrushEffectSlot::Postprocess => instance.postprocess_effect(),
    }
}

fn effect_mut(instance: &mut BrushPresetInstance, slot: BrushEffectSlot) -> &mut EffectInstance {
    match slot {
        BrushEffectSlot::Spacing => instance.spacing_effect_mut(),
        BrushEffectSlot::Main => instance.main_effect_mut(),
        BrushEffectSlot::Postprocess => instance.postprocess_effect_mut(),
    }
}

fn new_spacing_effect(assets: AssetRegistry) -> Result<EffectInstance> {
    let output_id = EffectOutputSlotId::new(Uuid::new_v4());
    let mut graph = Graph::new(spacing_effect_resources(assets));
    graph.add_node(Point::ORIGIN, OutputSpacingNode);

    let mut effect = EffectInstance {
        name: "Spacing".into(),
        passes: IndexMap::from_iter([(
            EffectPassId::new(Uuid::new_v4()),
            EffectPass {
                name: "Spacing".into(),
                graph,
                dispatch_strategy: EffectPassDispatchStrategy::Once,
            },
        )]),
        inputs: Default::default(),
        outputs: IndexMap::from_iter([(
            output_id,
            EffectOutputSlot {
                name: SPACING_OUTPUT.into(),
                id: output_id,
                ty: Arc::new(F32Type),
            },
        )]),
    };
    effect.sync_pass_graph_effect_properties()?;
    Ok(effect)
}

fn new_main_effect(assets: AssetRegistry) -> Result<EffectInstance> {
    let output_id = EffectOutputSlotId::new(Uuid::new_v4());
    let mut graph = Graph::new(main_effect_resources(assets));
    let node = graph.add_node(Point::ORIGIN, OutputMainDabBufferNode);
    let port = graph
        .get_node(&node)
        .unwrap()
        .data
        .state::<OutputMainDabBufferNode>()
        .unwrap()
        .id;
    let mut effect = EffectInstance {
        name: "Main".into(),
        passes: IndexMap::from_iter([(
            EffectPassId::new(Uuid::new_v4()),
            EffectPass {
                name: "Main".into(),
                graph,
                dispatch_strategy: EffectPassDispatchStrategy::EveryOutputLayerPixel(port),
            },
        )]),
        inputs: Default::default(),
        outputs: IndexMap::from_iter([(
            output_id,
            EffectOutputSlot {
                name: MAIN_DAB_BUFFER.into(),
                id: output_id,
                ty: Arc::new(LayerType {
                    texel_type: TexelType::RGBA8,
                }),
            },
        )]),
    };
    effect.sync_pass_graph_effect_properties()?;
    Ok(effect)
}

fn new_postprocess_effect() -> EffectInstance {
    let output_id = EffectOutputSlotId::new(Uuid::new_v4());

    EffectInstance {
        name: "Postprocess".into(),
        passes: Default::default(),
        inputs: Default::default(),
        outputs: IndexMap::from_iter([(
            output_id,
            EffectOutputSlot {
                name: STROKE_RESULT.into(),
                id: output_id,
                ty: Arc::new(LayerType {
                    texel_type: TexelType::RGBA8,
                }),
            },
        )]),
    }
}
