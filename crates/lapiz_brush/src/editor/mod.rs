use std::{any::Any, collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result};
use iced_core::{Element, Event, Length, Size, Theme, window};
use iced_futures::{Subscription, event::listen_with};
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use lapiz_assets::{
    AssetAppExt as _,
    asset::{Asset, AssetId, UntypedAssetId},
    store::AssetRegistry,
};
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer,
    global::{Global, Globals},
    windows::{WindowView, WindowViewId},
};
use lapiz_utils::{log_err::LogErr as _, wrapper};
use lapiz_widgets::{button, column, flex::Flex, label, panel, row, scrollable, title_bar};
use uuid::Uuid;

use crate::{
    asset::BrushPreset, editor::lapiz_engine::LapizBrushEngineEditor,
    tool::CurrentBrushPresetExt as _,
};

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

/// Standalone window for registered brush engine editors.
pub struct BrushEditor {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    brushes: Vec<(BrushEditorEngineId, UntypedBrushPresetEntry)>,
    selected: Option<SelectedBrushEditor>,
}

struct SelectedBrushEditor {
    index: usize,
    asset_id: UntypedAssetId,
    session: BrushEditorSessionId,
    engine: Box<dyn ErasedBrushEditorEngine>,
}

pub enum BrushEditorMessage {
    SelectBrush(usize),
    NewBrush(BrushEditorEngineId),
    EngineMessage {
        session: BrushEditorSessionId,
        message: Box<dyn Any + Send>,
    },
    WindowUnfocused(window::Id),
    Close,
    Maximize,
    Minimize,
    Drag,
}

impl WindowView for BrushEditor {
    type Message = BrushEditorMessage;
    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new("brush_editor")
    }

    fn boot(
        _params: Option<Self::BootParams>,
        globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)> {
        let (main_window, open) = open(window::Settings {
            decorations: false,
            size: Size {
                width: 1280.0,
                height: 800.0,
            },
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific {
                corner_preference: window::settings::platform::CornerPreference::DoNotRound,
                ..Default::default()
            },
            ..Default::default()
        });

        let engines = globals.global::<BrushEngineEditorRegistry>();
        let assets = globals.assets();
        let brushes = engines.all_brushes(assets);

        Ok((
            Self {
                windows: [main_window].into(),
                main_window,
                brushes,
                selected: None,
            },
            open.discard(),
        ))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        globals: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let titlebar = title_bar(label(t!("brush_editor_title")).window_title())
            .on_close(BrushEditorMessage::Close)
            .on_maximize(BrushEditorMessage::Maximize)
            .on_minimize(BrushEditorMessage::Minimize)
            .on_drag(BrushEditorMessage::Drag);

        let brush_list = self
            .brushes
            .iter()
            .enumerate()
            .map(|(index, (_, entry))| {
                button(label(&entry.name))
                    .width(Length::Fill)
                    .activated(
                        self.selected
                            .as_ref()
                            .is_some_and(|selected| selected.index == index),
                    )
                    .on_press(BrushEditorMessage::SelectBrush(index))
                    .into()
            })
            .collect::<Vec<_>>();

        let mut create_buttons = column![
            button(label(t!("new_brush")))
                .on_press(BrushEditorMessage::NewBrush(LapizBrushEngineEditor::id())),
        ]
        .gap(4.0);
        let mut additional_engines = globals
            .global::<BrushEngineEditorRegistry>()
            .factories
            .iter()
            .filter(|(id, _)| **id != LapizBrushEngineEditor::id())
            .collect::<Vec<_>>();
        additional_engines.sort_by_key(|(id, _)| id.0);
        for (id, registration) in additional_engines {
            create_buttons = create_buttons.push(
                button(label(t!("new_engine_brush", engine = &registration.label)))
                    .on_press(BrushEditorMessage::NewBrush(*id)),
            );
        }

        let sidebar = panel(
            column![
                create_buttons,
                label(t!("brushes")).strong(),
                scrollable(Flex::column(brush_list).gap(2.0))
                    .width(Length::Fill)
                    .height(Length::Fill),
            ]
            .gap(6.0),
        )
        .padding(8)
        .width(220);

        let view = match self.selected.as_ref() {
            Some(selected) => {
                let session = selected.session;
                selected
                    .engine
                    .view(globals)
                    .map(move |message| BrushEditorMessage::EngineMessage { session, message })
            }
            None => row![label(t!("select_a_brush")).muted(), label("")]
                .gap(8.0)
                .into(),
        };

        panel(column![
            titlebar,
            row![sidebar, view].gap(8.0).height(Length::Fill)
        ])
    }

    fn update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            BrushEditorMessage::SelectBrush(index) => {
                if let Err(error) = self.select_brush(index, globals) {
                    log::error!("Failed to load brush preset: {error:#}");
                }
                Task::none()
            }
            BrushEditorMessage::NewBrush(engine_id) => {
                if let Err(error) = self.new_brush(engine_id, globals) {
                    log::error!("Failed to create brush preset: {error:#}");
                }
                Task::none()
            }
            BrushEditorMessage::EngineMessage { session, message } => {
                let Some(selected) = self
                    .selected
                    .as_mut()
                    .filter(|selected| selected.session == session)
                else {
                    return Task::none();
                };
                let task = selected.engine.update(message, globals);
                self.refresh_brush_names(globals);
                task.map(move |message| BrushEditorMessage::EngineMessage { session, message })
            }
            BrushEditorMessage::WindowUnfocused(window) => {
                let current_brush = globals
                    .current_brush_preset()
                    .and_then(|b| b.instance().asset_id());

                // recompiles the brush preset if it's currently selected
                if window == self.main_window
                    && let Some(selected) = self.selected.as_ref()
                    && current_brush == Some(selected.asset_id)
                {
                    let result = selected.engine.compile_brush(globals).and_then(|preset| {
                        globals.set_current_brush_preset(&preset, Some(selected.asset_id))
                    });
                    if let Err(error) = result {
                        log::error!("Failed to apply brush preset on editor focus loss: {error:#}");
                    }
                }
                Task::none()
            }
            BrushEditorMessage::Close => close(self.main_window),
            BrushEditorMessage::Maximize => toggle_maximize(self.main_window),
            BrushEditorMessage::Minimize => minimize(self.main_window, true),
            BrushEditorMessage::Drag => drag(self.main_window),
        }
    }

    fn subscription(&self, globals: &Globals) -> Subscription<Self::Message> {
        let external = listen_with(|event, _status, window| match event {
            Event::Window(window::Event::Unfocused) => {
                Some(BrushEditorMessage::WindowUnfocused(window))
            }
            _ => None,
        });
        let engine = match self.selected.as_ref() {
            Some(selected) => {
                let session = selected.session;
                selected
                    .engine
                    .subscription(globals)
                    .with(session)
                    .map(|(session, message)| BrushEditorMessage::EngineMessage {
                        session,
                        message,
                    })
            }
            None => Subscription::none(),
        };

        Subscription::batch([external, engine])
    }

    fn close(self, _: &mut Globals) -> Task<()> {
        close(self.main_window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.main_window)
    }
}

impl BrushEditor {
    fn select_brush(&mut self, index: usize, globals: &mut Globals) -> Result<()> {
        // recompiles the previously selected brush preset if it's currently selected
        if let Some(old_selected) = self.selected.take()
            && let Some(current_brush) = globals
                .current_brush_preset()
                .and_then(|b| b.instance().asset_id())
            && old_selected.asset_id == current_brush
        {
            old_selected
                .engine
                .compile_brush(globals)
                .and_then(|preset| {
                    globals.set_current_brush_preset(&preset, Some(old_selected.asset_id))
                })
                .log_err();
        }

        let Some((engine_id, entry)) = self.brushes.get(index) else {
            return Ok(());
        };
        let mut engine = globals
            .global::<BrushEngineEditorRegistry>()
            .create(*engine_id)?;
        let assets = globals.assets().clone();
        engine.load_brush(entry.id, &assets, globals)?;

        self.selected = Some(SelectedBrushEditor {
            index,
            asset_id: entry.id,
            session: BrushEditorSessionId::new(Uuid::new_v4()),
            engine,
        });
        Ok(())
    }

    fn new_brush(&mut self, engine_id: BrushEditorEngineId, globals: &mut Globals) -> Result<()> {
        let mut engine = globals
            .global::<BrushEngineEditorRegistry>()
            .create(engine_id)?;
        let assets = globals.assets().clone();
        let id = engine.new_brush(&assets, globals)?;
        let entry = engine
            .query_brushes(&assets)?
            .into_iter()
            .find(|entry| entry.id == id)
            .context("The new brush must appear in its engine's brush list")?;
        let index = self.brushes.len();
        self.brushes.push((engine_id, entry));
        self.selected = Some(SelectedBrushEditor {
            index,
            asset_id: id,
            session: BrushEditorSessionId::new(Uuid::new_v4()),
            engine,
        });
        Ok(())
    }

    fn refresh_brush_names(&mut self, globals: &Globals) {
        let Some(selected) = self.selected.as_ref() else {
            return;
        };
        let Ok(entries) = selected.engine.query_brushes(globals.assets()).logged_err() else {
            return;
        };
        let engine_id = self.brushes[selected.index].0;
        let mut names = entries
            .into_iter()
            .map(|entry| (entry.id, entry.name))
            .collect::<HashMap<_, _>>();
        for (id, entry) in &mut self.brushes {
            if *id == engine_id
                && let Some(name) = names.remove(&entry.id)
            {
                entry.name = name;
            }
        }
    }
}
