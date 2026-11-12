use std::f32::consts;

/// Extension trait for computing the shortest signed angular difference in radians within `[-π, π)`.
pub trait AngleDifference {
    fn angle_difference(self, rhs: Self) -> Self;
}

impl AngleDifference for f32 {
    fn angle_difference(self, rhs: Self) -> Self {
        (self - rhs + consts::PI).rem_euclid(consts::TAU) - consts::PI
    }
}

/// Extension trait for linearly interpolating between two angles in radians along the shortest arc.
pub trait LerpAngle {
    fn lerp_angle(self, rhs: Self, t: f32) -> Self;
}

impl LerpAngle for f32 {
    fn lerp_angle(self, rhs: Self, t: f32) -> Self {
        let diff = (rhs - self) % consts::TAU;
        let shortest = (2.0 * diff) % consts::TAU - diff;
        self + shortest * t
    }
}
