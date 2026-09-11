use serde::{Deserialize, Serialize};

use crate::model::{oklab::OkLab, xyz::Xyz};

/// OKLCh — cylindrical form of [`OkLab`].
///
/// This is a pure coordinate transform of OKLab; no additional parameters
/// are introduced. The hue and chroma correspond to the OKLab a/b axes
/// converted to polar form.
///
/// - **L** — lightness, `[0.0, 1.0]`, identical to OKLab L
/// - **C** — chroma, `[0.0, +∞)`, saturation distance from the neutral axis
/// - **h** — hue angle in degrees `[0°, 360°)`
///
/// Like OKLab, the white point is D65 internally; chromatic adaptation
/// from/to PCS D50 is handled transparently by the `from_xyz` / `into_xyz`
/// methods.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OkLch {
    /// Oklab lightness in `[0.0, 1.0]`.
    pub l: f32,
    /// Oklab chroma, in `[0.0, +inf)`.
    pub c: f32,
    /// Hue angle in `[0.0, 360.0)`.
    pub h: f32,
}

impl OkLch {
    pub const fn new(l: f32, c: f32, h: f32) -> Self {
        Self { l, c, h }
    }

    pub fn from_xyz(xyz: Xyz) -> Self {
        Self::from_oklab(OkLab::from_xyz(xyz))
    }

    pub fn into_xyz(self) -> Xyz {
        self.into_oklab().into_xyz()
    }

    pub fn from_oklab(oklab: OkLab) -> Self {
        let c = oklab.a.hypot(oklab.b);
        let h = oklab.b.atan2(oklab.a).to_degrees().rem_euclid(360.0);
        Self::new(oklab.l, c, h)
    }

    pub fn into_oklab(self) -> OkLab {
        let h = self.h.to_radians();
        OkLab::new(self.l, self.c * h.cos(), self.c * h.sin())
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{oklch::OkLch, tests::roundtrip_test};

    #[test]
    fn roundtrip() {
        roundtrip_test(
            |l, c, h| OkLch::new(l, c, h * 360.0),
            OkLch::into_xyz,
            OkLch::from_xyz,
        );
    }
}
