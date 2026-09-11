use anyhow::{Result, anyhow};
use moxcms::{ColorProfile, Matrix3f};
use serde::{Deserialize, Serialize};

use crate::model::xyz::Xyz;

/// Linear (scene-referred) RGB color in an arbitrary RGB working space.
///
/// `Rgb` stores **linear** light values — no gamma / transfer function is
/// applied. The color space (primaries, white point, tone curve) is NOT
/// stored on the struct; it is supplied at conversion time via the
/// `rgb_to_xyz` matrix parameter.
///
/// To convert to/from [`Xyz`], you must provide a 3×3 linear transformation
/// matrix specific to the RGB working space (e.g. sRGB → XYZ, Display P3 → XYZ).
///
/// - **r** — linear red, `[0.0, 1.0]` for display-referenced, unbounded for scene
/// - **g** — linear green, same range
/// - **b** — linear blue, same range
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgb {
    /// Linear red component in `[0.0, 1.0]`.
    pub r: f32,
    /// Linear green component in `[0.0, 1.0]`.
    pub g: f32,
    /// Linear blue component in `[0.0, 1.0]`.
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

    pub fn into_xyz(&self, rgb_to_xyz: Matrix3f) -> Xyz {
        let xyz =
            moxcms::Xyz::from_linear_rgb(moxcms::Rgb::new(self.r, self.g, self.b), rgb_to_xyz);
        Xyz::new(xyz.x, xyz.y, xyz.z)
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
