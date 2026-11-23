use iced_core::{Element, Length, Theme, alignment, keyboard::Modifiers};
use lapiz_i18n::t;
use lapiz_input::key::KeySequence;
use lapiz_runtime::{Renderer, global::Globals};
use lapiz_tools::{GlobalToolBindings, ToolFunctionRegistry, ToolId, manifest::ToolBinding};
use lapiz_utils::log_err::LogErr as _;
use lapiz_widgets::{
    button, column, combo_box::ComboBox, context_menu, icon, icon_button, kbd, label, menu, row,
    scrollable,
};

use crate::SettingsMessage;

#[derive(Clone)]
pub enum ToolBindingsMessage {
    AddToolBinding(AddToolBinding),
    RebindTool(usize, KeySequence),
    DeleteBinding(usize),
}

#[derive(Clone)]
pub enum AddToolBinding {
    Start,
    SelectTool(ToolId),
    SelectHoldMode(bool),
    RecordShortcut(KeySequence),
    Submit,
    Cancel,
}

#[derive(Default)]
struct BindingDraft {
    tool: Option<ToolId>,
    is_temporary: bool,
    shortcut: Option<KeySequence>,
    save_error: Option<String>,
}

#[derive(Default)]
pub struct ToolBindingsPage {
    draft: Option<BindingDraft>,
}

impl ToolBindingsPage {
    pub fn view(&self, globals: &Globals) -> Element<'_, ToolBindingsMessage, Theme, Renderer> {
        let bindings = globals.global::<GlobalToolBindings>();
        let registry = globals.global::<ToolFunctionRegistry>();
        let manifest = bindings.manifest();
        let mut bindings = manifest.bindings.iter().enumerate().collect::<Vec<_>>();
        bindings.sort_by(|(_, a), (_, b)| {
            a.tool
                .cmp(&b.tool)
                .then(a.is_temporary.cmp(&b.is_temporary))
        });

        let rows = bindings.into_iter().map(|(index, binding)| {
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
            let original = binding.shortcut;
            let keys = kbd(original)
                .listenable()
                .check_conflict(move |keys| keys != original && manifest.binding_count(keys) != 0)
                .on_confirm(move |keys| ToolBindingsMessage::RebindTool(index, keys));

            let content = row![
                glyph,
                label(name)
                    .width(Length::Fill)
                    .height(24)
                    .align_y(alignment::Vertical::Center),
                row![keys].width(240)
            ]
            .width(Length::Fill)
            .gap(12);
            let menu = menu().item(
                t!("settings_delete_tool_binding"),
                None,
                ToolBindingsMessage::DeleteBinding(index),
            );
            context_menu(content, menu).into()
        });

        let footer = if let Some(draft) = &self.draft {
            let conflict = draft
                .shortcut
                .is_some_and(|keys| manifest.binding_count(keys) != 0);
            self.draft_view(draft, registry, conflict)
        } else {
            button(
                row![
                    icon::plus().size(16),
                    label(t!("settings_add_tool_binding"))
                ]
                .gap(6),
            )
            .on_press(ToolBindingsMessage::AddToolBinding(AddToolBinding::Start))
            .into()
        };

        column![
            scrollable(column(rows).width(Length::Fill).gap(6).padding(12))
                .width(Length::Fill)
                .height(Length::Fill),
            column![footer].width(Length::Fill).padding([0, 12]),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(8)
        .into()
    }

    fn draft_view(
        &self,
        draft: &BindingDraft,
        registry: &ToolFunctionRegistry,
        conflict: bool,
    ) -> Element<'_, ToolBindingsMessage, Theme, Renderer> {
        let mut tools = registry.tool_ids().cloned().collect::<Vec<_>>();
        tools.sort_by_cached_key(|tool| (t!(tool), tool.0.clone()));

        let glyph = draft
            .tool
            .as_ref()
            .and_then(|tool| registry.icon(tool))
            .unwrap_or_else(icon::info)
            .size(18);
        let tool = ComboBox::new_with(
            tools,
            draft.tool.clone(),
            |tool| ToolBindingsMessage::AddToolBinding(AddToolBinding::SelectTool(tool)),
            |tool| label(t!(tool)).into(),
        )
        .placeholder(t!("settings_select_tool"))
        .width(Length::Fill);
        let mode = ComboBox::new_with(
            [false, true],
            Some(draft.is_temporary),
            |hold| ToolBindingsMessage::AddToolBinding(AddToolBinding::SelectHoldMode(hold)),
            |hold| {
                label(if *hold {
                    t!("settings_tool_mode_hold")
                } else {
                    t!("settings_tool_mode_switch")
                })
                .into()
            },
        )
        .width(130);
        let keys = kbd(draft.shortcut.unwrap_or(KeySequence {
            key: None,
            modifiers: Modifiers::empty(),
        }))
        .placeholder(t!("settings_record_tool_shortcut"))
        .listenable()
        .confirm_on_release()
        .on_confirm(|keys| {
            ToolBindingsMessage::AddToolBinding(AddToolBinding::RecordShortcut(keys))
        });
        let can_submit = draft.tool.is_some() && draft.shortcut.is_some() && !conflict;

        let mut footer = column![
            row![
                glyph,
                tool,
                mode,
                row![keys].width(220),
                icon_button(icon::check()).on_press_maybe(
                    can_submit
                        .then_some(ToolBindingsMessage::AddToolBinding(AddToolBinding::Submit)),
                ),
                icon_button(icon::close())
                    .on_press(ToolBindingsMessage::AddToolBinding(AddToolBinding::Cancel)),
            ]
            .width(Length::Fill)
            .gap(12)
        ];

        if conflict {
            footer = footer.push(label(t!("settings_tool_shortcut_conflict")));
        } else if let Some(error) = &draft.save_error {
            footer = footer.push(label(t!(
                "settings_tool_bindings_save_error",
                error = error.clone()
            )));
        }

        footer.gap(4).width(Length::Fill).into()
    }

    pub(crate) fn update(&mut self, message: ToolBindingsMessage, globals: &mut Globals) {
        match message {
            ToolBindingsMessage::AddToolBinding(message) => {
                self.update_add_tool_binding(message, globals);
            }
            ToolBindingsMessage::RebindTool(index, shortcut) => {
                globals
                    .global_mut::<GlobalToolBindings>()
                    .rebind(index, shortcut)
                    .log_err();
            }
            ToolBindingsMessage::DeleteBinding(index) => {
                globals
                    .global_mut::<GlobalToolBindings>()
                    .remove_binding(index)
                    .log_err();
            }
        }
    }

    fn update_add_tool_binding(&mut self, message: AddToolBinding, globals: &mut Globals) {
        match (&mut self.draft, message) {
            (draft, AddToolBinding::Start) => {
                draft.get_or_insert_with(BindingDraft::default);
            }
            (draft, AddToolBinding::Cancel) => *draft = None,
            (Some(draft), AddToolBinding::SelectTool(tool)) => {
                draft.tool = Some(tool);
                draft.save_error = None;
            }
            (Some(draft), AddToolBinding::SelectHoldMode(hold)) => {
                draft.is_temporary = hold;
                draft.save_error = None;
            }
            (Some(draft), AddToolBinding::RecordShortcut(keys)) => {
                draft.shortcut = Some(keys);
                draft.save_error = None;
            }
            (Some(draft), AddToolBinding::Submit) => {
                let (Some(tool), Some(shortcut)) = (draft.tool.clone(), draft.shortcut) else {
                    return;
                };
                let binding = ToolBinding {
                    tool,
                    shortcut,
                    is_temporary: draft.is_temporary,
                };

                match globals
                    .global_mut::<GlobalToolBindings>()
                    .add_binding(binding)
                {
                    Ok(()) => self.draft = None,
                    Err(error) => draft.save_error = Some(error.to_string()),
                }
            }
            (None, _) => {}
        }
    }
}
