use std::ops::RangeInclusive;

use iced_core::Length;
use lapiz_assets::{asset::AssetId, store::AssetRegistry};
use lapiz_i18n::t;
use lapiz_image::blend_modes::BlendMode;
use lapiz_render::texture::{Image, thumbnail::ImageThumbnail};
use lapiz_widgets::{
    button, checkbox, column, combo_box::ComboBox, flex::Flex, label, row, scrollable, space,
    spin_slider::SpinSlider,
};

use super::{EditorElement, PhotoshopBrushEditor, PhotoshopBrushEditorMessage};
use crate::parameter::{
    BrushTip, ComputedBrushTip, DynamicsControl, PhotoshopBrushPreset, SampledBrushTip,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    BrushTip,
    ShapeDynamics,
    Scattering,
    Texture,
    DualBrush,
    ColorDynamics,
    Transfer,
    BrushPose,
    Noise,
    WetEdges,
    BuildUp,
    Smoothing,
    ProtectTexture,
}

impl Section {
    pub const ALL: [Self; 13] = [
        Self::BrushTip,
        Self::ShapeDynamics,
        Self::Scattering,
        Self::Texture,
        Self::DualBrush,
        Self::ColorDynamics,
        Self::Transfer,
        Self::BrushPose,
        Self::Noise,
        Self::WetEdges,
        Self::BuildUp,
        Self::Smoothing,
        Self::ProtectTexture,
    ];

    pub(super) fn label(self) -> String {
        t!(match self {
            Self::BrushTip => "photoshop_brush_tip",
            Self::ShapeDynamics => "photoshop_brush_shape",
            Self::Scattering => "photoshop_brush_scattering",
            Self::Texture => "photoshop_brush_texture",
            Self::DualBrush => "photoshop_brush_dual",
            Self::ColorDynamics => "photoshop_brush_color",
            Self::Transfer => "photoshop_brush_transfer",
            Self::BrushPose => "photoshop_brush_pose",
            Self::Noise => "photoshop_brush_noise",
            Self::WetEdges => "photoshop_brush_wet_edges",
            Self::BuildUp => "photoshop_brush_build_up",
            Self::Smoothing => "photoshop_brush_smoothing",
            Self::ProtectTexture => "photoshop_brush_protect_texture",
        })
    }

    pub(super) fn enabled(self, brush: &PhotoshopBrushPreset) -> Option<bool> {
        Some(match self {
            Self::BrushTip => return None,
            Self::ShapeDynamics => brush.shape_dynamics.enabled,
            Self::Scattering => brush.scattering.enabled,
            Self::Texture => brush.texture.enabled,
            Self::DualBrush => brush.dual_brush.enabled,
            Self::ColorDynamics => brush.color_dynamics.enabled,
            Self::Transfer => brush.transfer.enabled,
            Self::BrushPose => brush.brush_pose.enabled,
            Self::Noise => brush.noise,
            Self::WetEdges => brush.wet_edges,
            Self::BuildUp => brush.build_up,
            Self::Smoothing => brush.smoothing.enabled,
            Self::ProtectTexture => brush.protect_texture,
        })
    }

    pub(super) fn set_enabled(self, brush: &mut PhotoshopBrushPreset, enabled: bool) {
        match self {
            Self::BrushTip => {}
            Self::ShapeDynamics => brush.shape_dynamics.enabled = enabled,
            Self::Scattering => brush.scattering.enabled = enabled,
            Self::Texture => brush.texture.enabled = enabled,
            Self::DualBrush => brush.dual_brush.enabled = enabled,
            Self::ColorDynamics => brush.color_dynamics.enabled = enabled,
            Self::Transfer => brush.transfer.enabled = enabled,
            Self::BrushPose => brush.brush_pose.enabled = enabled,
            Self::Noise => brush.noise = enabled,
            Self::WetEdges => brush.wet_edges = enabled,
            Self::BuildUp => brush.build_up = enabled,
            Self::Smoothing => brush.smoothing.enabled = enabled,
            Self::ProtectTexture => brush.protect_texture = enabled,
        }
    }
}

pub(super) fn section_toggle(
    section: Section,
    brush: &PhotoshopBrushPreset,
) -> EditorElement<'static> {
    match section.enabled(brush) {
        Some(enabled) => checkbox(enabled)
            .size(14)
            .on_toggle(move |value| PhotoshopBrushEditorMessage::ToggleSection(section, value))
            .into(),
        None => space().width(14).into(),
    }
}

macro_rules! number_field {
    ($brush:ident, $enabled:expr, $key:literal, $($field:ident).+, $range:expr, $unit:literal) => {
        number($key, $brush.$($field).+, $range, $unit, $enabled,
            |brush, value| brush.$($field).+ = value)
    };
}

macro_rules! percent_field {
    ($brush:ident, $enabled:expr, $key:literal, $($field:ident).+) => {
        number_field!($brush, $enabled, $key, $($field).+, 0.0..=100.0, "%")
    };
}

macro_rules! toggle_field {
    ($brush:ident, $enabled:expr, $key:literal, $($field:ident).+) => {
        toggle($key, $brush.$($field).+, $enabled, |brush, value| brush.$($field).+ = value)
    };
}

macro_rules! control_field {
    ($brush:ident, $enabled:expr, $($field:ident).+) => {
        control($brush.$($field).+, $enabled, |brush, value| brush.$($field).+ = value)
    };
}

impl PhotoshopBrushEditor {
    pub(super) fn view_section(&self, assets: &AssetRegistry) -> EditorElement<'_> {
        let brush = &self.brush;
        let enabled = self.section.enabled(brush).unwrap_or(true);
        match self.section {
            Section::BrushTip => self.view_tip(TipTarget::Primary, assets, true),
            Section::ShapeDynamics => column![
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_size_jitter",
                    shape_dynamics.size_jitter_percent
                ),
                control_field!(brush, enabled, shape_dynamics.size_control),
                percent_field!(
                    brush,
                    enabled && brush.shape_dynamics.size_control != DynamicsControl::Off,
                    "photoshop_brush_minimum_diameter",
                    shape_dynamics.minimum_diameter_percent
                ),
                number_field!(
                    brush,
                    enabled && brush.shape_dynamics.size_control == DynamicsControl::PenTilt,
                    "photoshop_brush_tilt_scale",
                    shape_dynamics.tilt_scale_percent,
                    0.0..=200.0,
                    "%"
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_angle_jitter",
                    shape_dynamics.angle_jitter_percent
                ),
                control_field!(brush, enabled, shape_dynamics.angle_control),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_roundness_jitter",
                    shape_dynamics.roundness_jitter_percent
                ),
                control_field!(brush, enabled, shape_dynamics.roundness_control),
                percent_field!(
                    brush,
                    enabled && brush.shape_dynamics.roundness_control != DynamicsControl::Off,
                    "photoshop_brush_minimum_roundness",
                    shape_dynamics.minimum_roundness_percent
                ),
                row![
                    toggle_field!(
                        brush,
                        enabled,
                        "photoshop_brush_flip_x_jitter",
                        shape_dynamics.flip_x_jitter
                    ),
                    toggle_field!(
                        brush,
                        enabled,
                        "photoshop_brush_flip_y_jitter",
                        shape_dynamics.flip_y_jitter
                    ),
                ]
                .gap(16.0),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_projection",
                    shape_dynamics.brush_projection
                ),
            ]
            .gap(10.0)
            .into(),
            Section::Scattering => column![
                number_field!(
                    brush,
                    enabled,
                    "photoshop_brush_scatter",
                    scattering.scatter_percent,
                    0.0..=1000.0,
                    "%"
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_both_axes",
                    scattering.both_axes
                ),
                control_field!(brush, enabled, scattering.scatter_control),
                unsigned(
                    "photoshop_brush_count",
                    brush.scattering.count,
                    1..=16,
                    enabled,
                    |brush, value| brush.scattering.count = value
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_count_jitter",
                    scattering.count_jitter_percent
                ),
                control_field!(brush, enabled, scattering.count_control),
            ]
            .gap(10.0)
            .into(),
            Section::Texture => self.view_texture(assets, enabled),
            Section::DualBrush => self.view_dual(assets, enabled),
            Section::ColorDynamics => column![
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_apply_per_tip",
                    color_dynamics.apply_per_tip
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_foreground_background_jitter",
                    color_dynamics.foreground_background_jitter_percent
                ),
                control_field!(brush, enabled, color_dynamics.foreground_background_control),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_hue_jitter",
                    color_dynamics.hue_jitter_percent
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_saturation_jitter",
                    color_dynamics.saturation_jitter_percent
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_brightness_jitter",
                    color_dynamics.brightness_jitter_percent
                ),
                number_field!(
                    brush,
                    enabled,
                    "photoshop_brush_purity",
                    color_dynamics.purity_percent,
                    -100.0..=100.0,
                    "%"
                ),
            ]
            .gap(10.0)
            .into(),
            Section::Transfer => column![
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_opacity_jitter",
                    transfer.opacity_jitter_percent
                ),
                control_field!(brush, enabled, transfer.opacity_control),
                percent_field!(
                    brush,
                    enabled && brush.transfer.opacity_control != DynamicsControl::Off,
                    "photoshop_brush_minimum",
                    transfer.minimum_opacity_percent
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_flow_jitter",
                    transfer.flow_jitter_percent
                ),
                control_field!(brush, enabled, transfer.flow_control),
                percent_field!(
                    brush,
                    enabled && brush.transfer.flow_control != DynamicsControl::Off,
                    "photoshop_brush_minimum",
                    transfer.minimum_flow_percent
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_wetness_jitter",
                    transfer.wetness_jitter_percent
                ),
                control_field!(brush, enabled, transfer.wetness_control),
                percent_field!(
                    brush,
                    enabled && brush.transfer.wetness_control != DynamicsControl::Off,
                    "photoshop_brush_minimum",
                    transfer.minimum_wetness_percent
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_mix_jitter",
                    transfer.mix_jitter_percent
                ),
                control_field!(brush, enabled, transfer.mix_control),
                percent_field!(
                    brush,
                    enabled && brush.transfer.mix_control != DynamicsControl::Off,
                    "photoshop_brush_minimum",
                    transfer.minimum_mix_percent
                ),
            ]
            .gap(10.0)
            .into(),
            Section::BrushPose => column![
                number_field!(
                    brush,
                    enabled,
                    "photoshop_brush_tilt_x",
                    brush_pose.tilt_x_percent,
                    -100.0..=100.0,
                    "%"
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_override_tilt_x",
                    brush_pose.override_tilt_x
                ),
                number_field!(
                    brush,
                    enabled,
                    "photoshop_brush_tilt_y",
                    brush_pose.tilt_y_percent,
                    -100.0..=100.0,
                    "%"
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_override_tilt_y",
                    brush_pose.override_tilt_y
                ),
                number_field!(
                    brush,
                    enabled,
                    "photoshop_brush_rotation",
                    brush_pose.rotation_degrees,
                    0.0..=360.0,
                    "°"
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_override_rotation",
                    brush_pose.override_rotation
                ),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_pressure",
                    brush_pose.pressure_percent
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_override_pressure",
                    brush_pose.override_pressure
                ),
            ]
            .gap(10.0)
            .into(),
            Section::Smoothing => column![
                label(t!("photoshop_brush_compiler_limit")).muted(),
                percent_field!(
                    brush,
                    enabled,
                    "photoshop_brush_amount",
                    smoothing.amount_percent
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_pulled_string",
                    smoothing.pulled_string_mode
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_catch_up",
                    smoothing.stroke_catch_up
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_catch_up_end",
                    smoothing.catch_up_on_stroke_end
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_adjust_zoom",
                    smoothing.adjust_for_zoom
                ),
                toggle_field!(
                    brush,
                    enabled,
                    "photoshop_brush_pressure_smoothing",
                    smoothing.pressure_smoothing
                ),
            ]
            .gap(10.0)
            .into(),
            Section::Noise => label(t!("photoshop_brush_toggle_hint")).muted().into(),
            Section::WetEdges | Section::BuildUp | Section::ProtectTexture => {
                label(t!("photoshop_brush_compiler_limit")).muted().into()
            }
        }
    }

    fn view_texture(&self, assets: &AssetRegistry, enabled: bool) -> EditorElement<'_> {
        let brush = &self.brush;
        let dynamics_enabled = enabled && brush.texture.texture_each_tip;
        column![
            image_selector(
                "photoshop_brush_pattern",
                brush.texture.pattern,
                assets,
                enabled,
                |brush, id| brush.texture.pattern = Some(id)
            ),
            toggle_field!(brush, enabled, "photoshop_brush_invert", texture.invert),
            number_field!(
                brush,
                enabled,
                "photoshop_brush_scale",
                texture.scale_percent,
                1.0..=1000.0,
                "%"
            ),
            integer(
                "photoshop_brush_brightness",
                brush.texture.brightness,
                -150..=150,
                enabled,
                |brush, value| brush.texture.brightness = value
            ),
            integer(
                "photoshop_brush_contrast",
                brush.texture.contrast,
                -50..=100,
                enabled,
                |brush, value| brush.texture.contrast = value
            ),
            toggle_field!(
                brush,
                enabled,
                "photoshop_brush_each_tip",
                texture.texture_each_tip
            ),
            blend_mode(brush.texture.blend_mode, enabled, |brush, value| brush
                .texture
                .blend_mode =
                value),
            percent_field!(
                brush,
                enabled,
                "photoshop_brush_depth",
                texture.depth_percent
            ),
            percent_field!(
                brush,
                dynamics_enabled && brush.texture.depth_control != DynamicsControl::Off,
                "photoshop_brush_minimum_depth",
                texture.minimum_depth_percent
            ),
            percent_field!(
                brush,
                dynamics_enabled,
                "photoshop_brush_depth_jitter",
                texture.depth_jitter_percent
            ),
            control_field!(brush, dynamics_enabled, texture.depth_control),
        ]
        .gap(10.0)
        .into()
    }

    fn view_dual(&self, assets: &AssetRegistry, enabled: bool) -> EditorElement<'_> {
        let brush = &self.brush;
        let scatter_enabled = enabled && brush.dual_brush.scatter_enabled;
        column![
            blend_mode(brush.dual_brush.blend_mode, enabled, |brush, value| brush
                .dual_brush
                .blend_mode =
                value),
            toggle_field!(brush, enabled, "photoshop_brush_flip", dual_brush.flip),
            self.view_tip(TipTarget::Secondary, assets, enabled),
            toggle_field!(
                brush,
                enabled,
                "photoshop_brush_use_scatter",
                dual_brush.scatter_enabled
            ),
            number_field!(
                brush,
                scatter_enabled,
                "photoshop_brush_scatter",
                dual_brush.scatter_percent,
                0.0..=1000.0,
                "%"
            ),
            toggle_field!(
                brush,
                scatter_enabled,
                "photoshop_brush_both_axes",
                dual_brush.both_axes
            ),
            control_field!(brush, scatter_enabled, dual_brush.scatter_control),
            unsigned(
                "photoshop_brush_count",
                brush.dual_brush.count,
                1..=16,
                scatter_enabled,
                |brush, value| brush.dual_brush.count = value
            ),
            percent_field!(
                brush,
                scatter_enabled,
                "photoshop_brush_count_jitter",
                dual_brush.count_jitter_percent
            ),
            control_field!(brush, scatter_enabled, dual_brush.count_control),
        ]
        .gap(10.0)
        .into()
    }

    fn view_tip(
        &self,
        target: TipTarget,
        assets: &AssetRegistry,
        enabled: bool,
    ) -> EditorElement<'_> {
        let tip = target.get(&self.brush);
        let selected_image = match tip {
            BrushTip::Sampled(tip) => Some(tip.image),
            _ => None,
        };
        let round = button(label(t!("photoshop_brush_computed")))
            .activated(matches!(tip, BrushTip::Computed(_)))
            .on_press_maybe(enabled.then(|| {
                PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| {
                    target.select_computed(brush)
                }))
            }));
        let images = assets.all_handles_of::<Image>().unwrap_or_default();
        let image_buttons = images
            .into_iter()
            .map(|handle| {
                let id = handle.id();
                button(ImageThumbnail::new(handle))
                    .width(175)
                    .height(44)
                    .activated(selected_image == Some(id))
                    .on_press_maybe(enabled.then(|| {
                        PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| {
                            target.select_image(brush, id)
                        }))
                    }))
                    .into()
            })
            .collect::<Vec<_>>();
        let images = if image_buttons.is_empty() {
            EditorElement::from(label(t!("photoshop_brush_no_images")).muted())
        } else {
            scrollable(Flex::row(image_buttons).wrap().gap(4.0).width(Length::Fill))
                .height(110)
                .into()
        };

        let (size, flip_x, flip_y, angle, roundness, spacing_enabled, spacing) = match tip {
            BrushTip::Computed(tip) => (
                tip.size_pixels,
                tip.flip_x,
                tip.flip_y,
                tip.angle_degrees,
                tip.roundness_percent,
                tip.spacing_enabled,
                tip.spacing_percent,
            ),
            BrushTip::Sampled(tip) => (
                tip.size_pixels,
                tip.flip_x,
                tip.flip_y,
                tip.angle_degrees,
                tip.roundness_percent,
                tip.spacing_enabled,
                tip.spacing_percent,
            ),
        };
        let mut fields = column![
            round,
            images,
            number(
                "photoshop_brush_size",
                size,
                0.1..=1000.0,
                " px",
                enabled,
                move |brush, value| {
                    match target.get_mut(brush) {
                        BrushTip::Computed(tip) => tip.size_pixels = value,
                        BrushTip::Sampled(tip) => tip.size_pixels = value,
                    }
                }
            ),
            row![
                toggle(
                    "photoshop_brush_flip_x",
                    flip_x,
                    enabled,
                    move |brush, value| {
                        match target.get_mut(brush) {
                            BrushTip::Computed(tip) => tip.flip_x = value,
                            BrushTip::Sampled(tip) => tip.flip_x = value,
                        }
                    }
                ),
                toggle(
                    "photoshop_brush_flip_y",
                    flip_y,
                    enabled,
                    move |brush, value| {
                        match target.get_mut(brush) {
                            BrushTip::Computed(tip) => tip.flip_y = value,
                            BrushTip::Sampled(tip) => tip.flip_y = value,
                        }
                    }
                ),
            ]
            .gap(16.0),
            number(
                "photoshop_brush_angle",
                angle,
                -180.0..=180.0,
                "°",
                enabled,
                move |brush, value| {
                    match target.get_mut(brush) {
                        BrushTip::Computed(tip) => tip.angle_degrees = value,
                        BrushTip::Sampled(tip) => tip.angle_degrees = value,
                    }
                }
            ),
            number(
                "photoshop_brush_roundness",
                roundness,
                0.0..=100.0,
                "%",
                enabled,
                move |brush, value| {
                    match target.get_mut(brush) {
                        BrushTip::Computed(tip) => tip.roundness_percent = value,
                        BrushTip::Sampled(tip) => tip.roundness_percent = value,
                    }
                }
            ),
        ]
        .gap(10.0);
        if let BrushTip::Computed(tip) = tip {
            fields = fields.push(number(
                "photoshop_brush_hardness",
                tip.hardness_percent,
                0.0..=100.0,
                "%",
                enabled,
                move |brush, value| {
                    if let BrushTip::Computed(tip) = target.get_mut(brush) {
                        tip.hardness_percent = value;
                    }
                },
            ));
        }
        fields
            .push(toggle(
                "photoshop_brush_spacing_enabled",
                spacing_enabled,
                enabled,
                move |brush, value| match target.get_mut(brush) {
                    BrushTip::Computed(tip) => tip.spacing_enabled = value,
                    BrushTip::Sampled(tip) => tip.spacing_enabled = value,
                },
            ))
            .push(number(
                "photoshop_brush_spacing",
                spacing,
                0.1..=1000.0,
                "%",
                enabled && spacing_enabled,
                move |brush, value| match target.get_mut(brush) {
                    BrushTip::Computed(tip) => tip.spacing_percent = value,
                    BrushTip::Sampled(tip) => tip.spacing_percent = value,
                },
            ))
            .into()
    }
}

#[derive(Clone, Copy)]
enum TipTarget {
    Primary,
    Secondary,
}

impl TipTarget {
    fn get(self, brush: &PhotoshopBrushPreset) -> &BrushTip {
        match self {
            Self::Primary => &brush.brush_tip,
            Self::Secondary => &brush.dual_brush.brush_tip,
        }
    }

    fn get_mut(self, brush: &mut PhotoshopBrushPreset) -> &mut BrushTip {
        match self {
            Self::Primary => &mut brush.brush_tip,
            Self::Secondary => &mut brush.dual_brush.brush_tip,
        }
    }

    fn select_computed(self, brush: &mut PhotoshopBrushPreset) {
        let tip = self.get_mut(brush);
        if let BrushTip::Sampled(sampled) = tip {
            *tip = BrushTip::Computed(ComputedBrushTip {
                size_pixels: sampled.size_pixels,
                flip_x: sampled.flip_x,
                flip_y: sampled.flip_y,
                angle_degrees: sampled.angle_degrees,
                roundness_percent: sampled.roundness_percent,
                spacing_enabled: sampled.spacing_enabled,
                spacing_percent: sampled.spacing_percent,
                ..ComputedBrushTip::default()
            });
        }
    }

    fn select_image(self, brush: &mut PhotoshopBrushPreset, image: AssetId<Image>) {
        let tip = self.get_mut(brush);
        match tip {
            BrushTip::Sampled(tip) => tip.image = image,
            BrushTip::Computed(computed) => {
                *tip = BrushTip::Sampled(SampledBrushTip {
                    image,
                    size_pixels: computed.size_pixels,
                    flip_x: computed.flip_x,
                    flip_y: computed.flip_y,
                    angle_degrees: computed.angle_degrees,
                    roundness_percent: computed.roundness_percent,
                    spacing_enabled: computed.spacing_enabled,
                    spacing_percent: computed.spacing_percent,
                });
            }
        }
    }
}

fn number(
    key: &str,
    value: f32,
    range: RangeInclusive<f32>,
    suffix: &str,
    enabled: bool,
    set: impl Fn(&mut PhotoshopBrushPreset, f32) + Copy + Send + 'static,
) -> EditorElement<'static> {
    column![
        label(t!(key)).muted(),
        SpinSlider::new(range, value)
            .precision(1)
            .suffix(suffix)
            .allow_beyond_range(false)
            .disabled(!enabled)
            .on_change(
                move |value| PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| set(
                    brush, value
                )))
            ),
    ]
    .gap(3.0)
    .into()
}

fn unsigned(
    key: &str,
    value: u32,
    range: RangeInclusive<u32>,
    enabled: bool,
    set: impl Fn(&mut PhotoshopBrushPreset, u32) + Copy + Send + 'static,
) -> EditorElement<'static> {
    row![
        label(t!(key)).width(Length::Fill),
        SpinSlider::new(range, value)
            .precision(0)
            .allow_beyond_range(false)
            .disabled(!enabled)
            .width(180)
            .on_change(
                move |value| PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| set(
                    brush, value
                )))
            ),
    ]
    .gap(8.0)
    .into()
}

fn integer(
    key: &str,
    value: i32,
    range: RangeInclusive<i32>,
    enabled: bool,
    set: impl Fn(&mut PhotoshopBrushPreset, i32) + Copy + Send + 'static,
) -> EditorElement<'static> {
    column![
        label(t!(key)).muted(),
        SpinSlider::new(range, value)
            .precision(0)
            .allow_beyond_range(false)
            .disabled(!enabled)
            .on_change(
                move |value| PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| set(
                    brush, value
                )))
            ),
    ]
    .gap(3.0)
    .into()
}

fn toggle(
    key: &str,
    value: bool,
    enabled: bool,
    set: impl Fn(&mut PhotoshopBrushPreset, bool) + Copy + Send + 'static,
) -> EditorElement<'static> {
    checkbox(value)
        .label(t!(key))
        .on_toggle_maybe(enabled.then_some(move |value| {
            PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| set(brush, value)))
        }))
        .into()
}

fn control(
    value: DynamicsControl,
    enabled: bool,
    set: fn(&mut PhotoshopBrushPreset, DynamicsControl),
) -> EditorElement<'static> {
    let steps = match value {
        DynamicsControl::Fade { steps } => steps,
        _ => 100,
    };
    let options = vec![
        DynamicsControl::Off,
        DynamicsControl::Fade { steps },
        DynamicsControl::PenPressure,
        DynamicsControl::PenTilt,
        DynamicsControl::StylusWheel,
        DynamicsControl::InitialDirection,
        DynamicsControl::Direction,
        DynamicsControl::Rotation,
    ];
    let choice = if enabled {
        EditorElement::from(
            ComboBox::new_with(
                options,
                Some(value),
                move |value| {
                    PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| set(brush, value)))
                },
                |value| label(control_label(*value)).into(),
            )
            .width(Length::Fill),
        )
    } else {
        button(label(control_label(value)))
            .width(Length::Fill)
            .into()
    };
    let mut content =
        column![row![label(t!("photoshop_brush_control")).width(120), choice].gap(8.0)].gap(6.0);
    if let DynamicsControl::Fade { steps } = value {
        content = content.push(unsigned(
            "photoshop_brush_fade_steps",
            steps,
            1..=10000,
            enabled,
            move |brush, steps| {
                set(brush, DynamicsControl::Fade { steps });
            },
        ));
    }
    content.into()
}

fn control_label(value: DynamicsControl) -> String {
    t!(match value {
        DynamicsControl::Off => "photoshop_brush_off",
        DynamicsControl::Fade { .. } => "photoshop_brush_fade",
        DynamicsControl::PenPressure => "photoshop_brush_pen_pressure",
        DynamicsControl::PenTilt => "photoshop_brush_pen_tilt",
        DynamicsControl::StylusWheel => "photoshop_brush_stylus_wheel",
        DynamicsControl::InitialDirection => "photoshop_brush_initial_direction",
        DynamicsControl::Direction => "photoshop_brush_direction",
        DynamicsControl::Rotation => "photoshop_brush_rotation",
    })
}

fn blend_mode(
    value: BlendMode,
    enabled: bool,
    set: fn(&mut PhotoshopBrushPreset, BlendMode),
) -> EditorElement<'static> {
    let choice = if enabled {
        EditorElement::from(
            ComboBox::new_with(
                BlendMode::ALL.to_vec(),
                Some(value),
                move |value| {
                    PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| set(brush, value)))
                },
                |mode| label(t!(mode.shader_func())).into(),
            )
            .width(Length::Fill),
        )
    } else {
        button(label(t!(value.shader_func())))
            .width(Length::Fill)
            .into()
    };
    row![label(t!("photoshop_brush_mode")).width(120), choice]
        .gap(8.0)
        .into()
}

fn image_selector(
    key: &str,
    selected: Option<AssetId<Image>>,
    assets: &AssetRegistry,
    enabled: bool,
    set: fn(&mut PhotoshopBrushPreset, AssetId<Image>),
) -> EditorElement<'static> {
    let options = assets.all_handles_of::<Image>().unwrap_or_default();
    let selected = selected.and_then(|id| options.iter().find(|handle| handle.id() == id).cloned());
    if !enabled {
        let selected = selected
            .map(|handle| EditorElement::from(ImageThumbnail::new(handle)))
            .unwrap_or_else(|| label("").into());
        return column![label(t!(key)).muted(), button(selected).width(Length::Fill),]
            .gap(3.0)
            .into();
    }
    if options.is_empty() {
        return label(t!("photoshop_brush_no_images")).muted().into();
    }
    column![
        label(t!(key)).muted(),
        ComboBox::new_with(
            options,
            selected,
            move |handle| {
                let id = handle.id();
                PhotoshopBrushEditorMessage::Edit(Box::new(move |brush| set(brush, id)))
            },
            |handle| ImageThumbnail::new(handle.clone()).into()
        )
        .width(Length::Fill),
    ]
    .gap(3.0)
    .into()
}
