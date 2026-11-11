use bevy_math::Rect;
use glam::Mat3;

/// Extension trait for transforming an axis-aligned [`Rect`] by a 2D affine [`Mat3`] and recomputing its bounding box.
pub trait RectTransform {
    fn transform(&mut self, mat: &Mat3);
    fn transformed(self, mat: &Mat3) -> Self;
}

impl RectTransform for Rect {
    fn transform(&mut self, mat: &Mat3) {
        let tl = mat.transform_point2(self.min);
        let tr = mat.transform_point2(self.max.with_y(self.min.y));
        let bl = mat.transform_point2(self.max.with_x(self.min.x));
        let br = mat.transform_point2(self.max);
        self.min = tl.min(tr).min(bl).min(br);
        self.max = tl.max(tr).max(bl).max(br);
    }

    fn transformed(mut self, mat: &Mat3) -> Self {
        self.transform(mat);
        self
    }
}
