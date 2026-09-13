use iced_runtime::Task;
use lapiz_runtime::{
    Services,
    windows::{OpenWindowViewCommand, ToggleWindowViewCommand, WindowCommandBuffer, WindowViewId},
};

use crate::{ActionFunction, ActionId};

#[derive(Default)]
pub struct OpenBrushEditorAction;

impl ActionFunction for OpenBrushEditorAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("open_brush_editor_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        services
            .service_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new(WindowViewId::new(
                "brush_editor",
            )));
        Task::none()
    }
}

#[derive(Default)]
pub struct ToggleFilterPanelAction;

impl ActionFunction for ToggleFilterPanelAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("toggle_filter_panel_action".into())
    }

    fn trigger(&self, services: &mut Services) -> Task<Self::Message> {
        services
            .service_mut::<WindowCommandBuffer>()
            .push(ToggleWindowViewCommand::new(WindowViewId::new(
                "filter_panel",
            )));
        Task::none()
    }
}
