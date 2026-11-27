use iced_runtime::Task;
use lapiz_asset_tag_editor::ASSET_TAG_EDITOR_VIEW_ID;
use lapiz_runtime::{
    global::Globals,
    windows::{OpenWindowViewCommand, ToggleWindowViewCommand, WindowCommandBuffer, WindowViewId},
};
use lapiz_shader_function_editor::SHADER_FUNCTION_EDITOR_VIEW_ID;

use crate::{ActionFunction, ActionId};

#[derive(Default)]
pub struct OpenAssetTagEditorAction;

impl ActionFunction for OpenAssetTagEditorAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("open_asset_tag_editor".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        globals
            .global_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new(WindowViewId::new(
                ASSET_TAG_EDITOR_VIEW_ID,
            )));
        Task::none()
    }
}

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

/// Action that opens the shader function editor window view.
#[derive(Default)]
pub struct OpenShaderFunctionEditorAction;

impl ActionFunction for OpenShaderFunctionEditorAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("open_shader_function_editor_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        globals
            .global_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new(WindowViewId::new(
                SHADER_FUNCTION_EDITOR_VIEW_ID,
            )));
        Task::none()
    }
}

#[derive(Default)]
pub struct OpenSettingsAction;

impl ActionFunction for OpenSettingsAction {
    type Message = ();

    fn id(&self) -> ActionId {
        ActionId::new("open_settings_action".into())
    }

    fn trigger(&self, globals: &mut Globals) -> Task<Self::Message> {
        globals
            .global_mut::<WindowCommandBuffer>()
            .push(OpenWindowViewCommand::new(WindowViewId::new("settings")));
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
