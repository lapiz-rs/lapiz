use lapiz_assets::AssetAppExt as _;
use lapiz_brush::editor::BrushEngineEditorRegistry;
use lapiz_runtime::{Runtime, plugin::Plugin};

use crate::{asset::PhotoshopBrushSerializer, editor::PhotoshopBrushEditor};

pub mod asset;
pub mod convert;
pub mod editor;
pub mod parameter;

lapiz_i18n::define_i18n!("photoshop_brush");

pub struct PhotoshopBrushPlugin;

impl Plugin for PhotoshopBrushPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();

        let globals = app.globals_mut();
        globals.add_asset_serializer::<PhotoshopBrushSerializer>();
        globals
            .global_mut::<BrushEngineEditorRegistry>()
            .register::<PhotoshopBrushEditor>();
    }
}
