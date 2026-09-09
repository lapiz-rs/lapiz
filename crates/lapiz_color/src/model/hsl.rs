use crate::model::rgb::Rgb;

/// HSL (hue, saturation, lightness) cylindrical representation of [`Rgb`].
///
/// HSL is calculated directly from the linear-light RGB components stored by
/// [`Rgb`]. No transfer function or color-profile conversion is applied.
///
/// - **h** — hue angle in degrees `[0.0, 360.0)`
/// - **s** — saturation in `[0.0, 1.0]`
/// - **l** — lightness in `[0.0, 1.0]`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsl {
    /// Hue angle in `[0.0, 360.0)`.
    pub h: f32,
    /// Saturation in `[0.0, 1.0]`.
    pub s: f32,
    /// Lightness in `[0.0, 1.0]`.
    pub l: f32,
}

impl Hsl {
    pub const fn new(h: f32, s: f32, l: f32) -> Self {
        Self { h, s, l }
    }

    pub fn from_rgb(rgb: Rgb) -> Self {
        let max = rgb.r.max(rgb.g).max(rgb.b);
        let min = rgb.r.min(rgb.g).min(rgb.b);
        let delta = max - min;
        let lightness = (max + min) * 0.5;

        if delta <= f32::EPSILON {
            return Self::new(0.0, 0.0, lightness);
        }

        let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
        let hue_sector = if max == rgb.r {
            (rgb.g - rgb.b) / delta
        } else if max == rgb.g {
            (rgb.b - rgb.r) / delta + 2.0
        } else {
            (rgb.r - rgb.g) / delta + 4.0
        };

        Self::new((hue_sector * 60.0).rem_euclid(360.0), saturation, lightness)
    }

    pub fn into_rgb(self) -> Rgb {
        let chroma = (1.0 - (2.0 * self.l - 1.0).abs()) * self.s;
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
        let m = self.l - chroma * 0.5;

        Rgb::new(r + m, g + m, b + m)
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{
        hsl::Hsl,
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
                    let roundtrip = Hsl::from_rgb(rgb).into_rgb();

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
