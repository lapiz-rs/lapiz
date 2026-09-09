use crate::model::rgb::Rgb;

/// HSV (hue, saturation, value) cylindrical representation of [`Rgb`].
///
/// HSV is calculated directly from the linear-light RGB components stored by
/// [`Rgb`]. No transfer function or color-profile conversion is applied.
///
/// - **h** — hue angle in degrees `[0.0, 360.0)`
/// - **s** — saturation in `[0.0, 1.0]`
/// - **v** — value in `[0.0, 1.0]`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    /// Hue angle in `[0.0, 360.0)`.
    pub h: f32,
    /// Saturation in `[0.0, 1.0]`.
    pub s: f32,
    /// Value in `[0.0, 1.0]`.
    pub v: f32,
}

impl Hsv {
    pub const fn new(h: f32, s: f32, v: f32) -> Self {
        Self { h, s, v }
    }

    pub fn from_rgb(rgb: Rgb) -> Self {
        let max = rgb.r.max(rgb.g).max(rgb.b);
        let min = rgb.r.min(rgb.g).min(rgb.b);
        let delta = max - min;

        if delta <= f32::EPSILON {
            return Self::new(0.0, 0.0, max);
        }

        let hue_sector = if max == rgb.r {
            (rgb.g - rgb.b) / delta
        } else if max == rgb.g {
            (rgb.b - rgb.r) / delta + 2.0
        } else {
            (rgb.r - rgb.g) / delta + 4.0
        };

        Self::new((hue_sector * 60.0).rem_euclid(360.0), delta / max, max)
    }

    pub fn into_rgb(self) -> Rgb {
        let chroma = self.v * self.s;
        let hue_sector = self.h.rem_euclid(360.0) / 60.0;
        let x = chroma * (1.0 - (hue_sector.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match hue_sector as u32 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let m = self.v - chroma;

        Rgb::new(r + m, g + m, b + m)
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        hsv::Hsv,
        rgb::Rgb,
        tests::{TEST_EPSILON, TEST_SEGMENTS},
    };

    #[test]
    fn roundtrip() {
        for r in 0..TEST_SEGMENTS {
            for g in 0..TEST_SEGMENTS {
                for b in 0..TEST_SEGMENTS {
                    let rgb = Rgb::new(
                        r as f32 / (TEST_SEGMENTS - 1) as f32,
                        g as f32 / (TEST_SEGMENTS - 1) as f32,
                        b as f32 / (TEST_SEGMENTS - 1) as f32,
                    );
                    let roundtrip = Hsv::from_rgb(rgb).into_rgb();

                    assert!(
                        (rgb.r - roundtrip.r).abs() < TEST_EPSILON
                            && (rgb.g - roundtrip.g).abs() < TEST_EPSILON
                            && (rgb.b - roundtrip.b).abs() < TEST_EPSILON,
                        "rgb={rgb:?} roundtrip={roundtrip:?}"
                    );
                }
            }
        }
    }
}
