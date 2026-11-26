use std::{collections::HashMap, sync::Arc};

use anyhow::{Context as _, Result, bail};
use indexmap::IndexMap;

use crate::{
    asset::{
        EffectAsset, EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy,
        EffectPassId, EffectPassInputSlotId, EffectPassOutputSlotId, SerializableEffectInputSlot,
        SerializableEffectOutputSlot, SerializableEffectPass,
    },
    render::{EffectPassInputSlotSource, EffectPassOutputSlotTarget},
    shader_graph::{
        builtin::nodes::pass_io::DispatchIndexNode,
        graph::{
            Graph, GraphResources,
            node::{GraphNodeOutputTarget, PassInput, PassInputChoice, PassOutputChoice},
            slot::ErasedGraphValueType,
            variable::GraphShaderLiteral,
        },
    },
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
        let mut target_choices = vec![];
        for id in self.outputs.keys() {
            let slot = self
                .outputs
                .get(id)
                .with_context(|| format!("Unknown effect output {id:?}"))?;
            let label = format!("{} ({})", slot.name, slot.ty.id().id);
            target_choices.push(PassOutputChoice {
                label,
                ty: slot.ty.clone(),
                target: *id,
            });
        }
        let available_targets = Arc::<[PassOutputChoice]>::from(target_choices);

        let mut passes_to_reconcile = HashMap::with_capacity(self.passes.len());
        for (pass_id, pass) in &mut self.passes {
            let mut needs_reconcile = false;
            for node in pass.graph.iter_nodes_mut() {
                if let Some(state) = node.data.state_mut::<DispatchIndexNode>()
                    && state.cached_dispatch_strategy != pass.dispatch_strategy
                {
                    state.cached_dispatch_strategy = pass.dispatch_strategy;
                    needs_reconcile = true;
                }

                needs_reconcile |= node
                    .data
                    .on_available_pass_output_choices_changed(available_targets.clone());
            }
            passes_to_reconcile.insert(*pass_id, needs_reconcile);
        }

        let pass_outputs = self.collect_pass_outputs()?;

        let mut available_sources = HashMap::with_capacity(self.passes.len());
        for pass_id in self.passes.keys() {
            let mut choices = vec![];
            for id in self.inputs.keys() {
                let slot = self
                    .inputs
                    .get(id)
                    .with_context(|| format!("Unknown effect input {id:?}"))?;
                let label = format!("{} ({})", slot.name, slot.ty.id().id);
                let ty = slot.ty.clone();

                choices.push(PassInputChoice {
                    label,
                    ty,
                    input: PassInput::Effect(*id),
                });
            }
            for (output_id, info) in &pass_outputs {
                if &info.producer == pass_id {
                    continue;
                }

                let info = pass_outputs
                    .get(output_id)
                    .with_context(|| format!("Unknown pass output {output_id:?}"))?;
                let label = format!(
                    "{} / {} ({})",
                    self.passes[&info.producer].name,
                    info.name,
                    info.ty.id().id
                );
                let ty = info.ty.clone();
                choices.push(PassInputChoice {
                    label,
                    ty,
                    input: PassInput::Pass(*output_id),
                });
            }
            available_sources.insert(*pass_id, Arc::<[_]>::from(choices));
        }

        for (pass_id, pass) in &mut self.passes {
            let sources = available_sources[pass_id].clone();
            let mut needs_reconcile = passes_to_reconcile[pass_id];
            for node in pass.graph.iter_nodes_mut() {
                needs_reconcile |= node.data.on_available_pass_input_choices_changed(sources.clone());
            }
            if needs_reconcile {
                pass.graph.reconcile_all_node_slots();
            }
        }
        Ok(())
    }

    fn collect_pass_outputs(&self) -> Result<HashMap<EffectPassOutputSlotId, PassOutputInfo>> {
        let mut outputs = HashMap::new();
        for (pass_id, pass) in &self.passes {
            for node in pass.graph.iter_nodes() {
                let Some(output) = node.data.as_pass_output() else {
                    continue;
                };
                let (ty, name) = match output.target {
                    GraphNodeOutputTarget::Pass(def) => (def.ty.clone(), def.name.clone()),
                    GraphNodeOutputTarget::Effect(id) => {
                        let slot = self
                            .outputs
                            .get(&id)
                            .with_context(|| format!("Unknown effect output {id:?}"))?;
                        (slot.ty.clone(), slot.name.clone())
                    }
                };
                let info = PassOutputInfo {
                    producer: *pass_id,
                    ty,
                    name,
                };
                if outputs.insert(output.id, info).is_some() {
                    bail!("Duplicate pass output ID {:?}", output.id);
                }
            }
        }
        Ok(outputs)
    }

    pub fn pass_io_labels(&self, pass_id: &EffectPassId) -> EffectPassIoLabels {
        let pass = self.passes.get(pass_id).expect("pass exists");
        let mut io = EffectPassIoLabels::default();
        for node in pass.graph.iter_nodes() {
            if let Some(input_port) = node.data.as_pass_input() {
                io.inputs.push(input_port.label.clone());
            }
            if let Some(output_port) = node.data.as_pass_output() {
                io.outputs.push(output_port.label.clone());
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
