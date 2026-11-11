use lapiz_assets::AssetAppExt as _;
use lapiz_runtime::{Runtime, plugin::Plugin};

use crate::{asset::FilterPresetSerializer, editor::FilterEditor, panel::FilterPanel};

pub mod asset;
pub mod editor;
pub mod instance;
pub mod panel;
pub mod render;

lapiz_i18n::define_i18n!("filter");

/// Registers the filter editor and application panel views along with the `.lfp` [`FilterPreset`](crate::asset::FilterPreset) asset serializer.
pub struct FilterPlugin;

impl Plugin for FilterPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.register_view::<FilterPanel>()
            .register_view::<FilterEditor>();

        app.globals_mut()
            .add_asset_serializer::<FilterPresetSerializer>();
    }
}
