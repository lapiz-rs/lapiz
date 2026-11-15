use lapiz_assets::asset::AssetId;
use lapiz_image::blend_modes::BlendMode;
use lapiz_render::texture::Image;

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub enum BrushTip {
    Computed(ComputedBrushTip),
    Sampled(SampledBrushTip),
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DynamicsControl {
    Off,
    Fade { steps: u32 },
    PenPressure,
    PenTilt,
    StylusWheel,
    InitialDirection,
    Direction,
    Rotation,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct Scattering {
    pub enabled: bool,
    pub scatter_percent: f32,
    pub both_axes: bool,
    pub scatter_control: DynamicsControl,
    pub count: u32,
    pub count_jitter_percent: f32,
    pub count_control: DynamicsControl,
}

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
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

#[derive(Clone, Debug, PartialEq)]
pub struct Smoothing {
    pub enabled: bool,
    pub amount_percent: f32,
    pub pulled_string_mode: bool,
    pub stroke_catch_up: bool,
    pub catch_up_on_stroke_end: bool,
    pub adjust_for_zoom: bool,
    pub pressure_smoothing: bool,
}
