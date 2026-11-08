import package::brush_types::{PenInput, PenInputBatch, ComputedPenInput, PenInputSampler, OutputSamples, Time};
import render::math::{bezier_eval, bezier_tangent_eval, mix_angle};

@group(0) @binding(0) var<storage> input_batch: PenInputBatch;
@group(0) @binding(1) var<storage, read_write> input_sampler: PenInputSampler;
@group(0) @binding(2) var<storage, read_write> output_samples: OutputSamples;
@group(0) @binding(3) var<storage, read_write> initial_pen_input: ComputedPenInput;

//CODEGENFLAG_INJECTED_RESOURCES

const BEZIER_SAMPLES: u32 = 32u;

fn compute_required_spacing(brush_sample: ComputedPenInput) -> f32 {
    //CODEGENFLAG_COMPUTED_GRAPH_REQUIRED_SPACING
}

fn bezier_t_for_arc_length(table: ptr<function, array<f32, BEZIER_SAMPLES + 1u>>, arc_goal: f32) -> f32 {
    const BISECT_ITERATIONS: u32 = u32(ceil(log2(f32(BEZIER_SAMPLES)))) + 1u;

    var lo: u32 = 0u;
    var hi: u32 = BEZIER_SAMPLES;
    for (var i = 0u; i < BISECT_ITERATIONS; i++) {
        let mid = (lo + hi) / 2u;
        if (*table)[mid] < arc_goal {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let lo_arc = (*table)[lo];
    let hi_arc = (*table)[hi];
    let lo_t = f32(lo) / f32(BEZIER_SAMPLES);
    let hi_t = f32(hi) / f32(BEZIER_SAMPLES);
    if hi_arc <= lo_arc {
        return lo_t;
    }
    return mix(lo_t, hi_t, (arc_goal - lo_arc) / (hi_arc - lo_arc));
}

fn compute_pen_input(last: PenInput, current: PenInput, p0: vec2f, p1: vec2f, p2: vec2f, p3: vec2f, t: f32, dab_index: u32, stroke_distance: f32) -> ComputedPenInput {
    let position = bezier_eval(p0, p1, p2, p3, t);
    let pressure = mix(last.pressure, current.pressure, t);
    let tilt = vec2f(
        mix_angle(last.tilt.x, current.tilt.x, t),
        mix_angle(last.tilt.y, current.tilt.y, t),
    );
    let angle = vec2f(
        mix_angle(last.angle.x, current.angle.x, t),
        mix_angle(last.angle.y, current.angle.y, t),
    );
    let time = Time(mix(last.time.now, current.time.now, t), last.time.stroke_begin);
    let draw_direction_vec = bezier_tangent_eval(p0, p1, p2, p3, t);
    let draw_direction_angle = atan2(draw_direction_vec.y, draw_direction_vec.x);
    return ComputedPenInput(position, draw_direction_vec, tilt, angle, draw_direction_angle, pressure, dab_index, stroke_distance, time);
}

fn sample_input(new_input: PenInput) {
    let p0 = input_sampler.last_input.position;
    let p1 = new_input.bezier_control_prev;
    let p2 = new_input.bezier_control_cur;
    let p3 = new_input.position;

    let new_computed = compute_pen_input(
        new_input,
        new_input,
        p0,
        p1,
        p2,
        p3,
        1.0,
        input_sampler.next_dab_index,
        input_sampler.stroke_distance,
    );

    if input_sampler.has_last_sample == 0 {
        input_sampler.last_input = new_input;
        input_sampler.last_sample = new_computed;
        input_sampler.has_last_sample = 1;

        return;
    }

    var arc_table: array<f32, BEZIER_SAMPLES + 1u>;
    arc_table[0] = 0.0;
    var prev_p = p0;
    for (var i: u32 = 1u; i <= BEZIER_SAMPLES; i++) {
        let t = f32(i) / f32(BEZIER_SAMPLES);
        let p = bezier_eval(p0, p1, p2, p3, t);
        arc_table[i] = arc_table[i - 1u] + distance(prev_p, p);
        prev_p = p;
    }
    let total_arc = arc_table[BEZIER_SAMPLES];

    if total_arc < 0.0001 {
        input_sampler.last_input = new_input;
        return;
    }

    var target_arc = input_sampler.distance_to_next_dab;
    while target_arc <= total_arc && output_samples.n_samples < arrayLength(&output_samples.samples) {
        let t = bezier_t_for_arc_length(&arc_table, target_arc);
        let interpolated = compute_pen_input(
            input_sampler.last_input,
            new_input,
            p0,
            p1,
            p2,
            p3,
            t,
            input_sampler.next_dab_index,
            input_sampler.stroke_distance + target_arc,
        );
        let spacing = compute_required_spacing(interpolated);
        if spacing <= 0.0 {
            input_sampler.last_input = new_input;
            return;
        }
        input_sampler.next_dab_index += 1u;

        output_samples.samples[output_samples.n_samples] = interpolated;
        output_samples.n_samples += 1u;
        target_arc += spacing;

        if input_sampler.has_initial_input == 0 {
            initial_pen_input = interpolated;
            input_sampler.has_initial_input = 1;
        }
    }

    if target_arc <= total_arc {
        output_samples.is_overflow = 1u;
    }
    input_sampler.distance_to_next_dab = target_arc - total_arc;
    input_sampler.stroke_distance += total_arc;
    input_sampler.last_input = new_input;
    if output_samples.n_samples > 0u {
        input_sampler.last_sample = output_samples.samples[output_samples.n_samples - 1u];
    }
}

@compute
@workgroup_size(1, 1, 1)
fn main() {
    output_samples.n_samples = 0u;
    output_samples.is_overflow = 0u;

    for (var i = 0u; i < input_batch.n_inputs; i++) {
        sample_input(input_batch.inputs[i]);
        if output_samples.is_overflow != 0u {
            return;
        }
    }
}
