use anyhow::{Result, anyhow};
use moxcms::{ColorProfile, Matrix3f, srgb_colorants};
use serde::{Deserialize, Serialize};

use crate::model::xyz::Xyz;

/// RGB components with no stored color space or transfer function.
///
/// The generic XYZ conversions operate on linear RGB and require a matrix for
/// the working space. The `srgb_*` XYZ conversions use sRGB-encoded components.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgb {
    /// Red component.
    pub r: f32,
    /// Green component.
    pub g: f32,
    /// Blue component.
    pub b: f32,
}

impl Rgb {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    pub fn from_xyz(xyz: Xyz, xyz_to_rgb: Matrix3f) -> Self {
        let rgb = moxcms::Xyz::new(xyz.x, xyz.y, xyz.z).to_linear_rgb(xyz_to_rgb);
        Self::new(rgb.r, rgb.g, rgb.b)
    }

    /// Converts D50 XYZ to sRGB-encoded components without clipping.
    pub fn srgb_from_xyz(xyz: Xyz) -> Self {
        Self::from_xyz(xyz, srgb_colorants().inverse().to_f32()).delinearize_to_srgb()
    }

    pub fn into_xyz(&self, rgb_to_xyz: Matrix3f) -> Xyz {
        let xyz =
            moxcms::Xyz::from_linear_rgb(moxcms::Rgb::new(self.r, self.g, self.b), rgb_to_xyz);
        Xyz::new(xyz.x, xyz.y, xyz.z)
    }

    pub fn srgb_into_xyz(&self) -> Xyz {
        self.linearize_srgb().into_xyz(srgb_colorants().to_f32())
    }

    pub fn linearize_srgb(self) -> Self {
        fn decode(value: f32) -> f32 {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }

        Self::new(decode(self.r), decode(self.g), decode(self.b))
    }

    pub fn delinearize_to_srgb(self) -> Self {
        fn encode(value: f32) -> f32 {
            if value <= 0.0031308 {
                12.92 * value
            } else {
                1.055 * value.powf(1.0 / 2.4) - 0.055
            }
        }

        Self::new(encode(self.r), encode(self.g), encode(self.b))
    }

    pub fn from_color_with_profile(
        color: moxcms::Rgb<f32>,
        profile: &ColorProfile,
    ) -> Result<Self> {
        let r_trc = profile
            .red_trc
            .as_ref()
            .ok_or_else(|| anyhow!("No red trc found"))?;
        let g_trc = profile
            .green_trc
            .as_ref()
            .ok_or_else(|| anyhow!("No green trc found"))?;
        let b_trc = profile
            .blue_trc
            .as_ref()
            .ok_or_else(|| anyhow!("No blue trc found"))?;

        let r_lin = r_trc.make_linear_evaluator()?;
        let g_lin = g_trc.make_linear_evaluator()?;
        let b_lin = b_trc.make_linear_evaluator()?;

        let r = r_lin.evaluate_value(color.r);
        let g = g_lin.evaluate_value(color.g);
        let b = b_lin.evaluate_value(color.b);

        Ok(Self::new(r, g, b))
    }
}

#[cfg(test)]
mod tests {
    use moxcms::ColorProfile;

    use crate::model::{rgb::Rgb, tests::roundtrip_test};

    #[test]
    fn roundtrip() {
        let profile = ColorProfile::new_srgb();
        let rgb_to_xyz = profile.rgb_to_xyz_matrix().to_f32();
        let xyz_to_rgb = rgb_to_xyz.inverse();

        roundtrip_test(
            Rgb::new,
            |rgb| rgb.into_xyz(rgb_to_xyz),
            |xyz| Rgb::from_xyz(xyz, xyz_to_rgb),
        );
    }
}
