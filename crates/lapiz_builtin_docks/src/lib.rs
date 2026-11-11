pub mod brush_preset;
pub mod canvas;
pub mod color_selector;
pub mod layers;
pub mod recent_files;
pub mod tool_box;
pub mod tool_options;

lapiz_i18n::define_i18n!("builtin_docks");

use lapiz_dock::DockRegistry;
use lapiz_runtime::{Runtime, plugin::Plugin};

use crate::{
    brush_preset::BrushPresetDock, color_selector::ColorSelectorDock, layers::LayersDock,
    recent_files::LandingDock, tool_box::ToolBoxDock, tool_options::ToolOptionsDock,
};

/// Plugin that initializes localization and registers all built-in workspace [`Dock`](lapiz_dock::dock::Dock) panels in [`DockRegistry`].
pub struct BuiltinDocksPlugin;

impl Plugin for BuiltinDocksPlugin {
    fn build(&self, _app: &mut Runtime) {
        i18n::init();
    }

    fn finish(&self, app: &mut Runtime) {
        let mut registry = DockRegistry::default();

        {
            let globals = app.globals_mut();

            registry.register(BrushPresetDock::new(globals));
            registry.register(ColorSelectorDock::new(globals));
            registry.register(ToolOptionsDock::new(globals));
        }

        registry.register(LandingDock::new());
        registry.register(LayersDock::default());
        registry.register(ToolBoxDock::new());

        app.add_global_instance(registry);
    }
}
