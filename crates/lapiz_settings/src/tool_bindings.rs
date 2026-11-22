use std::sync::Arc;

use iced_core::{Element, Length, Theme, alignment};
use lapiz_i18n::t;
use lapiz_input::key::KeySequence;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_tools::{GlobalToolBindings, ToolFunctionRegistry};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{column, container, icon, kbd, label, row, scrollable};

use crate::SettingsMessage;

#[derive(Default)]
pub struct ToolBindingsPage {}

impl ToolBindingsPage {
    pub fn view(&self, globals: &Globals) -> Element<'_, SettingsMessage, Theme, Renderer> {
        let bindings = globals.global::<GlobalToolBindings>();
        let registry = globals.global::<ToolFunctionRegistry>();
        let manifest = bindings.manifest();

        let rows = manifest
            .bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                let glyph = registry
                    .icon(&binding.tool)
                    .unwrap_or_else(icon::info)
                    .size(18);
                let name = t!(&binding.tool);
                let name = if binding.is_temporary {
                    t!("settings_temporary_tool", tool = name)
                } else {
                    name
                };
                let manifest = manifest.clone();
                let keys = kbd(binding.shortcut)
                    .listenable()
                    .check_conflict(move |keys| manifest.has_shortcut_conflict(index, keys))
                    .on_confirm(move |keys| SettingsMessage::RebindTool(index, keys));

                row![
                    glyph,
                    label(name)
                        .width(Length::Fill)
                        .height(24)
                        .align_y(alignment::Vertical::Center),
                    row![keys].width(240)
                ]
                .width(Length::Fill)
                .gap(12)
                .into()
            });

        container(
            scrollable(column(rows).width(Length::Fill).gap(6).padding(12))
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(8)
        .into()
    }
}
