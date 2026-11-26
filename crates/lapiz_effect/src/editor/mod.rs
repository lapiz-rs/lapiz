use std::fmt;

use iced_core::Length;
use lapiz_i18n::t;
use lapiz_widgets::{
    button, column, combo_box,
    component::{Component, component},
    flex::Flex,
    icon, icon_button, label, panel, row, scrollable, text_input,
};
use uuid::Uuid;

use crate::{
    asset::{EffectPassDispatchStrategy, EffectPassId},
    instance::{EffectInstance, EffectPass},
    shader_graph::{
        GraphElement, GraphRenderer, GraphTheme,
        builtin::types::{
            atomic::{ArrayAtomicI32Type, ArrayAtomicU32Type},
            handle::{ArrayType, LayerType},
        },
        editor::{GraphEditor, GraphEditorMessage},
        graph::{Graph, GraphResources},
    },
};

pub mod io;

#[cfg(test)]
mod tests;

/// A UI [`Component`] that renders either the multi-pass overview list or the active pass's
/// [`GraphEditor`] for an [`EffectInstance`].
pub struct EffectEditor<'a, Message> {
    instance: &'a EffectInstance,
    on_change: Box<dyn Fn(EffectEditorMessage) -> Message + 'a>,
}

impl<'a, Message> EffectEditor<'a, Message> {
    pub fn new(
        instance: &'a EffectInstance,
        on_change: impl Fn(EffectEditorMessage) -> Message + 'a,
    ) -> Self {
        Self {
            instance,
            on_change: Box::new(on_change),
        }
    }
}

impl<'a, Message> Component<'a, Message, GraphTheme, GraphRenderer> for EffectEditor<'a, Message> {
    type State = EffectEditorState;
    type Event = EffectEditorEvent;

    fn diff(&mut self, state: &mut Self::State) {
        if let Some(pass_id) = state.open_pass
            && !self.instance.passes.contains_key(&pass_id)
        {
            state.open_pass = None;
        }
        if let Some(pass_id) = state.renaming_pass
            && !self.instance.passes.contains_key(&pass_id)
        {
            state.renaming_pass = None;
        }
    }

    fn update(
        &mut self,
        state: &mut Self::State,
        event: Self::Event,
        _renderer: &GraphRenderer,
    ) -> Option<Message> {
        let message = match event {
            EffectEditorEvent::OpenPass(pass_id) => {
                state.open_pass = Some(pass_id);
                return None;
            }
            EffectEditorEvent::BackToPassList => {
                state.open_pass = None;
                return None;
            }
            EffectEditorEvent::PassRenameToggled(pass_id) => {
                state.renaming_pass = if state.renaming_pass == Some(pass_id) {
                    None
                } else {
                    Some(pass_id)
                };
                return None;
            }
            EffectEditorEvent::Graph(message) => {
                let pass_id = state.open_pass?;
                EffectEditorMessage::Graph(pass_id, message)
            }
            EffectEditorEvent::PassAddRequested => {
                let pass_id = EffectPassId::new(Uuid::new_v4());
                state.open_pass = Some(pass_id);
                EffectEditorMessage::PassAddRequested(pass_id)
            }
            EffectEditorEvent::PassRemoveRequested(pass_id) => {
                if state.open_pass == Some(pass_id) {
                    state.open_pass = None;
                }
                if state.renaming_pass == Some(pass_id) {
                    state.renaming_pass = None;
                }
                EffectEditorMessage::PassRemoveRequested(pass_id)
            }
            EffectEditorEvent::Apply(message) => message,
        };
        Some((self.on_change)(message))
    }

    fn view(&self, state: &Self::State) -> GraphElement<'a, Self::Event> {
        match state.open_pass {
            Some(pass_id) if self.instance.passes.contains_key(&pass_id) => {
                view_pass_graph(self.instance, pass_id)
            }
            _ => view_pass_list(self.instance, state),
        }
    }
}

/// Internal UI state for [`EffectEditor`].
///
/// `EffectEditorState` tracks whether the user is viewing the top-level pass list or a specific
/// pass's [`GraphEditor`], as well as which pass is currently being renamed inline.
#[derive(Default)]
pub struct EffectEditorState {
    pub open_pass: Option<EffectPassId>,
    pub renaming_pass: Option<EffectPassId>,
}

/// Internal component event handled by [`EffectEditor`].
#[derive(Clone, Debug)]
pub enum EffectEditorEvent {
    OpenPass(EffectPassId),
    BackToPassList,
    PassRenameToggled(EffectPassId),
    Graph(GraphEditorMessage),
    PassAddRequested,
    PassRemoveRequested(EffectPassId),
    Apply(EffectEditorMessage),
}

/// External mutation message emitted by [`EffectEditor`] for pass management, dispatch
/// configuration, I/O editing, and graph edits.
#[derive(Clone, Debug)]
pub enum EffectEditorMessage {
    Graph(EffectPassId, GraphEditorMessage),
    Io(io::EffectIoEditorMessage),
    PassRenamed(EffectPassId, String),
    PassDispatchStrategySelected(EffectPassId, EffectPassDispatchStrategy),
    PassMoveRequested { index: usize, up: bool },
    PassRemoveRequested(EffectPassId),
    PassAddRequested(EffectPassId),
}

impl EffectInstance {
    pub fn update(&mut self, message: EffectEditorMessage, resources: &GraphResources) {
        match message {
            EffectEditorMessage::Graph(pass_id, message) => {
                let Some(pass) = self.passes.get_mut(&pass_id) else {
                    return;
                };
                pass.graph.update(message);
                resync(self);
            }
            EffectEditorMessage::Io(message) => {
                io::apply(self, message);
                resync(self);
            }
            EffectEditorMessage::PassRenamed(pass_id, name) => {
                if let Some(pass) = self.passes.get_mut(&pass_id) {
                    pass.name = name;
                }
                resync(self);
            }
            EffectEditorMessage::PassDispatchStrategySelected(pass_id, strategy) => {
                if self.passes.contains_key(&pass_id)
                    && dispatch_choices(self, pass_id)
                        .iter()
                        .any(|choice| choice.strategy == strategy)
                {
                    self.passes.get_mut(&pass_id).unwrap().dispatch_strategy = strategy;
                    resync(self);
                }
            }
            EffectEditorMessage::PassMoveRequested { index, up } => {
                io::move_slot(&mut self.passes, index, up);
            }
            EffectEditorMessage::PassRemoveRequested(pass_id) => {
                self.passes.shift_remove(&pass_id);
                resync(self);
            }
            EffectEditorMessage::PassAddRequested(pass_id) => {
                let name = format!("Pass {}", self.passes.len() + 1);
                self.passes.insert(
                    pass_id,
                    EffectPass {
                        name,
                        graph: Graph::new(resources.clone()),
                        dispatch_strategy: EffectPassDispatchStrategy::Once,
                    },
                );
            }
        }
    }
}

fn view_pass_list<'a>(
    instance: &'a EffectInstance,
    state: &EffectEditorState,
) -> GraphElement<'a, EffectEditorEvent> {
    let pass_count = instance.passes.len();
    let mut pass_list = column!().gap(8.0);
    for (index, pass_id) in instance.passes.keys().enumerate() {
        pass_list = pass_list.push(pass_card(instance, state, *pass_id, index, pass_count));
    }
    pass_list = pass_list.push(
        button(row![icon::plus().size(13), label(t!("add_pass"))].gap(4.0))
            .outline()
            .on_press(EffectEditorEvent::PassAddRequested),
    );

    let passes_panel = panel(
        column![
            label(t!("passes")).strong(),
            scrollable(pass_list).height(Length::Fill),
        ]
        .gap(6.0),
    )
    .padding(8)
    .width(Length::Fill);

    passes_panel.height(Length::Fill).into()
}

fn pass_card<'a>(
    instance: &'a EffectInstance,
    state: &EffectEditorState,
    pass_id: EffectPassId,
    index: usize,
    pass_count: usize,
) -> GraphElement<'a, EffectEditorEvent> {
    let pass = &instance.passes[&pass_id];
    let io = instance.pass_io_labels(&pass_id);
    let body = row![
        io_column(t!("inputs"), &io.inputs),
        io_column(t!("outputs"), &io.outputs),
    ]
    .gap(8.0);

    let renaming = state.renaming_pass == Some(pass_id);
    let content = if renaming {
        GraphElement::from(
            row![
                text_input("", &pass.name)
                    .on_input(move |name| {
                        EffectEditorEvent::Apply(EffectEditorMessage::PassRenamed(pass_id, name))
                    })
                    .width(Length::Fill),
                icon_button(icon::check().size(13))
                    .on_press(EffectEditorEvent::PassRenameToggled(pass_id)),
            ]
            .gap(4.0)
            .width(Length::Fill),
        )
    } else {
        GraphElement::from(
            button(column![label(pass.name.clone()).strong(), body].gap(4.0))
                .width(Length::Fill)
                .height(Length::Shrink)
                .on_press(EffectEditorEvent::OpenPass(pass_id)),
        )
    };

    let mut controls = column!().gap(2.0).push(
        button(icon::trash().size(13))
            .width(24)
            .height(24)
            .padding(5)
            .danger()
            .on_press(EffectEditorEvent::PassRemoveRequested(pass_id)),
    );
    if index > 0 {
        controls = controls.push(icon_button(icon::chevron_up().size(13)).on_press(
            EffectEditorEvent::Apply(EffectEditorMessage::PassMoveRequested { index, up: true }),
        ));
    }
    if index + 1 < pass_count {
        controls = controls.push(icon_button(icon::chevron_down().size(13)).on_press(
            EffectEditorEvent::Apply(EffectEditorMessage::PassMoveRequested { index, up: false }),
        ));
    }
    controls = controls.push(
        icon_button(icon::pencil().size(13))
            .on_press(EffectEditorEvent::PassRenameToggled(pass_id)),
    );

    row![content, controls].gap(4.0).into()
}

fn io_column<'a>(title: String, labels: &[String]) -> GraphElement<'a, EffectEditorEvent> {
    let rows = labels
        .iter()
        .map(|label| lapiz_widgets::label(format!("• {label}")).faint().into())
        .collect::<Vec<_>>();
    column![label(title).muted(), Flex::column(rows)]
        .gap(2.0)
        .width(Length::Fill)
        .into()
}

fn view_pass_graph<'a>(
    instance: &'a EffectInstance,
    pass_id: EffectPassId,
) -> GraphElement<'a, EffectEditorEvent> {
    let pass = &instance.passes[&pass_id];
    let back = button(row![icon::chevron_left().size(13), label(t!("back_to_passes"))].gap(4.0))
        .on_press(EffectEditorEvent::BackToPassList);
    let choices = dispatch_choices(instance, pass_id);
    let selected_kind = DispatchStrategyKind::of(pass.dispatch_strategy);
    let kinds = [
        DispatchStrategyKind::Once,
        DispatchStrategyKind::EveryBufferElement,
        DispatchStrategyKind::EveryOutputLayerPixel,
        DispatchStrategyKind::EveryInputLayerPixel,
    ]
    .into_iter()
    .filter(|kind| {
        *kind == selected_kind
            || choices
                .iter()
                .any(|choice| DispatchStrategyKind::of(choice.strategy) == *kind)
    })
    .collect::<Vec<_>>();
    let targets = choices
        .iter()
        .filter(|choice| DispatchStrategyKind::of(choice.strategy) == selected_kind)
        .cloned()
        .collect::<Vec<_>>();
    let selected_target = targets
        .iter()
        .find(|choice| choice.strategy == pass.dispatch_strategy)
        .cloned();
    let dispatch_strategy = combo_box(kinds, Some(selected_kind), move |kind| {
        let strategy = if kind == selected_kind {
            pass.dispatch_strategy
        } else {
            choices
                .iter()
                .find(|choice| DispatchStrategyKind::of(choice.strategy) == kind)
                .expect("available dispatch strategies have a compatible target")
                .strategy
        };
        EffectEditorEvent::Apply(EffectEditorMessage::PassDispatchStrategySelected(
            pass_id, strategy,
        ))
    });

    let mut header = row![back, dispatch_strategy].gap(8.0);
    if selected_kind != DispatchStrategyKind::Once {
        header = header.push(
            combo_box(targets, selected_target, move |choice: DispatchChoice| {
                EffectEditorEvent::Apply(EffectEditorMessage::PassDispatchStrategySelected(
                    pass_id,
                    choice.strategy,
                ))
            })
            .placeholder(t!("dispatch_target")),
        );
    }
    header = header.push(label(pass.name.clone()).strong());

    column![
        header,
        component(GraphEditor::new(&pass.graph, EffectEditorEvent::Graph)),
    ]
    .gap(4.0)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DispatchStrategyKind {
    Once,
    EveryBufferElement,
    EveryOutputLayerPixel,
    EveryInputLayerPixel,
}

impl DispatchStrategyKind {
    fn of(strategy: EffectPassDispatchStrategy) -> Self {
        match strategy {
            EffectPassDispatchStrategy::Once => Self::Once,
            EffectPassDispatchStrategy::EveryBufferElement(_) => Self::EveryBufferElement,
            EffectPassDispatchStrategy::EveryOutputLayerPixel(_) => Self::EveryOutputLayerPixel,
            EffectPassDispatchStrategy::EveryInputLayerPixel(_) => Self::EveryInputLayerPixel,
        }
    }
}

impl fmt::Display for DispatchStrategyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&match self {
            Self::Once => t!("dispatch_once"),
            Self::EveryBufferElement => t!("dispatch_every_buffer_element"),
            Self::EveryOutputLayerPixel => t!("dispatch_every_output_layer_pixel"),
            Self::EveryInputLayerPixel => t!("dispatch_every_input_layer_pixel"),
        })
    }
}

#[derive(Clone, PartialEq)]
struct DispatchChoice {
    label: String,
    strategy: EffectPassDispatchStrategy,
}

impl fmt::Display for DispatchChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

fn dispatch_choices(instance: &EffectInstance, pass_id: EffectPassId) -> Vec<DispatchChoice> {
    let pass = &instance.passes[&pass_id];
    let mut choices = vec![DispatchChoice {
        label: t!("dispatch_once"),
        strategy: EffectPassDispatchStrategy::Once,
    }];

    for node in pass.graph.iter_nodes() {
        if let Some(input) = node.data.as_input_port() {
            let Some(ty) = &input.ty else {
                continue;
            };
            let strategy = if ty.is::<LayerType>() {
                EffectPassDispatchStrategy::EveryInputLayerPixel(input.id)
            } else if ty.is::<ArrayType>()
                || ty.is::<ArrayAtomicI32Type>()
                || ty.is::<ArrayAtomicU32Type>()
            {
                EffectPassDispatchStrategy::EveryBufferElement(input.id)
            } else {
                continue;
            };
            choices.push(DispatchChoice {
                label: input.label,
                strategy,
            });
        } else if let Some(output) = node.data.as_output_port() {
            if !output.ty.as_ref().is_some_and(|ty| ty.is::<LayerType>()) {
                continue;
            }
            choices.push(DispatchChoice {
                label: output.label,
                strategy: EffectPassDispatchStrategy::EveryOutputLayerPixel(output.id),
            });
        }
    }
    choices
}

fn resync(instance: &mut EffectInstance) {
    instance
        .sync_pass_graph_effect_properties()
        .expect("instances always sync");
}
