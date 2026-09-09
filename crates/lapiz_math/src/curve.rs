use glam::Vec2;

#[derive(Debug, Clone)]
pub struct CubicCurve {
    control_points: Vec<Vec2>,
    derivatives: Vec<f32>,
}

impl CubicCurve {
    pub fn new(control_points: Vec<Vec2>) -> Self {
        let derivatives = Self::calculate_derivatives(&control_points);
        Self {
            control_points,
            derivatives,
        }
    }

    pub fn calculate_derivatives(points: &[Vec2]) -> Vec<f32> {
        let n = points.len() - 1;

        let mut a = vec![0.0_f32; n + 1];
        let mut b = vec![0.0_f32; n + 1];
        let mut c = vec![0.0_f32; n + 1];
        let mut d = vec![0.0_f32; n + 1];

        {
            let dx = points[1].x - points[0].x;
            b[0] = 2.0 / dx;
            c[0] = 1.0 / dx;
            d[0] = 3.0 * (points[1].y - points[0].y) / (dx * dx);
        }

        for i in 1..n {
            let dx_prev = points[i].x - points[i - 1].x;
            let dx_next = points[i + 1].x - points[i].x;

            a[i] = 1.0 / dx_prev;
            b[i] = 2.0 * (1.0 / dx_prev + 1.0 / dx_next);
            c[i] = 1.0 / dx_next;
            d[i] = 3.0
                * ((points[i].y - points[i - 1].y) / (dx_prev * dx_prev)
                    + (points[i + 1].y - points[i].y) / (dx_next * dx_next));
        }

        {
            let dx = points[n].x - points[n - 1].x;
            a[n] = 1.0 / dx;
            b[n] = 2.0 / dx;
            d[n] = 3.0 * (points[n].y - points[n - 1].y) / (dx * dx);
        }

        for i in 1..=n {
            let w = a[i] / b[i - 1];
            b[i] -= w * c[i - 1];
            d[i] -= w * d[i - 1];
        }

        d[n] /= b[n];
        for i in (0..n).rev() {
            d[i] = (d[i] - c[i] * d[i + 1]) / b[i];
        }

        d
    }

    pub fn subdivide(&self, n: usize) -> Vec<Vec2> {
        (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                Vec2::new(t, self.sample(t))
            })
            .collect()
    }

    pub fn sample(&self, x: f32) -> f32 {
        let pts = &self.control_points;
        let ks = &self.derivatives;

        let x = x.clamp(pts[0].x, pts[pts.len() - 1].x);

        let i = match pts.binary_search_by(|p| p.x.partial_cmp(&x).unwrap()) {
            Ok(0) => 1,
            Ok(i) => i,
            Err(i) => i.max(1),
        };

        let dx = pts[i].x - pts[i - 1].x;
        let t = (x - pts[i - 1].x) / dx;
        let dy = pts[i].y - pts[i - 1].y;

        let a = ks[i - 1] * dx - dy;
        let b = -ks[i] * dx + dy;

        ((1.0 - t) * pts[i - 1].y + t * pts[i].y + t * (1.0 - t) * (a * (1.0 - t) + b * t))
            .clamp(0.0, 1.0)
    }

    pub fn control_points(&self) -> &[Vec2] {
        &self.control_points
    }

    pub fn derivatives(&self) -> &[f32] {
        &self.derivatives
    }
}

pub struct CubicBezierCurve {
    pub p0: Vec2,
    pub p1: Vec2,
    pub p2: Vec2,
    pub p3: Vec2,
}

impl CubicBezierCurve {
    pub fn new(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2) -> Self {
        Self { p0, p1, p2, p3 }
    }

    pub fn sample(&self, t: f32) -> Vec2 {
        let mt = 1.0 - t;
        mt * mt * mt * self.p0
            + 3.0 * mt * mt * t * self.p1
            + 3.0 * mt * t * t * self.p2
            + t * t * t * self.p3
    }

    pub fn tangent(&self, t: f32) -> Vec2 {
        let mt = 1.0 - t;
        3.0 * mt * mt * (self.p1 - self.p0)
            + 6.0 * mt * t * (self.p2 - self.p1)
            + 3.0 * t * t * (self.p3 - self.p2)
    }
}

#[cfg(test)]
mod tests {
    use glam::vec2;

    use super::*;

    #[test]
    fn test_interpolates_through_control_points() {
        let points = vec![
            vec2(0.0, 0.0),
            vec2(0.25, 0.3),
            vec2(0.5, 0.6),
            vec2(0.75, 0.8),
            vec2(1.0, 1.0),
        ];
        let curve = CubicCurve::new(points.clone());

        for p in &points {
            let y = curve.sample(p.x);
            assert!(
                (y - p.y).abs() < 1e-4,
                "At x={}, expected y={}, got y={}",
                p.x,
                p.y,
                y
            );
        }
    }

    #[test]
    fn test_clamps_to_range() {
        let points = vec![vec2(0.0, 0.0), vec2(0.5, 0.7), vec2(1.0, 1.0)];
        let curve = CubicCurve::new(points);

        let y_low = curve.sample(-0.5);
        assert!((y_low - curve.sample(0.0)).abs() < 1e-6);

        let y_high = curve.sample(1.5);
        assert!((y_high - curve.sample(1.0)).abs() < 1e-6);
    }

    #[test]
    fn test_linear_passthrough() {
        let points = vec![vec2(0.0, 0.0), vec2(0.5, 0.5), vec2(1.0, 1.0)];
        let curve = CubicCurve::new(points);

        for i in 0..=20 {
            let x = i as f32 / 20.0;
            let y = curve.sample(x);
            assert!(
                (y - x).abs() < 1e-4,
                "At x={}, expected y={}, got y={}",
                x,
                x,
                y
            );
        }
    }
}
