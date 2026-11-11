use iced_core::{Element, Theme};
use lapiz_assets::AssetAppExt as _;
use lapiz_render::texture::Image;
use lapiz_runtime::{Runtime, plugin::Plugin};
use toml::{Serializer, de::Deserializer};

use crate::{
    graph::texture::{ASSET_GRAPH_TEXTURE_STORAGE, GraphTextureStorage},
    save::SerializableGraphFunctionSerializer,
};

pub mod editor;
pub mod graph;
pub mod save;
pub mod wgsl_std;

pub type GraphSerializer<'a> = Serializer<'a>;
pub type GraphDeserializer<'a> = Deserializer<'a>;
pub type GraphRenderer = lapiz_runtime::Renderer;
pub type GraphTheme = Theme;
pub type GraphElement<'a, Message> = Element<'a, Message, GraphTheme, GraphRenderer>;

lapiz_i18n::define_i18n!("shader_graph");

pub fn init_i18n() {
    i18n::init();
}

/// Registers shader graph localization, function asset serialization, and global texture storage with the [`Runtime`].
pub struct ShaderGraphPlugin;

impl Plugin for ShaderGraphPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.globals_mut()
            .add_asset_serializer::<SerializableGraphFunctionSerializer>();
    }

    fn finish(&self, app: &mut Runtime) {
        let globals = app.globals();
        let assets = globals.assets();

        ASSET_GRAPH_TEXTURE_STORAGE
            .store(GraphTextureStorage::new(assets.all_handles_of::<Image>().unwrap()).into());
    }
}
