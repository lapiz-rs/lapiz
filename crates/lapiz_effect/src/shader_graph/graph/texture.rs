use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, LazyLock},
};

use arc_swap::ArcSwap;
use indexmap::IndexMap;
use lapiz_assets::asset::{AssetHandle, AssetId};
use lapiz_render::texture::Image;
use lapiz_utils::wrapper;
use parse_display::Display;
use serde::{Deserialize, Serialize};
wrapper! {
    /// Identifier for a texture asset referenced in a shader graph, or [`TextureId::NULL`] when unset.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    #[display("{0:?}")]
    pub TextureId : Option<AssetId<Image>>
}

impl TextureId {
    // Null texture should have a default fallback, so they're also valid.
    pub const NULL: Self = Self(None);
}

impl Default for TextureId {
    fn default() -> Self {
        Self::NULL
    }
}

/// Metadata and asset handle for an [`Image`] available to shader graph texture selectors.
#[derive(Clone)]
pub struct TextureObject {
    pub external_id: AssetId<Image>,
    pub name: String,
    pub handle: AssetHandle<Image>,
}

impl PartialEq for TextureObject {
    fn eq(&self, other: &Self) -> bool {
        self.external_id == other.external_id
    }
}

impl fmt::Display for TextureObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// A lookup table mapping [`TextureId`]s to their loaded [`TextureObject`] handles.
#[derive(Default, Clone)]
pub struct GraphTextureStorage {
    textures: HashMap<TextureId, TextureObject>,
}

pub static ASSET_GRAPH_TEXTURE_STORAGE: LazyLock<SharedGraphTextureStorage> =
    LazyLock::new(Default::default);

pub type SharedGraphTextureStorage = Arc<ArcSwap<GraphTextureStorage>>;

impl GraphTextureStorage {
    pub fn new(textures: Vec<AssetHandle<Image>>) -> Self {
        let textures = textures
            .into_iter()
            .map(|t| {
                let external_id = t.id();
                let id = TextureId(Some(external_id));
                let name = t.get().unwrap().metadata.name.clone();
                let object = TextureObject {
                    external_id,
                    name,
                    handle: t.clone(),
                };
                (id, object)
            })
            .collect();
        Self { textures }
    }

    pub fn get(&self, id: &TextureId) -> Option<&TextureObject> {
        self.textures.get(id)
    }

    pub fn all(&self) -> &HashMap<TextureId, TextureObject> {
        &self.textures
    }
}

/// Tracks which [`TextureId`]s are referenced during shader compilation and assigns local binding indices.
#[derive(Default)]
pub struct GraphTextureUsageRecorder {
    inner: IndexMap<TextureId, u32>,
}

impl GraphTextureUsageRecorder {
    pub fn use_texture(&mut self, id: TextureId) -> u32 {
        let e = self.inner.entry(id);
        let local_index = e.index() as u32;
        e.and_modify(|index| *index += 1).or_insert(0);
        local_index
    }

    pub fn used_textures_ordered(&self) -> Vec<TextureId> {
        self.inner.keys().cloned().collect()
    }
}
