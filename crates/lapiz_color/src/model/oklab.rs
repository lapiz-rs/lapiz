#![allow(clippy::excessive_precision)]
use serde::{Deserialize, Serialize};

use crate::model::xyz::Xyz;

/// OKLab perceptual color space (Björn Ottosson, 2020).
///
/// A perceptually uniform opponent-color space that improves upon CIELAB
/// by applying the nonlinear cube-root compression in LMS cone-response
/// space rather than in XYZ, which largely eliminates the blue–purple hue
/// shift present in CIELAB.
///
/// OKLab is defined relative to the **D65** illuminant. The public
/// [`OkLab::from_xyz`] / [`OkLab::into_xyz`] methods handle chromatic
/// adaptation from/to the PCS D50 white point automatically. Use
/// [`OkLab::from_xyz_d65`] / [`OkLab::into_xyz_d65`] if your XYZ values
/// are already D65-relative.
///
/// - **L** — lightness, `[0.0, 1.0]`
/// - **a** — green–red opponent axis (negative ≈ green, positive ≈ red)
/// - **b** — blue–yellow opponent axis (negative ≈ blue, positive ≈ yellow)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OkLab {
    /// Oklab lightness in `[0.0, 1.0]`.
    pub l: f32,
    /// Oklab green-red axis in `[-0.5, 0.5]`.
    pub a: f32,
    /// Oklab blue-yellow axis in `[-0.5, 0.5]`.
    pub b: f32,
}

impl OkLab {
    pub const fn new(l: f32, a: f32, b: f32) -> Self {
        Self { l, a, b }
    }

    pub fn from_xyz(xyz: Xyz) -> Self {
        Self::from_xyz_d65(xyz.cat(Xyz::D50_WHITE, Xyz::D65_WHITE))
    }

    pub fn into_xyz(self) -> Xyz {
        self.into_xyz_d65().cat(Xyz::D65_WHITE, Xyz::D50_WHITE)
    }

    pub fn from_xyz_d65(xyz: Xyz) -> Self {
        let l = 0.818_933_010_1 * xyz.x + 0.361_866_742_4 * xyz.y - 0.128_859_713_7 * xyz.z;
        let m = 0.032_984_543_6 * xyz.x + 0.929_311_871_5 * xyz.y + 0.036_145_638_7 * xyz.z;
        let s = 0.048_200_301_8 * xyz.x + 0.264_366_269_1 * xyz.y + 0.633_851_707 * xyz.z;

        let l_ = l.cbrt();
        let m_ = m.cbrt();
        let s_ = s.cbrt();

        Self::new(
            0.210_454_255_3 * l_ + 0.793_617_785 * m_ - 0.004_072_046_8 * s_,
            1.977_998_495_1 * l_ - 2.428_592_205 * m_ + 0.450_593_709_9 * s_,
            0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766 * s_,
        )
    }

    pub fn into_xyz_d65(self) -> Xyz {
        let l_ = self.l + 0.396_337_777_4 * self.a + 0.215_803_757_3 * self.b;
        let m_ = self.l - 0.105_561_345_8 * self.a - 0.063_854_172_8 * self.b;
        let s_ = self.l - 0.089_484_177_5 * self.a - 1.291_485_548 * self.b;

        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        Xyz::new(
            1.227_013_851_1 * l - 0.557_799_980_7 * m + 0.281_256_149 * s,
            -0.040_580_178_4 * l + 1.112_256_869_6 * m - 0.071_676_678_7 * s,
            -0.076_381_284_5 * l - 0.421_481_978_4 * m + 1.586_163_220_4 * s,
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{oklab::OkLab, tests::roundtrip_test};

    #[test]
    fn roundtrip() {
        roundtrip_test(
            |l, a, b| OkLab::new(l, a - 0.5, b - 0.5),
            OkLab::into_xyz,
            OkLab::from_xyz,
        );
    }
}
