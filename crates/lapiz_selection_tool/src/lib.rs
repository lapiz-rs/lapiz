use lapiz_runtime::{Application, plugin::Plugin};
use lapiz_tools::ToolsAppExt;

use crate::{
    freehand::{FreehandSelectionTool, PolygonSelectionTool},
    magic_wand::MagicWandSelectionTool,
    shape::{EllipticalSelectionTool, RectangularSelectionTool},
};

pub mod freehand;
pub mod magic_wand;
pub mod render;
pub mod shape;

lapiz_i18n::define_i18n!("selection_tool");

pub struct SelectionPlugin;

impl Plugin for SelectionPlugin {
    fn build(&self, app: &mut Application) {
        crate::i18n::init();
        app.runtime_mut()
            .services_mut()
            .add_tool_function::<RectangularSelectionTool>()
            .add_tool_function::<EllipticalSelectionTool>()
            .add_tool_function::<FreehandSelectionTool>()
            .add_tool_function::<PolygonSelectionTool>()
            .add_tool_function::<MagicWandSelectionTool>();
    }
}
