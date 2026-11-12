use lapiz_runtime::{Runtime, plugin::Plugin};
use lapiz_tools::ToolsAppExt as _;

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

/// A [`Plugin`] that registers the built-in selection tools into [`Globals`](lapiz_runtime::global::Globals).
///
/// Registers [`RectangularSelectionTool`], [`EllipticalSelectionTool`], [`FreehandSelectionTool`],
/// [`PolygonSelectionTool`], and [`MagicWandSelectionTool`].
pub struct SelectionPlugin;

impl Plugin for SelectionPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.globals_mut()
            .add_tool_function::<RectangularSelectionTool>()
            .add_tool_function::<EllipticalSelectionTool>()
            .add_tool_function::<FreehandSelectionTool>()
            .add_tool_function::<PolygonSelectionTool>()
            .add_tool_function::<MagicWandSelectionTool>();
    }
}
