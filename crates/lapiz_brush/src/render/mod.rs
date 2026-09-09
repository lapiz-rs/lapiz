use std::{num::NonZeroU64, sync::Arc};

use anyhow::Result;
use bevy_math::IRect;
use chrono::{DateTime, Utc};
use encase::ShaderType;
use glam::{IVec2, Vec2, Vec4};
use iced_runtime::Task;
use indexmap::IndexSet;
use lapiz_assets::{AssetAppExt, store::AssetRegistry};
use lapiz_canvas::{CanvasAppExt, CanvasId};
use lapiz_color::ForegroundBackgroundColorExt;
use lapiz_image::{
    composite::PixelPreviewOverrider,
    layer::{LayerId, properties::LayerTexelTypeProp},
    scan_pixels::ScanPixelsPipeline,
    texel::TexelType,
    tile::{DynamicLayerStorage, GpuLayerInfo, GpuTileStorage, LayerBinding, TileStorageAppExt},
};
use lapiz_input::mouse::PressedMouseState;
use lapiz_render::{
    buffer::{BufferVec, DynamicBuffer},
    readback::{
        AsyncBufferReadback, create_readback_buffer_and_schedule_copy_buffer,
        readback_buffer_on_submit_async,
    },
    render_context::RenderContextAppExt,
    texture::GpuImage,
    texture_atlas::{TextureAtlas, TextureAtlasBuilder},
};
use lapiz_runtime::Services;
use lapiz_shader_graph::graph::external::GraphExternalVariableStorage;
use parking_lot::Mutex;
use wgpu::{
    BindGroupEntry, BindGroupLayoutEntry, BindingResource, BindingType, Buffer, BufferBindingType,
    BufferDescriptor, BufferUsages, ComputePassDescriptor, Device, Extent3d, Queue, ShaderStages,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};

use crate::{
    input_processing::{InputProcessor, RawPenInput},
    instance::{BrushPresetInstance, CompiledBrushPreset},
    render::{
        graph::CanvasResources,
        pipeline::{
            BrushInputSamplingPipeline, BrushMainBoundsEvalPipeline, BrushMainPipeline,
            BrushPostProcessBoundsEvalPipeline, BrushPostProcessPipeline,
            PreparedBrushMainBoundsEvalPipelineData, PreparedBrushMainPipelineData,
            PreparedBrushPostProcessBoundsEvalPipelineData, PreparedInputSamplingPipelineData,
        },
    },
};

pub mod graph;
pub mod pipeline;
pub mod stroke_preview;

const EXTERNAL_VARIABLE_BASE_BINDING: u32 = 32;
pub const MAX_DABS_PER_STROKE: u32 = 256;

pub struct CanvasBrushStrokeSessionInfo {
    pub stroke_id: u64,
    pub stroke_begin: DateTime<Utc>,
    pub canvas_id: CanvasId,
    pub target_layer_id: LayerId,
    pub selection_layer_id: LayerId,
    pub target_layer_format: TexelType,
    pub selection_layer_format: TexelType,
}

pub struct CanvasBrushPresetOperator {
    instance: BrushPresetInstance,
    device: Device,
    queue: Queue,
    renderer: Option<BrushPresetRenderer>,
    session: Option<CanvasBrushStrokeSessionInfo>,
    input_processor: InputProcessor,
    cached_brush: Option<CompiledBrushPreset>,
    canvas_resources: DynamicBuffer<CanvasResources>,
}

impl CanvasBrushPresetOperator {
    pub fn new(
        instance: BrushPresetInstance,
        device: Device,
        queue: Queue,
        input_processor: InputProcessor,
    ) -> Self {
        Self {
            instance,
            renderer: None,
            device,
            queue,
            session: None,
            input_processor,
            cached_brush: None,
            canvas_resources: DynamicBuffer::new(
                Some("canvas_resources".into()),
                BufferUsages::STORAGE,
            ),
        }
    }

    pub fn instance(&self) -> &BrushPresetInstance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut BrushPresetInstance {
        self.cached_brush = None;
        &mut self.instance
    }

    pub fn begin_stroke(
        &mut self,
        input: &PressedMouseState,
        stroke_id: u64,
        canvas_id: CanvasId,
        services: &mut Services,
    ) -> Task<()> {
        let canvas = services
            .canvas(&canvas_id)
            .expect("Current canvas should exist");
        let position = canvas
            .transform
            .window_to_pixel(Vec2::new(input.position.x, input.position.y));
        let active_layer_id = canvas.active_layer_id();
        let selection_layer_id = canvas.image.selection_layer();
        if !canvas
            .active_layer_node()
            .properties()
            .contains::<LayerTexelTypeProp>()
        {
            log::warn!("Unable to paint to the active layer which cannot contain pixels.");
            return Task::none();
        }

        // update canvas resources
        let xyz_to_rgb = canvas
            .image
            .profile()
            .rgb_to_xyz_matrix()
            .to_f32()
            .inverse();
        let fg_color = services.foreground_color().get().into_rgb(xyz_to_rgb);
        let bg_color = services.background_color().get().into_rgb(xyz_to_rgb);
        self.canvas_resources.clear();
        self.canvas_resources.push(&CanvasResources {
            foreground_color: Vec4::new(fg_color.r, fg_color.g, fg_color.b, 1.0),
            background_color: Vec4::new(bg_color.r, bg_color.g, bg_color.b, 1.0),
        });
        self.canvas_resources
            .write_buffer(&self.device, &self.queue);

        let tiles = services.tile_storage();
        let target_layer_info = tiles
            .get_layer_info(active_layer_id)
            .expect("Active pixel layer should have GPU storage");
        let selection_layer_info = tiles
            .get_layer_info(selection_layer_id)
            .expect("Selection layer should have GPU storage");
        let session = CanvasBrushStrokeSessionInfo {
            stroke_id,
            stroke_begin: Utc::now(),
            canvas_id,
            target_layer_id: active_layer_id,
            selection_layer_id,
            target_layer_format: target_layer_info.texel_type,
            selection_layer_format: selection_layer_info.texel_type,
        };
        if let Some(last_session) = self.session.as_ref()
            && (last_session.target_layer_format != session.target_layer_format
                || last_session.selection_layer_format != session.selection_layer_format)
        {
            self.renderer = None;
        }

        let compiled_brush = self.cached_brush.get_or_insert_with(|| {
            self.instance
                .compile(EXTERNAL_VARIABLE_BASE_BINDING)
                .expect("Failed to compile brush preset")
        });
        println!("Compiled brush:\n{}", compiled_brush);
        let renderer = self.renderer.get_or_insert_with(|| {
            BrushPresetRenderer::new(
                compiled_brush,
                session.target_layer_format,
                session.selection_layer_format,
                services,
                &self.canvas_resources,
            )
        });

        self.input_processor.reset();
        let tiles = services.tile_storage();
        let target_layer = tiles
            .get_layer_binding_or_empty(session.target_layer_id)
            .expect("Failed to bind active pixel layer");
        let selection_layer = tiles
            .get_layer_binding_or_empty(session.selection_layer_id)
            .expect("Failed to bind selection layer");
        renderer.begin(&self.device, &self.queue, target_layer, selection_layer);

        let sample =
            self.input_processor
                .push(RawPenInput::new(position, session.stroke_begin, input));
        let task = sample.map(|sample| renderer.update(&self.device, &self.queue, sample));

        self.session = Some(session);
        task.unwrap_or_else(Task::none).discard()
    }

    pub fn update_stroke(&mut self, input: &PressedMouseState, services: &Services) -> Task<()> {
        let Some(renderer) = &mut self.renderer else {
            return Task::none();
        };
        let Some(session) = self.session.as_ref() else {
            return Task::none();
        };
        let canvas = services.canvas(&session.canvas_id).unwrap();
        let position = canvas
            .transform
            .window_to_pixel(Vec2::new(input.position.x, input.position.y));

        let Some(sample) =
            self.input_processor
                .push(RawPenInput::new(position, session.stroke_begin, input))
        else {
            return Task::none();
        };

        renderer.update(&self.device, &self.queue, sample).discard()
    }

    pub fn end_stroke(
        &mut self,
        input: &PressedMouseState,
        services: &mut Services,
    ) -> Task<BrushStrokeResult> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Task::none();
        };
        let Some(session) = self.session.take() else {
            return Task::none();
        };
        let canvas = services
            .canvas(&session.canvas_id)
            .expect("Stroke canvas should exist");
        let position = canvas
            .transform
            .window_to_pixel(Vec2::new(input.position.x, input.position.y));

        let mut updates = Vec::new();
        for sample in
            self.input_processor
                .flush(RawPenInput::new(position, session.stroke_begin, input))
        {
            updates.push(renderer.update(&self.device, &self.queue, sample));
        }
        let end_task = renderer.end(&self.device, &self.queue);

        let updates = Task::batch(updates).discard();

        let end = end_task.map({
            move |result| BrushStrokeResult {
                stroke_id: session.stroke_id,
                canvas_id: session.canvas_id,
                target_layer_id: session.target_layer_id,
                result,
            }
        });

        updates.chain(end)
    }

    pub fn preview(&mut self) -> Task<Option<BrushStrokePreview>> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Task::done(None);
        };

        let Some(session) = self.session.as_ref() else {
            return Task::done(None);
        };

        let stroke_id = session.stroke_id;
        let canvas_id = session.canvas_id;
        let target_layer_id = session.target_layer_id;

        renderer
            .generate_preview(&self.device, &self.queue)
            .map(move |result| {
                result.and_then(|result| {
                    Some(BrushStrokePreview {
                        stroke_id,
                        canvas_id,
                        target_layer_id,
                        overrider: PixelPreviewOverrider {
                            texture: result.texture_view()?.clone(),
                            tile_info_buffer: result.tile_info_buffer()?.clone(),
                        },
                        dirty_tiles: result.compute_tile_bounds(),
                    })
                })
            })
    }
}

#[derive(Clone)]
struct StrokePostprocessPipelines {
    main: BrushPostProcessPipeline,
    bounds_eval: BrushPostProcessBoundsEvalPipeline,
}

struct StrokeSession {
    shared: Arc<Mutex<SharedBrushRendererMainPassState>>,
    stroke_pp_cache: Arc<futures::lock::Mutex<StrokePostprocessCache>>,

    pen_input: DynamicBuffer<PenInput>,
    output_samples_packed: DynamicBuffer<OutputSamples>,
    dab_infos_packed: BufferVec<DabInfo>,
    main_bounds_eval_dispatch: Buffer,

    input_sample_prepared: PreparedInputSamplingPipelineData,
    main_bounds_eval_prepared: PreparedBrushMainBoundsEvalPipelineData,
}

pub struct BrushStrokePreview {
    pub stroke_id: u64,
    pub canvas_id: CanvasId,
    pub target_layer_id: LayerId,
    pub overrider: PixelPreviewOverrider,
    pub dirty_tiles: IRect,
}

pub struct BrushStrokeResult {
    pub stroke_id: u64,
    pub canvas_id: CanvasId,
    pub target_layer_id: LayerId,
    pub result: DynamicLayerStorage,
}

pub struct BrushPresetRenderer {
    input_sample: BrushInputSamplingPipeline,
    main: BrushMainPipeline,
    main_bounds_eval: BrushMainBoundsEvalPipeline,
    resources: StrokeResources,
    scan_pixels: ScanPixelsPipeline,
    stroke_pp_pipelines: Arc<[StrokePostprocessPipelines]>,

    input_sampler_buffer: DynamicBuffer<InputSampler>,
    session: Option<StrokeSession>,
}

impl BrushPresetRenderer {
    #[tracing::instrument(skip_all, name = "new_renderer")]
    pub fn new(
        brush: &CompiledBrushPreset,
        target_layer_format: TexelType,
        selection_layer_format: TexelType,
        services: &Services,
        canvas_resources: &DynamicBuffer<CanvasResources>,
    ) -> Self {
        let device = services.render_device();
        let queue = services.render_queue();
        let assets = services.assets();

        let resources = StrokeResources::new(
            device,
            queue,
            brush,
            target_layer_format,
            selection_layer_format,
            assets,
            canvas_resources,
        );
        let scan_pixels = ScanPixelsPipeline::new(device, selection_layer_format);

        let input_sample = BrushInputSamplingPipeline::new(
            device,
            &resources,
            brush.input_sampling.clone().into(),
        );

        let main = BrushMainPipeline::new(device, &resources, brush.main_graph.main.clone().into());
        let main_bounds_eval = BrushMainBoundsEvalPipeline::new(
            device,
            &resources,
            brush.main_graph.bounds_eval.clone().into(),
        );

        let mut stroke_pp = Vec::new();
        for graph in &brush.stroke_postprocess_graphs {
            let main = BrushPostProcessPipeline::new(device, &resources, graph.main.clone().into());
            let bounds_eval = BrushPostProcessBoundsEvalPipeline::new(
                device,
                &resources,
                graph.bounds_eval.clone().into(),
            );
            stroke_pp.push(StrokePostprocessPipelines { main, bounds_eval });
        }

        let mut input_sampler_buffer =
            DynamicBuffer::new(Some("input sampler buffer".into()), BufferUsages::STORAGE);
        input_sampler_buffer.push(&InputSampler::default());
        input_sampler_buffer.write_buffer(device, queue);

        Self {
            input_sample,
            main,
            main_bounds_eval,
            resources,
            scan_pixels,

            input_sampler_buffer,
            stroke_pp_pipelines: stroke_pp.into(),
            session: None,
        }
    }

    pub fn begin(
        &mut self,
        device: &Device,
        queue: &Queue,
        target_layer: LayerBinding,
        selection_layer: LayerBinding,
    ) {
        self.input_sampler_buffer.clear();
        self.input_sampler_buffer.push(&InputSampler::default());
        self.input_sampler_buffer.write_buffer(device, queue);

        let mut initial_pen_input = DynamicBuffer::new(
            Some("initial pen input buffer".into()),
            BufferUsages::STORAGE,
        );
        initial_pen_input.push(&ComputedPenInput::default());
        initial_pen_input.write_buffer(device, queue);

        let has_selection = self
            .scan_pixels
            .scan_to_binary_buffer(device, queue, &selection_layer);

        let mut pen_input_buffer =
            DynamicBuffer::new(Some("pen input buffer".into()), BufferUsages::STORAGE);
        pen_input_buffer.push(&PenInput::default());
        pen_input_buffer.write_buffer(device, queue);

        let main_bounds_eval_dispatch = device.create_buffer(&BufferDescriptor {
            label: Some("bounds eval dispatch"),
            size: std::mem::size_of::<u32>() as u64 * 4,
            usage: BufferUsages::STORAGE | BufferUsages::INDIRECT,
            mapped_at_creation: false,
        });
        let mut output_samples_packed = DynamicBuffer::new(
            Some("output samples buffer".into()),
            BufferUsages::COPY_SRC | BufferUsages::STORAGE,
        );
        // TODO Use uninit buffer
        output_samples_packed.push(&OutputSamples::new(MAX_DABS_PER_STROKE));
        output_samples_packed.write_buffer(device, queue);

        let mut dab_infos_packed = BufferVec::new(
            Some("dab info buffer".into()),
            BufferUsages::COPY_SRC | BufferUsages::STORAGE,
        );
        // TODO Use uninit buffer
        for _ in 0..MAX_DABS_PER_STROKE {
            dab_infos_packed.push(&DabInfo::default());
        }
        dab_infos_packed.write_buffer(device, queue);

        let mut output_samples_aligned =
            DynamicBuffer::new(Some("samples buffer".into()), BufferUsages::STORAGE);
        let mut samples_offsets = Vec::new();
        for _ in 0..MAX_DABS_PER_STROKE {
            samples_offsets.push(output_samples_aligned.push(&ComputedPenInput::default()) as u32);
        }
        output_samples_aligned.write_buffer(device, queue);

        let mut dab_infos_aligned =
            DynamicBuffer::new(Some("dab infos buffer".into()), BufferUsages::STORAGE);
        let mut dab_info_offsets = Vec::new();
        for _ in 0..MAX_DABS_PER_STROKE {
            dab_info_offsets.push(dab_infos_aligned.push(&DabInfo::default()) as u32);
        }
        dab_infos_aligned.write_buffer(device, queue);

        let intermediate_buffers = [
            DynamicLayerStorage::new(
                device.clone(),
                queue.clone(),
                GpuLayerInfo {
                    texel_type: self.resources.target_layer_format,
                },
            ),
            DynamicLayerStorage::new(
                device.clone(),
                queue.clone(),
                GpuLayerInfo {
                    texel_type: self.resources.target_layer_format,
                },
            ),
        ];

        let main_prepared = self.main.prepare(
            device,
            &target_layer,
            &has_selection,
            &selection_layer,
            &output_samples_aligned,
            &dab_infos_aligned,
            &self.resources,
            initial_pen_input.inner_buffer().unwrap(),
            &[
                intermediate_buffers[0].binding_or_empty(),
                intermediate_buffers[1].binding_or_empty(),
            ],
        );

        let input_sample_prepared = self.input_sample.prepare(
            device,
            &pen_input_buffer,
            &self.input_sampler_buffer,
            &output_samples_packed,
            &main_bounds_eval_dispatch,
            &self.resources,
            &initial_pen_input,
        );

        let main_bounds_eval_prepared = self.main_bounds_eval.prepare(
            device,
            &output_samples_packed,
            &dab_infos_packed,
            &target_layer,
            &has_selection,
            &selection_layer,
            &initial_pen_input,
            &self.resources,
        );

        let shared = SharedBrushRendererMainPassState {
            device: device.clone(),
            queue: queue.clone(),
            main: self.main.clone(),

            intermediate_buffers,
            round: 0,
            accumulated_tile_bounds: IRect::EMPTY,

            main_prepared,
            samples_offsets,
            dab_info_offsets,
            target_layer: target_layer.clone(),
            has_selection: has_selection.clone(),
            selection_layer: selection_layer.clone(),
            output_samples_aligned,
            dab_infos_aligned,
            resources: self.resources.clone(),
            initial_pen_input: initial_pen_input.inner_buffer().unwrap().clone(),
        };

        let mut stroke_pp_pipeline_cache = Vec::with_capacity(self.stroke_pp_pipelines.len());
        for pipeline in self.stroke_pp_pipelines.iter() {
            let mut dab_info_buffer = DynamicBuffer::new(
                Some("brush stroke pp dab info".into()),
                BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            );
            dab_info_buffer.push(&DabInfo::default());
            dab_info_buffer.write_buffer(device, queue);
            let mut stroke_pp_data =
                DynamicBuffer::new(Some("brush stroke pp data".into()), BufferUsages::STORAGE);
            stroke_pp_data.push(&StrokePostprocessData::default());
            stroke_pp_data.write_buffer(device, queue);

            let data = pipeline.bounds_eval.prepare(
                device,
                &stroke_pp_data,
                &target_layer,
                &has_selection,
                &selection_layer,
                &dab_info_buffer,
                &self.resources,
            );
            stroke_pp_pipeline_cache.push(StrokePostprocessPipelineCache {
                pipeline: pipeline.clone(),
                prepared_bounds_eval: data,
                dab_info_buffer,
                stroke_pp_data,
            });
        }

        let stroke_pp_cache = StrokePostprocessCache {
            resources: self.resources.clone(),
            target_layer,
            has_selection,
            selection_layer,
            pipeline_cache: stroke_pp_pipeline_cache,
        };

        self.session = Some(StrokeSession {
            shared: Arc::new(Mutex::new(shared)),
            stroke_pp_cache: Arc::new(futures::lock::Mutex::new(stroke_pp_cache)),

            pen_input: pen_input_buffer,
            output_samples_packed,
            dab_infos_packed,

            main_bounds_eval_dispatch,

            input_sample_prepared,
            main_bounds_eval_prepared,
        });
    }

    pub fn end(&mut self, device: &Device, queue: &Queue) -> Task<DynamicLayerStorage> {
        let Some(session) = self.session.take() else {
            return Task::none();
        };

        let device = device.clone();
        let queue = queue.clone();
        let stroke_pp_cache = session.stroke_pp_cache.clone();

        Task::future(async move {
            brush_renderer_worker_stroke_postprocess(session.shared, stroke_pp_cache, device, queue)
                .await
        })
    }

    // TODO: Copy unchanged tiles onto another buffer?
    pub fn update(
        &mut self,
        device: &Device,
        queue: &Queue,
        pen_input: PenInput,
    ) -> Task<Result<()>> {
        let Some(session) = &mut self.session else {
            return Task::none();
        };

        self.resources.update_external_var_buffers(queue);

        session.pen_input.clear();
        session.pen_input.push(&pen_input);
        session.pen_input.write_buffer(device, queue);

        let mut ec = device.create_command_encoder(&Default::default());

        ec.push_debug_group("brush preset update stroke");
        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("brush preset update pass"),
                ..Default::default()
            });

            self.input_sample
                .dispatch(&mut pass, &session.input_sample_prepared);
            self.main_bounds_eval.dispatch(
                &mut pass,
                &session.main_bounds_eval_prepared,
                &session.main_bounds_eval_dispatch,
            );
        }
        ec.pop_debug_group();

        let output_samples_staging = create_readback_buffer_and_schedule_copy_buffer(
            device,
            &mut ec,
            session.output_samples_packed.inner_buffer().unwrap(),
        );
        let dab_info_staging = create_readback_buffer_and_schedule_copy_buffer(
            device,
            &mut ec,
            session.dab_infos_packed.inner_buffer().unwrap(),
        );
        let samples_readback =
            readback_buffer_on_submit_async(&mut ec, &output_samples_staging, ..);
        let dab_info_readback = readback_buffer_on_submit_async(&mut ec, &dab_info_staging, ..);

        // unsafe {
        //     device.start_graphics_debugger_capture();
        // }
        queue.submit([ec.finish()]);
        // unsafe {
        //     device.stop_graphics_debugger_capture();
        // }

        let shared = session.shared.clone();

        Task::future(async move {
            brush_renderer_worker_main(shared, samples_readback, dab_info_readback).await
        })
    }

    pub fn generate_preview(
        &mut self,
        device: &Device,
        queue: &Queue,
    ) -> Task<Option<DynamicLayerStorage>> {
        let Some(session) = self.session.as_ref() else {
            return Task::done(None);
        };

        let device = device.clone();
        let queue = queue.clone();
        let stroke_pp_cache = session.stroke_pp_cache.clone();

        let shared = session.shared.clone();

        Task::future(async move {
            let (mut intermediate_buffers, mut round, mut accumulated_tile_bounds) = {
                let shared = shared.lock();

                (
                    if shared.round % 2 == 0 {
                        [
                            shared.intermediate_buffers[0].deep_clone(),
                            shared.intermediate_buffers[1].create_allocated_empty_sibling(),
                        ]
                    } else {
                        [
                            shared.intermediate_buffers[0].create_allocated_empty_sibling(),
                            shared.intermediate_buffers[1].deep_clone(),
                        ]
                    },
                    shared.round,
                    shared.accumulated_tile_bounds,
                )
            };

            if accumulated_tile_bounds.is_empty() {
                return None;
            }

            let mut cache = stroke_pp_cache.lock().await;

            // unsafe {
            //     device.start_graphics_debugger_capture();
            // }
            postprocess_stroke(
                &device,
                &queue,
                Time::default(),
                &mut intermediate_buffers,
                &mut round,
                &mut accumulated_tile_bounds,
                &mut cache,
            )
            .await;
            // unsafe {
            //     device.stop_graphics_debugger_capture();
            // }

            let [buffer_a, buffer_b] = intermediate_buffers;
            Some(if round % 2 == 0 { buffer_a } else { buffer_b })
        })
    }
}

struct SharedBrushRendererMainPassState {
    device: Device,
    queue: Queue,

    main: BrushMainPipeline,
    target_layer: LayerBinding,
    has_selection: Buffer,
    selection_layer: LayerBinding,
    output_samples_aligned: DynamicBuffer<ComputedPenInput>,
    dab_infos_aligned: DynamicBuffer<DabInfo>,
    resources: StrokeResources,
    initial_pen_input: Buffer,

    intermediate_buffers: [DynamicLayerStorage; 2],
    round: u32,
    accumulated_tile_bounds: IRect,

    main_prepared: PreparedBrushMainPipelineData,
    samples_offsets: Vec<u32>,
    dab_info_offsets: Vec<u32>,
}

async fn brush_renderer_worker_main(
    shared: Arc<Mutex<SharedBrushRendererMainPassState>>,
    samples: AsyncBufferReadback<OutputSamples>,
    dab_infos: AsyncBufferReadback<Vec<DabInfo>>,
) -> Result<()> {
    let samples = samples.into_inner().await??;
    let dab_infos = dab_infos.into_inner().await??;

    {
        let mut shared = shared.lock();
        let SharedBrushRendererMainPassState {
            device,
            queue,
            main,
            target_layer,
            has_selection,
            selection_layer,
            output_samples_aligned,
            dab_infos_aligned,
            resources,
            initial_pen_input,
            intermediate_buffers,
            round,
            accumulated_tile_bounds,
            main_prepared,
            samples_offsets,
            dab_info_offsets,
        } = &mut *shared;

        let dispatch_span = tracing::info_span!("main_dispatch");
        let _span = dispatch_span.enter();

        output_samples_aligned.clear();
        dab_infos_aligned.clear();

        let old_generation = intermediate_buffers[0].allocation_generation();

        let mut tiles_to_allocate = IndexSet::new();
        for (sample, dab_info) in samples
            .samples
            .into_iter()
            .take(samples.n_samples as usize)
            .zip(dab_infos)
        {
            output_samples_aligned.push(&sample);
            dab_infos_aligned.push(&dab_info);

            let rect = IRect {
                min: dab_info.bound_min,
                max: dab_info.bound_max,
            };
            tiles_to_allocate.extend(
                (rect.min.y..rect.max.y)
                    .flat_map(|y| (rect.min.x..rect.max.x).map(move |x| IVec2::new(x, y))),
            );
            *accumulated_tile_bounds = accumulated_tile_bounds.union(rect);
        }

        for b in intermediate_buffers.as_mut() {
            b.allocate_tiles_batch(&tiles_to_allocate);
        }

        let new_generation = intermediate_buffers[0].allocation_generation();
        let _new_tiles = intermediate_buffers[0].len();
        if old_generation != new_generation {
            *main_prepared = main.prepare(
                device,
                target_layer,
                has_selection,
                selection_layer,
                output_samples_aligned,
                dab_infos_aligned,
                resources,
                initial_pen_input,
                &[
                    intermediate_buffers[0].binding_or_empty(),
                    intermediate_buffers[1].binding_or_empty(),
                ],
            )
        }

        if accumulated_tile_bounds.is_empty() {
            return Err(anyhow::anyhow!("accumulated_tile_bounds is empty"));
        }

        output_samples_aligned.write_buffer(device, queue);
        dab_infos_aligned.write_buffer(device, queue);

        let mut ec = device.create_command_encoder(&Default::default());

        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("brush main pass"),
                ..Default::default()
            });
            main.dispatch(
                &mut pass,
                main_prepared,
                samples_offsets,
                dab_info_offsets,
                round,
                samples.n_samples,
            );
        }

        queue.submit([ec.finish()]);
    }

    Ok(())
}

pub struct StrokePostprocessPipelineCache {
    pipeline: StrokePostprocessPipelines,
    prepared_bounds_eval: PreparedBrushPostProcessBoundsEvalPipelineData,
    dab_info_buffer: DynamicBuffer<DabInfo>,
    stroke_pp_data: DynamicBuffer<StrokePostprocessData>,
}

pub struct StrokePostprocessCache {
    resources: StrokeResources,
    target_layer: LayerBinding,
    has_selection: Buffer,
    selection_layer: LayerBinding,

    pipeline_cache: Vec<StrokePostprocessPipelineCache>,
}

async fn brush_renderer_worker_stroke_postprocess(
    shared: Arc<Mutex<SharedBrushRendererMainPassState>>,
    stroke_pp_cache: Arc<futures::lock::Mutex<StrokePostprocessCache>>,

    device: Device,
    queue: Queue,
) -> DynamicLayerStorage {
    let (mut intermediate_buffers, mut round, mut accumulated_tile_bounds) = {
        let shared = shared.lock();
        (
            if shared.round.is_multiple_of(2) {
                [
                    shared.intermediate_buffers[0].deep_clone(),
                    shared.intermediate_buffers[1].create_allocated_empty_sibling(),
                ]
            } else {
                [
                    shared.intermediate_buffers[0].create_allocated_empty_sibling(),
                    shared.intermediate_buffers[1].deep_clone(),
                ]
            },
            shared.round,
            shared.accumulated_tile_bounds,
        )
    };
    let mut cache = stroke_pp_cache.lock().await;

    postprocess_stroke(
        &device,
        &queue,
        // TODO
        Time::default(),
        &mut intermediate_buffers,
        &mut round,
        &mut accumulated_tile_bounds,
        &mut cache,
    )
    .await;

    let [buffer_a, buffer_b] = intermediate_buffers;
    if round % 2 == 0 { buffer_a } else { buffer_b }
}

async fn postprocess_stroke(
    device: &Device,
    queue: &Queue,
    time: Time,
    intermediate_buffers: &mut [DynamicLayerStorage; 2],
    round: &mut u32,
    accumulated_tile_bounds: &mut IRect,
    cache: &mut StrokePostprocessCache,
) {
    let StrokePostprocessCache {
        resources,
        target_layer,
        has_selection,
        selection_layer,
        pipeline_cache,
    } = cache;

    if accumulated_tile_bounds.is_empty() {
        return;
    }

    for StrokePostprocessPipelineCache {
        pipeline,
        prepared_bounds_eval,
        dab_info_buffer,
        stroke_pp_data,
    } in pipeline_cache.iter_mut()
    {
        stroke_pp_data.clear();
        stroke_pp_data.push(&StrokePostprocessData {
            accumulated_pixel_bounds: GpuTileStorage::tile_rect_to_pixel(*accumulated_tile_bounds),
            time,
        });
        stroke_pp_data.write_buffer(device, queue);

        let mut ec = device.create_command_encoder(&Default::default());
        ec.push_debug_group("brush preset stroke postprocess");

        {
            let mut pass = ec.begin_compute_pass(&ComputePassDescriptor {
                label: Some("brush preset stroke postprocess pass"),
                ..Default::default()
            });

            pipeline
                .bounds_eval
                .dispatch(&mut pass, prepared_bounds_eval);
        }

        let dab_info_readback_buffer = create_readback_buffer_and_schedule_copy_buffer(
            device,
            &mut ec,
            dab_info_buffer.inner_buffer().unwrap(),
        );
        let dab_info_readback =
            readback_buffer_on_submit_async::<DabInfo, _>(&mut ec, &dab_info_readback_buffer, ..);

        ec.pop_debug_group();
        // unsafe {
        //     device.start_graphics_debugger_capture();
        // }
        queue.submit([ec.finish()]);
        // unsafe {
        //     device.stop_graphics_debugger_capture();
        // }

        let new_dab_info = dab_info_readback.into_inner().await.unwrap().unwrap();
        *accumulated_tile_bounds = IRect {
            min: new_dab_info.bound_min,
            max: new_dab_info.bound_max,
        };

        for b in intermediate_buffers.iter_mut() {
            b.allocate_tiles(IRect {
                min: new_dab_info.bound_min,
                max: new_dab_info.bound_max,
            });
        }

        let intermediate_buffers = [
            intermediate_buffers[0].binding().unwrap(),
            intermediate_buffers[1].binding().unwrap(),
        ];

        let prepared = pipeline.main.prepare(
            device,
            stroke_pp_data,
            target_layer,
            has_selection,
            selection_layer,
            dab_info_buffer,
            resources,
            &intermediate_buffers,
            *round,
        );
        let mut ec = device.create_command_encoder(&Default::default());
        {
            let mut pass = ec.begin_compute_pass(&Default::default());
            pipeline.main.dispatch(&mut pass, &prepared, round);
        }
        queue.submit([ec.finish()]);
    }
}

#[derive(ShaderType, Debug, Clone)]
pub struct OutputSamples {
    pub n_samples: u32,
    pub is_overflow: u32,
    #[shader(size(runtime))]
    pub samples: Vec<ComputedPenInput>,
}

impl OutputSamples {
    pub fn new(max_samples: u32) -> Self {
        Self {
            n_samples: 0,
            is_overflow: 0,
            samples: vec![ComputedPenInput::default(); max_samples as usize],
        }
    }
}

#[derive(ShaderType, Debug, Default, Clone, Copy)]
pub struct InputSampler {
    pub last_input: PenInput,
    pub last_sample: ComputedPenInput,
    pub has_last_sample: u32,
    pub has_initial_input: u32,
    pub next_dab_index: u32,
    pub distance_to_next_dab: f32,
    pub stroke_distance: f32,
}

#[derive(ShaderType, Debug, Default, Clone, Copy)]
pub struct PenInput {
    pub position: Vec2,
    pub tilt: Vec2,
    pub angle: Vec2,
    pub pressure: f32,
    pub time: Time,
    pub bezier_control_prev: Vec2,
    pub bezier_control_next: Vec2,
}

#[derive(ShaderType, Debug, Default, Clone, Copy)]
pub struct ComputedPenInput {
    pub position: Vec2,
    pub draw_direction_vec: Vec2,
    pub tilt: Vec2,
    pub angle: Vec2,
    pub draw_direction_angle: f32,
    pub pressure: f32,
    pub dab_index: u32,
    pub stroke_distance: f32,
    pub time: Time,
}

#[derive(ShaderType, Debug, Default, Clone, Copy)]
pub struct StrokePostprocessData {
    pub accumulated_pixel_bounds: IRect,
    pub time: Time,
}

#[derive(ShaderType, Debug, Default, Clone, Copy)]
pub struct Time {
    pub now: f32,
    pub stroke_begin: f32,
}

#[derive(ShaderType, Debug, Default, Clone, Copy)]
pub struct DabInfo {
    pub bound_min: IVec2,
    pub bound_max: IVec2,
}

#[derive(Clone)]
// TODO This should be renamed to RendererResources
pub struct StrokeResources {
    pub external_var_storage: Arc<GraphExternalVariableStorage>,
    pub external_var_layouts: Vec<BindGroupLayoutEntry>,
    pub external_var_buffers: Vec<Buffer>,
    pub referenced_textures: TextureAtlas,
    pub canvas_resources: Buffer,

    pub target_layer_format: TexelType,
    pub selection_layer_format: TexelType,
}

impl StrokeResources {
    fn new(
        device: &Device,
        queue: &Queue,
        brush: &CompiledBrushPreset,
        target_layer_format: TexelType,
        selection_layer_format: TexelType,
        assets: &AssetRegistry,
        canvas_resources: &DynamicBuffer<CanvasResources>,
    ) -> Self {
        let mut external_var_layouts = Vec::new();
        for cur_binding in (EXTERNAL_VARIABLE_BASE_BINDING..).take(brush.external_vars.all().len())
        {
            external_var_layouts.push(BindGroupLayoutEntry {
                binding: cur_binding,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }

        let mut external_var_buffers = Vec::new();
        for var in brush.external_vars.all().iter() {
            let (_, size) = var.value.ty().wgsl_type().unwrap();
            let gpu_buffer = device.create_buffer(&BufferDescriptor {
                label: Some("external variable buffer"),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut writer = queue
                .write_buffer_with(&gpu_buffer, 0, NonZeroU64::new(size).unwrap())
                .unwrap();
            var.value.try_write_into_shader_buffer(&mut writer).unwrap();
            external_var_buffers.push(gpu_buffer);
        }

        let empty_texture = device.create_texture(&TextureDescriptor {
            label: Some("empty texture"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::STORAGE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut referenced_textures_builder =
            TextureAtlasBuilder::with_capacity(brush.texture_usage.len());
        for id in &brush.texture_usage {
            if let Some(asset_id) = **id {
                let handle = assets.handle(asset_id).unwrap();
                let gpu_image = GpuImage::from_asset(
                    device,
                    queue,
                    &handle.get().unwrap(),
                    // TODO: This is weird but, adding TEXTURE_BINDING usage to avoid vulkan validation error:
                    // VALIDATION [VUID-VkImageViewCreateInfo-image-04441 (0xb75da543)]
                    // vkCreateImageView(): pCreateInfo->image (VkImage 0xb550000000b55) was created with VK_IMAGE_USAGE_TRANSFER_SRC_BIT|VK_IMAGE_USAGE_TRANSFER_DST_BIT but requires VK_IMAGE_USAGE_SAMPLED_BIT|VK_IMAGE_USAGE_STORAGE_BIT|VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT|VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT|VK_IMAGE_USAGE_TRANSIENT_ATTACHMENT_BIT|VK_IMAGE_USAGE_INPUT_ATTACHMENT_BIT|VK_IMAGE_USAGE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR|VK_IMAGE_USAGE_FRAGMENT_DENSITY_MAP_BIT_EXT|VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR|VK_IMAGE_USAGE_VIDEO_DECODE_DPB_BIT_KHR|VK_IMAGE_USAGE_VIDEO_ENCODE_SRC_BIT_KHR|VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR|VK_IMAGE_USAGE_SAMPLE_WEIGHT_BIT_QCOM|VK_IMAGE_USAGE_SAMPLE_BLOCK_MATCH_BIT_QCOM|VK_IMAGE_USAGE_VIDEO_ENCODE_QUANTIZATION_DELTA_MAP_BIT_KHR|VK_IMAGE_USAGE_VIDEO_ENCODE_EMPHASIS_MAP_BIT_KHR.
                    // The Vulkan spec states: image must have been created with a usage value containing at least one of the following: VK_IMAGE_USAGE_SAMPLED_BIT VK_IMAGE_USAGE_STORAGE_BIT VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT VK_IMAGE_USAGE_INPUT_ATTACHMENT_BIT VK_IMAGE_USAGE_TRANSIENT_ATTACHMENT_BIT VK_IMAGE_USAGE_FRAGMENT_SHADING_RATE_ATTACHMENT_BIT_KHR VK_IMAGE_USAGE_FRAGMENT_DENSITY_MAP_BIT_EXT VK_IMAGE_USAGE_VIDEO_DECODE_DST_BIT_KHR VK_IMAGE_USAGE_VIDEO_DECODE_DPB_BIT_KHR VK_IMAGE_USAGE_VIDEO_ENCODE_SRC_BIT_KHR VK_IMAGE_USAGE_VIDEO_ENCODE_DPB_BIT_KHR VK_IMAGE_USAGE_SAMPLE_WEIGHT_BIT_QCOM VK_IMAGE_USAGE_SAMPLE_BLOCK_MATCH_BIT_QCOM VK_IMAGE_USAGE_VIDEO_ENCODE_QUANTIZATION_DELTA_MAP_BIT_KHR VK_IMAGE_USAGE_VIDEO_ENCODE_EMPHASIS_MAP_BIT_KHR (https://docs.vulkan.org/spec/latest/chapters/resources.html#VUID-VkImageViewCreateInfo-image-04441)
                    TextureUsages::COPY_SRC | TextureUsages::TEXTURE_BINDING,
                );
                referenced_textures_builder.add_texture(gpu_image.texture.clone());
            } else {
                referenced_textures_builder.add_texture(empty_texture.clone());
            }
        }
        if referenced_textures_builder.is_empty() {
            referenced_textures_builder.add_texture(empty_texture.clone());
        }
        let referenced_textures = referenced_textures_builder
            .build(Some("referenced textures"), device, queue)
            .unwrap();

        Self {
            external_var_storage: brush.external_vars.clone(),
            external_var_layouts,
            external_var_buffers,
            referenced_textures,

            target_layer_format,
            selection_layer_format,
            canvas_resources: canvas_resources.inner_buffer().unwrap().clone(),
        }
    }

    pub fn update_external_var_buffers(&mut self, queue: &Queue) {
        for (ext_var, var_buffer) in self
            .external_var_storage
            .all()
            .iter()
            .zip(&self.external_var_buffers)
        {
            let mut writer = queue
                .write_buffer_with(var_buffer, 0, NonZeroU64::new(var_buffer.size()).unwrap())
                .unwrap();
            ext_var
                .value
                .try_write_into_shader_buffer(&mut writer)
                .unwrap();
        }
    }

    fn external_var_bindings(&self) -> Vec<BindGroupEntry<'_>> {
        self.external_var_buffers
            .iter()
            .enumerate()
            .map(|(i, buffer)| BindGroupEntry {
                binding: EXTERNAL_VARIABLE_BASE_BINDING + i as u32,
                resource: BindingResource::Buffer(buffer.as_entire_buffer_binding()),
            })
            .collect()
    }
}
