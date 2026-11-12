use std::{
    collections::HashMap,
    fmt, iter,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use anyhow::anyhow;
use iced_core::Length;
use indexmap::IndexMap;
use lapiz_i18n::t;
use lapiz_shader_graph_derive::stateless;
use lapiz_utils::{random_oklch_hue_chroma, wrapper};
use lapiz_widgets::{
    button, column, combo_box, container, flex::Flex, fluent_builder::When as _, label, popover,
    row, text_input, text_input::default,
};
use parking_lot::Mutex;
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wesl::syntax::{
    AssignmentOperator, AssignmentStatement, Declaration, DeclarationKind, Ident, Span, Spanned,
    Statement,
};
use wesl_quote::quote_statement;

use crate::{
    GraphElement,
    graph::{
        Graph, GraphResources, GraphVarIdentGenerator,
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeRegistry, GraphNodeUpdateContext,
            GraphNodeUpdateSignatureContext, GraphNodeViewContext, StatelessCommonGraphNode,
        },
        slot::{
            ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphDefaultInputSlot,
            GraphDefaultOutputSlot, GraphValueType,
        },
    },
    save::{GraphSerializable, SerializableGraph},
    wgsl_std::types::primitive::I32Type,
};

static UNIQUE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Shader graph node available inside a [`RepeatNode`] subgraph that outputs the current zero-based loop iteration index ([`I32Type`]).
#[derive(Default, Clone)]
pub struct RepeatIterationNode;

#[stateless]
impl StatelessCommonGraphNode for RepeatIterationNode {
    fn id(&self) -> &'static str {
        "repeat_iteration_node"
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RepeatIterationNode)
    }

    fn create_inputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultInputSlot> {
        Vec::new()
    }

    fn create_outputs(&self, _: GraphNodeCreateSlotsContext<'_>) -> Vec<GraphDefaultOutputSlot> {
        vec![GraphDefaultOutputSlot::new::<I32Type>("iteration".into())]
    }

    fn update_signature(&self, mut ctx: GraphNodeUpdateSignatureContext<'_>) {
        ctx.require_output_slot_as_graph_input(0, "Iteration".into());
    }

    fn generate_code(
        &self,
        _: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(String::new())
    }
}

wrapper! {
    /// A strongly-typed unique identifier for a loop-carried variable in a [`RepeatNode`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    pub RepeatVariableId : Uuid
}

/// Definition of a loop-carried local variable managed by a [`RepeatNode`].
#[derive(Clone)]
pub struct RepeatLocalSchema {
    pub id: RepeatVariableId,
    pub name: String,
    pub ty: Arc<dyn ErasedGraphValueType>,
}

/// Serializable DTO representation of a [`RepeatLocalSchema`].
#[derive(Clone, Serialize, Deserialize)]
pub struct SerializableRepeatLocalSchema {
    pub id: RepeatVariableId,
    pub name: String,
    pub ty: String,
}

impl GraphSerializable for RepeatLocalSchema {
    fn to_toml(&self) -> anyhow::Result<toml::Value> {
        let serializable = SerializableRepeatLocalSchema {
            id: self.id,
            name: self.name.clone(),
            ty: self.ty.id().id,
        };
        Ok(toml::Value::try_from(serializable)?)
    }

    fn from_toml(value: toml::Value, resources: &GraphResources) -> anyhow::Result<Self> {
        let serializable = SerializableRepeatLocalSchema::deserialize(value)?;
        let ty = resources
            .type_registry
            .resolve_type(&serializable.ty)
            .ok_or_else(|| anyhow::anyhow!("Unknown type: {}", serializable.ty))?;
        Ok(RepeatLocalSchema {
            id: serializable.id,
            name: serializable.name,
            ty: ty.clone(),
        })
    }
}

#[derive(Clone)]
struct RepeatLocalSchemaDraft {
    pub id: RepeatVariableId,
    pub name: String,
    pub ty: Option<Arc<dyn ErasedGraphValueType>>,
}

#[derive(Clone, Default)]
struct RepeatSchemaDraft {
    locals: IndexMap<RepeatVariableId, RepeatLocalSchemaDraft>,
}

impl RepeatSchemaDraft {
    pub fn new(locals: &IndexMap<RepeatVariableId, RepeatLocalSchema>) -> Self {
        Self {
            locals: locals
                .iter()
                .map(|(id, schema)| {
                    (
                        *id,
                        RepeatLocalSchemaDraft {
                            id: *id,
                            name: schema.name.clone(),
                            ty: Some(schema.ty.clone()),
                        },
                    )
                })
                .collect(),
        }
    }

    pub fn finalize(&self) -> IndexMap<RepeatVariableId, RepeatLocalSchema> {
        self.locals
            .iter()
            .map(|(id, schema)| {
                (
                    *id,
                    RepeatLocalSchema {
                        id: *id,
                        name: schema.name.clone(),
                        ty: schema.ty.clone().unwrap(),
                    },
                )
            })
            .collect()
    }
}

/// Per-instance state for [`RepeatNode`], owning the loop-carried variable schemas and the loop body subgraph.
pub struct RepeatNodeState {
    locals: Arc<Mutex<IndexMap<RepeatVariableId, RepeatLocalSchema>>>,
    revision: u64,
    body: Graph,
    schema_draft: Option<RepeatSchemaDraft>,
}

impl RepeatNodeState {
    pub fn body(&self) -> &Graph {
        &self.body
    }

    pub fn body_mut(&mut self) -> &mut Graph {
        &mut self.body
    }

    pub fn add_local<T: GraphValueType + Default>(&mut self, name: String) -> RepeatVariableId {
        let id = RepeatVariableId::new(Uuid::new_v4());
        self.locals.lock().insert(
            id,
            RepeatLocalSchema {
                id,
                name,
                ty: Arc::new(T::default()),
            },
        );
        self.revision += 1;
        id
    }

    pub fn sync_body_nodes(&mut self) {
        let input_ids = self
            .body
            .nodes
            .iter()
            .filter(|(_, node)| node.data.state::<RepeatInputNode>().is_some())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let output_ids = self
            .body
            .nodes
            .iter()
            .filter(|(_, node)| node.data.state::<RepeatOutputNode>().is_some())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();

        let locals = self.locals.lock().clone();
        for id in input_ids {
            self.body.update_node_state::<RepeatInputNode>(id, |st| {
                if let Some(variable) = st.variable
                    && !locals.contains_key(&variable)
                {
                    st.variable = None;
                }
            });
        }
        for id in output_ids {
            self.body.update_node_state::<RepeatOutputNode>(id, |st| {
                if let Some(variable) = st.variable
                    && !locals.contains_key(&variable)
                {
                    st.variable = None;
                }
            });
        }
        self.body.invalidate_cache();
    }
}

#[derive(Serialize, Deserialize)]
struct SerializableRepeatNodeState {
    locals: Vec<SerializableRepeatLocalSchema>,
    body: SerializableGraph,
}

impl GraphSerializable for RepeatNodeState {
    fn to_toml(&self) -> anyhow::Result<toml::Value> {
        let locals = self
            .locals
            .lock()
            .values()
            .map(|local| SerializableRepeatLocalSchema {
                id: local.id,
                name: local.name.clone(),
                ty: local.ty.id().id,
            })
            .collect();
        let body = self.body.as_serialized()?;
        Ok(toml::Value::try_from(SerializableRepeatNodeState {
            locals,
            body,
        })?)
    }

    fn from_toml(value: toml::Value, resources: &GraphResources) -> anyhow::Result<Self> {
        let serialized = SerializableRepeatNodeState::deserialize(value)?;
        let locals =
            serialized
                .locals
                .into_iter()
                .try_fold(IndexMap::new(), |mut locals, local| {
                    locals.insert(
                        local.id,
                        RepeatLocalSchema {
                            id: local.id,
                            name: local.name,
                            ty: resources
                                .type_registry
                                .resolve_type(&local.ty)
                                .ok_or_else(|| anyhow!("Type {} not found in registry", local.ty))?
                                .clone(),
                        },
                    );

                    Result::<_, anyhow::Error>::Ok(locals)
                })?;
        let locals = Arc::new(Mutex::new(locals));

        let repeat_node_extra = {
            let mut r = GraphNodeRegistry::default();
            r.register_boxed(Box::new(RepeatInputNode {
                locals: locals.clone(),
            }));
            r.register_boxed(Box::new(RepeatOutputNode {
                locals: locals.clone(),
            }));
            r.register::<RepeatIterationNode>();
            r
        };

        let mut node_registry = resources.node_registry.as_ref().clone();
        node_registry.merge(repeat_node_extra);

        let body_resources = GraphResources {
            type_registry: resources.type_registry.clone(),
            node_registry: Arc::new(node_registry),
            assets: resources.assets.clone(),
        };
        let (body, errors) = Graph::from_serialized(&serialized.body, body_resources);
        if !errors.is_empty() {
            return Err(anyhow!("Repeat body deserialization failed: {errors:?}"));
        }
        let body = body.ok_or_else(|| anyhow!("Repeat body is missing"))?;

        let mut state = Self {
            locals,
            revision: 0,
            body,
            schema_draft: None,
        };
        state.sync_body_nodes();
        Ok(state)
    }
}

/// Shader graph node inside a [`RepeatNode`] body subgraph that reads the current iteration value of a loop-carried variable.
#[derive(Default, Clone)]
pub struct RepeatInputNode {
    pub locals: Arc<Mutex<IndexMap<RepeatVariableId, RepeatLocalSchema>>>,
}

/// Per-instance state for [`RepeatInputNode`], storing the selected [`RepeatVariableId`].
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RepeatInputNodeState {
    pub variable: Option<RepeatVariableId>,
}

/// UI message emitted by [`RepeatInputNode`].
#[derive(Clone)]
pub enum RepeatInputNodeMessage {
    VariableChanged(RepeatVariableId),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for RepeatInputNode {
    type State = RepeatInputNodeState;
    type Message = RepeatInputNodeMessage;

    fn id(&self) -> &'static str {
        "repeat_input_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        RepeatInputNodeState::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RepeatInputNode)
    }

    fn create_inputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        Vec::new()
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        let locals = self.locals.lock();
        let Some(local) = state.variable.as_ref().and_then(|id| locals.get(id)) else {
            return Vec::new();
        };
        vec![GraphDefaultOutputSlot::new_boxed(
            format!("{} Current", local.name),
            local.ty.clone(),
        )]
    }

    fn update_signature(&self, state: &Self::State, mut ctx: GraphNodeUpdateSignatureContext<'_>) {
        let locals = self.locals.lock();
        let Some(local) = state.variable.as_ref().and_then(|id| locals.get(id)) else {
            return;
        };
        ctx.require_output_slot_as_graph_input(0, local.name.clone());
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'static, Self::Message> {
        let locals = repeat_variable_references(&self.locals.lock());
        let selected = state
            .variable
            .and_then(|id| locals.iter().find(|reference| reference.id == id).cloned());
        ctx.view_all_slots_with_header(
            combo_box(locals, selected, |reference| {
                RepeatInputNodeMessage::VariableChanged(reference.id)
            })
            .width(Length::Fill),
            RepeatInputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            RepeatInputNodeMessage::VariableChanged(variable) => state.variable = Some(variable),
            RepeatInputNodeMessage::LiteralUpdate(literal) => ctx.update_literal(literal),
        }
    }

    fn generate_code(
        &self,
        _: &Self::State,
        _: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(String::new())
    }
}

/// Shader graph node inside a [`RepeatNode`] body subgraph that writes the next iteration value of a loop-carried variable.
#[derive(Default, Clone)]
pub struct RepeatOutputNode {
    pub locals: Arc<Mutex<IndexMap<RepeatVariableId, RepeatLocalSchema>>>,
}

/// Per-instance state for [`RepeatOutputNode`], storing the selected [`RepeatVariableId`].
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct RepeatOutputNodeState {
    pub variable: Option<RepeatVariableId>,
}

/// UI message emitted by [`RepeatOutputNode`].
#[derive(Clone)]
pub enum RepeatOutputNodeMessage {
    VariableChanged(RepeatVariableId),
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

impl GraphNode for RepeatOutputNode {
    type State = RepeatOutputNodeState;
    type Message = RepeatOutputNodeMessage;

    fn id(&self) -> &'static str {
        "repeat_output_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        RepeatOutputNodeState::default()
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RepeatOutputNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        let locals = self.locals.lock();
        let Some(local) = state.variable.as_ref().and_then(|id| locals.get(id)) else {
            return Vec::new();
        };
        vec![GraphDefaultInputSlot::new_boxed(
            format!("{} Next", local.name),
            local.ty.clone(),
        )]
    }

    fn create_outputs(
        &self,
        _: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        Vec::new()
    }

    fn update_signature(&self, state: &Self::State, mut ctx: GraphNodeUpdateSignatureContext<'_>) {
        let locals = self.locals.lock();
        let Some(local) = state.variable.as_ref().and_then(|id| locals.get(id)) else {
            return;
        };
        ctx.require_input_slot_as_graph_output(0, local.name.clone());
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'static, Self::Message> {
        let locals = repeat_variable_references(&self.locals.lock());
        let selected = state
            .variable
            .and_then(|id| locals.iter().find(|reference| reference.id == id).cloned());
        ctx.view_all_slots_with_header(
            combo_box(locals, selected, |reference| {
                RepeatOutputNodeMessage::VariableChanged(reference.id)
            })
            .width(Length::Fill),
            RepeatOutputNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            RepeatOutputNodeMessage::VariableChanged(variable) => state.variable = Some(variable),
            RepeatOutputNodeMessage::LiteralUpdate(literal) => ctx.update_literal(literal),
        }
    }

    fn generate_code(
        &self,
        _: &Self::State,
        _: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        Ok(String::new())
    }
}

#[derive(Clone)]
struct RepeatVariableReference {
    id: RepeatVariableId,
    name: String,
}

impl fmt::Display for RepeatVariableReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name.fmt(f)
    }
}

impl PartialEq for RepeatVariableReference {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

fn repeat_variable_references(
    locals: &IndexMap<RepeatVariableId, RepeatLocalSchema>,
) -> Vec<RepeatVariableReference> {
    locals
        .values()
        .map(|local| RepeatVariableReference {
            id: local.id,
            name: local.name.clone(),
        })
        .collect()
}

/// A shader graph node that executes an embedded body subgraph in a WGSL `for` loop with loop-carried state variables.
///
/// `RepeatNode` exposes an `iterations` count slot alongside an initial input slot (`<Name> In`) and
/// final output slot (`<Name> Out`) for each configured [`RepeatLocalSchema`]. Inside the body
/// subgraph, [`RepeatInputNode`], [`RepeatOutputNode`], and [`RepeatIterationNode`] read and update
/// the loop-carried variables across iterations.
#[derive(Default, Clone)]
pub struct RepeatNode;

/// UI message emitted by [`RepeatNode`] for editing its loop-carried variable schema.
#[derive(Clone)]
pub enum RepeatNodeMessage {
    ToggleEditor,
    EditorAddLocal,
    EditorRemoveLocal(RepeatVariableId),
    EditorMoveLocalUp(RepeatVariableId),
    EditorMoveLocalDown(RepeatVariableId),
    EditorRenameLocal(RepeatVariableId, String),
    EditorChangeLocalType(RepeatVariableId, String),
    EditorConfirm,
    EditorCancel,
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

fn repeat_schema_editor_view(
    state: &RepeatNodeState,
    resources: &GraphResources,
) -> GraphElement<'static, RepeatNodeMessage> {
    let type_names = resources
        .type_registry
        .all_types()
        .keys()
        .map(|id| id.id.clone())
        .collect::<Vec<_>>();
    let draft = state.schema_draft.as_ref().expect("editor must be open");

    let rows = draft
        .locals
        .values()
        .map(|local| {
            let id = local.id;
            column![
                text_input(&t!("variable_name"), &local.name)
                    .size(12.0)
                    .style(default)
                    .on_input(move |name| { RepeatNodeMessage::EditorRenameLocal(id, name) }),
                row![
                    combo_box(
                        type_names.clone(),
                        local.ty.as_ref().map(|ty| ty.id().id),
                        move |ty| { RepeatNodeMessage::EditorChangeLocalType(id, ty.to_string()) },
                    )
                    .width(Length::Fill),
                    button(label("Up")).on_press(RepeatNodeMessage::EditorMoveLocalUp(id)),
                    button(label("Down")).on_press(RepeatNodeMessage::EditorMoveLocalDown(id)),
                    button(label("Delete")).on_press(RepeatNodeMessage::EditorRemoveLocal(id)),
                ]
                .gap(4.0),
            ]
            .gap(4.0)
            .into()
        })
        .collect::<Vec<GraphElement<'static, RepeatNodeMessage>>>();

    let valid = draft
        .locals
        .values()
        .all(|local| !local.name.is_empty() && local.name.trim() == local.name)
        && draft.locals.values().all(|local| {
            draft
                .locals
                .values()
                .filter(|other| other.name == local.name)
                .count()
                == 1
        });

    let panel = Flex::column(rows)
        .width(Length::Fixed(300.0))
        .padding(4)
        .gap(6.0)
        .push(
            row![button(label("Add Variable")).on_press(RepeatNodeMessage::EditorAddLocal)]
                .gap(6.0),
        )
        .push(
            row![
                button(label("Cancel")).on_press(RepeatNodeMessage::EditorCancel),
                button(label("Confirm"))
                    .when(valid, |b| b.on_press(RepeatNodeMessage::EditorConfirm))
            ]
            .gap(4.0),
        );

    container(panel)
        .style(|theme| container::Style {
            background: Some(theme.palette().background.base.color.into()),
            ..container::transparent(theme)
        })
        .into()
}

impl GraphNode for RepeatNode {
    type State = RepeatNodeState;

    type Message = RepeatNodeMessage;

    fn id(&self) -> &'static str {
        "repeat_node"
    }

    fn default_state(&self, ctx: GraphNodeDefaultStateContext<'_>) -> Self::State {
        let locals = Arc::new(Mutex::new(IndexMap::new()));
        let repeat_node_extra = {
            let mut r = GraphNodeRegistry::default();
            r.register_boxed(Box::new(RepeatInputNode {
                locals: locals.clone(),
            }));
            r.register_boxed(Box::new(RepeatOutputNode {
                locals: locals.clone(),
            }));
            r.register::<RepeatIterationNode>();
            r
        };

        let mut node_registry = ctx.resources.node_registry.as_ref().clone();
        node_registry.merge(repeat_node_extra);

        let body_resources = GraphResources {
            type_registry: ctx.resources.type_registry.clone(),
            node_registry: Arc::new(node_registry),
            assets: ctx.resources.assets.clone(),
        };

        RepeatNodeState {
            locals,
            revision: 0,
            body: Graph::new(body_resources),
            schema_draft: None,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(RepeatNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        iter::once(GraphDefaultInputSlot::new::<I32Type>("iterations".into()))
            .chain(state.locals.lock().values().map(|local| {
                GraphDefaultInputSlot::new_boxed(format!("{} In", local.name), local.ty.clone())
            }))
            .collect()
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _ctx: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        state
            .locals
            .lock()
            .values()
            .map(|local| {
                GraphDefaultOutputSlot::new_boxed(format!("{} Out", local.name), local.ty.clone())
            })
            .collect()
    }

    fn view(
        &self,
        state: &Self::State,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'static, Self::Message> {
        let trigger = button(label("Edit")).on_press(RepeatNodeMessage::ToggleEditor);
        let content = state
            .schema_draft
            .as_ref()
            .map(|_| repeat_schema_editor_view(state, ctx.resources));
        let popover = popover(trigger).content(content);
        ctx.view_all_slots_with_header(popover, RepeatNodeMessage::LiteralUpdate)
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            RepeatNodeMessage::ToggleEditor => {
                if state.schema_draft.is_some() {
                    state.schema_draft = None;
                } else {
                    state.schema_draft = Some(RepeatSchemaDraft::new(&state.locals.lock()));
                }
            }
            RepeatNodeMessage::EditorAddLocal => {
                if let Some(draft) = &mut state.schema_draft {
                    let new_id = RepeatVariableId::new(Uuid::new_v4());
                    draft.locals.insert(
                        new_id,
                        RepeatLocalSchemaDraft {
                            id: new_id,
                            name: String::new(),
                            ty: None,
                        },
                    );
                }
            }
            RepeatNodeMessage::EditorRemoveLocal(id) => {
                if let Some(draft) = &mut state.schema_draft {
                    draft.locals.shift_remove(&id);
                }
            }
            RepeatNodeMessage::EditorMoveLocalUp(id) => {
                if let Some(draft) = &mut state.schema_draft
                    && let Some(index) = draft.locals.get_index_of(&id)
                    && index > 0
                {
                    draft.locals.swap_indices(index, index - 1);
                }
            }
            RepeatNodeMessage::EditorMoveLocalDown(id) => {
                if let Some(draft) = &mut state.schema_draft
                    && let Some(index) = draft.locals.get_index_of(&id)
                    && index + 1 < draft.locals.len()
                {
                    draft.locals.swap_indices(index, index + 1);
                }
            }
            RepeatNodeMessage::EditorRenameLocal(id, name) => {
                if let Some(draft) = &mut state.schema_draft
                    && let Some(local) = draft.locals.get_mut(&id)
                {
                    local.name = name;
                }
            }
            RepeatNodeMessage::EditorChangeLocalType(id, ty) => {
                if let Some(draft) = &mut state.schema_draft
                    && let Some(local) = draft.locals.get_mut(&id)
                {
                    local.ty = Some(
                        ctx.resources
                            .type_registry
                            .resolve_type(&ty)
                            .unwrap()
                            .clone(),
                    );
                }
            }
            RepeatNodeMessage::EditorConfirm => {
                let Some(draft) = &mut state.schema_draft else {
                    return;
                };
                *state.locals.lock() = draft.finalize();
                state.revision += 1;
                state.schema_draft = None;
                state.sync_body_nodes();
            }
            RepeatNodeMessage::EditorCancel => {
                state.schema_draft = None;
            }
            RepeatNodeMessage::LiteralUpdate(literal) => ctx.update_literal(literal),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        let locals = state.locals.lock().clone();
        if locals.len() + 1 != ctx.inputs.len() || locals.len() != ctx.outputs.len() {
            return Err(anyhow!("Repeat parent slot invariant is invalid").into());
        }

        let iterations = ctx.get_input(0)?;
        let body = &state.body;
        let signature = body.signature();

        let mut current = HashMap::with_capacity(locals.len());
        let mut code = String::new();
        for (index, local) in locals.values().enumerate() {
            let value = Ident::new(ctx.ident_generator.next_output());
            let input = ctx.get_input(index + 1)?;
            code.push_str(&quote_statement! { var #value = #input; }.to_string());
            code.push('\n');
            current.insert(local.id, value);
        }

        let iteration = Ident::new(ctx.ident_generator.next_output());
        let mut body_inputs = Vec::with_capacity(signature.inputs.len());
        for slot_id in signature.inputs.keys() {
            let slot = body
                .slots
                .get_output(slot_id)
                .ok_or(GraphNodeCodeGenError::MissingOutputSlot)?;
            let node = body.get_node(&slot.node_id).ok_or_else(|| {
                GraphNodeCodeGenError::Custom(anyhow!("Repeat body node is missing"))
            })?;
            if node.data.is::<RepeatIterationNode>() {
                body_inputs.push(iteration.clone().into());
                continue;
            }
            let variable = node
                .data
                .state::<RepeatInputNode>()
                .and_then(|state| state.variable)
                .ok_or_else(|| anyhow!("Repeat Input has an invalid variable"))?;
            body_inputs.push(
                current
                    .get(&variable)
                    .cloned()
                    .ok_or_else(|| anyhow!("Repeat variable {variable} is not a local"))?
                    .into(),
            );
        }

        let mut next_slots = HashMap::with_capacity(locals.len());
        for slot_id in signature.outputs.keys() {
            let slot = body
                .slots
                .get_input(slot_id)
                .ok_or(GraphNodeCodeGenError::MissingInputSlot)?;
            let node = body
                .get_node(&slot.node_id)
                .ok_or_else(|| anyhow!("Repeat body node is missing"))?;
            let variable = node
                .data
                .state::<RepeatOutputNode>()
                .and_then(|state| state.variable)
                .ok_or_else(|| anyhow!("Repeat Output has an invalid variable"))?;
            if next_slots.insert(variable, *slot_id).is_some() {
                return Err(anyhow!("Repeat variable {variable} has duplicate outputs").into());
            }
        }
        for local in locals.values() {
            if !next_slots.contains_key(&local.id) {
                return Err(anyhow!(
                    "Repeat body is missing a Repeat Output for variable '{}'",
                    local.name
                )
                .into());
            }
        }

        code.push_str(&format!(
            "for (var {iteration} = 0i; {iteration} < {iterations}; {iteration}++) {{\n"
        ));
        let (body_output_idents, _, body_code, _) = body
            .compile(
                body_inputs,
                GraphVarIdentGenerator::new(format!(
                    "repeat_{}",
                    UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed)
                )),
            )
            .map_err(|error| GraphNodeCodeGenError::Custom(error.into()))?;
        code.push_str(&body_code);

        let body_outputs = signature
            .outputs
            .keys()
            .copied()
            .zip(body_output_idents)
            .collect::<HashMap<_, _>>();
        for local in locals.values() {
            let input_slot_id = next_slots[&local.id];
            let next = body_outputs
                .get(&input_slot_id)
                .ok_or_else(|| anyhow!("Repeat body output value of {} is missing", local.name))?;
            let input_slot = body
                .slots
                .get_input(&input_slot_id)
                .ok_or(GraphNodeCodeGenError::MissingInputSlot)?;
            let next = if let Some(connected) = input_slot.connected {
                let output_slot = body
                    .slots
                    .get_output(&connected)
                    .ok_or(GraphNodeCodeGenError::MissingOutputSlot)?;
                if output_slot.data_ty.id() != input_slot.data.ty().id() {
                    ctx.resources
                        .type_registry
                        .try_wgsl_cast(
                            &*output_slot.data_ty,
                            input_slot.data.ty().as_ref(),
                            next.clone(),
                        )
                        .ok_or(GraphNodeCodeGenError::FailedToCastVariable)?
                } else {
                    next.clone()
                }
            } else {
                next.clone()
            };
            let value = current[&local.id].clone();
            code.push_str(&quote_statement! { #value = #next; }.to_string());
            code.push('\n');
        }

        code.push_str("}\n");

        for (slot_id, local) in ctx.outputs.iter().zip(locals.values()) {
            ctx.output_slot_idents
                .insert(*slot_id, current[&local.id].clone().into());
        }

        Ok(code)
    }

    fn subgraphs<'a>(&self, state: &'a Self::State) -> Vec<&'a Graph> {
        vec![&state.body]
    }

    fn subgraphs_mut<'a>(&mut self, state: &'a mut Self::State) -> Vec<&'a mut Graph> {
        vec![&mut state.body]
    }
}
