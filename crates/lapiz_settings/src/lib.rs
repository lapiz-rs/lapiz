use std::sync::Arc;

use anyhow::Result;
use iced_core::{Element, Size, Theme, window};
use iced_runtime::{
    Task,
    window::{close, drag, minimize, open, toggle_maximize},
};
use lapiz_i18n::t;
use lapiz_runtime::{
    Renderer, Runtime,
    global::Globals,
    plugin::Plugin,
    windows::{WindowView, WindowViewId},
};
use lapiz_widgets::{column, label, panel, tabs::TabbedView, title_bar};

use crate::{
    action_bindings::{ActionBindingsMessage, ActionBindingsPage},
    tool_bindings::{ToolBindingsMessage, ToolBindingsPage},
};

pub mod action_bindings;
pub mod tool_bindings;

lapiz_i18n::define_i18n!("settings");

#[derive(Default)]
pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut Runtime) {
        i18n::init();
        app.register_view::<SettingsView>();
    }
}

pub struct SettingsView {
    window: window::Id,
    windows: Arc<[window::Id]>,
    tool_bindings: ToolBindingsPage,
    action_bindings: ActionBindingsPage,
    selected_tab: usize,
}

#[derive(Clone)]
pub enum SettingsMessage {
    Close,
    Drag,
    Maximize,
    Minimize,
    SelectTab(usize),
    ToolBindings(ToolBindingsMessage),
    ActionBindings(ActionBindingsMessage),
}

impl WindowView for SettingsView {
    type Message = SettingsMessage;
    type BootParams = ();

    fn id() -> WindowViewId {
        WindowViewId::new("settings")
    }

    fn boot(_: Option<Self::BootParams>, _: &mut Globals) -> Result<(Self, Task<Self::Message>)> {
        let (window, open) = open(window::Settings {
            decorations: false,
            size: Size::new(760.0, 540.0),
            #[cfg(target_os = "windows")]
            platform_specific: window::settings::PlatformSpecific {
                corner_preference: window::settings::platform::CornerPreference::DoNotRound,
                ..Default::default()
            },
            ..Default::default()
        });

        Ok((
            Self {
                window,
                windows: [window].into(),
                tool_bindings: ToolBindingsPage::default(),
                action_bindings: ActionBindingsPage::default(),
                selected_tab: 0,
            },
            open.discard(),
        ))
    }

    fn view<'a>(
        &'a self,
        _: window::Id,
        globals: &'a Globals,
    ) -> impl Into<Element<'a, Self::Message, Theme, Renderer>> {
        let titlebar = title_bar(label(t!("settings_title")).window_title())
            .on_close(SettingsMessage::Close)
            .on_drag(SettingsMessage::Drag)
            .on_maximize(SettingsMessage::Maximize)
            .on_minimize(SettingsMessage::Minimize);

        let tabs = TabbedView::new()
            .tab(
                t!("settings_tool_bindings_tab"),
                self.tool_bindings
                    .view(globals)
                    .map(SettingsMessage::ToolBindings),
            )
            .tab(
                t!("settings_action_bindings_tab"),
                self.action_bindings
                    .view(globals)
                    .map(SettingsMessage::ActionBindings),
            )
            .selected(self.selected_tab)
            .on_select(SettingsMessage::SelectTab);

        panel(column![titlebar, tabs])
    }

    fn update(
        &mut self,
        message: Self::Message,
        globals: &mut Globals,
    ) -> impl Into<Task<Self::Message>> {
        match message {
            SettingsMessage::Close => close(self.window),
            SettingsMessage::Drag => drag(self.window),
            SettingsMessage::Maximize => toggle_maximize(self.window),
            SettingsMessage::Minimize => minimize(self.window, true),
            SettingsMessage::SelectTab(index) => {
                self.selected_tab = index;
                Task::none()
            }
            SettingsMessage::ToolBindings(message) => {
                self.tool_bindings.update(message, globals);
                Task::none()
            }
            SettingsMessage::ActionBindings(message) => {
                self.action_bindings.update(message, globals);
                Task::none()
            }
        }
    }

    fn close(self, _: &mut Globals) -> Task<()> {
        close(self.window)
    }

    fn windows(&self) -> Arc<[window::Id]> {
        self.windows.clone()
    }

    fn root_window(&self) -> Option<window::Id> {
        Some(self.window)
    }
}
