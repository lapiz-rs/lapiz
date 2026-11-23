use iced_core::{Element, Length, Theme, alignment};
use lapiz_actions::manifest::GlobalActionBindings;
use lapiz_i18n::t;
use lapiz_input::key::KeySequence;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{column, container, kbd, label, row, scrollable};

use crate::SettingsMessage;

#[derive(Clone)]
pub enum ActionBindingsMessage {
    RebindAction(usize, KeySequence),
}

#[derive(Default)]
pub struct ActionBindingsPage {}

impl ActionBindingsPage {
    pub fn view(&self, globals: &Globals) -> Element<'_, ActionBindingsMessage, Theme, Renderer> {
        let manifest = globals.global::<GlobalActionBindings>().manifest();
        let rows = manifest.actions.iter().enumerate().map(|(index, binding)| {
            let name = t!(&binding.action_name);
            let manifest = manifest.clone();
            let original = binding.shortcut;
            let keys = kbd(original)
                .listenable()
                .check_conflict(move |keys| keys != original && manifest.binding_count(keys) != 0)
                .on_confirm(move |keys| ActionBindingsMessage::RebindAction(index, keys));

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

    pub fn update(&mut self, message: ActionBindingsMessage, globals: &mut Globals) {
        match message {
            ActionBindingsMessage::RebindAction(index, shortcut) => {
                globals
                    .global_mut::<GlobalActionBindings>()
                    .rebind(index, shortcut)
                    .log_err();
            }
        }
    }
}
