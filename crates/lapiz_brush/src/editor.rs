use std::{any::Any, collections::HashMap, path::Path, sync::Arc};

use anyhow::{Context as _, Result};
use iced_core::{Element, Event, Length, Size, Theme, window};
use iced_futures::{Subscription, event::listen_with};
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use lapiz_assets::{AssetAppExt as _, asset::UntypedAssetId};
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer,
    global::Globals,
    windows::{WindowView, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{button, column, flex::Flex, label, panel, row, scrollable, title_bar};
use uuid::Uuid;

use crate::{
    engine::{
        BrushEngine as _, BrushEngineId, BrushEngineRegistry, BrushEngineRegistryExt,
        BrushEngineSessionId, ErasedBrushEngine, UntypedBrushPresetEntry,
        lapiz_engine::LapizBrushEngine,
    },
    tool::CurrentBrushPresetExt as _,
};

/// Standalone window for registered brush engine editors.
pub struct BrushEditor {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    brushes: Vec<(BrushEngineId, UntypedBrushPresetEntry)>,
    selected: Option<SelectedBrushEditor>,
}

struct SelectedBrushEditor {
    index: usize,
    asset_id: UntypedAssetId,
    session: BrushEngineSessionId,
    engine: Box<dyn ErasedBrushEngine>,
}

pub enum BrushEditorMessage {
    SelectBrush(usize),
    NewBrush(BrushEngineId),
    EngineMessage {
        session: BrushEngineSessionId,
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

        let engines = globals.global::<BrushEngineRegistry>();
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

        let mut create_buttons = column![].gap(4.0);
        let mut additional_engines = globals
            .global::<BrushEngineRegistry>()
            .factories
            .iter()
            .collect::<Vec<_>>();
        additional_engines.sort_by_key(|(id, _)| id.0);
        for (id, registration) in additional_engines {
            create_buttons = create_buttons.push(
                button(label(t!(
                    "new_engine_brush",
                    engine = registration.label().clone()
                )))
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
                    .editor_view(globals)
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
                let task = selected.engine.editor_update(message, globals);
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
                    .editor_subscription(globals)
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
        let factory = globals.brush_engine_registry().get(engine_id)?;
        let engine = factory.load_brush_from_assets(entry.id, globals.assets())?;

        self.selected = Some(SelectedBrushEditor {
            index,
            asset_id: entry.id,
            session: BrushEngineSessionId::new(Uuid::new_v4()),
            engine,
        });
        Ok(())
    }

    fn new_brush(&mut self, engine_id: BrushEngineId, globals: &mut Globals) -> Result<()> {
        let factory = globals.brush_engine_registry().get(&engine_id)?;
        let assets = globals.assets().clone();
        let new_brush = factory.new_brush(&assets)?;
        let index = self.brushes.len();
        // TODO Replace this with asset event system;
        // self.brushes.push((engine_id, entry));

        let bundle = assets
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .map(|bundle| bundle.metadata().bundle_id)
            .context("A writable asset bundle is required for a new brush preset")?;
        let path = format!(
            "unnamed_{}.{}",
            Uuid::new_v4(),
            factory.brush_file_extension()
        );
        let asset_id = factory.add_brush_to_assets(new_brush, bundle, Path::new(&path), &assets)?;
        let engine = factory.load_brush_from_assets(asset_id, &assets)?;

        self.selected = Some(SelectedBrushEditor {
            index,
            asset_id,
            session: BrushEngineSessionId::new(Uuid::new_v4()),
            engine,
        });
        Ok(())
    }

    fn refresh_brush_names(&mut self, globals: &Globals) {
        let Some(selected) = self.selected.as_ref() else {
            return;
        };
        let factory = globals
            .brush_engine_registry()
            .get(&selected.engine.id())
            .unwrap();
        let Ok(entries) = factory.query_brushes(globals.assets()).logged_err() else {
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
