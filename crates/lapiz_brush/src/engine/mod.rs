use std::{any::Any, collections::HashMap};

use anyhow::{Context as _, Result};
use iced_core::{Element, Theme};
use iced_futures::Subscription;
use iced_runtime::Task;
use lapiz_assets::{
    asset::{Asset, AssetId, UntypedAssetId},
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
    pub BrushEditorEngineId : &'static str
}

wrapper! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub BrushEditorSessionId : Uuid
}

pub trait BrushEditorEngine: 'static {
    type Message: Send + 'static;
    type Brush: Asset;

    fn id() -> BrushEditorEngineId;
    fn label() -> String;
    fn query_brushes(&self, assets: &AssetRegistry) -> Result<Vec<BrushPresetEntry<Self::Brush>>>;
    fn load_brush(
        &mut self,
        asset: AssetId<Self::Brush>,
        assets: &AssetRegistry,
        globals: &mut Globals,
    ) -> Result<()>;
    fn new_brush(
        &mut self,
        assets: &AssetRegistry,
        globals: &mut Globals,
    ) -> Result<AssetId<Self::Brush>>;
    fn compile_brush(&self, globals: &Globals) -> Result<BrushPreset>;
    fn view<'a>(&'a self, globals: &'a Globals) -> Element<'a, Self::Message, Theme, Renderer>;
    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message>;
    fn subscription(&self, _globals: &Globals) -> Subscription<Self::Message> {
        Subscription::none()
    }
}

pub trait ErasedBrushEditorEngine: 'static {
    fn query_brushes(&self, assets: &AssetRegistry) -> Result<Vec<UntypedBrushPresetEntry>>;
    fn load_brush(
        &mut self,
        asset: UntypedAssetId,
        assets: &AssetRegistry,
        globals: &mut Globals,
    ) -> Result<()>;
    fn new_brush(
        &mut self,
        assets: &AssetRegistry,
        globals: &mut Globals,
    ) -> Result<UntypedAssetId>;
    fn compile_brush(&self, globals: &Globals) -> Result<BrushPreset>;
    fn view<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, Renderer>;
    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        globals: &mut Globals,
    ) -> Task<Box<dyn Any + Send>>;
    fn subscription(&self, globals: &Globals) -> Subscription<Box<dyn Any + Send>>;
}

impl<T: BrushEditorEngine> ErasedBrushEditorEngine for T {
    fn query_brushes(&self, assets: &AssetRegistry) -> Result<Vec<UntypedBrushPresetEntry>> {
        BrushEditorEngine::query_brushes(self, assets).map(|entries| {
            entries
                .into_iter()
                .map(|entry| UntypedBrushPresetEntry {
                    id: entry.id.into_untyped(),
                    name: entry.name,
                })
                .collect()
        })
    }

    fn load_brush(
        &mut self,
        asset: UntypedAssetId,
        assets: &AssetRegistry,
        globals: &mut Globals,
    ) -> Result<()> {
        BrushEditorEngine::load_brush(self, asset.into_typed(), assets, globals)
    }

    fn new_brush(
        &mut self,
        assets: &AssetRegistry,
        globals: &mut Globals,
    ) -> Result<UntypedAssetId> {
        BrushEditorEngine::new_brush(self, assets, globals).map(|id| id.into_untyped())
    }

    fn compile_brush(&self, globals: &Globals) -> Result<BrushPreset> {
        BrushEditorEngine::compile_brush(self, globals)
    }

    fn view<'a>(
        &'a self,
        globals: &'a Globals,
    ) -> Element<'a, Box<dyn Any + Send>, Theme, Renderer> {
        BrushEditorEngine::view(self, globals)
            .map(|message| Box::new(message) as Box<dyn Any + Send>)
    }

    fn update(
        &mut self,
        message: Box<dyn Any + Send>,
        globals: &mut Globals,
    ) -> Task<Box<dyn Any + Send>> {
        let message = *message
            .downcast::<T::Message>()
            .expect("invalid brush editor message type");
        BrushEditorEngine::update(self, message, globals)
            .map(|message| Box::new(message) as Box<dyn Any + Send>)
    }

    fn subscription(&self, globals: &Globals) -> Subscription<Box<dyn Any + Send>> {
        BrushEditorEngine::subscription(self, globals)
            .map(|message| Box::new(message) as Box<dyn Any + Send>)
    }
}

#[derive(Default)]
pub struct BrushEngineEditorRegistry {
    pub factories: HashMap<BrushEditorEngineId, BrushEditorEngineRegistration>,
}

pub struct BrushEditorEngineRegistration {
    pub label: String,
    pub create: Box<dyn Fn() -> Box<dyn ErasedBrushEditorEngine>>,
}

impl Global for BrushEngineEditorRegistry {}

impl BrushEngineEditorRegistry {
    pub fn register<T: BrushEditorEngine + Default>(&mut self) {
        self.factories.insert(
            T::id(),
            BrushEditorEngineRegistration {
                label: T::label(),
                create: Box::new(|| Box::new(T::default())),
            },
        );
    }

    pub fn create(&self, id: BrushEditorEngineId) -> Result<Box<dyn ErasedBrushEditorEngine>> {
        let factory = self
            .factories
            .get(&id)
            .with_context(|| format!("Brush editor engine {} must be registered", id.0))?;
        Ok((factory.create)())
    }

    pub fn all_brushes(
        &self,
        assets: &AssetRegistry,
    ) -> Vec<(BrushEditorEngineId, UntypedBrushPresetEntry)> {
        let mut brushes = self
            .factories
            .iter()
            .flat_map(|(engine_id, factory)| {
                let brushes = (factory.create)()
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

pub struct BrushPresetEntry<Brush: Asset> {
    pub id: AssetId<Brush>,
    pub name: String,
}

pub struct UntypedBrushPresetEntry {
    pub id: UntypedAssetId,
    pub name: String,
}
