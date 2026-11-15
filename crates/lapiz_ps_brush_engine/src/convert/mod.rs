use std::f32::consts::FRAC_PI_2;

use anyhow::{Context as _, Result, ensure};
use lapiz_assets::asset::AssetId;
use lapiz_brush::asset::{BrushPreset, BrushPresetMetadata, SerializableBrushParameter};
use lapiz_image::blend_modes::BlendMode;
use lapiz_render::texture::Image;

use crate::{
    convert::{
        graph::{BrushInputs, ComputedMainTip, MainGraphOptions, MainTip, SampledMainTip},
        wgsl::{
            BrushPose, BrushTexture, ColorAdjustment, DualBrush, DualBrushTip, Dynamics, Scatter,
        },
    },
    parameter::{self, BrushTip, DynamicsControl, PhotoshopBrush},
};

mod graph;
mod wgsl;

impl PhotoshopBrush {
    pub fn compile(&self) -> Result<BrushPreset> {
        ensure!(
            !self.wet_edges,
            "wet edges are outside the compiler support range"
        );
        ensure!(
            !self.build_up,
            "build-up is outside the compiler support range"
        );
        ensure!(
            !self.protect_texture,
            "protect texture is outside the compiler support range"
        );
        ensure!(
            !self.smoothing.enabled && !self.smoothing.pressure_smoothing,
            "smoothing is outside the compiler support range"
        );

        let shape = &self.shape_dynamics;
        let (size_dynamics, angle_dynamics, roundness_dynamics, tilt_scale) = if shape.enabled {
            ensure!(
                !shape.brush_projection,
                "brush projection is outside the compiler support range"
            );
            ensure!(
                !matches!(shape.angle_control, DynamicsControl::Fade { .. }),
                "angle fade is outside the compiler support range"
            );
            (
                Some(
                    Dynamics::new(
                        shape.size_control,
                        shape.size_jitter_percent,
                        shape.minimum_diameter_percent,
                    )
                    .context("size dynamics")?,
                ),
                Some(
                    Dynamics::new(shape.angle_control, shape.angle_jitter_percent, 0.0)
                        .context("angle dynamics")?,
                ),
                Some(
                    Dynamics::new(
                        shape.roundness_control,
                        shape.roundness_jitter_percent,
                        shape.minimum_roundness_percent,
                    )
                    .context("roundness dynamics")?,
                ),
                percent(shape.tilt_scale_percent, 0.0, 200.0).context("tilt scale")?,
            )
        } else {
            (None, None, None, 2.0)
        };

        let transfer = &self.transfer;
        let (opacity_dynamics, flow_dynamics) = if transfer.enabled {
            ensure!(
                transfer.wetness_jitter_percent == 0.0
                    && transfer.wetness_control == DynamicsControl::Off
                    && transfer.minimum_wetness_percent == 0.0,
                "wetness dynamics are outside the compiler support range"
            );
            ensure!(
                transfer.mix_jitter_percent == 0.0
                    && transfer.mix_control == DynamicsControl::Off
                    && transfer.minimum_mix_percent == 0.0,
                "mix dynamics are outside the compiler support range"
            );
            (
                Some(
                    Dynamics::new(
                        transfer.opacity_control,
                        transfer.opacity_jitter_percent,
                        transfer.minimum_opacity_percent,
                    )
                    .context("opacity dynamics")?,
                ),
                Some(
                    Dynamics::new(
                        transfer.flow_control,
                        transfer.flow_jitter_percent,
                        transfer.minimum_flow_percent,
                    )
                    .context("flow dynamics")?,
                ),
            )
        } else {
            (None, None)
        };

        let pose = &self.brush_pose;
        let pose = if pose.enabled {
            BrushPose {
                pressure: pose
                    .override_pressure
                    .then(|| percent(pose.pressure_percent, 0.0, 100.0))
                    .transpose()
                    .context("pose pressure")?,
                azimuth: pose
                    .override_rotation
                    .then(|| {
                        ensure!(
                            pose.rotation_degrees.is_finite(),
                            "pose rotation must be finite"
                        );
                        Ok(pose.rotation_degrees.to_radians())
                    })
                    .transpose()?,
                tilt_x: pose
                    .override_tilt_x
                    .then(|| {
                        percent(pose.tilt_x_percent, -100.0, 100.0).map(|tilt| tilt * FRAC_PI_2)
                    })
                    .transpose()
                    .context("pose tilt x")?,
                tilt_y: pose
                    .override_tilt_y
                    .then(|| {
                        percent(pose.tilt_y_percent, -100.0, 100.0).map(|tilt| tilt * FRAC_PI_2)
                    })
                    .transpose()
                    .context("pose tilt y")?,
            }
        } else {
            BrushPose::default()
        };

        let color = &self.color_dynamics;
        let color_adjustment = if color.enabled {
            Some(ColorAdjustment {
                hue_jitter: percent(color.hue_jitter_percent, 0.0, 100.0).context("hue jitter")?,
                saturation_jitter: percent(color.saturation_jitter_percent, 0.0, 100.0)
                    .context("saturation jitter")?,
                value_jitter: percent(color.brightness_jitter_percent, 0.0, 100.0)
                    .context("brightness jitter")?,
                purity: percent(color.purity_percent, -100.0, 100.0).context("purity")?,
                dynamics: Some(
                    Dynamics::new(
                        color.foreground_background_control,
                        color.foreground_background_jitter_percent,
                        0.0,
                    )
                    .context("foreground/background dynamics")?,
                ),
                per_tip: color.apply_per_tip,
                foreground_color: None,
            })
        } else {
            None
        };

        let scattering = &self.scattering;
        let scatter = scattering
            .enabled
            .then(|| {
                Scatter::new(
                    scattering.scatter_percent,
                    scattering.both_axes,
                    scattering.scatter_control,
                    scattering.count,
                    scattering.count_jitter_percent,
                    scattering.count_control,
                )
            })
            .transpose()
            .context("scattering")?;

        let texture = &self.texture;
        let (brush_texture, pattern_asset) = if texture.enabled {
            let pattern = texture
                .pattern
                .context("enabled texture requires a pattern")?;
            ensure!(
                texture.scale_percent.is_finite() && texture.scale_percent > 0.0,
                "texture scale must be finite and positive"
            );
            let contrast = if texture.contrast < 0 {
                texture.contrast as f32 / 50.0
            } else {
                texture.contrast as f32 / 100.0
            }
            .clamp(-1.0, 1.0);
            (
                Some(BrushTexture {
                    scale: texture.scale_percent / 100.0,
                    inverted: texture.invert,
                    depth: percent(texture.depth_percent, 0.0, 100.0).context("texture depth")?,
                    depth_dynamics: Some(
                        Dynamics::new(
                            texture.depth_control,
                            texture.depth_jitter_percent,
                            texture.minimum_depth_percent,
                        )
                        .context("texture depth dynamics")?,
                    ),
                    each_tip: texture.texture_each_tip,
                    blend_mode: texture.blend_mode,
                    brightness: (texture.brightness as f32 / 150.0).clamp(-1.0, 1.0),
                    contrast,
                    use_legacy: false,
                }),
                Some(pattern),
            )
        } else {
            (None, None)
        };

        let (dual_brush, dual_sample_asset) = self.dual_brush.compile()?;
        let options = MainGraphOptions {
            flow: 1.0,
            size_dynamics,
            opacity_dynamics,
            flow_dynamics,
            angle_dynamics,
            roundness_dynamics,
            tilt_scale,
            flip_x_jitter: shape.enabled && shape.flip_x_jitter,
            flip_y_jitter: shape.enabled && shape.flip_y_jitter,
            pose,
            color_adjustment,
            scatter,
            brush_texture,
            pattern_asset,
            noise: self.noise,
            dual_brush,
            dual_sample_asset,
        };

        let (tip, tip_asset) = self.brush_tip.compile()?;
        let size = match tip {
            MainTip::Computed(tip) => tip.diameter,
            MainTip::Sampled(tip) => tip.diameter,
        };
        let inputs = BrushInputs::new(
            size,
            tip_asset,
            options.pattern_asset,
            options.dual_sample_asset,
        )?;
        let (spacing_effect, main_effect) = match tip {
            MainTip::Computed(tip) => graph::computed_graphs(tip, options, &inputs)?,
            MainTip::Sampled(tip) => graph::sampled_graphs(tip, options, &inputs)?,
        };
        let postprocess_effect =
            graph::opacity_postprocess_effect(1.0, BlendMode::Normal, &inputs)?;

        Ok(BrushPreset {
            metadata: BrushPresetMetadata {
                name: self.name.clone(),
            },
            spacing_effect,
            main_effect,
            postprocess_effect,
            parameters: inputs
                .slots
                .into_iter()
                .map(|input| {
                    (
                        input.id,
                        SerializableBrushParameter {
                            name: input.name,
                            value: input.value,
                        },
                    )
                })
                .collect(),
        })
    }
}

fn percent(value: f32, minimum: f32, maximum: f32) -> Result<f32> {
    ensure!(
        value.is_finite() && (minimum..=maximum).contains(&value),
        "percentage must be finite and within {minimum}..={maximum}, got {value}"
    );
    Ok(value / 100.0)
}

impl Dynamics {
    fn new(control: DynamicsControl, jitter_percent: f32, minimum_percent: f32) -> Result<Self> {
        ensure!(
            control != DynamicsControl::StylusWheel,
            "stylus wheel is outside the compiler support range"
        );
        Ok(Self {
            control,
            jitter: percent(jitter_percent, 0.0, 100.0)?,
            minimum: percent(minimum_percent, 0.0, 100.0)?,
        })
    }
}

impl Scatter {
    fn new(
        scatter_percent: f32,
        both_axes: bool,
        scatter_control: DynamicsControl,
        count: u32,
        count_jitter_percent: f32,
        count_control: DynamicsControl,
    ) -> Result<Self> {
        ensure!(
            (1..=16).contains(&count),
            "scatter count must be within 1..=16, got {count}"
        );
        let amount = percent(scatter_percent, 0.0, 1000.0)?;
        let mut scatter_dynamics = Dynamics::new(scatter_control, 0.0, 0.0)?;
        scatter_dynamics.jitter = amount;

        Ok(Self {
            both_axes,
            scatter_dynamics: Some(scatter_dynamics),
            count,
            count_dynamics: Some(Dynamics::new(count_control, count_jitter_percent, 0.0)?),
        })
    }
}

impl BrushTip {
    fn compile(&self) -> Result<(MainTip, Option<AssetId<Image>>)> {
        let (tip, image) = match self {
            Self::Computed(tip) => {
                ensure!(
                    tip.spacing_enabled,
                    "computed tips require spacing to be enabled"
                );
                (
                    MainTip::Computed(ComputedMainTip {
                        diameter: tip.size_pixels,
                        hardness: percent(tip.hardness_percent, 0.0, 100.0).context("hardness")?,
                        angle: tip.angle_degrees.to_radians(),
                        roundness: percent(tip.roundness_percent, 0.0, 100.0)
                            .context("roundness")?
                            .max(0.001),
                        flip_x: tip.flip_x,
                        flip_y: tip.flip_y,
                        spacing: tip.spacing_percent / 100.0,
                    }),
                    None,
                )
            }
            Self::Sampled(tip) => {
                ensure!(
                    tip.spacing_enabled,
                    "sampled tips require spacing to be enabled"
                );
                (
                    MainTip::Sampled(SampledMainTip {
                        diameter: tip.size_pixels,
                        angle: tip.angle_degrees.to_radians(),
                        roundness: percent(tip.roundness_percent, 0.0, 100.0)
                            .context("roundness")?
                            .max(0.001),
                        flip_x: tip.flip_x,
                        flip_y: tip.flip_y,
                        spacing: tip.spacing_percent / 100.0,
                    }),
                    Some(tip.image),
                )
            }
        };
        let (diameter, angle, spacing) = match tip {
            MainTip::Computed(tip) => (tip.diameter, tip.angle, tip.spacing),
            MainTip::Sampled(tip) => (tip.diameter, tip.angle, tip.spacing),
        };
        ensure!(
            diameter.is_finite() && (0.1..=1000.0).contains(&diameter),
            "tip size must be finite and within 0.1..=1000 pixels"
        );
        ensure!(angle.is_finite(), "tip angle must be finite");
        ensure!(
            spacing.is_finite() && spacing >= 0.001,
            "tip spacing must be finite and at least 0.1 percent"
        );

        Ok((tip, image))
    }
}

impl parameter::DualBrush {
    fn compile(&self) -> Result<(Option<DualBrush>, Option<AssetId<Image>>)> {
        if !self.enabled {
            return Ok((None, None));
        }

        let (tip, image) = self.brush_tip.compile().context("dual brush tip")?;
        let (tip, spacing) = match tip {
            MainTip::Computed(tip) => (
                DualBrushTip::Computed {
                    diameter: tip.diameter,
                    hardness: tip.hardness,
                    angle: tip.angle,
                    roundness: tip.roundness,
                    flip_x: tip.flip_x,
                    flip_y: tip.flip_y,
                },
                tip.spacing,
            ),
            MainTip::Sampled(tip) => (
                DualBrushTip::Sampled {
                    diameter: tip.diameter,
                    angle: tip.angle,
                    roundness: tip.roundness,
                    flip_x: tip.flip_x,
                    flip_y: tip.flip_y,
                },
                tip.spacing,
            ),
        };
        let scatter = self
            .scatter_enabled
            .then(|| {
                Scatter::new(
                    self.scatter_percent,
                    self.both_axes,
                    self.scatter_control,
                    self.count,
                    self.count_jitter_percent,
                    self.count_control,
                )
            })
            .transpose()
            .context("dual brush scattering")?;

        Ok((
            Some(DualBrush {
                tip,
                flip: self.flip,
                blend_mode: self.blend_mode,
                spacing,
                scatter,
            }),
            image,
        ))
    }
}
