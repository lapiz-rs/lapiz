use iced_core::Color;

pub trait ColorMix {
    fn mix(&self, other: &Self, factor: f32) -> Self;
    fn mix_linear(&self, other: &Self, factor: f32) -> Self;
}

impl ColorMix for Color {
    fn mix(&self, other: &Self, factor: f32) -> Self {
        Self {
            r: self.r * factor + other.r * (1.0 - factor),
            g: self.g * factor + other.g * (1.0 - factor),
            b: self.b * factor + other.b * (1.0 - factor),
            a: self.a * factor + other.a * (1.0 - factor),
        }
    }

    fn mix_linear(&self, other: &Self, factor: f32) -> Self {
        let [r, g, b, a] = self.into_linear();
        let [r2, g2, b2, a2] = other.into_linear();
        Self::from_linear_rgba(
            r * factor + r2 * (1.0 - factor),
            g * factor + g2 * (1.0 - factor),
            b * factor + b2 * (1.0 - factor),
            a * factor + a2 * (1.0 - factor),
        )
    }
}
