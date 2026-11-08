use std::{any::Any, sync::Arc};

use anyhow::Result;
use iced::{
    Element, Length, Subscription, Task, Theme, event::listen_with, keyboard::key, widget::sensor,
    window,
};
use iced_core::keyboard;
use lapiz_actions::{
    ActionFunctionRegistry, ActionId,
    manifest::{ActionBindingManifestConfig, ActionCollection, MenuBarItem, MenuBarManifestConfig},
};
use lapiz_brush::tool::CurrentBrushPresetHandle;
use lapiz_builtin_docks::canvas::{CanvasDock, construct_canvas_dock_id};
use lapiz_canvas::{
    CanvasAppExt as _, CanvasToolProxyAppExt as _,
    event::{CanvasCreated, CanvasRemoved},
    recent::RecentFiles,
    tools::PanTool,
};
use lapiz_config::Config;
use lapiz_dock::{
    DockManager, DockMessage, DockRegistry,
    dock::{Dock, DockId},
    group::DockGroupId,
    layout::DockLayout,
};
use lapiz_i18n::{config::LanguageConfig, t};
use lapiz_image::{
    texel::TexelType,
    tile::{GpuLayerInfo, TileStorageAppExt as _},
};
use lapiz_input::key::KeyboardState;
use lapiz_runtime::{
    ApplicationTheme, Renderer, Runtime,
    event::Event as _,
    global::Globals,
    plugin::Plugin,
    windows::{WindowView, WindowViewId},
};
use lapiz_tools::{
    ErasedToolFunctionMessage, GlobalToolBindings, ToolFunction as _, ToolFunctionRegistry,
    ToolProxies, ToolProxy,
};
use lapiz_undo::{UndoStack, UndoStacks};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    column,
    divider::Divider,
    icon::{self, Icon},
    label, menu,
    menu::{Item, Menu, MenuBar},
    menu_bar, row, status_bar, title_bar,
};
use moxcms::ProfileText;
use unic_langid::LanguageIdentifier;

lapiz_i18n::define_i18n!("main_view");

pub struct MainViewPlugin;

impl Plugin for MainViewPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();

        app.window_manager_mut().set_root_view::<MainView>();
        app.register_view::<MainView>();
    }
}

pub struct MainView {
    dock_manager: DockManager,
    action_collection: ActionCollection,
    menu_manifest: MenuBarManifestConfig,
    last_canvas_dock: Option<DockId>,
    canvas_group_anchor: Option<DockGroupId>,
}

pub enum MainViewMessage {
    Dock(DockMessage),
    #[cfg(not(target_os = "android"))]
    MainWindowDrag,
    WindowUnfocused,
    KeyboardEvent(keyboard::Event),
    CanvasCreated(CanvasCreated),
    CanvasRemoved(CanvasRemoved),
    TriggerAction(ActionId),
    ActionMessage(ActionId, Box<dyn Any + Send + Sync>),
    ToolFunctionMessage(ErasedToolFunctionMessage),
    #[cfg(not(target_os = "android"))]
    MinimizeWindow(window::Id),
    #[cfg(not(target_os = "android"))]
    MaximizeWindow(window::Id),
    #[cfg(not(target_os = "android"))]
    CloseWindow(window::Id),
    MenuBar(MenuBarMessage),
}

#[derive(Clone)]
pub enum MenuBarMessage {
    TriggerAction(ActionId),
    ToggleDock(DockId),
    SetTheme(Theme),
    SetLanguage(LanguageIdentifier),
}

impl MainView {
    fn update_canvas_group_anchor(&mut self) {
        if let Some(group) = self
            .last_canvas_dock
            .as_ref()
            .and_then(|id| self.dock_manager.group_of(id))
        {
            self.canvas_group_anchor = Some(group);
        }
    }

    fn open_canvas_dock(&mut self, globals: &mut Globals, id: DockId) -> Task<DockMessage> {
        self.update_canvas_group_anchor();
        let target = self
            .canvas_group_anchor
            .filter(|group| self.dock_manager.contains_group(group))
            .or_else(|| self.dock_manager.largest_group());
        let task = if let Some(target) = target {
            self.dock_manager
                .open_dock_in_group(globals, id.clone(), &target)
        } else {
            self.dock_manager.open_dock(globals, id.clone())
        };

        self.last_canvas_dock = Some(id);
        self.update_canvas_group_anchor();
        task
    }

    fn status_cell<'a>(
        icon: Icon<'a>,
        text: String,
    ) -> Element<'a, MainViewMessage, Theme, Renderer> {
        row![icon.size(10).muted(), label(text).size(10).muted()]
            .gap(4)
            .padding([0, 6])
            .into()
    }

    fn menu_bar(&self, current_theme: &Theme) -> MenuBar<MenuBarMessage> {
        fn build_menu(
            items: &[MenuBarItem],
            collection: &ActionCollection,
        ) -> Menu<MenuBarMessage> {
            let mut menu = menu().min_width(236.0);
            for item in items {
                menu = match item {
                    MenuBarItem::Separator => menu.separator(),
                    MenuBarItem::Item(action) => {
                        let message = MenuBarMessage::TriggerAction(action.clone());
                        match collection.shortcut_for(action) {
                            Some(shortcut) => {
                                menu.item_shortcut(t!(action), &format!("{}", shortcut), message)
                            }
                            None => menu.item(t!(action), message),
                        }
                    }
                    MenuBarItem::Submenu { title, items } => {
                        menu.submenu(t!(title), build_menu(items, collection))
                    }
                };
            }
            menu
        }

        let mut menu_bar = menu_bar();
        let menu_manifest = self.menu_manifest.get();
        for category in &menu_manifest.categories {
            menu_bar = menu_bar.menu(
                t!(&category.title),
                build_menu(&category.items, &self.action_collection),
            );
        }

        let window_title = t!("menu_window");
        if let Some(window) = menu_bar.get_menu_mut(&window_title)
            && let Some(Item::Submenu { submenu, .. }) =
                window.get_item_mut(&t!("menu_theme_submenu"))
        {
            *submenu = Theme::ALL
                .iter()
                .fold(menu().min_width(220.0), |menu, theme| {
                    let message = MenuBarMessage::SetTheme(theme.clone());
                    if theme == current_theme {
                        menu.selected_item(theme.to_string(), message)
                    } else {
                        menu.item(theme.to_string(), message)
                    }
                });
        }

        if let Some(window) = menu_bar.get_menu_mut(&window_title)
            && let Some(Item::Submenu { submenu, .. }) =
                window.get_item_mut(&t!("menu_language_submenu"))
        {
            *submenu =
                lapiz_i18n::AVAILABLE
                    .iter()
                    .fold(menu().min_width(220.0), |menu, (id, name)| {
                        let Ok(langid) = id.parse::<LanguageIdentifier>() else {
                            return menu;
                        };
                        let message = MenuBarMessage::SetLanguage(langid.clone());
                        if langid == lapiz_i18n::current_language() {
                            menu.selected_item(*name, message)
                        } else {
                            menu.item(*name, message)
                        }
                    });
        }

        if let Some(window) = menu_bar.get_menu_mut(&window_title)
            && let Some(Item::Submenu { submenu, .. }) =
                window.get_item_mut(&t!("menu_dock_submenu"))
        {
            let mut docks = self.dock_manager.registered_docks().collect::<Vec<_>>();
            docks.sort_by(|dock_a, dock_b| {
                let name_a = dock_a.display_name();
                let name_b = dock_b.display_name();
                name_a.cmp(&name_b)
            });
            let dock_menu = docks
                .into_iter()
                .fold(menu().min_width(220.0), |menu, dock| {
                    let id = dock.id();
                    let name = dock.display_name();
                    if self.dock_manager.is_dock_open(&id) {
                        menu.selected_item(name, MenuBarMessage::ToggleDock(id))
                    } else {
                        menu.item(name, MenuBarMessage::ToggleDock(id))
                    }
                });
            *submenu = dock_menu;
        }

        menu_bar
    }

    fn switch_tool_keys(
        &mut self,
        globals: &mut Globals,
        is_keydown: bool,
    ) -> Task<MainViewMessage> {
        globals
            .update_current_tool_proxy(|tool_proxy, globals| {
                let keyboard_state = globals.global::<KeyboardState>();
                let seq = keyboard_state.get_sequence();

                let config = globals
                    .global::<GlobalToolBindings>()
                    .binding_for(seq)
                    .cloned();
                let Some(config) = config else {
                    return tool_proxy.switch_override_tool(None, globals);
                };

                if config.is_temporary {
                    tool_proxy.switch_override_tool(Some(config.tool.clone()), globals)
                } else if is_keydown {
                    tool_proxy.switch_tool(config.tool.clone(), globals)
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

    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new("main_view")
    }

    fn boot(
        _params: Option<Self::BootParams>,
        globals: &mut Globals,
    ) -> Result<(Self, Task<Self::Message>)> {
        let action_bindings = ActionBindingManifestConfig::read_or_init_or_fallback().get();
        log::info!(
            "Loading {} key bindings from manifest {}",
            action_bindings.actions.len(),
            action_bindings.name
        );
        let action_collection = ActionCollection::new(&action_bindings);

        let menu_manifest = MenuBarManifestConfig::read_or_init_or_fallback();

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
        let dock_registry = globals.remove_global::<DockRegistry>();
        let (mut dock_manager, dock_manager_task) = dock_registry.build(main_window);

        let saved_layout = Config::<DockLayout>::read_or_init_or_fallback();
        let dock_tasks = dock_manager
            .restore_layout(&saved_layout.get(), globals)
            .unwrap_or_else(Task::none)
            .map(MainViewMessage::Dock);

        Ok((
            Self {
                dock_manager,
                action_collection,
                menu_manifest,
                last_canvas_dock: None,
                canvas_group_anchor: None,
            },
            Task::batch([
                task.discard(),
                dock_manager_task.map(MainViewMessage::Dock),
                dock_tasks,
            ]),
        ))
    }

    fn view<'a>(
        &'a self,
        window: window::Id,
        globals: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, lapiz_runtime::Renderer>> {
        let dock = self
            .dock_manager
            .view(window, globals)?
            .map(MainViewMessage::Dock);

        if window != self.dock_manager.main_window() {
            return Some(dock);
        }

        let title_content = row![
            label("LAPIZ").window_title(),
            Element::new(
                self.menu_bar(&globals.global::<ApplicationTheme>().0)
                    .height(Length::Fill),
            )
            .map(MainViewMessage::MenuBar)
        ]
        .gap(12);

        #[cfg(not(target_os = "android"))]
        let title = title_bar(title_content)
            .on_minimize(MainViewMessage::MinimizeWindow(window))
            .on_maximize(MainViewMessage::MaximizeWindow(window))
            .on_close(MainViewMessage::CloseWindow(window))
            .on_drag(MainViewMessage::MainWindowDrag);

        #[cfg(target_os = "android")]
        let title = title_bar(title_content);

        let preset_name = globals
            .get_global::<CurrentBrushPresetHandle>()
            .and_then(|handle| handle.0.get().ok())
            .map(|preset| preset.metadata.name.clone())
            .unwrap_or_else(|| String::from("NO PRESET"));
        let mut status_cells = vec![Self::status_cell(icon::brush(), preset_name)];
        if let Some(canvas) = globals.current_canvas() {
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
        let status = status_bar(status_cells);

        let dock = sensor(dock)
            .on_show(|size| MainViewMessage::Dock(DockMessage::MainLayoutResized(size)))
            .on_resize(|size| MainViewMessage::Dock(DockMessage::MainLayoutResized(size)));
        let content = Element::from(
            column![title, dock, status]
                .width(Length::Fill)
                .height(Length::Fill),
        );
        Some(content)
    }

    fn update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>> {
        self.update_canvas_group_anchor();

        match message {
            MainViewMessage::Dock(m) => {
                let task = self.dock_manager.update(m, globals);
                self.update_canvas_group_anchor();
                task.map(MainViewMessage::Dock)
            }
            #[cfg(not(target_os = "android"))]
            MainViewMessage::MainWindowDrag => window::drag(self.dock_manager.main_window()),
            MainViewMessage::WindowUnfocused => {
                globals.global_mut::<KeyboardState>().clear();
                globals
                    .update_current_tool_proxy(|tool_proxy, globals| {
                        tool_proxy.switch_override_tool(None, globals)
                    })
                    .unwrap_or_else(Task::none)
                    .map(MainViewMessage::ToolFunctionMessage)
            }

            MainViewMessage::KeyboardEvent(event) => {
                let keyboard_state = globals.global_mut::<KeyboardState>();
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
                            && let Some(action_func) = globals
                                .global_mut::<ActionFunctionRegistry>()
                                .get(action.clone())
                        {
                            log::info!("Triggering action: {}", action.0);
                            return action_func.trigger(globals).map(move |message| {
                                MainViewMessage::ActionMessage(action.clone(), message)
                            });
                        }

                        self.switch_tool_keys(globals, true)
                    }
                    keyboard::Event::KeyReleased {
                        physical_key: key::Physical::Code(code),
                        ..
                    } => {
                        keyboard_state.release(*code);
                        self.switch_tool_keys(globals, false)
                    }
                    keyboard::Event::ModifiersChanged(modifiers) => {
                        keyboard_state.set_modifiers(*modifiers);

                        let new_modifier_count = keyboard_state.modifiers().bits().count_ones();
                        let is_keydown = new_modifier_count > old_modifier_count;
                        self.switch_tool_keys(globals, is_keydown)
                    }
                    _ => Task::none(),
                }
            }
            MainViewMessage::CanvasCreated(e) => {
                log::info!("Canvas created: {}", e.id);

                let tool_proxy = ToolProxy::new(globals.global::<ToolFunctionRegistry>());
                globals
                    .global_mut::<ToolProxies>()
                    .insert(*e.id, tool_proxy);
                let undo_stack = UndoStack::new(*e.id, 200);
                globals.global_mut::<UndoStacks>().insert(*e.id, undo_stack);

                let canvas = globals.canvas(&e.id).unwrap();
                globals.tile_storage().declare_layer(
                    canvas.image.selection_layer(),
                    GpuLayerInfo {
                        // TODO This will change when image depth is not 8 bit
                        texel_type: TexelType::A8,
                    },
                );

                Config::<RecentFiles>::read_or_init_or_fallback()
                    .update(|c| {
                        c.update(
                            canvas.local_file().source(),
                            canvas.local_file().name().to_owned(),
                        );
                    })
                    .log_err();

                let tool_task = globals
                    .update_tool_proxy(&e.id, |tool_proxy, globals| {
                        tool_proxy.switch_tool(PanTool::id(), globals)
                    })
                    .unwrap_or_else(Task::none);
                let dock = CanvasDock::new(e.id, self.dock_manager.main_window());
                let id = <CanvasDock as Dock>::id(&dock);
                self.dock_manager.register_dock(dock);

                let dock_task = self
                    .open_canvas_dock(globals, id)
                    .map(MainViewMessage::Dock);

                Task::batch([
                    tool_task.map(MainViewMessage::ToolFunctionMessage),
                    dock_task,
                ])
            }
            MainViewMessage::CanvasRemoved(e) => {
                log::info!("Canvas removed: {}", e.id);
                let id = DockId::new(construct_canvas_dock_id(e.id).into());
                self.dock_manager.unregister_dock(&id).discard()
            }
            MainViewMessage::TriggerAction(action_id) => {
                if let Some(action_func) = globals
                    .global_mut::<ActionFunctionRegistry>()
                    .get(action_id.clone())
                {
                    action_func.trigger(globals).map(move |message| {
                        MainViewMessage::ActionMessage(action_id.clone(), message)
                    })
                } else {
                    Task::none()
                }
            }
            MainViewMessage::ActionMessage(action_id, message) => {
                if let Some(action_func) = globals
                    .global_mut::<ActionFunctionRegistry>()
                    .get(action_id.clone())
                {
                    action_func
                        .handle_message(message, globals)
                        .map(move |message| {
                            MainViewMessage::ActionMessage(action_id.clone(), message)
                        })
                } else {
                    Task::none()
                }
            }
            MainViewMessage::ToolFunctionMessage(message) => globals
                .update_current_tool_proxy(|tool_proxy, globals| {
                    tool_proxy.handle_message(message, globals)
                })
                .unwrap_or_else(Task::none)
                .map(MainViewMessage::ToolFunctionMessage),
            #[cfg(not(target_os = "android"))]
            MainViewMessage::MinimizeWindow(id) => window::minimize(id, true),
            #[cfg(not(target_os = "android"))]
            MainViewMessage::MaximizeWindow(id) => window::toggle_maximize(id),
            #[cfg(not(target_os = "android"))]
            MainViewMessage::CloseWindow(id) => window::close(id),
            MainViewMessage::MenuBar(MenuBarMessage::ToggleDock(id)) => {
                if self.dock_manager.is_dock_open(&id) {
                    self.dock_manager.close_dock(&id).discard()
                } else {
                    self.dock_manager
                        .open_dock(globals, id)
                        .map(MainViewMessage::Dock)
                }
            }
            MainViewMessage::MenuBar(MenuBarMessage::SetTheme(theme)) => {
                globals.global_mut::<ApplicationTheme>().0 = theme;
                Task::none()
            }
            MainViewMessage::MenuBar(MenuBarMessage::SetLanguage(id)) => {
                lapiz_i18n::set_language(&id);
                LanguageConfig::read_or_init_or_fallback()
                    .update(|c| c.lang = Some(id))
                    .log_err();
                Task::none()
            }
            MainViewMessage::MenuBar(MenuBarMessage::TriggerAction(action_id)) => {
                Task::done(MainViewMessage::TriggerAction(action_id))
            }
        }
    }

    fn close(self, _globals: &mut Globals) -> Task<()> {
        Config::<DockLayout>::read_or_init_or_fallback()
            .update(|config| *config = self.dock_manager.layout_snapshot())
            .log_err();
        iced::exit()
    }

    fn subscription(&self, globals: &Globals) -> Subscription<Self::Message> {
        let external = listen_with(|event, _status, _window| match event {
            iced::Event::Window(window::Event::Unfocused) => Some(MainViewMessage::WindowUnfocused),
            iced::Event::Keyboard(event) => Some(MainViewMessage::KeyboardEvent(event)),
            _ => None,
        });

        let dock = self
            .dock_manager
            .subscription(globals)
            .map(MainViewMessage::Dock);
        let canvas_create = CanvasCreated::listen_to().map(MainViewMessage::CanvasCreated);
        let canvas_remove = CanvasRemoved::listen_to().map(MainViewMessage::CanvasRemoved);

        Subscription::batch([external, dock, canvas_create, canvas_remove])
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.dock_manager.windows().collect::<Vec<_>>().into()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.dock_manager.main_window())
    }
}
