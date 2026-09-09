use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
    sync::Arc,
};

use lapiz_assets::asset::{AssetHandle, AssetId};
use lapiz_render::wesl_jit;
use lapiz_runtime::Services;
use lapiz_shader_graph::{
    graph::{
        Graph, GraphResources,
        external::{
            ExternalVariable, ExternalVariableId, GraphExternalVariableStorage,
            generate_external_variable_binding,
        },
        function::{ASSET_GRAPH_FUNCTION_STORAGE, SharedGraphFunctionStorage},
        slot::ErasedGraphLiteralUpdateMessage,
        texture::{
            ASSET_GRAPH_TEXTURE_STORAGE, GraphTextureUsageRecorder, SharedGraphTextureStorage,
        },
    },
    save::{GraphDeserializeError, SerializableExternalVariable},
};

pub use crate::render::graph::{FILTER_GRAPH_NODES, FILTER_GRAPH_TYPES};
use crate::{
    asset::{
        FilterGroupId, FilterPreset, FilterPresetMetadata, FilterSlotRef, SerializableFilterGroup,
    },
    render::graph::FilterGraphData,
};

pub const EXTERNAL_VARIABLE_BASE_BINDING: u32 = 32;

pub struct CompiledFilterGroup {
    pub id: FilterGroupId,
    pub input: FilterSlotRef,
    pub output: FilterSlotRef,
    pub main: String,
    pub bounds_eval: String,
}

pub struct CompiledFilterPreset {
    pub groups: Vec<CompiledFilterGroup>,
    pub texture_usage: GraphTextureUsageRecorder,
    pub external_vars: Arc<GraphExternalVariableStorage>,
}

impl Display for CompiledFilterPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "-------------- Compiled filter preset --------------")?;
        for group in &self.groups {
            writeln!(f, "-------------- Filter group {} --------------", group.id)?;
            writeln!(f, "main:\n{}", group.main)?;
            writeln!(f, "bounds_eval:\n{}", group.bounds_eval)?;
        }
        Ok(())
    }
}

pub struct FilterGroup {
    pub id: FilterGroupId,
    pub name: String,
    pub input: FilterSlotRef,
    pub output: FilterSlotRef,
    pub graph: Graph<FilterGraphData>,
}

pub struct FilterInstance {
    filter_id: Option<AssetId<FilterPreset>>,
    metadata: FilterPresetMetadata,
    textures: SharedGraphTextureStorage,
    functions: SharedGraphFunctionStorage,
    groups: Vec<FilterGroup>,
    external_vars: Arc<GraphExternalVariableStorage>,
}

impl FilterInstance {
    pub fn from_asset(
        handle: &AssetHandle<FilterPreset>,
        _services: &Services,
    ) -> (Option<Self>, Vec<GraphDeserializeError>) {
        let Ok(preset) = handle.get() else {
            log::warn!("Filter preset asset is not loaded yet");
            return (None, Vec::new());
        };
        let (mut instance, errors) = Self::new(
            &preset,
            ASSET_GRAPH_TEXTURE_STORAGE.clone(),
            ASSET_GRAPH_FUNCTION_STORAGE.clone(),
        );
        if let Some(instance) = instance.as_mut() {
            instance.filter_id = Some(handle.id());
        }
        (instance, errors)
    }

    pub fn new(
        preset: &FilterPreset,
        textures: SharedGraphTextureStorage,
        functions: SharedGraphFunctionStorage,
    ) -> (Option<Self>, Vec<GraphDeserializeError>) {
        let external_vars = preset
            .external_vars
            .iter()
            .filter_map(|var| {
                var.deserialize(FILTER_GRAPH_TYPES.as_ref())
                    .inspect_err(|err| {
                        log::error!(
                            "Error deserializing external variable '{}': {}",
                            var.name,
                            err
                        );
                    })
                    .ok()
            })
            .collect::<Vec<_>>();
        let external_vars = Arc::new(GraphExternalVariableStorage::new(external_vars));

        let mut errors = Vec::new();
        let mut groups = Vec::with_capacity(preset.groups.len());
        for serialized in &preset.groups {
            let (g, e) = Graph::<FilterGraphData>::from_serialized(
                &serialized.graph,
                GraphResources {
                    type_registry: FILTER_GRAPH_TYPES.clone(),
                    node_registry: FILTER_GRAPH_NODES.clone(),
                    textures: textures.clone(),
                    functions: functions.clone(),
                    external_vars: external_vars.clone(),
                },
            );
            errors.extend(e);
            let Some(graph) = g else {
                return (None, errors);
            };
            groups.push(FilterGroup {
                id: serialized.id,
                name: serialized.name.clone(),
                input: serialized.input,
                output: serialized.output,
                graph,
            });
        }

        (
            Some(Self {
                filter_id: None,
                metadata: preset.metadata.clone(),
                textures,
                functions,
                groups,
                external_vars,
            }),
            errors,
        )
    }

    pub fn as_asset(&self) -> anyhow::Result<FilterPreset> {
        let groups = self
            .groups
            .iter()
            .map(|group| {
                Ok(SerializableFilterGroup {
                    id: group.id,
                    name: group.name.clone(),
                    input: group.input,
                    output: group.output,
                    graph: group.graph.as_serialized()?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let external_vars = self
            .external_vars
            .all()
            .iter()
            .map(|entry| SerializableExternalVariable::serialize(entry.value()))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(FilterPreset {
            metadata: self.metadata.clone(),
            groups,
            external_vars,
        })
    }

    pub fn topological_order(&self) -> Vec<usize> {
        let n = self.groups.len();
        let index_of = self
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| (g.id, i))
            .collect::<HashMap<_, _>>();

        let mut consumers = vec![Vec::<usize>::new(); n];
        let mut in_degree = vec![0usize; n];

        for (consumer_index, group) in self.groups.iter().enumerate() {
            if let FilterSlotRef::Group(producer_id) = group.input
                && let Some(&producer_index) = index_of.get(&FilterGroupId::new(producer_id))
            {
                consumers[producer_index].push(consumer_index);
                in_degree[consumer_index] += 1;
            }
        }

        let mut queue = (0..n)
            .filter(|&i| in_degree[i] == 0)
            .collect::<VecDeque<_>>();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = queue.pop_front() {
            order.push(i);
            for &consumer in &consumers[i] {
                in_degree[consumer] -= 1;
                if in_degree[consumer] == 0 {
                    queue.push_back(consumer);
                }
            }
        }

        for i in 0..n {
            if !order.contains(&i) {
                order.push(i);
            }
        }
        order
    }

    pub fn final_group_index(&self) -> usize {
        self.groups
            .iter()
            .position(|g| matches!(g.output, FilterSlotRef::Layer))
            .unwrap_or(0)
    }

    pub fn groups(&self) -> &[FilterGroup] {
        &self.groups
    }

    pub fn groups_mut(&mut self) -> &mut Vec<FilterGroup> {
        &mut self.groups
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub fn new_group(&mut self) -> usize {
        let index = self.groups.len();
        let graph = Graph::new(GraphResources {
            type_registry: FILTER_GRAPH_TYPES.clone(),
            node_registry: FILTER_GRAPH_NODES.clone(),
            textures: self.textures.clone(),
            functions: self.functions.clone(),
            external_vars: self.external_vars.clone(),
        });
        self.groups.push(FilterGroup {
            id: FilterGroupId(uuid::Uuid::new_v4()),
            name: format!("Group {}", index + 1),
            input: FilterSlotRef::Layer,
            output: FilterSlotRef::Layer,
            graph,
        });
        index
    }

    pub fn remove_group(&mut self, index: usize) {
        if index < self.groups.len() {
            self.groups.remove(index);
        }
    }

    pub fn move_group(&mut self, from: usize, to: usize) {
        if from >= self.groups.len() || to >= self.groups.len() || from == to {
            return;
        }
        let group = self.groups.remove(from);
        self.groups.insert(to, group);
    }

    pub fn asset_id(&self) -> Option<AssetId<FilterPreset>> {
        self.filter_id
    }

    pub fn metadata(&self) -> &FilterPresetMetadata {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut FilterPresetMetadata {
        &mut self.metadata
    }

    pub fn iter_external_vars(
        &self,
    ) -> impl Iterator<Item = (ExternalVariableId, ExternalVariable)> + '_ {
        self.external_vars.all().iter().map(|entry| {
            let var = entry.value().clone();
            (var.id, var)
        })
    }

    pub fn insert_external_var(&mut self, var: ExternalVariable) {
        self.external_vars.insert(var);
    }

    pub fn rename_external_var(&mut self, id: &ExternalVariableId, new_name: String) {
        self.external_vars.rename(id, new_name);
    }

    pub fn update_external_var(
        &self,
        id: &ExternalVariableId,
        message: ErasedGraphLiteralUpdateMessage,
    ) {
        self.external_vars.update(id, message);
    }

    pub fn remove_external_var(&mut self, id: &ExternalVariableId) {
        self.external_vars.remove(id);
    }

    #[tracing::instrument(skip_all, name = "compile_filter_preset")]
    pub fn compile(&self) -> anyhow::Result<CompiledFilterPreset> {
        let mut external_variable_bindings = String::new();
        for (binding, entry) in
            (EXTERNAL_VARIABLE_BASE_BINDING..).zip(self.external_vars.all().iter())
        {
            external_variable_bindings.push_str(&generate_external_variable_binding(
                0,
                binding,
                entry.value(),
            ));
        }

        let mut texture_usage = GraphTextureUsageRecorder::default();

        let mut groups = Vec::with_capacity(self.groups.len());
        for group_index in self.topological_order() {
            let group = &self.groups[group_index];
            let (_, _, shader) =
                group
                    .graph
                    .compile(Vec::new(), Default::default(), &mut texture_usage)?;

            let compiled = CompiledFilterGroup {
                id: group.id,
                input: group.input,
                output: group.output,
                main: compile_template(&shader, &external_variable_bindings, false)?,
                bounds_eval: compile_template(&shader, &external_variable_bindings, true)?,
            };
            groups.push(compiled);
        }

        let external_vars = self.external_vars.clone();

        Ok(CompiledFilterPreset {
            groups,
            texture_usage,
            external_vars,
        })
    }
}

fn compile_template(
    graph_shader: &str,
    external_variable_bindings: &str,
    bounds_eval: bool,
) -> anyhow::Result<String> {
    let shader = include_str!("render/filter_template.wesl")
        .replace("//CODEGENFLAG_COMPILED_GRAPH", graph_shader)
        .replace(
            "//CODEGENFLAG_EXTERNAL_VARIABLE_BINDINGS",
            external_variable_bindings,
        );

    let compiled_shader = wesl_jit::compile_wesl_with_config(
        shader,
        &[&lapiz_image::image::PACKAGE, &lapiz_render::render::PACKAGE],
        |compiler| {
            compiler.set_feature("BOUNDS_EVAL", bounds_eval);
        },
    )?;

    Ok(compiled_shader)
}
