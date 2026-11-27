use std::sync::Arc;

use anyhow::{Context as _, Result};
use iced_core::{Color, Element, Length, Size, Theme, window};
use iced_futures::Subscription;
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use lapiz_assets::{
    AssetAppExt as _,
    asset::{AssetHandle, AssetId},
    event::AssetEvent,
    loader::AssetSerializer as _,
};
use lapiz_effect::shader_graph::{
    GraphElement,
    editor::{GraphEditor, GraphEditorMessage},
    graph::{
        Graph, GraphResources,
        function::{
            GRAPH_FUNCTION_NODE_REGISTRY, GRAPH_FUNCTION_TYPE_REGISTRY, GraphFunction,
            GraphFunctionId,
        },
    },
    save::{SerializableGraphFunction, SerializableGraphFunctionSerializer},
};
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer,
    global::Globals,
    windows::{WindowView, WindowViewId},
};
use lapiz_widgets::{
    button, column, component::component, flex::Flex, label, panel, row, scrollable, text_input,
    title_bar,
};
use uuid::Uuid;

use crate::SHADER_FUNCTION_EDITOR_VIEW_ID;

const DEFAULT_FUNCTION_NAME: &str = "[Unnamed Function]";

/// Standalone window view for creating, authoring, and saving reusable [`SerializableGraphFunction`] assets.
pub struct ShaderFunctionEditor {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    functions: Vec<AssetHandle<SerializableGraphFunction>>,
    selected: Option<SelectedFunction>,
    name_buffer: String,
    dirty: bool,
    error: Option<String>,
}

struct SelectedFunction {
    handle: AssetHandle<SerializableGraphFunction>,
    function: GraphFunction,
}

/// UI messages handled by [`ShaderFunctionEditor`].
#[derive(Clone)]
pub enum ShaderFunctionEditorMessage {
    SelectFunction(AssetId<SerializableGraphFunction>),
    NewFunction,
    NameChanged(String),
    Graph(GraphEditorMessage),
    Save,
    AssetEvent(AssetEvent),

    Close,
    Maximize,
    Minimize,
    Drag,
}

type EditorElement<'a> = GraphElement<'a, ShaderFunctionEditorMessage>;

impl WindowView for ShaderFunctionEditor {
    type Message = ShaderFunctionEditorMessage;
    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new(SHADER_FUNCTION_EDITOR_VIEW_ID)
    }

    fn boot(
        _params: Option<Self::BootParams>,
        globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)> {
        let functions = globals
            .assets()
            .all_handles_of::<SerializableGraphFunction>()
            .context("Failed to list shader graph functions")?;
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
                functions,
                selected: None,
                name_buffer: String::new(),
                dirty: false,
                error: None,
            },
            open.discard(),
        ))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        _: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let titlebar = title_bar(label(t!("shader_function_editor_title")).window_title())
            .on_close(ShaderFunctionEditorMessage::Close)
            .on_maximize(ShaderFunctionEditorMessage::Maximize)
            .on_minimize(ShaderFunctionEditorMessage::Minimize)
            .on_drag(ShaderFunctionEditorMessage::Drag);

        let selected_id = self.selected.as_ref().map(|selected| selected.handle.id());
        let function_list = self
            .functions
            .iter()
            .map(|handle| {
                let id = handle.id();
                let name = if selected_id == Some(id) {
                    self.name_buffer.clone()
                } else {
                    handle
                        .get()
                        .map(|function| function.name.clone())
                        .unwrap_or_else(|_| "<loading>".to_string())
                };
                button(label(name))
                    .width(Length::Fill)
                    .activated(selected_id == Some(id))
                    .on_press(ShaderFunctionEditorMessage::SelectFunction(id))
                    .into()
            })
            .collect::<Vec<_>>();

        let sidebar = panel(
            column![
                label(t!("shader_functions")).strong(),
                scrollable(Flex::column(function_list).gap(2.0))
                    .width(Length::Fill)
                    .height(Length::Fill),
                button(label(t!("new_shader_function")))
                    .on_press(ShaderFunctionEditorMessage::NewFunction),
            ]
            .gap(6.0),
        )
        .padding(8)
        .width(220);

        let content = match self.selected.as_ref() {
            Some(selected) => self.view_selected(selected),
            None => self.view_empty(),
        };

        panel(column![
            titlebar,
            row![sidebar, content].gap(8.0).height(Length::Fill)
        ])
        .height(Length::Fill)
    }

    fn update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            ShaderFunctionEditorMessage::SelectFunction(id) => {
                if let Err(error) = self.select_function(id, globals) {
                    log::error!("Failed to load shader graph function: {error:#}");
                    self.error = Some(error.to_string());
                }
                Task::none()
            }
            ShaderFunctionEditorMessage::NewFunction => {
                if let Err(error) = self.new_function(globals) {
                    log::error!("Failed to create shader graph function: {error:#}");
                    self.error = Some(error.to_string());
                }
                Task::none()
            }
            ShaderFunctionEditorMessage::NameChanged(name) => {
                self.name_buffer = name.clone();
                if let Some(selected) = self.selected.as_mut() {
                    selected.function.name = name;
                    self.dirty = true;
                }
                Task::none()
            }
            ShaderFunctionEditorMessage::Graph(message) => {
                if let Some(selected) = self.selected.as_mut() {
                    selected.function.graph.update(message);
                    self.dirty = true;
                }
                Task::none()
            }
            ShaderFunctionEditorMessage::Save => {
                self.save();
                Task::none()
            }
            ShaderFunctionEditorMessage::AssetEvent(event) => {
                self.handle_asset_event(event, globals);
                Task::none()
            }
            ShaderFunctionEditorMessage::Close => close(self.main_window),
            ShaderFunctionEditorMessage::Maximize => toggle_maximize(self.main_window),
            ShaderFunctionEditorMessage::Minimize => minimize(self.main_window, true),
            ShaderFunctionEditorMessage::Drag => drag(self.main_window),
        }
    }

    fn subscription(&self, _globals: &Globals) -> Subscription<Self::Message> {
        AssetEvent::listen_for::<SerializableGraphFunction>()
            .map(ShaderFunctionEditorMessage::AssetEvent)
    }

    fn close(self, _: &mut Globals) -> Task<()> {
        close(self.main_window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.main_window)
    }
}

impl ShaderFunctionEditor {
    fn view_empty(&self) -> EditorElement<'_> {
        let placeholder = label(t!("select_a_shader_function")).muted();
        match self.error.as_ref() {
            Some(error) => column![
                placeholder,
                label(error.clone()).color(Color::from_rgb(1.0, 0.3, 0.3)),
            ]
            .gap(6.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
            None => column![placeholder]
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        }
    }

    fn view_selected<'a>(&'a self, selected: &'a SelectedFunction) -> EditorElement<'a> {
        let save_label = if self.dirty {
            t!("save_dirty")
        } else {
            t!("save")
        };
        let naming_row = row![
            label(t!("name")),
            text_input("", &self.name_buffer)
                .on_input(ShaderFunctionEditorMessage::NameChanged)
                .width(Length::Fill),
            button(label(save_label))
                .primary()
                .on_press_maybe(self.dirty.then_some(ShaderFunctionEditorMessage::Save)),
        ]
        .gap(6.0)
        .height(Length::Shrink);

        let header = match self.error.as_ref() {
            Some(error) => {
                let error_text = EditorElement::<'a>::from(
                    label(error.clone()).color(Color::from_rgb(1.0, 0.3, 0.3)),
                );
                EditorElement::<'a>::from(column![naming_row, error_text].gap(2.0))
            }
            None => EditorElement::<'a>::from(naming_row),
        };

        let graph_editor = component(GraphEditor::new(
            &selected.function.graph,
            ShaderFunctionEditorMessage::Graph,
        ));

        column![header, graph_editor]
            .gap(6.0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn select_function(
        &mut self,
        id: AssetId<SerializableGraphFunction>,
        globals: &Globals,
    ) -> Result<()> {
        let Some(handle) = self
            .functions
            .iter()
            .find(|handle| handle.id() == id)
            .cloned()
        else {
            return Ok(());
        };

        let serialized = handle
            .get()
            .context("Failed to read shader graph function asset")?;
        let (maybe_function, errors) = serialized.deserialize_func(Some(id), globals.assets());
        if !errors.is_empty() {
            log::error!(
                "Shader graph function '{}' deserialized with errors: {:?}",
                serialized.name,
                errors
            );
        }
        let function = maybe_function.with_context(|| {
            format!(
                "Failed to deserialize shader graph function '{}'",
                serialized.name
            )
        })?;

        self.name_buffer = function.name.clone();
        self.selected = Some(SelectedFunction { handle, function });
        self.dirty = false;
        self.error = None;
        Ok(())
    }

    fn new_function(&mut self, globals: &mut Globals) -> Result<()> {
        let assets = globals.assets().clone();
        let bundle = assets
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .map(|bundle| bundle.metadata().bundle_id)
            .context("A writable asset bundle is required for a new shader graph function")?;

        let resources = GraphResources {
            type_registry: GRAPH_FUNCTION_TYPE_REGISTRY.clone(),
            node_registry: GRAPH_FUNCTION_NODE_REGISTRY.clone(),
            assets: assets.clone(),
        };
        let mut function = GraphFunction {
            asset_id: None,
            id: GraphFunctionId::new(Uuid::new_v4()),
            name: DEFAULT_FUNCTION_NAME.into(),
            graph: Graph::new(resources),
        };
        let serialized = SerializableGraphFunction::serialize_func(&function)
            .context("Failed to serialize new shader graph function")?;

        let path = format!(
            "unnamed_function_{}.{}",
            Uuid::new_v4(),
            SerializableGraphFunctionSerializer::file_extension()
        );
        let id = assets
            .add_asset(bundle, path, Arc::new(serialized))
            .context("Failed to add new shader graph function asset")?;
        let handle = assets
            .handle(id)
            .context("Failed to obtain handle for new shader graph function")?;

        function.asset_id = Some(id);
        if !self.functions.iter().any(|existing| existing.id() == id) {
            self.functions.push(handle.clone());
        }

        self.name_buffer = function.name.clone();
        self.selected = Some(SelectedFunction { handle, function });
        self.dirty = true;
        self.error = None;
        Ok(())
    }

    fn save(&mut self) {
        let Some(selected) = self.selected.as_mut() else {
            return;
        };

        let serialized = match SerializableGraphFunction::serialize_func(&selected.function) {
            Ok(serialized) => serialized,
            Err(error) => {
                self.error = Some(format!(
                    "Failed to serialize shader graph function: {error}"
                ));
                return;
            }
        };
        if let Err(error) = selected.handle.update(serialized) {
            self.error = Some(format!("Failed to update shader graph function: {error}"));
            return;
        }
        if let Err(error) = selected.handle.write() {
            self.error = Some(format!("Failed to write shader graph function: {error}"));
            return;
        }

        self.dirty = false;
        self.error = None;
    }

    fn handle_asset_event(&mut self, event: AssetEvent, globals: &Globals) {
        match globals
            .assets()
            .all_handles_of::<SerializableGraphFunction>()
        {
            Ok(functions) => self.functions = functions,
            Err(error) => {
                log::error!("Failed to refresh shader graph function list: {error:#}");
                return;
            }
        }

        match event {
            AssetEvent::Removed { id, .. } => {
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.handle.id().into_untyped() == id)
                {
                    self.selected = None;
                    self.name_buffer.clear();
                    self.dirty = false;
                }
            }
            AssetEvent::BundlesChanged { .. } => {
                if self.selected.as_ref().is_some_and(|selected| {
                    !self
                        .functions
                        .iter()
                        .any(|handle| handle.id() == selected.handle.id())
                }) {
                    self.selected = None;
                    self.name_buffer.clear();
                    self.dirty = false;
                }
            }
            AssetEvent::Added { .. } | AssetEvent::Modified { .. } => {}
        }
    }
}
