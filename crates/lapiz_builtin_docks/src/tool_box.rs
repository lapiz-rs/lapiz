use std::sync::LazyLock;

use iced_core::{Border, Color, Element, Length, Theme, window};
use iced_runtime::Task;
use lapiz_canvas::CanvasToolProxyAppExt as _;
use lapiz_dock::dock::{Dock, DockId};
use lapiz_i18n::t;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_tools::{
    ErasedToolFunctionMessage, ToolFunctionRegistry, ToolId, manifest::ToolBoxManifestConfig,
};
use lapiz_widgets::{
    button,
    button::{Status, Style},
    divider::Divider,
    flex::Flex,
    icon, label, row, scrollable, svg, tooltip,
    tooltip::Position,
};

pub static TOOL_BOX_DOCK_ID: LazyLock<DockId> =
    LazyLock::new(|| DockId::new("tool_box_dock".into()));

/// Workspace [`Dock`] that renders grouped tool buttons from [`ToolBoxManifestConfig`] and switches the active canvas's current tool.
pub struct ToolBoxDock {
    manifest: ToolBoxManifestConfig,
}

/// Messages handled by [`ToolBoxDock`].
pub enum ToolBoxDockMessage {
    Switch(ToolId),
    ToolFunction(ErasedToolFunctionMessage),
}

impl ToolBoxDock {
    #[allow(
        clippy::new_without_default,
        reason = "Default cannot express the semantic of reading config from disk."
    )]
    pub fn new() -> Self {
        let manifest = ToolBoxManifestConfig::read_or_init_or_fallback();
        Self { manifest }
    }
}

impl Dock for ToolBoxDock {
    type Message = ToolBoxDockMessage;

    fn id(&self) -> DockId {
        TOOL_BOX_DOCK_ID.clone()
    }

    fn view<'a>(
        &'a self,
        _window_id: window::Id,
        globals: &'a Globals,
    ) -> Element<'a, Self::Message, Theme, Renderer> {
        let active_tool = globals
            .current_tool_proxy()
            .and_then(|proxy| proxy.current_tool());
        let tool_button = |tool: &ToolId| {
            let selected = active_tool == Some(tool);
            let glyph = globals
                .global::<ToolFunctionRegistry>()
                .icon(tool)
                .unwrap_or_else(icon::info)
                .size(12)
                .style(move |theme, _| {
                    let p = theme.palette();
                    svg::Style {
                        color: Some(if selected {
                            p.primary.base.text
                        } else {
                            p.background.weak.text
                        }),
                    }
                });
            let button = button(glyph)
                .width(28)
                .height(28)
                .padding(8)
                .style(move |theme, status| {
                    let p = theme.palette();
                    let hovered = matches!(status, Status::Hovered | Status::Pressed);
                    Style {
                        background: Some(
                            if selected {
                                p.primary.base.color
                            } else if hovered {
                                p.primary.weak.color
                            } else {
                                Color::TRANSPARENT
                            }
                            .into(),
                        ),
                        text_color: if selected {
                            p.primary.base.text
                        } else {
                            p.background.weak.text
                        },
                        border: Border {
                            radius: 0.0.into(),
                            width: if selected { 1.0 } else { 0.0 },
                            color: p.primary.base.color,
                        },
                        ..Default::default()
                    }
                })
                .on_press(ToolBoxDockMessage::Switch(tool.clone()));
            tooltip(button, label(t!(tool)), Position::Right).into()
        };
        let separator = || {
            row![Divider::horizontal(1)]
                .height(7)
                .padding([3, 0])
                .into()
        };
        let mut items = Vec::new();
        let manifest = self.manifest.get();
        for group in &manifest.groups {
            if !items.is_empty() {
                items.push(separator());
            }
            let buttons = group.tools.iter().map(tool_button).collect::<Vec<_>>();
            items.push(
                Flex::row(buttons)
                    .wrap()
                    .space_evenly()
                    .gap(1)
                    .width(Length::Fill)
                    .into(),
            );
        }
        let content = Flex::column(items).width(Length::Fill).gap(0).padding(4);

        scrollable(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Self::Message, globals: &mut Globals) -> Task<Self::Message> {
        match message {
            ToolBoxDockMessage::Switch(tool) => globals
                .update_current_tool_proxy(|proxy, globals| proxy.switch_tool(tool, globals))
                .unwrap_or_else(Task::none)
                .map(ToolBoxDockMessage::ToolFunction),
            ToolBoxDockMessage::ToolFunction(message) => globals
                .update_current_tool_proxy(|proxy, globals| proxy.handle_message(message, globals))
                .unwrap_or_else(Task::none)
                .map(ToolBoxDockMessage::ToolFunction),
        }
    }
}
