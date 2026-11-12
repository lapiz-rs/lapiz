use std::sync::LazyLock;

use iced_core::{Element, Length, Theme, window};
use iced_runtime::Task;
use lapiz_canvas::CanvasToolProxyAppExt as _;
use lapiz_dock::dock::{Dock, DockId};
use lapiz_i18n::t;
use lapiz_runtime::global::Globals;
use lapiz_tools::ErasedToolFunctionMessage;
use lapiz_widgets::{label, panel, scrollable, space};

pub static TOOL_OPTIONS_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("tool_options_dock".into()));

/// Workspace [`Dock`] that renders the active tool's options widget from the current canvas's [`ToolProxy`](lapiz_tools::ToolProxy).
pub struct ToolOptionsDock;

/// Messages handled by [`ToolOptionsDock`].
pub enum ToolOptionsDockMessage {
    ToolFunction(ErasedToolFunctionMessage),
}

impl ToolOptionsDock {
    pub fn new(_: &Globals) -> Self {
        Self
    }
}

impl Dock for ToolOptionsDock {
    type Message = ToolOptionsDockMessage;

    fn id(&self) -> DockId {
        TOOL_OPTIONS_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, lapiz_runtime::Renderer> {
        let Some(tool_proxy) = globals.current_tool_proxy() else {
            return space().into();
        };

        let Some(widget) = tool_proxy.tool_option_widget(globals) else {
            return panel(label(t!("no_tool_options")).muted())
                .padding(8)
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
        };

        panel(scrollable(widget.map(ToolOptionsDockMessage::ToolFunction)))
            .padding(4)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        match message {
            ToolOptionsDockMessage::ToolFunction(message) => globals
                .update_current_tool_proxy(|tool_proxy, globals| {
                    tool_proxy.handle_message(message, globals)
                })
                .unwrap_or_else(Task::none)
                .map(ToolOptionsDockMessage::ToolFunction),
        }
    }
}
