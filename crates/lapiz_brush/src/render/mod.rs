use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    mem,
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use bevy_math::IRect;
use chrono::{DateTime, Utc};
use encase::ShaderType;
use futures::{
    StreamExt as _,
    channel::{mpsc, oneshot},
};
use glam::{IVec2, IVec4, Vec2, Vec4, Vec4Swizzles as _};
use iced_runtime::Task;
use lapiz_canvas::{CanvasAppExt as _, CanvasId};
use lapiz_color::ForegroundBackgroundColorExt as _;
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
    readback::{create_readback_buffer_and_schedule_copy_buffer, readback_buffer_on_submit_async},
    storage_buffer::StorageBuffer,
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
use wgpu::{Buffer, BufferUsages, ComputePassDescriptor, Device, Queue};

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

/// Metadata describing an active brush stroke session on a specific canvas and target layer.
pub struct CanvasBrushStrokeSessionInfo {
    pub stroke_id: u64,
    pub stroke_begin: DateTime<Utc>,
    pub canvas_id: CanvasId,
    pub target_layer_id: LayerId,
    pub selection_layer_id: LayerId,
    pub target_layer_format: TexelType,
    pub selection_layer_format: TexelType,
}

/// High-level brush operator that drives a [`BrushPresetInstance`] across interactive canvas strokes.
///
/// `CanvasBrushPresetOperator` bridges canvas pointer events with the GPU brush engine:
///
/// 1. **Compilation & Caching**: Lazily compiles [`BrushPresetInstance`] into a [`BrushPresetRenderer`]
///    matching the active target and selection [`TexelType`]s, invalidating the cached renderer
///    when parameters or layer formats change.
/// 2. **Input Stabilization**: Transforms window pointer coordinates into canvas pixel space and
///    feeds [`RawPenInput`] samples through [`InputProcessor`] to produce smoothed [`PenInput`] segments.
/// 3. **Stroke Execution**: Starts a background GPU stroke worker on [`begin_stroke`](Self::begin_stroke),
///    streams new segments on [`update_stroke`](Self::update_stroke), generates intermediate
///    [`BrushStrokePreview`]s on [`preview`](Self::preview), and flushes the final
///    [`BrushStrokeResult`] on [`end_stroke`](Self::end_stroke).
pub struct CanvasBrushPresetOperator {
    instance: BrushPresetInstance,
    device: Device,
    queue: Queue,
    renderer: Option<BrushPresetRenderer>,
    session: Option<CanvasBrushStrokeSessionInfo>,
    input_processor: InputProcessor,
    foreground_color: StorageBuffer<Vec4>,
    background_color: StorageBuffer<Vec4>,
}

impl CanvasBrushPresetOperator {
    pub fn new(
        instance: BrushPresetInstance,
        device: Device,
        queue: Queue,
        input_processor: InputProcessor,
    ) -> Self {
        let mut foreground_color = StorageBuffer::default();
        foreground_color.set_label(Some("brush foreground color"));
        let mut background_color = StorageBuffer::default();
        background_color.set_label(Some("brush background color"));

        Self {
            instance,
            renderer: None,
            device,
            queue,
            session: None,
            input_processor,
            foreground_color,
            background_color,
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
        self.foreground_color
            .set(Vec4::new(foreground.r, foreground.g, foreground.b, 1.0));
        self.foreground_color
            .write_buffer(&self.device, &self.queue);
        self.background_color
            .set(Vec4::new(background.r, background.g, background.b, 1.0));
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
struct StrokeMailbox {
    inputs: Vec<PenInput>,
    preview_requests: VecDeque<oneshot::Sender<Option<DynamicLayerStorage>>>,
    finish: Option<oneshot::Sender<DynamicLayerStorage>>,
}

struct StrokeSession {
    mailbox: Arc<Mutex<StrokeMailbox>>,
    wake: mpsc::Sender<()>,
}

struct BrushStrokeWorker {
    state: BrushEffectState,
    input_sample: Arc<BrushInputSamplingPipeline>,
    resource_group: wgpu::BindGroup,
    input_batch: StorageBuffer<PenInputBatch>,
    batch_bounds: StorageBuffer<BatchBounds>,
    main_batch_group: wgpu::BindGroup,
    main_sample_index_offsets: Vec<u32>,
    input_sample_prepared: PreparedInputSamplingPipelineData,
}

/// Intermediate stroke preview produced during an active brush stroke for live canvas compositing.
pub struct BrushStrokePreview {
    pub stroke_id: u64,
    pub canvas_id: CanvasId,
    pub target_layer_id: LayerId,
    pub overrider: PixelPreviewOverrider,
    pub dirty_tiles: IRect,
}

/// Completed GPU tile storage produced when a brush stroke finishes and is ready to be committed to the undo stack.
pub struct BrushStrokeResult {
    pub stroke_id: u64,
    pub canvas_id: CanvasId,
    pub target_layer_id: LayerId,
    pub result: DynamicLayerStorage,
}

/// GPU execution engine for a [`CompiledBrushPreset`].
///
/// `BrushPresetRenderer` orchestrates the three-stage GPU brush pipeline for an individual stroke:
///
/// 1. **Input Curve Sampling** ([`BrushInputSamplingPipeline`]): Evaluates the spacing shader graph
///    along incoming cubic Bezier [`PenInputBatch`] segments to emit discrete [`ComputedPenInput`] dabs.
/// 2. **Main Dab Evaluation & Rendering** ([`BrushMainPipeline`](crate::render::pipeline::BrushMainPipeline)):
///    Runs an evaluation pass to compute each dab's pixel bounding box, allocates the required
///    GPU tiles, and dispatches per-dab compute passes that accumulate into the stroke buffer.
/// 3. **Postprocessing**: Executes the postprocess [`EffectRenderer`](lapiz_effect::render::EffectRenderer)
///    over the accumulated stroke buffer to produce either a live preview or the final [`DynamicLayerStorage`].
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
        foreground_color: &StorageBuffer<Vec4>,
        background_color: &StorageBuffer<Vec4>,
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
        let mut input_sampler = StorageBuffer::from(InputSampler::default());
        input_sampler.set_label(Some("input sampler"));
        input_sampler.write_buffer(device, queue);

        let mut initial_pen_input = StorageBuffer::from(ComputedPenInput::default());
        initial_pen_input.set_label(Some("initial pen input"));
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
        let mut input_batch = StorageBuffer::from(PenInputBatch::new(&[]));
        input_batch.set_label(Some("pen input batch"));
        input_batch.write_buffer(device, queue);
        let mut output_samples = StorageBuffer::from(OutputSamples::default());
        output_samples.set_label(Some("output samples"));
        output_samples.add_usages(BufferUsages::COPY_SRC);
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

        let mut batch_bounds = StorageBuffer::from(BatchBounds::default());
        batch_bounds.set_label(Some("brush batch bounds"));
        batch_bounds.add_usages(BufferUsages::COPY_SRC);
        batch_bounds.write_buffer(device, queue);
        let (main_batch_group, main_sample_index_offsets) = self.compiled.main.prepare_batch_group(
            device,
            queue,
            output_samples.buffer().unwrap(),
            initial_pen_input.buffer().unwrap(),
            batch_bounds.buffer().unwrap(),
        );
        let empty_accumulator = prepared_literal(
            Arc::new(LayerType {
                texel_type: self.resources.target_layer_format,
            }),
            &LayerReference,
            device,
            queue,
        )
        .expect("failed to prepare brush accumulation layer");
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
        });
        Task::future(worker.run(mailbox, receiver))
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
        self.input_batch.set(PenInputBatch::new(inputs));
        self.input_batch.write_buffer(&device, &queue);

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("brush input sampling pass"),
                timestamp_writes: None,
            });
            self.input_sample.dispatch(
                &mut pass,
                &self.input_sample_prepared,
                &self.resource_group,
            );
        }

        // batched eval
        let main_pipeline = &self.state.compiled.main;
        let builtins = main_builtins(&self.state)?;
        let resources = main_pipeline.prepare_resource_group(&device, &builtins)?;
        main_pipeline.dispatch_eval(&mut encoder, &self.main_batch_group, &resources);
        let bounds_buffer = self.batch_bounds.buffer().unwrap();
        let bounds_staging =
            create_readback_buffer_and_schedule_copy_buffer(&device, &mut encoder, bounds_buffer);
        let bounds_readback =
            readback_buffer_on_submit_async::<BatchBounds, _>(&mut encoder, &bounds_staging, ..);
        queue.submit([encoder.finish()]);

        // readback bounds and allocate
        let samples = bounds_readback.into_inner().await??;

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
        for (index, dab_bounds) in dab_bounds.into_iter().enumerate() {
            if dab_bounds.is_empty() {
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
            storage.copy_pixels_from(dab_storage, dab_bounds);
            let previous = accumulator.pixel_bounds.unwrap_or(IRect::EMPTY);
            let bounds = if previous.is_empty() {
                dab_bounds
            } else {
                previous.union(dab_bounds)
            };
            accumulator.pixel_bounds = Some(bounds);
            let mut encoded = encase::StorageBuffer::new(Vec::new());
            encoded.write(&IVec4::new(
                bounds.min.x,
                bounds.min.y,
                bounds.max.x,
                bounds.max.y,
            ))?;
            queue.write_buffer(&accumulator.bounds, 0, encoded.as_ref());
        }
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
        let result = run_postprocess(&mut self.state).ok();
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
        let result = run_postprocess(&mut self.state).expect("brush postprocess failed");
        let result = result.downcast::<PreparedLayer>();
        let PreparedLayerPixels::ReadWrite { storage, .. } = result.pixels else {
            panic!("brush postprocess returned a read-only layer");
        };
        storage
    }
}

fn run_postprocess(state: &mut BrushEffectState) -> Result<GraphShaderLiteral> {
    let accumulator = state
        .accumulator
        .take()
        .context("missing brush accumulator")?;
    let builtins = postprocess_builtins(state, accumulator)?;
    let mut outputs = state
        .compiled
        .postprocess
        .run(&state.compiled.postprocess_inputs, &builtins)?;
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
    value: &T::ValueType,
    device: &Device,
    queue: &Queue,
) -> Result<GraphShaderLiteral> {
    let prepared = ty.prepare_to_shader(value, device, queue)?;
    Ok(GraphShaderLiteral::new_boxed(Box::new(prepared), ty))
}

/// GPU storage buffer payload containing a fixed-capacity batch of [`PenInput`] segments for curve sampling.
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

/// GPU storage buffer payload holding the [`ComputedPenInput`] dabs emitted by [`BrushInputSamplingPipeline`].
#[derive(ShaderType, Debug, Clone)]
pub struct OutputSamples {
    pub n_samples: u32,
    pub is_overflow: u32,
    #[shader(size(runtime))]
    pub samples: Vec<ComputedPenInput>,
}

impl Default for OutputSamples {
    fn default() -> Self {
        Self {
            n_samples: 0,
            is_overflow: 0,
            samples: vec![ComputedPenInput::default(); MAX_SAMPLES_PER_INPUT_BATCH as usize],
        }
    }
}

/// Persistent GPU state carried across [`BrushInputSamplingPipeline`] dispatches within a single stroke.
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

/// Stabilized pen sample with cubic Bezier control points defining a curve segment from the previous input.
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

/// Interpolated per-dab pen state produced by GPU curve sampling and consumed by brush shader graphs.
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

/// Timestamp pair recording the current sample time and the stroke start time.
#[derive(ShaderType, Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Time {
    pub now: f32,
    pub stroke_begin: f32,
}

/// Pixel bounding box and cumulative stroke bounds computed for a single brush dab.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct DabBounds {
    pub bounds: IVec4,
    pub accumulator_bounds: IVec4,
}

/// GPU readback buffer payload containing the evaluated [`DabBounds`] for all dabs in a sample batch.
#[derive(Debug, ShaderType)]
pub struct BatchBounds {
    pub n_samples: u32,
    pub is_overflow: u32,
    #[shader(size(runtime))]
    pub entries: Vec<DabBounds>,
}

impl Default for BatchBounds {
    fn default() -> Self {
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

/// Borrowed GPU buffers and prepared layers representing the built-in host inputs to a brush shader.
pub struct BuiltinHostValues<'a> {
    pub foreground_color: &'a Buffer,
    pub background_color: &'a Buffer,
    pub has_selection: &'a Buffer,
    pub selection: &'a PreparedLayer,
    pub target_layer: &'a PreparedLayer,
}

/// Prepared GPU bind group layout and parameter buffers for the brush spacing stage.
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
        foreground_color: &StorageBuffer<Vec4>,
        background_color: &StorageBuffer<Vec4>,
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
            foreground_color: foreground_color.buffer().unwrap().clone(),
            background_color: background_color.buffer().unwrap().clone(),
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
