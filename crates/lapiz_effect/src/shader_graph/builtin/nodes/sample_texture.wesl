fn sample_texture(texture: texture_2d<u32>, pixel: vec2u) -> vec4f {
    if any(pixel >= textureDimensions(texture)) {
        return vec4f(0.0);
    }
    return image::texture_unpack::unpack_rgba8_texel(textureLoad(texture, vec2i(pixel), 0));
}
