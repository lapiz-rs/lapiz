use serde::{Deserialize, Serialize};

use crate::model::{lab::Lab, xyz::Xyz};

/// CIE L\*C\*h (CIELCH) — cylindrical form of [`Lab`] under the D50 illuminant.
///
/// Uses the same lightness L\* as CIELAB, but replaces the Cartesian a\*/b\*
/// axes with polar coordinates:
///
/// - **L\*** — lightness (0 = black, 100 = diffuse white)
/// - **C\*** — chroma (0 = achromatic, larger = more saturated)
/// - **h**   — hue angle in degrees `[0°, 360°)`
///
/// This is a pure coordinate transform of [`Lab`]; no additional parameters
/// are introduced. `C = √(a² + b²)`, `h = atan2(b, a)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lch {
    /// CIE L* lightness in `[0.0, 100.0]`.
    pub l: f32,
    /// CIE chroma, in `[0.0, +inf)`.
    pub c: f32,
    /// Hue angle in `[0.0, 360.0)`.
    pub h: f32,
}

impl Lch {
    pub const fn new(l: f32, c: f32, h: f32) -> Self {
        Self { l, c, h }
    }

    pub fn from_xyz(xyz: Xyz) -> Self {
        Self::from_lab(Lab::from_xyz(xyz))
    }

    pub fn into_xyz(self) -> Xyz {
        self.into_lab().into_xyz()
    }

    pub fn from_lab(lab: Lab) -> Self {
        let c = lab.a.hypot(lab.b);
        let h = lab.b.atan2(lab.a).to_degrees().rem_euclid(360.0);
        Self::new(lab.l, c, h)
    }

    pub fn into_lab(self) -> Lab {
        let h = self.h.to_radians();
        Lab::new(self.l, self.c * h.cos(), self.c * h.sin())
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{lch::Lch, tests::roundtrip_test};

    #[test]
    fn roundtrip() {
        roundtrip_test(
            |l, c, h| Lch::new(l * 100.0, c * 10.0, h * 360.0),
            Lch::into_xyz,
            Lch::from_xyz,
        );
    }
}
