use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock},
};

use anyhow::{Context as _, Result, bail};
use indexmap::IndexMap;
use lapiz_assets::{
    asset::{AssetHandle, AssetId},
    store::AssetRegistry,
};
use lapiz_effect::{
    asset::{EffectInputSlotId, EffectOutputSlotId, EffectPassInputSlotId, EffectPassOutputSlotId},
    instance::{EffectInputs, EffectInstance},
    nodes::{PassInput, PassInputNode, PassOutput, PassOutputNode},
    render::{EffectRenderer, pass_input_ident, pass_output_ident},
};
use lapiz_image::{image, texel::TexelType};
use lapiz_render::{bind_group_layout_entries::DynamicBindGroupLayoutEntries, render, wesl_jit};
use lapiz_shader_graph::{
    graph::{
        GraphResources, GraphVarIdentGenerator,
        function::GraphFunction,
        node::GraphNodeRegistry,
        slot::{ErasedGraphLiteralUpdateMessage, ErasedGraphValueType, GraphShaderStage},
        variable::{GraphLiteral, GraphShaderLiteral},
    },
    save::SerializableGraphLiteral,
};
use wgpu::{BindGroupLayoutEntry, Device, Queue, ShaderStages};

use crate::{
    asset::{BrushPreset, BrushPresetMetadata, SerializableBrushParameter},
    brush,
    render::graph::{
        BRUSH_GRAPH_TYPES, MAIN_DAB_BUFFER, SPACING_OUTPUT, STROKE_RESULT, brush_builtin_types,
        brush_graph_resources, main_builtin_types, main_graph_nodes, postprocess_builtin_types,
        postprocess_graph_nodes, spacing_graph_nodes,
    },
};

const SPACING_RESOURCE_GROUP: u32 = 1;

#[derive(Clone)]
pub struct BrushParameter {
    pub name: String,
    pub value: GraphLiteral,
}

pub struct CompiledBrushPreset {
    pub spacing: String,
    pub spacing_resource_layouts: Vec<BindGroupLayoutEntry>,
    pub spacing_builtin_types: BTreeMap<String, Arc<dyn ErasedGraphValueType>>,
    pub spacing_parameters: Arc<[GraphLiteral]>,
    pub main: EffectRenderer,
    pub postprocess: EffectRenderer,
    pub main_inputs: EffectInputs,
    pub postprocess_inputs: EffectInputs,
    pub main_dab_output: EffectOutputSlotId,
    pub postprocess_output: EffectOutputSlotId,
}

pub struct BrushPresetInstance {
    brush_id: Option<AssetId<BrushPreset>>,
    metadata: BrushPresetMetadata,

    spacing_effect: EffectInstance,
    main_effect: EffectInstance,
    postprocess_effect: EffectInstance,
    parameters: IndexMap<EffectInputSlotId, BrushParameter>,
}

impl BrushPresetInstance {
    pub fn from_asset(handle: &AssetHandle<BrushPreset>, assets: AssetRegistry) -> Result<Self> {
        let preset = handle
            .get()
            .map_err(|error| anyhow::anyhow!("Brush preset asset is not loaded yet: {error}"))?;
        let mut instance = Self::new(&preset, assets)?;
        instance.brush_id = Some(handle.id());
        Ok(instance)
    }

    pub fn new(preset: &BrushPreset, assets: AssetRegistry) -> Result<Self> {
        let spacing_effect = EffectInstance::from_asset(
            &preset.spacing_effect,
            spacing_effect_resources(assets.clone()),
        )?;
        let main_effect =
            EffectInstance::from_asset(&preset.main_effect, main_effect_resources(assets.clone()))?;
        let postprocess_effect = EffectInstance::from_asset(
            &preset.postprocess_effect,
            postprocess_effect_resources(assets.clone()),
        )?;

        let mut parameters = IndexMap::new();
        for effect in [&spacing_effect, &main_effect, &postprocess_effect] {
            for (id, slot) in &effect.inputs {
                let value = preset
                    .parameters
                    .get(id)
                    .and_then(|persisted| {
                        persisted
                            .value
                            .deserialize(BRUSH_GRAPH_TYPES.as_ref(), &assets)
                            .map_err(|error| {
                                log::warn!(
                                    "Brush parameter '{}' failed to deserialize, using default: {error}",
                                    slot.name
                                );
                                error
                            })
                            .ok()
                    })
                    .unwrap_or_else(|| {
                        GraphLiteral::new_boxed(slot.ty.default_literal(), slot.ty.clone())
                    });
                parameters.insert(
                    *id,
                    BrushParameter {
                        name: slot.name.clone(),
                        value,
                    },
                );
            }
        }

        Ok(Self {
            brush_id: None,
            metadata: preset.metadata.clone(),
            spacing_effect,
            main_effect,
            postprocess_effect,
            parameters,
        })
    }

    pub fn as_asset(&self, assets: &AssetRegistry) -> Result<BrushPreset> {
        let parameters = self
            .parameters
            .iter()
            .map(|(id, parameter)| {
                Ok((
                    *id,
                    SerializableBrushParameter {
                        name: parameter.name.clone(),
                        value: SerializableGraphLiteral::serialize(&parameter.value, assets)?,
                    },
                ))
            })
            .collect::<Result<_>>()?;

        Ok(BrushPreset {
            metadata: self.metadata.clone(),
            spacing_effect: self.spacing_effect.as_asset()?,
            main_effect: self.main_effect.as_asset()?,
            postprocess_effect: self.postprocess_effect.as_asset()?,
            parameters,
        })
    }

    pub fn asset_id(&self) -> Option<AssetId<BrushPreset>> {
        self.brush_id
    }

    pub fn metadata(&self) -> &BrushPresetMetadata {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut BrushPresetMetadata {
        &mut self.metadata
    }

    pub fn spacing_effect(&self) -> &EffectInstance {
        &self.spacing_effect
    }

    pub fn spacing_effect_mut(&mut self) -> &mut EffectInstance {
        &mut self.spacing_effect
    }

    pub fn main_effect(&self) -> &EffectInstance {
        &self.main_effect
    }

    pub fn main_effect_mut(&mut self) -> &mut EffectInstance {
        &mut self.main_effect
    }

    pub fn postprocess_effect(&self) -> &EffectInstance {
        &self.postprocess_effect
    }

    pub fn postprocess_effect_mut(&mut self) -> &mut EffectInstance {
        &mut self.postprocess_effect
    }

    pub fn parameters(&self) -> &IndexMap<EffectInputSlotId, BrushParameter> {
        &self.parameters
    }

    pub fn parameters_mut(&mut self) -> &mut IndexMap<EffectInputSlotId, BrushParameter> {
        &mut self.parameters
    }

    pub fn update_parameter(
        &mut self,
        id: &EffectInputSlotId,
        message: ErasedGraphLiteralUpdateMessage,
    ) {
        if let Some(parameter) = self.parameters.get_mut(id) {
            parameter.value.update(message);
        }
    }

    pub fn resync_parameters(&mut self) {
        self.parameters.retain(|id, _| {
            [
                &self.spacing_effect,
                &self.main_effect,
                &self.postprocess_effect,
            ]
            .iter()
            .any(|effect| effect.inputs.contains_key(id))
        });
        for effect in [
            &self.spacing_effect,
            &self.main_effect,
            &self.postprocess_effect,
        ] {
            for (id, slot) in &effect.inputs {
                let parameter = self
                    .parameters
                    .entry(*id)
                    .or_insert_with(|| BrushParameter {
                        name: slot.name.clone(),
                        value: GraphLiteral::new_boxed(slot.ty.default_literal(), slot.ty.clone()),
                    });
                parameter.name = slot.name.clone();
            }
        }
    }

    #[tracing::instrument(skip_all, name = "compile_brush_preset")]
    pub fn compile(
        &self,
        target_layer_format: TexelType,
        selection_layer_format: TexelType,
        device: &Device,
        queue: &Queue,
    ) -> Result<CompiledBrushPreset> {
        let main_dab_output = named_output(&self.main_effect, MAIN_DAB_BUFFER)?;

        let spacing_builtin_types =
            brush_builtin_types(target_layer_format, selection_layer_format);
        let (spacing, spacing_resource_layouts, spacing_parameters) =
            self.compile_spacing_pass(&spacing_builtin_types)?;

        let shader_deps = &[&brush::PACKAGE];
        let main_types = main_builtin_types(target_layer_format, selection_layer_format);
        let postprocess_types =
            postprocess_builtin_types(target_layer_format, selection_layer_format);
        let main = EffectRenderer::from_instance(
            &self.main_effect,
            main_types.into_iter().collect(),
            shader_deps,
            device.clone(),
            queue.clone(),
        )?;
        let postprocess = EffectRenderer::from_instance(
            &self.postprocess_effect,
            postprocess_types.into_iter().collect(),
            shader_deps,
            device.clone(),
            queue.clone(),
        )?;

        Ok(CompiledBrushPreset {
            spacing,
            spacing_resource_layouts,
            spacing_builtin_types,
            spacing_parameters: spacing_parameters.into(),
            main,
            postprocess,
            main_inputs: self.prepare_effect_inputs(&self.main_effect, device, queue)?,
            postprocess_inputs: self.prepare_effect_inputs(
                &self.postprocess_effect,
                device,
                queue,
            )?,
            main_dab_output,
            postprocess_output: named_output(&self.postprocess_effect, STROKE_RESULT)?,
        })
    }

    fn prepare_effect_inputs(
        &self,
        effect: &EffectInstance,
        device: &Device,
        queue: &Queue,
    ) -> Result<EffectInputs> {
        effect
            .inputs
            .iter()
            .map(|(id, slot)| {
                let parameter = self
                    .parameters
                    .get(id)
                    .with_context(|| format!("Brush effect input {id:?} has no parameter"))?;
                let prepared = slot
                    .ty
                    .prepare_to_shader(parameter.value.value(), device, queue)?;
                Ok((
                    *id,
                    GraphShaderLiteral::new_boxed(prepared, slot.ty.clone()),
                ))
            })
            .collect()
    }

    fn compile_spacing_pass(
        &self,
        builtin_types: &BTreeMap<String, Arc<dyn ErasedGraphValueType>>,
    ) -> Result<(String, Vec<BindGroupLayoutEntry>, Vec<GraphLiteral>)> {
        if self.spacing_effect.passes.len() != 1 {
            bail!("Brush spacing effect must have exactly one pass");
        }
        let graph = &self.spacing_effect.passes.first().unwrap().1.graph;
        let spacing_port = pass_output_port_of(
            &self.spacing_effect,
            named_output(&self.spacing_effect, SPACING_OUTPUT)
                .context("Brush spacing effect has no 'spacing' output")?,
        )?;
        let spacing_ty = self
            .spacing_effect
            .outputs
            .values()
            .find(|slot| slot.name == SPACING_OUTPUT)
            .map(|slot| slot.ty.clone())
            .with_context(|| format!("Brush spacing effect has no '{SPACING_OUTPUT}' output"))?;

        let mut declarations = String::new();
        let mut layouts = Vec::new();
        let mut binding = 0;
        for (name, ty) in builtin_types {
            let (next, entries, shader) = ty.push_shader_layout(
                name,
                GraphShaderStage::Input,
                SPACING_RESOURCE_GROUP,
                binding,
                DynamicBindGroupLayoutEntries::new(ShaderStages::COMPUTE),
                declarations,
            )?;
            binding = next;
            layouts.extend(entries.to_vec());
            declarations = shader;
        }

        let mut parameters = Vec::new();
        for (id, slot) in &self.spacing_effect.inputs {
            let parameter = self
                .parameters
                .get(id)
                .with_context(|| format!("Brush effect input {id:?} has no parameter"))?;
            let port = pass_input_port_of(&self.spacing_effect, *id)?;
            let (next, entries, shader) = slot.ty.push_shader_layout(
                &pass_input_ident(port),
                GraphShaderStage::Input,
                SPACING_RESOURCE_GROUP,
                binding,
                DynamicBindGroupLayoutEntries::new(ShaderStages::COMPUTE),
                declarations,
            )?;
            binding = next;
            layouts.extend(entries.to_vec());
            declarations = shader;
            parameters.push(parameter.value.clone());
        }

        let (_, _, code, extra_shader_body) =
            graph.compile(Vec::new(), GraphVarIdentGenerator::default())?;
        let output = pass_output_ident(spacing_port);
        let output_ty = spacing_ty.wgsl_type_name().with_context(|| {
            format!(
                "Brush spacing output must be a nameable value type, got '{}'",
                spacing_ty.id().id
            )
        })?;
        let body = format!("var {output}: {output_ty};\n{code}return {output};\n");
        let mut shader = include_str!("render/brush_sample.wesl")
            .replace("//CODEGENFLAG_COMPUTED_GRAPH_REQUIRED_SPACING", &body)
            .replace("//CODEGENFLAG_INJECTED_RESOURCES", &declarations);

        for body in extra_shader_body
            .into_iter()
            .collect::<BTreeMap<_, _>>()
            .into_values()
        {
            body.inject_body(&mut shader);
        }

        let shader = wesl_jit::compile_wesl_with_config_and_include(
            shader,
            &[&image::PACKAGE, &render::PACKAGE],
            |resolver| {
                resolver.add_module(
                    "package::brush_types".parse().unwrap(),
                    include_str!("render/brush_types.wesl").into(),
                );
            },
            |_| {},
        )
        .context("Brush spacing WESL compilation failed")?;

        Ok((shader, layouts, parameters))
    }
}

fn named_output(effect: &EffectInstance, name: &str) -> Result<EffectOutputSlotId> {
    effect
        .outputs
        .iter()
        .find(|(_, output)| output.name == name)
        .map(|(id, _)| *id)
        .with_context(|| format!("Brush effect has no '{name}' output"))
}

fn pass_input_port_of(
    effect: &EffectInstance,
    effect_input: EffectInputSlotId,
) -> Result<EffectPassInputSlotId> {
    for pass in effect.passes.values() {
        for node in pass.graph.iter_nodes() {
            let Some(state) = node.data.state::<PassInputNode>() else {
                continue;
            };
            if state.input == Some(PassInput::Effect(effect_input)) {
                return Ok(state.id);
            }
        }
    }
    bail!("Brush effect input {effect_input:?} is not bound by a pass input node")
}

fn pass_output_port_of(
    effect: &EffectInstance,
    effect_output: EffectOutputSlotId,
) -> Result<EffectPassOutputSlotId> {
    for pass in effect.passes.values() {
        for node in pass.graph.iter_nodes() {
            let Some(state) = node.data.state::<PassOutputNode>() else {
                continue;
            };
            if matches!(state.output, Some(PassOutput::Effect(id)) if id == effect_output) {
                return Ok(state.id);
            }
        }
    }
    bail!("Brush effect output {effect_output:?} is not bound by a pass output node")
}

pub static SPACING_GRAPH_NODES: LazyLock<Arc<GraphNodeRegistry>> =
    LazyLock::new(|| Arc::new(spacing_graph_nodes()));
pub static MAIN_GRAPH_NODES: LazyLock<Arc<GraphNodeRegistry>> =
    LazyLock::new(|| Arc::new(main_graph_nodes()));
pub static POSTPROCESS_GRAPH_NODES: LazyLock<Arc<GraphNodeRegistry>> =
    LazyLock::new(|| Arc::new(postprocess_graph_nodes()));

pub fn spacing_effect_resources(assets: AssetRegistry) -> GraphResources {
    brush_graph_resources(SPACING_GRAPH_NODES.clone(), assets)
}

pub fn main_effect_resources(assets: AssetRegistry) -> GraphResources {
    brush_graph_resources(MAIN_GRAPH_NODES.clone(), assets)
}

pub fn postprocess_effect_resources(assets: AssetRegistry) -> GraphResources {
    brush_graph_resources(POSTPROCESS_GRAPH_NODES.clone(), assets)
}

pub struct GraphFunctionInstance {
    graph_function: GraphFunction,
}

impl GraphFunctionInstance {
    pub fn new(graph_function: GraphFunction) -> Self {
        Self { graph_function }
    }

    pub fn graph_function(&self) -> &GraphFunction {
        &self.graph_function
    }

    pub fn graph_function_mut(&mut self) -> &mut GraphFunction {
        &mut self.graph_function
    }
}
