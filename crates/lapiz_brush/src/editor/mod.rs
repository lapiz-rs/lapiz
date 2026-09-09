use std::sync::Arc;

use iced_core::{Element, Length, keyboard, window};
use iced_futures::Subscription;
use iced_runtime::Task;
use iced_widget::{column, row, text};
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
        Graph, GraphResources,
        external::{ExternalVariable, ExternalVariableId},
        function::{
            ASSET_GRAPH_FUNCTION_STORAGE, GRAPH_FUNCTION_NODE_REGISTRY,
            GRAPH_FUNCTION_TYPE_REGISTRY, GraphFunction, GraphFunctionId, GraphFunctionStorage,
        },
        slot::ErasedGraphLiteralUpdateMessage,
        texture::ASSET_GRAPH_TEXTURE_STORAGE,
        variable::GraphLiteral,
    },
    save::{SerializableGraph, SerializableGraphFunction},
};
use lapiz_widgets::{
    button::Button, combo_box::ComboBox, fluent_builder::When, label::Label, panel::Panel,
    scrollable::Scrollable, text_input::TextInput,
};
use uuid::Uuid;

use crate::{
    asset::{BrushPreset, BrushPresetMetadata},
    instance::{BRUSH_GRAPH_TYPES, BrushPresetInstance, GraphFunctionInstance},
    tool::BrushServicesExt,
    widget::{BrushFunctionListDelegate, BrushPresetListDelegate},
};

pub struct BrushEditor {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    brushes: BrushPresetListDelegate,
    functions: BrushFunctionListDelegate,
    selected: Option<Selected>,
    name_buffer: String,
    new_external_name: String,
    new_external_type: Option<&'static str>,
    dirty: bool,
    graph_editor_state: GraphEditorState,
}

#[derive(Debug, Clone, Copy)]
pub enum BrushPresetGraph {
    RequiredSpacing,
    Main,
    StrokePostprocess { index: usize },
}

pub struct SelectedBrush {
    pub handle: AssetHandle<BrushPreset>,
    pub instance: BrushPresetInstance,
    pub viewing_graph: BrushPresetGraph,
}

pub struct SelectedFunction {
    pub handle: AssetHandle<SerializableGraphFunction>,
    pub id: GraphFunctionId,
    pub instance: GraphFunctionInstance,
}

// TODO In the future function editor will be split into another editor.
#[allow(clippy::large_enum_variant)]
pub enum Selected {
    Brush(SelectedBrush),
    Function(SelectedFunction),
}

#[derive(Clone)]
pub enum BrushEditorMessage {
    SelectBrush(usize),
    SelectFunction(usize),
    NewBrush,
    NewFunction,
    NameChanged(String),
    Save,
    Graph(GraphEditorMessage),
    SwitchGraph(BrushPresetGraph),
    NewStrokePostprocess,
    RemoveStrokePostprocess(usize),
    MoveStrokePostprocess { index: usize, up: bool },
    ExternalNameChanged(String),
    ExternalTypeChanged(&'static str),
    CreateExternalVariable,
    RenameExternalVariable(ExternalVariableId, String),
    UpdateExternalVariable(ExternalVariableId, ErasedGraphLiteralUpdateMessage),
    RemoveExternalVariable(ExternalVariableId),
}

impl WindowView for BrushEditor {
    type Message = BrushEditorMessage;

    fn id() -> WindowViewId {
        WindowViewId::new("brush_editor")
    }

    fn boot(services: &mut Services) -> (Self, Task<Self::Message>) {
        let brushes = BrushPresetListDelegate::new(
            services
                .assets()
                .all_handles_of::<BrushPreset>()
                .expect("Failed to list brush presets"),
        );
        let functions_storage = ASSET_GRAPH_FUNCTION_STORAGE.load();
        let functions = BrushFunctionListDelegate::new(functions_storage.all().values());
        let (main_window, open) = iced_runtime::window::open(window::Settings::default());
        (
            Self {
                windows: [main_window].into(),
                main_window,
                brushes,
                functions,
                selected: None,
                name_buffer: String::new(),
                new_external_name: String::new(),
                new_external_type: None,
                dirty: false,
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
        let brush_buttons = self
            .brushes
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let name = item.brush.get().unwrap().metadata.name.clone();
                Button::new(Label::new(name))
                    .width(Length::Fill)
                    .activated(item.selected)
                    .on_press(BrushEditorMessage::SelectBrush(index))
                    .into()
            });
        let function_buttons = self
            .functions
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                Button::new(Label::new(item.name.clone()))
                    .width(Length::Fill)
                    .activated(item.selected)
                    .on_press(BrushEditorMessage::SelectFunction(index))
                    .into()
            });
        let sidebar = Panel::new(
            column![
                row![
                    Button::new(Label::new(t!("new_brush"))).on_press(BrushEditorMessage::NewBrush),
                    Button::new(Label::new(t!("new_function")))
                        .on_press(BrushEditorMessage::NewFunction),
                ]
                .spacing(4),
                Scrollable::new(column![
                    Label::new(t!("brushes")).strong(),
                    column(brush_buttons).spacing(2),
                    Label::new(t!("functions")).strong(),
                    column(function_buttons).spacing(2),
                ])
            ]
            .spacing(6),
        )
        .padding(8)
        .width(220);

        let Some(selected) = self.selected.as_ref() else {
            return row![
                sidebar,
                Panel::new(Label::new(t!("select_a_brush_or_function"))).padding(12),
            ];
        };

        let title = row![
            TextInput::new("Name", &self.name_buffer)
                .on_input(BrushEditorMessage::NameChanged)
                .width(Length::Fill),
            Button::new(Label::new(if self.dirty {
                t!("save_dirty")
            } else {
                t!("save")
            }))
            .primary()
            .on_press(BrushEditorMessage::Save),
        ]
        .spacing(6);

        let content = match selected {
            Selected::Brush(brush) => self.view_brush(brush),
            Selected::Function(function) => column![
                title,
                Element::from(GraphEditor::new(
                    &function.instance.graph_function().graph,
                    &self.graph_editor_state
                ))
                .map(BrushEditorMessage::Graph),
            ]
            .spacing(6)
            .into(),
        };

        row![sidebar, Panel::new(content).padding(8).width(Length::Fill)]
    }

    fn update(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            BrushEditorMessage::SelectBrush(index) => self.select_brush(index, services),
            BrushEditorMessage::SelectFunction(index) => self.select_function(index, services),
            BrushEditorMessage::NewBrush => self.new_brush(services),
            BrushEditorMessage::NewFunction => self.new_function(services),
            BrushEditorMessage::NameChanged(name) => {
                self.name_buffer = name.clone();
                if let Some(selected) = self.selected.as_mut() {
                    match selected {
                        Selected::Brush(brush) => brush.instance.metadata_mut().name = name,
                        Selected::Function(function) => {
                            function.instance.graph_function_mut().name = name
                        }
                    }
                    self.dirty = true;
                }
            }
            BrushEditorMessage::Save => self.save(services),
            BrushEditorMessage::Graph(message) => {
                self.update_graph(message);
                self.dirty = true;
            }
            BrushEditorMessage::SwitchGraph(graph) => {
                if let Some(Selected::Brush(brush)) = self.selected.as_mut() {
                    brush.viewing_graph = graph;
                    self.graph_editor_state = GraphEditorState::default();
                }
            }
            BrushEditorMessage::NewStrokePostprocess => {
                if let Some(Selected::Brush(brush)) = self.selected.as_mut() {
                    let index = brush.instance.new_stroke_postprocess_graph();
                    brush.viewing_graph = BrushPresetGraph::StrokePostprocess { index };
                    self.dirty = true;
                    self.graph_editor_state = GraphEditorState::default();
                }
            }
            BrushEditorMessage::RemoveStrokePostprocess(index) => {
                if let Some(Selected::Brush(brush)) = self.selected.as_mut() {
                    brush.instance.remove_stroke_postprocess_graph(index);
                    brush.viewing_graph = BrushPresetGraph::Main;
                    self.dirty = true;
                    self.graph_editor_state = GraphEditorState::default();
                }
            }
            BrushEditorMessage::MoveStrokePostprocess { index, up } => {
                if let Some(Selected::Brush(brush)) = self.selected.as_mut() {
                    let graphs = brush.instance.stroke_postprocess_graphs_mut();
                    let destination = if up { index - 1 } else { index + 1 };
                    graphs.swap(index, destination);
                    brush.viewing_graph =
                        BrushPresetGraph::StrokePostprocess { index: destination };
                    self.dirty = true;
                    self.graph_editor_state = GraphEditorState::default();
                }
            }
            BrushEditorMessage::ExternalNameChanged(name) => self.new_external_name = name,
            BrushEditorMessage::ExternalTypeChanged(ty) => self.new_external_type = Some(ty),
            BrushEditorMessage::CreateExternalVariable => self.create_external_variable(),
            BrushEditorMessage::RenameExternalVariable(id, name) => {
                if let Some(Selected::Brush(brush)) = self.selected.as_mut() {
                    brush.instance.rename_external_var(&id, name);
                    self.dirty = true;
                }
            }
            BrushEditorMessage::UpdateExternalVariable(id, message) => {
                if let Some(Selected::Brush(brush)) = self.selected.as_mut() {
                    brush.instance.update_external_var(&id, message);
                    self.dirty = true;
                }
            }
            BrushEditorMessage::RemoveExternalVariable(id) => {
                if let Some(Selected::Brush(brush)) = self.selected.as_mut() {
                    brush.instance.remove_external_var(&id);
                    self.dirty = true;
                }
            }
        }
        Task::none()
    }

    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        let main_window = self.main_window;
        iced_futures::subscription::filter_map(("brush_editor", main_window), move |event| {
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
                    Some(BrushEditorMessage::Save)
                }
                iced_futures::subscription::Event::Interaction {
                    window,
                    event: iced_core::Event::Window(iced_core::window::Event::Unfocused),
                    status: _,
                } if window == main_window => Some(BrushEditorMessage::Save),
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

impl BrushEditor {
    fn view_brush<'a>(
        &'a self,
        brush: &'a SelectedBrush,
    ) -> Element<'a, BrushEditorMessage, GraphTheme, GraphRenderer> {
        let title = row![
            TextInput::new("Name", &self.name_buffer)
                .on_input(BrushEditorMessage::NameChanged)
                .width(Length::Fill),
            Button::new(Label::new(if self.dirty {
                t!("save_dirty")
            } else {
                t!("save")
            }))
            .primary()
            .on_press(BrushEditorMessage::Save),
        ]
        .spacing(6);

        let graph: Element<'_, GraphEditorMessage, GraphTheme, GraphRenderer> =
            match brush.viewing_graph {
                BrushPresetGraph::RequiredSpacing => GraphEditor::new(
                    brush.instance.required_spacing_graph(),
                    &self.graph_editor_state,
                )
                .into(),
                BrushPresetGraph::Main => {
                    GraphEditor::new(brush.instance.main_graph(), &self.graph_editor_state).into()
                }
                BrushPresetGraph::StrokePostprocess { index } => GraphEditor::new(
                    brush.instance.stroke_postprocess_graph(index).unwrap(),
                    &self.graph_editor_state,
                )
                .into(),
            };

        let mut graph_list = column![
            Button::new(Label::new(t!("required_spacing")))
                .width(Length::Fill)
                .on_press(BrushEditorMessage::SwitchGraph(
                    BrushPresetGraph::RequiredSpacing
                )),
            Button::new(Label::new(t!("main")))
                .width(Length::Fill)
                .on_press(BrushEditorMessage::SwitchGraph(BrushPresetGraph::Main)),
            Button::new(Label::new(t!("new_postprocess")))
                .on_press(BrushEditorMessage::NewStrokePostprocess),
        ]
        .spacing(3);
        let graph_count = brush.instance.stroke_postprocess_graphs().len();
        for index in 0..graph_count {
            let controls = row![
                Button::new(Label::new(t!("postprocess", index = index)))
                    .width(Length::Fill)
                    .on_press(BrushEditorMessage::SwitchGraph(
                        BrushPresetGraph::StrokePostprocess { index },
                    )),
                Button::new(Label::new(t!("delete")))
                    .danger()
                    .on_press(BrushEditorMessage::RemoveStrokePostprocess(index)),
            ]
            .spacing(2)
            .when(index > 0, |controls| {
                controls.push(
                    Button::new(Label::new("↑"))
                        .on_press(BrushEditorMessage::MoveStrokePostprocess { index, up: true }),
                )
            })
            .when(index + 1 < graph_count, |controls| {
                controls.push(
                    Button::new(Label::new("↓"))
                        .on_press(BrushEditorMessage::MoveStrokePostprocess { index, up: false }),
                )
            });
            graph_list = graph_list.push(controls);
        }

        let variable_rows = brush
            .instance
            .iter_external_vars()
            .map(|(id, variable)| {
                column![
                    row![
                        TextInput::new("Variable name", &variable.name)
                            .on_input(move |name| {
                                BrushEditorMessage::RenameExternalVariable(id, name)
                            })
                            .width(Length::Fill),
                        Button::new(Label::new(t!("delete")))
                            .danger()
                            .on_press(BrushEditorMessage::RemoveExternalVariable(id)),
                    ]
                    .spacing(3),
                    variable
                        .value
                        .ty()
                        .view_literal((*id).into(), variable.value.value())
                        .map(move |message| {
                            BrushEditorMessage::UpdateExternalVariable(id, message)
                        }),
                ]
                .spacing(3)
                .into()
            })
            .collect::<Vec<_>>();
        let types = BRUSH_GRAPH_TYPES
            .all_types()
            .keys()
            .copied()
            .collect::<Vec<_>>();
        let variables = column![
            text(t!("variables")),
            Scrollable::new(column(variable_rows).spacing(6)).height(Length::Fill),
            TextInput::new(&t!("new_variable_name"), &self.new_external_name)
                .on_input(BrushEditorMessage::ExternalNameChanged),
            ComboBox::new(
                types,
                self.new_external_type,
                BrushEditorMessage::ExternalTypeChanged,
            ),
            Button::new(Label::new(t!("add_variable"))).on_press_maybe(
                (!self.new_external_name.is_empty() && self.new_external_type.is_some())
                    .then_some(BrushEditorMessage::CreateExternalVariable),
            ),
        ]
        .spacing(5)
        .width(260);

        column![
            title,
            row![
                Panel::new(graph.map(BrushEditorMessage::Graph))
                    .transparent()
                    .width(Length::Fill)
                    .height(Length::Fill),
                Panel::new(graph_list).padding(4).width(220),
                Panel::new(variables).padding(4),
            ]
            .height(Length::Fill),
        ]
        .spacing(6)
        .into()
    }

    fn select_brush(&mut self, index: usize, services: &mut Services) {
        let handle = self
            .brushes
            .get(index)
            .expect("Selected brush should exist")
            .brush
            .clone();
        let (instance, errors) = BrushPresetInstance::from_asset(
            &handle,
            ASSET_GRAPH_TEXTURE_STORAGE.clone(),
            ASSET_GRAPH_FUNCTION_STORAGE.clone(),
        );
        for error in errors {
            log::error!("Failed to load brush preset: {error}");
        }
        let Some(instance) = instance else {
            return;
        };
        self.brushes.select(index);
        self.name_buffer = instance.metadata().name.clone();
        self.selected = Some(Selected::Brush(SelectedBrush {
            handle: handle.clone(),
            instance,
            viewing_graph: BrushPresetGraph::Main,
        }));
        self.dirty = false;
        self.graph_editor_state = GraphEditorState::default();
        services.set_current_brush_preset(handle);
    }

    fn select_function(&mut self, index: usize, services: &Services) {
        let item = self.functions.get(index).unwrap();
        let handle = services
            .assets()
            .all_handles_of::<SerializableGraphFunction>()
            .unwrap()
            .into_iter()
            .find(|handle| handle.get().unwrap().id == item.id)
            .unwrap();
        let serialized = handle.get().unwrap();
        let (function, errors) = serialized.deserialize_func(
            ASSET_GRAPH_TEXTURE_STORAGE.clone(),
            ASSET_GRAPH_FUNCTION_STORAGE.clone(),
            Some(handle.id()),
        );
        for error in errors {
            log::error!("Failed to load graph function: {error}");
        }
        let Some(function) = function else {
            return;
        };
        self.functions.select(index);
        self.name_buffer = function.name.clone();
        self.selected = Some(Selected::Function(SelectedFunction {
            handle,
            id: function.id,
            instance: GraphFunctionInstance::new(function),
        }));
        self.graph_editor_state = GraphEditorState::default();
        self.dirty = false;
    }

    fn new_brush(&mut self, services: &mut Services) {
        let preset = BrushPreset {
            metadata: BrushPresetMetadata {
                name: "[Unnamed Brush]".into(),
            },
            required_spacing_graph: SerializableGraph::default(),
            main_graph: SerializableGraph::default(),
            stroke_postprocess_graphs: Vec::new(),
            external_vars: Vec::new(),
        };
        let bundle = services
            .assets()
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .unwrap()
            .metadata()
            .bundle_id;
        let id = services
            .assets()
            .add_asset(bundle, "unnamed_brush.lapiz", Arc::new(preset))
            .unwrap();
        let handle = services.assets().handle(id).unwrap();
        let index = self.brushes.push(handle);
        self.select_brush(index, services);
        self.dirty = true;
    }

    fn new_function(&mut self, services: &Services) {
        let id = GraphFunctionId::new(Uuid::new_v4());
        let function = GraphFunction {
            asset_id: None,
            id,
            name: "[Unnamed Function]".into(),
            graph: Graph::new(GraphResources {
                type_registry: GRAPH_FUNCTION_TYPE_REGISTRY.clone(),
                node_registry: GRAPH_FUNCTION_NODE_REGISTRY.clone(),
                textures: ASSET_GRAPH_TEXTURE_STORAGE.clone(),
                functions: ASSET_GRAPH_FUNCTION_STORAGE.clone(),
                ..Default::default()
            }),
        };
        let serialized = SerializableGraphFunction::serialize_func(&function).unwrap();
        let bundle = services
            .assets()
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .unwrap()
            .metadata()
            .bundle_id;
        let asset_id = services
            .assets()
            .add_asset(bundle, "unnamed_function.lsf", Arc::new(serialized))
            .unwrap();
        let handle = services.assets().handle(asset_id).unwrap();
        self.name_buffer = function.name.clone();
        self.selected = Some(Selected::Function(SelectedFunction {
            handle,
            id,
            instance: GraphFunctionInstance::new(function),
        }));
        self.graph_editor_state = GraphEditorState::default();
        self.dirty = true;
    }

    fn save(&mut self, services: &mut Services) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        match selected {
            Selected::Brush(brush) => {
                let preset = brush.instance.as_asset().unwrap();
                brush.handle.update(preset).unwrap();
                brush.handle.write().unwrap();
                services.set_current_brush_preset(brush.handle.clone());
            }
            Selected::Function(function) => {
                let serialized =
                    SerializableGraphFunction::serialize_func(function.instance.graph_function())
                        .unwrap();
                function.handle.update(serialized).unwrap();
                function.handle.write().unwrap();
                ASSET_GRAPH_FUNCTION_STORAGE.store(
                    GraphFunctionStorage::new(
                        ASSET_GRAPH_TEXTURE_STORAGE.clone(),
                        ASSET_GRAPH_FUNCTION_STORAGE.clone(),
                        services
                            .assets()
                            .all_handles_of::<SerializableGraphFunction>()
                            .unwrap(),
                    )
                    .into(),
                );
            }
        }
        self.dirty = false;
    }

    fn update_graph(&mut self, message: GraphEditorMessage) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };
        match selected {
            Selected::Brush(brush) => match brush.viewing_graph {
                BrushPresetGraph::RequiredSpacing => {
                    self.graph_editor_state
                        .update(brush.instance.required_spacing_graph_mut(), message);
                }
                BrushPresetGraph::Main => {
                    self.graph_editor_state
                        .update(brush.instance.main_graph_mut(), message);
                }
                BrushPresetGraph::StrokePostprocess { index } => {
                    self.graph_editor_state.update(
                        brush
                            .instance
                            .stroke_postprocess_graphs_mut()
                            .get_mut(index)
                            .unwrap(),
                        message,
                    );
                }
            },
            Selected::Function(function) => {
                self.graph_editor_state
                    .update(&mut function.instance.graph_function_mut().graph, message);
            }
        }
    }

    fn create_external_variable(&mut self) {
        let Some(Selected::Brush(brush)) = self.selected.as_mut() else {
            return;
        };
        let ty = BRUSH_GRAPH_TYPES
            .get_type(
                self.new_external_type
                    .expect("New external variable type should be selected"),
            )
            .expect("Selected external variable type should exist");
        brush.instance.insert_external_var(ExternalVariable {
            id: ExternalVariableId::new(Uuid::new_v4()),
            name: std::mem::take(&mut self.new_external_name),
            value: GraphLiteral::new_boxed(ty.default_literal(), dyn_clone::clone_box(ty)),
        });
        self.dirty = true;
    }
}
