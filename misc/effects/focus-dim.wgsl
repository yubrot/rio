// Fades panels without keyboard focus toward the theme background.
//
// Parameters:
//   amount = 0.0-1.0  How far unfocused panels fade
//   fade   = seconds  Transition length; it advances only while frames render

struct EffectParams {
    amount: f32,
    fade: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let c = textureLoad(rio_input, vec2<i32>(pos), 0);
    let focused = f32(rio_frame.focused);
    let t = clamp((rio_frame.time - rio_frame.focus_time) / max(params.fade, 1e-3), 0.0, 1.0);
    // Dim weight: from the previous focus state toward the current one.
    let w = mix(focused, 1.0 - focused, t) * params.amount;
    return vec4<f32>(mix(c.rgb, rio_theme.background.rgb * c.a, w), c.a);
}
