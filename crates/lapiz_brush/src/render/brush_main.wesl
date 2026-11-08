import brush::brush_types::{OutputSamples, ComputedPenInput};

struct DabBounds {
    bounds: vec4i,
    accumulator_bounds: vec4i,
}

struct BatchBounds {
    n_samples: u32,
    is_overflow: u32,
    entries: array<DabBounds>,
}

@group(0) @binding(0) var<storage, read> samples: OutputSamples;
@group(0) @binding(1) var<storage, read> initial_pen_input: ComputedPenInput;
@group(0) @binding(2) var<storage, read_write> batch_bounds: BatchBounds;
@group(0) @binding(3) var<uniform> sample_index: u32;

//CODEGEN_RESOURCES
//CODEGEN_OUTPUT
//CODEGEN_HELPERS

//CODEGEN_BOUNDS

fn graph(dispatch_index: vec2i, brush_sample: ComputedPenInput, main_accumulate_bounds: vec4i) {
    //CODEGEN_GRAPH
}

fn union_bounds(previous: vec4i, next: vec4i) -> vec4i {
    if any(next.xy >= next.zw) { return previous; }
    if any(previous.xy >= previous.zw) { return next; }
    return vec4i(min(previous.xy, next.xy), max(previous.zw, next.zw));
}

@if(EVAL)
@compute @workgroup_size(1)
fn eval() {
    let count = min(samples.n_samples, arrayLength(&batch_bounds.entries));
    batch_bounds.n_samples = count;
    batch_bounds.is_overflow = samples.is_overflow | u32(samples.n_samples > count);
    var accumulated_bounds = main_accumulate_bounds;
    for (var index = 0u; index < count; index += 1u) {
        batch_bounds.entries[index].accumulator_bounds = accumulated_bounds;
        //CODEGEN_RESET_BOUNDS
        graph(vec2i(0), samples.samples[index], accumulated_bounds);
        //CODEGEN_SAVE_BOUNDS
    }
}

@if(!EVAL)
@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) id: vec3u) {
    //CODEGEN_DISPATCH
    graph(dispatch_index, samples.samples[sample_index], batch_bounds.entries[sample_index].accumulator_bounds);
}
