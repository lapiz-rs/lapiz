use iced_core::{Element, Theme};
use lapiz_assets::AssetAppExt;
use lapiz_render::texture::Image;
use lapiz_runtime::{Application, plugin::Plugin};

use crate::{
    graph::{
        function::{ASSET_GRAPH_FUNCTION_STORAGE, GraphFunctionStorage},
        texture::{ASSET_GRAPH_TEXTURE_STORAGE, GraphTextureStorage},
    },
    save::{SerializableGraphFunction, SerializableGraphFunctionSerializer},
};

pub mod editor;
pub mod graph;
pub mod save;
pub mod wgsl_std;

pub type GraphSerializer<'a> = toml::Serializer<'a>;
pub type GraphDeserializer<'a> = toml::de::Deserializer<'a>;
pub type GraphRenderer = iced_wgpu::Renderer;
pub type GraphTheme = Theme;
pub type GraphElement<'a, Message> = Element<'a, Message, GraphTheme, GraphRenderer>;

lapiz_i18n::define_i18n!("shader_graph");

pub struct ShaderGraphPlugin;

impl Plugin for ShaderGraphPlugin {
    fn build(&self, app: &mut Application) {
        crate::i18n::init();
        app.runtime_mut()
            .services_mut()
            .add_asset_serializer::<SerializableGraphFunctionSerializer>();
    }

    fn finish(&self, app: &mut Application) {
        let runtime = app.runtime();
        let services = runtime.services();
        let assets = services.assets();

        ASSET_GRAPH_TEXTURE_STORAGE
            .store(GraphTextureStorage::new(assets.all_handles_of::<Image>().unwrap()).into());
        ASSET_GRAPH_FUNCTION_STORAGE.store(
            GraphFunctionStorage::new(
                ASSET_GRAPH_TEXTURE_STORAGE.clone(),
                ASSET_GRAPH_FUNCTION_STORAGE.clone(),
                assets
                    .all_handles_of::<SerializableGraphFunction>()
                    .unwrap(),
            )
            .into(),
        );
    }
}
