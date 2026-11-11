use lapiz_runtime::{Runtime, plugin::Plugin};
use lapiz_tools::ToolsAppExt as _;

use crate::{
    free::FreeTransformTool, liquify::LiquifyTransformTool, perspective::PerspectiveTransformTool,
};

pub mod free;
pub mod liquify;
pub mod perspective;

lapiz_i18n::define_i18n!("transform_tool");

/// A [`Plugin`] that registers [`FreeTransformTool`], [`LiquifyTransformTool`], and [`PerspectiveTransformTool`].
pub struct FreeTransformPlugin;

impl Plugin for FreeTransformPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.globals_mut()
            .add_tool_function::<FreeTransformTool>()
            .add_tool_function::<LiquifyTransformTool>()
            .add_tool_function::<PerspectiveTransformTool>();
    }
}
