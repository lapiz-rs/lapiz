use serde::{Deserialize, Serialize};

use crate::model::{
    okcolor::{ChromaValues, toe, toe_inv},
    oklab::OkLab,
    xyz::Xyz,
};

/// Okhsl — a perceptually uniform HSL-like color space based on OKLab.
///
/// Designed by Björn Ottosson as a drop-in replacement for HSL that
/// preserves perceptual uniformity. Unlike traditional HSL, Okhsl
/// lightness is perceptually even, and the saturation scale accounts
/// for the display gamut so that `s = 1.0` corresponds to the boundary
/// of the sRGB gamut at the given hue and lightness.
///
/// - **h** — hue angle in degrees `[0°, 360°)`
/// - **s** — saturation, `[0.0, 1.0]` (1.0 = gamut boundary)
/// - **l** — perceptual lightness, `[0.0, 1.0]` (via the toe curve)
///
/// Like OKLab, the white point is D65 internally; chromatic adaptation
/// from/to PCS D50 is handled transparently via [`OkLab`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OkHsl {
    /// Hue angle in `[0.0, 360.0)`.
    pub h: f32,
    /// Saturation in `[0.0, 1.0]`.
    pub s: f32,
    /// Perceptual lightness in `[0.0, 1.0]`.
    pub l: f32,
}

impl OkHsl {
    pub const fn new(h: f32, s: f32, l: f32) -> Self {
        Self { h, s, l }
    }

    pub fn from_xyz(xyz: Xyz) -> Self {
        Self::from_oklab(OkLab::from_xyz(xyz))
    }

    pub fn into_xyz(self) -> Xyz {
        self.into_oklab().into_xyz()
    }

    pub fn from_oklab(oklab: OkLab) -> Self {
        let lightness = toe(oklab.l);
        let chroma = oklab.a.hypot(oklab.b);

        if chroma <= f32::EPSILON || oklab.l == 0.0 || oklab.l == 1.0 {
            return Self::new(0.0, 0.0, lightness);
        }

        let hue = oklab.b.atan2(oklab.a).to_degrees().rem_euclid(360.0);
        let chroma_values =
            ChromaValues::from_normalized(oklab.l, oklab.a / chroma, oklab.b / chroma);
        let mid = 0.8;
        let mid_inv = 1.25;

        let saturation = if chroma < chroma_values.mid {
            let k_1 = mid * chroma_values.zero;
            let k_2 = 1.0 - k_1 / chroma_values.mid;
            let t = chroma / (k_1 + k_2 * chroma);
            t * mid
        } else {
            let k_0 = chroma_values.mid;
            let k_1 = (1.0 - mid) * (chroma_values.mid * mid_inv).powi(2) / chroma_values.zero;
            let k_2 = 1.0 - k_1 / (chroma_values.max - chroma_values.mid);
            let t = (chroma - k_0) / (k_1 + k_2 * (chroma - k_0));
            mid + (1.0 - mid) * t
        };

        Self::new(hue, saturation, lightness)
    }

    pub fn into_oklab(self) -> OkLab {
        if self.l == 1.0 {
            return OkLab::new(1.0, 0.0, 0.0);
        }

        if self.l == 0.0 {
            return OkLab::new(0.0, 0.0, 0.0);
        }

        let h = self.h.to_radians();
        let a_ = h.cos();
        let b_ = h.sin();
        let lightness = toe_inv(self.l);
        let chroma_values = ChromaValues::from_normalized(lightness, a_, b_);
        let mid = 0.8;
        let mid_inv = 1.25;

        let chroma = if self.s < mid {
            let t = mid_inv * self.s;
            let k_1 = mid * chroma_values.zero;
            let k_2 = 1.0 - k_1 / chroma_values.mid;
            t * k_1 / (1.0 - k_2 * t)
        } else {
            let t = (self.s - mid) / (1.0 - mid);
            let k_0 = chroma_values.mid;
            let k_1 = (1.0 - mid) * chroma_values.mid * chroma_values.mid * mid_inv * mid_inv
                / chroma_values.zero;
            let k_2 = 1.0 - k_1 / (chroma_values.max - chroma_values.mid);
            k_0 + t * k_1 / (1.0 - k_2 * t)
        };

        OkLab::new(lightness, chroma * a_, chroma * b_)
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{okhsl::OkHsl, tests::roundtrip_test};

    #[test]
    fn roundtrip() {
        roundtrip_test(
            |h, s, l| OkHsl::new(h * 360.0, s, l),
            OkHsl::into_xyz,
            OkHsl::from_xyz,
        );
    }
}
