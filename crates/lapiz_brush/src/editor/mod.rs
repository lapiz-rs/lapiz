use std::sync::Arc;

use anyhow::Result;
use iced_core::{Element, Length, Size, Theme, alignment::Vertical, window};
use iced_futures::Subscription;
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use iced_widget::{Column, column, component::component, row};
use lapiz_assets::{AssetAppExt as _, asset::AssetHandle, store::AssetRegistry};
use lapiz_effect::{
    asset::{EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy, EffectPassId},
    editor::{EffectEditorMessage, EffectEditorState, EffectEditorView},
    instance::{EffectInputSlot, EffectInstance, EffectOutputSlot, EffectPass},
    nodes::{PassInput, PassInputNode},
};
use lapiz_i18n::t;
use lapiz_image::texel::TexelType;
use lapiz_runtime::{
    Services,
    windows::{WindowView, WindowViewId},
};
use lapiz_shader_graph::{
    GraphElement,
    editor::parameters::{ParametersEditor, ParametersEditorMessage},
    graph::{Graph, slot::ErasedGraphValueType},
    wgsl_std::types::{handle::LayerType, primitive::F32Type},
};
use lapiz_widgets::{
    button::Button, flex::Flex, label::Label, panel::Panel, scrollable::Scrollable, tabs::TabBar,
    text_input::TextInput, title_bar::TitleBar,
};
use uuid::Uuid;

use crate::{
    asset::{BrushPreset, BrushPresetMetadata},
    instance::{
        BrushParameter, BrushPresetInstance, main_effect_resources, postprocess_effect_resources,
        spacing_effect_resources,
    },
    render::graph::{MAIN_DAB_BUFFER, SPACING_OUTPUT, STROKE_RESULT},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub struct BrushEditor {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    brushes: Vec<AssetHandle<BrushPreset>>,
    selected_index: Option<usize>,
    selected: Option<SelectedBrush>,
    editor_states: BrushEffectEditorStates,
    active_slot: BrushEffectSlot,
    name_buffer: String,
    dirty: bool,
}

struct SelectedBrush {
    handle: AssetHandle<BrushPreset>,
    instance: BrushPresetInstance,
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
pub enum BrushEditorMessage {
    SelectBrush(usize),
    NewBrush,
    BrushNameChanged(String),
    Save,
    SelectEffectSlot(BrushEffectSlot),
    Effect(EffectEditorMessage),
    Parameters(ParametersEditorMessage),

    Close,
    Maximize,
    Minimize,
    Drag,
}

impl WindowView for BrushEditor {
    type Message = BrushEditorMessage;

    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new("brush_editor")
    }

    fn boot(
        _params: Option<Self::BootParams>,
        services: &mut Services,
    ) -> Result<(Self, Task<Self::Message>)> {
        let brushes = services
            .assets()
            .all_handles_of::<BrushPreset>()
            .expect("Failed to list brush presets");
        let (main_window, open) = open(window::Settings {
            decorations: false,
            size: Size {
                width: 1280.0,
                height: 800.0,
            },
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific {
                corner_preference: window::settings::platform::CornerPreference::DoNotRound,
                ..Default::default()
            },
            ..Default::default()
        });
        Ok((
            Self {
                windows: [main_window].into(),
                main_window,
                brushes,
                selected_index: None,
                selected: None,
                editor_states: BrushEffectEditorStates::new(services.assets()),
                active_slot: BrushEffectSlot::Main,
                name_buffer: String::new(),
                dirty: false,
            },
            open.discard(),
        ))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        _: &'a Services,
    ) -> impl Into<Element<'a, Self::Message, Theme, lapiz_runtime::Renderer>> {
        let titlebar = TitleBar::new(Label::new(t!("brush_editor_title")).window_title())
            .on_close(BrushEditorMessage::Close)
            .on_maximize(BrushEditorMessage::Maximize)
            .on_minimize(BrushEditorMessage::Minimize)
            .on_drag(BrushEditorMessage::Drag);

        let brush_list = self
            .brushes
            .iter()
            .enumerate()
            .map(|(index, handle)| {
                let name = handle
                    .get()
                    .map(|preset| preset.metadata.name.clone())
                    .unwrap_or_else(|_| "<loading>".to_string());
                Button::new(Label::new(name))
                    .width(Length::Fill)
                    .activated(self.selected_index == Some(index))
                    .on_press(BrushEditorMessage::SelectBrush(index))
                    .into()
            })
            .collect::<Vec<_>>();

        // Region 1: create a brush preset on top, pick one below.
        let sidebar = Panel::new(
            column![
                Button::new(Label::new(t!("new_brush"))).on_press(BrushEditorMessage::NewBrush),
                Label::new(t!("brushes")).strong(),
                Scrollable::new(Column::with_children(brush_list).spacing(2))
                    .width(Length::Fill)
                    .height(Length::Fill),
            ]
            .spacing(6),
        )
        .padding(8)
        .width(220);

        let empty = || -> EditorElement<'_> { Label::new("").into() };
        let (center, parameters) = match self.selected.as_ref() {
            Some(selected) => (self.view_selected(selected), self.view_parameters(selected)),
            None => (Label::new(t!("select_a_brush")).muted().into(), empty()),
        };

        Panel::new(Flex::column([
            titlebar.into(),
            row![sidebar, center, parameters]
                .spacing(8)
                .height(Length::Fill)
                .into(),
        ]))
    }

    fn update(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            BrushEditorMessage::SelectBrush(index) => self.select_brush(index, services),
            BrushEditorMessage::NewBrush => self.new_brush(services),
            BrushEditorMessage::BrushNameChanged(name) => {
                self.name_buffer = name.clone();
                if let Some(selected) = self.selected.as_mut() {
                    selected.instance.metadata_mut().name = name;
                    self.dirty = true;
                }
                Task::none()
            }
            BrushEditorMessage::Save => self.save(services),
            BrushEditorMessage::SelectEffectSlot(slot) => {
                self.active_slot = slot;
                Task::none()
            }
            BrushEditorMessage::Effect(message) => {
                if let Some(selected) = self.selected.as_mut() {
                    self.editor_states.get_mut(self.active_slot).update(
                        effect_mut(&mut selected.instance, self.active_slot),
                        message,
                    );
                    self.dirty = true;
                }
                Task::none()
            }
            BrushEditorMessage::Parameters(message) => {
                if let Some(selected) = self.selected.as_mut() {
                    apply_parameter_message(&mut selected.instance, message);
                    self.dirty = true;
                }
                Task::none()
            }
            BrushEditorMessage::Close => close(self.main_window),
            BrushEditorMessage::Maximize => toggle_maximize(self.main_window),
            BrushEditorMessage::Minimize => minimize(self.main_window, true),
            BrushEditorMessage::Drag => drag(self.main_window),
        }
    }

    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        Subscription::none()
    }

    fn close(self, _: &mut Services) -> Task<()> {
        close(self.main_window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.main_window)
    }
}

type EditorElement<'a> = GraphElement<'a, BrushEditorMessage>;

impl BrushEditor {
    fn view_selected<'a>(&'a self, selected: &'a SelectedBrush) -> EditorElement<'a> {
        let save_label = if self.dirty {
            t!("save_dirty")
        } else {
            t!("save")
        };
        let naming = row![
            Label::new(t!("name")),
            TextInput::new("", &self.name_buffer)
                .on_input(BrushEditorMessage::BrushNameChanged)
                .width(Length::Fill),
            Button::new(Label::new(save_label))
                .primary()
                .on_press_maybe(self.dirty.then_some(BrushEditorMessage::Save)),
        ]
        .spacing(6)
        .align_y(Vertical::Center)
        .height(Length::Shrink);

        let effect_editor = GraphElement::from(EffectEditorView::new(
            effect(&selected.instance, self.active_slot),
            self.editor_states.get(self.active_slot),
        ))
        .map(BrushEditorMessage::Effect);

        let slots = BrushEffectSlot::ALL
            .into_iter()
            .fold(TabBar::new(), |tabs, slot| {
                tabs.push(
                    Label::new(slot.label()),
                    slot == self.active_slot,
                    BrushEditorMessage::SelectEffectSlot(slot),
                )
            });

        column![naming, effect_editor, slots.width(Length::Fill)]
            .spacing(6)
            .height(Length::Fill)
            .into()
    }

    fn view_parameters<'a>(&'a self, selected: &'a SelectedBrush) -> EditorElement<'a> {
        let resources = &self.editor_states.get(self.active_slot).resources;
        let parameters = component(ParametersEditor::new(
            selected
                .instance
                .parameters()
                .values()
                .map(|parameter| (parameter.name.as_str(), &parameter.value)),
            &resources.type_registry,
            &resources.assets,
            BrushEditorMessage::Parameters,
        ));

        Panel::new(column![Label::new(t!("parameters")).strong(), parameters].spacing(6))
            .padding(8)
            .width(320)
            .height(Length::Fill)
            .into()
    }

    fn select_brush(&mut self, index: usize, services: &Services) -> Task<BrushEditorMessage> {
        let Some(handle) = self.brushes.get(index).cloned() else {
            return Task::none();
        };
        let instance = match BrushPresetInstance::from_asset(&handle, services.assets().clone()) {
            Ok(instance) => instance,
            Err(error) => {
                log::error!("Failed to load brush preset: {error:#}");
                return Task::none();
            }
        };
        self.selected_index = Some(index);
        self.name_buffer = instance.metadata().name.clone();
        self.selected = Some(SelectedBrush { handle, instance });
        self.editor_states = BrushEffectEditorStates::new(services.assets());
        self.active_slot = BrushEffectSlot::Main;
        self.dirty = false;
        Task::none()
    }

    fn new_brush(&mut self, services: &mut Services) -> Task<BrushEditorMessage> {
        let assets = services.assets().clone();
        let f32_ty = Arc::new(F32Type) as Arc<dyn ErasedGraphValueType>;
        let layer_ty = Arc::new(LayerType {
            texel_type: TexelType::RGBA8,
        });

        let mut spacing = conventional_effect("Spacing", [(SPACING_OUTPUT, f32_ty)]);
        // The spacing effect executes inside input sampling and must stay a
        // single pass, so new brushes start with that pass.
        spacing.passes.insert(
            EffectPassId::new(Uuid::new_v4()),
            EffectPass {
                name: "Spacing".into(),
                graph: Graph::new(spacing_effect_resources(assets.clone())),
                dispatch_strategy: EffectPassDispatchStrategy::Once,
            },
        );

        let preset = BrushPreset {
            metadata: BrushPresetMetadata {
                name: "[Unnamed Brush]".into(),
            },
            spacing_effect: spacing
                .as_asset()
                .expect("freshly built effects always serialize"),
            main_effect: conventional_effect("Main", [(MAIN_DAB_BUFFER, layer_ty.clone() as _)])
                .as_asset()
                .expect("freshly built effects always serialize"),
            postprocess_effect: conventional_effect(
                "Postprocess",
                [(STROKE_RESULT, layer_ty as _)],
            )
            .as_asset()
            .expect("freshly built effects always serialize"),
            parameters: Default::default(),
        };
        let Some(bundle) = services
            .assets()
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .map(|bundle| bundle.metadata().bundle_id)
        else {
            log::error!("No writable asset bundle available for a new brush preset");
            return Task::none();
        };
        let path = format!("unnamed_brush_{}.lapiz", Uuid::new_v4());
        let id = match services.assets().add_asset(bundle, path, Arc::new(preset)) {
            Ok(id) => id,
            Err(error) => {
                log::error!("Failed to add new brush preset asset: {error:#}");
                return Task::none();
            }
        };
        let Ok(handle) = services.assets().handle(id) else {
            log::error!("Failed to obtain handle for new brush preset");
            return Task::none();
        };
        let index = self.brushes.len();
        self.brushes.push(handle);
        let task = self.select_brush(index, services);
        self.dirty = true;
        task
    }

    fn save(&mut self, services: &Services) -> Task<BrushEditorMessage> {
        let Some(selected) = self.selected.as_mut() else {
            return Task::none();
        };
        let preset = match selected.instance.as_asset(services.assets()) {
            Ok(preset) => preset,
            Err(error) => {
                log::error!("Failed to serialize brush preset: {error:#}");
                return Task::none();
            }
        };
        if let Err(error) = selected.handle.update(preset) {
            log::error!("Failed to update brush preset: {error:#}");
            return Task::none();
        }
        if let Err(error) = selected.handle.write() {
            log::error!("Failed to write brush preset: {error:#}");
            return Task::none();
        }
        self.dirty = false;
        Task::none()
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

fn conventional_effect(
    name: &str,
    outputs: impl IntoIterator<Item = (impl Into<String>, Arc<dyn ErasedGraphValueType>)>,
) -> EffectInstance {
    EffectInstance {
        name: name.into(),
        passes: Default::default(),
        inputs: Default::default(),
        outputs: outputs
            .into_iter()
            .map(|(name, ty)| {
                let id = EffectOutputSlotId::new(Uuid::new_v4());
                (
                    id,
                    EffectOutputSlot {
                        name: name.into(),
                        id,
                        ty,
                    },
                )
            })
            .collect(),
    }
}
