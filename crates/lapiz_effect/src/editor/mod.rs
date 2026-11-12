use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use iced_core::Length;
use lapiz_i18n::t;
use lapiz_shader_graph::{
    GraphElement,
    editor::{GraphEditor, GraphEditorMessage, GraphEditorState},
    graph::{Graph, GraphResources},
    wgsl_std::types::{
        atomic::{ArrayAtomicI32Type, ArrayAtomicU32Type},
        handle::{ArrayType, LayerType},
    },
};
use lapiz_widgets::{
    button, column, combo_box, flex::Flex, icon, icon_button, label, panel, row, scrollable,
    text_input,
};
use uuid::Uuid;

use crate::{
    asset::{EffectPassDispatchStrategy, EffectPassId, EffectPassOutputSlotId},
    instance::{EffectInstance, EffectPass},
    nodes::{PassInput, PassInputNode, PassOutput, PassOutputChoiceTarget, PassOutputNode},
};

pub mod io;

#[cfg(test)]
mod tests;

/// Persistent UI state for editing an [`EffectInstance`] in [`EffectEditorView`].
///
/// `EffectEditorState` tracks whether the user is viewing the top-level pass list or a specific
/// pass's [`GraphEditor`], maintains per-pass [`GraphEditorState`]s, and resynchronizes pass I/O
/// bindings after every edit.
pub struct EffectEditorState {
    pub open_pass: Option<EffectPassId>,
    pub graph_editor_states: HashMap<EffectPassId, GraphEditorState>,
    pub resources: GraphResources,
    pub renaming_pass: Option<EffectPassId>,
}

impl EffectEditorState {
    pub fn new(resources: GraphResources) -> Self {
        Self {
            open_pass: None,
            graph_editor_states: HashMap::new(),
            resources,
            renaming_pass: None,
        }
    }

    pub fn update(&mut self, instance: &mut EffectInstance, message: EffectEditorMessage) {
        match message {
            EffectEditorMessage::OpenPass(pass_id) => {
                self.graph_editor_states.entry(pass_id).or_default();
                self.open_pass = Some(pass_id);
            }
            EffectEditorMessage::BackToPassList => self.open_pass = None,
            EffectEditorMessage::Graph(message) => {
                let pass_id = self
                    .open_pass
                    .expect("graph messages only arrive while a pass is open");
                let pass = instance
                    .passes
                    .get_mut(&pass_id)
                    .expect("the open pass exists");
                let editor_state = self
                    .graph_editor_states
                    .get_mut(&pass_id)
                    .expect("graph editor state is created when a pass is opened");
                editor_state.update(&mut pass.graph, message);
                resync(instance);
            }
            EffectEditorMessage::Io(message) => {
                io::apply(instance, message);
                resync(instance);
            }
            EffectEditorMessage::PassRenamed(pass_id, name) => {
                if let Some(pass) = instance.passes.get_mut(&pass_id) {
                    pass.name = name;
                }
                resync(instance);
            }
            EffectEditorMessage::PassDispatchStrategySelected(pass_id, strategy) => {
                if instance.passes.contains_key(&pass_id)
                    && dispatch_choices(instance, pass_id)
                        .iter()
                        .any(|choice| choice.strategy == strategy)
                {
                    instance.passes.get_mut(&pass_id).unwrap().dispatch_strategy = strategy;
                    resync(instance);
                }
            }
            EffectEditorMessage::PassRenameToggled(pass_id) => {
                self.renaming_pass = if self.renaming_pass == Some(pass_id) {
                    None
                } else {
                    Some(pass_id)
                };
            }
            EffectEditorMessage::PassMoveRequested { index, up } => {
                io::move_slot(&mut instance.passes, index, up);
            }
            EffectEditorMessage::PassRemoveRequested(pass_id) => {
                instance.passes.shift_remove(&pass_id);
                self.graph_editor_states.remove(&pass_id);
                if self.open_pass == Some(pass_id) {
                    self.open_pass = None;
                }
                resync(instance);
            }
            EffectEditorMessage::PassAddRequested => {
                let pass_id = EffectPassId::new(Uuid::new_v4());
                let name = format!("Pass {}", instance.passes.len() + 1);
                instance.passes.insert(
                    pass_id,
                    EffectPass {
                        name,
                        graph: Graph::new(self.resources.clone()),
                        dispatch_strategy: EffectPassDispatchStrategy::Once,
                    },
                );
                self.graph_editor_states.entry(pass_id).or_default();
                self.open_pass = Some(pass_id);
            }
        }
    }
}

/// UI message emitted by [`EffectEditorView`] for pass management, dispatch configuration, I/O editing, and graph edits.
#[derive(Debug, Clone)]
pub enum EffectEditorMessage {
    OpenPass(EffectPassId),
    BackToPassList,
    Graph(GraphEditorMessage),
    Io(io::EffectIoEditorMessage),
    PassRenamed(EffectPassId, String),
    PassDispatchStrategySelected(EffectPassId, EffectPassDispatchStrategy),
    PassRenameToggled(EffectPassId),
    PassMoveRequested { index: usize, up: bool },
    PassRemoveRequested(EffectPassId),
    PassAddRequested,
}

/// A view builder that renders either the multi-pass overview list or the active pass's [`GraphEditor`] for an [`EffectInstance`].
pub struct EffectEditorView<'a> {
    instance: &'a EffectInstance,
    state: &'a EffectEditorState,
}

impl<'a> EffectEditorView<'a> {
    pub fn new(instance: &'a EffectInstance, state: &'a EffectEditorState) -> Self {
        Self { instance, state }
    }
}

impl<'a> From<EffectEditorView<'a>> for GraphElement<'a, EffectEditorMessage> {
    fn from(value: EffectEditorView<'a>) -> Self {
        let EffectEditorView { instance, state } = value;
        match state.open_pass {
            Some(pass_id) => view_pass_graph(instance, state, pass_id),
            None => view_pass_list(instance, state),
        }
    }
}

fn view_pass_list<'a>(
    instance: &'a EffectInstance,
    state: &'a EffectEditorState,
) -> GraphElement<'a, EffectEditorMessage> {
    let pass_count = instance.passes.len();
    let mut pass_list = column!().gap(8.0);
    for (index, pass_id) in instance.passes.keys().enumerate() {
        pass_list = pass_list.push(pass_card(instance, state, *pass_id, index, pass_count));
    }
    pass_list = pass_list.push(
        button(row![icon::plus().size(13), label(t!("add_pass"))].gap(4.0))
            .outline()
            .on_press(EffectEditorMessage::PassAddRequested),
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
    state: &'a EffectEditorState,
    pass_id: EffectPassId,
    index: usize,
    pass_count: usize,
) -> GraphElement<'a, EffectEditorMessage> {
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
                    .on_input(move |name| EffectEditorMessage::PassRenamed(pass_id, name))
                    .width(Length::Fill),
                icon_button(icon::check().size(13))
                    .on_press(EffectEditorMessage::PassRenameToggled(pass_id)),
            ]
            .gap(4.0)
            .width(Length::Fill),
        )
    } else {
        GraphElement::from(
            button(column![label(pass.name.clone()).strong(), body].gap(4.0))
                .width(Length::Fill)
                .height(Length::Shrink)
                .on_press(EffectEditorMessage::OpenPass(pass_id)),
        )
    };

    let mut controls = column!().gap(2.0).push(
        button(icon::trash().size(13))
            .width(24)
            .height(24)
            .padding(5)
            .danger()
            .on_press(EffectEditorMessage::PassRemoveRequested(pass_id)),
    );
    if index > 0 {
        controls = controls.push(
            icon_button(icon::chevron_up().size(13))
                .on_press(EffectEditorMessage::PassMoveRequested { index, up: true }),
        );
    }
    if index + 1 < pass_count {
        controls = controls.push(
            icon_button(icon::chevron_down().size(13))
                .on_press(EffectEditorMessage::PassMoveRequested { index, up: false }),
        );
    }
    controls = controls.push(
        icon_button(icon::pencil().size(13))
            .on_press(EffectEditorMessage::PassRenameToggled(pass_id)),
    );

    row![content, controls].gap(4.0).into()
}

fn io_column<'a>(title: String, labels: &[String]) -> GraphElement<'a, EffectEditorMessage> {
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
    state: &'a EffectEditorState,
    pass_id: EffectPassId,
) -> GraphElement<'a, EffectEditorMessage> {
    let pass = &instance.passes[&pass_id];
    let graph_state = state
        .graph_editor_states
        .get(&pass_id)
        .expect("graph editor state is created when a pass is opened");
    let back = button(row![icon::chevron_left().size(13), label(t!("back_to_passes"))].gap(4.0))
        .transparent()
        .on_press(EffectEditorMessage::BackToPassList);
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
        EffectEditorMessage::PassDispatchStrategySelected(pass_id, strategy)
    });

    let mut header = row![back, dispatch_strategy].gap(8.0);
    if selected_kind != DispatchStrategyKind::Once {
        header = header.push(
            combo_box(targets, selected_target, move |choice: DispatchChoice| {
                EffectEditorMessage::PassDispatchStrategySelected(pass_id, choice.strategy)
            })
            .placeholder(t!("dispatch_target")),
        );
    }
    header = header.push(label(pass.name.clone()).strong());

    column![
        header,
        GraphElement::from(GraphEditor::new(&pass.graph, graph_state))
            .map(EffectEditorMessage::Graph),
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
        if let Some(state) = node.data.state::<PassInputNode>() {
            let (Some(input), Some(ty)) = (state.input, &state.cached_ty) else {
                continue;
            };
            let strategy = if ty.is::<LayerType>() {
                EffectPassDispatchStrategy::EveryInputLayerPixel(state.id)
            } else if ty.is::<ArrayType>()
                || ty.is::<ArrayAtomicI32Type>()
                || ty.is::<ArrayAtomicU32Type>()
            {
                EffectPassDispatchStrategy::EveryBufferElement(state.id)
            } else {
                continue;
            };
            let label = state
                .available_sources
                .iter()
                .find(|choice| choice.input == Some(input))
                .expect("synced options contain the current binding")
                .label
                .clone();
            choices.push(DispatchChoice { label, strategy });
        } else if let Some(state) = node.data.state::<PassOutputNode>() {
            if !state
                .cached_ty
                .as_ref()
                .is_some_and(|ty| ty.is::<LayerType>())
            {
                continue;
            }
            let label = match &state.output {
                Some(PassOutput::Pass(def)) => format!("{} ({})", def.name, def.ty.id().id),
                Some(PassOutput::Effect(id)) => state
                    .available_targets
                    .iter()
                    .find(|choice| choice.target == PassOutputChoiceTarget::Effect(*id))
                    .expect("synced options contain the current binding")
                    .label
                    .clone(),
                None => continue,
            };
            choices.push(DispatchChoice {
                label,
                strategy: EffectPassDispatchStrategy::EveryOutputLayerPixel(state.id),
            });
        }
    }
    choices
}

fn resync(instance: &mut EffectInstance) {
    sanitize_bindings(instance);
    instance
        .sync_pass_graph_effect_properties()
        .expect("sanitized instances always sync");
}

fn sanitize_bindings(instance: &mut EffectInstance) {
    let effect_inputs = instance.inputs.keys().copied().collect::<HashSet<_>>();
    let effect_outputs = instance.outputs.keys().copied().collect::<HashSet<_>>();
    let pass_outputs = instance
        .passes
        .values()
        .flat_map(|pass| pass.graph.iter_nodes())
        .filter_map(|node| node.data.state::<PassOutputNode>())
        .map(|state| state.id)
        .collect::<HashSet<EffectPassOutputSlotId>>();

    for pass in instance.passes.values_mut() {
        for node in pass.graph.iter_nodes_mut() {
            if let Some(state) = node.data.state_mut::<PassInputNode>()
                && let Some(input) = state.input
                && match input {
                    PassInput::Effect(id) => !effect_inputs.contains(&id),
                    PassInput::Pass(id) => !pass_outputs.contains(&id),
                }
            {
                state.input = None;
            }
            if let Some(state) = node.data.state_mut::<PassOutputNode>()
                && let Some(PassOutput::Effect(id)) = &state.output
                && !effect_outputs.contains(id)
            {
                state.output = None;
            }
        }
    }
}
