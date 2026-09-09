use chrono::{DateTime, Utc};
use glam::Vec2;
use lapiz_input::mouse::PressedMouseState;
use ringbuffer::{AllocRingBuffer, RingBuffer};

use crate::render::{ComputedPenInput, PenInput, Time};

#[derive(Debug, Clone, Copy, Default)]
pub struct RawPenInput {
    pub position: Vec2,
    pub pressure: f32,
    pub tilt: Vec2,
    // TODO: split altitude and azimuth for better readability
    pub angle: Vec2,
    pub time: Time,
}

const TIMESTAMP_MOD: i64 = 1_000_000;

impl RawPenInput {
    pub fn new(position_ps: Vec2, stroke_begin: DateTime<Utc>, mouse: &PressedMouseState) -> Self {
        Self {
            position: position_ps,
            time: Time {
                now: (Utc::now().timestamp_micros() % TIMESTAMP_MOD) as f32,
                stroke_begin: (stroke_begin.timestamp_micros() % TIMESTAMP_MOD) as f32,
            },
            pressure: mouse.force,
            tilt: mouse.tilt,
            angle: Vec2::new(mouse.altitude, mouse.azimuth),
        }
    }
}

pub struct InputProcessor {
    samples: AllocRingBuffer<RawPenInput>,
    stabilizer: Box<dyn InputSampleStabilizer>,
    older_stable: Option<RawPenInput>,
    prev_stable: Option<RawPenInput>,
    last_sample: Option<ComputedPenInput>,
    arc_offset: f32,
}

impl Default for InputProcessor {
    fn default() -> Self {
        Self::new(32, Box::new(BasicStabilizer))
    }
}

impl InputProcessor {
    pub fn new(buffer_size: usize, stabilizer: Box<dyn InputSampleStabilizer>) -> Self {
        Self {
            samples: AllocRingBuffer::new(buffer_size),
            stabilizer,
            older_stable: None,
            prev_stable: None,
            last_sample: None,
            arc_offset: 0.0,
        }
    }

    pub fn set_buffer_size(&mut self, buffered_samples: usize) {
        let mut new_buffer = AllocRingBuffer::new(buffered_samples);
        new_buffer.extend(self.samples.clone());
        self.samples = new_buffer;
    }

    pub fn set_stabilizer(&mut self, stabilizer: Box<dyn InputSampleStabilizer>) {
        self.stabilizer = stabilizer;
    }

    pub fn push(&mut self, input: RawPenInput) -> Option<PenInput> {
        self.samples.enqueue(input);

        let stabilized = self.stabilizer.stabilize(&self.samples)?;

        let cp = match (self.older_stable, self.prev_stable) {
            (Some(older), Some(prev)) => {
                let new_pos = stabilized.position;
                let tangent_start = (new_pos - older.position) * 0.5;
                let tangent_end = new_pos - prev.position;
                let cp1 = prev.position + tangent_start / 3.0;
                let cp2 = new_pos - tangent_end / 3.0;
                Some((cp1, cp2))
            }
            (None, Some(prev)) => {
                let new_pos = stabilized.position;
                let d = new_pos - prev.position;
                let cp1 = prev.position + d / 3.0;
                let cp2 = prev.position + d * (2.0 / 3.0);
                Some((cp1, cp2))
            }
            _ => None,
        };

        self.older_stable = self.prev_stable;
        self.prev_stable = Some(stabilized);

        let (bezier_control_prev, bezier_control_next) = cp?;

        Some(PenInput {
            position: input.position,
            pressure: input.pressure,
            tilt: input.tilt,
            angle: input.angle,
            time: input.time,
            bezier_control_prev,
            bezier_control_next,
        })
    }

    pub fn reset(&mut self) {
        self.samples.clear();
        self.older_stable = None;
        self.prev_stable = None;
        self.last_sample = None;
        self.arc_offset = 0.0;
    }

    pub fn flush(&mut self, final_input: RawPenInput) -> Vec<PenInput> {
        let steps = self.stabilizer.convergence_steps();
        let mut result = Vec::new();
        for _ in 0..steps {
            result.extend(self.push(final_input));
        }
        result
    }
}

pub trait InputSampleStabilizer: Send + Sync + 'static {
    fn stabilize(&mut self, inputs: &AllocRingBuffer<RawPenInput>) -> Option<RawPenInput>;
    fn convergence_steps(&self) -> usize;
}

pub struct GaussianStabilizer {
    kernel: Vec<f32>,
}

impl GaussianStabilizer {
    pub fn new(radius: usize) -> Self {
        if radius == 0 {
            return Self { kernel: vec![1.0] };
        }

        let window = 2 * radius + 1;
        let sigma = radius as f32 / 2.0;
        let center = radius as f32;

        let mut kernel: Vec<f32> = (0..window)
            .map(|i| {
                let x = i as f32 - center;
                (-x * x / (2.0 * sigma * sigma)).exp()
            })
            .collect();

        let sum: f32 = kernel.iter().sum();
        kernel.iter_mut().for_each(|k| *k /= sum);

        Self { kernel }
    }
}

impl InputSampleStabilizer for GaussianStabilizer {
    fn stabilize(&mut self, inputs: &AllocRingBuffer<RawPenInput>) -> Option<RawPenInput> {
        if inputs.is_empty() {
            return None;
        }

        let window = self.kernel.len();
        let available = inputs.len().min(window);
        let kernel_offset = window - available;

        let mut weighted_pos = Vec2::ZERO;
        let mut weighted_pressure = 0.0_f32;
        let mut weighted_tilt = Vec2::ZERO;
        let mut weighted_angle = Vec2::ZERO;

        let mut weight_sum = 0.0_f32;
        for (i, sample) in inputs.iter().rev().take(available).enumerate() {
            let w = self.kernel[kernel_offset + i];
            weighted_pos += sample.position * w;
            weighted_pressure += sample.pressure * w;
            weighted_tilt += sample.tilt * w;
            weighted_angle += sample.angle * w;
            weight_sum += w;
        }

        Some(RawPenInput {
            position: weighted_pos / weight_sum,
            pressure: weighted_pressure / weight_sum,
            tilt: weighted_tilt / weight_sum,
            angle: weighted_angle / weight_sum,
            time: inputs.back().unwrap().time,
        })
    }

    fn convergence_steps(&self) -> usize {
        self.kernel.len()
    }
}

pub struct BasicStabilizer;

impl InputSampleStabilizer for BasicStabilizer {
    fn stabilize(&mut self, inputs: &AllocRingBuffer<RawPenInput>) -> Option<RawPenInput> {
        inputs.back().copied()
    }

    fn convergence_steps(&self) -> usize {
        1
    }
}
