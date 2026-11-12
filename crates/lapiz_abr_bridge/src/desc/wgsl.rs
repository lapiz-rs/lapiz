use std::{
    f32::consts::{FRAC_1_SQRT_2, FRAC_PI_4},
    sync::LazyLock,
};

use lapiz_abr::descriptor::class::DynamicsControl;
use lapiz_image::blend_modes::BlendMode;
#[allow(
    clippy::wildcard_imports,
    reason = "wesl quote macros resolve syntax types from this prelude"
)]
use wesl::syntax::*;
use wesl_quote::{quote_expression, quote_statement};

use crate::desc::graph::{ComputedMainTip, MainGraphOptions, SampledMainTip};

pub const USER_SIZE: &str = "user_size";
pub const USER_OPACITY: &str = "user_opacity";
pub const USER_FLOW: &str = "user_flow";

pub const MAIN_PIXEL_POSITION_INPUT: &str = "pixel_position";
pub const MAIN_PEN_POSITION_INPUT: &str = "pen_position";
pub const MAIN_FOREGROUND_COLOR_INPUT: &str = "foreground_color";
pub const MAIN_BACKGROUND_COLOR_INPUT: &str = "background_color";
pub const MAIN_CURRENT_COLOR_INPUT: &str = "current_color";
pub const MAIN_TIP_TEXTURE_INPUT: &str = "tip_texture";
pub const MAIN_PATTERN_TEXTURE_INPUT: &str = "pattern_texture";
pub const DUAL_TIP_TEXTURE_INPUT: &str = "dual_tip_texture";
pub const MAIN_COLOR_OUTPUT: &str = "color";
pub const MAIN_BOUNDS_OUTPUT: &str = "bounds";
pub const POSTPROCESS_INPUT_COLOR: &str = "input_color";
pub const POSTPROCESS_STROKE_BOUNDS_INPUT: &str = "stroke_bounds";
pub const POSTPROCESS_TARGET_COLOR_INPUT: &str = "target_color";
pub const REQUIRED_SPACING_OUTPUT: &str = "required_spacing";
pub const PRESSURE_INPUT: &str = "pressure";
pub const TILT_INPUT: &str = "tilt";
pub const AZIMUTH_INPUT: &str = "azimuth";
pub const DIRECTION_INPUT: &str = "direction";
pub const INITIAL_DIRECTION_INPUT: &str = "initial_direction";
pub const DAB_INDEX_INPUT: &str = "dab_index";
pub const STROKE_DISTANCE_INPUT: &str = "stroke_distance";
pub const STROKE_BEGIN_INPUT: &str = "stroke_begin";

/// Normalized property dynamics parameters used to emit WGSL modulation expressions.
#[derive(Clone, Copy)]
pub struct Dynamics {
    pub control: DynamicsControl,
    pub fade_steps: i32,
    pub jitter: f32,
    pub minimum: f32,
}

/// Optional static overrides for pen pressure, azimuth, and tilt angles in generated WGSL shaders.
#[derive(Clone, Copy, Default)]
pub struct BrushPose {
    pub pressure: Option<f32>,
    pub azimuth: Option<f32>,
    pub tilt_x: Option<f32>,
    pub tilt_y: Option<f32>,
}

/// Normalized dab scattering parameters used to emit per-copy offset and count loops in WGSL.
#[derive(Clone, Copy)]
pub struct Scatter {
    pub both_axes: bool,
    pub scatter_dynamics: Option<Dynamics>,
    pub count: u32,
    pub count_dynamics: Option<Dynamics>,
}

/// Normalized pattern texture modulation settings used to emit WGSL texture sampling and blending code.
#[derive(Clone, Copy)]
pub struct BrushTexture {
    pub scale: f32,
    pub inverted: bool,
    pub depth: f32,
    pub depth_dynamics: Option<Dynamics>,
    pub each_tip: bool,
    pub blend_mode: BlendMode,
    pub brightness: f32,
    pub contrast: f32,
    pub use_legacy: bool,
}

/// Normalized secondary brush tip shape and parameters used when emitting dual-brush WGSL code.
#[derive(Clone, Copy)]
pub enum DualBrushTip {
    Computed {
        diameter: f32,
        hardness: f32,
        angle: f32,
        roundness: f32,
        flip_x: bool,
        flip_y: bool,
    },
    Sampled {
        diameter: f32,
        angle: f32,
        roundness: f32,
        flip_x: bool,
        flip_y: bool,
    },
}

/// Normalized secondary brush configuration used to emit dual-brush mask evaluation in WGSL.
#[derive(Clone, Copy)]
pub struct DualBrush {
    pub tip: DualBrushTip,
    pub flip: bool,
    pub blend_mode: BlendMode,
    pub spacing: f32,
    pub scatter: Option<Scatter>,
}

/// Normalized foreground/background color dynamics and HSV/purity jitter settings for WGSL code generation.
#[derive(Clone, Copy)]
pub struct ColorAdjustment {
    pub hue_jitter: f32,
    pub saturation_jitter: f32,
    pub value_jitter: f32,
    pub purity: f32,
    pub dynamics: Option<Dynamics>,
    pub per_tip: bool,
    pub foreground_color: Option<[f32; 3]>,
}

// TODO: This is really ugly, can we avoid this?
static USER_SIZE_IDENT: LazyLock<Ident> = LazyLock::new(|| Ident::new(USER_SIZE.to_string()));
static USER_OPACITY_IDENT: LazyLock<Ident> = LazyLock::new(|| Ident::new(USER_OPACITY.to_string()));
static USER_FLOW_IDENT: LazyLock<Ident> = LazyLock::new(|| Ident::new(USER_FLOW.to_string()));

static MAIN_PIXEL_POSITION_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_PIXEL_POSITION_INPUT.to_string()));
static MAIN_PEN_POSITION_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_PEN_POSITION_INPUT.to_string()));
static MAIN_FOREGROUND_COLOR_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_FOREGROUND_COLOR_INPUT.to_string()));
static MAIN_BACKGROUND_COLOR_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_BACKGROUND_COLOR_INPUT.to_string()));
static MAIN_CURRENT_COLOR_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_CURRENT_COLOR_INPUT.to_string()));
static MAIN_TIP_TEXTURE_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_TIP_TEXTURE_INPUT.to_string()));
static MAIN_PATTERN_TEXTURE_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_PATTERN_TEXTURE_INPUT.to_string()));
static DUAL_TIP_TEXTURE_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(DUAL_TIP_TEXTURE_INPUT.to_string()));
static MAIN_COLOR_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_COLOR_OUTPUT.to_string()));
static MAIN_BOUNDS_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(MAIN_BOUNDS_OUTPUT.to_string()));
static POSTPROCESS_INPUT_COLOR_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(POSTPROCESS_INPUT_COLOR.to_string()));
static POSTPROCESS_STROKE_BOUNDS_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(POSTPROCESS_STROKE_BOUNDS_INPUT.to_string()));
static POSTPROCESS_TARGET_COLOR_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(POSTPROCESS_TARGET_COLOR_INPUT.to_string()));
static REQUIRED_SPACING_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(REQUIRED_SPACING_OUTPUT.to_string()));
static PRESSURE_IDENT: LazyLock<Ident> = LazyLock::new(|| Ident::new(PRESSURE_INPUT.to_string()));
static TILT_IDENT: LazyLock<Ident> = LazyLock::new(|| Ident::new(TILT_INPUT.to_string()));
static AZIMUTH_IDENT: LazyLock<Ident> = LazyLock::new(|| Ident::new(AZIMUTH_INPUT.to_string()));
static DIRECTION_IDENT: LazyLock<Ident> = LazyLock::new(|| Ident::new(DIRECTION_INPUT.to_string()));
static INITIAL_DIRECTION_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(INITIAL_DIRECTION_INPUT.to_string()));
static DAB_INDEX_IDENT: LazyLock<Ident> = LazyLock::new(|| Ident::new(DAB_INDEX_INPUT.to_string()));
static STROKE_DISTANCE_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(STROKE_DISTANCE_INPUT.to_string()));
static STROKE_BEGIN_IDENT: LazyLock<Ident> =
    LazyLock::new(|| Ident::new(STROKE_BEGIN_INPUT.to_string()));

fn pose_pressure(pose: BrushPose) -> Expression {
    match pose.pressure {
        Some(pressure) => quote_expression!(#pressure * 1.0),
        None => {
            let pressure = (*PRESSURE_IDENT).clone();
            quote_expression!(#pressure)
        }
    }
}

fn pose_azimuth(pose: BrushPose) -> Expression {
    match pose.azimuth {
        Some(azimuth) => quote_expression!(#azimuth * 1.0),
        None => {
            let azimuth = (*AZIMUTH_IDENT).clone();
            quote_expression!(#azimuth)
        }
    }
}

fn pose_tilt_components(pose: BrushPose) -> (Expression, Expression) {
    let tilt = (*TILT_IDENT).clone();
    let tilt_x = match pose.tilt_x {
        Some(tilt_x) => quote_expression!(#tilt_x * 1.0),
        None => quote_expression!(#tilt.x),
    };
    let tilt_y = match pose.tilt_y {
        Some(tilt_y) => quote_expression!(#tilt_y * 1.0),
        None => quote_expression!(#tilt.y),
    };
    (tilt_x, tilt_y)
}

fn pose_tilt(pose: BrushPose) -> Expression {
    let (tilt_x, tilt_y) = pose_tilt_components(pose);
    quote_expression!(vec2f(#tilt_x, #tilt_y))
}

fn pose_tilt_angle(pose: BrushPose) -> Expression {
    let (tilt_x, tilt_y) = pose_tilt_components(pose);
    quote_expression!(atan2(#tilt_y, #tilt_x))
}

fn dab_random(random_offset: f32) -> Expression {
    let dab_index = (*DAB_INDEX_IDENT).clone();
    quote_expression!(fract(
        sin(f32(#dab_index) * 12.9898 + #random_offset) * 43758.5453
    ))
}

fn stroke_random(random_offset: f32) -> Expression {
    let stroke_begin = (*STROKE_BEGIN_IDENT).clone();
    quote_expression!(fract(
        sin(#stroke_begin * 12.9898 + #random_offset) * 43758.5453
    ))
}

fn scatter_copy_random(random_offset: f32) -> Expression {
    let dab_index = (*DAB_INDEX_IDENT).clone();
    quote_expression!(fract(
        sin(
            f32(#dab_index) * 12.9898
                + f32(copy_index) * 78.233
                + #random_offset
        ) * 43758.5453
    ))
}

fn flip_jitter_component(jitter: bool, random_offset: f32) -> Expression {
    if jitter {
        let random = scatter_copy_random(random_offset);
        quote_expression!(select(1.0, -1.0, #random < 0.5))
    } else {
        quote_expression!(1.0)
    }
}

fn flip_jitter_scale(flip_x_jitter: bool, flip_y_jitter: bool) -> Expression {
    let x = flip_jitter_component(flip_x_jitter, 311.113);
    let y = flip_jitter_component(flip_y_jitter, 317.119);
    quote_expression!(vec2f(#x, #y))
}

fn pixel_random(random_offset: f32) -> Expression {
    let pixel = (*MAIN_PIXEL_POSITION_IDENT).clone();
    let dab_index = (*DAB_INDEX_IDENT).clone();
    quote_expression!(fract(
        sin(
            dot(#pixel, vec2f(12.9898, 78.233))
                + f32(#dab_index) * 37.719
                + f32(copy_index) * 19.193
                + #random_offset
        ) * 43758.5453
    ))
}

fn noise_statement(noise: bool) -> Statement {
    if !noise {
        return quote_statement! {{}};
    }
    let random = pixel_random(211.327);
    quote_statement! {
        copy_mask = select(0.0, 1.0, #random < copy_mask);
    }
}

fn color_random(per_tip: bool, random_offset: f32) -> Expression {
    if per_tip {
        dab_random(random_offset)
    } else {
        stroke_random(random_offset)
    }
}

fn dynamics_control(dynamics: Dynamics, pose: BrushPose) -> Expression {
    let pressure = pose_pressure(pose);
    let tilt = pose_tilt(pose);
    let azimuth = pose_azimuth(pose);
    let direction = (*DIRECTION_IDENT).clone();
    let initial_direction = (*INITIAL_DIRECTION_IDENT).clone();
    let dab_index = (*DAB_INDEX_IDENT).clone();
    match dynamics.control {
        DynamicsControl::Off => quote_expression!(1.0),
        DynamicsControl::Fade if dynamics.fade_steps <= 0 => quote_expression!(0.0),
        DynamicsControl::Fade => {
            let fade_steps = dynamics.fade_steps as f32;
            quote_expression!(
                1.0 - clamp(f32(#dab_index) / #fade_steps, 0.0, 1.0)
            )
        }
        DynamicsControl::PenPressure => quote_expression!(clamp(#pressure, 0.0, 1.0)),
        DynamicsControl::PenTilt => {
            quote_expression!(clamp(length(#tilt) / render::math::FRAC_PI_2, 0.0, 1.0))
        }
        DynamicsControl::InitialDirection => {
            quote_expression!(clamp(#initial_direction / render::math::TAU + 0.5, 0.0, 1.0))
        }
        DynamicsControl::Direction => {
            quote_expression!(clamp(#direction / render::math::TAU + 0.5, 0.0, 1.0))
        }
        DynamicsControl::Rotation => {
            quote_expression!(fract(#azimuth / render::math::TAU + 1.0))
        }
        DynamicsControl::StylusWheel => unreachable!(),
    }
}

fn dynamics_factor(dynamics: Dynamics, random_offset: f32, pose: BrushPose) -> Expression {
    let control = dynamics_control(dynamics, pose);
    let jitter = dynamics.jitter;
    let minimum = dynamics.minimum;
    let random = dab_random(random_offset);

    quote_expression!(
        mix(#minimum, 1.0, #control) * (1.0 - #jitter * #random)
    )
}

fn color_dynamics_factor(dynamics: Dynamics, per_tip: bool, pose: BrushPose) -> Expression {
    let control = dynamics_control(dynamics, pose);
    let minimum = dynamics.minimum;
    let jitter = dynamics.jitter;
    let random = if per_tip {
        dab_random(89.417)
    } else {
        stroke_random(89.417)
    };
    quote_expression!(
        mix(#minimum, 1.0, #control) * (1.0 - #jitter * #random)
    )
}

fn scatter_dynamics_factor(dynamics: Dynamics, pose: BrushPose) -> Expression {
    let control = dynamics_control(dynamics, pose);
    let amount = dynamics.jitter * 0.5;
    let minimum = dynamics.minimum;
    quote_expression!(#amount * mix(#minimum, 1.0, #control))
}

fn scatter_offset_expression(scatter: Option<Scatter>, pose: BrushPose) -> Expression {
    let Some(scatter) = scatter else {
        return quote_expression!(vec2f(0.0));
    };
    let factor = match scatter.scatter_dynamics {
        Some(dynamics) => scatter_dynamics_factor(dynamics, pose),
        None => quote_expression!(0.0),
    };
    let first_random = scatter_copy_random(127.413);

    if scatter.both_axes {
        let second_random = scatter_copy_random(149.819);
        quote_expression!(
            vec2f(
                cos(#first_random * render::math::TAU),
                sin(#first_random * render::math::TAU),
            ) * #second_random
                * tip_diameter
                * #factor
        )
    } else {
        let direction = (*DIRECTION_IDENT).clone();
        quote_expression!(
            vec2f(-sin(#direction), cos(#direction))
                * (#first_random * 2.0 - 1.0)
                * tip_diameter
                * #factor
        )
    }
}

fn scatter_count_expression(scatter: Option<Scatter>, pose: BrushPose) -> Expression {
    let Some(scatter) = scatter else {
        return quote_expression!(1u);
    };
    let count = scatter.count.max(1) as f32;
    let Some(dynamics) = scatter.count_dynamics else {
        return quote_expression!(u32(#count));
    };
    let factor = dynamics_factor(dynamics, 173.531, pose);
    quote_expression!(max(
        1u,
        min(u32(#count), u32(round(#count * #factor))),
    ))
}

fn dual_copy_random(random_offset: f32) -> Expression {
    quote_expression!(fract(
        sin(
            f32(dual_dab_index) * 12.9898
                + f32(dual_copy_index) * 78.233
                + #random_offset
        ) * 43758.5453
    ))
}

fn dual_scatter_offset_expression(scatter: Option<Scatter>, pose: BrushPose) -> Expression {
    let Some(scatter) = scatter else {
        return quote_expression!(vec2f(0.0));
    };
    let factor = match scatter.scatter_dynamics {
        Some(dynamics) => scatter_dynamics_factor(dynamics, pose),
        None => quote_expression!(0.0),
    };
    let first_random = dual_copy_random(239.117);
    if scatter.both_axes {
        let second_random = dual_copy_random(251.173);
        quote_expression!(
            vec2f(
                cos(#first_random * render::math::TAU),
                sin(#first_random * render::math::TAU),
            ) * #second_random
                * dual_diameter
                * #factor
        )
    } else {
        let direction = (*DIRECTION_IDENT).clone();
        quote_expression!(
            vec2f(-sin(#direction), cos(#direction))
                * (#first_random * 2.0 - 1.0)
                * dual_diameter
                * #factor
        )
    }
}

fn dual_scatter_count_expression(scatter: Option<Scatter>, pose: BrushPose) -> Expression {
    let Some(scatter) = scatter else {
        return quote_expression!(1u);
    };
    let count = scatter.count.max(1) as f32;
    let Some(dynamics) = scatter.count_dynamics else {
        return quote_expression!(u32(#count));
    };
    let factor = dynamics_factor(dynamics, 263.719, pose);
    quote_expression!(max(
        1u,
        min(u32(#count), u32(round(#count * #factor))),
    ))
}

fn size_diameter_statement(dynamics: Option<Dynamics>, pose: BrushPose) -> Statement {
    let user_size = (*USER_SIZE_IDENT).clone();
    let Some(dynamics) = dynamics else {
        return quote_statement! {
            tip_diameter = clamp(#user_size, 0.1, 1000.0);
        };
    };
    let factor = dynamics_factor(dynamics, 78.233, pose);
    let minimum = dynamics.minimum;

    quote_statement! {
        tip_diameter = clamp(#user_size, 0.1, 1000.0)
            * clamp(#factor, #minimum, 1.0);
    }
}

fn tip_roundness_statement(
    roundness: f32,
    dynamics: Option<Dynamics>,
    tilt_scale: f32,
    pose: BrushPose,
) -> Statement {
    let Some(dynamics) = dynamics else {
        return quote_statement! {{
            tip_roundness = #roundness;
            roundness_angle = 0.0;
        }};
    };
    let tilt = pose_tilt(pose);
    let tilt_angle = pose_tilt_angle(pose);
    let (control, angle) = if dynamics.control == DynamicsControl::PenTilt {
        (
            quote_expression!(
                1.0
                    - clamp(
                        length(#tilt) / render::math::FRAC_PI_2 * #tilt_scale,
                        0.0,
                        1.0,
                    )
            ),
            tilt_angle,
        )
    } else {
        (dynamics_control(dynamics, pose), quote_expression!(0.0))
    };
    let jitter = dynamics.jitter;
    let minimum = dynamics.minimum.min(roundness);
    let random = dab_random(91.417);

    quote_statement! {{
        tip_roundness = clamp(
            mix(#minimum, #roundness, #control)
                * (1.0 - #jitter * #random),
            #minimum,
            #roundness,
        );
        roundness_angle = #angle;
    }}
}

fn tip_angle_expression(angle: f32, dynamics: Option<Dynamics>, pose: BrushPose) -> Expression {
    let Some(dynamics) = dynamics else {
        return quote_expression!(#angle + 0.0);
    };
    let pressure = pose_pressure(pose);
    let tilt = pose_tilt(pose);
    let azimuth = pose_azimuth(pose);
    let direction = (*DIRECTION_IDENT).clone();
    let initial_direction = (*INITIAL_DIRECTION_IDENT).clone();
    let minimum = dynamics.minimum;
    let (orientation, amplitude) = match dynamics.control {
        DynamicsControl::Off => (quote_expression!(0.0), quote_expression!(1.0)),
        DynamicsControl::PenPressure => (
            quote_expression!(0.0),
            quote_expression!(mix(
                #minimum,
                1.0,
                clamp(#pressure, 0.0, 1.0),
            )),
        ),
        DynamicsControl::PenTilt => (
            quote_expression!(0.0),
            quote_expression!(mix(
                #minimum,
                1.0,
                clamp(length(#tilt) / render::math::FRAC_PI_2, 0.0, 1.0),
            )),
        ),
        DynamicsControl::InitialDirection => (
            quote_expression!(#initial_direction),
            quote_expression!(1.0),
        ),
        DynamicsControl::Direction => (quote_expression!(#direction), quote_expression!(1.0)),
        DynamicsControl::Rotation => (quote_expression!(#azimuth + 0.0), quote_expression!(1.0)),
        DynamicsControl::Fade | DynamicsControl::StylusWheel => unreachable!(),
    };
    let jitter = dynamics.jitter;
    let random = dab_random(59.719);

    quote_expression!(
        #angle
            + #orientation
            + (#random - 0.5) * render::math::TAU * #jitter * #amplitude
    )
}

fn tip_foreground_statement(adjustment: Option<ColorAdjustment>, pose: BrushPose) -> Statement {
    let runtime_foreground = (*MAIN_FOREGROUND_COLOR_IDENT).clone();
    let Some(adjustment) = adjustment else {
        return quote_statement! {
            tip_foreground = #runtime_foreground;
        };
    };
    let foreground = match adjustment.foreground_color {
        Some([red, green, blue]) => quote_expression!(vec4f(#red, #green, #blue, 1.0)),
        None => quote_expression!(#runtime_foreground),
    };
    let base_foreground = match adjustment.dynamics {
        Some(dynamics) => {
            let background = (*MAIN_BACKGROUND_COLOR_IDENT).clone();
            let factor = color_dynamics_factor(dynamics, adjustment.per_tip, pose);
            quote_expression!(mix(#background, #foreground, #factor))
        }
        None => foreground,
    };
    let hue_offset = if adjustment.hue_jitter > 0.0 {
        let random = color_random(adjustment.per_tip, 17.131);
        let jitter = adjustment.hue_jitter;
        quote_expression!((#random - 0.5) * #jitter)
    } else {
        quote_expression!(0.0)
    };
    let saturation_offset = if adjustment.saturation_jitter > 0.0 {
        let random = color_random(adjustment.per_tip, 21.7);
        let jitter = adjustment.saturation_jitter;
        quote_expression!((#random * 2.0 - 1.0) * #jitter)
    } else {
        quote_expression!(0.0)
    };
    let value_offset = if adjustment.value_jitter > 0.0 {
        let random = color_random(adjustment.per_tip, 73.219);
        let jitter = adjustment.value_jitter;
        quote_expression!((#random * 2.0 - 1.0) * #jitter)
    } else {
        quote_expression!(0.0)
    };
    let purity = adjustment.purity;
    let adjusted_saturation = if purity < 0.0 {
        quote_expression!(jittered_saturation * (1.0 + #purity))
    } else {
        quote_expression!(jittered_saturation + (1.0 - jittered_saturation) * #purity)
    };

    quote_statement! {{
        let source_foreground = #base_foreground;
        let color_min = min(
            source_foreground.r,
            min(source_foreground.g, source_foreground.b),
        );
        let color_max = max(
            source_foreground.r,
            max(source_foreground.g, source_foreground.b),
        );
        let color_delta = color_max - color_min;
        var hue: f32 = 0.0;
        if color_delta > 0.0 {
            if color_max == source_foreground.r {
                hue = (source_foreground.g - source_foreground.b) / color_delta;
            } else if color_max == source_foreground.g {
                hue = 2.0 + (source_foreground.b - source_foreground.r) / color_delta;
            } else {
                hue = 4.0 + (source_foreground.r - source_foreground.g) / color_delta;
            }
            hue /= 6.0;
        }
        let saturation = select(0.0, color_delta / color_max, color_max > 0.0);
        hue = fract(hue + 1.0 + #hue_offset);
        let jittered_saturation = clamp(
            saturation + #saturation_offset,
            0.0,
            1.0,
        );
        let final_saturation = #adjusted_saturation;
        let jittered_value = clamp(
            color_max + #value_offset,
            0.0,
            1.0,
        );
        let hue_rgb = clamp(
            abs(
                fract(vec3f(hue) + vec3f(1.0, 0.6666666667, 0.3333333333))
                    * 6.0
                    - vec3f(3.0)
            ) - vec3f(1.0),
            vec3f(0.0),
            vec3f(1.0),
        );
        tip_foreground = vec4f(
            jittered_value
                * mix(vec3f(1.0), hue_rgb, final_saturation),
            source_foreground.a,
        );
    }}
}

fn texture_mask_statement(
    texture: Option<BrushTexture>,
    pose: BrushPose,
    base_diameter: f32,
    tip_angle: Expression,
) -> Statement {
    let Some(texture) = texture else {
        return quote_statement! {{}};
    };
    let pattern = (*MAIN_PATTERN_TEXTURE_IDENT).clone();
    let scale = texture.scale;
    let pattern_sample = if texture.each_tip {
        quote_statement! {{
            let pattern_texture_size = vec2i(textureDimensions(#pattern));
            let pattern_transform = render::math::scale_rotate_transform_anchored_inv(
                vec2f(
                    #scale * tip_diameter / #base_diameter,
                    #scale * tip_diameter * tip_roundness / #base_diameter,
                ),
                #tip_angle + roundness_angle,
                tip_center,
                vec2f(0.5) * vec2f(pattern_texture_size),
            );
            let pattern_pixel = vec2i(
                (pattern_transform * vec3f(pixel_position, 1.0)).xy
            );
            let wrapped_pattern_pixel = (
                (pattern_pixel % pattern_texture_size) + pattern_texture_size
            ) % pattern_texture_size;
            pattern_sample = textureLoad(#pattern, wrapped_pattern_pixel, 0);
        }}
    } else {
        quote_statement! {{
            let pattern_texture_size = vec2i(textureDimensions(#pattern));
            let pattern_transform = render::math::scale_rotate_transform_anchored_inv(
                vec2f(#scale),
                0.0,
                vec2f(0.0),
                vec2f(0.0),
            );
            let pattern_pixel = vec2i(
                (pattern_transform * vec3f(pixel_position, 1.0)).xy
            );
            let wrapped_pattern_pixel = (
                (pattern_pixel % pattern_texture_size) + pattern_texture_size
            ) % pattern_texture_size;
            pattern_sample = textureLoad(#pattern, wrapped_pattern_pixel, 0);
        }}
    };
    let base_value = quote_expression!(pattern_sample.r);
    let adjusted_value = if texture.use_legacy {
        let b = texture.brightness;
        let c = texture.contrast + 1.0;
        quote_expression!(clamp((#base_value - 0.5) * #c + #b + 0.5, 0.0, 1.0))
    } else {
        let b = texture.brightness / 2.0;
        let c = texture.contrast;
        let slant = ((c + 1.0) * FRAC_PI_4).tan();
        let brightness_adjusted = if b < 0.0 {
            quote_expression!(#base_value * (1.0 + #b))
        } else {
            quote_expression!(#base_value + (1.0 - #base_value) * #b)
        };
        quote_expression!(clamp((#brightness_adjusted - 0.5) * #slant + 0.5, 0.0, 1.0))
    };
    let texture_value = if texture.inverted {
        quote_expression!(1.0 - #adjusted_value)
    } else {
        adjusted_value
    };
    let depth = texture.depth;
    let effective_depth = match texture.depth_dynamics {
        Some(dynamics) => {
            let control = dynamics_control(dynamics, pose);
            let minimum = dynamics.minimum;
            let jitter = dynamics.jitter;
            let random = dab_random(197.731);
            quote_expression!(
                mix(#minimum, #depth, #control)
                    * (1.0 - #jitter * #random)
            )
        }
        None => quote_expression!(#depth * 1.0),
    };
    let blend = Ident::new(texture.blend_mode.shader_func().to_string());

    quote_statement! {{
        var pattern_sample: vec4f;
        @#pattern_sample {}
        let pattern_color = vec4f(vec3f(#texture_value), 1.0);
        let mask_color = vec4f(vec3f(copy_mask), 1.0);
        let textured_mask = select(0.0, image::blend_modes::#blend(pattern_color, mask_color).r, copy_mask > 0.0);
        copy_mask = mix(copy_mask, textured_mask, #effective_depth);
    }}
}

fn dual_mask_statement(dual: Option<DualBrush>, pose: BrushPose, main_diameter: f32) -> Statement {
    let Some(dual) = dual else {
        return quote_statement! {{}};
    };
    let pixel = (*MAIN_PIXEL_POSITION_IDENT).clone();
    let pen = (*MAIN_PEN_POSITION_IDENT).clone();
    let direction = (*DIRECTION_IDENT).clone();
    let stroke_distance = (*STROKE_DISTANCE_IDENT).clone();
    let diameter = match dual.tip {
        DualBrushTip::Computed { diameter, .. } | DualBrushTip::Sampled { diameter, .. } => {
            diameter
        }
    };
    let dual_spacing = match dual.tip {
        DualBrushTip::Computed { .. } => {
            let spacing = dual.spacing;
            quote_statement! {
                dual_spacing = max(dual_diameter * #spacing, 0.001);
            }
        }
        DualBrushTip::Sampled { .. } => {
            let texture = (*DUAL_TIP_TEXTURE_IDENT).clone();
            let spacing = dual.spacing;
            quote_statement! {{
                let dual_spacing_texture_size = vec2f(textureDimensions(#texture));
                let dual_spacing_scale = min(
                    dual_spacing_texture_size.x,
                    dual_spacing_texture_size.y,
                ) / max(
                    dual_spacing_texture_size.x,
                    dual_spacing_texture_size.y,
                );
                dual_spacing = max(dual_diameter * #spacing * dual_spacing_scale, 0.001);
            }}
        }
    };
    let along_stroke_scatter = dual
        .scatter
        .filter(|scatter| scatter.both_axes)
        .and_then(|scatter| scatter.scatter_dynamics)
        .map(|dynamics| dynamics.jitter * 0.5)
        .unwrap_or(0.0);
    let dual_support_factor = FRAC_1_SQRT_2 + along_stroke_scatter;
    let random_flip = if dual.flip {
        let random = dual_copy_random(281.917);
        quote_expression!(select(1.0, -1.0, #random < 0.5))
    } else {
        quote_expression!(1.0)
    };
    let sample_dual = match dual.tip {
        DualBrushTip::Computed {
            hardness,
            angle,
            roundness,
            flip_x,
            flip_y,
            ..
        } => {
            let flip_x = if flip_x { -1.0f32 } else { 1.0f32 };
            let flip_y = if flip_y { -1.0f32 } else { 1.0f32 };
            quote_statement! {{
                let dual_radius = dual_diameter * 0.5;
                let dual_edge = max(dual_radius * (1.0 - #hardness), 1.0);
                let dual_delta = (#pixel - dual_center)
                    * vec2f(#flip_x * #random_flip, #flip_y);
                let dual_distance = render::math::sdf_ellipse(
                    render::math::rotate_mat2x2(#angle) * dual_delta,
                    vec2f(0.0),
                    vec2f(dual_radius, max(dual_radius * #roundness, 0.001)),
                );
                dual_copy_mask = 1.0 - smoothstep(-dual_edge, 0.0, dual_distance);
            }}
        }
        DualBrushTip::Sampled {
            angle,
            roundness,
            flip_x,
            flip_y,
            ..
        } => {
            let texture = (*DUAL_TIP_TEXTURE_IDENT).clone();
            let flip_x = if flip_x { -1.0f32 } else { 1.0f32 };
            let flip_y = if flip_y { -1.0f32 } else { 1.0f32 };
            quote_statement! {{
                let dual_texture_size = vec2f(textureDimensions(#texture));
                let dual_base_size = max(dual_texture_size.x, dual_texture_size.y);
                let dual_scale = vec2f(
                    #flip_x * #random_flip * dual_diameter / dual_base_size,
                    #flip_y * dual_diameter * #roundness / dual_base_size,
                );
                let dual_transform = render::math::scale_rotate_transform_anchored_inv(
                    dual_scale,
                    #angle,
                    dual_center,
                    vec2f(0.5) * dual_texture_size,
                );
                let dual_texture_pixel = (
                    dual_transform * vec3f(#pixel, 1.0)
                ).xy;
                var dual_sample = vec4f(0.0);
                if all(dual_texture_pixel >= vec2f(0.0))
                    && all(dual_texture_pixel < dual_texture_size) {
                    dual_sample = textureLoad(
                        #texture,
                        vec2i(dual_texture_pixel),
                        0,
                    );
                }
                dual_copy_mask = dual_sample.a * dual_sample.r;
            }}
        }
    };
    let scatter_offset = dual_scatter_offset_expression(dual.scatter, pose);
    let active_copy_count = dual_scatter_count_expression(dual.scatter, pose);
    let blend = Ident::new(dual.blend_mode.shader_func().to_string());

    let user_size = (*USER_SIZE_IDENT).clone();
    quote_statement! {{
        let dual_diameter = #diameter
            * clamp(#user_size, 0.1, 1000.0)
            / max(#main_diameter, 0.1);
        var dual_spacing: f32;
        @#dual_spacing {}
        let dual_distance_along_stroke = #stroke_distance;
        let dual_direction = vec2f(cos(#direction), sin(#direction));
        let dual_last_dab_index = u32(floor(dual_distance_along_stroke / dual_spacing));
        let dual_pixel_distance = max(
            0.0,
            dual_distance_along_stroke + dot(#pixel - #pen, dual_direction),
        );
        let dual_nearest_dab_index = min(
            dual_last_dab_index,
            u32(round(dual_pixel_distance / dual_spacing)),
        );
        let dual_overlap_count = u32(ceil(
            dual_diameter * #dual_support_factor / dual_spacing
        )) + 1u;
        let dual_first_dab_index = dual_nearest_dab_index
            - min(dual_nearest_dab_index, dual_overlap_count);
        let dual_end_dab_index = min(
            dual_last_dab_index,
            dual_nearest_dab_index + dual_overlap_count,
        );
        let dual_active_copy_count = #active_copy_count;
        var dual_mask = 0.0;
        for (
            var dual_dab_index = dual_first_dab_index;
            dual_dab_index <= dual_end_dab_index;
            dual_dab_index += 1u
        ) {
            let dual_stroke_center = #pen + dual_direction * (
                f32(dual_dab_index) * dual_spacing - dual_distance_along_stroke
            );
            for (
                var dual_copy_index = 0u;
                dual_copy_index < dual_active_copy_count;
                dual_copy_index += 1u
            ) {
                let dual_center = dual_stroke_center + #scatter_offset;
                var dual_copy_mask = 0.0;
                @#sample_dual {}
                dual_mask = 1.0 - (1.0 - dual_mask) * (1.0 - dual_copy_mask);
            }
        }
        let dual_color = vec4f(vec3f(dual_mask), 1.0);
        let mask_color = vec4f(vec3f(tip_mask), 1.0);
        tip_mask = select(0.0, image::blend_modes::#blend(dual_color, mask_color).r, tip_mask > 0.0);
    }}
}

fn tip_color_statement(
    flow: f32,
    flow_dynamics: Option<Dynamics>,
    opacity_dynamics: Option<Dynamics>,
    pose: BrushPose,
) -> Statement {
    let color = (*MAIN_COLOR_IDENT).clone();
    let current_color = (*MAIN_CURRENT_COLOR_IDENT).clone();
    let user_flow = (*USER_FLOW_IDENT).clone();
    let effective_flow = match flow_dynamics {
        Some(dynamics) => {
            let factor = dynamics_factor(dynamics, 19.673, pose);
            quote_expression!(#flow * clamp(#user_flow, 0.0, 1.0) * #factor)
        }
        None => quote_expression!(#flow * clamp(#user_flow, 0.0, 1.0)),
    };
    let Some(dynamics) = opacity_dynamics else {
        return quote_statement! {{
            let tip_color = vec4f(
                tip_foreground.rgb,
                tip_foreground.a * tip_mask * #effective_flow,
            );
            #color = image::blend_modes::blend_normal(tip_color, #current_color);
        }};
    };
    let opacity_factor = dynamics_factor(dynamics, 39.346, pose);

    quote_statement! {{
        let tip_color = vec4f(
            tip_foreground.rgb,
            tip_foreground.a * tip_mask * #effective_flow,
        );
        let previous_color = #current_color;
        let accumulated_color = image::blend_modes::blend_normal(
            tip_color,
            previous_color,
        );
        #color = vec4f(
            accumulated_color.rgb,
            max(
                previous_color.a,
                min(
                    accumulated_color.a,
                    tip_foreground.a * #opacity_factor,
                ),
            ),
        );
    }}
}

pub fn computed_required_spacing(
    spacing: f32,
    size_dynamics: Option<Dynamics>,
    pose: BrushPose,
) -> String {
    let required_spacing = (*REQUIRED_SPACING_IDENT).clone();
    let size_diameter = size_diameter_statement(size_dynamics, pose);
    quote_statement! {{
        var tip_diameter: f32;
        @#size_diameter {}
        #required_spacing = max(tip_diameter * #spacing, 0.001);
    }}
    .to_string()
}

pub fn sampled_required_spacing(
    spacing: f32,
    size_dynamics: Option<Dynamics>,
    pose: BrushPose,
) -> String {
    let texture = (*MAIN_TIP_TEXTURE_IDENT).clone();
    let required_spacing = (*REQUIRED_SPACING_IDENT).clone();
    let size_diameter = size_diameter_statement(size_dynamics, pose);
    quote_statement! {{
        var tip_diameter: f32;
        @#size_diameter {}
        let tip_texture_size = vec2f(textureDimensions(#texture));
        let spacing_scale = min(tip_texture_size.x, tip_texture_size.y)
            / max(tip_texture_size.x, tip_texture_size.y);
        #required_spacing = max(tip_diameter * #spacing * spacing_scale, 0.001);
    }}
    .to_string()
}

pub fn computed_main(tip: ComputedMainTip, options: MainGraphOptions) -> String {
    let ComputedMainTip {
        diameter,
        hardness,
        angle,
        roundness,
        flip_x,
        flip_y,
        ..
    } = tip;
    let MainGraphOptions {
        flow,
        size_dynamics,
        opacity_dynamics,
        flow_dynamics,
        angle_dynamics,
        roundness_dynamics,
        tilt_scale,
        flip_x_jitter,
        flip_y_jitter,
        pose,
        color_adjustment,
        scatter,
        brush_texture,
        noise,
        dual_brush,
        ..
    } = options;

    let pixel = (*MAIN_PIXEL_POSITION_IDENT).clone();
    let pen = (*MAIN_PEN_POSITION_IDENT).clone();
    let bounds = (*MAIN_BOUNDS_IDENT).clone();
    let flip_x = if flip_x { -1.0f32 } else { 1.0f32 };
    let flip_y = if flip_y { -1.0f32 } else { 1.0f32 };
    let flip_jitter = flip_jitter_scale(flip_x_jitter, flip_y_jitter);
    let size_diameter = size_diameter_statement(size_dynamics, pose);
    let tip_roundness = tip_roundness_statement(roundness, roundness_dynamics, tilt_scale, pose);
    let tip_angle = tip_angle_expression(angle, angle_dynamics, pose);
    let tip_foreground = tip_foreground_statement(color_adjustment, pose);
    let tip_color = tip_color_statement(flow, flow_dynamics, opacity_dynamics, pose);
    let scatter_offset = scatter_offset_expression(scatter, pose);
    let active_copy_count = scatter_count_expression(scatter, pose);
    let texture_mask = texture_mask_statement(brush_texture, pose, diameter, tip_angle.clone());
    let noise = noise_statement(noise);
    let dual_mask = dual_mask_statement(dual_brush, pose, diameter);

    quote_statement! {{
        var tip_diameter: f32;
        var tip_roundness: f32;
        var roundness_angle: f32;
        var tip_foreground: vec4f;
        @#size_diameter {}
        @#tip_roundness {}
        @#tip_foreground {}
        let tip_radius = tip_diameter * 0.5;
        let tip_radii = vec2f(
            tip_radius,
            max(tip_radius * tip_roundness, 0.001),
        );
        let tip_edge = max(
            1.0,
            max(tip_radii.x, tip_radii.y) * (1.0 - #hardness),
        );
        let active_copy_count = #active_copy_count;
        var tip_mask = 0.0;
        var combined_bounds = render::math::Rect(
            vec2f(1000000.0),
            vec2f(-1000000.0),
        );
        for (var copy_index = 0u; copy_index < active_copy_count; copy_index++) {
            let tip_center = #pen + #scatter_offset;
            let tip_delta = (#pixel - tip_center) * vec2f(#flip_x, #flip_y) * #flip_jitter;
            let tip_distance = render::math::sdf_ellipse(
                render::math::rotate_mat2x2(#tip_angle + roundness_angle) * tip_delta,
                vec2f(0.0),
                tip_radii,
            );
            var copy_mask = 1.0 - smoothstep(-tip_edge, 0.0, tip_distance);
            @#noise {}
            @#texture_mask {}
            tip_mask = 1.0 - (1.0 - tip_mask) * (1.0 - copy_mask);
            let copy_bounds = render::math::Rect(
                tip_center - vec2f(tip_radius),
                tip_center + vec2f(tip_radius),
            );
            combined_bounds = render::math::Rect(
                min(combined_bounds.min, copy_bounds.min),
                max(combined_bounds.max, copy_bounds.max),
            );
        }
        @#dual_mask {}
        @#tip_color {}
        #bounds = combined_bounds;
    }}
    .to_string()
}

pub fn sampled_main(tip: SampledMainTip, options: MainGraphOptions) -> String {
    let SampledMainTip {
        diameter,
        angle,
        roundness,
        flip_x,
        flip_y,
        ..
    } = tip;
    let MainGraphOptions {
        flow,
        size_dynamics,
        opacity_dynamics,
        flow_dynamics,
        angle_dynamics,
        roundness_dynamics,
        tilt_scale,
        flip_x_jitter,
        flip_y_jitter,
        pose,
        color_adjustment,
        scatter,
        brush_texture,
        noise,
        dual_brush,
        ..
    } = options;

    let texture = (*MAIN_TIP_TEXTURE_IDENT).clone();
    let pixel = (*MAIN_PIXEL_POSITION_IDENT).clone();
    let pen = (*MAIN_PEN_POSITION_IDENT).clone();
    let bounds = (*MAIN_BOUNDS_IDENT).clone();
    let flip_x = if flip_x { -1.0f32 } else { 1.0f32 };
    let flip_y = if flip_y { -1.0f32 } else { 1.0f32 };
    let flip_jitter = flip_jitter_scale(flip_x_jitter, flip_y_jitter);
    let size_diameter = size_diameter_statement(size_dynamics, pose);
    let tip_roundness = tip_roundness_statement(roundness, roundness_dynamics, tilt_scale, pose);
    let tip_angle = tip_angle_expression(angle, angle_dynamics, pose);
    let tip_foreground = tip_foreground_statement(color_adjustment, pose);
    let tip_color = tip_color_statement(flow, flow_dynamics, opacity_dynamics, pose);
    let scatter_offset = scatter_offset_expression(scatter, pose);
    let active_copy_count = scatter_count_expression(scatter, pose);
    let texture_mask = texture_mask_statement(brush_texture, pose, diameter, tip_angle.clone());
    let noise = noise_statement(noise);
    let dual_mask = dual_mask_statement(dual_brush, pose, diameter);

    quote_statement! {{
        var tip_diameter: f32;
        var tip_roundness: f32;
        var roundness_angle: f32;
        var tip_foreground: vec4f;
        @#size_diameter {}
        @#tip_roundness {}
        @#tip_foreground {}
        let tip_texture_size = vec2f(textureDimensions(#texture));
        let tip_base_size = max(tip_texture_size.x, tip_texture_size.y);
        let tip_scale = vec2f(
            #flip_x * tip_diameter / tip_base_size,
            #flip_y * tip_diameter * tip_roundness / tip_base_size,
        );
        let tip_anchor = vec2f(0.5);
        let active_copy_count = #active_copy_count;
        var tip_mask = 0.0;
        var combined_bounds = render::math::Rect(
            vec2f(1000000.0),
            vec2f(-1000000.0),
        );
        for (var copy_index = 0u; copy_index < active_copy_count; copy_index += 1u) {
            let tip_center = #pen + #scatter_offset;
            let copy_scale = tip_scale * #flip_jitter;
            let tip_transform = render::math::scale_rotate_transform_anchored_inv(
                copy_scale,
                #tip_angle + roundness_angle,
                tip_center,
                tip_anchor * tip_texture_size,
            );
            let tip_texture_pixel = (
                tip_transform * vec3f(#pixel, 1.0)
            ).xy;
            var tip_sample = vec4f(0.0);
            if all(tip_texture_pixel >= vec2f(0.0))
                && all(tip_texture_pixel < tip_texture_size) {
                tip_sample = textureLoad(#texture, vec2i(tip_texture_pixel), 0);
            }
            var copy_mask = tip_sample.a * tip_sample.r;
            @#noise {}
            @#texture_mask {}
            tip_mask = 1.0 - (1.0 - tip_mask) * (1.0 - copy_mask);
            let copy_transform = render::math::scale_rotate_transform_anchored(
                copy_scale,
                #tip_angle + roundness_angle,
                tip_center,
                tip_anchor * tip_texture_size,
            );
            let copy_bounds = render::math::transform_rect(
                render::math::Rect(vec2f(0.0), tip_texture_size),
                copy_transform,
            );
            combined_bounds = render::math::Rect(
                min(combined_bounds.min, copy_bounds.min),
                max(combined_bounds.max, copy_bounds.max),
            );
        }
        @#dual_mask {}
        @#tip_color {}
        #bounds = combined_bounds;
    }}
    .to_string()
}

pub fn opacity_postprocess(opacity: f32, blend_mode: BlendMode) -> String {
    let user_opacity = (*USER_OPACITY_IDENT).clone();
    let input_color = (*POSTPROCESS_INPUT_COLOR_IDENT).clone();
    let stroke_bounds = (*POSTPROCESS_STROKE_BOUNDS_IDENT).clone();
    let target_color = (*POSTPROCESS_TARGET_COLOR_IDENT).clone();
    let color = (*MAIN_COLOR_IDENT).clone();
    let bounds = (*MAIN_BOUNDS_IDENT).clone();
    let blend = Ident::new(blend_mode.shader_func().to_string());

    quote_statement! {{
        let stroke_color = vec4f(
            #input_color.rgb,
            #input_color.a * #opacity * clamp(#user_opacity, 0.0, 1.0),
        );
        #color = image::blend_modes::#blend(stroke_color, #target_color);
        #bounds = #stroke_bounds;
    }}
    .to_string()
}
