use serde::{Deserialize, Serialize};

use crate::model::{
    okcolor::{ST, find_cusp, oklab_to_linear_srgb, toe, toe_inv},
    oklab::OkLab,
    xyz::Xyz,
};

/// Okhsv — a perceptually uniform HSV-like color space based on OKLab.
///
/// Designed by Björn Ottosson as a drop-in replacement for HSV that
/// preserves perceptual uniformity. Unlike traditional HSV, Okhsv
/// value is perceptually even, and the saturation scale accounts for
/// the display gamut so that `s = 1.0` corresponds to the sRGB gamut
/// boundary at the given hue and value.
///
/// - **h** — hue angle in degrees `[0°, 360°)`
/// - **s** — saturation, `[0.0, 1.0]` (1.0 = gamut boundary)
/// - **v** — value (perceptual brightness), `[0.0, 1.0]` (via the toe curve)
///
/// Like OKLab, the white point is D65 internally; chromatic adaptation
/// from/to PCS D50 is handled transparently via [`OkLab`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OkHsv {
    /// Hue angle in `[0.0, 360.0)`.
    pub h: f32,
    /// Saturation in `[0.0, 1.0]`.
    pub s: f32,
    /// Value in `[0.0, 1.0]`.
    pub v: f32,
}

impl OkHsv {
    pub const fn new(h: f32, s: f32, v: f32) -> Self {
        Self { h, s, v }
    }

    pub fn from_xyz(xyz: Xyz) -> Self {
        Self::from_oklab(OkLab::from_xyz(xyz))
    }

    pub fn into_xyz(self) -> Xyz {
        self.into_oklab().into_xyz()
    }

    pub fn from_oklab(oklab: OkLab) -> Self {
        if oklab.l == 0.0 {
            return Self::new(0.0, 0.0, 0.0);
        }

        let chroma = oklab.a.hypot(oklab.b);
        if chroma <= f32::EPSILON {
            return Self::new(0.0, 0.0, toe(oklab.l));
        }

        let a_ = oklab.a / chroma;
        let b_ = oklab.b / chroma;
        let hue = oklab.b.atan2(oklab.a).to_degrees().rem_euclid(360.0);
        let cusp = find_cusp(a_, b_);
        let st_max = ST::from_lc(cusp);
        let s_0 = 0.5;
        let k = 1.0 - s_0 / st_max.s;

        let t = st_max.t / (chroma + oklab.l * st_max.t);
        let l_v = t * oklab.l;
        let c_v = t * chroma;
        let l_vt = toe_inv(l_v);
        let c_vt = c_v * l_vt / l_v;
        let rgb_scale = oklab_to_linear_srgb(OkLab::new(l_vt, a_ * c_vt, b_ * c_vt));
        let lightness_scale_factor =
            (1.0 / rgb_scale.0.max(rgb_scale.1).max(rgb_scale.2).max(0.0)).cbrt();
        let l_r = toe(oklab.l / lightness_scale_factor);
        let v = l_r / l_v;
        let s = (s_0 + st_max.t) * c_v / (st_max.t * s_0 + st_max.t * k * c_v);

        Self::new(hue, s, v)
    }

    pub fn into_oklab(self) -> OkLab {
        if self.v == 0.0 {
            return OkLab::new(0.0, 0.0, 0.0);
        }

        if self.s == 0.0 {
            return OkLab::new(toe_inv(self.v), 0.0, 0.0);
        }

        let h = self.h.to_radians();
        let a_ = h.cos();
        let b_ = h.sin();
        let cusp = ST::from_lc(find_cusp(a_, b_));
        let s_0 = 0.5;
        let k = 1.0 - s_0 / cusp.s;

        let l_v = 1.0 - self.s * s_0 / (s_0 + cusp.t - cusp.t * k * self.s);
        let c_v = self.s * cusp.t * s_0 / (s_0 + cusp.t - cusp.t * k * self.s);
        let l_vt = toe_inv(l_v);
        let c_vt = c_v * l_vt / l_v;
        let mut lightness = self.v * l_v;
        let mut chroma = self.v * c_v;
        let lightness_new = toe_inv(lightness);
        chroma = chroma * lightness_new / lightness;
        let rgb_scale = oklab_to_linear_srgb(OkLab::new(l_vt, a_ * c_vt, b_ * c_vt));
        let lightness_scale_factor =
            (1.0 / rgb_scale.0.max(rgb_scale.1).max(rgb_scale.2).max(0.0)).cbrt();

        lightness = lightness_new * lightness_scale_factor;
        chroma *= lightness_scale_factor;

        OkLab::new(lightness, chroma * a_, chroma * b_)
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{okhsv::OkHsv, tests::roundtrip_test};

    #[test]
    fn roundtrip() {
        roundtrip_test(
            |h, s, v| OkHsv::new(h * 360.0, s, v),
            OkHsv::into_xyz,
            OkHsv::from_xyz,
        );
    }
}
