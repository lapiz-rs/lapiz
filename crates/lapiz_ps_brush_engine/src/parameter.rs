use lapiz_assets::asset::AssetId;
use lapiz_image::blend_modes::BlendMode;
use lapiz_render::texture::Image;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PhotoshopBrush {
    pub name: String,
    pub brush_tip: BrushTip,
    pub shape_dynamics: ShapeDynamics,
    pub scattering: Scattering,
    pub texture: Texture,
    pub dual_brush: DualBrush,
    pub color_dynamics: ColorDynamics,
    pub transfer: Transfer,
    pub brush_pose: BrushPose,
    pub noise: bool,
    pub wet_edges: bool,
    pub build_up: bool,
    pub smoothing: Smoothing,
    pub protect_texture: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum BrushTip {
    Computed(ComputedBrushTip),
    Sampled(SampledBrushTip),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ComputedBrushTip {
    pub size_pixels: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub angle_degrees: f32,
    pub roundness_percent: f32,
    pub hardness_percent: f32,
    pub spacing_enabled: bool,
    pub spacing_percent: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SampledBrushTip {
    pub image: AssetId<Image>,
    pub size_pixels: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    pub angle_degrees: f32,
    pub roundness_percent: f32,
    pub spacing_enabled: bool,
    pub spacing_percent: f32,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum DynamicsControl {
    #[default]
    Off,
    Fade {
        steps: u32,
    },
    PenPressure,
    PenTilt,
    StylusWheel,
    InitialDirection,
    Direction,
    Rotation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ShapeDynamics {
    pub enabled: bool,
    pub size_jitter_percent: f32,
    pub size_control: DynamicsControl,
    pub minimum_diameter_percent: f32,
    pub tilt_scale_percent: f32,
    pub angle_jitter_percent: f32,
    pub angle_control: DynamicsControl,
    pub roundness_jitter_percent: f32,
    pub roundness_control: DynamicsControl,
    pub minimum_roundness_percent: f32,
    pub flip_x_jitter: bool,
    pub flip_y_jitter: bool,
    pub brush_projection: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Scattering {
    pub enabled: bool,
    pub scatter_percent: f32,
    pub both_axes: bool,
    pub scatter_control: DynamicsControl,
    pub count: u32,
    pub count_jitter_percent: f32,
    pub count_control: DynamicsControl,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Texture {
    pub enabled: bool,
    pub pattern: Option<AssetId<Image>>,
    pub invert: bool,
    pub scale_percent: f32,
    pub brightness: i32,
    pub contrast: i32,
    pub texture_each_tip: bool,
    pub blend_mode: BlendMode,
    pub depth_percent: f32,
    pub minimum_depth_percent: f32,
    pub depth_jitter_percent: f32,
    pub depth_control: DynamicsControl,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DualBrush {
    pub enabled: bool,
    pub blend_mode: BlendMode,
    pub flip: bool,
    pub brush_tip: BrushTip,
    pub scatter_enabled: bool,
    pub scatter_percent: f32,
    pub both_axes: bool,
    pub scatter_control: DynamicsControl,
    pub count: u32,
    pub count_jitter_percent: f32,
    pub count_control: DynamicsControl,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ColorDynamics {
    pub enabled: bool,
    pub apply_per_tip: bool,
    pub foreground_background_jitter_percent: f32,
    pub foreground_background_control: DynamicsControl,
    pub hue_jitter_percent: f32,
    pub saturation_jitter_percent: f32,
    pub brightness_jitter_percent: f32,
    pub purity_percent: f32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Transfer {
    pub enabled: bool,
    pub opacity_jitter_percent: f32,
    pub opacity_control: DynamicsControl,
    pub minimum_opacity_percent: f32,
    pub flow_jitter_percent: f32,
    pub flow_control: DynamicsControl,
    pub minimum_flow_percent: f32,
    pub wetness_jitter_percent: f32,
    pub wetness_control: DynamicsControl,
    pub minimum_wetness_percent: f32,
    pub mix_jitter_percent: f32,
    pub mix_control: DynamicsControl,
    pub minimum_mix_percent: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BrushPose {
    pub enabled: bool,
    pub tilt_x_percent: f32,
    pub override_tilt_x: bool,
    pub tilt_y_percent: f32,
    pub override_tilt_y: bool,
    pub rotation_degrees: f32,
    pub override_rotation: bool,
    pub pressure_percent: f32,
    pub override_pressure: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Smoothing {
    pub enabled: bool,
    pub amount_percent: f32,
    pub pulled_string_mode: bool,
    pub stroke_catch_up: bool,
    pub catch_up_on_stroke_end: bool,
    pub adjust_for_zoom: bool,
    pub pressure_smoothing: bool,
}

impl Default for PhotoshopBrush {
    fn default() -> Self {
        Self {
            name: "[Unnamed Brush]".into(),
            brush_tip: BrushTip::default(),
            shape_dynamics: ShapeDynamics::default(),
            scattering: Scattering::default(),
            texture: Texture::default(),
            dual_brush: DualBrush::default(),
            color_dynamics: ColorDynamics::default(),
            transfer: Transfer::default(),
            brush_pose: BrushPose::default(),
            noise: false,
            wet_edges: false,
            build_up: false,
            smoothing: Smoothing::default(),
            protect_texture: false,
        }
    }
}

impl Default for BrushTip {
    fn default() -> Self {
        Self::Computed(ComputedBrushTip::default())
    }
}

impl Default for ComputedBrushTip {
    fn default() -> Self {
        Self {
            size_pixels: 30.0,
            flip_x: false,
            flip_y: false,
            angle_degrees: 0.0,
            roundness_percent: 100.0,
            hardness_percent: 100.0,
            spacing_enabled: true,
            spacing_percent: 25.0,
        }
    }
}

impl Default for ShapeDynamics {
    fn default() -> Self {
        Self {
            enabled: false,
            size_jitter_percent: 0.0,
            size_control: DynamicsControl::Off,
            minimum_diameter_percent: 0.0,
            tilt_scale_percent: 200.0,
            angle_jitter_percent: 0.0,
            angle_control: DynamicsControl::Off,
            roundness_jitter_percent: 0.0,
            roundness_control: DynamicsControl::Off,
            minimum_roundness_percent: 0.0,
            flip_x_jitter: false,
            flip_y_jitter: false,
            brush_projection: false,
        }
    }
}

impl Default for Scattering {
    fn default() -> Self {
        Self {
            enabled: false,
            scatter_percent: 0.0,
            both_axes: false,
            scatter_control: DynamicsControl::Off,
            count: 1,
            count_jitter_percent: 0.0,
            count_control: DynamicsControl::Off,
        }
    }
}

impl Default for Texture {
    fn default() -> Self {
        Self {
            enabled: false,
            pattern: None,
            invert: false,
            scale_percent: 100.0,
            brightness: 0,
            contrast: 0,
            texture_each_tip: false,
            blend_mode: BlendMode::Multiply,
            depth_percent: 100.0,
            minimum_depth_percent: 0.0,
            depth_jitter_percent: 0.0,
            depth_control: DynamicsControl::Off,
        }
    }
}

impl Default for DualBrush {
    fn default() -> Self {
        Self {
            enabled: false,
            blend_mode: BlendMode::Multiply,
            flip: false,
            brush_tip: BrushTip::default(),
            scatter_enabled: false,
            scatter_percent: 0.0,
            both_axes: false,
            scatter_control: DynamicsControl::Off,
            count: 1,
            count_jitter_percent: 0.0,
            count_control: DynamicsControl::Off,
        }
    }
}

impl Default for BrushPose {
    fn default() -> Self {
        Self {
            enabled: false,
            tilt_x_percent: 0.0,
            override_tilt_x: false,
            tilt_y_percent: 0.0,
            override_tilt_y: false,
            rotation_degrees: 0.0,
            override_rotation: false,
            pressure_percent: 100.0,
            override_pressure: false,
        }
    }
}

impl Default for Smoothing {
    fn default() -> Self {
        Self {
            enabled: false,
            amount_percent: 0.0,
            pulled_string_mode: false,
            stroke_catch_up: true,
            catch_up_on_stroke_end: false,
            adjust_for_zoom: true,
            pressure_smoothing: false,
        }
    }
}
