use lapiz_assets::AssetAppExt as _;
use lapiz_runtime::{Runtime, plugin::Plugin};
use lapiz_tools::ToolsAppExt as _;

use crate::{
    asset::BrushPresetSerializer,
    editor::BrushEditor,
    engine::{BrushEngineRegistry, lapiz_engine::LapizBrushEngine},
    tool::BrushTool,
};

pub mod asset;
pub mod editor;
pub mod engine;
pub mod input_processing;
pub mod instance;
pub mod render;
pub mod tool;
pub mod widget;

wesl::wesl_pkg!(pub brush);

lapiz_i18n::define_i18n!("brush");

/// Plugin that registers [`BrushPresetSerializer`], [`BrushTool`], [`BrushEditor`], and background stroke preview generation.
pub struct BrushPlugin;

impl Plugin for BrushPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();

        app.register_view::<BrushEditor>();

        let globals = app.globals_mut();
        globals.add_asset_serializer::<BrushPresetSerializer>();
        globals.add_tool_function::<BrushTool>();

        let mut registry = BrushEngineRegistry::default();
        registry.register::<LapizBrushEngine>();
        app.add_global_instance(registry);
    }
}
