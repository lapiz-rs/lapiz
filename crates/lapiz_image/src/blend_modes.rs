use std::sync::LazyLock;

use parse_display::Display;
use serde::{Deserialize, Serialize};

use crate::composite::{BlendFunction, BlendFunctionId};

/// Built-in compositing blend modes supported by the layer compositor.
///
/// Each variant implements [`BlendFunction`] and maps to a WGSL blending function in
/// `image::blend_modes` for GPU-accelerated layer compositing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
#[display(style = "snake_case")]
#[repr(usize)]
pub enum BlendMode {
    // Normal
    Normal,

    // Darken
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,

    // Lighten
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    Additive,
    LighterColor,

    // Contrast
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,

    // Inversion / Comparative
    Difference,
    Exclusion,
    Subtract,
    Subtractive,
    Divide,

    // Component (HSL)
    Hue,
    Saturation,
    Color,
    Luminosity,

    Dissolve,
    Behind,
    Clear,
    Height,
    LinearHeight,
}

impl BlendMode {
    pub const ALL: [BlendMode; 33] = [
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::Additive,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Subtractive,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
        BlendMode::Dissolve,
        BlendMode::Behind,
        BlendMode::Clear,
        BlendMode::Height,
        BlendMode::LinearHeight,
    ];
}

pub static ALL_IDS: LazyLock<[BlendFunctionId; 33]> = LazyLock::new(|| {
    [
        BlendFunctionId::new("blend_normal".into()),
        BlendFunctionId::new("blend_darken".into()),
        BlendFunctionId::new("blend_multiply".into()),
        BlendFunctionId::new("blend_color_burn".into()),
        BlendFunctionId::new("blend_linear_burn".into()),
        BlendFunctionId::new("blend_darker_color".into()),
        BlendFunctionId::new("blend_lighten".into()),
        BlendFunctionId::new("blend_screen".into()),
        BlendFunctionId::new("blend_color_dodge".into()),
        BlendFunctionId::new("blend_linear_dodge".into()),
        BlendFunctionId::new("blend_additive".into()),
        BlendFunctionId::new("blend_lighter_color".into()),
        BlendFunctionId::new("blend_overlay".into()),
        BlendFunctionId::new("blend_soft_light".into()),
        BlendFunctionId::new("blend_hard_light".into()),
        BlendFunctionId::new("blend_vivid_light".into()),
        BlendFunctionId::new("blend_linear_light".into()),
        BlendFunctionId::new("blend_pin_light".into()),
        BlendFunctionId::new("blend_hard_mix".into()),
        BlendFunctionId::new("blend_difference".into()),
        BlendFunctionId::new("blend_exclusion".into()),
        BlendFunctionId::new("blend_subtract".into()),
        BlendFunctionId::new("blend_subtractive".into()),
        BlendFunctionId::new("blend_divide".into()),
        BlendFunctionId::new("blend_hue".into()),
        BlendFunctionId::new("blend_saturation".into()),
        BlendFunctionId::new("blend_color".into()),
        BlendFunctionId::new("blend_luminosity".into()),
        BlendFunctionId::new("blend_dissolve".into()),
        BlendFunctionId::new("blend_behind".into()),
        BlendFunctionId::new("blend_clear".into()),
        BlendFunctionId::new("blend_height".into()),
        BlendFunctionId::new("blend_linear_height".into()),
    ]
});

impl BlendMode {
    pub fn shader_func(&self) -> &'static str {
        match self {
            BlendMode::Normal => "blend_normal",
            BlendMode::Darken => "blend_darken",
            BlendMode::Multiply => "blend_multiply",
            BlendMode::ColorBurn => "blend_color_burn",
            BlendMode::LinearBurn => "blend_linear_burn",
            BlendMode::DarkerColor => "blend_darker_color",
            BlendMode::Lighten => "blend_lighten",
            BlendMode::Screen => "blend_screen",
            BlendMode::ColorDodge => "blend_color_dodge",
            BlendMode::LinearDodge => "blend_linear_dodge",
            BlendMode::Additive => "blend_additive",
            BlendMode::LighterColor => "blend_lighter_color",
            BlendMode::Overlay => "blend_overlay",
            BlendMode::SoftLight => "blend_soft_light",
            BlendMode::HardLight => "blend_hard_light",
            BlendMode::VividLight => "blend_vivid_light",
            BlendMode::LinearLight => "blend_linear_light",
            BlendMode::PinLight => "blend_pin_light",
            BlendMode::HardMix => "blend_hard_mix",
            BlendMode::Difference => "blend_difference",
            BlendMode::Exclusion => "blend_exclusion",
            BlendMode::Subtract => "blend_subtract",
            BlendMode::Subtractive => "blend_subtractive",
            BlendMode::Divide => "blend_divide",
            BlendMode::Hue => "blend_hue",
            BlendMode::Saturation => "blend_saturation",
            BlendMode::Color => "blend_color",
            BlendMode::Luminosity => "blend_luminosity",
            BlendMode::Dissolve => "blend_dissolve",
            BlendMode::Behind => "blend_behind",
            BlendMode::Clear => "blend_clear",
            BlendMode::Height => "blend_height",
            BlendMode::LinearHeight => "blend_linear_height",
        }
    }
}

impl BlendFunction for BlendMode {
    fn id(&self) -> BlendFunctionId {
        ALL_IDS.get(*self as usize).unwrap().clone()
    }

    /// Notice, to make the function call valid, `lapiz_image` must be added as a dependency.
    fn wgsl_function_call(&self, src_ident: &str, dst_ident: &str) -> String {
        format!(
            "return image::blend_modes::{}({}, {});",
            self.shader_func(),
            src_ident,
            dst_ident
        )
    }
}
