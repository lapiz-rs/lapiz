use std::{any::Any, collections::HashMap, sync::Arc};

use iced_runtime::Task;
use lapiz_assets::AssetAppExt;
use lapiz_runtime::{Application, Services, plugin::Plugin, service::Service};
use lapiz_utils::wrapper;
use parse_display::Display;
use serde::{Deserialize, Serialize};

use crate::{
    edit::{PasteIntoNewLayerAction, RedoAction, UndoAction},
    file::{OpenFileAction, SaveFileAction},
    layer::{
        CreateNewLayerAction, DeleteSelectedLayersAction, GroupSelectedLayersAction,
        MoveLayerDownAction, MoveLayerUpAction, SelectNextLayerAction, SelectPreviousLayerAction,
    },
    selection::DeleteSelectionAction,
    window::{OpenBrushEditorAction, ToggleFilterPanelAction},
};

pub mod edit;
pub mod file;
pub mod layer;
pub mod manifest;
pub mod selection;
pub mod window;

wrapper! {
    #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    pub ActionId : Arc<str>
}

pub struct ActionPlugin;

impl Plugin for ActionPlugin {
    fn build(&self, app: &mut Application) {
        let mut runtime = app.runtime_mut();
        runtime.add_service::<ActionFunctionRegistry>();
        let services = runtime.services_mut();
        services
            .add_action_function::<DeleteSelectionAction>()
            .add_action_function::<OpenFileAction>()
            .add_action_function::<SaveFileAction>()
            .add_action_function::<CreateNewLayerAction>()
            .add_action_function::<MoveLayerUpAction>()
            .add_action_function::<MoveLayerDownAction>()
            .add_action_function::<DeleteSelectedLayersAction>()
            .add_action_function::<GroupSelectedLayersAction>()
            .add_action_function::<SelectNextLayerAction>()
            .add_action_function::<SelectPreviousLayerAction>()
            .add_action_function::<PasteIntoNewLayerAction>()
            .add_action_function::<OpenBrushEditorAction>()
            .add_action_function::<ToggleFilterPanelAction>()
            .add_action_function::<UndoAction>()
            .add_action_function::<RedoAction>();
    }
}

pub trait ActionAppExt {
    fn add_action_function<A: ActionFunction + Default>(&mut self) -> &mut Self;
}

impl ActionAppExt for Services {
    fn add_action_function<A: ActionFunction + Default>(&mut self) -> &mut Self {
        self.service_mut::<ActionFunctionRegistry>().register::<A>();
        self
    }
}

pub trait ActionFunction: Send + Sync + 'static {
    type Message: Send + Sync + 'static;

    fn id(&self) -> ActionId;
    fn trigger(&self, services: &mut Services) -> Task<Self::Message>;
    fn handle_message(
        &self,
        _message: Self::Message,
        _services: &mut Services,
    ) -> Task<Self::Message> {
        Task::none()
    }
}

pub trait ErasedActionFunction: Send + Sync + 'static {
    fn id(&self) -> ActionId;
    fn trigger(&self, services: &mut Services) -> Task<Box<dyn Any + Send + Sync>>;
    fn handle_message(
        &self,
        _message: Box<dyn Any + Send + Sync>,
        _services: &mut Services,
    ) -> Task<Box<dyn Any + Send + Sync>> {
        Task::none()
    }
}

impl<T: ActionFunction> ErasedActionFunction for T {
    fn id(&self) -> ActionId {
        self.id()
    }

    fn trigger(&self, services: &mut Services) -> Task<Box<dyn Any + Send + Sync>> {
        self.trigger(services)
            .map(|message| Box::new(message) as Box<dyn Any + Send + Sync>)
    }

    fn handle_message(
        &self,
        message: Box<dyn Any + Send + Sync>,
        services: &mut Services,
    ) -> Task<Box<dyn Any + Send + Sync>> {
        let message = message
            .downcast::<T::Message>()
            .expect("Invalid message type");
        self.handle_message(*message, services)
            .map(|message| Box::new(message) as Box<dyn Any + Send + Sync>)
    }
}

#[derive(Default)]
pub struct ActionFunctionRegistry {
    functions: HashMap<ActionId, Arc<dyn ErasedActionFunction>>,
}

impl Service for ActionFunctionRegistry {}

impl ActionFunctionRegistry {
    pub fn register<A: ActionFunction + Default>(&mut self) {
        let action = A::default();
        self.functions.insert(action.id(), Arc::new(action));
    }

    pub fn get(&self, action_id: ActionId) -> Option<Arc<dyn ErasedActionFunction>> {
        self.functions.get(&action_id).cloned()
    }
}
