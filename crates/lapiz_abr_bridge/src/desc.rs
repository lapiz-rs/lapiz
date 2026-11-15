use std::collections::HashMap;

use anyhow::{Context as _, Result, bail, ensure};
use lapiz_abr::descriptor::{
    UnitFloat,
    class::{self, Descriptor, PropertyDynamics, ToolOptions},
};
use lapiz_assets::asset::AssetId;
use lapiz_image::blend_modes::BlendMode;
use lapiz_photoshop_brush_engine::parameter::{self, PhotoshopBrushPreset};
use lapiz_render::texture::Image;
use uuid::Uuid;

fn parse_blend_mode(mode: class::BlendMode) -> BlendMode {
    match mode {
        class::BlendMode::Normal => BlendMode::Normal,
        class::BlendMode::Dissolve => BlendMode::Dissolve,
        class::BlendMode::Behind => BlendMode::Behind,
        class::BlendMode::Clear => BlendMode::Clear,
        class::BlendMode::Darken => BlendMode::Darken,
        class::BlendMode::Multiply => BlendMode::Multiply,
        class::BlendMode::ColorBurn => BlendMode::ColorBurn,
        class::BlendMode::LinearBurn => BlendMode::LinearBurn,
        class::BlendMode::DarkerColor => BlendMode::DarkerColor,
        class::BlendMode::Lighten => BlendMode::Lighten,
        class::BlendMode::Screen => BlendMode::Screen,
        class::BlendMode::ColorDodge => BlendMode::ColorDodge,
        class::BlendMode::LinearDodge => BlendMode::LinearDodge,
        class::BlendMode::LighterColor => BlendMode::LighterColor,
        class::BlendMode::Overlay => BlendMode::Overlay,
        class::BlendMode::SoftLight => BlendMode::SoftLight,
        class::BlendMode::HardLight => BlendMode::HardLight,
        class::BlendMode::VividLight => BlendMode::VividLight,
        class::BlendMode::LinearLight => BlendMode::LinearLight,
        class::BlendMode::PinLight => BlendMode::PinLight,
        class::BlendMode::HardMix => BlendMode::HardMix,
        class::BlendMode::Difference => BlendMode::Difference,
        class::BlendMode::Exclusion => BlendMode::Exclusion,
        class::BlendMode::Subtract => BlendMode::Subtract,
        class::BlendMode::SubtractTexture => BlendMode::Subtractive,
        class::BlendMode::Divide => BlendMode::Divide,
        class::BlendMode::Hue => BlendMode::Hue,
        class::BlendMode::Saturation => BlendMode::Saturation,
        class::BlendMode::Color => BlendMode::Color,
        class::BlendMode::Luminosity => BlendMode::Luminosity,
        class::BlendMode::Height => BlendMode::Height,
        class::BlendMode::LinearHeight => BlendMode::LinearHeight,
    }
}

fn parse_tip(
    tip: &class::BrushTip,
    sample_assets: &HashMap<Uuid, AssetId<Image>>,
) -> Result<parameter::BrushTip> {
    match tip {
        class::BrushTip::Computed(tip) => {
            Ok(parameter::BrushTip::Computed(parameter::ComputedBrushTip {
                size_pixels: tip.diameter.value as f32,
                flip_x: tip.flip_x,
                flip_y: tip.flip_y,
                angle_degrees: tip.angle.value as f32,
                roundness_percent: tip.roundness.value as f32,
                hardness_percent: tip.hardness.value as f32,
                spacing_enabled: tip.interpolation,
                spacing_percent: tip.spacing.value as f32,
            }))
        }
        class::BrushTip::Sampled(tip) => {
            Ok(parameter::BrushTip::Sampled(parameter::SampledBrushTip {
                image: sample_assets
                    .get(&tip.id)
                    .copied()
                    .with_context(|| format!("sample not found {}", tip.id))?,
                size_pixels: tip.diameter.value as f32,
                flip_x: tip.flip_x,
                flip_y: tip.flip_y,
                angle_degrees: tip.angle.value as f32,
                roundness_percent: tip.roundness.value as f32,
                spacing_enabled: tip.interpolation,
                spacing_percent: tip.spacing.value as f32,
            }))
        }
        class::BrushTip::DBrush(_) => bail!("unsupported bristle brush tip"),
    }
}

fn parse_control(dynamics: Option<&PropertyDynamics>) -> Result<parameter::DynamicsControl> {
    let Some(dynamics) = dynamics else {
        return Ok(parameter::DynamicsControl::Off);
    };

    Ok(match dynamics.control {
        class::DynamicsControl::Off => parameter::DynamicsControl::Off,
        class::DynamicsControl::Fade => parameter::DynamicsControl::Fade {
            steps: dynamics
                .fade_steps
                .try_into()
                .context("negative fade steps")?,
        },
        class::DynamicsControl::PenPressure => parameter::DynamicsControl::PenPressure,
        class::DynamicsControl::PenTilt => parameter::DynamicsControl::PenTilt,
        class::DynamicsControl::StylusWheel => parameter::DynamicsControl::StylusWheel,
        class::DynamicsControl::InitialDirection => parameter::DynamicsControl::InitialDirection,
        class::DynamicsControl::Direction => parameter::DynamicsControl::Direction,
        class::DynamicsControl::Rotation => parameter::DynamicsControl::Rotation,
    })
}

fn jitter(dynamics: Option<&PropertyDynamics>) -> f32 {
    dynamics.map_or(0.0, |dynamics| dynamics.jitter.value as f32)
}

fn minimum(dynamics: Option<&PropertyDynamics>, value: Option<&UnitFloat>) -> f32 {
    percent(value, 0.0).max(percent(
        dynamics.and_then(|dynamics| dynamics.minimum.as_ref()),
        0.0,
    ))
}

fn percent(value: Option<&UnitFloat>, default: f32) -> f32 {
    value.map_or(default, |value| value.value as f32)
}

fn parse_count(count: f64) -> Result<u32> {
    ensure!(
        count.is_finite() && count.fract() == 0.0 && (1.0..=f64::from(u32::MAX)).contains(&count),
        "invalid scatter count {count}"
    );
    Ok(count as u32)
}

pub fn parse_desc(
    brush: &Descriptor,
    sample_assets: &HashMap<Uuid, AssetId<Image>>,
    pattern_assets: &HashMap<Uuid, AssetId<Image>>,
) -> Result<PhotoshopBrushPreset> {
    let paint_options = match &brush.tool_options {
        Some(ToolOptions::Paint(options)) => Some(options),
        _ => None,
    };
    let size = brush
        .size_dynamics
        .as_ref()
        .or_else(|| paint_options.and_then(|options| options.size_dynamics.as_ref()));
    let opacity = brush
        .opacity_dynamics
        .as_ref()
        .or_else(|| paint_options.and_then(|options| options.opacity_dynamics.as_ref()));
    let flow = brush
        .flow_dynamics
        .as_ref()
        .or_else(|| paint_options.and_then(|options| options.flow_dynamics.as_ref()));
    let color = brush
        .color_dynamics
        .as_ref()
        .or_else(|| paint_options.and_then(|options| options.color_dynamics.as_ref()));
    let angle = brush.angle_dynamics.as_ref();
    let roundness = brush.roundness_dynamics.as_ref();
    let scatter = brush.scatter_dynamics.as_ref();
    let count = brush.count_dynamics.as_ref();
    let depth = brush.texture_depth_dynamics.as_ref();
    let wetness = brush.wetness_dynamics.as_ref();
    let mix = brush.mix_dynamics.as_ref();
    let pattern = brush
        .texture
        .as_ref()
        .and_then(|texture| pattern_assets.get(&texture.id))
        .copied();
    ensure!(
        !brush.use_texture || pattern.is_some(),
        "missing texture pattern"
    );

    let dual = &brush.dual_brush;
    let dual_tip = dual
        .brush
        .as_ref()
        .map(|tip| parse_tip(tip, sample_assets))
        .transpose()?;
    ensure!(
        !dual.enabled || dual_tip.is_some(),
        "missing dual brush tip"
    );
    let mut dual_tip = dual_tip.unwrap_or_default();
    if let Some(spacing) = &dual.spacing {
        match &mut dual_tip {
            parameter::BrushTip::Computed(tip) => tip.spacing_percent = spacing.value as f32,
            parameter::BrushTip::Sampled(tip) => tip.spacing_percent = spacing.value as f32,
        }
    }
    let dual_scatter = dual.scatter_dynamics.as_ref();
    let dual_count = dual.count_dynamics.as_ref();

    let mut result = PhotoshopBrushPreset {
        name: brush.name.clone(),
        brush_tip: parse_tip(&brush.brush, sample_assets).context("brush tip")?,
        shape_dynamics: parameter::ShapeDynamics {
            enabled: brush.use_tip_dynamics,
            size_jitter_percent: jitter(size),
            size_control: parse_control(size).context("size control")?,
            minimum_diameter_percent: minimum(size, brush.minimum_diameter.as_ref()),
            tilt_scale_percent: percent(brush.tilt_scale.as_ref(), 200.0),
            angle_jitter_percent: jitter(angle),
            angle_control: parse_control(angle).context("angle control")?,
            roundness_jitter_percent: jitter(roundness),
            roundness_control: parse_control(roundness).context("roundness control")?,
            minimum_roundness_percent: minimum(roundness, brush.minimum_roundness.as_ref()),
            flip_x_jitter: brush.flip_x_jitter,
            flip_y_jitter: brush.flip_y_jitter,
            brush_projection: brush.brush_projection,
        },
        scattering: parameter::Scattering {
            enabled: brush.use_scatter,
            scatter_percent: jitter(scatter),
            both_axes: brush.scatter_both_axes,
            scatter_control: parse_control(scatter).context("scatter control")?,
            count: parse_count(brush.scatter_count)?,
            count_jitter_percent: jitter(count),
            count_control: parse_control(count).context("count control")?,
        },
        texture: parameter::Texture {
            enabled: brush.use_texture,
            pattern,
            invert: brush.texture_inverted,
            scale_percent: percent(brush.texture_scale.as_ref(), 100.0),
            brightness: brush.texture_brightness,
            contrast: brush.texture_contrast,
            texture_each_tip: brush.txt_c,
            blend_mode: parse_blend_mode(
                brush
                    .texture_blend_mode
                    .unwrap_or(class::BlendMode::Multiply),
            ),
            depth_percent: percent(brush.texture_depth.as_ref(), 100.0),
            minimum_depth_percent: minimum(depth, brush.texture_minimum_depth.as_ref()),
            depth_jitter_percent: jitter(depth),
            depth_control: parse_control(depth).context("texture depth control")?,
        },
        dual_brush: parameter::DualBrush {
            enabled: dual.enabled,
            blend_mode: parse_blend_mode(dual.blend_mode.unwrap_or(class::BlendMode::Multiply)),
            flip: dual.flip,
            brush_tip: dual_tip,
            scatter_enabled: dual.use_scatter,
            scatter_percent: jitter(dual_scatter),
            both_axes: dual.scatter_both_axes,
            scatter_control: parse_control(dual_scatter).context("dual scatter control")?,
            count: parse_count(dual.scatter_count)?,
            count_jitter_percent: jitter(dual_count),
            count_control: parse_control(dual_count).context("dual count control")?,
        },
        color_dynamics: parameter::ColorDynamics {
            enabled: brush.use_color_dynamics,
            apply_per_tip: brush.color_dynamics_per_tip,
            foreground_background_jitter_percent: jitter(color),
            foreground_background_control: parse_control(color).context("color control")?,
            hue_jitter_percent: percent(brush.hue_jitter.as_ref(), 0.0),
            saturation_jitter_percent: percent(brush.saturation_jitter.as_ref(), 0.0),
            brightness_jitter_percent: percent(brush.value_jitter.as_ref(), 0.0),
            purity_percent: percent(brush.purity_jitter.as_ref(), 0.0),
        },
        transfer: parameter::Transfer {
            enabled: brush.use_paint_dynamics,
            opacity_jitter_percent: jitter(opacity),
            opacity_control: parse_control(opacity).context("opacity control")?,
            minimum_opacity_percent: minimum(opacity, None),
            flow_jitter_percent: jitter(flow),
            flow_control: parse_control(flow).context("flow control")?,
            minimum_flow_percent: minimum(flow, None),
            wetness_jitter_percent: jitter(wetness),
            wetness_control: parse_control(wetness).context("wetness control")?,
            minimum_wetness_percent: minimum(wetness, None),
            mix_jitter_percent: jitter(mix),
            mix_control: parse_control(mix).context("mix control")?,
            minimum_mix_percent: minimum(mix, None),
        },
        brush_pose: parameter::BrushPose {
            enabled: brush.use_brush_pose,
            tilt_x_percent: brush.brush_pose_tilt_x as f32 / 90.0 * 100.0,
            override_tilt_x: brush.override_pose_tilt_x,
            tilt_y_percent: brush.brush_pose_tilt_y as f32 / 90.0 * 100.0,
            override_tilt_y: brush.override_pose_tilt_y,
            rotation_degrees: brush.brush_pose_angle as f32,
            override_rotation: brush.override_pose_angle,
            pressure_percent: percent(brush.brush_pose_pressure.as_ref(), 100.0),
            override_pressure: brush.override_pose_pressure,
        },
        noise: brush.noise,
        wet_edges: brush.wtdg,
        build_up: brush.repeat,
        smoothing: parameter::Smoothing::default(),
        protect_texture: brush.protect_texture,
    };

    match &brush.tool_options {
        Some(ToolOptions::Paint(options)) => {
            result.smoothing = parameter::Smoothing {
                enabled: options.smoothing,
                amount_percent: options.smoothing_value as f32,
                pulled_string_mode: options.smoothing_radius_mode,
                stroke_catch_up: options.smoothing_catchup,
                catch_up_on_stroke_end: options.smoothing_catchup_at_end,
                adjust_for_zoom: options.smoothing_zoom_compensation,
                pressure_smoothing: options.pressure_smoothing,
            };
        }
        Some(ToolOptions::Eraser(options)) => {
            result.smoothing = parameter::Smoothing {
                enabled: options.smoothing,
                amount_percent: options.smoothing_value as f32,
                pulled_string_mode: options.smoothing_radius_mode,
                stroke_catch_up: options.smoothing_catchup,
                catch_up_on_stroke_end: options.smoothing_catchup_at_end,
                adjust_for_zoom: options.smoothing_zoom_compensation,
                pressure_smoothing: options.pressure_smoothing,
            };
        }
        Some(ToolOptions::Smudge(options)) => {
            result.smoothing = parameter::Smoothing {
                enabled: options.smoothing,
                amount_percent: options.smoothing_value as f32,
                pulled_string_mode: options.smoothing_radius_mode,
                stroke_catch_up: options.smoothing_catchup,
                catch_up_on_stroke_end: options.smoothing_catchup_at_end,
                adjust_for_zoom: options.smoothing_zoom_compensation,
                pressure_smoothing: options.pressure_smoothing,
            };
        }
        Some(ToolOptions::Sh(options)) => result.smoothing.enabled = options.smoothing,
        None => {}
    }

    Ok(result)
}
