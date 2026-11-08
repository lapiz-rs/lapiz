import image::image_tiling::{TileInfo};
const MAIN_WORKGROUP_SIZE = vec3u(16, 16, 1);
const ESTIMATION_WORKGROUP_SIZE = vec3u(1, 1, 16);

struct StrokeInfo {
    // In tiles
    accumulated_bound_min: vec2i,
    accumulated_bound_max: vec2i,
    total_dabs: u32,
}

struct OutputSamples {
    n_samples: u32,
    is_overflow: u32,
    samples: array<ComputedPenInput>,
}

struct PenInputBatch {
    n_inputs: u32,
    inputs: array<PenInput>,
}

struct DabInfos {
    n_dabs: u32,
    buf: array<DabInfo>,
}

struct DabInfo {
    // In tiles
    bound_min: vec2i,
    bound_max: vec2i,
}

struct ComputedPenInput {
    position: vec2f,
    draw_direction_vec: vec2f,
    tilt: vec2f,
    angle: vec2f,
    draw_direction_angle: f32,
    pressure: f32,
    dab_index: u32,
    stroke_distance: f32,
    time: Time,
}

struct PenInputSampler {
    last_input: PenInput,
    last_sample: ComputedPenInput,
    has_last_sample: u32,
    has_initial_input: u32,
    next_dab_index: u32,
    distance_to_next_dab: f32,
    stroke_distance: f32,
}

struct PenInput {
    position: vec2f,
    tilt: vec2f,
    angle: vec2f,
    pressure: f32,
    time: Time,
    // Inner control points of the cubic Bezier from the previous position to
    // this one.  Computed on the CPU by BezierInputSampler.
    bezier_control_prev: vec2f,
    bezier_control_cur: vec2f,
}

struct Time {
    now: f32,
    stroke_begin: f32,
}
