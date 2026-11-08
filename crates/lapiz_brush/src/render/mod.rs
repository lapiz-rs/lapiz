use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    mem,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail};
use bevy_math::IRect;
use chrono::{DateTime, Utc};
use encase::{ShaderSize as _, ShaderType, StorageBuffer};
use futures::{
    StreamExt as _,
    channel::{mpsc, oneshot},
};
use glam::{IVec2, IVec4, Vec2, Vec4, Vec4Swizzles};
use iced_runtime::Task;
use lapiz_canvas::{CanvasAppExt as _, CanvasId};
use lapiz_color::ForegroundBackgroundColorExt as _;
use lapiz_effect::render::EffectRunTiming;
use lapiz_image::{
    composite::PixelPreviewOverrider,
    layer::{
        LayerId,
        properties::builtin::{LayerTexelTypePropertyExt as _, TexelSource},
    },
    layer_bounds::LayerBoundsPipeline,
    scan_pixels::ScanPixelsPipeline,
    texel::TexelType,
    tile::{DynamicLayerStorage, GpuTileStorage, LayerBinding, TileStorageAppExt as _},
};
use lapiz_input::mouse::PressedMouseState;
use lapiz_render::{
    bind_group_entries::DynamicBindGroupEntries,
    buffer::DynamicBuffer,
    readback::{create_readback_buffer_and_schedule_copy_buffer, readback_buffer_on_submit_async},
};
use lapiz_runtime::global::Globals;
use lapiz_shader_graph::{
    graph::{
        slot::{ErasedGraphValueType, GraphShaderStage, GraphValueType},
        variable::{GraphLiteral, GraphShaderLiteral, GraphShaderLiteralValue},
    },
    wgsl_std::types::handle::{LayerReference, LayerType, PreparedLayer, PreparedLayerPixels},
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, ComputePassDescriptor, Device, Features, PollType,
    Queue,
};
use wgpu_profiler::{GpuProfiler, GpuProfilerSettings, GpuTimerQueryResult};

use crate::{
    input_processing::{InputProcessor, RawPenInput},
    instance::{BrushPresetInstance, CompiledBrushPreset},
    render::{
        graph::{
            BACKGROUND_COLOR_BUILTIN, FOREGROUND_COLOR_BUILTIN, HAS_SELECTION_BUILTIN,
            MAIN_ACCUMULATE_BUFFER, SELECTION_BUILTIN, TARGET_LAYER_BUILTIN,
        },
        pipeline::{BrushInputSamplingPipeline, PreparedInputSamplingPipelineData},
    },
};

pub mod graph;
pub mod pipeline;
pub mod stroke_preview;

const MAX_SAMPLES_PER_INPUT_BATCH: u32 = 256;
const MAX_INPUTS_PER_BATCH: usize = 32;

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
    foreground_color: DynamicBuffer<Vec4>,
    background_color: DynamicBuffer<Vec4>,
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
            foreground_color: DynamicBuffer::new(
                Some("brush foreground color".into()),
                BufferUsages::STORAGE,
            ),
            background_color: DynamicBuffer::new(
                Some("brush background color".into()),
                BufferUsages::STORAGE,
            ),
        }
    }

    pub fn instance(&self) -> &BrushPresetInstance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut BrushPresetInstance {
        self.renderer = None;
        &mut self.instance
    }

    pub fn begin_stroke(
        &mut self,
        input: &PressedMouseState,
        stroke_id: u64,
        canvas_id: CanvasId,
        globals: &mut Globals,
    ) -> Task<()> {
        let canvas = globals
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
            .get_texel_prop()
            .is_some_and(|property| property.source == TexelSource::DirectlyDefined)
        {
            log::warn!("Unable to paint to the active layer which cannot contain pixels.");
            return Task::none();
        }

        let xyz_to_rgb = canvas
            .image
            .profile()
            .rgb_to_xyz_matrix()
            .to_f32()
            .inverse();
        let foreground = globals.foreground_color().get().into_rgb(xyz_to_rgb);
        let background = globals.background_color().get().into_rgb(xyz_to_rgb);
        self.foreground_color.clear();
        self.foreground_color
            .push(&Vec4::new(foreground.r, foreground.g, foreground.b, 1.0));
        self.foreground_color
            .write_buffer(&self.device, &self.queue);
        self.background_color.clear();
        self.background_color
            .push(&Vec4::new(background.r, background.g, background.b, 1.0));
        self.background_color
            .write_buffer(&self.device, &self.queue);

        let tiles = globals.tile_storage();
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
        if self.session.as_ref().is_some_and(|previous| {
            previous.target_layer_format != session.target_layer_format
                || previous.selection_layer_format != session.selection_layer_format
        }) {
            self.renderer = None;
        }

        if self.renderer.is_none() {
            let compiled = self
                .instance
                .compile(
                    session.target_layer_format,
                    session.selection_layer_format,
                    &self.device,
                    &self.queue,
                )
                .expect("Failed to compile brush preset");
            self.renderer = Some(BrushPresetRenderer::new(
                compiled,
                session.target_layer_format,
                session.selection_layer_format,
                &self.device,
                &self.queue,
                &self.foreground_color,
                &self.background_color,
            ));
        }
        let renderer = self.renderer.as_mut().unwrap();

        self.input_processor.reset();
        let tiles = globals.tile_storage();
        let target_layer = tiles
            .get_layer_binding_or_empty(session.target_layer_id)
            .expect("Failed to bind active pixel layer");
        let selection_layer = tiles
            .get_layer_binding_or_empty(session.selection_layer_id)
            .expect("Failed to bind selection layer");
        let worker = renderer.begin(&self.device, &self.queue, target_layer, selection_layer);
        renderer.record_raw_inputs(1);

        if let Some(sample) =
            self.input_processor
                .push(RawPenInput::new(position, session.stroke_begin, input))
        {
            renderer.update(sample);
        }
        self.session = Some(session);
        worker
    }

    pub fn update_stroke(&mut self, input: &PressedMouseState, globals: &Globals) -> Task<()> {
        let (Some(renderer), Some(session)) = (&mut self.renderer, &self.session) else {
            return Task::none();
        };
        renderer.record_raw_inputs(1);
        let canvas = globals.canvas(&session.canvas_id).unwrap();
        let position = canvas
            .transform
            .window_to_pixel(Vec2::new(input.position.x, input.position.y));
        if let Some(sample) =
            self.input_processor
                .push(RawPenInput::new(position, session.stroke_begin, input))
        {
            renderer.update(sample);
        }
        Task::none()
    }

    pub fn end_stroke(
        &mut self,
        input: &PressedMouseState,
        globals: &mut Globals,
    ) -> Task<BrushStrokeResult> {
        let (Some(renderer), Some(session)) = (&mut self.renderer, self.session.take()) else {
            return Task::none();
        };
        renderer.record_raw_inputs(1);
        let canvas = globals
            .canvas(&session.canvas_id)
            .expect("Stroke canvas should exist");
        let position = canvas
            .transform
            .window_to_pixel(Vec2::new(input.position.x, input.position.y));
        for sample in
            self.input_processor
                .flush(RawPenInput::new(position, session.stroke_begin, input))
        {
            renderer.update(sample);
        }
        renderer.end().map(move |result| BrushStrokeResult {
            stroke_id: session.stroke_id,
            canvas_id: session.canvas_id,
            target_layer_id: session.target_layer_id,
            result,
        })
    }

    pub fn preview(&mut self) -> Task<Option<BrushStrokePreview>> {
        let (Some(renderer), Some(session)) = (&mut self.renderer, &self.session) else {
            return Task::done(None);
        };
        let stroke_id = session.stroke_id;
        let canvas_id = session.canvas_id;
        let target_layer_id = session.target_layer_id;
        renderer.generate_preview().map(move |result| {
            let result = result.filter(|result| !result.is_empty())?;
            Some(BrushStrokePreview {
                stroke_id,
                canvas_id,
                target_layer_id,
                overrider: PixelPreviewOverrider::from_layer_storage(&result),
                dirty_tiles: result.compute_tile_bounds(),
            })
        })
    }
}

#[derive(Default)]
struct BrushStrokeProfile {
    raw_inputs: u64,
    input_batches: u64,
    dabs: u64,
    dab_tiles: u64,
    effect_eval_cpu: Duration,
    effect_readback: Duration,
    effect_main_cpu: Duration,
    preview_count: u64,
    preview: Duration,
}

impl BrushStrokeProfile {
    fn record_main_effect(&mut self, timing: EffectRunTiming) {
        self.effect_eval_cpu += timing.eval_cpu;
        self.effect_readback += timing.readback;
        self.effect_main_cpu += timing.main_cpu;
    }

    fn log(&self) {
        log::info!(
            target: "lapiz_brush::profile",
            "stroke raw_inputs={} input_batches={} dabs={} dab_tiles={} effect_eval_cpu_ms={:.3} effect_readback_ms={:.3} effect_main_cpu_ms={:.3} preview_count={} preview_ms={:.3}",
            self.raw_inputs,
            self.input_batches,
            self.dabs,
            self.dab_tiles,
            self.effect_eval_cpu.as_secs_f64() * 1_000.0,
            self.effect_readback.as_secs_f64() * 1_000.0,
            self.effect_main_cpu.as_secs_f64() * 1_000.0,
            self.preview_count,
            self.preview.as_secs_f64() * 1_000.0,
        );
    }
}

#[derive(Default)]
struct StrokeMailbox {
    inputs: Vec<PenInput>,
    preview_requests: VecDeque<oneshot::Sender<Option<DynamicLayerStorage>>>,
    finish: Option<oneshot::Sender<DynamicLayerStorage>>,
}

struct StrokeSession {
    mailbox: Arc<Mutex<StrokeMailbox>>,
    wake: mpsc::Sender<()>,
    profile: Arc<Mutex<BrushStrokeProfile>>,
}

struct BrushStrokeWorker {
    state: BrushEffectState,
    input_sample: Arc<BrushInputSamplingPipeline>,
    input_profiler: GpuProfiler,
    profile: Arc<Mutex<BrushStrokeProfile>>,
    resource_group: wgpu::BindGroup,
    input_batch: DynamicBuffer<PenInputBatch>,
    batch_bounds: DynamicBuffer<BatchBounds>,
    main_batch_group: wgpu::BindGroup,
    main_sample_index_offsets: Vec<u32>,
    input_sample_prepared: PreparedInputSamplingPipelineData,
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
    input_sample: Arc<BrushInputSamplingPipeline>,
    resources: StrokeResources,
    scan_pixels: ScanPixelsPipeline,
    target_layer_bounds: Arc<LayerBoundsPipeline>,
    selection_layer_bounds: Arc<LayerBoundsPipeline>,
    compiled: Arc<CompiledBrushPreset>,
    session: Option<StrokeSession>,
}

impl BrushPresetRenderer {
    pub fn new(
        brush: CompiledBrushPreset,
        target_layer_format: TexelType,
        selection_layer_format: TexelType,
        device: &Device,
        queue: &Queue,
        foreground_color: &DynamicBuffer<Vec4>,
        background_color: &DynamicBuffer<Vec4>,
    ) -> Self {
        let resources = StrokeResources::new(
            device,
            queue,
            &brush,
            target_layer_format,
            selection_layer_format,
            foreground_color,
            background_color,
        );
        let input_sample = BrushInputSamplingPipeline::new(
            device,
            &resources.resource_layout,
            brush.spacing.clone().into(),
        );
        Self {
            input_sample: Arc::new(input_sample),
            resources,
            scan_pixels: ScanPixelsPipeline::new(device, selection_layer_format),
            target_layer_bounds: Arc::new(LayerBoundsPipeline::new(
                device,
                target_layer_format,
                false,
            )),
            selection_layer_bounds: Arc::new(LayerBoundsPipeline::new(
                device,
                selection_layer_format,
                false,
            )),
            compiled: Arc::new(brush),
            session: None,
        }
    }

    pub fn begin(
        &mut self,
        device: &Device,
        queue: &Queue,
        target_layer: LayerBinding,
        selection_layer: LayerBinding,
    ) -> Task<()> {
        let mut input_sampler =
            DynamicBuffer::new(Some("input sampler".into()), BufferUsages::STORAGE);
        input_sampler.push(&InputSampler::default());
        input_sampler.write_buffer(device, queue);

        let mut initial_pen_input = DynamicBuffer::new(
            Some("initial pen input".into()),
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
        );
        initial_pen_input.push(&ComputedPenInput::default());
        initial_pen_input.write_buffer(device, queue);

        let has_selection = self
            .scan_pixels
            .scan_to_binary_buffer(device, queue, &selection_layer);
        let target_layer_bounds = self.target_layer_bounds.create_result_buffer_uninit(device);
        let selection_layer_bounds = self
            .selection_layer_bounds
            .create_result_buffer_uninit(device);
        let mut encoder = device.create_command_encoder(&Default::default());
        self.target_layer_bounds.dispatch_to(
            device,
            queue,
            &mut encoder,
            &target_layer,
            None,
            &target_layer_bounds,
        );
        self.selection_layer_bounds.dispatch_to(
            device,
            queue,
            &mut encoder,
            &selection_layer,
            None,
            &selection_layer_bounds,
        );
        queue.submit([encoder.finish()]);
        let prepared_target_layer =
            PreparedLayer::from_binding(target_layer.clone(), target_layer_bounds.clone());
        let prepared_selection_layer =
            PreparedLayer::from_binding(selection_layer.clone(), selection_layer_bounds.clone());
        let mut input_batch =
            DynamicBuffer::new(Some("pen input batch".into()), BufferUsages::STORAGE);
        input_batch.push(&PenInputBatch::new(&[]));
        input_batch.write_buffer(device, queue);
        let mut output_samples = DynamicBuffer::new(
            Some("output samples".into()),
            BufferUsages::COPY_SRC | BufferUsages::STORAGE,
        );
        output_samples.push(&OutputSamples::new());
        output_samples.write_buffer(device, queue);

        let input_sample_prepared = self.input_sample.prepare(
            device,
            &input_batch,
            &input_sampler,
            &output_samples,
            &initial_pen_input,
        );
        let resource_group = self.resources.resource_bind_group(
            device,
            &BuiltinHostValues {
                foreground_color: &self.resources.foreground_color,
                background_color: &self.resources.background_color,
                has_selection: &has_selection,
                selection: &prepared_selection_layer,
                target_layer: &prepared_target_layer,
            },
        );

        let mut batch_bounds = DynamicBuffer::new(
            Some("brush batch bounds".into()),
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        );
        batch_bounds.push(&BatchBounds::new());
        batch_bounds.write_buffer(device, queue);
        let (main_batch_group, main_sample_index_offsets) = self.compiled.main.prepare_batch_group(
            device,
            queue,
            output_samples.inner_buffer().unwrap(),
            initial_pen_input.inner_buffer().unwrap(),
            batch_bounds.inner_buffer().unwrap(),
        );
        let profile = Arc::new(Mutex::new(BrushStrokeProfile::default()));
        let empty_accumulator = prepared_literal(
            Arc::new(LayerType {
                texel_type: self.resources.target_layer_format,
            }),
            &LayerReference,
            device,
            queue,
        )
        .expect("failed to prepare brush accumulation layer");
        let input_profiler = GpuProfiler::new(
            device,
            GpuProfilerSettings {
                enable_timer_queries: device.features().contains(Features::TIMESTAMP_QUERY),
                ..Default::default()
            },
        )
        .expect("valid brush GPU profiler settings");
        let worker = BrushStrokeWorker {
            state: BrushEffectState {
                compiled: self.compiled.clone(),
                target_layer,
                target_layer_bounds,
                selection_layer,
                selection_layer_bounds,
                has_selection,
                foreground_color: self.resources.foreground_color.clone(),
                background_color: self.resources.background_color.clone(),
                accumulator: Some(empty_accumulator),
                device: device.clone(),
                queue: queue.clone(),
                target_layer_format: self.resources.target_layer_format,
                selection_layer_format: self.resources.selection_layer_format,
            },
            input_sample: self.input_sample.clone(),
            input_profiler,
            profile: profile.clone(),
            resource_group,
            input_batch,
            batch_bounds,
            main_batch_group,
            main_sample_index_offsets,
            input_sample_prepared,
        };
        let mailbox = Arc::new(Mutex::new(StrokeMailbox::default()));
        let (wake, receiver) = mpsc::channel(1);
        self.session = Some(StrokeSession {
            mailbox: mailbox.clone(),
            wake,
            profile,
        });
        Task::future(worker.run(mailbox, receiver))
    }

    pub fn record_raw_inputs(&self, count: usize) {
        if let Some(session) = &self.session {
            session.profile.lock().raw_inputs += count as u64;
        }
    }

    pub fn update(&mut self, input: PenInput) {
        let Some(session) = &mut self.session else {
            return;
        };
        session.mailbox.lock().inputs.push(input);
        let _ = session.wake.try_send(());
    }

    pub fn end(&mut self) -> Task<DynamicLayerStorage> {
        let Some(mut session) = self.session.take() else {
            return Task::none();
        };
        let (sender, receiver) = oneshot::channel();
        let previous = session.mailbox.lock().finish.replace(sender);
        assert!(
            previous.is_none(),
            "brush stroke finish was already requested"
        );
        let _ = session.wake.try_send(());
        Task::future(async move {
            receiver
                .await
                .expect("brush stroke worker stopped before producing a result")
        })
    }

    pub fn generate_preview(&mut self) -> Task<Option<DynamicLayerStorage>> {
        let Some(session) = &mut self.session else {
            return Task::done(None);
        };
        let (sender, receiver) = oneshot::channel();
        session.mailbox.lock().preview_requests.push_back(sender);
        let _ = session.wake.try_send(());
        Task::future(async move { receiver.await.unwrap_or(None) })
    }
}

enum StrokeWork {
    Inputs(Vec<PenInput>),
    Preview(oneshot::Sender<Option<DynamicLayerStorage>>),
    Finish(oneshot::Sender<DynamicLayerStorage>),
}

impl BrushStrokeWorker {
    async fn run(mut self, mailbox: Arc<Mutex<StrokeMailbox>>, mut wake: mpsc::Receiver<()>) {
        while wake.next().await.is_some() {
            loop {
                let work = {
                    let mut mailbox = mailbox.lock();
                    if !mailbox.inputs.is_empty() {
                        Some(StrokeWork::Inputs(mem::take(&mut mailbox.inputs)))
                    } else if let Some(request) = mailbox.preview_requests.pop_front() {
                        Some(StrokeWork::Preview(request))
                    } else {
                        mailbox.finish.take().map(StrokeWork::Finish)
                    }
                };
                let Some(work) = work else {
                    break;
                };
                match work {
                    StrokeWork::Inputs(inputs) => {
                        for inputs in inputs.chunks(MAX_INPUTS_PER_BATCH) {
                            if let Err(error) = self.process_input_batch(inputs).await {
                                log::error!("Brush main effects failed: {error:#}");
                            }
                        }
                    }
                    StrokeWork::Preview(sender) => {
                        sender.send(self.generate_preview()).ok();
                    }
                    StrokeWork::Finish(sender) => {
                        sender.send(self.finish()).ok();
                        return;
                    }
                }
            }
        }
    }

    async fn process_input_batch(&mut self, inputs: &[PenInput]) -> Result<()> {
        // sample input
        let device = self.state.device.clone();
        let queue = self.state.queue.clone();
        self.input_batch.clear();
        self.input_batch.push(&PenInputBatch::new(inputs));
        self.input_batch.write_buffer(&device, &queue);
        let batch_index = {
            let mut profile = self.profile.lock();
            profile.input_batches += 1;
            profile.input_batches
        };

        let mut encoder = device.create_command_encoder(&Default::default());
        log_finished_gpu_profiles(&mut self.input_profiler, &queue);
        let query = self.input_profiler.begin_pass_query(
            format!("brush/input_sample/batch_{batch_index}"),
            &mut encoder,
        );
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("brush input sampling pass"),
                timestamp_writes: query.compute_pass_timestamp_writes(),
            });
            self.input_sample.dispatch(
                &mut pass,
                &self.input_sample_prepared,
                &self.resource_group,
            );
        }
        self.input_profiler.end_query(&mut encoder, query);

        // batched eval
        let main_pipeline = &self.state.compiled.main;
        let eval_started = Instant::now();
        let builtins = main_builtins(&self.state)?;
        let resources = main_pipeline.prepare_resource_group(&device, &builtins)?;
        main_pipeline.dispatch_eval(&mut encoder, &self.main_batch_group, &resources);
        let eval_cpu = eval_started.elapsed();
        let bounds_buffer = self.batch_bounds.inner_buffer().unwrap();
        let bounds_staging =
            create_readback_buffer_and_schedule_copy_buffer(&device, &mut encoder, bounds_buffer);
        let bounds_readback =
            readback_buffer_on_submit_async::<BatchBounds, _>(&mut encoder, &bounds_staging, ..);
        self.input_profiler.resolve_queries(&mut encoder);
        let readback_started = Instant::now();
        queue.submit([encoder.finish()]);
        self.input_profiler
            .end_frame()
            .expect("brush input GPU profile frame must be complete");

        // readback bounds and allocate
        let samples = bounds_readback.into_inner().await??;
        let bounds_readback = readback_started.elapsed();
        let batch_dab_count = samples.n_samples as usize;
        let main_started = Instant::now();

        let count = samples.n_samples as usize;
        if samples.is_overflow != 0 {
            bail!("Brush sample batch exceeds its output capacity");
        }

        let mut dab_bounds = Vec::with_capacity(count);
        let mut tiles = HashSet::new();
        for entry in samples.entries.iter().take(count) {
            let rect = IRect {
                min: entry.bounds.xy(),
                max: entry.bounds.zw(),
            };

            if !rect.is_empty() {
                let dab_tiles = GpuTileStorage::pixel_rect_to_tile(rect);
                for y in dab_tiles.min.y..=dab_tiles.max.y {
                    for x in dab_tiles.min.x..=dab_tiles.max.x {
                        tiles.insert(IVec2::new(x, y));
                    }
                }
            }

            dab_bounds.push(rect);
        }

        let mut dab = self.state.compiled.main.prepare_output(&device, &queue)?;
        let PreparedLayerPixels::ReadWrite {
            storage: dab_storage,
            ..
        } = &mut dab.pixels
        else {
            unreachable!();
        };
        dab_storage.allocate_tiles_batch(&tiles);
        let accumulator = self
            .state
            .accumulator
            .as_mut()
            .context("Missing brush accumulator")?
            .as_mut::<PreparedLayer>();
        let PreparedLayerPixels::ReadWrite { storage, .. } = &mut accumulator.pixels else {
            bail!("Brush accumulator must have writable storage");
        };
        storage.allocate_tiles_batch(&tiles);

        // run main
        let builtins = main_builtins(&self.state)?;
        let resources = main_pipeline.prepare_resource_group(&device, &builtins)?;
        let output = main_pipeline.prepare_output_group(&device, &dab)?;
        let PreparedLayerPixels::ReadWrite {
            storage: dab_storage,
            ..
        } = &dab.pixels
        else {
            unreachable!();
        };
        let mut batch_dab_tiles = Vec::with_capacity(batch_dab_count);
        for (index, dab_bounds) in dab_bounds.into_iter().enumerate() {
            if dab_bounds.is_empty() {
                batch_dab_tiles.push(0);
                continue;
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            main_pipeline.dispatch_main(
                &mut encoder,
                &self.main_batch_group,
                &resources,
                &output,
                self.main_sample_index_offsets[index],
                dab_bounds,
            )?;
            queue.submit([encoder.finish()]);
            let accumulator = self
                .state
                .accumulator
                .as_mut()
                .unwrap()
                .as_mut::<PreparedLayer>();
            let PreparedLayerPixels::ReadWrite { storage, .. } = &mut accumulator.pixels else {
                unreachable!();
            };
            batch_dab_tiles.push(
                dab_storage
                    .iter_tile_indices()
                    .filter(|tile| {
                        !GpuTileStorage::tile_to_pixel_rect(*tile)
                            .intersect(dab_bounds)
                            .is_empty()
                    })
                    .count(),
            );
            storage.copy_pixels_from(dab_storage, dab_bounds);
            let previous = accumulator.pixel_bounds.unwrap_or(IRect::EMPTY);
            let bounds = if previous.is_empty() {
                dab_bounds
            } else {
                previous.union(dab_bounds)
            };
            accumulator.pixel_bounds = Some(bounds);
            let mut encoded = StorageBuffer::new(Vec::new());
            encoded.write(&IVec4::new(
                bounds.min.x,
                bounds.min.y,
                bounds.max.x,
                bounds.max.y,
            ))?;
            queue.write_buffer(&accumulator.bounds, 0, encoded.as_ref());
        }
        let effect_timing = EffectRunTiming {
            eval_cpu,
            readback: bounds_readback,
            main_cpu: main_started.elapsed(),
        };

        let accumulator_tiles = self
            .state
            .accumulator
            .as_ref()
            .and_then(|literal| literal.try_as_ref::<PreparedLayer>())
            .and_then(|layer| match &layer.pixels {
                PreparedLayerPixels::ReadWrite { storage, .. } => Some(storage.len()),
                PreparedLayerPixels::ReadOnly(_) => None,
            })
            .unwrap_or(0);
        let batch_tile_count = batch_dab_tiles.iter().sum::<usize>();
        {
            let mut profile = self.profile.lock();
            profile.dabs += batch_dab_count as u64;
            profile.dab_tiles += batch_tile_count as u64;
            profile.record_main_effect(effect_timing);
        }
        log::info!(
            target: "lapiz_brush::profile",
            "batch index={} inputs={} dabs={} dab_tiles={:?} batch_tiles={} accumulator_tiles={} eval_cpu_ms={:.3} effect_readback_ms={:.3} main_cpu_ms={:.3}",
            batch_index,
            inputs.len(),
            batch_dab_count,
            batch_dab_tiles,
            batch_tile_count,
            accumulator_tiles,
            effect_timing.eval_cpu.as_secs_f64() * 1_000.0,
            effect_timing.readback.as_secs_f64() * 1_000.0,
            effect_timing.main_cpu.as_secs_f64() * 1_000.0,
        );
        Ok(())
    }

    fn generate_preview(&mut self) -> Option<DynamicLayerStorage> {
        let accumulator = self.state.accumulator.as_ref()?;
        let ty = accumulator.ty().clone();
        let accumulator = accumulator.as_ref::<PreparedLayer>();
        if accumulator
            .pixel_bounds
            .is_none_or(|bounds| bounds.is_empty())
        {
            return None;
        }
        let prepared = accumulator.deep_clone();
        let original = self
            .state
            .accumulator
            .replace(GraphShaderLiteral::new_boxed(Box::new(prepared), ty));
        let started = Instant::now();
        let mut timing = EffectRunTiming::default();
        let result = run_postprocess(&mut self.state, &mut timing).ok();
        let elapsed = started.elapsed();
        {
            let mut profile = self.profile.lock();
            profile.preview_count += 1;
            profile.preview += elapsed;
        }
        log::info!(
            target: "lapiz_brush::profile",
            "preview duration_ms={:.3} eval_cpu_ms={:.3} readback_ms={:.3} main_cpu_ms={:.3}",
            elapsed.as_secs_f64() * 1_000.0,
            timing.eval_cpu.as_secs_f64() * 1_000.0,
            timing.readback.as_secs_f64() * 1_000.0,
            timing.main_cpu.as_secs_f64() * 1_000.0,
        );
        self.state.accumulator = original;
        result.map(|literal| {
            let result = literal.downcast::<PreparedLayer>();
            let PreparedLayerPixels::ReadWrite { storage, .. } = result.pixels else {
                panic!("brush postprocess returned a read-only layer");
            };
            storage
        })
    }

    fn finish(&mut self) -> DynamicLayerStorage {
        let mut timing = EffectRunTiming::default();
        let result =
            run_postprocess(&mut self.state, &mut timing).expect("brush postprocess failed");
        self.state
            .compiled
            .postprocess
            .finish_profiling()
            .expect("failed to finish postprocess effect profiling");
        self.state
            .device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("failed to finish brush input profiling");
        log_finished_gpu_profiles(&mut self.input_profiler, &self.state.queue);
        self.profile.lock().log();
        let result = result.downcast::<PreparedLayer>();
        let PreparedLayerPixels::ReadWrite { storage, .. } = result.pixels else {
            panic!("brush postprocess returned a read-only layer");
        };
        storage
    }
}

fn run_postprocess(
    state: &mut BrushEffectState,
    timing: &mut EffectRunTiming,
) -> Result<GraphShaderLiteral> {
    let accumulator = state
        .accumulator
        .take()
        .context("missing brush accumulator")?;
    let builtins = postprocess_builtins(state, accumulator)?;
    let mut outputs = state.compiled.postprocess.run_profiled(
        &state.compiled.postprocess_inputs,
        &builtins,
        timing,
    )?;
    outputs
        .remove(&state.compiled.postprocess_output)
        .context("postprocess effect did not produce the stroke result")
}

struct BrushEffectState {
    compiled: Arc<CompiledBrushPreset>,
    target_layer: LayerBinding,
    target_layer_bounds: Buffer,
    selection_layer: LayerBinding,
    selection_layer_bounds: Buffer,
    has_selection: Buffer,
    foreground_color: Buffer,
    background_color: Buffer,
    accumulator: Option<GraphShaderLiteral>,
    device: Device,
    queue: Queue,
    target_layer_format: TexelType,
    selection_layer_format: TexelType,
}

fn log_finished_gpu_profiles(profiler: &mut GpuProfiler, queue: &Queue) {
    while let Some(results) = profiler.process_finished_frame(queue.get_timestamp_period()) {
        log_gpu_profile_results(&results);
    }
}

fn log_gpu_profile_results(results: &[GpuTimerQueryResult]) {
    for result in results {
        if let Some(time) = &result.time {
            log::info!(
                target: "lapiz_gpu_profile",
                "scope={} duration_ms={:.6}",
                result.label,
                (time.end - time.start) * 1_000.0
            );
        }
        log_gpu_profile_results(&result.nested_queries);
    }
}

fn main_builtins(state: &BrushEffectState) -> Result<HashMap<String, GraphShaderLiteral>> {
    let mut values = base_builtins(state);
    let accumulator = state
        .accumulator
        .as_ref()
        .context("Missing brush accumulator")?
        .as_ref::<PreparedLayer>();
    let PreparedLayerPixels::ReadWrite { storage, .. } = &accumulator.pixels else {
        bail!("Brush accumulator must have writable storage");
    };
    values.insert(
        MAIN_ACCUMULATE_BUFFER.into(),
        GraphShaderLiteral::new_boxed(
            Box::new(PreparedLayer::from_binding(
                storage.binding_or_empty(),
                accumulator.bounds.clone(),
            )),
            Arc::new(LayerType {
                texel_type: state.target_layer_format,
            }),
        ),
    );
    Ok(values)
}

fn postprocess_builtins(
    state: &BrushEffectState,
    accumulator: GraphShaderLiteral,
) -> Result<HashMap<String, GraphShaderLiteral>> {
    let mut values = base_builtins(state);
    values.insert(MAIN_ACCUMULATE_BUFFER.into(), accumulator);
    Ok(values)
}

fn base_builtins(state: &BrushEffectState) -> HashMap<String, GraphShaderLiteral> {
    let types = graph::brush_builtin_types(state.target_layer_format, state.selection_layer_format);
    let mut values = HashMap::new();
    for (name, ty) in types {
        let value = match name.as_str() {
            FOREGROUND_COLOR_BUILTIN => {
                Box::new(state.foreground_color.clone()) as Box<dyn GraphShaderLiteralValue>
            }
            BACKGROUND_COLOR_BUILTIN => Box::new(state.background_color.clone()),
            HAS_SELECTION_BUILTIN => Box::new(state.has_selection.clone()),
            SELECTION_BUILTIN => Box::new(PreparedLayer::from_binding(
                state.selection_layer.clone(),
                state.selection_layer_bounds.clone(),
            )),
            TARGET_LAYER_BUILTIN => Box::new(PreparedLayer::from_binding(
                state.target_layer.clone(),
                state.target_layer_bounds.clone(),
            )),
            _ => unreachable!(),
        };
        values.insert(name, GraphShaderLiteral::new_boxed(value, ty));
    }
    values
}

fn prepared_literal<T: GraphValueType>(
    ty: Arc<T>,
    value: &T::AssociatedLiteralType,
    device: &Device,
    queue: &Queue,
) -> Result<GraphShaderLiteral> {
    let prepared = ty.prepare_to_shader(value, device, queue)?;
    Ok(GraphShaderLiteral::new_boxed(Box::new(prepared), ty))
}

#[derive(Clone, Debug, ShaderType)]
pub struct PenInputBatch {
    pub n_inputs: u32,
    #[shader(size(runtime))]
    pub inputs: Vec<PenInput>,
}

impl PenInputBatch {
    pub fn new(inputs: &[PenInput]) -> Self {
        assert!(inputs.len() <= MAX_INPUTS_PER_BATCH);
        let mut batch = inputs.to_vec();
        batch.resize(MAX_INPUTS_PER_BATCH, PenInput::default());
        Self {
            n_inputs: inputs.len() as u32,
            inputs: batch,
        }
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
    pub fn new() -> Self {
        Self {
            n_samples: 0,
            is_overflow: 0,
            samples: vec![ComputedPenInput::default(); MAX_SAMPLES_PER_INPUT_BATCH as usize],
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

#[derive(ShaderType, Debug, Default, Clone, Copy, Serialize, Deserialize)]
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

#[derive(ShaderType, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Time {
    pub now: f32,
    pub stroke_begin: f32,
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct DabBounds {
    pub bounds: IVec4,
    pub accumulator_bounds: IVec4,
}

#[derive(Debug, ShaderType)]
pub struct BatchBounds {
    pub n_samples: u32,
    pub is_overflow: u32,
    #[shader(size(runtime))]
    pub entries: Vec<DabBounds>,
}

impl BatchBounds {
    pub fn new() -> Self {
        Self {
            n_samples: 0,
            is_overflow: 0,
            entries: vec![
                DabBounds {
                    bounds: IVec4::ZERO,
                    accumulator_bounds: IVec4::ZERO
                };
                MAX_SAMPLES_PER_INPUT_BATCH as usize
            ],
        }
    }
}

pub struct BuiltinHostValues<'a> {
    pub foreground_color: &'a Buffer,
    pub background_color: &'a Buffer,
    pub has_selection: &'a Buffer,
    pub selection: &'a PreparedLayer,
    pub target_layer: &'a PreparedLayer,
}

pub struct StrokeResources {
    resource_layout: wgpu::BindGroupLayout,
    builtin_types: BTreeMap<String, Arc<dyn ErasedGraphValueType>>,
    parameters: Arc<[GraphLiteral]>,
    prepared_parameters: Vec<Box<dyn GraphShaderLiteralValue>>,
    foreground_color: Buffer,
    background_color: Buffer,
    target_layer_format: TexelType,
    selection_layer_format: TexelType,
}

impl StrokeResources {
    fn new(
        device: &Device,
        queue: &Queue,
        brush: &CompiledBrushPreset,
        target_layer_format: TexelType,
        selection_layer_format: TexelType,
        foreground_color: &DynamicBuffer<Vec4>,
        background_color: &DynamicBuffer<Vec4>,
    ) -> Self {
        let prepared_parameters = brush
            .spacing_parameters
            .iter()
            .map(|parameter| {
                parameter
                    .ty()
                    .prepare_to_shader(parameter.value(), device, queue)
                    .expect("failed to prepare spacing parameter")
            })
            .collect();
        Self {
            resource_layout: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("brush spacing resources"),
                entries: &brush.spacing_resource_layouts,
            }),
            builtin_types: brush.spacing_builtin_types.clone(),
            parameters: brush.spacing_parameters.clone(),
            prepared_parameters,
            foreground_color: foreground_color.inner_buffer().unwrap().clone(),
            background_color: background_color.inner_buffer().unwrap().clone(),
            target_layer_format,
            selection_layer_format,
        }
    }

    fn resource_bind_group(
        &self,
        device: &Device,
        builtins: &BuiltinHostValues<'_>,
    ) -> wgpu::BindGroup {
        let mut binding = 0;
        let mut entries = Vec::new();
        for (name, ty) in &self.builtin_types {
            let value = match name.as_str() {
                FOREGROUND_COLOR_BUILTIN => {
                    builtins.foreground_color as &dyn GraphShaderLiteralValue
                }
                BACKGROUND_COLOR_BUILTIN => builtins.background_color,
                HAS_SELECTION_BUILTIN => builtins.has_selection,
                SELECTION_BUILTIN => builtins.selection,
                TARGET_LAYER_BUILTIN => builtins.target_layer,
                _ => unreachable!(),
            };
            let (next, bound) = ty
                .push_shader_binding(
                    GraphShaderStage::Input,
                    value,
                    binding,
                    DynamicBindGroupEntries::new(),
                )
                .expect("failed to bind spacing builtin");
            binding = next;
            entries.extend(bound.to_vec());
        }
        for (parameter, prepared) in self.parameters.iter().zip(&self.prepared_parameters) {
            let (next, bound) = parameter
                .ty()
                .push_shader_binding(
                    GraphShaderStage::Input,
                    prepared.as_ref(),
                    binding,
                    DynamicBindGroupEntries::new(),
                )
                .expect("failed to bind spacing parameter");
            binding = next;
            entries.extend(bound.to_vec());
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("brush spacing resources"),
            layout: &self.resource_layout,
            entries: &entries,
        })
    }
}
