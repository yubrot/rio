// Adds the blurred chain input on top of the unmodified terminal.
// Use after blur-h.wgsl + blur-v.wgsl for a glow.
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let p = vec2<i32>(pos);
    let sharp = textureLoad(rio_original, p, 0);
    let blurred = textureLoad(rio_input, p, 0);
    return vec4<f32>(sharp.rgb + blurred.rgb * 0.8, sharp.a);
}
