use glam::{Mat3, Vec3};
use serde::{Deserialize, Serialize};

/// CIE 1931 XYZ tristimulus values under the D50 illuminant (ICC PCS).
///
/// Uses the CIE 1931 2° Standard Observer color matching functions.
/// Values are relative — the D50 white point is normalized to `Y = 1.0`.
///
/// Note that X and Z may exceed `1.0` for highly saturated colors;
/// only Y is guaranteed to be within `[0.0, 1.0]` for colors within
/// the D50 gamut.
///
/// Chromatic adaptation between illuminants is provided via [`Xyz::cat`],
/// which applies a Bradford transform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Xyz {
    /// CIE X tristimulus value (relative, D50). May exceed `1.0`.
    pub x: f32,
    /// CIE Y tristimulus value — relative luminance, `[0.0, 1.0]` under D50.
    pub y: f32,
    /// CIE Z tristimulus value (relative, D50). May exceed `1.0`.
    pub z: f32,
}

impl Xyz {
    pub const D50_WHITE: Self = Self::new(0.964_22, 1.0, 0.825_21);
    pub const D65_WHITE: Self = Self::new(0.950_47, 1.0, 1.088_83);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn cat(&self, src_white: Xyz, dst_white: Xyz) -> Self {
        let adapted = Self::cat_matrix(src_white, dst_white) * Vec3::new(self.x, self.y, self.z);
        Self::new(adapted.x, adapted.y, adapted.z)
    }

    /// Returns the 3×3 Bradford chromatic adaptation matrix from `src_white` to `dst_white`.
    pub fn cat_matrix(src_white: Xyz, dst_white: Xyz) -> Mat3 {
        let bradford = Mat3::from_cols_array(&[
            0.895_1, -0.750_2, 0.038_9, 0.266_4, 1.713_5, -0.068_5, -0.161_4, 0.036_7, 1.029_6,
        ]);
        let inv_bradford = Mat3::from_cols_array(&[
            0.986_992_9,
            0.432_305_3,
            -0.008_528_7,
            -0.147_054_3,
            0.518_360_3,
            0.040_042_8,
            0.159_962_7,
            0.049_291_2,
            0.968_486_7,
        ]);

        let src_lms = bradford * Vec3::new(src_white.x, src_white.y, src_white.z);
        let dst_lms = bradford * Vec3::new(dst_white.x, dst_white.y, dst_white.z);
        let scale = Mat3::from_cols_array(&[
            dst_lms.x / src_lms.x,
            0.0,
            0.0,
            0.0,
            dst_lms.y / src_lms.y,
            0.0,
            0.0,
            0.0,
            dst_lms.z / src_lms.z,
        ]);

        inv_bradford * scale * bradford
    }
}
