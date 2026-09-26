use std::sync::Arc;

use anyhow::Result;
use iced_core::{Element, Length, Size, Theme, alignment::Vertical, window};
use iced_futures::Subscription;
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use iced_widget::{Column, column, component::component, row};
use lapiz_assets::{AssetAppExt as _, asset::AssetHandle};
use lapiz_effect::{
    asset::{EffectInputSlotId, EffectOutputSlotId},
    editor::{EffectEditorMessage, EffectEditorState, EffectEditorView},
    instance::{EffectInputSlot, EffectInstance, EffectOutputSlot},
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
    wgsl_std::types::handle::LayerType,
};
use lapiz_widgets::{
    button::Button, flex::Flex, label::Label, panel::Panel, scrollable::Scrollable,
    text_input::TextInput, title_bar::TitleBar,
};
use uuid::Uuid;

use crate::{
    asset::{FilterPreset, FilterPresetMetadata},
    instance::{FilterInstance, FilterParameter},
    render::graph::filter_graph_resources,
};

pub struct FilterEditor {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    filters: Vec<AssetHandle<FilterPreset>>,
    selected_index: Option<usize>,
    selected: Option<SelectedFilter>,
    effect_editor_state: EffectEditorState,
    filter_name_buffer: String,
    dirty: bool,
    validation_error: Option<String>,
}

pub struct SelectedFilter {
    pub handle: AssetHandle<FilterPreset>,
    pub instance: FilterInstance,
}

#[derive(Clone)]
pub enum FilterEditorMessage {
    SelectFilter(usize),
    NewFilter,
    FilterNameChanged(String),
    Save,
    Effect(EffectEditorMessage),
    Parameters(ParametersEditorMessage),

    Close,
    Maximize,
    Minimize,
    Drag,
}

impl WindowView for FilterEditor {
    type Message = FilterEditorMessage;

    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new("filter_editor")
    }

    fn boot(
        _params: Option<Self::BootParams>,
        services: &mut Services,
    ) -> Result<(Self, Task<Self::Message>)> {
        let filters = services
            .assets()
            .all_handles_of::<FilterPreset>()
            .expect("Failed to list filter presets");
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
                filters,
                selected_index: None,
                selected: None,
                effect_editor_state: EffectEditorState::new(filter_graph_resources(
                    services.assets().clone(),
                )),
                filter_name_buffer: String::new(),
                dirty: false,
                validation_error: None,
            },
            open.discard(),
        ))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        _: &'a Services,
    ) -> impl Into<Element<'a, Self::Message, Theme, lapiz_runtime::Renderer>> {
        let titlebar = TitleBar::new(Label::new(t!("filter_editor_title")).window_title())
            .on_close(FilterEditorMessage::Close)
            .on_maximize(FilterEditorMessage::Maximize)
            .on_minimize(FilterEditorMessage::Minimize)
            .on_drag(FilterEditorMessage::Drag);

        let filter_list = self
            .filters
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
                    .on_press(FilterEditorMessage::SelectFilter(index))
                    .into()
            })
            .collect::<Vec<_>>();

        // Region 1: pick or create a filter.
        let sidebar = Panel::new(
            column![
                Label::new(t!("filters")).strong(),
                Scrollable::new(Column::with_children(filter_list).spacing(2))
                    .width(Length::Fill)
                    .height(Length::Fill),
                Button::new(Label::new(t!("new_filter"))).on_press(FilterEditorMessage::NewFilter),
            ]
            .spacing(6),
        )
        .padding(8)
        .width(220);

        let empty = || -> EditorElement<'_> { Label::new("").into() };
        let (naming, effect_editor, parameters) = match self.selected.as_ref() {
            Some(selected) => self.view_selected(selected),
            None => (
                Label::new(t!("select_a_filter_to_adjust")).muted().into(),
                empty(),
                empty(),
            ),
        };

        // Region 2: naming on top, effect editor filling the rest.
        let center = column![naming, effect_editor]
            .spacing(6)
            .height(Length::Fill);

        Panel::new(Flex::column([
            titlebar.into(),
            row![sidebar, center, parameters].spacing(8).into(),
        ]))
        .height(Length::Fill)
    }

    fn update(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            FilterEditorMessage::SelectFilter(index) => self.select_filter(index, services),
            FilterEditorMessage::NewFilter => self.new_filter(services),
            FilterEditorMessage::FilterNameChanged(name) => {
                self.filter_name_buffer = name.clone();
                if let Some(selected) = self.selected.as_mut() {
                    selected.instance.metadata_mut().name = name;
                    self.dirty = true;
                }
                Task::none()
            }
            FilterEditorMessage::Save => self.save(services),
            FilterEditorMessage::Effect(message) => {
                if let Some(selected) = self.selected.as_mut() {
                    self.effect_editor_state
                        .update(selected.instance.effect_mut(), message);
                    self.dirty = true;
                    self.revalidate();
                }
                Task::none()
            }
            FilterEditorMessage::Parameters(message) => {
                if let Some(selected) = self.selected.as_mut() {
                    apply_parameter_message(&mut selected.instance, message);
                    self.dirty = true;
                    self.revalidate();
                }
                Task::none()
            }
            FilterEditorMessage::Close => close(self.main_window),
            FilterEditorMessage::Maximize => toggle_maximize(self.main_window),
            FilterEditorMessage::Minimize => minimize(self.main_window, true),
            FilterEditorMessage::Drag => drag(self.main_window),
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

type EditorElement<'a> = GraphElement<'a, FilterEditorMessage>;

fn apply_parameter_message(instance: &mut FilterInstance, message: ParametersEditorMessage) {
    match message {
        ParametersEditorMessage::Add { name, value } => {
            instance.parameters_mut().insert(
                EffectInputSlotId::new(Uuid::new_v4()),
                FilterParameter { name, value },
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
    let effect = instance.effect_mut();
    effect.inputs.retain(|_, slot| slot.ty.is::<LayerType>());
    effect.inputs.extend(
        parameters
            .into_iter()
            .map(|(id, name, ty)| (id, EffectInputSlot { name, id, ty })),
    );
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
        .expect("filter parameter inputs always sync");
}

impl FilterEditor {
    fn view_selected<'a>(
        &'a self,
        selected: &'a SelectedFilter,
    ) -> (EditorElement<'a>, EditorElement<'a>, EditorElement<'a>) {
        let status = if self.dirty {
            Label::new("*").muted()
        } else {
            Label::new("")
        };
        let naming_row = row![
            Label::new(t!("name")),
            TextInput::new("", &self.filter_name_buffer)
                .on_input(FilterEditorMessage::FilterNameChanged)
                .width(Length::Fill),
            Button::new(Label::new(t!("save")))
                .primary()
                .on_press_maybe(
                    (self.dirty && self.validation_error.is_none())
                        .then_some(FilterEditorMessage::Save),
                ),
            status,
        ]
        .spacing(6)
        .align_y(Vertical::Center)
        .height(Length::Shrink);

        let naming = match self.validation_error.as_ref() {
            Some(error) => {
                let error_text = EditorElement::<'a>::from(
                    iced_widget::Text::new(error.clone())
                        .color(iced_core::Color::from_rgb(1.0, 0.3, 0.3)),
                );
                column![naming_row, error_text].spacing(2).into()
            }
            None => naming_row.into(),
        };

        // Region 2 bottom: the effect editor (passes and their graphs).
        let effect_editor = GraphElement::from(EffectEditorView::new(
            selected.instance.effect(),
            &self.effect_editor_state,
        ))
        .map(FilterEditorMessage::Effect);

        let resources = &self.effect_editor_state.resources;
        let parameter_editor = component(ParametersEditor::new(
            selected
                .instance
                .parameters()
                .values()
                .map(|parameter| (parameter.name.as_str(), &parameter.value)),
            &resources.type_registry,
            &resources.assets,
            FilterEditorMessage::Parameters,
        ));
        let parameters =
            Panel::new(column![Label::new(t!("parameters")).strong(), parameter_editor].spacing(6))
                .padding(8)
                .width(320)
                .height(Length::Fill);

        (naming, effect_editor, parameters.into())
    }

    fn select_filter(&mut self, index: usize, services: &Services) -> Task<FilterEditorMessage> {
        let Some(handle) = self.filters.get(index).cloned() else {
            return Task::none();
        };
        let instance = match FilterInstance::from_asset(&handle, services.assets()) {
            Ok(instance) => instance,
            Err(e) => {
                log::error!("Failed to load filter preset: {e}");
                return Task::none();
            }
        };
        self.selected_index = Some(index);
        self.filter_name_buffer = instance.metadata().name.clone();
        self.selected = Some(SelectedFilter { handle, instance });
        self.effect_editor_state =
            EffectEditorState::new(filter_graph_resources(services.assets().clone()));
        self.dirty = false;
        self.validation_error = None;
        Task::none()
    }

    fn new_filter(&mut self, services: &mut Services) -> Task<FilterEditorMessage> {
        let layer_ty = Arc::new(LayerType {
            texel_type: TexelType::RGBA8,
        });
        let target = EffectInputSlotId::new(Uuid::new_v4());
        let output = EffectOutputSlotId::new(Uuid::new_v4());
        let effect = EffectInstance {
            name: "Filter".into(),
            passes: Default::default(),
            inputs: indexmap::IndexMap::from([(
                target,
                EffectInputSlot {
                    name: "Target".into(),
                    id: target,
                    ty: layer_ty.clone(),
                },
            )]),
            outputs: indexmap::IndexMap::from([(
                output,
                EffectOutputSlot {
                    name: "Layer".into(),
                    id: output,
                    ty: layer_ty,
                },
            )]),
        };
        let preset = FilterPreset {
            metadata: FilterPresetMetadata {
                name: "[Unnamed Filter]".into(),
            },
            effect: effect
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
            log::error!("No writable asset bundle available for a new filter preset");
            return Task::none();
        };
        let path = format!("unnamed_filter_{}.lfp", Uuid::new_v4());
        let id = match services.assets().add_asset(bundle, path, Arc::new(preset)) {
            Ok(id) => id,
            Err(err) => {
                log::error!("Failed to add new filter preset asset: {err}");
                return Task::none();
            }
        };
        let Some(handle) = services.assets().handle(id).ok() else {
            log::error!("Failed to obtain handle for new filter preset");
            return Task::none();
        };
        let index = self.filters.len();
        self.filters.push(handle);
        let task = self.select_filter(index, services);
        self.dirty = true;
        task
    }

    fn save(&mut self, services: &mut Services) -> Task<FilterEditorMessage> {
        let Some(selected) = self.selected.as_mut() else {
            return Task::none();
        };
        if self.validation_error.is_some() {
            return Task::none();
        }
        let preset = match selected.instance.as_asset(services.assets()) {
            Ok(preset) => preset,
            Err(err) => {
                self.validation_error = Some(format!("Failed to serialize filter: {err}"));
                return Task::none();
            }
        };
        if let Err(err) = selected.handle.update(preset) {
            self.validation_error = Some(format!("Failed to update filter preset: {err}"));
            return Task::none();
        }
        if let Err(err) = selected.handle.write() {
            self.validation_error = Some(format!("Failed to write filter preset: {err}"));
            return Task::none();
        }
        self.dirty = false;
        self.validation_error = None;
        Task::none()
    }

    fn revalidate(&mut self) {
        let Some(selected) = self.selected.as_ref() else {
            self.validation_error = None;
            return;
        };
        let effect = selected.instance.effect();
        let layer_inputs = effect
            .inputs
            .values()
            .filter(|slot| slot.ty.is::<LayerType>())
            .count();
        let layer_outputs = effect
            .outputs
            .values()
            .filter(|slot| slot.ty.is::<LayerType>())
            .count();
        self.validation_error = match (layer_inputs, layer_outputs) {
            (1, 1) => None,
            (inputs, outputs) => Some(format!(
                "A filter needs exactly one layer input and one layer output (currently {inputs} input(s), {outputs} output(s))."
            )),
        };
    }
}
