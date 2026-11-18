use std::{any::Any, collections::HashMap, marker::PhantomData, path::Path, sync::Arc};

use anyhow::{Context as _, Result};
use iced_core::{Element, Theme};
use iced_futures::Subscription;
use iced_runtime::Task;
use lapiz_assets::{
    asset::{Asset, AssetId, ErasedAsset, UntypedAssetId},
    bundle::BundleId,
    loader::AssetSerializer,
    store::AssetRegistry,
};
use lapiz_runtime::{
    Renderer,
    global::{Global, Globals},
};
use lapiz_utils::{log_err::LogErr as _, wrapper};
use uuid::Uuid;

use crate::asset::BrushPreset;

pub mod lapiz_engine;

wrapper! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub BrushEngineId : &'static str
}

wrapper! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub BrushEngineSessionId : Uuid
}

pub trait BrushEngine: 'static {
    type Message: Send + 'static;
    type Brush: Asset;
    type BrushSerializer: AssetSerializer<Asset = Self::Brush>;

    fn id() -> BrushEngineId;
    fn label() -> String;
    fn query_brushes(assets: &AssetRegistry) -> Result<Vec<BrushPresetEntry<Self::Brush>>>;
    fn load_brush(
        brush: &Self::Brush,
        asset_id: Option<AssetId<Self::Brush>>,
        assets: &AssetRegistry,
    ) -> Result<Self>
    where
        Self: Sized;
    fn load_brush_from_assets(asset: AssetId<Self::Brush>, assets: &AssetRegistry) -> Result<Self>
    where
        Self: Sized,
    {
        let brush = Self::get_brush_from_assets(asset, assets)?;
        Self::load_brush(&brush, Some(asset), assets)
    }
    fn new_brush(assets: &AssetRegistry) -> Result<Self::Brush>;
    fn add_brush_to_assets(
        brush: Self::Brush,
        bundle_id: BundleId,
        path: &Path,
        assets: &AssetRegistry,
    ) -> Result<AssetId<Self::Brush>> {
        assets.add_asset(bundle_id, path, Arc::new(brush))
    }
    fn get_brush_from_assets(
        asset: AssetId<Self::Brush>,
        assets: &AssetRegistry,
    ) -> Result<Arc<Self::Brush>> {
        assets.handle(asset)?.get()
    }
    fn brush_asset_id(&self) -> Option<AssetId<Self::Brush>>;
    fn compile_brush(&self, globals: &Globals) -> Result<BrushPreset>;
    fn editor_view<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer>;
    fn editor_update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> Task<Self::Message>;
    fn editor_subscription(&self, _globals: &Globals) -> Subscription<Self::Message> {
        Subscription::none()
    }
}

pub trait ErasedBrushEngine: 'static {
    fn id(&self) -> BrushEngineId;
    fn label(&self) -> String;
    fn brush_asset_id(&self) -> Option<UntypedAssetId>;
    fn brush_file_extension(&self) -> &'static str;
    fn compile_brush(&self, globals: &Globals) -> Result<BrushPreset>;
    fn editor_view<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, Renderer>;
    fn editor_update(
        &mut self,
        message: Box<dyn Any + Send>,
        globals: &mut Globals,
    ) -> Task<Box<dyn Any + Send>>;
    fn editor_subscription(&self, globals: &Globals) -> Subscription<Box<dyn Any + Send>>;
}

impl<T: BrushEngine> ErasedBrushEngine for T {
    fn id(&self) -> BrushEngineId {
        T::id()
    }

    fn label(&self) -> String {
        T::label()
    }

    fn brush_asset_id(&self) -> Option<UntypedAssetId> {
        self.brush_asset_id().map(|id| id.into_untyped())
    }

    fn brush_file_extension(&self) -> &'static str {
        T::BrushSerializer::file_extension()
    }

    fn compile_brush(&self, globals: &Globals) -> Result<BrushPreset> {
        BrushEngine::compile_brush(self, globals)
    }

    fn editor_view<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, Renderer> {
        BrushEngine::editor_view(self, globals)
            .map(|message| Box::new(message) as Box<dyn Any + Send>)
    }

    fn editor_update(
        &mut self,
        message: Box<dyn Any + Send>,
        globals: &mut Globals,
    ) -> Task<Box<dyn Any + Send>> {
        let message = *message
            .downcast::<T::Message>()
            .expect("invalid brush editor message type");
        BrushEngine::editor_update(self, message, globals)
            .map(|message| Box::new(message) as Box<dyn Any + Send>)
    }

    fn editor_subscription(&self, globals: &Globals) -> Subscription<Box<dyn Any + Send>> {
        BrushEngine::editor_subscription(self, globals)
            .map(|message| Box::new(message) as Box<dyn Any + Send>)
    }
}

pub trait BrushEngineFactory {
    fn id(&self) -> BrushEngineId;
    fn label(&self) -> String;
    fn brush_file_extension(&self) -> &'static str;
    fn query_brushes(&self, assets: &AssetRegistry) -> Result<Vec<UntypedBrushPresetEntry>>;
    fn load_brush(
        &self,
        brush: &dyn ErasedAsset,
        asset_id: Option<UntypedAssetId>,
        assets: &AssetRegistry,
    ) -> Result<Box<dyn ErasedBrushEngine>>;
    fn load_brush_from_assets(
        &self,
        asset: UntypedAssetId,
        assets: &AssetRegistry,
    ) -> Result<Box<dyn ErasedBrushEngine>>;
    fn new_brush(&self, assets: &AssetRegistry) -> Result<Box<dyn ErasedAsset>>;
    fn add_brush_to_assets(
        &self,
        brush: Box<dyn ErasedAsset>,
        bundle_id: BundleId,
        path: &Path,
        assets: &AssetRegistry,
    ) -> Result<UntypedAssetId>;
}

pub struct BrushEngineFactoryInstance<E: BrushEngine> {
    _marker: PhantomData<E>,
}

impl<E: BrushEngine> Default for BrushEngineFactoryInstance<E> {
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<E: BrushEngine> BrushEngineFactory for BrushEngineFactoryInstance<E> {
    fn id(&self) -> BrushEngineId {
        E::id()
    }

    fn label(&self) -> String {
        E::label()
    }

    fn brush_file_extension(&self) -> &'static str {
        E::BrushSerializer::file_extension()
    }

    fn query_brushes(&self, assets: &AssetRegistry) -> Result<Vec<UntypedBrushPresetEntry>> {
        E::query_brushes(assets).map(|brushes| {
            brushes
                .into_iter()
                .map(|entry| UntypedBrushPresetEntry {
                    id: entry.id.into_untyped(),
                    name: entry.name,
                })
                .collect()
        })
    }

    fn load_brush(
        &self,
        brush: &dyn ErasedAsset,
        asset_id: Option<UntypedAssetId>,
        assets: &AssetRegistry,
    ) -> Result<Box<dyn ErasedBrushEngine>> {
        let brush = brush
            .downcast_ref::<E::Brush>()
            .ok_or_else(|| anyhow::anyhow!("invalid brush type"))?;
        E::load_brush(brush, asset_id.map(|id| id.into_typed()), assets)
            .map(|engine| Box::new(engine) as Box<_>)
    }

    fn load_brush_from_assets(
        &self,
        asset: UntypedAssetId,
        assets: &AssetRegistry,
    ) -> Result<Box<dyn ErasedBrushEngine>> {
        E::load_brush_from_assets(asset.into_typed(), assets)
            .map(|engine| Box::new(engine) as Box<_>)
    }

    fn new_brush(&self, assets: &AssetRegistry) -> Result<Box<dyn ErasedAsset>> {
        E::new_brush(assets).map(|brush| Box::new(brush) as Box<_>)
    }

    fn add_brush_to_assets(
        &self,
        brush: Box<dyn ErasedAsset>,
        bundle_id: BundleId,
        path: &Path,
        assets: &AssetRegistry,
    ) -> Result<UntypedAssetId> {
        let brush = brush
            .downcast::<E::Brush>()
            .map_err(|_| anyhow::anyhow!("invalid brush type"))?;
        E::add_brush_to_assets(*brush, bundle_id, path, assets).map(|id| id.into_untyped())
    }
}

#[derive(Default)]
pub struct BrushEngineRegistry {
    pub factories: HashMap<BrushEngineId, Box<dyn BrushEngineFactory>>,
}

impl Global for BrushEngineRegistry {}

impl BrushEngineRegistry {
    pub fn register<T: BrushEngine>(&mut self) {
        self.factories.insert(
            T::id(),
            Box::new(BrushEngineFactoryInstance::<T>::default()),
        );
    }

    pub fn get(&self, engine_id: &BrushEngineId) -> Result<&dyn BrushEngineFactory> {
        self.factories
            .get(engine_id)
            .map(|b| b.as_ref())
            .context("Brush engine not found")
    }

    pub fn all_brushes(
        &self,
        assets: &AssetRegistry,
    ) -> Vec<(BrushEngineId, UntypedBrushPresetEntry)> {
        let mut brushes = self
            .factories
            .iter()
            .flat_map(|(engine_id, factory)| {
                let brushes = factory
                    .query_brushes(assets)
                    .logged_err()
                    .unwrap_or_default();

                brushes.into_iter().map(|entry| (*engine_id, entry))
            })
            .collect::<Vec<_>>();
        brushes.sort_by(|(_, brush_a), (_, brush_b)| brush_a.name.cmp(&brush_b.name));
        brushes
    }
}

pub trait BrushEngineRegistryExt {
    fn brush_engine_registry(&self) -> &BrushEngineRegistry;
    fn brush_engine_registry_mut(&mut self) -> &mut BrushEngineRegistry;
}

impl BrushEngineRegistryExt for Globals {
    fn brush_engine_registry(&self) -> &BrushEngineRegistry {
        self.global::<BrushEngineRegistry>()
    }

    fn brush_engine_registry_mut(&mut self) -> &mut BrushEngineRegistry {
        self.global_mut::<BrushEngineRegistry>()
    }
}

pub struct BrushPresetEntry<Brush: Asset> {
    pub id: AssetId<Brush>,
    pub name: String,
}

pub struct UntypedBrushPresetEntry {
    pub id: UntypedAssetId,
    pub name: String,
}
