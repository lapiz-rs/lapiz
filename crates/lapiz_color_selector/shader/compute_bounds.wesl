import color::{
    gray::{Gray, gray_into_xyz},
    hsl::{Hsl, hsl_into_rgb},
    hsv::{Hsv, hsv_into_rgb},
    lab::{Lab, lab_into_xyz},
    lch::{Lch, lch_into_xyz},
    okhsl::{OkHsl, okhsl_into_xyz},
    okhsv::{OkHsv, okhsv_into_xyz},
    oklab::{OkLab, oklab_into_xyz},
    oklch::{OkLch, oklch_into_xyz},
    rgb::{Rgb, rgb_from_xyz, rgb_into_xyz},
    xyz::Xyz,
};

import package::{
    input_transform::picker_to_pcs,
    image_transform::pcs_to_image,
};

struct Settings {
    channel_ranges: array<vec4f, 3>,
    base_channels: vec3f,
    out_of_gamut_color: vec3f,
    use_out_of_gamut_color: u32,
    color_model: u32,
    variable_channels: u32,
    plane_shape: u32,
    rotation: f32,
    flip_axis: u32,
    primary_channel: u32,
    ring_rotation: f32,
    reversed_ring: u32,
    saturate_primary_channel: u32,
    ring_width: f32,
    texture_size: f32,
    x_range: vec2f,
    y_range: vec2f,
}

struct Params {
    resolution: vec2u,
}

struct OutputBounds {
    x_min: atomic<u32>,
    y_min: atomic<u32>,
    x_max: atomic<u32>,
    y_max: atomic<u32>,
}

@group(0) @binding(0) var<uniform> settings: Settings;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> output_bounds: OutputBounds;

fn channels_into_image(ch: vec3f) -> vec3f {
    var xyz = Xyz(0.0, 0.0, 0.0);
    switch settings.color_model {
        case 0: {
            xyz = gray_into_xyz(Gray(ch[0]));
        }
        case 1: {
            let rgb = hsl_into_rgb(Hsl(ch[0], ch[1], ch[2]));
            let _xyz = picker_to_pcs(vec3f(rgb.r, rgb.g, rgb.b));
            xyz = Xyz(_xyz[0], _xyz[1], _xyz[2]);
        }
        case 2: {
            let rgb = hsv_into_rgb(Hsv(ch[0], ch[1], ch[2]));
            let _xyz = picker_to_pcs(vec3f(rgb.r, rgb.g, rgb.b));
            xyz = Xyz(_xyz[0], _xyz[1], _xyz[2]);
        }
        case 3: {
            xyz = lab_into_xyz(Lab(ch[0], ch[1], ch[2]));
        }
        case 4: {
            xyz = lch_into_xyz(Lch(ch[0], ch[1], ch[2]));
        }
        case 5: {
            xyz = okhsl_into_xyz(OkHsl(ch[0], ch[1], ch[2]));
        }
        case 6: {
            xyz = okhsv_into_xyz(OkHsv(ch[0], ch[1], ch[2]));
        }
        case 7: {
            xyz = oklab_into_xyz(OkLab(ch[0], ch[1], ch[2]));
        }
        case 8: {
            xyz = oklch_into_xyz(OkLch(ch[0], ch[1], ch[2]));
        }
        case 9: {
            let _xyz = picker_to_pcs(ch);
            xyz = Xyz(_xyz[0], _xyz[1], _xyz[2]);
        }
        case 10: {
            xyz = Xyz(ch[0], ch[1], ch[2]);
        }
        default: {}
    }

    return pcs_to_image(vec3f(xyz.x, xyz.y, xyz.z));
}

fn saturate_primary_channel(ch: vec3f) -> vec3f {
    var saturated = ch;
    switch settings.color_model {
        case 1, 5: {
            saturated[1] = 1.0;
            saturated[2] = 0.5;
        }
        case 2, 6: {
            saturated[1] = 1.0;
            saturated[2] = 1.0;
        }
        default: {}
    }
    return saturated;
}

@compute
@workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) global_id: vec3u) {
    if any(global_id.xy >= params.resolution) {
        return;
    }

    let uv = vec2f(global_id.xy) / vec2f(params.resolution - vec2u(1u));
    var ch = settings.base_channels;
    if settings.saturate_primary_channel != 0u {
        ch = saturate_primary_channel(ch);
    }

    var variable_index = 0u;
    for (var channel = 0u; channel < 3u; channel += 1u) {
        if ((settings.variable_channels >> channel) & 1u) != 0u {
            ch[channel] = mix(
                settings.channel_ranges[channel].x,
                settings.channel_ranges[channel].y,
                uv[variable_index],
            );
            variable_index += 1u;
        }
    }

    let image = channels_into_image(ch);
    let in_gamut = all(image <= vec3f(1.0 + 1e-2)) && all(image >= vec3f(-1e-2));
    if in_gamut {
        atomicMin(&output_bounds.x_min, global_id.x);
        atomicMin(&output_bounds.y_min, global_id.y);
        atomicMax(&output_bounds.x_max, global_id.x);
        atomicMax(&output_bounds.y_max, global_id.y);
    }
}
