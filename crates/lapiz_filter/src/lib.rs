use lapiz_assets::AssetAppExt;
use lapiz_runtime::{Application, plugin::Plugin};

use crate::asset::FilterPresetSerializer;

pub mod asset;
pub mod editor;
pub mod instance;
pub mod panel;
pub mod render;

lapiz_i18n::define_i18n!("filter");

pub struct FilterPlugin;

impl Plugin for FilterPlugin {
    fn build(&self, app: &mut Application) {
        crate::i18n::init();
        let mut runtime = app.runtime_mut();
        let services = runtime.services_mut();
        services.add_asset_serializer::<FilterPresetSerializer>();
    }
}
