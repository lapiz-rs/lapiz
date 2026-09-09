use std::{any::Any, collections::HashMap, rc::Rc, sync::Arc};

use iced_core::{Element, Theme};
use iced_runtime::{Task, futures::Subscription};
use iced_widget::{Stack, space};
use lapiz_assets::AssetAppExt;
use lapiz_input::{
    key::{KeySequence, KeyboardState},
    mouse::{HoverMouseState, PressedMouseState},
};
use lapiz_runtime::{Application, Renderer, Services, plugin::Plugin, service::Service};
use lapiz_utils::{Deref, DerefMut, wrapper};
use lapiz_widgets::icon::Icon;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::manifest::{ToolBinding, ToolBindingManifest, ToolBindingManifestSerializer};

pub mod manifest;

pub struct ToolsPlugin;

impl Plugin for ToolsPlugin {
    fn build(&self, app: &mut Application) {
        app.add_service::<ToolFunctionRegistry>()
            .add_service::<ToolProxies>()
            .add_service::<GlobalToolBindings>();
        app.runtime_mut()
            .services_mut()
            .add_asset_serializer::<ToolBindingManifestSerializer>();
    }

    fn finish(&self, app: &mut Application) {
        let mut runtime = app.runtime_mut();
        let services = runtime.services_mut();
        let manifest = services
            .assets()
            .all_handles_of::<ToolBindingManifest>()
            .expect("Failed to read tool binding manifests")
            .into_iter()
            .next()
            .expect("At least one tool binding manifest should exist")
            .get()
            .expect("Failed to load tool binding manifest");

        let bindings = manifest
            .bindings
            .iter()
            .cloned()
            .map(|binding| (binding.shortcut, binding))
            .collect();
        services.service_mut::<GlobalToolBindings>().bindings = bindings;
    }
}

pub trait ToolsAppExt {
    fn add_tool_function<T: ToolFunction + Default>(&mut self) -> &mut Self;
}

impl ToolsAppExt for Services {
    fn add_tool_function<T: ToolFunction + Default>(&mut self) -> &mut Self {
        self.service_mut::<ToolFunctionRegistry>().register::<T>();
        self
    }
}

wrapper! {
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Display, Serialize, Deserialize)]
    #[display("{0}")]
    pub ToolId : Arc<str>
}

pub trait ToolFunction: 'static {
    type Message: Send + Sync + 'static;

    fn id() -> ToolId;
    fn icon() -> Icon<'static>;
    fn activate(&mut self, _: &mut Services) -> Task<Self::Message> {
        Task::none()
    }
    fn hover(
        &mut self,
        _: &KeyboardState,
        _: &HoverMouseState,
        _: &mut Services,
    ) -> Task<Self::Message> {
        Task::none()
    }
    fn begin(
        &mut self,
        _: &KeyboardState,
        _: &PressedMouseState,
        _: &mut Services,
    ) -> Task<Self::Message> {
        Task::none()
    }
    fn update(
        &mut self,
        _: &KeyboardState,
        _: &PressedMouseState,
        _: &mut Services,
    ) -> Task<Self::Message> {
        Task::none()
    }
    fn end(
        &mut self,
        _: &KeyboardState,
        _: &PressedMouseState,
        _: &mut Services,
    ) -> Task<Self::Message> {
        Task::none()
    }
    fn deactivate(&mut self, _: &mut Services) -> Task<Self::Message> {
        Task::none()
    }
    fn handle_message(&mut self, _: Self::Message, _: &mut Services) -> Task<Self::Message> {
        Task::none()
    }
    fn tool_option_widget<'a>(
        &'a self,
        _: &'a Services,
    ) -> Option<Element<'a, Self::Message, iced_core::Theme, lapiz_runtime::Renderer>> {
        None
    }
    fn canvas_overlay<'a>(
        &'a self,
        _: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        space().into()
    }
    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::none()
    }
}

pub trait ErasedToolFunction: 'static {
    fn id(&self) -> ToolId;
    fn icon(&self) -> Icon<'static>;
    fn activate(&mut self, services: &mut Services) -> Task<ErasedToolFunctionMessage>;
    fn hover(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &HoverMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage>;
    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage>;
    fn update(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage>;
    fn end(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage>;
    fn deactivate(&mut self, services: &mut Services) -> Task<ErasedToolFunctionMessage>;
    fn handle_message(
        &mut self,
        message: Box<dyn Any + Send + Sync>,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage>;
    fn tool_option_widget<'a>(
        &'a self,
        services: &'a Services,
    ) -> Option<Element<'a, ErasedToolFunctionMessage, Theme, Renderer>>;
    fn canvas_overlay<'a>(
        &'a self,
        services: &'a Services,
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

    fn activate(&mut self, services: &mut Services) -> Task<ErasedToolFunctionMessage> {
        self.activate(services)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn hover(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &HoverMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage> {
        self.hover(keyboard, mouse, services)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn begin(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage> {
        self.begin(keyboard, mouse, services)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn update(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage> {
        self.update(keyboard, mouse, services)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn end(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage> {
        self.end(keyboard, mouse, services)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn deactivate(&mut self, services: &mut Services) -> Task<ErasedToolFunctionMessage> {
        self.deactivate(services)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn handle_message(
        &mut self,
        message: Box<dyn Any + Send + Sync>,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage> {
        let message = message
            .downcast::<T::Message>()
            .expect("Invalid message type passed to tool function");
        self.handle_message(*message, services)
            .map(move |message| ErasedToolFunctionMessage {
                tool_id: T::id(),
                message: Box::new(message),
            })
    }

    fn tool_option_widget<'a>(
        &'a self,
        services: &'a Services,
    ) -> Option<Element<'a, ErasedToolFunctionMessage, Theme, Renderer>> {
        let id = T::id();
        self.tool_option_widget(services).map(|e| {
            e.map(move |message| ErasedToolFunctionMessage {
                tool_id: id.clone(),
                message: Box::new(message),
            })
        })
    }

    fn canvas_overlay<'a>(
        &'a self,
        services: &'a Services,
    ) -> Element<'a, ErasedToolFunctionMessage, Theme, Renderer> {
        self.canvas_overlay(services)
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

pub struct ErasedToolFunctionMessage {
    pub tool_id: ToolId,
    pub message: Box<dyn Any + Send + Sync>,
}

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

impl Service for ToolFunctionRegistry {}

struct State {
    function: ToolId,
    // TODO prevent tool switching on some scenarios like the tool is updating
    is_updating: bool,
}

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
        services: &mut Services,
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
                    .deactivate(services)
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
            .activate(services);
        self.current_state = Some(State {
            function: tool,
            is_updating: false,
        });
        deactivate.chain(activate)
    }

    pub fn switch_override_tool(
        &mut self,
        tool: Option<ToolId>,
        services: &mut Services,
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
                    .deactivate(services)
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
                .activate(services);
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
        services: &mut Services,
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
            .begin(keyboard, mouse, services)
    }

    pub fn mouse_moved_pressing(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage> {
        let Some(state) = self.override_state.as_ref().or(self.current_state.as_ref()) else {
            return Task::none();
        };
        let function = self.tool_functions.get_mut(&state.function).unwrap();
        if state.is_updating {
            function.update(keyboard, mouse, services)
        } else {
            Task::none()
        }
    }

    pub fn mouse_moved_hovering(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &HoverMouseState,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage> {
        let Some(state) = self.override_state.as_ref().or(self.current_state.as_ref()) else {
            return Task::none();
        };
        let function = self.tool_functions.get_mut(&state.function).unwrap();
        if state.is_updating {
            Task::none()
        } else {
            function.hover(keyboard, mouse, services)
        }
    }

    pub fn mouse_released(
        &mut self,
        keyboard: &KeyboardState,
        mouse: &PressedMouseState,
        services: &mut Services,
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
            .end(keyboard, mouse, services)
    }

    pub fn handle_message(
        &mut self,
        message: ErasedToolFunctionMessage,
        services: &mut Services,
    ) -> Task<ErasedToolFunctionMessage> {
        let Some(function) = self.tool_functions.get_mut(&message.tool_id) else {
            return Task::none();
        };
        function.handle_message(message.message, services)
    }

    pub fn tool_option_widget<'a>(
        &'a self,
        services: &'a Services,
    ) -> Option<Element<'a, ErasedToolFunctionMessage, iced_core::Theme, lapiz_runtime::Renderer>>
    {
        let overrider_widget = self
            .override_state
            .as_ref()
            .and_then(|s| self.tool_functions.get(&s.function))
            .and_then(|f| f.tool_option_widget(services));
        if let Some(widget) = overrider_widget {
            return Some(widget);
        }

        self.current_state
            .as_ref()
            .and_then(|s| self.tool_functions.get(&s.function))
            .and_then(|f| f.tool_option_widget(services))
    }

    pub fn canvas_overlay<'a>(
        &'a self,
        services: &'a Services,
    ) -> Element<'a, ErasedToolFunctionMessage, Theme, Renderer> {
        let mut overlays = Stack::new();

        if let Some(state) = &self.current_state {
            overlays = overlays.push(
                self.tool_functions
                    .get(&state.function)
                    .unwrap()
                    .canvas_overlay(services),
            );
        }

        if let Some(state) = &self.override_state {
            overlays = overlays.push(
                self.tool_functions
                    .get(&state.function)
                    .unwrap()
                    .canvas_overlay(services),
            );
        }

        overlays.into()
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

#[derive(Default, Deref, DerefMut)]
pub struct ToolProxies {
    proxies: HashMap<Uuid, ToolProxy>,
}

impl Service for ToolProxies {}

#[derive(Default)]
pub struct GlobalToolBindings {
    bindings: Vec<(KeySequence, ToolBinding)>,
}

impl Service for GlobalToolBindings {}

impl GlobalToolBindings {
    pub fn binding_for(&self, shortcut: KeySequence) -> Option<&ToolBinding> {
        self.bindings
            .iter()
            .find_map(|(key, binding)| (*key == shortcut).then_some(binding))
    }
}
