use lapiz_assets::AssetAppExt as _;
use lapiz_runtime::{Runtime, plugin::Plugin};

use crate::asset::EffectAssetSerializer;

lapiz_i18n::define_i18n!("effect");

pub mod asset;
pub mod editor;
pub mod instance;
pub mod nodes;
pub mod render;

pub fn init_i18n() {
    i18n::init();
}

/// Registers effect localization and the [`EffectAssetSerializer`] with the [`Runtime`].
pub struct EffectPlugin;

impl Plugin for EffectPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.globals_mut()
            .add_asset_serializer::<EffectAssetSerializer>();
    }
}
