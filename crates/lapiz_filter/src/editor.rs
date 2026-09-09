use std::{collections::BTreeSet, sync::Arc};

use iced_core::{Element, Length, keyboard, window};
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_widget::{column, container, row, text};
use lapiz_assets::{AssetAppExt, asset::AssetHandle};
use lapiz_i18n::t;
use lapiz_runtime::{
    Services,
    windows::{WindowView, WindowViewId},
};
use lapiz_shader_graph::{
    GraphRenderer, GraphTheme,
    editor::{GraphEditor, GraphEditorMessage, GraphEditorState},
    graph::{
        external::{ExternalVariable, ExternalVariableId},
        slot::ErasedGraphLiteralUpdateMessage,
        variable::GraphLiteral,
    },
};
use lapiz_widgets::{
    button::Button, combo_box::ComboBox, fluent_builder::When, label::Label, panel::Panel,
    scrollable::Scrollable, text_input::TextInput,
};
use uuid::Uuid;

use crate::{
    asset::{
        FilterGroupId, FilterPreset, FilterPresetMetadata, FilterSlotRef, SerializableFilterGroup,
    },
    instance::{FilterGroup, FilterInstance},
    render::graph::FILTER_GRAPH_TYPES,
};

const LAYER_OPTION: &str = "Layer";

pub struct FilterEditor {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    filters: FilterPresetListDelegate,
    selected: Option<SelectedFilter>,
    filter_name_buffer: String,
    group_name_buffer: String,
    new_external_name: String,
    new_external_type: Option<&'static str>,
    dirty: bool,
    validation_error: Option<String>,
    graph_editor_state: GraphEditorState,
}

pub struct SelectedFilter {
    pub handle: AssetHandle<FilterPreset>,
    pub instance: FilterInstance,
    pub viewing_group: usize,
}

#[derive(Clone)]
pub enum FilterEditorMessage {
    SelectFilter(usize),
    NewFilter,
    FilterNameChanged(String),
    GroupNameChanged(String),
    Save,
    Graph(GraphEditorMessage),
    SwitchGroup(usize),
    AddGroup,
    RemoveGroup(usize),
    MoveGroup { index: usize, up: bool },
    GroupInputChanged(String),
    GroupOutputChanged(String),
    ExternalNameChanged(String),
    ExternalTypeChanged(&'static str),
    CreateExternalVariable,
    RenameExternalVariable(ExternalVariableId, String),
    UpdateExternalVariable(ExternalVariableId, ErasedGraphLiteralUpdateMessage),
    RemoveExternalVariable(ExternalVariableId),
}

impl WindowView for FilterEditor {
    type Message = FilterEditorMessage;

    fn id() -> WindowViewId {
        WindowViewId::new("filter_editor")
    }

    fn boot(services: &mut Services) -> (Self, Task<Self::Message>) {
        let filters = FilterPresetListDelegate::new(
            services
                .assets()
                .all_handles_of::<FilterPreset>()
                .expect("Failed to list filter presets"),
        );
        let (main_window, open) = iced_runtime::window::open(window::Settings::default());
        (
            Self {
                windows: [main_window].into(),
                main_window,
                filters,
                selected: None,
                filter_name_buffer: String::new(),
                group_name_buffer: String::new(),
                new_external_name: String::new(),
                new_external_type: None,
                dirty: false,
                validation_error: None,
                graph_editor_state: GraphEditorState::default(),
            },
            open.discard(),
        )
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        _: &'a Services,
    ) -> impl Into<Element<'a, Self::Message, GraphTheme, GraphRenderer>> {
        let filter_buttons = self
            .filters
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let name = item
                    .filter
                    .get()
                    .map(|preset| preset.metadata.name.clone())
                    .unwrap_or_else(|_| "<loading>".to_string());
                Button::new(Label::new(name))
                    .width(Length::Fill)
                    .activated(item.selected)
                    .on_press(FilterEditorMessage::SelectFilter(index))
                    .into()
            });
        let sidebar = Panel::new(
            column![
                row![
                    Button::new(Label::new(t!("new_filter")))
                        .on_press(FilterEditorMessage::NewFilter)
                ]
                .spacing(4),
                Label::new(t!("filters")).strong(),
                column(filter_buttons).spacing(2),
            ]
            .spacing(6),
        )
        .padding(8)
        .width(220);

        let Some(selected) = self.selected.as_ref() else {
            return row![
                sidebar,
                Panel::new(Label::new(t!("select_a_filter"))).padding(12)
            ];
        };

        let title = row![
            TextInput::new("Filter name", &self.filter_name_buffer)
                .on_input(FilterEditorMessage::FilterNameChanged)
                .width(Length::Fill),
            Button::new(Label::new(if self.dirty { "Save *" } else { "Save" }))
                .primary()
                .on_press(FilterEditorMessage::Save),
        ]
        .spacing(6);

        let group_name = row![
            TextInput::new("Group name", &self.group_name_buffer)
                .on_input(FilterEditorMessage::GroupNameChanged)
                .width(Length::Fill),
        ]
        .spacing(6);

        let content = column![
            title,
            group_name,
            row![
                Panel::new(
                    Element::from(GraphEditor::new(
                        &selected.instance.groups()[selected.viewing_group].graph,
                        &self.graph_editor_state,
                    ))
                    .map(FilterEditorMessage::Graph)
                )
                .width(Length::Fill)
                .height(Length::Fill),
                Panel::new(self.view_group_controls(selected))
                    .padding(4)
                    .width(220),
                Panel::new(self.view_variables(selected)).padding(4),
            ]
            .height(Length::Fill),
        ]
        .spacing(6);

        row![sidebar, Panel::new(content).padding(8).width(Length::Fill)]
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
            }
            FilterEditorMessage::GroupNameChanged(name) => {
                self.group_name_buffer = name.clone();
                if let Some(selected) = self.selected.as_mut() {
                    selected.instance.groups_mut()[selected.viewing_group].name = name;
                    self.dirty = true;
                }
            }
            FilterEditorMessage::Save => self.save(services),
            FilterEditorMessage::Graph(message) => {
                self.update_graph(message);
                self.dirty = true;
                self.revalidate();
            }
            FilterEditorMessage::SwitchGroup(index) => {
                if let Some(selected) = self.selected.as_mut()
                    && index < selected.instance.groups().len()
                {
                    selected.viewing_group = index;
                    self.graph_editor_state = GraphEditorState::default();
                    self.group_name_buffer = selected.instance.groups()[index].name.clone();
                }
            }
            FilterEditorMessage::AddGroup => {
                if let Some(selected) = self.selected.as_mut() {
                    let index = selected.instance.new_group();
                    selected.viewing_group = index;
                    self.group_name_buffer = selected.instance.groups()[index].name.clone();
                    self.dirty = true;
                    self.graph_editor_state = GraphEditorState::default();
                    self.revalidate();
                }
            }
            FilterEditorMessage::RemoveGroup(index) => {
                if let Some(selected) = self.selected.as_mut() {
                    // at least one group
                    if selected.instance.groups().len() <= 1 {
                        return Task::none();
                    }
                    if index < selected.instance.groups().len() {
                        selected.instance.remove_group(index);
                        if selected.viewing_group > index {
                            selected.viewing_group -= 1;
                        }
                        if selected.viewing_group >= selected.instance.groups().len() {
                            selected.viewing_group = selected.instance.groups().len() - 1;
                        }
                        self.group_name_buffer = selected.instance.groups()[selected.viewing_group]
                            .name
                            .clone();
                        self.dirty = true;
                        self.graph_editor_state = GraphEditorState::default();
                        self.revalidate();
                    }
                }
            }
            FilterEditorMessage::MoveGroup { index, up } => {
                if let Some(selected) = self.selected.as_mut() {
                    let len = selected.instance.groups().len();
                    if index >= len || (up && index == 0) || (!up && index + 1 >= len) {
                        return Task::none();
                    }
                    let groups = selected.instance.groups_mut();
                    let destination = if up { index - 1 } else { index + 1 };
                    groups.swap(index, destination);
                    selected.viewing_group = destination;
                    self.dirty = true;
                    self.graph_editor_state = GraphEditorState::default();
                    self.revalidate();
                }
            }
            FilterEditorMessage::GroupInputChanged(value) => {
                if let Some(selected) = self.selected.as_mut()
                    && let Some(slot) = slot_from_pick_value(&value, selected)
                {
                    selected.instance.groups_mut()[selected.viewing_group].input = slot;
                    self.dirty = true;
                    self.revalidate();
                }
            }
            FilterEditorMessage::GroupOutputChanged(value) => {
                if let Some(selected) = self.selected.as_mut()
                    && let Some(slot) = slot_from_pick_value(&value, selected)
                {
                    selected.instance.groups_mut()[selected.viewing_group].output = slot;
                    self.dirty = true;
                    self.revalidate();
                }
            }
            FilterEditorMessage::ExternalNameChanged(name) => self.new_external_name = name,
            FilterEditorMessage::ExternalTypeChanged(ty) => self.new_external_type = Some(ty),
            FilterEditorMessage::CreateExternalVariable => self.create_external_variable(),
            FilterEditorMessage::RenameExternalVariable(id, name) => {
                if let Some(selected) = self.selected.as_mut() {
                    selected.instance.rename_external_var(&id, name);
                    self.dirty = true;
                }
            }
            FilterEditorMessage::UpdateExternalVariable(id, message) => {
                if let Some(selected) = self.selected.as_mut() {
                    selected.instance.update_external_var(&id, message);
                    self.dirty = true;
                }
            }
            FilterEditorMessage::RemoveExternalVariable(id) => {
                if let Some(selected) = self.selected.as_mut() {
                    selected.instance.remove_external_var(&id);
                    self.dirty = true;
                }
            }
        }
        Task::none()
    }

    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        let main_window = self.main_window;
        iced_futures::subscription::filter_map(("filter_editor", main_window), move |event| {
            match event {
                iced_futures::subscription::Event::Interaction {
                    window,
                    event:
                        iced_core::Event::Keyboard(keyboard::Event::KeyPressed {
                            key, modifiers, ..
                        }),
                    status: _,
                } if window == main_window
                    && modifiers.control()
                    && matches!(
                        &key,
                        keyboard::Key::Character(character)
                            if character.eq_ignore_ascii_case("s")
                    ) =>
                {
                    Some(FilterEditorMessage::Save)
                }
                iced_futures::subscription::Event::Interaction {
                    window,
                    event: iced_core::Event::Window(iced_core::window::Event::Unfocused),
                    status: _,
                } if window == main_window => Some(FilterEditorMessage::Save),
                _ => None,
            }
        })
    }

    fn close(self, _: &mut Services) -> Task<()> {
        iced_runtime::window::close(self.main_window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.main_window)
    }
}

impl FilterEditor {
    fn view_group_controls<'a>(
        &'a self,
        selected: &'a SelectedFilter,
    ) -> Element<'a, FilterEditorMessage, GraphTheme, GraphRenderer> {
        let group_count = selected.instance.groups().len();
        let mut graph_list = column![
            Button::new(Label::new(t!("add_group"))).on_press(FilterEditorMessage::AddGroup),
        ]
        .spacing(3);

        for index in 0..group_count {
            let group = &selected.instance.groups()[index];
            let is_current = index == selected.viewing_group;
            let controls = row![
                Button::new(Label::new(if is_current {
                    format!("{}  <", group.name)
                } else {
                    group.name.clone()
                }))
                .width(Length::Fill)
                .activated(is_current)
                .on_press(FilterEditorMessage::SwitchGroup(index)),
                Button::new(Label::new(t!("delete")))
                    .danger()
                    .on_press_maybe(
                        (group_count > 1).then_some(FilterEditorMessage::RemoveGroup(index)),
                    ),
            ]
            .spacing(2)
            .when(index > 0, |controls| {
                controls.push(
                    Button::new(Label::new("↑"))
                        .on_press(FilterEditorMessage::MoveGroup { index, up: true }),
                )
            })
            .when(index + 1 < group_count, |controls| {
                controls.push(
                    Button::new(Label::new("↓"))
                        .on_press(FilterEditorMessage::MoveGroup { index, up: false }),
                )
            });
            graph_list = graph_list.push(controls);
        }

        let current = &selected.instance.groups()[selected.viewing_group];

        let mut options = vec![LAYER_OPTION.to_string()];
        for group in selected.instance.groups() {
            if group.id != current.id {
                options.push(group.name.clone());
            }
        }

        let slot_options = std::iter::once(LAYER_OPTION.to_string())
            .chain(
                selected
                    .instance
                    .groups()
                    .iter()
                    .filter(|g| g.id != current.id)
                    .map(|g| g.name.clone()),
            )
            .collect::<Vec<_>>();

        let input_value = slot_to_pick_value(&current.input, selected);
        let output_value = slot_to_pick_value(&current.output, selected);

        let error = match self.validation_error.as_ref() {
            Some(err) => {
                container(text(err.clone()).color(iced_core::Color::from_rgb(1.0, 0.3, 0.3)))
                    .padding(4)
            }
            None => container(text("")),
        };

        column![
            text(t!("shader_groups")),
            Scrollable::new(graph_list).height(Length::Fill),
            container(row![
                column![
                    text(t!("input_source")),
                    ComboBox::new(
                        slot_options.clone(),
                        input_value,
                        FilterEditorMessage::GroupInputChanged,
                    ),
                ]
                .spacing(2),
                column![
                    text(t!("output_target")),
                    ComboBox::new(
                        slot_options,
                        output_value,
                        FilterEditorMessage::GroupOutputChanged,
                    ),
                ]
                .spacing(2),
            ])
            .padding(4),
            error,
        ]
        .spacing(5)
        .into()
    }

    fn view_variables<'a>(
        &'a self,
        selected: &'a SelectedFilter,
    ) -> Element<'a, FilterEditorMessage, GraphTheme, GraphRenderer> {
        let variable_rows = selected
            .instance
            .iter_external_vars()
            .map(|(id, variable)| {
                column![
                    row![
                        TextInput::new("Variable name", &variable.name)
                            .on_input(move |name| {
                                FilterEditorMessage::RenameExternalVariable(id, name)
                            })
                            .width(Length::Fill),
                        Button::new(Label::new(t!("delete")))
                            .danger()
                            .on_press(FilterEditorMessage::RemoveExternalVariable(id)),
                    ]
                    .spacing(3),
                    variable
                        .value
                        .ty()
                        .view_literal((*id).into(), variable.value.value())
                        .map(move |message| {
                            FilterEditorMessage::UpdateExternalVariable(id, message)
                        }),
                ]
                .spacing(3)
                .into()
            })
            .collect::<Vec<_>>();
        let types = FILTER_GRAPH_TYPES
            .all_types()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        let variables = column![
            text(t!("variables")),
            Scrollable::new(column(variable_rows).spacing(6)).height(Length::Fill),
            TextInput::new("New variable name", &self.new_external_name)
                .on_input(FilterEditorMessage::ExternalNameChanged),
            ComboBox::new(
                types,
                self.new_external_type,
                FilterEditorMessage::ExternalTypeChanged,
            ),
            Button::new(Label::new(t!("add_variable"))).on_press_maybe(
                (!self.new_external_name.is_empty() && self.new_external_type.is_some())
                    .then_some(FilterEditorMessage::CreateExternalVariable),
            ),
        ]
        .spacing(5)
        .width(260);

        variables.into()
    }

    fn select_filter(&mut self, index: usize, services: &Services) {
        let handle = self
            .filters
            .get(index)
            .expect("Selected filter should exist")
            .filter
            .clone();
        let (instance, errors) = FilterInstance::from_asset(&handle, services);
        for error in errors {
            log::error!("Failed to load filter preset: {error}");
        }
        let Some(instance) = instance else {
            return;
        };
        self.filters.select(index);
        self.filter_name_buffer = instance.metadata().name.clone();
        self.group_name_buffer = instance.groups()[0].name.clone();
        self.selected = Some(SelectedFilter {
            handle: handle.clone(),
            instance,
            viewing_group: 0,
        });
        self.dirty = false;
        self.validation_error = None;
        self.graph_editor_state = GraphEditorState::default();
    }

    fn new_filter(&mut self, services: &mut Services) {
        let group_id = FilterGroupId::random();
        let preset = FilterPreset {
            metadata: FilterPresetMetadata {
                name: "[Unnamed Filter]".into(),
            },
            groups: vec![SerializableFilterGroup {
                id: group_id,
                name: "Group 1".into(),
                input: FilterSlotRef::Layer,
                output: FilterSlotRef::Layer,
                graph: Default::default(),
            }],
            external_vars: Vec::new(),
        };
        let Some(bundle) = services
            .assets()
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .map(|bundle| bundle.metadata().bundle_id)
        else {
            log::error!("No writable asset bundle available for a new filter preset");
            return;
        };
        let path = format!("unnamed_filter_{}.lfp", Uuid::new_v4());
        let id = match services.assets().add_asset(bundle, path, Arc::new(preset)) {
            Ok(id) => id,
            Err(err) => {
                log::error!("Failed to add new filter preset asset: {err}");
                return;
            }
        };
        let Some(handle) = services.assets().handle(id).ok() else {
            log::error!("Failed to obtain handle for new filter preset");
            return;
        };
        let Some(index) = self.filters.push(handle) else {
            log::error!("New filter preset asset is not available yet");
            return;
        };
        self.select_filter(index, services);
        self.dirty = true;
    }

    fn save(&mut self, _services: &mut Services) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        if self.validation_error.is_some() {
            return;
        }
        let preset = match selected.instance.as_asset() {
            Ok(preset) => preset,
            Err(err) => {
                self.validation_error = Some(format!("Failed to serialize filter: {err}"));
                return;
            }
        };
        if let Err(err) = selected.handle.update(preset) {
            self.validation_error = Some(format!("Failed to update filter preset: {err}"));
            return;
        }
        if let Err(err) = selected.handle.write() {
            self.validation_error = Some(format!("Failed to write filter preset: {err}"));
            return;
        }
        self.dirty = false;
        self.validation_error = None;
    }

    fn update_graph(&mut self, message: GraphEditorMessage) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        self.graph_editor_state.update(
            &mut selected.instance.groups_mut()[selected.viewing_group].graph,
            message,
        );
    }

    fn create_external_variable(&mut self) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        let ty = FILTER_GRAPH_TYPES
            .get_type(
                self.new_external_type
                    .expect("New external variable type should be selected"),
            )
            .expect("Selected external variable type should exist");
        selected.instance.insert_external_var(ExternalVariable {
            id: ExternalVariableId::new(Uuid::new_v4()),
            name: std::mem::take(&mut self.new_external_name),
            value: GraphLiteral::new_boxed(ty.default_literal(), dyn_clone::clone_box(ty)),
        });
        self.dirty = true;
    }

    fn revalidate(&mut self) {
        let Some(selected) = self.selected.as_ref() else {
            self.validation_error = None;
            return;
        };
        self.validation_error = validate_groups(selected.instance.groups()).err();
    }
}

pub fn validate_groups(groups: &[FilterGroup]) -> Result<(), String> {
    if groups.is_empty() {
        return Err("A filter must contain at least one shader group.".to_string());
    }

    // unique group ids
    let mut seen = BTreeSet::new();
    for group in groups {
        if !seen.insert(*group.id) {
            return Err("Duplicate shader group id detected.".to_string());
        }
    }

    let index_of = |id: Uuid| groups.iter().position(|g| *g.id == id);

    // referenced groups must exist and must not reference themselves
    for (idx, group) in groups.iter().enumerate() {
        let bad = |slot: &FilterSlotRef| match slot {
            FilterSlotRef::Group(id) => {
                let Some(ref_idx) = index_of(*id) else {
                    return true;
                };
                ref_idx == idx
            }
            FilterSlotRef::Layer => false,
        };
        if bad(&group.input) || bad(&group.output) {
            return Err(format!(
                "Group '{}' references a missing or self-referencing group.",
                group.name
            ));
        }
    }

    // exactly one group has output == Layer
    let layer_outputs = groups
        .iter()
        .filter(|g| g.output == FilterSlotRef::Layer)
        .count();
    if layer_outputs == 0 {
        return Err("No shader group outputs to the layer; exactly one is required.".to_string());
    }
    if layer_outputs > 1 {
        return Err(
            "More than one shader group outputs to the layer; exactly one is required.".to_string(),
        );
    }

    // if group A outputs to Group(B), then B must input from Group(A)
    for group in groups {
        if let FilterSlotRef::Group(target) = group.output {
            let target_idx = index_of(target).unwrap();
            if groups[target_idx].input != FilterSlotRef::Group(*group.id) {
                return Err(format!(
                    "Group '{}' outputs to '{}' but that group's input is not '{}'.",
                    group.name, groups[target_idx].name, group.name
                ));
            }
        }
    }

    // acyclic
    let mut indegree: Vec<usize> = vec![0; groups.len()];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
    let mut edges_seen = BTreeSet::new();
    for (idx, group) in groups.iter().enumerate() {
        if let FilterSlotRef::Group(target) = group.output {
            let target_idx = index_of(target).unwrap();
            let edge = (idx, target_idx);
            if edges_seen.insert(edge) {
                indegree[target_idx] += 1;
                outgoing[idx].push(target_idx);
            }
        }
    }
    let mut queue: Vec<usize> = groups
        .iter()
        .enumerate()
        .filter(|(i, _)| indegree[*i] == 0)
        .map(|(i, _)| i)
        .collect();
    let mut visited = 0;
    while let Some(idx) = queue.pop() {
        visited += 1;
        for &next in &outgoing[idx] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                queue.push(next);
            }
        }
    }
    if visited != groups.len() {
        return Err("The shader group connections form a cycle.".to_string());
    }

    Ok(())
}

fn slot_to_pick_value(slot: &FilterSlotRef, selected: &SelectedFilter) -> Option<String> {
    match slot {
        FilterSlotRef::Layer => Some(LAYER_OPTION.to_string()),
        FilterSlotRef::Group(id) => selected
            .instance
            .groups()
            .iter()
            .find(|g| *g.id == *id)
            .map(|g| g.name.clone()),
    }
}

fn slot_from_pick_value(value: &str, selected: &SelectedFilter) -> Option<FilterSlotRef> {
    if value == LAYER_OPTION {
        return Some(FilterSlotRef::Layer);
    }
    selected
        .instance
        .groups()
        .iter()
        .find(|g| g.name == value)
        .map(|g| FilterSlotRef::Group(*g.id))
}

pub struct FilterPresetListItem {
    pub filter: AssetHandle<FilterPreset>,
    pub name: String,
    pub selected: bool,
}

impl FilterPresetListItem {
    pub fn new(filter: AssetHandle<FilterPreset>) -> Option<Self> {
        let name = filter.get().ok()?.metadata.name.clone();
        Some(Self {
            filter,
            name,
            selected: false,
        })
    }
}

pub struct FilterPresetListDelegate {
    items: Vec<FilterPresetListItem>,
    selected: Option<usize>,
}

impl FilterPresetListDelegate {
    pub fn new(filters: Vec<AssetHandle<FilterPreset>>) -> Self {
        Self {
            items: filters
                .into_iter()
                .filter_map(FilterPresetListItem::new)
                .collect(),
            selected: None,
        }
    }

    pub fn get(&self, index: usize) -> Option<&FilterPresetListItem> {
        self.items.get(index)
    }

    pub fn items(&self) -> &[FilterPresetListItem] {
        &self.items
    }

    pub fn select(&mut self, index: usize) {
        assert!(index < self.items.len(), "Filter index should exist");
        if let Some(previous) = self.selected {
            self.items[previous].selected = false;
        }
        self.items[index].selected = true;
        self.selected = Some(index);
    }

    pub fn push(&mut self, filter: AssetHandle<FilterPreset>) -> Option<usize> {
        let item = FilterPresetListItem::new(filter)?;
        self.items.push(item);
        Some(self.items.len() - 1)
    }
}
