// Multiplies the panel by a color.
//
// Parameters:
//   color    = [r, g, b]  Linear tint color, 0.0-1.0 per channel
//   strength = 0.0-1.0    How much of the tint to apply

struct EffectParams {
    color: vec3<f32>,
    strength: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let c = textureLoad(rio_input, vec2<i32>(pos), 0);
    let tint = mix(vec3<f32>(1.0), params.color, params.strength);
    return vec4<f32>(c.rgb * tint, c.a);
}
