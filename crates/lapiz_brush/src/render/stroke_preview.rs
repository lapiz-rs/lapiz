use std::{
    f32::consts::TAU,
    fs::{self, File},
};

use anyhow::{Result, anyhow, ensure};
use glam::{IVec4, Vec2, Vec4};
use iced_runtime::Task;
use image::{ImageFormat, RgbaImage};
use lapiz_assets::asset::AssetHandle;
use lapiz_image::{
    layer_bounds::LayerBoundsPipeline,
    texel::TexelType,
    tile::{DynamicLayerStorage, GpuLayerInfo, GpuTileInfo, LayerBinding},
};
use lapiz_render::{
    bind_group_entries::BindGroupEntries,
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    buffer::DynamicBuffer,
    readback::{
        create_readback_buffer_and_schedule_copy_texture, readback_buffer_raw_on_submit_async,
    },
    render_context::RenderContextAppExt,
    util::DevicePollExt,
};
use lapiz_runtime::Services;
use lapiz_shader_graph::graph::{
    function::ASSET_GRAPH_FUNCTION_STORAGE, texture::ASSET_GRAPH_TEXTURE_STORAGE,
};
use lapiz_utils::log_err::LogErr;
use tracing::info;
use wesl::include_wesl;
use wgpu::{
    BindGroupDescriptor, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, BufferUsages,
    ComputePassDescriptor, ComputePipeline, ComputePipelineDescriptor, Device, Extent3d,
    PipelineLayoutDescriptor, Queue, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StorageTextureAccess, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureViewDescriptor,
};

use crate::{
    asset::BrushPreset,
    input_processing::{BasicStabilizer, InputProcessor, RawPenInput},
    instance::BrushPresetInstance,
    render::{BrushPresetRenderer, EXTERNAL_VARIABLE_BASE_BINDING, Time, graph::CanvasResources},
};

pub const CACHED_STROKE_PREVIEW_SIZE: (u32, u32) = (512, 256);

pub fn load_cached_stroke_preview_or_generate(
    brush: &AssetHandle<BrushPreset>,
    services: &Services,
) -> Result<Task<Result<RgbaImage>>> {
    // TODO resolve path at a dedicated module
    let cache_dir = std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow!("Unable to resolve the executable directory."))?
        .join("cache");
    let cache_path = cache_dir.join(format!("preview-{}.png", brush.id()));

    if cache_path.exists()
        && let Ok(img) = image::open(&cache_path).logged_err()
    {
        info!("Loaded cached stroke preview from {}", cache_path.display());
        return Ok(Task::done(Ok(img.into_rgba8())));
    }

    info!("Generating stroke preview for brush {}", brush.id());
    fs::create_dir_all(&cache_dir)?;

    let (instance, errs) = BrushPresetInstance::from_asset(
        brush,
        ASSET_GRAPH_TEXTURE_STORAGE.clone(),
        ASSET_GRAPH_FUNCTION_STORAGE.clone(),
    );

    let Some(instance) = instance else {
        return Err(anyhow::anyhow!(
            "Failed to create brush preset instance: {:?}",
            errs
        ));
    };

    let texture = create_stroke_preview(
        &instance,
        &predefined_curve_samples(CACHED_STROKE_PREVIEW_SIZE.0, CACHED_STROKE_PREVIEW_SIZE.1),
        CACHED_STROKE_PREVIEW_SIZE.0,
        CACHED_STROKE_PREVIEW_SIZE.1,
        services,
        &CanvasResources {
            foreground_color: Vec4::ONE,
            background_color: Vec4::ZERO,
        },
    )?;

    let device = services.render_device().clone();
    let queue = services.render_queue().clone();

    Ok(texture
        .then(move |texture| readback_preview(device.clone(), queue.clone(), texture))
        .map(move |img| {
            let img = img.logged_err().unwrap_or_else(|_| {
                RgbaImage::new(CACHED_STROKE_PREVIEW_SIZE.0, CACHED_STROKE_PREVIEW_SIZE.1)
            });
            let mut file = File::create(&cache_path)?;
            img.write_to(&mut file, ImageFormat::Png)?;
            info!("Stroke preview saved to {}", cache_path.display());
            Ok(img)
        }))
}

fn readback_preview(device: Device, queue: Queue, texture: Texture) -> Task<Result<RgbaImage>> {
    let mut ec = device.create_command_encoder(&Default::default());
    let staging = create_readback_buffer_and_schedule_copy_texture(&device, &mut ec, &texture);
    let mut readback = Some(readback_buffer_raw_on_submit_async(&mut ec, &staging, ..));
    let si = queue.submit([ec.finish()]);

    let width = texture.width();
    let height = texture.height();

    Task::future(async move {
        let _ = device.poll_indefinitely_for(si);
    })
    .then(move |_| {
        let readback = readback
            .take()
            .expect("stroke preview readback task must only run once");
        Task::future(async move {
            let rgba_bytes = readback.into_inner().await??;
            RgbaImage::from_raw(width, height, rgba_bytes)
                .ok_or_else(|| anyhow!("Unable to create preview image."))
        })
    })
}

pub fn predefined_curve_samples(width: u32, height: u32) -> [RawPenInput; 32] {
    std::array::from_fn(|i| {
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

pub fn create_stroke_preview(
    brush: &BrushPresetInstance,
    samples: &[RawPenInput],
    width: u32,
    height: u32,
    services: &Services,
    canvas_resources: &CanvasResources,
) -> Result<Task<Texture>> {
    let target_layer = DynamicLayerStorage::new(
        services.render_device().clone(),
        services.render_queue().clone(),
        GpuLayerInfo {
            texel_type: TexelType::RGBA8,
        },
    );
    create_stroke_preview_on_target(
        brush,
        samples,
        width,
        height,
        services,
        canvas_resources,
        target_layer,
    )
}

pub fn create_stroke_preview_on_target(
    brush: &BrushPresetInstance,
    samples: &[RawPenInput],
    width: u32,
    height: u32,
    services: &Services,
    canvas_resources: &CanvasResources,
    target_layer: DynamicLayerStorage,
) -> Result<Task<Texture>> {
    ensure!(
        samples.len() >= 2,
        "stroke preview requires at least two samples"
    );
    ensure!(
        width > 0 && height > 0,
        "stroke preview dimensions must be non-zero"
    );

    let device = services.render_device();
    let queue = services.render_queue();
    let selection_layer = DynamicLayerStorage::new(
        device.clone(),
        queue.clone(),
        GpuLayerInfo {
            texel_type: TexelType::A8,
        },
    );

    let canvas_resources = {
        let mut b = DynamicBuffer::new(
            Some("canvas_resources_buffer".into()),
            BufferUsages::STORAGE,
        );
        b.push(canvas_resources);
        b.write_buffer(device, queue);
        b
    };

    let compiled = brush.compile(EXTERNAL_VARIABLE_BASE_BINDING)?;
    let mut renderer = BrushPresetRenderer::new(
        &compiled,
        target_layer.layer_info().texel_type,
        selection_layer.layer_info().texel_type,
        services,
        &canvas_resources,
    );

    let mut input_processor = InputProcessor::new(256, Box::new(BasicStabilizer));

    renderer.begin(
        device,
        queue,
        target_layer.binding_or_empty(),
        selection_layer.binding_or_empty(),
    );

    let mut render_tasks = Vec::new();

    for sample in samples.iter().take(samples.len() - 1) {
        if let Some(pen_input) = input_processor.push(*sample) {
            render_tasks.push(renderer.update(device, queue, pen_input));
        }
    }

    for pen_input in input_processor.flush(*samples.last().unwrap()) {
        render_tasks.push(renderer.update(device, queue, pen_input));
    }

    let final_result = renderer.end(device, queue);
    let device = device.clone();
    let queue = queue.clone();

    Ok(Task::batch(render_tasks)
        .discard()
        .chain(final_result.map(move |result| {
            map_result_texture(device.clone(), queue.clone(), width, height, result)
        })))
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
