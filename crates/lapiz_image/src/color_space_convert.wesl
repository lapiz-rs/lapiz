import image::image_tiling::{TileInfo, TILE_SIZE};
import package::icc_transform::transform_color;

@group(0) @binding(0) var tex: texture_storage_2d_array<r32uint, read_write>;

@compute
@workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) global_id: vec3u) {
    let color = image::texture_unpack::unpack_rgba8_texel(textureLoad(tex, global_id.xy, global_id.z));
    let transformed_color = transform_color(color.rgb);
    textureStore(tex, global_id.xy, global_id.z, image::texture_unpack::pack_rgba8_texel(vec4f(transformed_color, color.a)));
}
