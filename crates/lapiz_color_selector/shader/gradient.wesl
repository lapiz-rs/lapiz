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
    output_transform::pcs_to_output,
};

struct VertexInput {
    @location(0) position: vec2f,
    @location(1) uv: vec2f,
}

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

struct Settings {
    channel_ranges: array<vec4f, 3>,
    base_channels: vec3f,
    out_of_gamut_color: vec3f,
    use_out_of_gamut_color: u32,
    // Gray,
    // Hsl,
    // Hsv,
    // Lab,
    // Lch,
    // OkHsl,
    // OkHsv,
    // OkLab,
    // OkLch,
    // Rgb,
    // Xyz,
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

@group(0) @binding(0) var<uniform> settings: Settings;

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var position = in.position;
    if ((settings.flip_axis & 1u) != 0u) {
        position.x = -position.x;
    }
    if ((settings.flip_axis & 2u) != 0u) {
        position.y = -position.y;
    }

    let rotation_sin = sin(settings.rotation);
    let rotation_cos = cos(settings.rotation);
    position = vec2f(
        rotation_cos * position.x + rotation_sin * position.y,
        -rotation_sin * position.x + rotation_cos * position.y,
    );

    return VertexOutput(vec4f(position, 0.0, 1.0), in.uv);
}

fn channels_into_rgb(ch: vec3f) -> vec3f {
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

    let output = pcs_to_output(vec3f(xyz.x, xyz.y, xyz.z));
    if settings.use_out_of_gamut_color == 0u {
        return output;
    }

    let image = pcs_to_image(vec3f(xyz.x, xyz.y, xyz.z));
    let out_of_gamut = any(image > vec3f(1.0 + 1e-2)) || any(image < vec3f(-1e-2));
    return select(output, settings.out_of_gamut_color, out_of_gamut);
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

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@if(IS_SRGB_OUTPUT)
fn transform_output(c: vec3f) -> vec3f {
    return vec3f(srgb_to_linear(c.r), srgb_to_linear(c.g), srgb_to_linear(c.b));
}

@else
fn transform_output(c: vec3f) -> vec3f {
    return c;
}

@vertex
fn ring_vertex(in: VertexInput) -> VertexOutput {
    return VertexOutput(vec4f(in.position, 0.0, 1.0), in.uv);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4f {
    var normalized_uv = in.uv;
    if settings.plane_shape == 1u {
        normalized_uv.x = select(
            0.5,
            (in.uv.x - 0.5 * (1.0 - in.uv.y)) / in.uv.y,
            in.uv.y > 1e-6,
        );
    }
    let remapped_uv = vec2f(
        mix(settings.x_range.x, settings.x_range.y, normalized_uv.x),
        mix(settings.y_range.x, settings.y_range.y, normalized_uv.y),
    );
    var ch = settings.base_channels;
    if (settings.saturate_primary_channel != 0u) {
        ch = saturate_primary_channel(ch);
    }
    var j = 0u;
    for (var i = 0u; i < 3u; i += 1u) {
        if ((settings.variable_channels >> i) & 1u) != 0u {
            ch[i] = mix(
                settings.channel_ranges[i].x,
                settings.channel_ranges[i].y,
                remapped_uv[j],
            );
            j += 1u;
        }
    }

    return vec4f(transform_output(channels_into_rgb(ch)), 1.0);
}

@fragment
fn ring_fragment(in: VertexOutput) -> @location(0) vec4f {
    let antialias_width = 1.0 / settings.texture_size;
    let outer_radius = 0.5 - antialias_width;
    let inner_radius = max(outer_radius - settings.ring_width / settings.texture_size, 0.0);
    let radius = length(in.uv - vec2f(0.5));
    let inner_alpha = smoothstep(
        inner_radius - antialias_width,
        inner_radius + antialias_width,
        radius,
    );
    let outer_alpha = 1.0 - smoothstep(
        outer_radius - antialias_width,
        outer_radius + antialias_width,
        radius,
    );
    let alpha = inner_alpha * outer_alpha;

    let centered = in.uv - vec2f(0.5);
    var angle = atan2(centered.y, centered.x) + settings.ring_rotation;
    if (settings.reversed_ring != 0u) {
        angle = -angle;
    }
    let channel_factor = fract(angle / 6.283185307179586);

    var ch = settings.base_channels;
    if (settings.saturate_primary_channel != 0u) {
        ch = saturate_primary_channel(ch);
    }
    ch[settings.primary_channel] = mix(
        settings.channel_ranges[settings.primary_channel].x,
        settings.channel_ranges[settings.primary_channel].y,
        channel_factor,
    );

    return vec4f(transform_output(channels_into_rgb(ch)), alpha);
}
