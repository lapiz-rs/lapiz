use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail, ensure};
use indexmap::IndexMap;
use lapiz_image::{image, tile::DynamicLayerStorage};
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    bind_group_layout_entries::DynamicBindGroupLayoutEntries, render, wesl_jit,
};
use lapiz_shader_graph::{
    graph::{
        Graph, GraphVarIdentGenerator,
        slot::{ErasedGraphValueType, GraphShaderStage},
        variable::GraphShaderLiteral,
    },
    wgsl_std::types::{
        atomic::{ArrayAtomicI32Type, ArrayAtomicU32Type, PreparedAtomicArray},
        handle::{ArrayType, LayerType, PreparedArray, PreparedLayer, PreparedLayerPixels},
        layer_tile_info_ident,
    },
};
use wesl::{
    CodegenPkg,
    syntax::{
        BinaryExpression, BinaryOperator, CompoundStatement, Declaration, DeclarationKind,
        Expression, FunctionCall, Ident, IfClause, IfStatement, IndexingExpression,
        LiteralExpression, NamedComponentExpression, ReturnStatement, Span, Spanned, Statement,
        TypeExpression, UnaryExpression, UnaryOperator,
    },
};
use wesl_quote::quote_statement;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, CommandEncoder,
    ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor, Device,
    PipelineLayoutDescriptor, Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages,
};

use crate::{
    asset::{
        EffectInputSlotId, EffectOutputSlotId, EffectPassDispatchStrategy, EffectPassInputSlotId,
        EffectPassOutputSlotId,
    },
    instance::{
        EffectInputSlot, EffectInputs, EffectInputsDecl, EffectInstance, EffectOutputs,
        EffectPassInputsDecl, EffectPassOutputTypes, EffectPassOutputs, EffectPassOutputsDecl,
    },
    nodes::{PassInput, PassInputNode, PassOutput, PassOutputNode},
};

pub fn pass_input_ident(slot: EffectPassInputSlotId) -> String {
    format!("pass_input_{}", slot.to_string().replace('-', "_"))
}

pub fn pass_output_ident(slot: EffectPassOutputSlotId) -> String {
    format!("pass_output_{}", slot.to_string().replace('-', "_"))
}

pub enum EffectPassInputSlotSource {
    PassOutput(EffectPassOutputSlotId),
    EffectInput(EffectInputSlotId),
}

pub struct EffectPassOutputSlotTarget {
    pub ty: Arc<dyn ErasedGraphValueType>,
}

pub struct EffectRenderer {
    inputs: Vec<EffectInputSlot>,
    outputs: HashMap<EffectOutputSlotId, EffectPassOutputSlotId>,
    builtin_literals: HashMap<String, Arc<dyn ErasedGraphValueType>>,

    passes: Vec<EffectRenderPass>,
    device: Device,
    queue: Queue,
}

impl EffectRenderer {
    pub fn from_instance(
        instance: &EffectInstance,
        builtin_literals: HashMap<String, Arc<dyn ErasedGraphValueType>>,
        shader_deps: &[&CodegenPkg],
        device: Device,
        queue: Queue,
    ) -> Result<Self> {
        let inputs = instance
            .inputs
            .iter()
            .map(|(id, slot)| (*id, slot))
            .collect::<HashMap<_, _>>();
        let outputs = instance
            .outputs
            .iter()
            .map(|(id, slot)| (*id, slot))
            .collect::<HashMap<_, _>>();
        ensure!(
            inputs.len() == instance.inputs.len(),
            "Duplicate effect input"
        );
        ensure!(
            outputs.len() == instance.outputs.len(),
            "Duplicate effect output"
        );

        let mut exports = HashMap::new();
        let mut pass_inputs = HashMap::new();
        let mut pass_outputs = HashMap::new();
        let mut pass_output_ids = HashMap::new();
        let mut pass_output_types = HashMap::new();

        for (id, pass) in &instance.passes {
            let mut pass_input_decl = IndexMap::new();
            let mut pass_output_decl = IndexMap::new();

            for node in pass.graph.iter_nodes() {
                if let Some(state) = node.data.state::<PassInputNode>() {
                    let Some(input) = &state.input else {
                        bail!("Pass input node has no input: {:?}", state.id);
                    };

                    pass_input_decl.insert(
                        state.id,
                        match input {
                            PassInput::Pass(id) => EffectPassInputSlotSource::PassOutput(*id),
                            PassInput::Effect(id) => {
                                ensure!(inputs.contains_key(id), "Unknown effect input {id:?}");
                                EffectPassInputSlotSource::EffectInput(*id)
                            }
                        },
                    );
                } else if let Some(state) = node.data.state::<PassOutputNode>() {
                    let Some(output) = &state.output else {
                        bail!("Pass output node has no output: {:?}", state.id);
                    };
                    let ty = match output {
                        PassOutput::Pass(def) => def.ty.clone(),
                        PassOutput::Effect(external) => {
                            let output = outputs.get(external).context("Unknown effect output")?;
                            ensure!(
                                exports.insert(*external, state.id).is_none(),
                                "Multiple nodes target effect output {external:?}"
                            );
                            output.ty.clone()
                        }
                    };
                    ensure!(
                        pass_output_types.insert(state.id, ty.clone()).is_none(),
                        "Duplicate pass output ID {:?}",
                        state.id
                    );
                    pass_output_decl.insert(state.id, EffectPassOutputSlotTarget { ty });
                    ensure!(
                        pass_output_ids.insert(state.id, *id).is_none(),
                        "Duplicate pass output ID {:?}",
                        state.id
                    );
                }
            }

            pass_input_decl.sort_by(|a, _, b, _| a.0.cmp(&b.0));
            pass_output_decl.sort_by(|a, _, b, _| a.0.cmp(&b.0));

            pass_inputs.insert(*id, pass_input_decl);
            pass_outputs.insert(*id, pass_output_decl);
        }

        for output in outputs.keys() {
            ensure!(
                exports.contains_key(output),
                "Effect output {output:?} has no producer"
            );
        }

        // Pass dependencies come only from inputs wired to other passes' outputs.
        let mut dependencies = HashMap::new();
        for (id, decl) in &pass_inputs {
            let mut sources = HashSet::new();
            for source in decl.values() {
                if let EffectPassInputSlotSource::PassOutput(slot) = source {
                    let producer = *pass_output_ids
                        .get(slot)
                        .with_context(|| format!("Unknown pass output {slot:?}"))?;
                    ensure!(producer != *id, "Pass references its own output");
                    sources.insert(producer);
                }
            }
            dependencies.insert(*id, sources);
        }

        let mut order = Vec::new();
        let mut done = HashSet::new();
        while order.len() < instance.passes.len() {
            let Some(id) = instance
                .passes
                .keys()
                .find(|id| !done.contains(*id) && dependencies[id].is_subset(&done))
            else {
                bail!("Cyclic pass dependencies");
            };
            order.push(*id);
            done.insert(*id);
        }

        let effect_inputs_decl = instance
            .inputs
            .iter()
            .map(|(id, slot)| (*id, slot.ty.clone()))
            .collect::<EffectInputsDecl>();

        let mut passes = Vec::with_capacity(order.len());
        for id in order {
            let source = &instance.passes[&id];
            // Decl maps are single HashMaps iterated by both layout and resource
            // sides, so their iteration order stays consistent within a renderer.
            let inputs_decl = pass_inputs
                .remove(&id)
                .unwrap()
                .into_iter()
                .collect::<EffectPassInputsDecl>();
            let outputs_decl = pass_outputs
                .remove(&id)
                .unwrap()
                .into_iter()
                .collect::<EffectPassOutputsDecl>();

            for source_ref in inputs_decl.values() {
                let ty = match source_ref {
                    EffectPassInputSlotSource::PassOutput(id) => pass_output_types
                        .get(id)
                        .with_context(|| format!("Unknown pass output {id:?}"))?,
                    EffectPassInputSlotSource::EffectInput(id) => effect_inputs_decl
                        .get(id)
                        .with_context(|| format!("Unknown effect input {id:?}"))?,
                };
                if ty.is::<LayerType>() {
                    ensure!(
                        matches!(
                            source.dispatch_strategy,
                            EffectPassDispatchStrategy::EveryOutputLayerPixel(_)
                                | EffectPassDispatchStrategy::EveryInputLayerPixel(_)
                        ),
                        "Layer pass inputs require layer pixel dispatch"
                    );
                }
                if ty.is::<ArrayType>()
                    || ty.is::<ArrayAtomicI32Type>()
                    || ty.is::<ArrayAtomicU32Type>()
                {
                    ensure!(
                        matches!(
                            source.dispatch_strategy,
                            EffectPassDispatchStrategy::EveryBufferElement(_)
                        ),
                        "Array pass inputs require EveryBufferElement dispatch"
                    );
                }
            }

            if outputs_decl
                .values()
                .any(|output| output.ty.is::<LayerType>())
            {
                ensure!(
                    matches!(
                        source.dispatch_strategy,
                        EffectPassDispatchStrategy::EveryOutputLayerPixel(_)
                    ),
                    "Layer pass outputs require EveryOutputLayerPixel dispatch"
                );
            }

            match source.dispatch_strategy {
                EffectPassDispatchStrategy::Once => {}
                EffectPassDispatchStrategy::EveryBufferElement(id) => {
                    let source = inputs_decl
                        .get(&id)
                        .context("Dispatch input is not in this pass")?;
                    match source {
                        EffectPassInputSlotSource::PassOutput(slot) => {
                            let ty = pass_output_types.get(slot).context("Unknown pass output")?;
                            ensure!(
                                ty.is::<ArrayType>()
                                    || ty.is::<ArrayAtomicI32Type>()
                                    || ty.is::<ArrayAtomicU32Type>(),
                                "EveryBufferElement requires Array"
                            );
                        }
                        EffectPassInputSlotSource::EffectInput(slot) => {
                            let ty = &inputs.get(slot).context("Unknown effect input")?.ty;
                            ensure!(
                                ty.is::<ArrayType>()
                                    || ty.is::<ArrayAtomicI32Type>()
                                    || ty.is::<ArrayAtomicU32Type>(),
                                "EveryBufferElement requires Array"
                            );
                        }
                    }
                }
                EffectPassDispatchStrategy::EveryOutputLayerPixel(id) => {
                    let target = outputs_decl
                        .get(&id)
                        .context("Dispatch output is not in this pass")?;
                    ensure!(
                        target.ty.is::<LayerType>(),
                        "EveryOutputLayerPixel requires Layer"
                    );
                }
                EffectPassDispatchStrategy::EveryInputLayerPixel(id) => {
                    let source = inputs_decl
                        .get(&id)
                        .context("Dispatch input is not in this pass")?;
                    let ty = match source {
                        EffectPassInputSlotSource::PassOutput(id) => {
                            pass_output_types.get(id).context("Unknown pass output")?
                        }
                        EffectPassInputSlotSource::EffectInput(id) => {
                            effect_inputs_decl.get(id).context("Unknown effect input")?
                        }
                    };
                    ensure!(ty.is::<LayerType>(), "EveryInputLayerPixel requires Layer");
                }
            }

            passes.push(EffectRenderPass::new(
                inputs_decl,
                outputs_decl,
                &effect_inputs_decl,
                &pass_output_types,
                &builtin_literals,
                source.dispatch_strategy,
                &source.graph,
                shader_deps,
                &device,
            )?);
        }

        Ok(Self {
            inputs: instance.inputs.values().cloned().collect(),
            outputs: exports,
            builtin_literals,
            passes,
            device,
            queue,
        })
    }

    pub fn run(
        &self,
        inputs: &EffectInputs,
        builtin_literals: &HashMap<String, GraphShaderLiteral>,
    ) -> Result<EffectOutputs> {
        for def in &self.inputs {
            let literal = inputs
                .get(&def.id)
                .with_context(|| format!("Missing effect input {:?}", def.id))?;
            ensure!(
                def.ty.id() == literal.ty().id(),
                "Wrong effect input type for {:?}",
                def.id
            );
        }
        for (name, ty) in &self.builtin_literals {
            let literal = builtin_literals
                .get(name)
                .with_context(|| format!("Missing effect builtin literal '{name}'"))?;
            ensure!(
                ty.id() == literal.ty().id(),
                "Wrong effect builtin literal type for '{name}'"
            );
        }

        let mut produced = EffectPassOutputs::new();
        for pass in &self.passes {
            pass.init_output_values(&mut produced, &self.device, &self.queue)?;
            pass.run(
                inputs,
                builtin_literals,
                &mut produced,
                &self.device,
                &self.queue,
            )?;
        }

        self.outputs
            .iter()
            .map(|(id, source)| {
                Ok((
                    *id,
                    produced.remove(source).context("Missing export resource")?,
                ))
            })
            .collect()
    }
}

struct PreparedEffectRenderPassStage {
    input_group: BindGroup,
    output_group: BindGroup,
    builtin_group: BindGroup,
    dispatch: [u32; 3],
}

struct EffectRenderPassStage {
    is_eval: bool,
    pipeline: ComputePipeline,
    input_layout: BindGroupLayout,
    output_layout: BindGroupLayout,
    builtin_layout: BindGroupLayout,
    dispatch: EffectPassDispatchStrategy,
}

impl EffectRenderPassStage {
    fn new(
        device: &Device,
        graph: &Graph,
        pass_inputs_decl: &EffectPassInputsDecl,
        pass_outputs_decl: &EffectPassOutputsDecl,
        effect_inputs_decl: &EffectInputsDecl,
        pass_output_types: &HashMap<EffectPassOutputSlotId, Arc<dyn ErasedGraphValueType>>,
        builtin_literals: &HashMap<String, Arc<dyn ErasedGraphValueType>>,
        dispatch: EffectPassDispatchStrategy,
        is_eval: bool,
        shader_deps: &[&CodegenPkg],
    ) -> Result<Self> {
        let mut declarations = String::new();

        let input_entries = {
            let mut binding = 0;
            let mut bindings = DynamicBindGroupLayoutEntries::new(ShaderStages::COMPUTE);
            for (id, source) in pass_inputs_decl {
                let ty = match source {
                    EffectPassInputSlotSource::PassOutput(id) => pass_output_types
                        .get(id)
                        .with_context(|| format!("Unknown pass output {id:?}"))?,
                    EffectPassInputSlotSource::EffectInput(id) => {
                        effect_inputs_decl.get(id).unwrap()
                    }
                };
                let (next, extended, shader) = ty.push_shader_layout(
                    &pass_input_ident(*id),
                    GraphShaderStage::Input,
                    0,
                    binding,
                    bindings,
                    declarations,
                )?;
                binding = next;
                bindings = extended;
                declarations = shader;
            }
            bindings
        };

        let output_entries = {
            let stage = if is_eval {
                GraphShaderStage::Eval
            } else {
                GraphShaderStage::Main
            };
            let mut binding = 0;
            let mut bindings = DynamicBindGroupLayoutEntries::new(ShaderStages::COMPUTE);
            for (id, target) in pass_outputs_decl {
                let (next, extended, shader) = target.ty.push_shader_layout(
                    &pass_output_ident(*id),
                    stage,
                    1,
                    binding,
                    bindings,
                    declarations,
                )?;
                binding = next;
                bindings = extended;
                declarations = shader;
            }
            bindings
        };

        let builtin_entries = {
            let mut binding = 0;
            let mut bindings = DynamicBindGroupLayoutEntries::new(ShaderStages::COMPUTE);
            let mut names = builtin_literals.keys().collect::<Vec<_>>();
            names.sort();
            for name in names {
                let (next, extended, shader) = builtin_literals[name].push_shader_layout(
                    name,
                    GraphShaderStage::Input,
                    2,
                    binding,
                    bindings,
                    declarations,
                )?;
                binding = next;
                bindings = extended;
                declarations = shader;
            }
            bindings
        };

        let template =
            include_str!("effect_template.wesl").replace("//CODEGEN_FLAG_BINDINGS", &declarations);

        let wgsl = compile_shader(
            graph,
            template,
            pass_inputs_decl,
            pass_outputs_decl,
            effect_inputs_decl,
            pass_output_types,
            builtin_literals,
            dispatch,
            is_eval,
            shader_deps,
        )?;
        let input_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("effect inputs"),
            entries: &input_entries,
        });
        let output_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("effect outputs"),
            entries: &output_entries,
        });
        let builtin_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("effect builtin literals"),
            entries: &builtin_entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("effect pipeline layout"),
            bind_group_layouts: &[
                Some(&input_layout),
                Some(&output_layout),
                Some(&builtin_layout),
            ],
            immediate_size: 0,
        });
        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("effect shader"),
            source: ShaderSource::Wgsl(wgsl.into()),
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("effect pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some(if is_eval { "eval" } else { "main" }),
            compilation_options: Default::default(),
            cache: None,
        });
        Ok(Self {
            is_eval,
            pipeline,
            input_layout,
            output_layout,
            builtin_layout,
            dispatch,
        })
    }

    fn prepare(
        &self,
        pass_inputs_decl: &EffectPassInputsDecl,
        pass_outputs_decl: &EffectPassOutputsDecl,
        pass_outputs_global: &EffectPassOutputs,
        effect_inputs: &EffectInputs,
        builtin_literals: &HashMap<String, GraphShaderLiteral>,
        device: &Device,
    ) -> Result<PreparedEffectRenderPassStage> {
        let input_entries = {
            let mut binding = 0;
            let mut bindings = DynamicBindGroupEntries::new();
            for source in pass_inputs_decl.values() {
                let literal = match source {
                    EffectPassInputSlotSource::PassOutput(id) => {
                        pass_outputs_global.get(id).unwrap()
                    }
                    EffectPassInputSlotSource::EffectInput(id) => effect_inputs.get(id).unwrap(),
                };
                let (next, extended) = literal.ty().push_shader_binding(
                    GraphShaderStage::Input,
                    literal.value(),
                    binding,
                    bindings,
                )?;
                binding = next;
                bindings = extended;
            }
            bindings
        };

        let output_entries = {
            let stage = if self.is_eval {
                GraphShaderStage::Eval
            } else {
                GraphShaderStage::Main
            };
            let mut binding = 0;
            let mut bindings = DynamicBindGroupEntries::new();
            for id in pass_outputs_decl.keys() {
                let literal = pass_outputs_global.get(id).unwrap();
                let (next, extended) =
                    literal
                        .ty()
                        .push_shader_binding(stage, literal.value(), binding, bindings)?;
                binding = next;
                bindings = extended;
            }
            bindings
        };

        let input_group = device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &self.input_layout,
            entries: &input_entries,
        });
        let output_group = device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &self.output_layout,
            entries: &output_entries,
        });
        let builtin_entries = {
            let mut binding = 0;
            let mut bindings = DynamicBindGroupEntries::new();
            let mut names = builtin_literals.keys().collect::<Vec<_>>();
            names.sort();
            for name in names {
                let literal = &builtin_literals[name];
                let (next, extended) = literal.ty().push_shader_binding(
                    GraphShaderStage::Input,
                    literal.value(),
                    binding,
                    bindings,
                )?;
                binding = next;
                bindings = extended;
            }
            bindings
        };
        let builtin_group = device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &self.builtin_layout,
            entries: &builtin_entries,
        });

        let dispatch = if self.is_eval {
            [1, 1, 1]
        } else {
            match self.dispatch {
                EffectPassDispatchStrategy::Once => [1, 1, 1],
                EffectPassDispatchStrategy::EveryBufferElement(id) => {
                    let literal = match pass_inputs_decl
                        .get(&id)
                        .context("Dispatch input is not in this pass")?
                    {
                        EffectPassInputSlotSource::PassOutput(id) => {
                            pass_outputs_global.get(id).unwrap()
                        }
                        EffectPassInputSlotSource::EffectInput(id) => {
                            effect_inputs.get(id).unwrap()
                        }
                    };
                    let len = literal
                        .try_as_ref::<PreparedArray>()
                        .map(|array| array.len)
                        .or_else(|| {
                            literal
                                .try_as_ref::<PreparedAtomicArray>()
                                .map(|array| array.len)
                        })
                        .context("EveryBufferElement requires Array")?;
                    [len.div_ceil(64), 1, 1]
                }
                EffectPassDispatchStrategy::EveryOutputLayerPixel(id) => {
                    let layer = pass_outputs_global
                        .get(&id)
                        .context("Dispatch output is not in this pass")?
                        .try_as_ref::<PreparedLayer>()
                        .context("EveryOutputLayerPixel requires Layer")?;
                    let PreparedLayerPixels::ReadWrite { storage, .. } = &layer.pixels else {
                        bail!("EveryOutputLayerPixel requires a writable layer");
                    };
                    let side = DynamicLayerStorage::TILE_SIZE.div_ceil(16);
                    [side, side, u32::try_from(storage.len())?]
                }
                EffectPassDispatchStrategy::EveryInputLayerPixel(id) => {
                    let literal = match pass_inputs_decl
                        .get(&id)
                        .context("Dispatch input is not in this pass")?
                    {
                        EffectPassInputSlotSource::PassOutput(id) => {
                            pass_outputs_global.get(id).unwrap()
                        }
                        EffectPassInputSlotSource::EffectInput(id) => {
                            effect_inputs.get(id).unwrap()
                        }
                    };
                    let layer = literal
                        .try_as_ref::<PreparedLayer>()
                        .context("EveryInputLayerPixel requires Layer")?;
                    let PreparedLayerPixels::ReadWrite { storage, .. } = &layer.pixels else {
                        bail!("EveryInputLayerPixel requires a writable layer");
                    };
                    let side = DynamicLayerStorage::TILE_SIZE.div_ceil(16);
                    [side, side, u32::try_from(storage.len())?]
                }
            }
        };

        Ok(PreparedEffectRenderPassStage {
            input_group,
            output_group,
            builtin_group,
            dispatch,
        })
    }

    fn dispatch(
        &self,
        encoder: &mut CommandEncoder,
        prepared: &PreparedEffectRenderPassStage,
    ) -> Result<()> {
        let mut compute = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("effect pass"),
            timestamp_writes: None,
        });
        compute.set_pipeline(&self.pipeline);
        compute.set_bind_group(0, &prepared.input_group, &[]);
        compute.set_bind_group(1, &prepared.output_group, &[]);
        compute.set_bind_group(2, &prepared.builtin_group, &[]);
        compute.dispatch_workgroups(
            prepared.dispatch[0],
            prepared.dispatch[1],
            prepared.dispatch[2],
        );
        Ok(())
    }
}

struct EffectRenderPass {
    inputs_decl: EffectPassInputsDecl,
    outputs_decl: EffectPassOutputsDecl,
    eval: Option<EffectRenderPassStage>,
    main: EffectRenderPassStage,
}

impl EffectRenderPass {
    fn new(
        pass_inputs_decl: EffectPassInputsDecl,
        pass_outputs_decl: EffectPassOutputsDecl,
        effect_inputs_decl: &EffectInputsDecl,
        pass_output_types: &HashMap<EffectPassOutputSlotId, Arc<dyn ErasedGraphValueType>>,
        builtin_literals: &HashMap<String, Arc<dyn ErasedGraphValueType>>,
        dispatch: EffectPassDispatchStrategy,
        graph: &Graph,
        shader_deps: &[&CodegenPkg],
        device: &Device,
    ) -> Result<Self> {
        let requires_eval = pass_outputs_decl.values().any(|v| v.ty.requires_eval());

        let eval = requires_eval
            .then(|| {
                EffectRenderPassStage::new(
                    device,
                    graph,
                    &pass_inputs_decl,
                    &pass_outputs_decl,
                    effect_inputs_decl,
                    pass_output_types,
                    builtin_literals,
                    dispatch,
                    true,
                    shader_deps,
                )
            })
            .transpose()?;
        let main = EffectRenderPassStage::new(
            device,
            graph,
            &pass_inputs_decl,
            &pass_outputs_decl,
            effect_inputs_decl,
            pass_output_types,
            builtin_literals,
            dispatch,
            false,
            shader_deps,
        )?;

        Ok(Self {
            inputs_decl: pass_inputs_decl,
            outputs_decl: pass_outputs_decl,
            eval,
            main,
        })
    }

    fn init_output_values(
        &self,
        pass_output_values_global: &mut EffectPassOutputs,
        device: &Device,
        queue: &Queue,
    ) -> Result<()> {
        for (id, output) in &self.outputs_decl {
            let literal = output.ty.default_literal();
            let prepared = output
                .ty
                .prepare_to_shader(literal.as_ref(), device, queue)?;
            pass_output_values_global.insert(
                *id,
                GraphShaderLiteral::new_boxed(prepared, output.ty.clone()),
            );
        }
        Ok(())
    }

    fn run(
        &self,
        effect_inputs: &EffectInputs,
        builtin_literals: &HashMap<String, GraphShaderLiteral>,
        pass_outputs_global: &mut EffectPassOutputs,
        device: &Device,
        queue: &Queue,
    ) -> Result<()> {
        if let Some(eval) = &self.eval {
            let mut encoder = device.create_command_encoder(&Default::default());
            let eval_prepared = eval.prepare(
                &self.inputs_decl,
                &self.outputs_decl,
                pass_outputs_global,
                effect_inputs,
                builtin_literals,
                device,
            )?;
            eval.dispatch(&mut encoder, &eval_prepared)?;
            queue.submit([encoder.finish()]);

            for id in self.outputs_decl.keys() {
                let output = pass_outputs_global.get_mut(id).unwrap();
                let ty = output.ty().clone();
                ty.post_eval(output.value_mut(), device, queue)?;
            }
        }

        let mut encoder = device.create_command_encoder(&Default::default());
        let main_prepared = self.main.prepare(
            &self.inputs_decl,
            &self.outputs_decl,
            pass_outputs_global,
            effect_inputs,
            builtin_literals,
            device,
        )?;
        self.main.dispatch(&mut encoder, &main_prepared)?;
        queue.submit([encoder.finish()]);
        Ok(())
    }
}

fn compile_shader(
    graph: &Graph,
    template: String,
    pass_inputs_decl: &EffectPassInputsDecl,
    pass_outputs_decl: &EffectPassOutputsDecl,
    effect_inputs_decl: &EffectInputsDecl,
    pass_output_types: &EffectPassOutputTypes,
    builtin_literals: &HashMap<String, Arc<dyn ErasedGraphValueType>>,
    dispatch: EffectPassDispatchStrategy,
    is_eval: bool,
    dependencies: &[&CodegenPkg],
) -> Result<String> {
    let (_, _, graph_shader, extra_shader_body) = graph
        .compile(Vec::new(), GraphVarIdentGenerator::default())
        .context("Effect graph code generation failed")?;

    let mut resource_helpers = String::new();
    for (id, source) in pass_inputs_decl {
        let ty = match source {
            EffectPassInputSlotSource::PassOutput(id) => pass_output_types
                .get(id)
                .with_context(|| format!("Unknown pass output {id:?}"))?,
            EffectPassInputSlotSource::EffectInput(id) => effect_inputs_decl
                .get(id)
                .with_context(|| format!("Unknown effect input {id:?}"))?,
        };

        if let Some(body) =
            ty.generate_extra_shader_body(GraphShaderStage::Input, &pass_input_ident(*id))
        {
            resource_helpers.push_str(&body);
        }
    }
    let mut builtin_names = builtin_literals.keys().collect::<Vec<_>>();
    builtin_names.sort();
    for name in builtin_names {
        if let Some(body) =
            builtin_literals[name].generate_extra_shader_body(GraphShaderStage::Input, name)
        {
            resource_helpers.push_str(&body);
        }
    }
    for (id, target) in pass_outputs_decl {
        if let Some(body) = target.ty.generate_extra_shader_body(
            if is_eval {
                GraphShaderStage::Eval
            } else {
                GraphShaderStage::Main
            },
            &pass_output_ident(*id),
        ) {
            resource_helpers.push_str(&body);
        }
    }

    let setup = dispatch_setup(dispatch)?;
    let mut shader = template
        .replace("//CODEGEN_FLAG_RESOURCE_HELPERS", &resource_helpers)
        .replace("//CODEGEN_FLAG_COMPILED_GRAPH", &graph_shader)
        .replace("//CODEGEN_FLAG_DISPATCH_SETUP", &setup);

    for body in extra_shader_body
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        .into_values()
    {
        body.inject_body(&mut shader);
    }

    let mut deps = vec![&image::PACKAGE, &render::PACKAGE];
    deps.extend_from_slice(dependencies);

    wesl_jit::compile_wesl_with_config(shader, &deps, |compiler| {
        compiler.set_feature("EVAL", is_eval);
        match dispatch {
            EffectPassDispatchStrategy::Once => {
                compiler.set_feature("DISPATCH_ONCE", true);
            }
            EffectPassDispatchStrategy::EveryBufferElement(_) => {
                compiler.set_feature("DISPATCH_EVERY_BUFFER_ELEMENT", true);
            }
            EffectPassDispatchStrategy::EveryOutputLayerPixel(_) => {
                compiler.set_feature("DISPATCH_EVERY_OUTPUT_LAYER_PIXEL", true);
            }
            EffectPassDispatchStrategy::EveryInputLayerPixel(_) => {
                compiler.set_feature("DISPATCH_EVERY_INPUT_LAYER_PIXEL", true);
            }
        }
    })
    .context("Effect WESL compilation failed")
}

fn dispatch_setup(dispatch: EffectPassDispatchStrategy) -> Result<String> {
    Ok(match dispatch {
        EffectPassDispatchStrategy::Once => quote_statement! {
            let dispatch_index = 0u;
        }
        .to_string(),
        EffectPassDispatchStrategy::EveryBufferElement(id) => {
            let name = Ident::new(pass_input_ident(id));
            format!(
                "{}\n{}\n",
                quote_statement! {
                    if id.x >= arrayLength(&#name) { return; }
                },
                quote_statement! {
                    let dispatch_index = id.x;
                }
            )
        }
        EffectPassDispatchStrategy::EveryOutputLayerPixel(id) => {
            let name = pass_output_ident(id);
            let tile_info = Ident::new(layer_tile_info_ident(&name));
            format!(
                "{}\n{}\n",
                quote_statement! {
                    if id.z >= arrayLength(&#tile_info) { return; }
                },
                quote_statement! {
                    let dispatch_index = #tile_info[id.z].origin + vec2i(id.xy);
                }
            )
        }
        EffectPassDispatchStrategy::EveryInputLayerPixel(id) => {
            let name = pass_input_ident(id);
            let tile_info = Ident::new(layer_tile_info_ident(&name));
            format!(
                "{}\n{}\n",
                quote_statement! {
                    if id.z >= arrayLength(&#tile_info) { return; }
                },
                quote_statement! {
                    let dispatch_index = #tile_info[id.z].origin + vec2i(id.xy);
                }
            )
        }
    })
}
