import image::image_tiling::{TILE_SIZE, TileInfo};
import package::icc_transform::calibrate_color;

struct Canvas {
    transform: mat3x3f,
    inverse_transform: mat3x3f,
    size: vec2u,
    tile_count: vec2u,
    tile_size: u32,
    time: f32,
}

@group(0) @binding(0) var tiles: texture_storage_2d_array<r32uint, read>;
@group(0) @binding(1) var<storage> tile_info: array<TileInfo>;
@group(0) @binding(2) var<uniform> canvas: Canvas;
// This is actually a bgra texture. Swap channels before writing to it.
@group(0) @binding(3) var output: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(4) var selection: texture_storage_2d_array<r8unorm, read>;
@group(0) @binding(5) var<storage> selection_tile_info: array<TileInfo>;

const BACKGROUND_COLOR: vec4f = vec4f(0.5, 0.5, 0.5, 1.0);

fn checkerboard_pattern(pos: vec2u) -> vec4f {
    let checker_size = 8u;
    let checker_pos = pos / checker_size;
    let is_dark = (checker_pos.x + checker_pos.y) % 2u == 0u;
    return mix(vec4f(0.6, 0.6, 0.6, 1.0), vec4f(0.4, 0.4, 0.4, 1.0), f32(is_dark));
}

fn tile_checkerboard_pattern(pos: vec2u) -> vec4f {
    let checker_pos = pos / (TILE_SIZE);
    let is_dark = (checker_pos.x + checker_pos.y) % 2u == 0u;
    return mix(vec4f(1.0, 0.9, 0.9, 1.0), vec4f(0.9, 1.0, 0.9, 1.0), f32(is_dark));
}

fn convert_pixel_to_tile_index(pixel: vec2i) -> vec3u {
    for (var i = 0u; i < arrayLength(&tile_info); i = i + 1u) {
        let info = tile_info[i];
        if pixel.x >= info.origin.x && pixel.x < info.origin.x + i32(TILE_SIZE) &&
            pixel.y >= info.origin.y && pixel.y < info.origin.y + i32(TILE_SIZE) {
            return vec3u(vec2u(pixel - info.origin), i);
        }
    }
    return vec3u(0, 0, 0xffffffffu);
}

fn sample_selection_binary(pixel: vec2i) -> bool {
    for (var i = 0u; i < arrayLength(&selection_tile_info); i += 1u) {
        let tile = selection_tile_info[i];
        if all(pixel >= tile.origin) && all(pixel < tile.origin + i32(TILE_SIZE)) {
            let local = pixel - tile.origin;
            let value = textureLoad(selection, local, i).r;
            return value != 0.0;
        }
    }

    return false;
}

fn is_at_selection_edge(center_ws: vec2i) -> bool {
    let center_ps = (canvas.inverse_transform * vec3f(vec2f(center_ws), 1.0)).xy;
    let center = sample_selection_binary(vec2i(center_ps));
    if !center {
        return false;
    }

    const OFFSETS: array<vec2i, 4> = array<vec2i, 4>(vec2i(1, 0), vec2i(0, 1), vec2i(-1, 0), vec2i(0, -1));
    for (var i = 0u; i < 4u; i += 1u) {
        let neighbour_ws = center_ws + OFFSETS[i];
        let neighbor_ps = (canvas.inverse_transform * vec3f(vec2f(neighbour_ws), 1.0)).xy;

        if sample_selection_binary(vec2i(neighbor_ps)) != center {
            return true;
        }
    }
    return false;
}

fn marching_ants(pixel: vec2f) -> vec4f {
    let w = (pixel.x + pixel.y) / 4.0 + canvas.time * 2.0;
    let v = select(0.0, 1.0, u32(w) % 2u != 0u);
    return vec4f(v, v, v, 1.0);
}

@compute
@workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) index: vec3u) {
    let pixel_pos = index.xy;
    let canvas_pos = (canvas.inverse_transform * vec3f(vec2f(pixel_pos), 1.0)).xy;

    if is_at_selection_edge(vec2i(pixel_pos)) {
        textureStore(output, pixel_pos, marching_ants(vec2f(pixel_pos)));
        return;
    }

    var color = BACKGROUND_COLOR;

    if all(canvas_pos >= vec2f(0.0)) && all(canvas_pos < vec2f(canvas.size)) {
        let tile_index = convert_pixel_to_tile_index(vec2i(canvas_pos));
        let img = image::texture_unpack::unpack_rgba8_texel(textureLoad(tiles, tile_index.xy, tile_index.z));
        let calibrated = calibrate_color(img.rgb);
        let transparent_checkerboard = checkerboard_pattern(vec2u(canvas_pos));
        color = mix(transparent_checkerboard, vec4f(calibrated, img.a), img.a);

        @if (DEBUG_TILE_CHECKERBOARD) {
            let tile_checkerboard = tile_checkerboard_pattern(vec2u(canvas_pos));
            color *= tile_checkerboard;
        }
    }

    textureStore(output, pixel_pos, color);
}
