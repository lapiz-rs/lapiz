use std::{
    collections::HashMap,
    f32::consts::{SQRT_2, TAU},
    sync::Arc,
};

use glam::{Mat2, Vec2};
use iced_core::{Color, Point, Rectangle};
use iced_runtime::Task;
use iced_wgpu::Primitive;
use lapiz_render::{
    bind_group_layout_entries::{BindGroupLayoutEntries, binding_types},
    render_context::RenderContextAppExt as _,
    uniform_buffer::UniformBuffer,
};
use lapiz_runtime::global::Globals;
use lapiz_widgets::shader;
use moxcms::ColorProfile;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    Device, IndexFormat, Queue, RenderPass, ShaderStages, TextureFormat,
};

use crate::{
    BarState, ColorModel, ColorSelectorMessage, ColorSelectorState, GRADIENT_RING_GAP,
    GradientPlaneShape, PlaneState,
    config::{GradientPlaneConfig, GradientPlaneFlipAxis},
    pipeline::{GradientMesh, GradientPipeline, GradientRingPipeline, GradientSettings},
};

fn unmap_normalized(value: f32, range: Vec2) -> f32 {
    let width = range.y - range.x;
    if width.abs() <= f32::EPSILON {
        0.5
    } else {
        (value - range.x) / width
    }
}

fn plane_scale(config: &GradientPlaneConfig, size: f32) -> f32 {
    if !config.show_primary_channel_ring {
        return 1.0;
    }

    let antialias_width = 1.0 / size;
    let inner_radius =
        (0.5 - antialias_width - (config.primary_channel_ring_width + GRADIENT_RING_GAP) / size)
            .max(0.0);
    let circumradius = match config.shape {
        GradientPlaneShape::Square => SQRT_2,
        GradientPlaneShape::Triangle => 1.0,
    };
    2.0 * inner_radius / circumradius
}

fn plane_position_to_uv(shape: GradientPlaneShape, position: Vec2) -> Vec2 {
    match shape {
        GradientPlaneShape::Square => (position + Vec2::ONE) * 0.5,
        GradientPlaneShape::Triangle => {
            let y = (1.0 - position.y) / 1.5;
            let x = 0.5 + position.x / (3.0_f32.sqrt() * y.max(f32::EPSILON));
            Vec2::new(x, y)
        }
    }
}

fn plane_uv_to_position(shape: GradientPlaneShape, uv: Vec2) -> Vec2 {
    match shape {
        GradientPlaneShape::Square => uv * 2.0 - Vec2::ONE,
        GradientPlaneShape::Triangle => {
            Vec2::new(3.0_f32.sqrt() * uv.y * (uv.x - 0.5), 1.0 - 1.5 * uv.y)
        }
    }
}

impl ColorSelectorState {
    pub(crate) fn plane_uv_from_window_position(
        &self,
        index: usize,
        position: Point,
    ) -> Option<(Vec2, bool)> {
        let config = self.presets.get(self.selected_preset)?.planes.get(index)?;
        let bounds = self.planes.get(index)?.bounds;
        let width = bounds.width;
        let height = bounds.height;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }

        let output_position = Vec2::new(
            2.0 * (position.x - bounds.x) / width - 1.0,
            1.0 - 2.0 * (position.y - bounds.y) / height,
        );
        let mut input_position = Mat2::from_angle(config.rotation) * output_position;
        if config.flip_axis.contains(GradientPlaneFlipAxis::X) {
            input_position.x = -input_position.x;
        }
        if config.flip_axis.contains(GradientPlaneFlipAxis::Y) {
            input_position.y = -input_position.y;
        }

        let scale = plane_scale(config, width);
        if scale <= f32::EPSILON {
            return None;
        }
        input_position /= scale;
        let uv = plane_position_to_uv(config.shape, input_position);
        let clamped = uv.clamp(Vec2::ZERO, Vec2::ONE);
        Some((clamped, (uv - clamped).length_squared() <= 1e-6))
    }

    pub(crate) fn plane_indicator_position(&self, index: usize) -> Option<Vec2> {
        let config = self.presets.get(self.selected_preset)?.planes.get(index)?;
        let size = self.planes.get(index)?.size;
        let channels = config.model.channels(self.color, &self.profile);
        let ranges = config.model.channel_ranges();
        let variable_channels = self.plane_variable_channels(index, config);
        let mut uv = Vec2::ZERO;
        let mut variable_index = 0;
        for channel in 0..3 {
            if variable_channels & (1 << channel) != 0 {
                uv[variable_index] = ((channels[channel] - ranges[channel].x)
                    / (ranges[channel].y - ranges[channel].x))
                    .clamp(0.0, 1.0);
                variable_index += 1;
            }
        }
        let (x_range, y_range) = self.plane_normalized_ranges(index);
        uv.x = unmap_normalized(uv.x, x_range);
        uv.y = unmap_normalized(uv.y, y_range);
        uv = uv.clamp(Vec2::ZERO, Vec2::ONE);

        let mut position = plane_uv_to_position(config.shape, uv) * plane_scale(config, size);
        if config.flip_axis.contains(GradientPlaneFlipAxis::X) {
            position.x = -position.x;
        }
        if config.flip_axis.contains(GradientPlaneFlipAxis::Y) {
            position.y = -position.y;
        }
        position = Mat2::from_angle(-config.rotation) * position;
        Some(Vec2::new(
            (position.x + 1.0) * 0.5,
            (1.0 - position.y) * 0.5,
        ))
    }

    pub(crate) fn ring_indicator_position(&self, index: usize) -> Option<Vec2> {
        let config = self.presets.get(self.selected_preset)?.planes.get(index)?;
        if !config.show_primary_channel_ring {
            return None;
        }
        let size = self.planes.get(index)?.size;
        let channel = self.plane_primary_channel(index, config) as usize;
        let channels = config.model.channels(self.color, &self.profile);
        let range = config.model.channel_ranges()[channel];
        let factor = ((channels[channel] - range.x) / (range.y - range.x)).clamp(0.0, 1.0);
        let angle = if config.reversed_ring {
            -factor * TAU - config.ring_rotation
        } else {
            factor * TAU - config.ring_rotation
        };
        let antialias_width = 1.0 / size;
        let outer_radius = 0.5 - antialias_width;
        let inner_radius = (outer_radius - config.primary_channel_ring_width / size).max(0.0);
        let radius = (inner_radius + outer_radius) * 0.5;
        Some(Vec2::new(
            0.5 + angle.cos() * radius,
            0.5 - angle.sin() * radius,
        ))
    }

    pub(crate) fn bar_indicator_position(&self, index: usize) -> Option<f32> {
        let config = self.presets.get(self.selected_preset)?.bars.get(index)?;
        let channels = config.model.channels(self.color, &self.profile);
        let range = config.model.channel_ranges()[config.channel as usize];
        Some(((channels[config.channel as usize] - range.x) / (range.y - range.x)).clamp(0.0, 1.0))
    }

    pub(crate) fn indicator_color(&self) -> Color {
        let value = ColorModel::Gray.channels(self.color, &self.profile).x;
        if value > 0.5 {
            Color::from_rgb8(0x00, 0x00, 0x00)
        } else {
            Color::from_rgb8(0xff, 0xff, 0xff)
        }
    }

    pub(crate) fn rebuild_plane_state(&mut self, globals: &Globals) {
        let Some(preset) = self.presets.get(self.selected_preset) else {
            self.planes.clear();
            return;
        };
        let device = globals.render_device();
        self.planes = preset
            .planes
            .iter()
            .map(|config| PlaneState {
                mesh: Arc::new(GradientMesh::new_plane(
                    device,
                    config.shape,
                    plane_scale(config, 0.0),
                )),
                size: 0.0,
                bounds: Rectangle::default(),
                ranges: (Vec2::new(0.0, 1.0), Vec2::new(0.0, 1.0)),
                primary_channel_override: None,
            })
            .collect();
    }

    pub(crate) fn rebuild_bar_states(&mut self, globals: &Globals) {
        let Some(preset) = self.presets.get(self.selected_preset) else {
            self.bars.clear();
            return;
        };
        let device = globals.render_device();
        self.bars = preset
            .bars
            .iter()
            .map(|_| BarState {
                mesh: Arc::new(GradientMesh::new_bar(device)),
                bounds: Rectangle::default(),
            })
            .collect();
    }

    pub(crate) fn update_plane_bounds(&mut self, index: usize, bounds: Rectangle, device: &Device) {
        let size = bounds.width.min(bounds.height).round().max(1.0);

        let Some(plane) = self.planes.get_mut(index) else {
            return;
        };
        if plane.bounds == bounds {
            return;
        }
        plane.bounds = bounds;
        if plane.size == size {
            return;
        }

        let config = &self.presets[self.selected_preset].planes[index];
        plane.mesh = Arc::new(GradientMesh::new_plane(
            device,
            config.shape,
            plane_scale(config, size),
        ));
        plane.size = size;
        plane.ranges = (Vec2::new(0.0, 1.0), Vec2::new(0.0, 1.0));
    }

    pub(crate) fn refresh_clip_bounds(&self, globals: &Globals) -> Task<ColorSelectorMessage> {
        let preset = &self.presets[self.selected_preset];
        if !preset.clip_to_gamut {
            return Task::none();
        }

        let device = globals.render_device().clone();
        let queue = globals.render_queue().clone();

        let mut tasks = Vec::new();

        for (index, (config, plane)) in preset.planes.iter().zip(&self.planes).enumerate() {
            let settings = GradientSettings::new_plane(
                preset.out_of_gamut_color,
                preset.use_out_of_gamut_color,
                config.model.channels(self.color, &self.profile),
                config,
                plane.primary_channel_override,
                plane.size,
            );

            let readback = self
                .compute_bounds_pipeline
                .compute(&device, &queue, &settings);
            tasks.push(
                Task::future(async move {
                    let bounds = readback.into_inner().await.ok()?.ok()?;
                    let (x_range, y_range) = bounds
                        .normalized_ranges()
                        .unwrap_or((Vec2::new(0.0, 1.0), Vec2::new(0.0, 1.0)));
                    Some(ColorSelectorMessage::ClipBoundsComputed {
                        index,
                        x_range,
                        y_range,
                    })
                })
                .and_then(Task::done),
            );
        }

        Task::batch(tasks)
    }

    pub(crate) fn update_bar_bounds(&mut self, index: usize, bounds: Rectangle) {
        let Some(bar) = self.bars.get_mut(index) else {
            return;
        };
        if bar.bounds == bounds {
            return;
        }
        bar.bounds = bounds;
    }
}

#[derive(Debug)]
pub(crate) struct SurfaceDrawData {
    pub(crate) id: u64,
    pub(crate) mesh: Arc<GradientMesh>,
    pub(crate) settings: GradientSettings,
    pub(crate) ring_settings: Option<GradientSettings>,
    pub(crate) profile: Arc<ColorProfile>,
    pub(crate) output_profile: Arc<ColorProfile>,
    // TODO Remove this once lapiz_image uses Arc<ColorProfile> for image profile
    pub(crate) output_profile_version: u64,
}

#[derive(Debug)]
pub(crate) struct GradientDrawPrimitive {
    pub(crate) data: Arc<SurfaceDrawData>,
}

impl Primitive for GradientDrawPrimitive {
    type Pipeline = GradientDirectPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &Device,
        queue: &Queue,
        _bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        pipeline.prepare(device, queue, &self.data);
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut RenderPass<'_>) -> bool {
        let data = &self.data;
        let Some(instance) = pipeline.instances.get(&data.id) else {
            return false;
        };
        let Some(gradient) = &pipeline.gradient else {
            return false;
        };

        render_pass.set_pipeline(&gradient.pipeline);
        render_pass.set_bind_group(0, &instance.bind_group, &[]);
        render_pass.set_vertex_buffer(0, data.mesh.vertices.slice(..));
        render_pass.set_index_buffer(data.mesh.indices.slice(..), IndexFormat::Uint16);
        render_pass.draw_indexed(0..data.mesh.n_indices, 0, 0..1);

        if let Some(ring_bind_group) = &instance.ring_bind_group
            && let Some(ring) = &pipeline.ring
        {
            render_pass.set_pipeline(&ring.pipeline);
            render_pass.set_bind_group(0, ring_bind_group, &[]);
            render_pass.set_vertex_buffer(0, ring.mesh.vertices.slice(..));
            render_pass.set_index_buffer(ring.mesh.indices.slice(..), IndexFormat::Uint16);
            render_pass.draw_indexed(0..ring.mesh.n_indices, 0, 0..1);
        }

        true
    }
}

struct Instance {
    settings_buffer: UniformBuffer<GradientSettings>,
    bind_group: BindGroup,
    ring_settings_buffer: Option<UniformBuffer<GradientSettings>>,
    ring_bind_group: Option<BindGroup>,
}

pub(crate) struct GradientDirectPipeline {
    format: TextureFormat,
    layout: BindGroupLayout,
    gradient: Option<GradientPipeline>,
    ring: Option<GradientRingPipeline>,
    profile_version: u64,
    instances: HashMap<u64, Instance>,
}

impl shader::Pipeline for GradientDirectPipeline {
    fn new(device: &Device, _queue: &Queue, format: TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("gradient settings layout"),
            entries: BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (binding_types::uniform_buffer::<GradientSettings>(false),),
            )
            .as_ref(),
        });

        Self {
            format,
            layout,
            gradient: None,
            ring: None,
            profile_version: 0,
            instances: HashMap::new(),
        }
    }

    fn trim(&mut self) {
        self.instances.clear();
    }
}

impl GradientDirectPipeline {
    fn prepare(&mut self, device: &Device, queue: &Queue, data: &SurfaceDrawData) {
        if self.gradient.is_none() || self.profile_version != data.output_profile_version {
            self.gradient = Some(GradientPipeline::new(
                device,
                &self.layout,
                &data.profile,
                &data.output_profile,
                self.format,
            ));
            self.ring = Some(GradientRingPipeline::new(
                device,
                &self.layout,
                &data.profile,
                &data.output_profile,
                self.format,
            ));
            self.profile_version = data.output_profile_version;
        }

        let instance = self.instances.entry(data.id).or_insert_with(|| {
            let mut settings_buffer = UniformBuffer::from(data.settings.clone());
            settings_buffer.set_label(Some("gradient_settings_uniform"));
            settings_buffer.write_buffer(device, queue);

            let bind_group = device.create_bind_group(&BindGroupDescriptor {
                label: Some("gradient settings bind group"),
                layout: &self.layout,
                entries: &[BindGroupEntry {
                    binding: 0,
                    resource: settings_buffer.binding().unwrap(),
                }],
            });

            let (ring_settings_buffer, ring_bind_group) = data
                .ring_settings
                .as_ref()
                .map(|buffer| {
                    let mut ring_settings_buffer = UniformBuffer::from(buffer.clone());
                    ring_settings_buffer.set_label(Some("gradient_ring_settings_uniform"));
                    ring_settings_buffer.write_buffer(device, queue);

                    let ring_bind_group = device.create_bind_group(&BindGroupDescriptor {
                        label: Some("gradient ring settings bind group"),
                        layout: &self.layout,
                        entries: &[BindGroupEntry {
                            binding: 0,
                            resource: ring_settings_buffer.binding().unwrap(),
                        }],
                    });
                    (ring_settings_buffer, ring_bind_group)
                })
                .unzip();

            Instance {
                settings_buffer,
                bind_group,
                ring_settings_buffer,
                ring_bind_group,
            }
        });

        instance.settings_buffer.set(data.settings.clone());
        instance.settings_buffer.write_buffer(device, queue);

        if let (Some(buffer), Some(settings)) =
            (&mut instance.ring_settings_buffer, &data.ring_settings)
        {
            buffer.set(settings.clone());
            buffer.write_buffer(device, queue);
        }
    }
}
