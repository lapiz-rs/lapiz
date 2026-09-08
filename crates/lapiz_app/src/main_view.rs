use std::{any::Any, sync::Arc};

use iced::keyboard::key;
use iced::{
    Element, Length, Subscription, Task, Theme,
    keyboard::{self},
    mouse, window,
};
use iced_wgpu::Renderer;
use iced_widget::pane_grid;
use lapiz_actions::{
    ActionFunctionRegistry, ActionId,
    manifest::{ActionCollection, KeyBindingDefManifest, MenuBarItem, MenuBarManifest},
};
use lapiz_assets::AssetAppExt;
use lapiz_brush::tool::CurrentBrushPresetHandle;
use lapiz_canvas::{CanvasAppExt, CanvasToolProxyAppExt};
use lapiz_canvas::{
    event::{CanvasCreated, CanvasRemoved},
    tools::PanTool,
};
use lapiz_dock::group::DockGroupId;
use lapiz_dock::{
    DockManager, DockMessage,
    dock::{Dock, DockId},
};
use lapiz_input::key::KeyboardState;
use lapiz_runtime::{
    ApplicationTheme, Services,
    event::Event,
    windows::{WindowView, WindowViewId},
};
use lapiz_tools::{ErasedToolFunctionMessage, GlobalToolBindings, ToolFunction};
use lapiz_widgets::{
    bar::StatusBar,
    divider::Divider,
    flex::Flex,
    icon::{self, Icon},
    label::Label,
    menu::{Item, Menu, MenuBar},
    title_bar::TitleBar,
};
use moxcms::ProfileText;

use crate::dock::{
    BRUSH_PRESETS_DOCK_ID, BrushPresetDock, COLOR_SELECTOR_DOCK_ID, CanvasDock, ColorSelectorDock,
    LAYER_DOCK_ID, LayersDock, TOOL_BOX_DOCK_ID, TOOL_OPTIONS_DOCK_ID, ToolBoxDock,
    ToolOptionsDock, construct_canvas_dock_id,
};

pub struct MainView {
    dock_manager: DockManager,
    action_collection: ActionCollection,
    menu_manifest: MenuBarManifest,
    canvas_group_anchor: Option<DockGroupId>,
}

pub enum MainViewMessage {
    Dock(DockMessage),
    WindowEvent(window::Id, window::Event),
    KeyboardEvent(window::Id, keyboard::Event),
    MouseEvent(window::Id, mouse::Event),
    CanvasCreated(CanvasCreated),
    CanvasRemoved(CanvasRemoved),
    TriggerAction(ActionId),
    ActionMessage(ActionId, Box<dyn Any + Send + Sync>),
    ToolFunctionMessage(ErasedToolFunctionMessage),
    MinimizeWindow(window::Id),
    MaximizeWindow(window::Id),
    CloseWindow(window::Id),
    MenuBar(MenuBarMessage),
}

#[derive(Clone)]
pub enum MenuBarMessage {
    TriggerAction(ActionId),
    SetTheme(Theme),
}

impl MainView {
    fn status_cell<'a>(
        icon: Icon<'a>,
        text: String,
    ) -> Element<'a, MainViewMessage, Theme, Renderer> {
        Flex::row([
            icon.size(10).muted().into(),
            Label::new(text).size(10).muted().into(),
        ])
        .gap(4)
        .padding([0, 6])
        .into()
    }

    fn menu_bar(&self, current_theme: &Theme) -> MenuBar<MenuBarMessage> {
        fn build_menu(
            items: &[MenuBarItem],
            collection: &ActionCollection,
        ) -> Menu<MenuBarMessage> {
            let mut menu = Menu::new().min_width(236.0);
            for item in items {
                menu = match item {
                    MenuBarItem::Separator => menu.separator(),
                    MenuBarItem::Item(action) => {
                        let message = MenuBarMessage::TriggerAction(action.clone());
                        match collection.shortcut_for(action) {
                            Some(shortcut) => menu.item_shortcut(
                                action.to_string(),
                                &format!("{}", shortcut),
                                message,
                            ),
                            None => menu.item(action.to_string(), message),
                        }
                    }
                    MenuBarItem::Submenu { title, items } => {
                        menu.submenu(title.clone(), build_menu(items, collection))
                    }
                };
            }
            menu
        }

        let mut menu_bar = MenuBar::new();
        for category in &self.menu_manifest.categories {
            menu_bar = menu_bar.menu(
                category.title.clone(),
                build_menu(&category.items, &self.action_collection),
            );
        }

        if let Some(window) = menu_bar.get_menu_mut("Window")
            && let Some(Item::Submenu { submenu, .. }) = window.get_item_mut("Theme")
        {
            *submenu = Theme::ALL
                .iter()
                .fold(Menu::new().min_width(220.0), |menu, theme| {
                    let message = MenuBarMessage::SetTheme(theme.clone());
                    if theme == current_theme {
                        menu.selected_item(theme.to_string(), message)
                    } else {
                        menu.item(theme.to_string(), message)
                    }
                });
        }

        menu_bar
    }

    fn switch_tool_keys(
        &mut self,
        services: &mut Services,
        is_keydown: bool,
    ) -> Task<MainViewMessage> {
        services
            .update_current_tool_proxy(|tool_proxy, services| {
                let keyboard_state = services.service::<KeyboardState>();
                let seq = keyboard_state.get_sequence();

                let config = services
                    .service::<GlobalToolBindings>()
                    .binding_for(seq)
                    .cloned();
                let Some(config) = config else {
                    return tool_proxy.switch_override_tool(None, services);
                };

                if config.is_temporary {
                    tool_proxy.switch_override_tool(Some(config.tool.clone()), services)
                } else if is_keydown {
                    tool_proxy.switch_tool(config.tool.clone(), services)
                } else {
                    Task::none()
                }
            })
            .unwrap_or_else(Task::none)
            .map(MainViewMessage::ToolFunctionMessage)
    }
}

impl WindowView for MainView {
    type Message = MainViewMessage;

    fn id() -> WindowViewId {
        WindowViewId::new("main_view")
    }

    fn boot(services: &mut Services) -> (Self, Task<Self::Message>) {
        let assets = services.assets();
        let manifests = assets.all_handles_of::<KeyBindingDefManifest>().unwrap();
        let manifest = manifests.first().unwrap().get().unwrap();

        log::info!(
            "Loading {} key bindings from manifest {}",
            manifest.actions.len(),
            manifest.name
        );
        let action_collection = ActionCollection::new(&manifest);

        // TODO: move to a proper config directory once the app has one.
        let menu_manifest = match std::fs::read_to_string("assets/menu_bar_manifest.toml") {
            Ok(content) => match toml::from_str(&content) {
                Ok(manifest) => manifest,
                Err(error) => {
                    log::error!("Failed to parse menu bar manifest: {error}");
                    MenuBarManifest::default()
                }
            },
            Err(error) => {
                log::error!("Failed to read menu bar manifest: {error}");
                MenuBarManifest::default()
            }
        };

        let (main_window, task) = window::open(window::Settings {
            decorations: false,
            size: iced::Size::new(1280.0, 800.0),
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific {
                corner_preference: window::settings::platform::CornerPreference::DoNotRound,
                ..Default::default()
            },
            ..Default::default()
        });
        let (mut dock_manager, dock_manager_task) = DockManager::new(main_window);
        dock_manager.register_dock(LayersDock::new());
        dock_manager.register_dock(ToolBoxDock::new());
        dock_manager.register_dock(ToolOptionsDock::new(services));
        dock_manager.register_dock(BrushPresetDock::new(services));
        dock_manager.register_dock(ColorSelectorDock::new(services));

        let task_tool_options = dock_manager.open_dock(TOOL_OPTIONS_DOCK_ID.clone());
        let tool_options = *dock_manager
            .dock_state()
            .dock_in_group(&TOOL_OPTIONS_DOCK_ID)
            .unwrap()
            .id();
        let task_tool_box = dock_manager.open_dock_split(
            TOOL_BOX_DOCK_ID.clone(),
            &tool_options,
            pane_grid::Edge::Left,
            0.06,
        );
        let task_color_selector = dock_manager.open_dock_split(
            COLOR_SELECTOR_DOCK_ID.clone(),
            &tool_options,
            pane_grid::Edge::Right,
            0.76,
        );
        let color_selector = *dock_manager
            .dock_state()
            .dock_in_group(&COLOR_SELECTOR_DOCK_ID)
            .unwrap()
            .id();
        let task_brush_presets = dock_manager.open_dock_split(
            BRUSH_PRESETS_DOCK_ID.clone(),
            &color_selector,
            pane_grid::Edge::Bottom,
            0.34,
        );
        let brush_preset = &dock_manager
            .dock_state()
            .dock_in_group(&BRUSH_PRESETS_DOCK_ID)
            .unwrap()
            .id()
            .clone();

        let task_layer = dock_manager.open_dock_split(
            LAYER_DOCK_ID.clone(),
            brush_preset,
            pane_grid::Edge::Bottom,
            0.5,
        );

        let dock_tasks = Task::batch([
            task_tool_options,
            task_tool_box,
            task_brush_presets,
            task_layer,
            task_color_selector,
        ])
        .map(MainViewMessage::Dock);

        (
            Self {
                dock_manager,
                action_collection,
                menu_manifest,
                canvas_group_anchor: None,
            },
            Task::batch([
                task.discard(),
                dock_manager_task.map(MainViewMessage::Dock),
                dock_tasks,
            ]),
        )
    }

    fn view<'a>(
        &'a self,
        window: window::Id,
        services: &'a Services,
    ) -> impl Into<Element<'a, Self::Message, Theme, iced_wgpu::Renderer>> {
        let dock = self
            .dock_manager
            .view(window, services)?
            .map(MainViewMessage::Dock);

        if window != self.dock_manager.main_window().id {
            return Some(dock);
        }

        let window_decorations = &self.dock_manager.main_window().window_decorations;
        let title_content = Flex::row([
            Label::new("LAPIZ").size(13).strong().into(),
            Element::new(
                self.menu_bar(&services.service::<ApplicationTheme>().0)
                    .height(Length::Fill),
            )
            .map(MainViewMessage::MenuBar),
            window_decorations.caption_region(),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .gap(12)
        .padding([0, 10]);

        let title = TitleBar::new(title_content)
            .on_minimize(MainViewMessage::MinimizeWindow(window))
            .on_maximize(MainViewMessage::MaximizeWindow(window))
            .on_close(MainViewMessage::CloseWindow(window));

        let preset_name = services
            .get_service::<CurrentBrushPresetHandle>()
            .and_then(|handle| handle.0.get().ok())
            .map(|preset| preset.metadata.name.clone())
            .unwrap_or_else(|| String::from("NO PRESET"));
        let mut status_cells: Vec<Element<'_, MainViewMessage, Theme, Renderer>> =
            vec![Self::status_cell(icon::brush(), preset_name)];
        if let Some(canvas) = services.current_canvas() {
            let size = canvas.image.size();
            status_cells.extend([
                Divider::vertical(1).into(),
                Self::status_cell(icon::canvas_size(), format!("{} × {}", size.x, size.y)),
                Self::status_cell(
                    icon::palette(),
                    match canvas.image.profile().description.as_ref() {
                        Some(ProfileText::PlainString(name)) => name.clone(),
                        Some(ProfileText::Localizable(strings)) => strings
                            .first()
                            .map(|string| string.value.clone())
                            .unwrap_or_else(|| String::from("untitled")),
                        Some(ProfileText::Description(string)) => {
                            if string.unicode_string.is_empty() {
                                string.ascii_string.clone()
                            } else {
                                string.unicode_string.clone()
                            }
                        }
                        None => String::from("untitled"),
                    },
                ),
                Divider::vertical(1).into(),
                Self::status_cell(
                    icon::refresh(),
                    format!("{:.0}°", canvas.transform.rotation().to_degrees()),
                ),
                Self::status_cell(
                    icon::zoom(),
                    format!("{:.0}%", canvas.transform.zoom() * 100.0),
                ),
            ]);
        } else {
            status_cells.extend([
                Divider::vertical(1).into(),
                Self::status_cell(icon::canvas_size(), String::from("NO CANVAS")),
            ]);
        }
        let status = StatusBar::new(status_cells);

        let content: Element<'a, MainViewMessage, Theme, Renderer> =
            Flex::column([title.into(), dock, status.into()])
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        Some(content)
    }

    fn update(
        &mut self,
        message: Self::Message,
        services: &mut Services,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            MainViewMessage::Dock(m) => self
                .dock_manager
                .update(m, services)
                .map(MainViewMessage::Dock),
            MainViewMessage::WindowEvent(id, event) => {
                self.dock_manager.on_window_event(id, event).discard()
            }

            MainViewMessage::KeyboardEvent(_window, event) => {
                let keyboard_state = services.service_mut::<KeyboardState>();
                let old_modifier_count = keyboard_state.modifiers().bits().count_ones();

                match &event {
                    keyboard::Event::KeyPressed {
                        physical_key: key::Physical::Code(code),
                        repeat: false,
                        ..
                    } => {
                        if *code == key::Code::ControlLeft
                            || *code == key::Code::ControlRight
                            || *code == key::Code::ShiftLeft
                            || *code == key::Code::ShiftRight
                            || *code == key::Code::AltLeft
                            || *code == key::Code::AltRight
                            || *code == key::Code::SuperLeft
                            || *code == key::Code::SuperRight
                            || *code == key::Code::Meta
                        {
                            return Task::none();
                        }
                        keyboard_state.press(*code);

                        // TODO prevent any action from triggering when a tool is updating.
                        if let Some(action) = self
                            .action_collection
                            .get_action_id(keyboard_state.get_sequence())
                            && let Some(action_func) = services
                                .service_mut::<ActionFunctionRegistry>()
                                .get(action.clone())
                        {
                            log::info!("Triggering action: {}", action.0);
                            return action_func.trigger(services).map(move |message| {
                                MainViewMessage::ActionMessage(action.clone(), message)
                            });
                        }

                        self.switch_tool_keys(services, true)
                    }
                    keyboard::Event::KeyReleased {
                        physical_key: key::Physical::Code(code),
                        ..
                    } => {
                        keyboard_state.release(*code);
                        self.switch_tool_keys(services, false)
                    }
                    keyboard::Event::ModifiersChanged(modifiers) => {
                        keyboard_state.set_modifiers(*modifiers);

                        let new_modifier_count = keyboard_state.modifiers().bits().count_ones();
                        let is_keydown = new_modifier_count > old_modifier_count;
                        self.switch_tool_keys(services, is_keydown)
                    }
                    _ => Task::none(),
                }
            }
            MainViewMessage::MouseEvent(window, event) => {
                match event {
                    mouse::Event::CursorMoved { position } => {
                        return self
                            .dock_manager
                            .on_cursor_moved(window, position)
                            .map(MainViewMessage::Dock);
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        return self
                            .dock_manager
                            .on_float_window_drag_end()
                            .map(MainViewMessage::Dock);
                    }
                    _ => {}
                }

                Task::none()
            }
            MainViewMessage::CanvasCreated(e) => {
                log::info!("Canvas created: {}", e.id);
                let tool_task = services
                    .update_tool_proxy(&e.id, |tool_proxy, services| {
                        tool_proxy.switch_tool(PanTool::id(), services)
                    })
                    .unwrap_or_else(Task::none);
                let dock = CanvasDock::new(e.id, self.dock_manager.main_window().id);
                let id = <CanvasDock as Dock>::id(&dock);
                self.dock_manager.register_dock(dock);

                let dock_task = if let Some(target) = self.canvas_group_anchor {
                    self.dock_manager.open_dock_in_group(id.clone(), &target)
                } else {
                    let task = self.dock_manager.open_dock(id.clone());
                    self.canvas_group_anchor = self
                        .dock_manager
                        .dock_state()
                        .dock_in_group(&id)
                        .map(|group| *group.id());
                    task
                }
                .map(MainViewMessage::Dock);

                Task::batch([
                    tool_task.map(MainViewMessage::ToolFunctionMessage),
                    dock_task,
                ])
            }
            MainViewMessage::CanvasRemoved(e) => {
                log::info!("Canvas removed: {}", e.id);
                let id = DockId::new(construct_canvas_dock_id(e.id).into());
                self.dock_manager.unregister_dock(&id);
                Task::none()
            }
            MainViewMessage::TriggerAction(action_id) => {
                if let Some(action_func) = services
                    .service_mut::<ActionFunctionRegistry>()
                    .get(action_id.clone())
                {
                    action_func.trigger(services).map(move |message| {
                        MainViewMessage::ActionMessage(action_id.clone(), message)
                    })
                } else {
                    Task::none()
                }
            }
            MainViewMessage::ActionMessage(action_id, message) => {
                if let Some(action_func) = services
                    .service_mut::<ActionFunctionRegistry>()
                    .get(action_id.clone())
                {
                    action_func
                        .handle_message(message, services)
                        .map(move |message| {
                            MainViewMessage::ActionMessage(action_id.clone(), message)
                        })
                } else {
                    Task::none()
                }
            }
            MainViewMessage::ToolFunctionMessage(message) => services
                .update_current_tool_proxy(|tool_proxy, services| {
                    tool_proxy.handle_message(message, services)
                })
                .unwrap_or_else(Task::none)
                .map(MainViewMessage::ToolFunctionMessage),
            MainViewMessage::MinimizeWindow(id) => window::minimize(id, true),
            MainViewMessage::MaximizeWindow(id) => window::toggle_maximize(id),
            MainViewMessage::CloseWindow(id) => window::close(id),
            MainViewMessage::MenuBar(MenuBarMessage::SetTheme(theme)) => {
                services.service_mut::<ApplicationTheme>().0 = theme;
                Task::none()
            }
            MainViewMessage::MenuBar(MenuBarMessage::TriggerAction(action_id)) => {
                Task::done(MainViewMessage::TriggerAction(action_id))
            }
        }
    }

    fn close(self, _services: &mut Services) -> Task<()> {
        iced::exit()
    }

    fn subscription(&self, services: &Services) -> Subscription<Self::Message> {
        let external = iced::event::listen_with(|event, _status, window| match event {
            iced::Event::Window(e) => Some(MainViewMessage::WindowEvent(window, e)),
            iced::Event::Keyboard(e) => Some(MainViewMessage::KeyboardEvent(window, e)),
            iced::Event::Mouse(e) => Some(MainViewMessage::MouseEvent(window, e)),
            _ => None,
        });

        let dock = self
            .dock_manager
            .subscription(services)
            .map(MainViewMessage::Dock);
        let canvas_create = CanvasCreated::listen_to().map(MainViewMessage::CanvasCreated);
        let canvas_remove = CanvasRemoved::listen_to().map(MainViewMessage::CanvasRemoved);

        Subscription::batch([external, dock, canvas_create, canvas_remove])
    }

    fn windows(&self) -> Arc<[iced_core::window::Id]> {
        self.dock_manager
            .window_infos()
            .map(|i| i.id)
            .chain(self.dock_manager.sub_windows())
            .collect::<Vec<_>>()
            .into()
    }

    fn root_window(&self) -> Option<iced_core::window::Id> {
        Some(self.dock_manager.main_window().id)
    }
}
