use std::{cell::RefCell, sync::LazyLock};

use iced::{Element, Length, Size, Subscription, Task, Theme, window};
use lapiz_color::{
    BackgroundColorChanged, Color, ForegroundBackgroundColorExt as _, ForegroundColorChanged,
    model::rgb::Rgb,
};
use lapiz_color_selector::{
    ColorSelector, ColorSelectorMessage, ColorSelectorState,
    config::{
        ColorSelectorConfigEditorState, ColorSelectorConfigGroup, ColorSelectorConfigMessage,
    },
};
use lapiz_config::Config;
use lapiz_dock::dock::{Dock, DockId};
use lapiz_i18n::t;
use lapiz_runtime::{Renderer, Services, event::Event as _};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    button::Button, flex::Flex, label::Label, panel::Panel, scrollable::Scrollable,
    title_bar::TitleBar,
};
use moxcms::ColorProfile;

#[derive(Clone)]
pub enum ColorSelectorDockMessage {
    RawWindowId(u64),
    WindowMoved,
    ColorSelector(ColorSelectorMessage),
    ConfigEditor(ColorSelectorConfigMessage),
    OpenSettings,
    SettingsWindowClosed,
    SettingsWindowDrag,
    ForegroundColorChanged(ForegroundColorChanged),
    BackgroundColorChanged(BackgroundColorChanged),
    ConfigChanged,
}

pub static COLOR_SELECTOR_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("color_selector_dock".into()));

pub struct ColorSelectorDock {
    selector: ColorSelectorState,
    config_editor: ColorSelectorConfigEditorState,
    window_id: RefCell<window::Id>,
    settings_window_id: Option<window::Id>,
    cached_config: Config<ColorSelectorConfigGroup>,

    last_color: Color,
    is_foreground_color: bool,
}

impl ColorSelectorDock {
    pub fn new(services: &Services) -> Self {
        let cached_config = Config::<ColorSelectorConfigGroup>::read_or_init_or_fallback();
        let configs = cached_config.get();

        Self {
            selector: ColorSelectorState::new(
                Color::Rgb(Rgb::new(0.0, 0.0, 0.0)),
                ColorProfile::new_srgb(),
                configs.configs.clone(),
                0,
                services,
            ),
            config_editor: ColorSelectorConfigEditorState::new(configs.configs.clone(), Some(0)),
            window_id: RefCell::new(window::Id::unique()),
            settings_window_id: None,
            cached_config,
            last_color: **services.foreground_color(),
            is_foreground_color: true,
        }
    }
}

impl Dock for ColorSelectorDock {
    type Message = ColorSelectorDockMessage;

    fn id(&self) -> DockId {
        COLOR_SELECTOR_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        window_id: window::Id,
        _services: &'a Services,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        self.window_id.replace(window_id);

        if self.settings_window_id == Some(window_id) {
            let titlebar =
                TitleBar::new(Label::new(t!("color_selector_settings_title")).window_title())
                    .on_close(ColorSelectorDockMessage::ConfigEditor(
                        ColorSelectorConfigMessage::Cancelled,
                    ))
                    .on_drag(ColorSelectorDockMessage::SettingsWindowDrag);
            Element::from(Panel::new(Flex::column([
                titlebar.into(),
                self.config_editor
                    .view()
                    .map(ColorSelectorDockMessage::ConfigEditor),
            ])))
        } else {
            Flex::column([
                Scrollable::new(ColorSelector::new(
                    &self.selector,
                    ColorSelectorDockMessage::ColorSelector,
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
                Button::new(Label::new(t!("settings")))
                    .width(Length::Fill)
                    .on_press(ColorSelectorDockMessage::OpenSettings)
                    .into(),
            ])
            .gap(4)
            .height(Length::Fill)
            .into()
        }
    }

    fn update(&mut self, message: Self::Message, services: &mut Services) -> Task<Self::Message> {
        match message {
            ColorSelectorDockMessage::WindowMoved => {
                let window_id = *self.window_id.borrow();
                window::raw_id::<()>(window_id).map(ColorSelectorDockMessage::RawWindowId)
            }
            ColorSelectorDockMessage::RawWindowId(id) => self
                .selector
                .set_output_profile(id, services)
                .map(ColorSelectorDockMessage::ColorSelector),
            ColorSelectorDockMessage::ColorSelector(ColorSelectorMessage::Confirmed(color)) => {
                if self.is_foreground_color {
                    **services.foreground_color_mut() = color;
                    ForegroundColorChanged::broadcast(ForegroundColorChanged::new(
                        self.last_color,
                        color,
                    ));
                } else {
                    **services.background_color_mut() = color;
                    BackgroundColorChanged::broadcast(BackgroundColorChanged::new(
                        self.last_color,
                        color,
                    ));
                }

                Task::none()
            }
            ColorSelectorDockMessage::ColorSelector(m) => self
                .selector
                .update(m, services)
                .map(ColorSelectorDockMessage::ColorSelector),
            ColorSelectorDockMessage::OpenSettings => {
                if let Some(id) = self.settings_window_id {
                    window::gain_focus(id)
                } else {
                    let (id, task) = window::open(window::Settings {
                        decorations: false,
                        size: Size {
                            width: 700.0,
                            height: 900.0,
                        },
                        #[cfg(target_os = "windows")]
                        platform_specific: window::settings::PlatformSpecific {
                            corner_preference:
                                window::settings::platform::CornerPreference::DoNotRound,
                            ..Default::default()
                        },
                        ..Default::default()
                    });
                    self.settings_window_id = Some(id);
                    self.config_editor = ColorSelectorConfigEditorState::new(
                        self.selector.configs().to_vec(),
                        Some(0),
                    );
                    task.discard()
                }
            }
            ColorSelectorDockMessage::SettingsWindowClosed => {
                self.settings_window_id = None;
                Task::none()
            }
            ColorSelectorDockMessage::SettingsWindowDrag => {
                if let Some(id) = self.settings_window_id {
                    window::drag(id)
                } else {
                    Task::none()
                }
            }
            ColorSelectorDockMessage::ConfigEditor(ColorSelectorConfigMessage::Cancelled) => {
                if let Some(id) = self.settings_window_id {
                    window::close(id)
                } else {
                    Task::none()
                }
            }
            ColorSelectorDockMessage::ConfigEditor(ColorSelectorConfigMessage::Confirmed) => {
                let configs = ColorSelectorConfigGroup {
                    configs: self.config_editor.configs().to_vec(),
                };

                self.cached_config
                    .update(|old| *old = configs.clone())
                    .log_err();

                Task::none()
            }
            ColorSelectorDockMessage::ConfigEditor(m) => {
                self.config_editor.update(m);
                Task::none()
            }
            ColorSelectorDockMessage::ForegroundColorChanged(event) => {
                if !self.is_foreground_color {
                    return Task::none();
                }
                self.last_color = event.new;
                self.selector
                    .set_color(event.new, services)
                    .map(ColorSelectorDockMessage::ColorSelector)
            }
            ColorSelectorDockMessage::BackgroundColorChanged(event) => {
                if self.is_foreground_color {
                    return Task::none();
                }
                self.last_color = event.new;
                self.selector
                    .set_color(event.new, services)
                    .map(ColorSelectorDockMessage::ColorSelector)
            }
            ColorSelectorDockMessage::ConfigChanged => {
                let new_config = self.cached_config.get();
                self.selector
                    .set_configs(new_config.configs.clone(), services)
                    .map(ColorSelectorDockMessage::ColorSelector)
            }
        }
    }

    fn on_open(&mut self, _services: &mut Services) -> Task<Self::Message> {
        Task::done(ColorSelectorDockMessage::WindowMoved)
    }

    fn subscription(&self, _services: &Services) -> Subscription<Self::Message> {
        let cur_window = *self.window_id.borrow();

        let window_moved =
            window::events()
                .with(cur_window)
                .filter_map(|(cur_window, (window_id, event))| {
                    if matches!(event, window::Event::Moved(_)) && cur_window == window_id {
                        Some(ColorSelectorDockMessage::WindowMoved)
                    } else {
                        None
                    }
                });

        let settings_window_closed = window::events().with(self.settings_window_id).filter_map(
            |(settings_window_id, (window_id, event))| {
                if matches!(event, window::Event::Closed) && Some(window_id) == settings_window_id {
                    Some(ColorSelectorDockMessage::SettingsWindowClosed)
                } else {
                    None
                }
            },
        );

        let foreground_color_changed = ForegroundColorChanged::listen_to()
            .map(ColorSelectorDockMessage::ForegroundColorChanged);
        let background_color_changed = BackgroundColorChanged::listen_to()
            .map(ColorSelectorDockMessage::BackgroundColorChanged);

        let config_changed = self
            .cached_config
            .listen_to()
            .map(|_| ColorSelectorDockMessage::ConfigChanged);

        Subscription::batch([
            window_moved,
            settings_window_closed,
            foreground_color_changed,
            background_color_changed,
            config_changed,
        ])
    }

    fn sub_windows(&self) -> Vec<window::Id> {
        if let Some(id) = self.settings_window_id {
            vec![id]
        } else {
            Vec::new()
        }
    }
}
