use iced_runtime::Task;
use lapiz_runtime::{
    global::Globals,
    windows::{OpenWindowViewCommand, ToggleWindowViewCommand, WindowCommandBuffer, WindowViewId},
};

use crate::{ActionFunction, ActionId};

/// Action that opens the brush editor window view.
#[derive(Default)]
pub struct OpenBrushEditorAction;

impl ActionFunction for OpenBrushEditorAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("open_brush_editor_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        globals
            .global_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new(WindowViewId::new(
                "brush_editor",
            )));
        Task::none()
    }
}

/// Action that toggles the visibility of the filter panel window view.
#[derive(Default)]
pub struct ToggleFilterPanelAction;

impl ActionFunction for ToggleFilterPanelAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("toggle_filter_panel_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        globals
            .global_mut::<WindowCommandBuffer>()
            .push(ToggleWindowViewCommand::new(WindowViewId::new(
                "filter_panel",
            )));
        Task::none()
    }
}
