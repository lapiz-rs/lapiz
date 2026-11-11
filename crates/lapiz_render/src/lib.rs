wesl::wesl_pkg!(pub render);

use lapiz_assets::AssetAppExt as _;
use lapiz_runtime::{Runtime, plugin::Plugin};

use crate::{
    resources::{FullscreenVertex, GlobalSamplers},
    texture::ImageSerializer,
};

pub mod bind_group_entries;
pub mod bind_group_layout_entries;
pub mod buffer_vec;
pub mod owned_bind_group_entries;
pub mod readback;
pub mod render_context;
pub mod resources;
pub mod storage_buffer;
pub mod texture;
pub mod texture_atlas;
pub mod uniform_buffer;
pub mod util;
pub mod wesl_jit;

/// Runtime plugin that registers shared rendering globals ([`GlobalSamplers`], [`FullscreenVertex`]) and the [`ImageSerializer`] asset loader.
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut Runtime) {
        app.add_global::<GlobalSamplers>()
            .add_global::<FullscreenVertex>();

        app.globals_mut().add_asset_serializer::<ImageSerializer>();
    }
}
