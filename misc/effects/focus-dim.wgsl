// Fades panels without keyboard focus toward the theme background.

const DIM: f32 = 0.5;
// Seconds; the fade advances only while frames are rendered.
const FADE: f32 = 0.2;

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let c = textureLoad(rio_input, vec2<i32>(pos), 0);
    let focused = f32(rio_frame.focused);
    let t = clamp((rio_frame.time - rio_frame.focus_time) / FADE, 0.0, 1.0);
    // Dim weight: from the previous focus state toward the current one.
    let w = mix(focused, 1.0 - focused, t) * DIM;
    return vec4<f32>(mix(c.rgb, rio_theme.background.rgb * c.a, w), c.a);
}
