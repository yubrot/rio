// Passes the input through unchanged.
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    return textureLoad(rio_input, vec2<i32>(pos), 0);
}
