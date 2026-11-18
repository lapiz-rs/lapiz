use std::{array::from_fn, f32::consts::TAU};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use futures::{FutureExt as _, StreamExt as _, future::BoxFuture};
use glam::{IVec4, Vec2, Vec4};
use iced_runtime::{Action, task::into_stream};
use image::RgbaImage;
use lapiz_assets::{
    AssetAppExt as _,
    asset::{AssetHandle, UntypedAssetId},
};
use lapiz_image::{
    layer_bounds::LayerBoundsPipeline,
    texel::TexelType,
    tile::{DynamicLayerStorage, GpuLayerInfo, GpuTileInfo, LayerBinding},
};
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    readback::{
        create_readback_buffer_and_schedule_copy_texture, readback_buffer_raw_on_submit_async,
    },
    render_context::RenderContextAppExt as _,
    storage_buffer::StorageBuffer,
    util::DevicePollExt as _,
};
use lapiz_runtime::global::Globals;
use lapiz_utils::log_err::LogErr as _;
use wesl::include_wesl;
use wgpu::{
    BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, ComputePassDescriptor,
    ComputePipeline, ComputePipelineDescriptor, Device, Extent3d, PipelineLayoutDescriptor, Queue,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StorageTextureAccess, Texture,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
};

use crate::{
    asset::BrushPreset,
    engine::{BrushEngine, BrushEngineRegistryExt as _},
    input_processing::{BasicStabilizer, InputProcessor, RawPenInput},
    instance::BrushPresetInstance,
    render::{BrushPresetRenderer, Time, graph::CanvasResources},
};

pub const CACHED_STROKE_PREVIEW_SIZE: (u32, u32) = (512, 256);

pub fn generate_stroke_preview<E: BrushEngine>(
    handle: &AssetHandle<E::Brush>,
    globals: &Globals,
) -> BoxFuture<'static, Result<RgbaImage>> {
    let prepare = || -> Result<_> {
        let brush = handle.get()?;
        let untyped_id = handle.id().into_untyped();
        let factory = globals.brush_engine_registry().get(&E::id())?;
        let engine = factory.load_brush(brush.as_ref(), Some(untyped_id), globals.assets())?;
        Ok(generate_stroke_preview_compiled(
            &engine.compile_brush(globals)?,
            Some(untyped_id),
            globals,
        ))
    };
    let preview = prepare();
    async move { preview?.await }.boxed()
}

/// Prepares owned resources so the returned task does not borrow the application globals.
pub fn generate_stroke_preview_compiled(
    brush: &BrushPreset,
    asset_id: Option<UntypedAssetId>,
    globals: &Globals,
) -> BoxFuture<'static, Result<RgbaImage>> {
    let instance = BrushPresetInstance::new(brush, asset_id, globals.assets().clone())
        .map_err(|error| anyhow!("Failed to create brush preset instance: {error:#}"));
    let device = globals.render_device().clone();
    let queue = globals.render_queue().clone();

    async move {
        let instance = instance?;
        let texture = create_stroke_preview_with(
            &instance,
            &predefined_curve_samples(CACHED_STROKE_PREVIEW_SIZE.0, CACHED_STROKE_PREVIEW_SIZE.1),
            CACHED_STROKE_PREVIEW_SIZE.0,
            CACHED_STROKE_PREVIEW_SIZE.1,
            &device,
            &queue,
            &CanvasResources {
                foreground_color: Vec4::ONE,
                background_color: Vec4::ZERO,
            },
        )
        .await?;

        let img = readback_preview(&device, &queue, texture).await;
        let img = img.logged_err().unwrap_or_else(|_| {
            RgbaImage::new(CACHED_STROKE_PREVIEW_SIZE.0, CACHED_STROKE_PREVIEW_SIZE.1)
        });
        Ok(img)
    }
    .boxed()
}

async fn readback_preview(device: &Device, queue: &Queue, texture: Texture) -> Result<RgbaImage> {
    let mut ec = device.create_command_encoder(&Default::default());
    let staging = create_readback_buffer_and_schedule_copy_texture(device, &mut ec, &texture);
    let mut readback = Some(readback_buffer_raw_on_submit_async(&mut ec, &staging, ..));
    let si = queue.submit([ec.finish()]);

    let width = texture.width();
    let height = texture.height();

    let _ = device.poll_indefinitely_for(si);

    let readback = readback
        .take()
        .expect("stroke preview readback task must only run once");
    let rgba_bytes = readback.into_inner().await??;
    RgbaImage::from_raw(width, height, rgba_bytes)
        .ok_or_else(|| anyhow!("Unable to create preview image."))
}

pub fn predefined_curve_samples(width: u32, height: u32) -> [RawPenInput; 32] {
    from_fn(|i| {
        let t = i as f32 / 31.0;
        let azimuth = t * TAU;
        let altitude = (30.0 + 30.0 * t).to_radians();
        let tan_altitude = altitude.tan();

        RawPenInput {
            position: Vec2::new(
                width as f32 * t,
                height as f32 * (0.5 + 0.25 * (t * TAU).sin()),
            ),
            pressure: t,
            tilt: Vec2::new(
                (azimuth.cos() / tan_altitude).atan(),
                (azimuth.sin() / tan_altitude).atan(),
            ),
            angle: Vec2::new(altitude, azimuth),
            time: Time {
                now: t,
                stroke_begin: 0.0,
            },
        }
    })
}

pub async fn create_stroke_preview_with(
    brush: &BrushPresetInstance,
    samples: &[RawPenInput],
    width: u32,
    height: u32,
    device: &Device,
    queue: &Queue,
    canvas_resources: &CanvasResources,
) -> Result<Texture> {
    let target_layer = DynamicLayerStorage::new(
        device.clone(),
        queue.clone(),
        GpuLayerInfo {
            texel_type: TexelType::RGBA8,
        },
    );
    create_stroke_preview_on_target_with(
        brush,
        samples,
        width,
        height,
        device,
        queue,
        canvas_resources,
        target_layer,
    )
    .await
}

pub async fn create_stroke_preview_on_target(
    brush: &BrushPresetInstance,
    samples: &[RawPenInput],
    width: u32,
    height: u32,
    globals: &Globals,
    canvas_resources: &CanvasResources,
    target_layer: DynamicLayerStorage,
) -> Result<Texture> {
    create_stroke_preview_on_target_with(
        brush,
        samples,
        width,
        height,
        globals.render_device(),
        globals.render_queue(),
        canvas_resources,
        target_layer,
    )
    .await
}

pub async fn create_stroke_preview_on_target_with(
    brush: &BrushPresetInstance,
    samples: &[RawPenInput],
    width: u32,
    height: u32,
    device: &Device,
    queue: &Queue,
    canvas_resources: &CanvasResources,
    mut target_layer: DynamicLayerStorage,
) -> Result<Texture> {
    ensure!(
        samples.len() >= 2,
        "stroke preview requires at least two samples"
    );
    ensure!(
        width > 0 && height > 0,
        "stroke preview dimensions must be non-zero"
    );

    let mut selection_layer = DynamicLayerStorage::new(
        device.clone(),
        queue.clone(),
        GpuLayerInfo {
            texel_type: TexelType::A8,
        },
    );

    let foreground_color = {
        let mut buffer = StorageBuffer::from(canvas_resources.foreground_color);
        buffer.set_label(Some("brush preview foreground color"));
        buffer.write_buffer(device, queue);
        buffer
    };
    let background_color = {
        let mut buffer = StorageBuffer::from(canvas_resources.background_color);
        buffer.set_label(Some("brush preview background color"));
        buffer.write_buffer(device, queue);
        buffer
    };

    let compiled = brush.compile(
        target_layer.layer_info().texel_type,
        selection_layer.layer_info().texel_type,
        device,
        queue,
    )?;
    let mut renderer = BrushPresetRenderer::new(
        compiled,
        target_layer.layer_info().texel_type,
        selection_layer.layer_info().texel_type,
        device,
        queue,
        &foreground_color,
        &background_color,
    );

    let mut input_processor = InputProcessor::new(256, Box::new(BasicStabilizer));

    // Ensure both layers have at least a null tile so binding() works without
    // the app-global empty-layer bindings.
    target_layer.get_tile_or_allocate(GpuTileInfo::NULL.index);
    selection_layer.get_tile_or_allocate(GpuTileInfo::NULL.index);

    let worker = renderer.begin(
        device,
        queue,
        target_layer.binding_or_empty(),
        selection_layer.binding_or_empty(),
    );

    for sample in samples.iter().take(samples.len() - 1) {
        if let Some(pen_input) = input_processor.push(*sample) {
            renderer.update(pen_input);
        }
    }

    for pen_input in input_processor.flush(*samples.last().unwrap()) {
        renderer.update(pen_input);
    }

    let final_result = renderer.end();
    let device = device.clone();
    let queue = queue.clone();

    if let Some(mut stream) = into_stream(worker) {
        while stream.next().await.is_some() {}
    }

    let mut stream = into_stream(final_result).context("no output from stroke preview renderer")?;
    let Some(Action::Output(result)) = stream.next().await else {
        bail!("stroke preview renderer did not produce an output");
    };
    let texture = map_result_texture(device.clone(), queue.clone(), width, height, result);
    Ok(texture)
}

fn map_result_texture(
    device: Device,
    queue: Queue,
    width: u32,
    height: u32,
    result: DynamicLayerStorage,
) -> Texture {
    let output_texture = device.create_texture(&TextureDescriptor {
        label: Some("stroke preview texture"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::STORAGE_BINDING
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let Some(result_binding) = result.binding() else {
        return output_texture;
    };
    let mut ec = device.create_command_encoder(&Default::default());
    let result_bounds = LayerBoundsPipeline::new(&device, TexelType::RGBA8, false).dispatch(
        &device,
        &queue,
        &mut ec,
        &result_binding,
        None,
    );
    queue.submit([ec.finish()]);

    ComposeStrokePreviewPipeline::new(&device).dispatch(
        &device,
        &queue,
        &result_binding,
        &result_bounds,
        &output_texture,
    );

    output_texture
}

struct ComposeStrokePreviewPipeline {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl ComposeStrokePreviewPipeline {
    pub fn new(device: &Device) -> Self {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("compose stroke preview bind group layout"),
            entries: BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    binding_types::texture_storage_2d_array(
                        TexelType::RGBA8.wgpu_format(),
                        StorageTextureAccess::ReadOnly,
                    ),
                    binding_types::storage_buffer_read_only::<GpuTileInfo>(false),
                    binding_types::storage_buffer_read_only::<IVec4>(false),
                    binding_types::texture_storage_2d(
                        TextureFormat::Rgba8Unorm,
                        StorageTextureAccess::WriteOnly,
                    ),
                ),
            )
            .as_ref(),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("compose stroke preview pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("compose stroke preview shader"),
            source: ShaderSource::Wgsl(include_wesl!("compose_stroke_preview").into()),
        });
        let pipeline = device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some("compose stroke preview pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self { layout, pipeline }
    }

    fn dispatch(
        &self,
        device: &Device,
        queue: &Queue,
        result: &LayerBinding,
        result_bounds: &Buffer,
        output: &Texture,
    ) {
        let output_view = output.create_view(&TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("compose stroke preview bind group"),
            layout: &self.layout,
            entries: BindGroupEntries::sequential((
                &result.texture,
                result.tile_info_buffer.as_entire_binding(),
                result_bounds.as_entire_binding(),
                &output_view,
            ))
            .as_ref(),
        });

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("compose stroke preview pass"),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(output.width().div_ceil(16), output.height().div_ceil(16), 1);
        }
        queue.submit([encoder.finish()]);
    }
}
