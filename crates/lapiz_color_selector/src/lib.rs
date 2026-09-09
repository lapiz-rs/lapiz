use std::sync::Arc;

use glam::{Vec2, Vec3};
use iced_core::{Element, Length, Point, Rectangle, Theme};
use iced_runtime::Task;
use iced_wgpu::Renderer;
use iced_widget::{Row, column, text};
use lapiz_color::{
    Color,
    model::{
        gray::Gray, hsl::Hsl, hsv::Hsv, lab::Lab, lch::Lch, okhsl::OkHsl, okhsv::OkHsv,
        oklab::OkLab, oklch::OkLch, rgb::Rgb, xyz::Xyz,
    },
    platform,
};
use lapiz_render::render_context::RenderContextAppExt;
use lapiz_runtime::{Application, Services, plugin::Plugin};
use lapiz_widgets::{fluent_builder::When, radio::Radio, spin_slider::SpinSlider};
use moxcms::ColorProfile;
use parse_display::Display;

use crate::{
    config::{ColorSelectorConfig, GradientBarConfig, GradientPlaneConfig},
    control::{ActiveSelection, GradientSurface, PlaneRow, SurfaceTarget},
    pipeline::{ComputeBoundsPipeline, GradientMesh, GradientSettings},
    render::SurfaceDrawData,
};

lapiz_i18n::define_i18n!("color_selector");

pub mod config;
mod control;
mod pipeline;
mod render;

pub struct ColorSelectorPlugin;

impl Plugin for ColorSelectorPlugin {
    fn build(&self, _app: &mut Application) {
        i18n::init();
    }
}

const GRADIENT_RING_GAP: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[display(style = "snake_case")]
#[repr(u32)]
pub enum GradientPlaneShape {
    Square,
    Triangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[repr(u32)]
pub enum ColorModel {
    #[display("Gray")]
    Gray,
    #[display("HSL")]
    Hsl,
    #[display("HSV")]
    Hsv,
    #[display("Lab")]
    Lab,
    #[display("LCh")]
    Lch,
    #[display("OkHSL")]
    OkHsl,
    #[display("OkHSV")]
    OkHsv,
    #[display("OkLab")]
    OkLab,
    #[display("OkLCh")]
    OkLch,
    #[display("RGB")]
    Rgb,
    #[display("XYZ")]
    Xyz,
}

impl ColorModel {
    pub const ALL: [Self; 11] = [
        Self::Gray,
        Self::Hsl,
        Self::Hsv,
        Self::Lab,
        Self::Lch,
        Self::OkHsl,
        Self::OkHsv,
        Self::OkLab,
        Self::OkLch,
        Self::Rgb,
        Self::Xyz,
    ];

    pub const PLANE_MODELS: [Self; 10] = [
        Self::Hsl,
        Self::Hsv,
        Self::Lab,
        Self::Lch,
        Self::OkHsl,
        Self::OkHsv,
        Self::OkLab,
        Self::OkLch,
        Self::Rgb,
        Self::Xyz,
    ];

    pub const fn channel_labels(self) -> &'static [&'static str] {
        match self {
            Self::Gray => &["V"],
            Self::Hsl => &["H", "S", "L"],
            Self::Hsv => &["H", "S", "V"],
            Self::Lab => &["L", "a", "b"],
            Self::Lch => &["L", "C", "h"],
            Self::OkHsl => &["H", "S", "L"],
            Self::OkHsv => &["H", "S", "V"],
            Self::OkLab => &["L", "a", "b"],
            Self::OkLch => &["L", "C", "h"],
            Self::Rgb => &["R", "G", "B"],
            Self::Xyz => &["X", "Y", "Z"],
        }
    }

    pub(crate) const fn hue_channel(self) -> Option<u8> {
        match self {
            Self::Hsl | Self::Hsv | Self::OkHsl | Self::OkHsv => Some(0),
            Self::Lch | Self::OkLch => Some(2),
            _ => None,
        }
    }

    pub fn channel_ranges(&self) -> [Vec2; 3] {
        match self {
            ColorModel::Gray => [Vec2::new(0.0, 1.0), Vec2::ZERO, Vec2::ZERO],
            ColorModel::Lab => [
                Vec2::new(0.0, 100.0),
                Vec2::new(-128.0, 127.0),
                Vec2::new(-128.0, 127.0),
            ],
            ColorModel::Lch => [
                Vec2::new(0.0, 100.0),
                Vec2::new(0.0, 150.0),
                Vec2::new(0.0, 360.0),
            ],
            ColorModel::Hsl | ColorModel::Hsv | ColorModel::OkHsl | ColorModel::OkHsv => [
                Vec2::new(0.0, 360.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(0.0, 1.0),
            ],
            ColorModel::OkLab => [
                Vec2::new(0.0, 1.0),
                Vec2::new(-0.5, 0.5),
                Vec2::new(-0.5, 0.5),
            ],
            ColorModel::OkLch => [
                Vec2::new(0.0, 1.0),
                Vec2::new(0.0, 0.4),
                Vec2::new(0.0, 360.0),
            ],
            ColorModel::Rgb => [Vec2::new(0.0, 1.0); 3],
            ColorModel::Xyz => [
                Vec2::new(0.0, 1.5),
                Vec2::new(0.0, 1.0),
                Vec2::new(0.0, 1.5),
            ],
        }
    }

    pub const fn display_scale(&self) -> &'static [f32] {
        match self {
            Self::Gray => &[100.0],
            Self::Hsl | Self::Hsv | Self::OkHsl | Self::OkHsv => &[1.0, 100.0, 100.0],
            Self::Lab => &[1.0, 100.0 / 128.0, 100.0 / 128.0],
            Self::Lch => &[1.0, 1.0, 1.0],
            Self::OkLab => &[100.0, 200.0, 200.0],
            Self::OkLch => &[100.0, 100.0, 1.0],
            Self::Rgb => &[100.0, 100.0, 100.0],
            Self::Xyz => &[100.0 / 1.5, 100.0, 100.0 / 1.5],
        }
    }

    pub fn channels(self, color: Color, profile: &ColorProfile) -> Vec3 {
        let rgb_to_xyz = profile.rgb_to_xyz_matrix().to_f32();
        let xyz = match color {
            Color::Gray(gray) => gray.into_xyz(),
            Color::Hsl(hsl) => hsl.into_rgb().into_xyz(rgb_to_xyz),
            Color::Hsv(hsv) => hsv.into_rgb().into_xyz(rgb_to_xyz),
            Color::Lab(lab) => lab.into_xyz(),
            Color::Lch(lch) => lch.into_xyz(),
            Color::OkHsl(ok_hsl) => ok_hsl.into_xyz(),
            Color::OkHsv(ok_hsv) => ok_hsv.into_xyz(),
            Color::OkLab(ok_lab) => ok_lab.into_xyz(),
            Color::OkLch(ok_lch) => ok_lch.into_xyz(),
            Color::Rgb(rgb) => rgb.into_xyz(rgb_to_xyz),
            Color::Xyz(xyz) => xyz,
        };

        match self {
            Self::Gray => {
                let gray = Gray::from_xyz(xyz);
                Vec3::new(gray.v, 0.0, 0.0)
            }
            Self::Hsl => {
                let hsl = Hsl::from_rgb(Rgb::from_xyz(xyz, rgb_to_xyz.inverse()));
                Vec3::new(hsl.h, hsl.s, hsl.l)
            }
            Self::Hsv => {
                let hsv = Hsv::from_rgb(Rgb::from_xyz(xyz, rgb_to_xyz.inverse()));
                Vec3::new(hsv.h, hsv.s, hsv.v)
            }
            Self::Lab => {
                let lab = Lab::from_xyz(xyz);
                Vec3::new(lab.l, lab.a, lab.b)
            }
            Self::Lch => {
                let lch = Lch::from_xyz(xyz);
                Vec3::new(lch.l, lch.c, lch.h)
            }
            Self::OkHsl => {
                let okhsl = OkHsl::from_xyz(xyz);
                Vec3::new(okhsl.h, okhsl.s, okhsl.l)
            }
            Self::OkHsv => {
                let okhsv = OkHsv::from_xyz(xyz);
                Vec3::new(okhsv.h, okhsv.s, okhsv.v)
            }
            Self::OkLab => {
                let oklab = OkLab::from_xyz(xyz);
                Vec3::new(oklab.l, oklab.a, oklab.b)
            }
            Self::OkLch => {
                let oklch = OkLch::from_xyz(xyz);
                Vec3::new(oklch.l, oklch.c, oklch.h)
            }
            Self::Rgb => {
                let rgb = Rgb::from_xyz(xyz, rgb_to_xyz.inverse());
                Vec3::new(rgb.r, rgb.g, rgb.b)
            }
            Self::Xyz => Vec3::new(xyz.x, xyz.y, xyz.z),
        }
    }

    pub fn color_from_channels(self, channels: Vec3) -> Color {
        match self {
            Self::Gray => Color::Gray(Gray::new(channels.x)),
            Self::Hsl => Color::Hsl(Hsl::new(channels.x, channels.y, channels.z)),
            Self::Hsv => Color::Hsv(Hsv::new(channels.x, channels.y, channels.z)),
            Self::Lab => Color::Lab(Lab::new(channels.x, channels.y, channels.z)),
            Self::Lch => Color::Lch(Lch::new(channels.x, channels.y, channels.z)),
            Self::OkHsl => Color::OkHsl(OkHsl::new(channels.x, channels.y, channels.z)),
            Self::OkHsv => Color::OkHsv(OkHsv::new(channels.x, channels.y, channels.z)),
            Self::OkLab => Color::OkLab(OkLab::new(channels.x, channels.y, channels.z)),
            Self::OkLch => Color::OkLch(OkLch::new(channels.x, channels.y, channels.z)),
            Self::Rgb => Color::Rgb(Rgb::new(channels.x, channels.y, channels.z)),
            Self::Xyz => Color::Xyz(Xyz::new(channels.x, channels.y, channels.z)),
        }
    }
}

pub struct ColorSelector<'a, Message> {
    state: &'a ColorSelectorState,
    on_state_message: Box<dyn Fn(ColorSelectorMessage) -> Message>,
}

impl<'a, Message> ColorSelector<'a, Message> {
    pub fn new(
        state: &'a ColorSelectorState,
        on_state_message: impl Fn(ColorSelectorMessage) -> Message + 'static,
    ) -> Self {
        Self {
            state,
            on_state_message: Box::new(on_state_message),
        }
    }
}

impl<'a, Message: 'a> From<ColorSelector<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(value: ColorSelector<'a, Message>) -> Self {
        let state = value.state;
        if state.presets.is_empty() {
            return column!().into();
        }

        let indicator_color = state.indicator_color();
        let preset = &state.presets[state.selected_preset];
        let max_planes_per_row = preset.max_planes_per_row.clamp(1, 5);
        let max_plane_cell_size = preset.max_plane_size.clamp(128, 512) as f32;

        let planes =
            preset
                .planes
                .chunks(max_planes_per_row)
                .enumerate()
                .map(|(row_index, row)| {
                    let surfaces = row
                        .iter()
                        .enumerate()
                        .map(|(column_index, _)| {
                            let index = row_index * max_planes_per_row + column_index;

                            GradientSurface::plane(
                                index,
                                state.plane_surface_data(index),
                                max_plane_cell_size,
                                state
                                    .planes
                                    .get(index)
                                    .map(|p| p.bounds)
                                    .unwrap_or_default(),
                            )
                            .plane_indicator(state.plane_indicator_position(index))
                            .ring_indicator(state.ring_indicator_position(index))
                            .indicator_color(indicator_color)
                        })
                        .collect();
                    PlaneRow::new(surfaces)
                        .spacing(5.0)
                        .max_cell_size(max_plane_cell_size + 5.0)
                        .into()
                });

        let bars = preset.bars.iter().enumerate().map(|(index, config)| {
            let channel = config.channel as usize;
            let label = config.model.channel_labels()[channel];
            let locked = state.bar_primary_channel_locked(config.model, config.channel);

            Row::new()
                .spacing(4)
                .when(config.show_channel_label, |r| r.push(text(label).width(12)))
                .push(
                    GradientSurface::bar(
                        index,
                        state.bar_surface_data(index),
                        config.bar_height.clamp(10.0, 40.0),
                        state.bars.get(index).map(|b| b.bounds).unwrap_or_default(),
                    )
                    .bar_indicator(state.bar_indicator_position(index).unwrap_or(-1.0))
                    .indicator_color(indicator_color),
                )
                .when(config.show_precise_spin_box, |r| {
                    let range = config.model.channel_ranges()[channel];
                    let scale = config.model.display_scale()[channel];
                    let value = state.bar_display_value(config.model, config.channel);
                    r.push(
                        SpinSlider::new(range.x * scale..=range.y * scale, value)
                            .on_change(move |value| {
                                ColorSelectorMessage::BarValueChanged(index, value)
                            })
                            .width(90)
                            .precision(2),
                    )
                })
                .when(config.show_primary_channel_lock, |r| {
                    let model = config.model;
                    let channel = config.channel;
                    r.push(Radio::new("", true, locked.then_some(true), move |_| {
                        ColorSelectorMessage::PrimaryChannelLock(model, channel)
                    }))
                })
                .into()
        });

        let presets_selector =
            Row::with_children(state.presets.iter().enumerate().map(|(index, preset)| {
                Radio::new(
                    preset.name.clone(),
                    index,
                    Some(state.selected_preset),
                    ColorSelectorMessage::SwitchPreset,
                )
                .into()
            }))
            .spacing(8);

        let content = column![
            column(planes).spacing(5),
            column(bars).spacing(8).padding(8),
            presets_selector,
        ]
        .width(Length::Fill);

        Element::new(content).map(value.on_state_message)
    }
}

#[derive(Clone)]
pub enum ColorSelectorMessage {
    SurfacePress(SurfaceTarget, Point),
    SurfaceMove(Point),
    SurfaceRelease,
    SurfaceBoundsChanged(SurfaceTarget, Rectangle),
    BarValueChanged(usize, f32),
    SwitchPreset(usize),
    PrimaryChannelLock(ColorModel, u8),
    ClipBoundsComputed {
        index: usize,
        x_range: Vec2,
        y_range: Vec2,
    },
    Changed(Color),
    Confirmed(Color),
}

struct PlaneState {
    mesh: Arc<GradientMesh>,
    size: f32,
    ranges: (Vec2, Vec2),
    bounds: Rectangle,
    primary_channel_override: Option<u8>,
}

struct BarState {
    mesh: Arc<GradientMesh>,
    bounds: Rectangle,
}

pub struct ColorSelectorState {
    color: Color,
    profile: ColorProfile,
    output_profile: ColorProfile,
    output_profile_version: u64,

    presets: Vec<ColorSelectorConfig>,
    selected_preset: usize,

    compute_bounds_pipeline: ComputeBoundsPipeline,
    active_selection: Option<ActiveSelection>,

    planes: Vec<PlaneState>,
    bars: Vec<BarState>,

    cursor_position: Point,
}

impl ColorSelectorState {
    pub fn new(
        color: Color,
        profile: ColorProfile,
        presets: Vec<ColorSelectorConfig>,
        selected_preset: usize,
        services: &Services,
    ) -> Self {
        let device = services.render_device();
        let selected_preset = if presets.is_empty() {
            0
        } else {
            selected_preset.min(presets.len() - 1)
        };

        let output_profile = ColorProfile::new_srgb();

        let mut this = Self {
            color,

            presets,
            selected_preset,

            compute_bounds_pipeline: ComputeBoundsPipeline::new(device, &profile),
            active_selection: None,

            planes: Vec::new(),
            bars: Vec::new(),

            cursor_position: Point::ORIGIN,

            output_profile_version: 0,
            profile,
            output_profile,
        };
        this.rebuild_plane_state(services);
        this.rebuild_bar_states(services);
        this
    }

    pub fn set_output_profile(
        &mut self,
        raw_window_id: u64,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        let Ok(output_profile) = platform::get_window_color_profile(raw_window_id) else {
            return Task::none();
        };

        self.output_profile = output_profile;
        self.output_profile_version += 1;
        self.refresh_clip_bounds(services)
    }

    pub fn color(&self) -> Color {
        self.color
    }

    pub fn set_color(&mut self, color: Color, services: &Services) -> Task<ColorSelectorMessage> {
        self.color = color;
        self.refresh_clip_bounds(services)
    }

    pub fn configs(&self) -> &[ColorSelectorConfig] {
        &self.presets
    }

    pub fn selected_config(&self) -> Option<usize> {
        (!self.presets.is_empty()).then_some(self.selected_preset)
    }

    pub fn set_configs(
        &mut self,
        configs: Vec<ColorSelectorConfig>,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        self.presets = configs;
        self.selected_preset = self.selected_preset.min(self.presets.len() - 1);
        self.active_selection = None;
        self.rebuild_plane_state(services);
        self.rebuild_bar_states(services);
        self.refresh_clip_bounds(services)
    }

    fn bar_display_value(&self, model: ColorModel, channel: u8) -> f32 {
        model.channels(self.color, &self.profile)[channel as usize]
            * model.display_scale()[channel as usize]
    }

    fn set_bar_display_value(
        &mut self,
        model: ColorModel,
        channel: u8,
        value: f32,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        let channel = channel as usize;
        let scale = model.display_scale()[channel];
        let range = model.channel_ranges()[channel];
        let mut channels = model.channels(self.color, &self.profile);
        channels[channel] = (value / scale).clamp(range.x, range.y);
        self.color = model.color_from_channels(channels);
        self.refresh_clip_bounds(services)
    }

    fn plane_primary_channel(&self, index: usize, config: &GradientPlaneConfig) -> u8 {
        self.planes
            .get(index)
            .and_then(|plane| plane.primary_channel_override)
            .unwrap_or_else(|| {
                (0..3)
                    .find(|channel| config.variable_channels & (1 << channel) == 0)
                    .unwrap_or(0)
            })
    }

    fn bar_uses_saturated_primary_channel(&self, config: &GradientBarConfig) -> bool {
        config.model.hue_channel() == Some(config.channel)
            && self.presets[self.selected_preset]
                .planes
                .iter()
                .any(|plane| plane.model == config.model && plane.ring_bar_saturated_hue_channel)
    }

    fn bar_primary_channel_locked(&self, model: ColorModel, channel: u8) -> bool {
        self.presets[self.selected_preset]
            .planes
            .iter()
            .zip(&self.planes)
            .any(|(config, plane)| {
                config.model == model && plane.primary_channel_override == Some(channel)
            })
    }

    fn toggle_primary_channel_override(
        &mut self,
        model: ColorModel,
        channel: u8,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        let enabled = self.bar_primary_channel_locked(model, channel);
        let mut changed = false;
        for (config, plane) in self.presets[self.selected_preset]
            .planes
            .iter()
            .zip(&mut self.planes)
        {
            if config.model == model {
                plane.primary_channel_override = (!enabled).then_some(channel);
                changed = true;
            }
        }
        if changed {
            self.refresh_clip_bounds(services)
        } else {
            Task::none()
        }
    }

    fn plane_variable_channels(&self, index: usize, config: &GradientPlaneConfig) -> u8 {
        self.planes
            .get(index)
            .and_then(|plane| plane.primary_channel_override)
            .map_or(config.variable_channels, |channel| 0b111 & !(1 << channel))
    }

    fn plane_normalized_ranges(&self, index: usize) -> (Vec2, Vec2) {
        if !self.presets[self.selected_preset].clip_to_gamut {
            return (Vec2::new(0.0, 1.0), Vec2::new(0.0, 1.0));
        }

        self.planes
            .get(index)
            .map(|plane| plane.ranges)
            .unwrap_or((Vec2::new(0.0, 1.0), Vec2::new(0.0, 1.0)))
    }

    fn switch_preset(&mut self, index: usize, services: &Services) -> Task<ColorSelectorMessage> {
        if index >= self.presets.len() || index == self.selected_preset {
            return Task::none();
        }

        self.selected_preset = index;
        self.active_selection = None;
        self.rebuild_plane_state(services);
        self.rebuild_bar_states(services);
        self.refresh_clip_bounds(services)
    }

    pub fn update(
        &mut self,
        message: ColorSelectorMessage,
        services: &Services,
    ) -> Task<ColorSelectorMessage> {
        match message {
            ColorSelectorMessage::SurfacePress(target, position) => {
                self.cursor_position = position;
                match target {
                    SurfaceTarget::Plane(index) => {
                        self.start_plane_selection(index, position, services)
                    }
                    SurfaceTarget::Bar(index) => {
                        self.start_bar_selection(index, position, services)
                    }
                }
            }
            ColorSelectorMessage::SurfaceMove(position) => {
                self.cursor_position = position;
                self.update_active_selection(position, services)
            }
            ColorSelectorMessage::SurfaceRelease => {
                let Some(target) = self.active_selection.map(ActiveSelection::surface_target)
                else {
                    return Task::none();
                };
                self.finish_active_selection(target, self.cursor_position, services)
            }
            ColorSelectorMessage::SurfaceBoundsChanged(target, bounds) => match target {
                SurfaceTarget::Plane(index) => {
                    self.update_plane_bounds(index, bounds, services.render_device());
                    Task::none()
                }
                SurfaceTarget::Bar(index) => {
                    self.update_bar_bounds(index, bounds);
                    Task::none()
                }
            },
            ColorSelectorMessage::BarValueChanged(index, value) => {
                let Some(config) = self
                    .presets
                    .get(self.selected_preset)
                    .and_then(|preset| preset.bars.get(index))
                else {
                    return Task::none();
                };
                self.set_bar_display_value(config.model, config.channel, value, services)
            }
            ColorSelectorMessage::SwitchPreset(index) => self.switch_preset(index, services),
            ColorSelectorMessage::PrimaryChannelLock(model, channel) => {
                self.toggle_primary_channel_override(model, channel, services)
            }
            ColorSelectorMessage::ClipBoundsComputed {
                index,
                x_range,
                y_range,
            } => {
                if let Some(plane) = self.planes.get_mut(index) {
                    plane.ranges = (x_range, y_range);
                }
                Task::none()
            }
            ColorSelectorMessage::Changed(_) | ColorSelectorMessage::Confirmed(_) => Task::none(),
        }
    }

    fn plane_surface_data(&self, index: usize) -> Option<Arc<SurfaceDrawData>> {
        let plane = self.planes.get(index)?;
        let config = self.presets.get(self.selected_preset)?.planes.get(index)?;
        let preset = &self.presets[self.selected_preset];
        let mut ring_settings = GradientSettings::new_plane(
            preset.out_of_gamut_color,
            preset.use_out_of_gamut_color,
            config.model.channels(self.color, &self.profile),
            config,
            plane.primary_channel_override,
            plane.size,
        );
        let (x_range, y_range) = self.plane_normalized_ranges(index);
        let settings = GradientSettings {
            saturate_primary_channel: 0,
            x_range,
            y_range,
            ..ring_settings.clone()
        };
        ring_settings.x_range = x_range;
        ring_settings.y_range = y_range;

        Some(Arc::new(SurfaceDrawData {
            id: index as u64,
            mesh: plane.mesh.clone(),
            settings,
            ring_settings: config.show_primary_channel_ring.then_some(ring_settings),
            profile: Arc::new(self.profile.clone()),
            output_profile: Arc::new(self.output_profile.clone()),
            output_profile_version: self.output_profile_version,
        }))
    }

    fn bar_surface_data(&self, index: usize) -> Option<Arc<SurfaceDrawData>> {
        let config = self.presets.get(self.selected_preset)?.bars.get(index)?;
        let bar = self.bars.get(index)?;
        let preset = &self.presets[self.selected_preset];
        let settings = GradientSettings::new_bar(
            preset.out_of_gamut_color,
            preset.use_out_of_gamut_color,
            config.model.channels(self.color, &self.profile),
            config,
            self.bar_uses_saturated_primary_channel(config),
        );

        Some(Arc::new(SurfaceDrawData {
            id: (index as u64) | (1 << 32),
            mesh: bar.mesh.clone(),
            settings,
            ring_settings: None,
            profile: Arc::new(self.profile.clone()),
            output_profile: Arc::new(self.output_profile.clone()),
            output_profile_version: self.output_profile_version,
        }))
    }
}
