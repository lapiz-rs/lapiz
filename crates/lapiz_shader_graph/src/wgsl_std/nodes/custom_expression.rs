use std::{cell::RefCell, convert::identity, sync::Arc};

use anyhow::anyhow;
use iced_core::{
    Event, Layout, Length, Rectangle, Renderer as _, Shell, Size, Widget, alignment, layout,
    pointer,
    pointer::mouse,
    renderer,
    shell::Bus,
    text::{self, Renderer as _, Wrapping, paragraph::Plain, parser::PlainText},
    widget::{Operation, Tree, tree},
};
use iced_widget::scrollable::{Direction, Scrollbar};
use indexmap::IndexMap;
use lapiz_i18n::t;
use lapiz_utils::{random_oklch_hue_chroma, wrapper};
use lapiz_widgets::{
    button, column, combo_box, container, fluent_builder::When as _, label, popover, row,
    scrollable, text_editor, text_input, text_input::default,
};
use parse_display::Display;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wesl::syntax::{
    AssignmentOperator, AssignmentStatement, Declaration, DeclarationKind, Ident, Span, Spanned,
    Statement, TypeExpression,
};
use wesl_quote::quote_statement;

use crate::{
    GraphElement, GraphRenderer, GraphTheme,
    editor::NODE_WIDTH,
    graph::{
        GraphResources,
        node::{
            GraphNode, GraphNodeCodeGenContext, GraphNodeCodeGenError, GraphNodeCreateSlotsContext,
            GraphNodeDefaultStateContext, GraphNodeUpdateContext, GraphNodeViewContext,
        },
        slot::{
            ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphDefaultInputSlot,
            GraphDefaultOutputSlot, GraphValueType,
        },
    },
    save::GraphSerializable,
};

wrapper! {
    /// A strongly-typed unique identifier for a user-defined variable on a [`CustomExpressionNode`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
    pub CustomExpressionVariableId : Uuid
}

/// A user-defined input or output variable on a [`CustomExpressionNode`], mapping a display slot name to a WGSL identifier.
#[derive(Clone)]
pub struct CustomExpressionVariable {
    pub id: CustomExpressionVariableId,
    pub display_name: String,
    pub name: String,
    pub ty: Arc<dyn ErasedGraphValueType>,
}

#[derive(Serialize, Deserialize)]
struct SerializableCustomExpressionVariable {
    id: CustomExpressionVariableId,
    display_name: String,
    name: String,
    ty: String,
}

/// Per-instance state for [`CustomExpressionNode`], holding its custom input/output variable lists and inline WGSL source code.
#[derive(Default)]
pub struct CustomExpressionNodeState {
    inputs: IndexMap<CustomExpressionVariableId, CustomExpressionVariable>,
    outputs: IndexMap<CustomExpressionVariableId, CustomExpressionVariable>,
    code: String,
    draft: Option<CustomExpressionDraft>,
}

impl CustomExpressionNodeState {
    pub fn set_code(&mut self, code: impl Into<String>) {
        self.code = code.into();
    }

    pub fn add_input<T: GraphValueType + Default>(
        &mut self,
        name: impl Into<String>,
    ) -> CustomExpressionVariableId {
        Self::add_variable::<T>(&mut self.inputs, name)
    }

    pub fn add_input_non_default<T: GraphValueType>(
        &mut self,
        name: impl Into<String>,
        ty: T,
    ) -> CustomExpressionVariableId {
        let name = name.into();
        let id = CustomExpressionVariableId::new(Uuid::new_v4());
        self.inputs.insert(
            id,
            CustomExpressionVariable {
                id,
                display_name: name.clone(),
                name,
                ty: Arc::new(ty),
            },
        );
        id
    }

    pub fn add_output<T: GraphValueType + Default>(
        &mut self,
        name: impl Into<String>,
    ) -> CustomExpressionVariableId {
        Self::add_variable::<T>(&mut self.outputs, name)
    }

    fn add_variable<T: GraphValueType + Default>(
        variables: &mut IndexMap<CustomExpressionVariableId, CustomExpressionVariable>,
        name: impl Into<String>,
    ) -> CustomExpressionVariableId {
        let name = name.into();
        let id = CustomExpressionVariableId::new(Uuid::new_v4());
        variables.insert(
            id,
            CustomExpressionVariable {
                id,
                display_name: name.clone(),
                name,
                ty: Arc::new(T::default()),
            },
        );
        id
    }
}

#[derive(Clone)]
struct CustomExpressionVariableDraft {
    id: CustomExpressionVariableId,
    display_name: String,
    name: String,
    ty: Option<Arc<dyn ErasedGraphValueType>>,
}

/// Distinguishes whether a [`CustomExpressionVariable`] is an input or output slot of a [`CustomExpressionNode`].
#[derive(Clone, Copy)]
pub enum CustomExpressionVariableKind {
    Input,
    Output,
}

struct CustomExpressionDraft {
    inputs: IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft>,
    outputs: IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft>,
}

impl CustomExpressionDraft {
    fn new(state: &CustomExpressionNodeState) -> Self {
        let to_draft = |variables: &IndexMap<_, CustomExpressionVariable>| {
            variables
                .iter()
                .map(|(id, variable)| {
                    (
                        *id,
                        CustomExpressionVariableDraft {
                            id: *id,
                            display_name: variable.display_name.clone(),
                            name: variable.name.clone(),
                            ty: Some(variable.ty.clone()),
                        },
                    )
                })
                .collect()
        };
        Self {
            inputs: to_draft(&state.inputs),
            outputs: to_draft(&state.outputs),
        }
    }

    fn variables_mut(
        &mut self,
        kind: CustomExpressionVariableKind,
    ) -> &mut IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft> {
        match kind {
            CustomExpressionVariableKind::Input => &mut self.inputs,
            CustomExpressionVariableKind::Output => &mut self.outputs,
        }
    }

    fn finalize(
        variables: &IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft>,
    ) -> IndexMap<CustomExpressionVariableId, CustomExpressionVariable> {
        variables
            .iter()
            .map(|(id, variable)| {
                (
                    *id,
                    CustomExpressionVariable {
                        id: *id,
                        display_name: variable.display_name.clone(),
                        name: variable.name.clone(),
                        ty: variable.ty.clone().unwrap(),
                    },
                )
            })
            .collect()
    }
}

#[derive(Serialize, Deserialize)]
struct SerializableCustomExpressionNodeState {
    inputs: Vec<SerializableCustomExpressionVariable>,
    outputs: Vec<SerializableCustomExpressionVariable>,
    code: String,
}

impl GraphSerializable for CustomExpressionNodeState {
    fn to_toml(&self) -> anyhow::Result<toml::Value> {
        let serialize = |variables: &IndexMap<_, CustomExpressionVariable>| {
            variables
                .values()
                .map(|variable| SerializableCustomExpressionVariable {
                    id: variable.id,
                    display_name: variable.display_name.clone(),
                    name: variable.name.clone(),
                    ty: variable.ty.id().id,
                })
                .collect()
        };
        Ok(toml::Value::try_from(
            SerializableCustomExpressionNodeState {
                inputs: serialize(&self.inputs),
                outputs: serialize(&self.outputs),
                code: self.code.clone(),
            },
        )?)
    }

    fn from_toml(value: toml::Value, resources: &GraphResources) -> anyhow::Result<Self> {
        let serialized = SerializableCustomExpressionNodeState::deserialize(value)?;
        let deserialize = |variables: Vec<SerializableCustomExpressionVariable>| {
            variables
                .into_iter()
                .map(|variable| {
                    let ty = resources
                        .type_registry
                        .resolve_type(&variable.ty)
                        .ok_or_else(|| anyhow!("Unknown type"))?;
                    Ok((
                        variable.id,
                        CustomExpressionVariable {
                            id: variable.id,
                            display_name: variable.display_name,
                            name: variable.name,
                            ty: ty.clone(),
                        },
                    ))
                })
                .collect::<anyhow::Result<IndexMap<_, _>>>()
        };
        Ok(Self {
            inputs: deserialize(serialized.inputs)?,
            outputs: deserialize(serialized.outputs)?,
            code: serialized.code,
            draft: None,
        })
    }
}

const CUSTOM_EXPRESSION_CODE_TEXT_SIZE: f32 = 12.0;
const CUSTOM_EXPRESSION_CODE_PADDING: f32 = 5.0;

struct CustomExpressionCodeEditor<'a> {
    code: &'a str,
}

// TODO Probably avoid this pattern? We are storing the actual state in widget tree,
//      because text_editor::Content is not sync, but GraphNode::State must be sync.
struct CustomExpressionCodeEditorState {
    content: RefCell<text_editor::Content<GraphRenderer>>,
    paragraph: Plain<<GraphRenderer as text::Renderer>::Paragraph>,
    width: f32,
}

fn custom_expression_text_editor(
    content: &text_editor::Content<GraphRenderer>,
    width: f32,
) -> text_editor::TextEditor<'_, PlainText, text_editor::Action, GraphTheme, GraphRenderer> {
    text_editor(content)
        .placeholder("WGSL")
        .size(CUSTOM_EXPRESSION_CODE_TEXT_SIZE)
        .padding(CUSTOM_EXPRESSION_CODE_PADDING)
        .wrapping(Wrapping::None)
        .width(width)
        .height(Length::Shrink)
        .on_action(identity)
}

impl Widget<CustomExpressionNodeMessage, GraphTheme, GraphRenderer>
    for CustomExpressionCodeEditor<'_>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<CustomExpressionCodeEditorState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(CustomExpressionCodeEditorState {
            content: RefCell::new(text_editor::Content::with_text(self.code)),
            paragraph: Plain::default(),
            width: NODE_WIDTH,
        })
    }

    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<CustomExpressionCodeEditorState>();
        if state.content.borrow().text() != self.code {
            *state.content.borrow_mut() = text_editor::Content::with_text(self.code);
        }
        let content = state.content.borrow();
        let mut editor = custom_expression_text_editor(&content, state.width);

        if tree.children.is_empty() {
            tree.children
                .push(Tree::new(&editor as &dyn Widget<_, _, _>));
        } else {
            tree.children[0].diff(&mut editor as &mut dyn Widget<_, _, _>);
        }
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &GraphRenderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<CustomExpressionCodeEditorState>();
        state.paragraph.update(text::Text {
            content: self.code,
            bounds: Size::new(f32::INFINITY, f32::INFINITY),
            size: CUSTOM_EXPRESSION_CODE_TEXT_SIZE.into(),
            line_height: text::LineHeight::default(),
            font: renderer.default_font(),
            align_x: text::Alignment::Left,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: Wrapping::None,
            ellipsis: text::Ellipsis::None,
            hint_factor: renderer.hint_factor(),
        });

        state.width =
            (state.paragraph.min_width().ceil() + CUSTOM_EXPRESSION_CODE_PADDING * 2.0 + 1.0)
                .max(NODE_WIDTH);

        custom_expression_text_editor(&state.content.borrow(), state.width).layout(
            &mut tree.children[0],
            renderer,
            limits,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &GraphRenderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        custom_expression_text_editor(&state.content.borrow(), state.width).operate(
            &mut tree.children[0],
            layout,
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &GraphRenderer,
        shell: &mut Shell<'_, CustomExpressionNodeMessage>,
        viewport: &Rectangle,
    ) {
        if matches!(event, Event::Pointer(pointer::Event::WheelScrolled { .. })) {
            return;
        }

        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        let mut actions = Bus::new();
        let mut child_shell = shell.local(&mut actions);
        custom_expression_text_editor(&state.content.borrow(), state.width).update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            &mut child_shell,
            viewport,
        );
        if !child_shell.is_empty() {
            child_shell.capture_event();
            child_shell.invalidate_layout();
            child_shell.request_redraw();
        }
        shell.merge(child_shell, |action| {
            let mut content = state.content.borrow_mut();
            content.perform(action);
            CustomExpressionNodeMessage::CodeChanged(content.text())
        });
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &GraphRenderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        custom_expression_text_editor(&state.content.borrow(), state.width).mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut GraphRenderer,
        theme: &GraphTheme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<CustomExpressionCodeEditorState>();
        custom_expression_text_editor(&state.content.borrow(), state.width).draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
}

/// A shader graph node that executes user-authored inline WGSL statements with customizable input and output slots.
///
/// `CustomExpressionNode` wraps its body in a scoped WGSL block (`{ ... }`), binding each configured
/// input slot to its chosen WGSL identifier and copying each output identifier back to the node's
/// generated output variables at the end of the block.
#[derive(Default, Clone)]
pub struct CustomExpressionNode;

/// UI message emitted by [`CustomExpressionNode`] for editing its variable schema and WGSL source code.
#[derive(Clone)]
pub enum CustomExpressionNodeMessage {
    ToggleEditor,
    AddVariable(CustomExpressionVariableKind),
    RemoveVariable(CustomExpressionVariableKind, CustomExpressionVariableId),
    MoveVariableUp(CustomExpressionVariableKind, CustomExpressionVariableId),
    MoveVariableDown(CustomExpressionVariableKind, CustomExpressionVariableId),
    ChangeDisplayName(
        CustomExpressionVariableKind,
        CustomExpressionVariableId,
        String,
    ),
    ChangeName(
        CustomExpressionVariableKind,
        CustomExpressionVariableId,
        String,
    ),
    ChangeType(
        CustomExpressionVariableKind,
        CustomExpressionVariableId,
        String,
    ),
    CodeChanged(String),
    Confirm,
    Cancel,
    LiteralUpdate(ErasedGraphLiteralUpdateMessage),
}

fn custom_expression_variable_rows(
    variables: &IndexMap<CustomExpressionVariableId, CustomExpressionVariableDraft>,
    kind: CustomExpressionVariableKind,
    resources: &GraphResources,
) -> Vec<GraphElement<'static, CustomExpressionNodeMessage>> {
    let type_names = resources
        .type_registry
        .all_types()
        .keys()
        .map(|id| id.id.clone())
        .collect::<Vec<_>>();
    variables
        .values()
        .map(|variable| {
            let id = variable.id;
            column![
                row![
                    text_input("Slot Name", &variable.display_name)
                        .size(12.0)
                        .style(default)
                        .on_input(move |name| {
                            CustomExpressionNodeMessage::ChangeDisplayName(kind, id, name)
                        }),
                    text_input("WGSL Name", &variable.name)
                        .size(12.0)
                        .style(default)
                        .on_input(move |name| {
                            CustomExpressionNodeMessage::ChangeName(kind, id, name)
                        }),
                ]
                .gap(4.0),
                row![
                    combo_box(
                        type_names.clone(),
                        variable.ty.as_ref().map(|ty| ty.id().id),
                        move |ty| {
                            CustomExpressionNodeMessage::ChangeType(kind, id, ty.to_string())
                        }
                    )
                    .width(Length::Fill),
                    button(label("Up"))
                        .on_press(CustomExpressionNodeMessage::MoveVariableUp(kind, id)),
                    button(label("Down"))
                        .on_press(CustomExpressionNodeMessage::MoveVariableDown(kind, id)),
                    button(label("Delete"))
                        .on_press(CustomExpressionNodeMessage::RemoveVariable(kind, id)),
                ]
                .gap(4.0),
            ]
            .gap(4.0)
            .into()
        })
        .collect()
}

fn is_custom_expression_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn custom_expression_editor_view(
    state: &CustomExpressionNodeState,
    resources: &GraphResources,
) -> GraphElement<'static, CustomExpressionNodeMessage> {
    let draft = state.draft.as_ref().expect("editor must be open");
    let input_rows = custom_expression_variable_rows(
        &draft.inputs,
        CustomExpressionVariableKind::Input,
        resources,
    );
    let output_rows = custom_expression_variable_rows(
        &draft.outputs,
        CustomExpressionVariableKind::Output,
        resources,
    );
    let variables = draft
        .inputs
        .values()
        .chain(draft.outputs.values())
        .collect::<Vec<_>>();
    let valid = variables.iter().all(|variable| {
        !variable.display_name.is_empty()
            && variable.display_name.trim() == variable.display_name
            && is_custom_expression_identifier(&variable.name)
            && variable.ty.is_some()
            && variables
                .iter()
                .filter(|other| other.name == variable.name)
                .count()
                == 1
    });
    let panel = column![]
        .width(Length::Fixed(500.0))
        .padding(4)
        .gap(6.0)
        .push(label(t!("inputs")).size(12))
        .extend(input_rows)
        .push(
            button(label("Add Input")).on_press(CustomExpressionNodeMessage::AddVariable(
                CustomExpressionVariableKind::Input,
            )),
        )
        .push(label(t!("outputs")).size(12))
        .extend(output_rows)
        .push(
            button(label("Add Output")).on_press(CustomExpressionNodeMessage::AddVariable(
                CustomExpressionVariableKind::Output,
            )),
        )
        .push(
            row![
                button(label("Cancel")).on_press(CustomExpressionNodeMessage::Cancel),
                button(label("Confirm")).when(valid, |button| {
                    button.on_press(CustomExpressionNodeMessage::Confirm)
                }),
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

impl GraphNode for CustomExpressionNode {
    type State = CustomExpressionNodeState;

    type Message = CustomExpressionNodeMessage;

    fn id(&self) -> &'static str {
        "custom_expression_node"
    }

    fn default_state(&self, _: GraphNodeDefaultStateContext<'_>) -> Self::State {
        CustomExpressionNodeState {
            inputs: IndexMap::new(),
            outputs: IndexMap::new(),
            code: String::new(),
            draft: None,
        }
    }

    fn header_hue_chroma(&self) -> (f32, f32) {
        random_oklch_hue_chroma!(CustomExpressionNode)
    }

    fn create_inputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultInputSlot> {
        state
            .inputs
            .values()
            .map(|variable| {
                GraphDefaultInputSlot::new_boxed(variable.display_name.clone(), variable.ty.clone())
            })
            .collect()
    }

    fn create_outputs(
        &self,
        state: &Self::State,
        _: GraphNodeCreateSlotsContext<'_>,
    ) -> Vec<GraphDefaultOutputSlot> {
        state
            .outputs
            .values()
            .map(|variable| {
                GraphDefaultOutputSlot::new_boxed(
                    variable.display_name.clone(),
                    variable.ty.clone(),
                )
            })
            .collect()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        ctx: GraphNodeViewContext<'_>,
    ) -> GraphElement<'a, Self::Message> {
        let trigger = button(label("Edit")).on_press(CustomExpressionNodeMessage::ToggleEditor);
        let content = state
            .draft
            .as_ref()
            .map(|_| custom_expression_editor_view(state, ctx.resources));
        ctx.view_all_slots_with_header(
            column![
                popover(trigger).content(content),
                scrollable(GraphElement::new(CustomExpressionCodeEditor {
                    code: &state.code
                }))
                .direction(Direction::Both {
                    vertical: Scrollbar::new(),
                    horizontal: Scrollbar::new(),
                })
                .width(Length::Fill)
                .height(140.0)
            ]
            .gap(4.0),
            CustomExpressionNodeMessage::LiteralUpdate,
        )
    }

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
        mut ctx: GraphNodeUpdateContext<'_>,
    ) {
        match message {
            CustomExpressionNodeMessage::ToggleEditor => {
                state.draft = if state.draft.is_some() {
                    None
                } else {
                    Some(CustomExpressionDraft::new(state))
                };
            }
            CustomExpressionNodeMessage::AddVariable(kind) => {
                if let Some(draft) = &mut state.draft {
                    let id = CustomExpressionVariableId::new(Uuid::new_v4());
                    draft.variables_mut(kind).insert(
                        id,
                        CustomExpressionVariableDraft {
                            id,
                            display_name: String::new(),
                            name: String::new(),
                            ty: None,
                        },
                    );
                }
            }
            CustomExpressionNodeMessage::RemoveVariable(kind, id) => {
                if let Some(draft) = &mut state.draft {
                    draft.variables_mut(kind).shift_remove(&id);
                }
            }
            CustomExpressionNodeMessage::MoveVariableUp(kind, id) => {
                if let Some(draft) = &mut state.draft {
                    let variables = draft.variables_mut(kind);
                    if let Some(index) = variables.get_index_of(&id)
                        && index > 0
                    {
                        variables.swap_indices(index, index - 1);
                    }
                }
            }
            CustomExpressionNodeMessage::MoveVariableDown(kind, id) => {
                if let Some(draft) = &mut state.draft {
                    let variables = draft.variables_mut(kind);
                    if let Some(index) = variables.get_index_of(&id)
                        && index + 1 < variables.len()
                    {
                        variables.swap_indices(index, index + 1);
                    }
                }
            }
            CustomExpressionNodeMessage::ChangeDisplayName(kind, id, name) => {
                if let Some(variable) = state
                    .draft
                    .as_mut()
                    .and_then(|draft| draft.variables_mut(kind).get_mut(&id))
                {
                    variable.display_name = name;
                }
            }
            CustomExpressionNodeMessage::ChangeName(kind, id, name) => {
                if let Some(variable) = state
                    .draft
                    .as_mut()
                    .and_then(|draft| draft.variables_mut(kind).get_mut(&id))
                {
                    variable.name = name;
                }
            }
            CustomExpressionNodeMessage::ChangeType(kind, id, ty) => {
                if let Some(variable) = state
                    .draft
                    .as_mut()
                    .and_then(|draft| draft.variables_mut(kind).get_mut(&id))
                {
                    variable.ty = ctx.resources.type_registry.resolve_type(&ty);
                }
            }
            CustomExpressionNodeMessage::CodeChanged(code) => state.code = code,
            CustomExpressionNodeMessage::Confirm => {
                let Some(draft) = state.draft.take() else {
                    return;
                };
                state.inputs = CustomExpressionDraft::finalize(&draft.inputs);
                state.outputs = CustomExpressionDraft::finalize(&draft.outputs);
            }
            CustomExpressionNodeMessage::Cancel => state.draft = None,
            CustomExpressionNodeMessage::LiteralUpdate(message) => ctx.update_literal(message),
        }
    }

    fn generate_code(
        &self,
        state: &Self::State,
        mut ctx: GraphNodeCodeGenContext<'_>,
    ) -> Result<String, GraphNodeCodeGenError> {
        if state.inputs.len() != ctx.inputs.len() || state.outputs.len() != ctx.outputs.len() {
            return Err(anyhow!("Invalid slots").into());
        }

        let names = state
            .inputs
            .values()
            .chain(state.outputs.values())
            .map(|variable| variable.name.clone())
            .collect::<Vec<_>>();

        let mut outputs = Vec::with_capacity(state.outputs.len());
        let mut code = String::new();
        for (index, variable) in state.outputs.values().enumerate() {
            let mut output = ctx.get_output(index)?;
            while names.contains(&output.to_string()) {
                output = Ident::new(ctx.ident_generator.next_output());
                ctx.output_slot_idents
                    .insert(ctx.outputs[index], output.clone().into());
            }
            let ty = variable
                .ty
                .wgsl_type_name()
                .ok_or_else(|| GraphNodeCodeGenError::Custom(anyhow!("Invalid type")))?;
            code.push_str(&quote_statement! { var #output: #ty; }.to_string());
            code.push('\n');
            outputs.push(output);
        }

        code.push_str("{\n");

        for (index, variable) in state.inputs.values().enumerate() {
            let name = variable.name.clone();
            let input = ctx.get_input(index)?;
            code.push_str(&quote_statement! { let #name = #input; }.to_string());
            code.push('\n');
        }
        for variable in state.outputs.values() {
            let name = variable.name.clone();
            let ty = variable
                .ty
                .wgsl_type_name()
                .ok_or_else(|| GraphNodeCodeGenError::Custom(anyhow!("Invalid type")))?;
            code.push_str(&quote_statement! { var #name: #ty; }.to_string());
            code.push('\n');
        }
        code.push_str(&state.code);
        if !state.code.is_empty() && !state.code.ends_with('\n') {
            code.push('\n');
        }
        for ((_, variable), output) in state.outputs.iter().zip(outputs) {
            let name = Ident::new(variable.name.clone());
            code.push_str(&quote_statement! { #output = #name; }.to_string());
            code.push('\n');
        }

        code.push_str("}\n");
        Ok(code)
    }
}
