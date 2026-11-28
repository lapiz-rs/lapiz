use std::{any::Any, path::Path, sync::Arc};

use anyhow::{Context as _, Result};
use iced_core::{Element, Event, Length, Size, Theme, window};
use iced_futures::{Subscription, event::listen_with};
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use lapiz_assets::{AssetAppExt as _, asset::UntypedAssetId, event::AssetEvent};
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer,
    global::Globals,
    windows::{WindowView, WindowViewId},
};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    button, column, flex::Flex, label, panel, row, scrollable, text_input, title_bar,
};
use uuid::Uuid;

use crate::{
    engine::{
        BrushEngineId, BrushEngineRegistry, BrushEngineRegistryExt as _, BrushEngineSessionId,
        ErasedBrushEngine, UntypedBrushPresetEntry,
    },
    tool::CurrentBrushPresetExt as _,
};

/// Standalone window for registered brush engine editors.
pub struct BrushEditor {
    windows: Arc<[window::Id]>,
    main_window: window::Id,
    brushes: Vec<(BrushEngineId, UntypedBrushPresetEntry)>,
    creating_brush: Option<(BrushEngineId, String)>,
    selected: Option<SelectedBrushEditor>,
}

struct SelectedBrushEditor {
    asset_id: UntypedAssetId,
    session: BrushEngineSessionId,
    engine: Box<dyn ErasedBrushEngine>,
}

pub enum BrushEditorMessage {
    SelectBrush(UntypedAssetId),
    ToggleNewBrush(BrushEngineId),
    NewBrushNameChanged(String),
    ConfirmNewBrush,
    CancelNewBrush,
    SaveBrush,
    AssetEvent(AssetEvent),
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

#[derive(Clone)]
enum NewBrushInputMessage {
    Changed(String),
    Submit,
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
                creating_brush: None,
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
            .map(|(_, entry)| {
                button(label(&entry.name))
                    .width(Length::Fill)
                    .activated(
                        self.selected
                            .as_ref()
                            .is_some_and(|selected| selected.asset_id == entry.id),
                    )
                    .on_press(BrushEditorMessage::SelectBrush(entry.id))
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
            let is_creating = self
                .creating_brush
                .as_ref()
                .is_some_and(|(creating_id, _)| creating_id == id);
            create_buttons = create_buttons.push(
                button(label(t!(
                    "new_engine_brush",
                    engine = registration.label().clone()
                )))
                .width(Length::Fill)
                .activated(is_creating)
                .on_press(BrushEditorMessage::ToggleNewBrush(*id)),
            );
            if let Some((creating_id, name)) = self.creating_brush.as_ref()
                && creating_id == id
            {
                let can_confirm = sanitize_asset_file_stem(name).is_some();
                let name_input = Element::from(
                    text_input(&t!("name"), name)
                        .compact()
                        .width(Length::Fill)
                        .on_input(NewBrushInputMessage::Changed)
                        .on_submit_maybe(can_confirm.then_some(NewBrushInputMessage::Submit)),
                )
                .map(|msg| match msg {
                    NewBrushInputMessage::Changed(name) => {
                        BrushEditorMessage::NewBrushNameChanged(name)
                    }
                    NewBrushInputMessage::Submit => BrushEditorMessage::ConfirmNewBrush,
                });
                let create_box = panel(
                    column![
                        name_input,
                        row![
                            button(label(t!("create")))
                                .primary()
                                .width(Length::Fill)
                                .on_press_maybe(
                                    can_confirm.then_some(BrushEditorMessage::ConfirmNewBrush),
                                ),
                            button(label(t!("cancel")))
                                .width(Length::Fill)
                                .on_press(BrushEditorMessage::CancelNewBrush),
                        ]
                        .gap(4.0),
                    ]
                    .gap(4.0),
                )
                .padding(6)
                .width(Length::Fill);
                create_buttons = create_buttons.push(create_box);
            }
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
                column![
                    selected
                        .engine
                        .editor_view(globals)
                        .map(move |message| BrushEditorMessage::EngineMessage { session, message }),
                    button(label(if selected.engine.editor_dirty() {
                        t!("save_dirty")
                    } else {
                        t!("save")
                    }))
                    .width(Length::Fill)
                    .on_press(BrushEditorMessage::SaveBrush)
                ]
                .width(Length::Fill)
                .gap(8)
            }
            None => row![label(t!("select_a_brush")).muted(), label("")].gap(8.0),
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
            BrushEditorMessage::SelectBrush(asset_id) => {
                if let Err(error) = self.select_brush(asset_id, globals) {
                    log::error!("Failed to load brush preset: {error:#}");
                }
                Task::none()
            }
            BrushEditorMessage::ToggleNewBrush(engine_id) => {
                if self
                    .creating_brush
                    .as_ref()
                    .is_some_and(|(creating_id, _)| *creating_id == engine_id)
                {
                    self.creating_brush = None;
                } else {
                    self.creating_brush = Some((engine_id, String::new()));
                }
                Task::none()
            }
            BrushEditorMessage::NewBrushNameChanged(name) => {
                if let Some((_, buffer)) = self.creating_brush.as_mut() {
                    *buffer = name;
                }
                Task::none()
            }
            BrushEditorMessage::ConfirmNewBrush => {
                let Some((engine_id, name)) = self.creating_brush.clone() else {
                    return Task::none();
                };
                if let Err(error) = self.new_brush(engine_id, name, globals) {
                    log::error!("Failed to create brush preset: {error:#}");
                }
                Task::none()
            }
            BrushEditorMessage::CancelNewBrush => {
                self.creating_brush = None;
                Task::none()
            }
            BrushEditorMessage::AssetEvent(event) => {
                self.handle_asset_event(event, globals);
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
                task.map(move |message| BrushEditorMessage::EngineMessage { session, message })
            }
            BrushEditorMessage::SaveBrush => {
                let Some(selected) = self.selected.as_mut() else {
                    return Task::none();
                };

                if selected.engine.editor_dirty() {
                    selected.engine.editor_save(globals).log_err();
                }

                Task::none()
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

        let assets =
            AssetEvent::listen_for_types(globals.brush_engine_registry().brush_asset_types())
                .map(BrushEditorMessage::AssetEvent);

        Subscription::batch([external, engine, assets])
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
    fn select_brush(&mut self, asset_id: UntypedAssetId, globals: &mut Globals) -> Result<()> {
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

        let Some((engine_id, entry)) = self.brushes.iter().find(|(_, entry)| entry.id == asset_id)
        else {
            return Ok(());
        };
        let factory = globals.brush_engine_registry().get(engine_id)?;
        let engine = factory.load_brush_from_assets(entry.id, globals.assets())?;

        self.selected = Some(SelectedBrushEditor {
            asset_id: entry.id,
            session: BrushEngineSessionId::new(Uuid::new_v4()),
            engine,
        });
        Ok(())
    }

    fn new_brush(
        &mut self,
        engine_id: BrushEngineId,
        name: String,
        globals: &mut Globals,
    ) -> Result<()> {
        let display_name = name.trim();
        let Some(file_stem) = sanitize_asset_file_stem(display_name) else {
            return Ok(());
        };

        let factory = globals.brush_engine_registry().get(&engine_id)?;
        let assets = globals.assets().clone();
        let new_brush = factory.new_brush(display_name.to_string(), &assets)?;

        let bundle = assets
            .bundles()
            .find(|bundle| !bundle.is_readonly())
            .map(|bundle| bundle.metadata().bundle_id)
            .context("A writable asset bundle is required for a new brush preset")?;
        let path = format!("{file_stem}.{}", factory.brush_file_extension());
        let asset_id = factory.add_brush_to_assets(new_brush, bundle, Path::new(&path), &assets)?;
        let engine = factory.load_brush_from_assets(asset_id, &assets)?;

        self.creating_brush = None;
        self.selected = Some(SelectedBrushEditor {
            asset_id,
            session: BrushEngineSessionId::new(Uuid::new_v4()),
            engine,
        });
        Ok(())
    }

    fn handle_asset_event(&mut self, event: AssetEvent, globals: &Globals) {
        self.brushes = globals
            .brush_engine_registry()
            .all_brushes(globals.assets());

        match event {
            AssetEvent::Removed { id, .. } => {
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.asset_id == id)
                {
                    self.selected = None;
                }
            }
            AssetEvent::BundlesChanged { .. } => {
                if self.selected.as_ref().is_some_and(|selected| {
                    !self
                        .brushes
                        .iter()
                        .any(|(_, entry)| entry.id == selected.asset_id)
                }) {
                    self.selected = None;
                }
            }
            AssetEvent::Added { .. } | AssetEvent::Modified { .. } => {}
        }
    }
}

fn sanitize_asset_file_stem(name: &str) -> Option<String> {
    let stem = name
        .trim()
        .chars()
        .map(|ch| {
            if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control()
            {
                '_'
            } else {
                ch
            }
        })
        .collect::<String>();
    let stem = stem.trim_matches(|ch: char| ch == '.' || ch.is_whitespace());
    (!stem.is_empty()).then(|| stem.to_string())
}
