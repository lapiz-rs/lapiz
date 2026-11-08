use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use anyhow::{Context as _, Result};
use bevy_math::IRect;
use glam::UVec2;
use lapiz_image::image;
use lapiz_render::{
    bind_group_entries::{BindGroupEntries, DynamicBindGroupEntries},
    bind_group_layout_entries::{
        BindGroupLayoutEntries, DynamicBindGroupLayoutEntries, binding_types,
    },
    render,
    storage_buffer::StorageBuffer,
    uniform_buffer::DynamicUniformBuffer,
    wesl_jit,
};
use lapiz_shader_graph::{
    graph::{
        Graph, GraphVarIdentGenerator,
        slot::{ErasedGraphValueType, GraphShaderStage, GraphValueType},
        variable::GraphShaderLiteral,
    },
    wgsl_std::types::{
        handle::{LayerReference, LayerType, PreparedLayer},
        layer_bounds_ident,
    },
};
#[allow(
    clippy::wildcard_imports,
    reason = "wesl quote macros resolve syntax types from this prelude"
)]
use wesl::syntax::*;
use wesl_quote::{quote_declaration, quote_statement};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, Buffer,
    CommandEncoder, ComputePass, ComputePipeline, ComputePipelineDescriptor, Device,
    PipelineLayoutDescriptor, Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages,
};

use crate::{
    brush,
    render::{
        BatchBounds, ComputedPenInput, InputSampler, MAX_SAMPLES_PER_INPUT_BATCH, OutputSamples,
        PenInputBatch,
        graph::{BRUSH_SAMPLE_BUILTIN, INITIAL_PEN_INPUT_BUILTIN},
    },
};

pub struct PreparedInputSamplingPipelineData {
    bind_group: BindGroup,
}

pub struct BrushInputSamplingPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl BrushInputSamplingPipeline {
    pub fn new(
        device: &Device,
        resource_layout: &BindGroupLayout,
        compiled_shader: Cow<'_, str>,
    ) -> Self {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("brush input sampling layout"),
            entries: BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::storage_buffer_read_only::<PenInputBatch>(false),
                    binding_types::storage_buffer::<InputSampler>(false),
                    binding_types::storage_buffer::<OutputSamples>(false),
                    binding_types::storage_buffer::<ComputedPenInput>(false),
                ),
            )
            .as_ref(),
        });
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("brush input sampling shader"),
            source: ShaderSource::Wgsl(compiled_shader),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("brush input sampling pipeline layout"),
            bind_group_layouts: &[Some(&layout), Some(resource_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("brush input sampling pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { layout, pipeline }
    }

    #[must_use]
    pub fn prepare(
        &self,
        device: &Device,
        input_batch: &StorageBuffer<PenInputBatch>,
        input_sampler: &StorageBuffer<InputSampler>,
        output_samples: &StorageBuffer<OutputSamples>,
        initial_pen_input: &StorageBuffer<ComputedPenInput>,
    ) -> PreparedInputSamplingPipelineData {
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("brush input sampling bind group"),
            layout: &self.layout,
            entries: BindGroupEntries::sequential((
                input_batch.binding().unwrap(),
                input_sampler.binding().unwrap(),
                output_samples.binding().unwrap(),
                initial_pen_input.binding().unwrap(),
            ))
            .as_ref(),
        });
        PreparedInputSamplingPipelineData { bind_group }
    }

    pub fn dispatch(
        &self,
        pass: &mut ComputePass,
        data: &PreparedInputSamplingPipelineData,
        resource_group: &BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &data.bind_group, &[]);
        pass.set_bind_group(1, resource_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

pub struct BrushMainPipeline {
    eval: ComputePipeline,
    main: ComputePipeline,
    batch_layout: BindGroupLayout,
    resource_layout: BindGroupLayout,
    output_layout: BindGroupLayout,
    builtin_types: BTreeMap<String, Arc<dyn ErasedGraphValueType>>,
    inputs: Vec<(String, GraphShaderLiteral)>,
    output_type: LayerType,
}

impl BrushMainPipeline {
    pub fn new(
        graph: &Graph,
        output: &str,
        output_type: LayerType,
        mut builtin_types: BTreeMap<String, Arc<dyn ErasedGraphValueType>>,
        inputs: Vec<(String, GraphShaderLiteral)>,
        device: &Device,
    ) -> Result<Self> {
        builtin_types.remove(BRUSH_SAMPLE_BUILTIN);
        builtin_types.remove(INITIAL_PEN_INPUT_BUILTIN);

        let mut declarations = String::new();
        let mut helpers = String::new();
        let mut layouts = Vec::new();
        let mut binding = 0;
        for (name, ty) in builtin_types
            .iter()
            .map(|(name, ty)| (name.as_str(), ty.as_ref()))
            .chain(
                inputs
                    .iter()
                    .map(|(name, literal)| (name.as_str(), literal.ty().as_ref())),
            )
        {
            let (next, entries, shader) = ty.push_shader_layout(
                name,
                GraphShaderStage::Input,
                1,
                binding,
                DynamicBindGroupLayoutEntries::new(ShaderStages::COMPUTE),
                declarations,
            )?;
            binding = next;
            layouts.extend(entries.to_vec());
            declarations = shader;
            if let Some(body) = ty.generate_extra_shader_body(GraphShaderStage::Input, name) {
                helpers.push_str(&body);
            }
        }
        let (_, _, code, extra_bodies) =
            graph.compile(Vec::new(), GraphVarIdentGenerator::default())?;
        let bounds = Ident::new(layer_bounds_ident(output));
        let bounds_decl = quote_declaration! { var<private> #bounds: vec4i; }.to_string();
        let reset = quote_statement! { #bounds = vec4i(0); }.to_string();
        let save = format!(
            "{}\n{}",
            quote_statement! { batch_bounds.entries[index].bounds = #bounds; },
            quote_statement! { accumulated_bounds = union_bounds(accumulated_bounds, #bounds); }
        );
        let dispatch = format!(
            "{}\n{}\n{}",
            quote_statement! { let bounds = batch_bounds.entries[sample_index].bounds; },
            quote_statement! { let dispatch_index = bounds.xy + vec2i(id.xy); },
            quote_statement! { if any(dispatch_index >= bounds.zw) { return; } }
        );
        let template = include_str!("brush_main.wesl")
            .replace("//CODEGEN_RESOURCES", &declarations)
            .replace("//CODEGEN_BOUNDS", &bounds_decl)
            .replace("//CODEGEN_GRAPH", &code)
            .replace("//CODEGEN_RESET_BOUNDS", &reset)
            .replace("//CODEGEN_SAVE_BOUNDS", &save)
            .replace("//CODEGEN_DISPATCH", &dispatch);

        let (_, output_entries, output_declarations) = GraphValueType::push_shader_layout(
            &output_type,
            output,
            GraphShaderStage::Main,
            2,
            0,
            DynamicBindGroupLayoutEntries::new(ShaderStages::COMPUTE),
            String::new(),
        )?;
        let output_helpers = GraphValueType::generate_extra_shader_body(
            &output_type,
            GraphShaderStage::Main,
            output,
        )
        .unwrap_or_default();
        let extra_bodies = extra_bodies.into_iter().collect::<BTreeMap<_, _>>();

        let compile = |is_eval| -> Result<String> {
            let mut shader = template
                .replace(
                    "//CODEGEN_OUTPUT",
                    if is_eval { "" } else { &output_declarations },
                )
                .replace("//CODEGEN_HELPERS", &format!("{helpers}\n{output_helpers}"));
            for body in extra_bodies.values() {
                body.inject_body(&mut shader);
            }
            wesl_jit::compile_wesl_with_config(
                shader,
                &[&image::PACKAGE, &render::PACKAGE, &brush::PACKAGE],
                |compiler| {
                    compiler.set_feature("EVAL", is_eval);
                },
            )
        };

        let batch_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("brush main batch"),
            entries: BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::storage_buffer_read_only::<OutputSamples>(false),
                    binding_types::storage_buffer_read_only::<ComputedPenInput>(false),
                    binding_types::storage_buffer::<BatchBounds>(false),
                    binding_types::uniform_buffer::<u32>(true),
                ),
            )
            .as_ref(),
        });
        let resource_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("brush main resources"),
            entries: &layouts,
        });
        let output_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("brush main output"),
            entries: &output_entries,
        });

        let make_pipeline = |is_eval| -> Result<ComputePipeline> {
            let module = device.create_shader_module(ShaderModuleDescriptor {
                label: Some("brush main shader"),
                source: ShaderSource::Wgsl(compile(is_eval)?.into()),
            });
            let layouts = if is_eval {
                vec![Some(&batch_layout), Some(&resource_layout)]
            } else {
                vec![
                    Some(&batch_layout),
                    Some(&resource_layout),
                    Some(&output_layout),
                ]
            };
            let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("brush main pipeline"),
                bind_group_layouts: &layouts,
                immediate_size: 0,
            });
            Ok(device.create_compute_pipeline(&ComputePipelineDescriptor {
                label: Some("brush main pipeline"),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(if is_eval { "eval" } else { "main" }),
                compilation_options: Default::default(),
                cache: None,
            }))
        };

        Ok(Self {
            eval: make_pipeline(true)?,
            main: make_pipeline(false)?,
            batch_layout,
            resource_layout,
            output_layout,
            builtin_types,
            inputs,
            output_type,
        })
    }

    pub fn prepare_batch_group(
        &self,
        device: &Device,
        queue: &Queue,
        samples: &Buffer,
        initial: &Buffer,
        bounds: &Buffer,
    ) -> (BindGroup, Vec<u32>) {
        let mut sample_index_buffer = DynamicUniformBuffer::new_with_alignment(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        ));
        sample_index_buffer.set_label(Some("sample index buffer"));
        let mut sample_index_offsets = Vec::with_capacity(MAX_SAMPLES_PER_INPUT_BATCH as usize);
        for index in 0..MAX_SAMPLES_PER_INPUT_BATCH {
            let offset = sample_index_buffer.push(&index);
            sample_index_offsets.push(offset);
        }
        sample_index_buffer.write_buffer(device, queue);

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("brush main batch"),
            layout: &self.batch_layout,
            entries: BindGroupEntries::sequential((
                samples.as_entire_binding(),
                initial.as_entire_binding(),
                bounds.as_entire_binding(),
                sample_index_buffer.binding().unwrap(),
            ))
            .as_ref(),
        });

        (bind_group, sample_index_offsets)
    }

    pub fn prepare_resource_group(
        &self,
        device: &Device,
        builtins: &HashMap<String, GraphShaderLiteral>,
    ) -> Result<BindGroup> {
        let mut binding = 0;
        let mut entries = Vec::new();
        for value in self
            .builtin_types
            .iter()
            .map(|(name, ty)| -> Result<_> {
                Ok((
                    ty.as_ref(),
                    builtins
                        .get(name)
                        .with_context(|| format!("Missing brush builtin {name}"))?,
                ))
            })
            .chain(
                self.inputs
                    .iter()
                    .map(|(_, literal)| Ok((literal.ty().as_ref(), literal))),
            )
        {
            let (ty, literal) = value?;
            let (next, bound) = ty.push_shader_binding(
                GraphShaderStage::Input,
                literal.value(),
                binding,
                DynamicBindGroupEntries::new(),
            )?;
            binding = next;
            entries.extend(bound.to_vec());
        }
        Ok(device.create_bind_group(&BindGroupDescriptor {
            label: Some("brush main resources"),
            layout: &self.resource_layout,
            entries: &entries,
        }))
    }

    pub fn prepare_output(&self, device: &Device, queue: &Queue) -> Result<PreparedLayer> {
        GraphValueType::prepare_to_shader(&self.output_type, &LayerReference, device, queue)
    }

    pub fn prepare_output_group(
        &self,
        device: &Device,
        layer: &PreparedLayer,
    ) -> Result<BindGroup> {
        let (_, entries) = GraphValueType::push_shader_binding(
            &self.output_type,
            GraphShaderStage::Main,
            layer,
            0,
            DynamicBindGroupEntries::new(),
        )?;
        Ok(device.create_bind_group(&BindGroupDescriptor {
            label: Some("brush main output"),
            layout: &self.output_layout,
            entries: &entries.to_vec(),
        }))
    }

    pub fn dispatch_eval(
        &self,
        encoder: &mut CommandEncoder,
        batch: &BindGroup,
        resources: &BindGroup,
    ) {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.eval);
        pass.set_bind_group(0, batch, &[0]);
        pass.set_bind_group(1, resources, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    pub fn dispatch_main(
        &self,
        encoder: &mut CommandEncoder,
        batch: &BindGroup,
        resources: &BindGroup,
        output: &BindGroup,
        sample_index_offset: u32,
        bounds: IRect,
    ) -> Result<()> {
        let size = bounds.size().as_uvec2();
        let size = UVec2::new(size.x.div_ceil(16), size.y.div_ceil(16));

        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.main);
        pass.set_bind_group(0, batch, &[sample_index_offset]);
        pass.set_bind_group(1, resources, &[]);
        pass.set_bind_group(2, output, &[]);
        pass.dispatch_workgroups(size.x, size.y, 1);
        Ok(())
    }
}
