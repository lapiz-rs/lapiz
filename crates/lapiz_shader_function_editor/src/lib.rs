use lapiz_runtime::{Runtime, plugin::Plugin};

use crate::editor::ShaderFunctionEditor;

lapiz_i18n::define_i18n!("shader_function_editor");

pub mod editor;

pub const SHADER_FUNCTION_EDITOR_VIEW_ID: &str = "shader_function_editor";

/// Registers localization and the [`ShaderFunctionEditor`] window view with the [`Runtime`].
pub struct ShaderFunctionEditorPlugin;

impl Plugin for ShaderFunctionEditorPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.register_view::<ShaderFunctionEditor>();
    }
}
