use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result, bail};
use indexmap::IndexMap;
use lapiz_i18n::t;
use lapiz_shader_graph::graph::{
    Graph, GraphResources, slot::ErasedGraphValueType, variable::GraphShaderLiteral,
};

use crate::{
    asset::{
        EffectAsset, EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy,
        EffectPassId, EffectPassInputSlotId, EffectPassOutputSlotId, SerializableEffectInputSlot,
        SerializableEffectOutputSlot, SerializableEffectPass,
    },
    nodes::{
        DispatchIndexNode, PassInput, PassInputChoice, PassInputNode, PassOutput, PassOutputChoice,
        PassOutputChoiceTarget, PassOutputNode, TypeChoice,
    },
    render::{EffectPassInputSlotSource, EffectPassOutputSlotTarget},
};

pub type EffectInputsDecl = HashMap<EffectInputSlotId, Arc<dyn ErasedGraphValueType>>;
pub type EffectOutputsDecl = HashMap<EffectOutputSlotId, Arc<dyn ErasedGraphValueType>>;
pub type EffectPassInputsDecl = HashMap<EffectPassInputSlotId, EffectPassInputSlotSource>;
pub type EffectPassOutputsDecl = HashMap<EffectPassOutputSlotId, EffectPassOutputSlotTarget>;

pub type EffectInputs = HashMap<EffectInputSlotId, GraphShaderLiteral>;
pub type EffectOutputs = HashMap<EffectOutputSlotId, GraphShaderLiteral>;
pub type EffectPassOutputs = HashMap<EffectPassOutputSlotId, GraphShaderLiteral>;
pub type EffectPassOutputTypes = HashMap<EffectPassOutputSlotId, Arc<dyn ErasedGraphValueType>>;

/// The live, editable runtime representation of a multi-pass shader effect instantiated from an [`EffectAsset`].
///
/// `EffectInstance` owns the resolved [`EffectInputSlot`]s, [`EffectOutputSlot`]s, and ordered
/// [`EffectPass`]es (each containing a live [`Graph`]). Whenever passes or I/O slots are modified,
/// [`sync_pass_graph_effect_properties`](Self::sync_pass_graph_effect_properties) propagates the
/// available input sources, output targets, value types, and dispatch strategies into each pass's
/// [`PassInputNode`], [`PassOutputNode`], and [`DispatchIndexNode`] states before reconciling node
/// slots.
pub struct EffectInstance {
    pub name: String,
    pub passes: IndexMap<EffectPassId, EffectPass>,
    pub inputs: IndexMap<EffectInputSlotId, EffectInputSlot>,
    pub outputs: IndexMap<EffectOutputSlotId, EffectOutputSlot>,
}

impl EffectInstance {
    pub fn from_asset(asset: &EffectAsset, resources: GraphResources) -> Result<Self> {
        let inputs = asset
            .inputs
            .iter()
            .map(|slot| {
                Ok((
                    slot.id,
                    EffectInputSlot {
                        name: slot.name.clone(),
                        id: slot.id,
                        ty: resources
                            .type_registry
                            .resolve_type(&slot.ty)
                            .with_context(|| format!("Unknown effect input type '{}'", slot.ty))?
                            .clone(),
                    },
                ))
            })
            .collect::<Result<IndexMap<_, _>>>()?;
        let outputs = asset
            .outputs
            .iter()
            .map(|slot| {
                Ok((
                    slot.id,
                    EffectOutputSlot {
                        name: slot.name.clone(),
                        id: slot.id,
                        ty: resources
                            .type_registry
                            .resolve_type(&slot.ty)
                            .with_context(|| format!("Unknown effect output type '{}'", slot.ty))?
                            .clone(),
                    },
                ))
            })
            .collect::<Result<IndexMap<_, _>>>()?;

        let mut passes = IndexMap::with_capacity(asset.passes.len());
        for pass in &asset.passes {
            let (graph, errors) = Graph::from_serialized(
                &pass.graph,
                GraphResources {
                    type_registry: resources.type_registry.clone(),
                    node_registry: resources.node_registry.clone(),
                    assets: resources.assets.clone(),
                },
            );
            if let Some(error) = errors.first() {
                bail!("Pass '{}' graph failed to deserialize: {error}", pass.name);
            }
            let graph = graph.context("Pass graph failed to deserialize")?;
            passes.insert(
                pass.id,
                EffectPass {
                    name: pass.name.clone(),
                    graph,
                    dispatch_strategy: pass.dispatch_strategy,
                },
            );
        }

        let mut instance = Self {
            name: asset.name.clone(),
            passes,
            inputs,
            outputs,
        };
        instance.sync_pass_graph_effect_properties()?;
        Ok(instance)
    }

    pub fn as_asset(&self) -> Result<EffectAsset> {
        Ok(EffectAsset {
            name: self.name.clone(),
            inputs: self
                .inputs
                .values()
                .map(|slot| SerializableEffectInputSlot {
                    name: slot.name.clone(),
                    id: slot.id,
                    ty: slot.ty.id().id,
                })
                .collect(),
            outputs: self
                .outputs
                .values()
                .map(|slot| SerializableEffectOutputSlot {
                    name: slot.name.clone(),
                    id: slot.id,
                    ty: slot.ty.id().id,
                })
                .collect(),
            passes: self
                .passes
                .iter()
                .map(|(id, pass)| {
                    Ok(SerializableEffectPass {
                        id: *id,
                        name: pass.name.clone(),
                        graph: pass.graph.as_serialized()?,
                        dispatch_strategy: pass.dispatch_strategy,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub fn single_pass_id(&self) -> Option<EffectPassId> {
        if self.passes.len() == 1 {
            Some(*self.passes.keys().next().unwrap())
        } else {
            None
        }
    }

    pub fn single_pass(&self) -> Option<&EffectPass> {
        if self.passes.len() == 1 {
            Some(self.passes.values().next().unwrap())
        } else {
            None
        }
    }

    pub fn single_pass_mut(&mut self) -> Option<&mut EffectPass> {
        if self.passes.len() == 1 {
            Some(self.passes.values_mut().next().unwrap())
        } else {
            None
        }
    }

    pub fn sync_pass_graph_effect_properties(&mut self) -> Result<()> {
        let effect_input_types = self
            .inputs
            .iter()
            .map(|(id, input)| (*id, input.ty.clone()))
            .collect::<HashMap<_, _>>();
        let effect_output_types = self
            .outputs
            .iter()
            .map(|(id, output)| (*id, output.ty.clone()))
            .collect::<HashMap<_, _>>();
        let pass_outputs = self.collect_pass_outputs()?;

        let available_types = self
            .passes
            .values()
            .next()
            .map(|pass| {
                pass.graph
                    .resources()
                    .type_registry
                    .all_types()
                    .values()
                    .map(|ty| TypeChoice {
                        label: ty.id().id,
                        ty: ty.clone(),
                    })
                    .collect::<Arc<[TypeChoice]>>()
            })
            .unwrap_or_default();

        let mut available_sources = HashMap::with_capacity(self.passes.len());
        for pass_id in self.passes.keys() {
            let mut choices = vec![PassInputChoice {
                label: t!("unbound"),
                input: None,
            }];
            for id in self.inputs.keys() {
                choices.push(PassInputChoice {
                    label: self.input_label(PassInput::Effect(*id), &pass_outputs)?,
                    input: Some(PassInput::Effect(*id)),
                });
            }
            for (output_id, info) in &pass_outputs {
                if &info.producer == pass_id {
                    continue;
                }
                choices.push(PassInputChoice {
                    label: self.input_label(PassInput::Pass(*output_id), &pass_outputs)?,
                    input: Some(PassInput::Pass(*output_id)),
                });
            }
            available_sources.insert(*pass_id, Arc::<[_]>::from(choices));
        }

        let mut target_choices = vec![PassOutputChoice {
            label: t!("unbound"),
            target: PassOutputChoiceTarget::Unbound,
        }];
        for id in self.outputs.keys() {
            target_choices.push(PassOutputChoice {
                label: self.output_label(&PassOutput::Effect(*id))?,
                target: PassOutputChoiceTarget::Effect(*id),
            });
        }
        target_choices.push(PassOutputChoice {
            label: t!("local_buffer"),
            target: PassOutputChoiceTarget::LocalBuffer,
        });
        let available_targets = Arc::<[PassOutputChoice]>::from(target_choices);

        for (pass_id, pass) in &mut self.passes {
            let sources = available_sources[pass_id].clone();
            for node in pass.graph.iter_nodes_mut() {
                if let Some(state) = node.data.state_mut::<DispatchIndexNode>() {
                    state.cached_dispatch_strategy = pass.dispatch_strategy;
                } else if let Some(state) = node.data.state_mut::<PassInputNode>() {
                    state.available_sources = sources.clone();
                    state.cached_ty = match state.input {
                        Some(PassInput::Effect(id)) => Some(
                            effect_input_types
                                .get(&id)
                                .with_context(|| format!("Unknown effect input {id:?}"))?
                                .clone(),
                        ),
                        Some(PassInput::Pass(id)) => Some(
                            pass_outputs
                                .get(&id)
                                .with_context(|| format!("Unknown pass output {id:?}"))?
                                .ty
                                .clone(),
                        ),
                        None => None,
                    };
                } else if let Some(state) = node.data.state_mut::<PassOutputNode>() {
                    state.available_targets = available_targets.clone();
                    state.available_types = available_types.clone();
                    state.cached_ty = match &state.output {
                        Some(PassOutput::Pass(def)) => Some(def.ty.clone()),
                        Some(PassOutput::Effect(id)) => Some(
                            effect_output_types
                                .get(id)
                                .with_context(|| format!("Unknown effect output {id:?}"))?
                                .clone(),
                        ),
                        None => None,
                    };
                }
            }
            pass.graph.reconcile_all_node_slots();
        }
        Ok(())
    }

    fn collect_pass_outputs(&self) -> Result<HashMap<EffectPassOutputSlotId, PassOutputInfo>> {
        let mut outputs = HashMap::new();
        for (pass_id, pass) in &self.passes {
            for node in pass.graph.iter_nodes() {
                let Some(state) = node.data.state::<PassOutputNode>() else {
                    continue;
                };
                let Some(output) = &state.output else {
                    continue;
                };
                let (ty, name) = match output {
                    PassOutput::Pass(def) => (def.ty.clone(), def.name.clone()),
                    PassOutput::Effect(id) => {
                        let slot = self
                            .outputs
                            .get(id)
                            .with_context(|| format!("Unknown effect output {id:?}"))?;
                        (slot.ty.clone(), slot.name.clone())
                    }
                };
                let info = PassOutputInfo {
                    producer: *pass_id,
                    ty,
                    name,
                };
                if outputs.insert(state.id, info).is_some() {
                    bail!("Duplicate pass output ID {:?}", state.id);
                }
            }
        }
        Ok(outputs)
    }

    fn input_label(
        &self,
        input: PassInput,
        pass_outputs: &HashMap<EffectPassOutputSlotId, PassOutputInfo>,
    ) -> Result<String> {
        Ok(match input {
            PassInput::Effect(id) => {
                let slot = self
                    .inputs
                    .get(&id)
                    .with_context(|| format!("Unknown effect input {id:?}"))?;
                format!("{} ({})", slot.name, slot.ty.id().id)
            }
            PassInput::Pass(id) => {
                let info = pass_outputs
                    .get(&id)
                    .with_context(|| format!("Unknown pass output {id:?}"))?;
                format!(
                    "{} / {} ({})",
                    self.passes[&info.producer].name,
                    info.name,
                    info.ty.id().id
                )
            }
        })
    }

    fn output_label(&self, output: &PassOutput) -> Result<String> {
        Ok(match output {
            PassOutput::Pass(def) => format!("{} ({})", def.name, def.ty.id().id),
            PassOutput::Effect(id) => {
                let slot = self
                    .outputs
                    .get(id)
                    .with_context(|| format!("Unknown effect output {id:?}"))?;
                format!("{} ({})", slot.name, slot.ty.id().id)
            }
        })
    }

    pub fn pass_io_labels(&self, pass_id: &EffectPassId) -> EffectPassIoLabels {
        let pass = self.passes.get(pass_id).expect("pass exists");
        let mut io = EffectPassIoLabels::default();
        for node in pass.graph.iter_nodes() {
            if let Some(state) = node.data.state::<PassInputNode>() {
                let label = match state.input {
                    None => t!("unbound"),
                    Some(input) => state
                        .available_sources
                        .iter()
                        .find(|choice| choice.input == Some(input))
                        .map(|choice| choice.label.clone())
                        .expect("synced options contain the current binding"),
                };
                io.inputs.push(label);
            } else if let Some(state) = node.data.state::<PassOutputNode>() {
                let label = match &state.output {
                    None => t!("unbound"),
                    Some(PassOutput::Effect(id)) => state
                        .available_targets
                        .iter()
                        .find(|choice| choice.target == PassOutputChoiceTarget::Effect(*id))
                        .map(|choice| choice.label.clone())
                        .expect("synced options contain the current binding"),
                    Some(PassOutput::Pass(def)) => format!("{} ({})", def.name, def.ty.id().id),
                };
                io.outputs.push(label);
            }
        }
        io
    }
}

/// Human-readable input and output binding labels summarized for an [`EffectPass`] card in the editor.
#[derive(Default)]
pub struct EffectPassIoLabels {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

struct PassOutputInfo {
    producer: EffectPassId,
    ty: Arc<dyn ErasedGraphValueType>,
    name: String,
}

/// A single compute pass within an [`EffectInstance`], pairing a shader [`Graph`] with an [`EffectPassDispatchStrategy`].
pub struct EffectPass {
    pub name: String,
    pub graph: Graph,
    pub dispatch_strategy: EffectPassDispatchStrategy,
}

/// A resolved external input slot on an [`EffectInstance`].
#[derive(Clone)]
pub struct EffectInputSlot {
    pub name: String,
    pub id: EffectInputSlotId,
    pub ty: Arc<dyn ErasedGraphValueType>,
}

/// A resolved external output slot on an [`EffectInstance`].
#[derive(Clone)]
pub struct EffectOutputSlot {
    pub name: String,
    pub id: EffectOutputSlotId,
    pub ty: Arc<dyn ErasedGraphValueType>,
}
