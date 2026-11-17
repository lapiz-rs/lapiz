use lapiz_runtime::{Runtime, plugin::Plugin};

lapiz_i18n::define_i18n!("asset_tag_editor");

mod model;
mod view;

pub const ASSET_TAG_EDITOR_VIEW_ID: &str = "asset_tag_editor";

pub struct AssetTagEditorPlugin;

impl Plugin for AssetTagEditorPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.register_view::<view::AssetTagEditor>();
    }
}
