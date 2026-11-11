use std::{any::Any, collections::HashMap, rc::Rc, sync::Arc};

use iced_core::{Element, Length, Theme};
use iced_runtime::{Task, futures::Subscription};
use lapiz_input::{
    key::{KeySequence, KeyboardState},
    mouse::{HoverMouseState, PressedMouseState},
};
use lapiz_runtime::{
    Renderer, Runtime,
    global::{Global, Globals},
    plugin::Plugin,
};
use lapiz_utils::{Deref, DerefMut, wrapper};
use lapiz_widgets::{icon::Icon, space, stack};
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::manifest::{ToolBinding, ToolBindingManifestConfig};

pub mod manifest;

/// A [`Plugin`] that initializes the tool subsystem and loads configured tool shortcut bindings.
///
/// Registers [`ToolFunctionRegistry`], [`ToolProxies`], and [`GlobalToolBindings`] into the
/// runtime's [`Globals`], and populates shortcut bindings from [`ToolBindingManifestConfig`]
/// during plugin completion.
pub struct ToolsPlugin;

impl Plugin for ToolsPlugin {
    fn build(&self, app: &mut Runtime) {
        app.add_global::<ToolFunctionRegistry>()
            .add_global::<ToolProxies>()
            .add_global::<GlobalToolBindings>();
    }

    fn finish(&self, app: &mut Runtime) {
        let tool_bindings = ToolBindingManifestConfig::read_or_init_or_fallback();

        let bindings = tool_bindings
            .get()
            .bindings
            .iter()
            .cloned()
            .map(|binding| (binding.shortcut, binding))
            .collect();

        app.globals_mut()
            .global_mut::<GlobalToolBindings>()
            .bindings = bindings;
    }
}

/// Extension trait for registering [`ToolFunction`] implementations into [`Globals`].
pub trait ToolsAppExt {
    fn add_tool_function<T: ToolFunction + Default>(&mut self) -> &mut Self;
}

impl ToolsAppExt for Globals {
    fn add_tool_function<T: ToolFunction + Default>(&mut self) -> &mut Self {
        self.global_mut::<ToolFunctionRegistry>().register::<T>();
        self
    }
}

wrapper! {
    /// A unique string identifier for a registered [`ToolFunction`].
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Display, Serialize, Deserialize)]
    #[display("{0}")]
    pub ToolId : Arc<str>
}

/// A linear history stack for tracking tool-local state changes and supporting undo/redo.
///
/// Pushing a new state truncates any redo history beyond the current cursor before appending
/// the snapshot.
pub struct ChangesTracker<S> {
    cursor: usize,
    history: Vec<S>,
}

impl<S> Default for ChangesTracker<S> {
    fn default() -> Self {
        Self {
            cursor: 0,
            history: Vec::new(),
        }
    }
}

impl<S> ChangesTracker<S> {
    pub fn push(&mut self, state: S) {
        self.history.truncate(self.cursor);
        self.history.push(state);
        self.cursor = self.history.len();
    }

    pub fn undo(&mut self) -> Option<&S> {
        self.cursor = self.cursor.checked_sub(1)?;
        self.history.get(self.cursor)
    }

    pub fn redo(&mut self) -> Option<&S> {
        let state = self.history.get(self.cursor)?;
        self.cursor += 1;
        Some(state)
    }
}

/// An interactive canvas tool that responds to pointer/keyboard input and renders UI overlays.
///
/// `ToolFunction` defines the lifecycle and interaction contract for all tools in Lapiz (such as
/// brushes, selection tools, transform tools, and the eyedropper). Tool types are registered in the
/// global [`ToolFunctionRegistry`] and instantiated per canvas inside a [`ToolProxy`].
///
/// # Lifecycle and Interaction
///
/// - **Activation**: [`activate`](ToolFunction::activate) and [`deactivate`](ToolFunction::deactivate)
///   are called when switching tools or temporary override tools.
/// - **Pointer Events**: [`hover`](ToolFunction::hover) handles unpressed cursor movement, while a stroke
///   or drag gesture progresses through [`begin`](ToolFunction::begin), [`update`](ToolFunction::update),
///   and [`end`](ToolFunction::end).
/// - **History**: Tools with internal multi-step state can intercept [`undo`](ToolFunction::undo) and
///   [`redo`](ToolFunction::redo) before canvas-level history is triggered.
/// - **UI Integration**: Tools can expose a configuration panel via [`tool_option_widget`](ToolFunction::tool_option_widget),
///   draw custom interactive gizmos over the canvas via [`canvas_overlay`](ToolFunction::canvas_overlay),
///   and listen to background events via [`subscription`](ToolFunction::subscription).
pub trait ToolFunction: 'static {
    type Message: Send + Sync + 'static;

    fn id() -> ToolId;
    fn icon() -> Icon<'static>;
    fn activate(&mut self, _: &mut Globals) -> Task<Self::Message> {
        Task::none()
    }
    fn hover(
        &mut self,
        _: &KeyboardState,
        _: &HoverMouseState,
        _: &mut Globals,
    ) -> Task<Self::Message> {
        Task::none()
    }
    fn begin(
        &mut self,
        _: &KeyboardState,
        _: &PressedMouseState,
        _: &mut Globals,
    ) -> Task<Self::Message> {
        Task::none()
    }
    fn update(
        &mut self,
        _: &KeyboardState,
        _: &PressedMouseState,
        _: &mut Globals,
    ) -> Task<Self::Message> {
        Task::none()
    }
    fn end(
        &mut self,
        _: &KeyboardState,
        _: &PressedMouseState,
        _: &mut Globals,
    ) -> Task<Self::Message> {
        Task::none()
    }
    fn undo(&mut self, _: &mut Globals) -> bool {
        false
    }
    fn redo(&mut self, _: &mut Globals) -> bool {
        false
    }
    fn deactivate(&mut self, _: &mut Globals) -> Task<Self::Message> {
        Task::none()
    }
    fn handle_message(&mut self, _: Self::Message, _: &mut Globals) -> Task<Self::Message> {
        Task::none()
    }
    fn tool_option_widget<'a>(
        &'a self,
        _: &'a Globals,
    ) -> Option<Element<'a, Self::Message, iced_core::Theme, lapiz_runtime::Renderer>> {
        None
    }
    fn canvas_overlay<'a>(&'a self, _: &'a Globals) -> Element<'a, Self::Message, Theme, Renderer> {
        space().into()
    }
    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::none()
    }
}

pub trait ErasedToolFunction: 'static {
    fn id(&self) -> ToolId;
    fn icon(&self) -> Icon<'static>;
    fn activate(&mut self, globals: &mut Globals) -> Task<ErasedToolFunctionMessage>;
    fn hover(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &HoverMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage>;
    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage>;
    fn update(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage>;
    fn end(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage>;
    fn undo(&mut self, globals: &mut Globals) -> bool;
    fn redo(&mut self, globals: &mut Globals) -> bool;
    fn deactivate(&mut self, globals: &mut Globals) -> Task<ErasedToolFunctionMessage>;
    fn handle_message(
        &mut self,
        message: Box<dyn Any + Send + Sync>,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage>;
    fn tool_option_widget<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Option<Element<'a, ErasedToolFunctionMessage, Theme, Renderer>>;
    fn canvas_overlay<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, ErasedToolFunctionMessage, Theme, Renderer>;
    fn subscription(&self) -> Subscription<ErasedToolFunctionMessage>;
}

impl<T: ToolFunction> ErasedToolFunction for T {
    fn id(&self) -> ToolId {
        T::id()
    }

    fn icon(&self) -> Icon<'static> {
        T::icon()
    }

    fn activate(&mut self, globals: &mut Globals) -> Task<ErasedToolFunctionMessage> {
        self.activate(globals)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn hover(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &HoverMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        self.hover(keyboard, mouse, globals)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        self.begin(keyboard, mouse, globals)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn update(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        self.update(keyboard, mouse, globals)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn end(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        self.end(keyboard, mouse, globals)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn undo(&mut self, globals: &mut Globals) -> bool {
        ToolFunction::undo(self, globals)
    }

    fn redo(&mut self, globals: &mut Globals) -> bool {
        ToolFunction::redo(self, globals)
    }

    fn deactivate(&mut self, globals: &mut Globals) -> Task<ErasedToolFunctionMessage> {
        self.deactivate(globals)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn handle_message(
        &mut self,
        message: Box<dyn Any + Send + Sync>,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        let message = message
            .downcast::<T::Message>()
            .expect("Invalid message type passed to tool function");
        self.handle_message(*message, globals)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn tool_option_widget<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Option<Element<'a, ErasedToolFunctionMessage, Theme, Renderer>> {
        let id = T::id();
        self.tool_option_widget(globals).map(|e| {
            e.map(move |message| ErasedToolFunctionMessage {
                tool_id: id.clone(),
                message: Box::new(message),
            })
        })
    }

    fn canvas_overlay<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, ErasedToolFunctionMessage, Theme, Renderer> {
        self.canvas_overlay(globals)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn subscription(&self) -> Subscription<ErasedToolFunctionMessage> {
        self.subscription()
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }
}

/// A type-erased message emitted by an [`ErasedToolFunction`], tagged with its [`ToolId`].
pub struct ErasedToolFunctionMessage {
    pub tool_id: ToolId,
    pub message: Box<dyn Any + Send + Sync>,
}

/// Global registry of factory functions for spawning registered [`ToolFunction`] instances.
///
/// Tools are registered at startup via [`ToolsAppExt::add_tool_function`] or
/// [`register`](ToolFunctionRegistry::register). When a new canvas is created, [`ToolProxy::new`]
/// uses this registry to instantiate a fresh set of tool states for that canvas.
#[derive(Default)]
pub struct ToolFunctionRegistry {
    spawners: HashMap<ToolId, Rc<dyn Fn() -> Box<dyn ErasedToolFunction>>>,
}

impl ToolFunctionRegistry {
    pub fn register<T: ToolFunction + Default>(&mut self) {
        self.spawners
            .insert(T::id(), Rc::new(|| Box::new(T::default())));
    }

    pub fn icon(&self, id: &ToolId) -> Option<Icon<'static>> {
        let spawn = self.spawners.get(id)?;
        Some(spawn().icon())
    }
}

impl Global for ToolFunctionRegistry {}

/// Active state of a selected or temporary override tool within a [`ToolProxy`].
struct State {
    function: ToolId,
    // TODO prevent tool switching on some scenarios like the tool is updating
    is_updating: bool,
}

/// Per-canvas manager that owns instantiated [`ToolFunction`]s and routes input, messages, and UI.
///
/// Each canvas has its own `ToolProxy` stored in [`ToolProxies`]. It tracks both a primary active
/// tool (`current_state`) and an optional temporary override tool (`override_state`, such as a
/// held-shortcut tool). When an override tool is active, it takes precedence for input events,
/// undo/redo, subscriptions, and tool option widgets, while both overlays are stacked on the canvas.
pub struct ToolProxy {
    current_state: Option<State>,
    override_state: Option<State>,
    tool_functions: HashMap<ToolId, Box<dyn ErasedToolFunction>>,
}

impl ToolProxy {
    pub fn new(registry: &ToolFunctionRegistry) -> Self {
        let tool_functions = registry
            .spawners
            .iter()
            .map(|(id, spawner)| (id.clone(), spawner()))
            .collect();

        Self {
            current_state: None,
            override_state: None,
            tool_functions,
        }
    }

    pub fn switch_tool(
        &mut self,
        tool: ToolId,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        if Some(&tool) == self.current_tool() {
            return Task::none();
        }

        log::info!("Switching tool: {}", tool);
        let deactivate = self
            .current_state
            .take()
            .map(|state| {
                self.tool_functions
                    .get_mut(&state.function)
                    .unwrap()
                    .deactivate(globals)
            })
            .unwrap_or_else(Task::none);

        if !self.tool_functions.contains_key(&tool) {
            log::error!("Unable to switch to tool {:?}: not found in registry", tool);
            return deactivate;
        }

        let activate = self
            .tool_functions
            .get_mut(&tool)
            .unwrap()
            .activate(globals);
        self.current_state = Some(State {
            function: tool,
            is_updating: false,
        });
        deactivate.chain(activate)
    }

    pub fn switch_override_tool(
        &mut self,
        tool: Option<ToolId>,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        if tool.as_ref() == self.override_tool() {
            return Task::none();
        }

        log::info!("Switching override tool: {:?}", tool);
        let mut deactivate = self
            .override_state
            .take()
            .map(|state| {
                self.tool_functions
                    .get_mut(&state.function)
                    .unwrap()
                    .deactivate(globals)
            })
            .unwrap_or_else(Task::none);

        if let Some(tool) = tool {
            if !self.tool_functions.contains_key(&tool) {
                log::error!("Unable to switch to tool {:?}: not found in registry", tool);
                return deactivate;
            }

            let activate = self
                .tool_functions
                .get_mut(&tool)
                .unwrap()
                .activate(globals);
            self.override_state = Some(State {
                function: tool,
                is_updating: false,
            });
            deactivate = deactivate.chain(activate);
        }

        deactivate
    }

    pub fn mouse_pressed(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        let Some(state) = self.override_state.as_mut().or(self.current_state.as_mut()) else {
            return Task::none();
        };
        if state.is_updating {
            return Task::none();
        }
        state.is_updating = true;
        self.tool_functions
            .get_mut(&state.function)
            .unwrap()
            .begin(keyboard, mouse, globals)
    }

    pub fn mouse_moved_pressing(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        let Some(state) = self.override_state.as_ref().or(self.current_state.as_ref()) else {
            return Task::none();
        };
        let function = self.tool_functions.get_mut(&state.function).unwrap();
        if state.is_updating {
            function.update(keyboard, mouse, globals)
        } else {
            Task::none()
        }
    }

    pub fn mouse_moved_hovering(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &HoverMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        let Some(state) = self.override_state.as_ref().or(self.current_state.as_ref()) else {
            return Task::none();
        };
        let function = self.tool_functions.get_mut(&state.function).unwrap();
        if state.is_updating {
            Task::none()
        } else {
            function.hover(keyboard, mouse, globals)
        }
    }

    pub fn mouse_released(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        let Some(state) = self.override_state.as_mut().or(self.current_state.as_mut()) else {
            return Task::none();
        };
        if !state.is_updating {
            return Task::none();
        }
        state.is_updating = false;
        self.tool_functions
            .get_mut(&state.function)
            .unwrap()
            .end(keyboard, mouse, globals)
    }

    pub fn undo(&mut self, globals: &mut Globals) -> bool {
        let Some(state) = self.override_state.as_mut().or(self.current_state.as_mut()) else {
            return false;
        };
        self.tool_functions
            .get_mut(&state.function)
            .unwrap()
            .undo(globals)
    }

    pub fn redo(&mut self, globals: &mut Globals) -> bool {
        let Some(state) = self.override_state.as_mut().or(self.current_state.as_mut()) else {
            return false;
        };
        self.tool_functions
            .get_mut(&state.function)
            .unwrap()
            .redo(globals)
    }

    pub fn handle_message(
        &mut self,
        message: ErasedToolFunctionMessage,
        globals: &mut Globals,
    ) -> Task<ErasedToolFunctionMessage> {
        let Some(function) = self.tool_functions.get_mut(&message.tool_id) else {
            return Task::none();
        };
        function.handle_message(message.message, globals)
    }

    pub fn tool_option_widget<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Option<Element<'a, ErasedToolFunctionMessage, iced_core::Theme, lapiz_runtime::Renderer>>
    {
        let overrider_widget = self
            .override_state
            .as_ref()
            .and_then(|s| self.tool_functions.get(&s.function))
            .and_then(|f| f.tool_option_widget(globals));
        if let Some(widget) = overrider_widget {
            return Some(widget);
        }

        self.current_state
            .as_ref()
            .and_then(|s| self.tool_functions.get(&s.function))
            .and_then(|f| f.tool_option_widget(globals))
    }

    pub fn canvas_overlay<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, ErasedToolFunctionMessage, Theme, Renderer> {
        let mut overlays = stack![];

        if let Some(state) = &self.current_state {
            overlays = overlays.push(
                self.tool_functions
                    .get(&state.function)
                    .unwrap()
                    .canvas_overlay(globals),
            );
        }

        if let Some(state) = &self.override_state {
            overlays = overlays.push(
                self.tool_functions
                    .get(&state.function)
                    .unwrap()
                    .canvas_overlay(globals),
            );
        }

        overlays.width(Length::Fill).height(Length::Fill).into()
    }

    pub fn subscription(&self) -> Option<Subscription<ErasedToolFunctionMessage>> {
        let state = self
            .override_state
            .as_ref()
            .or(self.current_state.as_ref())?;
        Some(
            self.tool_functions
                .get(&state.function)
                .unwrap()
                .subscription(),
        )
    }

    pub fn current_tool(&self) -> Option<&ToolId> {
        Some(&self.current_state.as_ref()?.function)
    }

    pub fn override_tool(&self) -> Option<&ToolId> {
        Some(&self.override_state.as_ref()?.function)
    }
}

/// Global collection mapping canvas [`Uuid`]s to their respective [`ToolProxy`] instances.
#[derive(Default, Deref, DerefMut)]
pub struct ToolProxies {
    proxies: HashMap<Uuid, ToolProxy>,
}

impl Global for ToolProxies {}

/// Global lookup table of keyboard shortcuts mapped to [`ToolBinding`] definitions.
#[derive(Default)]
pub struct GlobalToolBindings {
    bindings: Vec<(KeySequence, ToolBinding)>,
}

impl Global for GlobalToolBindings {}

impl GlobalToolBindings {
    pub fn binding_for(&self, shortcut: KeySequence) -> Option<&ToolBinding> {
        self.bindings
            .iter()
            .find_map(|(key, binding)| (*key == shortcut).then_some(binding))
    }
}
