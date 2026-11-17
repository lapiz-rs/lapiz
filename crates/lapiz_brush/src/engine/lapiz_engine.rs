use std::sync::Arc;

use anyhow::{Context as _, Result};
use iced_core::{Element, Length, Point, Theme};
use iced_runtime::Task;
use indexmap::IndexMap;
use lapiz_assets::{
    AssetAppExt as _,
    asset::{AssetHandle, AssetId},
    store::AssetRegistry,
};
use lapiz_effect::{
    asset::{EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy, EffectPassId},
    editor::{EffectEditorMessage, EffectEditorState, EffectEditorView},
    instance::{EffectInputSlot, EffectInstance, EffectOutputSlot, EffectPass},
    nodes::{PassInput, PassInputNode, PassOutput, PassOutputNode},
};
use lapiz_i18n::t;
use lapiz_image::texel::TexelType;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_shader_graph::{
    GraphElement,
    editor::{
        GraphEditor,
        parameters::{ParametersEditor, ParametersEditorMessage},
    },
    graph::{Graph, slot::ErasedGraphValueType},
    wgsl_std::types::{handle::LayerType, primitive::F32Type},
};
use lapiz_widgets::{button, column, component::component, label, panel, row, tab_bar, text_input};
use uuid::Uuid;

use crate::{
    asset::{BrushPreset, BrushPresetMetadata},
    engine::{BrushEngine, BrushEngineId, BrushPresetEntry},
    instance::{
        BrushParameter, BrushPresetInstance, main_effect_resources, postprocess_effect_resources,
        spacing_effect_resources,
    },
    render::graph::{MAIN_DAB_BUFFER, SPACING_OUTPUT, STROKE_RESULT},
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
    selected: Option<SelectedBrush>,
    active_slot: BrushEffectSlot,
    name_buffer: String,
    dirty: bool,
}

struct SelectedBrush {
    handle: AssetHandle<BrushPreset>,
    instance: BrushPresetInstance,
    editor_states: BrushEffectEditorStates,
}

struct BrushEffectEditorStates {
    spacing: EffectEditorState,
    main: EffectEditorState,
    postprocess: EffectEditorState,
}

impl BrushEffectEditorStates {
    fn new(assets: &AssetRegistry) -> Self {
        Self {
            spacing: EffectEditorState::new(spacing_effect_resources(assets.clone())),
            main: EffectEditorState::new(main_effect_resources(assets.clone())),
            postprocess: EffectEditorState::new(postprocess_effect_resources(assets.clone())),
        }
    }

    fn get(&self, slot: BrushEffectSlot) -> &EffectEditorState {
        match slot {
            BrushEffectSlot::Spacing => &self.spacing,
            BrushEffectSlot::Main => &self.main,
            BrushEffectSlot::Postprocess => &self.postprocess,
        }
    }

    fn get_mut(&mut self, slot: BrushEffectSlot) -> &mut EffectEditorState {
        match slot {
            BrushEffectSlot::Spacing => &mut self.spacing,
            BrushEffectSlot::Main => &mut self.main,
            BrushEffectSlot::Postprocess => &mut self.postprocess,
        }
    }
}

#[derive(Clone)]
pub enum LapizBrushEngineMessage {
    BrushNameChanged(String),
    Save,
    SelectEffectSlot(BrushEffectSlot),
    Effect(EffectEditorMessage),
    Parameters(ParametersEditorMessage),
}

impl Default for LapizBrushEngine {
    fn default() -> Self {
        Self {
            selected: None,
            active_slot: BrushEffectSlot::Main,
            name_buffer: String::new(),
            dirty: false,
        }
    }
}

impl BrushEngine for LapizBrushEngine {
    type Message = LapizBrushEngineMessage;
    type Brush = BrushPreset;

    fn id() -> BrushEngineId {
        BrushEngineId::new("lapiz_brush_engine")
    }

    fn label() -> String {
        t!("lapiz_brush_engine")
    }

    fn query_brushes(&self, assets: &AssetRegistry) -> Result<Vec<BrushPresetEntry<Self::Brush>>> {
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
        &mut self,
        asset: AssetId<Self::Brush>,
        assets: &AssetRegistry,
        _globals: &mut Globals,
    ) -> Result<()> {
        let handle = assets.handle(asset)?;
        let mut instance = BrushPresetInstance::new(
            handle.get()?.as_ref(),
            Some(handle.id().into_untyped()),
            assets.clone(),
        )?;
        let mut editor_states = BrushEffectEditorStates::new(assets);

        for slot in [BrushEffectSlot::Spacing, BrushEffectSlot::Main] {
            let effect = effect_mut(&mut instance, slot);
            let pass_id = effect
                .single_pass_id()
                .expect("loaded brush spacing and main effects each have one pass");
            editor_states
                .get_mut(slot)
                .update(effect, EffectEditorMessage::OpenPass(pass_id));
        }

        self.name_buffer = instance.metadata().name.clone();
        self.selected = Some(SelectedBrush {
            handle,
            instance,
            editor_states,
        });
        self.active_slot = BrushEffectSlot::Main;
        self.dirty = false;
        Ok(())
    }

    fn new_brush(
        &mut self,
        assets: &AssetRegistry,
        globals: &mut Globals,
    ) -> Result<AssetId<Self::Brush>> {
        let preset = BrushPreset {
            metadata: BrushPresetMetadata {
                name: "[Unnamed Brush]".into(),
            },
            spacing_effect: new_spacing_effect(assets.clone()).as_asset()?,
            main_effect: new_main_effect(assets.clone())?.as_asset()?,
            postprocess_effect: new_postprocess_effect().as_asset()?,
            parameters: Default::default(),
        };
        let bundle = assets
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .map(|bundle| bundle.metadata().bundle_id)
            .context("A writable asset bundle is required for a new brush preset")?;
        let path = format!("unnamed_brush_{}.lapiz", Uuid::new_v4());
        let id = assets.add_asset(bundle, path, Arc::new(preset))?;
        self.load_brush(id, assets, globals)?;
        self.dirty = true;
        Ok(id)
    }

    fn compile_brush(&self, globals: &Globals) -> Result<BrushPreset> {
        self.selected
            .as_ref()
            .context("A selected brush is required for compilation")?
            .instance
            .as_asset(globals.assets())
    }

    fn editor_view<'a>(
        &'a self,
        _globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        match self.selected.as_ref() {
            Some(selected) => row![self.view_selected(selected), self.view_parameters(selected)]
                .gap(8.0)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => row![label(t!("select_a_brush")).muted(), label("")]
                .gap(8.0)
                .into(),
        }
    }

    fn editor_update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> Task<Self::Message> {
        match message {
            LapizBrushEngineMessage::BrushNameChanged(name) => {
                self.name_buffer = name.clone();
                if let Some(selected) = self.selected.as_mut() {
                    selected.instance.metadata_mut().name = name;
                    self.dirty = true;
                }
            }
            LapizBrushEngineMessage::Save => {
                if let Err(error) = self.save(globals) {
                    log::error!("Failed to save brush preset: {error:#}");
                }
            }
            LapizBrushEngineMessage::SelectEffectSlot(slot) => {
                self.active_slot = slot;
            }
            LapizBrushEngineMessage::Effect(message) => {
                if let Some(selected) = self.selected.as_mut() {
                    selected.editor_states.get_mut(self.active_slot).update(
                        effect_mut(&mut selected.instance, self.active_slot),
                        message,
                    );
                }
            }
            LapizBrushEngineMessage::Parameters(message) => {
                if let Some(selected) = self.selected.as_mut() {
                    apply_parameter_message(&mut selected.instance, message);
                    self.dirty = true;
                }
            }
        }
        Task::none()
    }
}

type EditorElement<'a> = GraphElement<'a, LapizBrushEngineMessage>;

impl LapizBrushEngine {
    fn view_selected<'a>(&'a self, selected: &'a SelectedBrush) -> EditorElement<'a> {
        let save_label = if self.dirty {
            t!("save_dirty")
        } else {
            t!("save")
        };
        let naming = row![
            label(t!("name")),
            text_input("", &self.name_buffer)
                .on_input(LapizBrushEngineMessage::BrushNameChanged)
                .width(Length::Fill),
            button(label(save_label))
                .primary()
                .on_press_maybe(self.dirty.then_some(LapizBrushEngineMessage::Save)),
        ]
        .gap(6.0)
        .height(Length::Shrink);

        let effect_editor = match self.active_slot {
            BrushEffectSlot::Spacing | BrushEffectSlot::Main => {
                let state = selected.editor_states.get(self.active_slot);
                let instance = effect(&selected.instance, self.active_slot);
                let (pass_id, pass) = (
                    instance.single_pass_id().unwrap(),
                    instance.single_pass().unwrap(),
                );

                GraphElement::from(GraphEditor::new(
                    &pass.graph,
                    &state.graph_editor_states[&pass_id],
                ))
                .map(|message| LapizBrushEngineMessage::Effect(EffectEditorMessage::Graph(message)))
            }
            BrushEffectSlot::Postprocess => GraphElement::from(EffectEditorView::new(
                effect(&selected.instance, self.active_slot),
                selected.editor_states.get(self.active_slot),
            ))
            .map(LapizBrushEngineMessage::Effect),
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
        let resources = &selected.editor_states.get(self.active_slot).resources;
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

    fn save(&mut self, globals: &Globals) -> Result<()> {
        let Some(selected) = self.selected.as_ref() else {
            return Ok(());
        };
        let preset = self.compile_brush(globals)?;
        selected.handle.update(preset)?;
        selected.handle.write()?;
        self.dirty = false;
        Ok(())
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
    for pass in effect.passes.values_mut() {
        for node in pass.graph.iter_nodes_mut() {
            if let Some(state) = node.data.state_mut::<PassInputNode>()
                && let Some(PassInput::Effect(id)) = state.input
                && !effect.inputs.contains_key(&id)
            {
                state.input = None;
            }
        }
    }
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

fn new_spacing_effect(assets: AssetRegistry) -> EffectInstance {
    let output_id = EffectOutputSlotId::new(Uuid::new_v4());

    EffectInstance {
        name: "Spacing".into(),
        passes: IndexMap::from_iter([(
            EffectPassId::new(Uuid::new_v4()),
            EffectPass {
                name: "Spacing".into(),
                graph: Graph::new(spacing_effect_resources(assets)),
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
    }
}

fn new_main_effect(assets: AssetRegistry) -> Result<EffectInstance> {
    let output_id = EffectOutputSlotId::new(Uuid::new_v4());
    let mut graph = Graph::new(main_effect_resources(assets));
    let node = graph.add_node(Point::ORIGIN, PassOutputNode);
    graph.update_node_state::<PassOutputNode>(node, |state| {
        state.output = Some(PassOutput::Effect(output_id));
    });
    let port = graph
        .get_node(&node)
        .unwrap()
        .data
        .state::<PassOutputNode>()
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
