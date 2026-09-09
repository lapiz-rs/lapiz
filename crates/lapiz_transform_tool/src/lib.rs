use lapiz_runtime::{Application, plugin::Plugin};
use lapiz_tools::ToolsAppExt;

use crate::{
    free::FreeTransformTool, liquify::LiquifyTransformTool, perspective::PerspectiveTransformTool,
};

pub mod free;
pub mod liquify;
pub mod perspective;

lapiz_i18n::define_i18n!("transform_tool");

pub struct FreeTransformPlugin;

impl Plugin for FreeTransformPlugin {
    fn build(&self, app: &mut Application) {
        crate::i18n::init();
        app.runtime_mut()
            .services_mut()
            .add_tool_function::<FreeTransformTool>()
            .add_tool_function::<LiquifyTransformTool>()
            .add_tool_function::<PerspectiveTransformTool>();
    }
}
