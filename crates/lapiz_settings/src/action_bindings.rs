use iced_core::{Element, Length, Theme, alignment};
use lapiz_actions::manifest::GlobalActionBindings;
use lapiz_i18n::t;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_widgets::{column, container, kbd, label, row, scrollable};

use crate::SettingsMessage;

#[derive(Default)]
pub struct ActionBindingsPage {}

impl ActionBindingsPage {
    pub fn view(&self, globals: &Globals) -> Element<'_, SettingsMessage, Theme, Renderer> {
        let manifest = globals.global::<GlobalActionBindings>().manifest();
        let rows = manifest.actions.iter().enumerate().map(|(index, binding)| {
            let name = t!(&binding.action_name);
            let manifest = manifest.clone();
            let keys = kbd(binding.shortcut)
                .listenable()
                .check_conflict(move |keys| manifest.has_shortcut_conflict(index, keys))
                .on_confirm(move |keys| SettingsMessage::RebindAction(index, keys));

            row![
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
