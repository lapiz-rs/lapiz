use serde::{Deserialize, Serialize};

use crate::model::xyz::Xyz;

/// CIE 1976 L\*a\*b\* (CIELAB) perceptual color space under the D50 illuminant.
///
/// A perceptually uniform opponent-color space defined by the CIE.
/// Three approximately orthogonal axes:
///
/// - **L\*** — lightness (0 = black, 100 = diffuse white)
/// - **a\*** — green–red opponent axis (negative = green, positive = red)
/// - **b\*** — blue–yellow opponent axis (negative = blue, positive = yellow)
///
/// The conversion from XYZ normalizes by a reference white before applying
/// the nonlinear cube-root compression. By default this is D50 ([`Xyz::D50_WHITE`]);
/// use [`Lab::from_xyz_with`] / [`Lab::into_xyz_with`] for other illuminants.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lab {
    /// CIE L* lightness in `[0.0, 100.0]`.
    pub l: f32,
    /// CIE a* green-red axis in `[-128.0, 127.0]`.
    pub a: f32,
    /// CIE b* blue-yellow axis in `[-128.0, 127.0]`.
    pub b: f32,
}

impl Lab {
    pub const fn new(l: f32, a: f32, b: f32) -> Self {
        Self { l, a, b }
    }

    pub fn from_xyz(xyz: Xyz) -> Self {
        Self::from_xyz_with(xyz, Xyz::D50_WHITE)
    }

    pub fn into_xyz(self) -> Xyz {
        self.into_xyz_with(Xyz::D50_WHITE)
    }

    pub fn from_xyz_with(xyz: Xyz, white: Xyz) -> Self {
        let x = lab_f(xyz.x / white.x);
        let y = lab_f(xyz.y / white.y);
        let z = lab_f(xyz.z / white.z);

        Self::new(116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z))
    }

    pub fn into_xyz_with(self, white: Xyz) -> Xyz {
        let y = (self.l + 16.0) / 116.0;
        let x = y + self.a / 500.0;
        let z = y - self.b / 200.0;

        Xyz::new(
            white.x * lab_f_inv(x),
            white.y * lab_f_inv(y),
            white.z * lab_f_inv(z),
        )
    }
}

const EPSILON: f32 = 216.0 / 24_389.0;
const KAPPA: f32 = 24_389.0 / 27.0;

fn lab_f(t: f32) -> f32 {
    if t > EPSILON {
        t.cbrt()
    } else {
        (KAPPA * t + 16.0) / 116.0
    }
}

fn lab_f_inv(t: f32) -> f32 {
    let t3 = t * t * t;
    if t3 > EPSILON {
        t3
    } else {
        (116.0 * t - 16.0) / KAPPA
    }
}

#[cfg(test)]
mod tests {
    use glam::FloatExt;

    use crate::model::{lab::Lab, tests::roundtrip_test};

    #[test]
    fn roundtrip() {
        roundtrip_test(
            |l, a, b| Lab::new(l * 100.0, -30.0.lerp(30.0, a), -30.0.lerp(30.0, b)),
            Lab::into_xyz,
            Lab::from_xyz,
        );
    }
}
