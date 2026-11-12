use glam::{Mat3, Vec2};
use moxcms::Matrix3f;

/// Extension trait for constructing a 2D affine [`Mat3`] transformation around a custom anchor point.
pub trait Mat3ScaleRotattionTranslationWithAnchor {
    fn from_scale_angle_translation_with_anchor(
        scale: Vec2,
        angle: f32,
        translation: Vec2,
        anchor: Vec2,
    ) -> Self;
}

impl Mat3ScaleRotattionTranslationWithAnchor for Mat3 {
    fn from_scale_angle_translation_with_anchor(
        scale: Vec2,
        angle: f32,
        translation: Vec2,
        anchor: Vec2,
    ) -> Self {
        let (s, c) = angle.sin_cos();

        let m00 = scale.x * c;
        let m10 = scale.x * s;
        let m01 = -scale.y * s;
        let m11 = scale.y * c;

        let m02 = translation.x - (anchor.x * m00 + anchor.y * m01);
        let m12 = translation.y - (anchor.x * m10 + anchor.y * m11);

        Self::from_cols_array(&[m00, m10, 0.0, m01, m11, 0.0, m02, m12, 1.0])
    }
}

/// Extension trait for converting a row-major [`moxcms::Matrix3f`] into a column-major [`Mat3`].
pub trait FromMoxcmsMatrix3f {
    fn from_moxcms(mat: Matrix3f) -> Self;
}

impl FromMoxcmsMatrix3f for Mat3 {
    fn from_moxcms(mat: Matrix3f) -> Self {
        // Convert row major matrix to column major
        Self::from_cols_array_2d(&[
            [mat.v[0][0], mat.v[1][0], mat.v[2][0]],
            [mat.v[0][1], mat.v[1][1], mat.v[2][1]],
            [mat.v[0][2], mat.v[1][2], mat.v[2][2]],
        ])
    }
}
