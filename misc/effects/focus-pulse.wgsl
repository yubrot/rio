// Pulses a soft border around the focused panel; other panels are unchanged.
//
// Parameters:
//   color     = [r, g, b, a]  Linear premultiplied border color
//   period    = seconds       Length of one pulse
//   thickness = pixels        Border width

struct EffectParams {
    color: vec4<f32>,
    period: f32,
    thickness: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let base = textureLoad(rio_input, vec2<i32>(pos), 0);
    if (rio_frame.focused == 0u) {
        return base;
    }
    let size = rio_frame.resolution;
    let edge = min(min(pos.x, size.x - pos.x), min(pos.y, size.y - pos.y));
    let pulse = 0.5 + 0.5 * sin(rio_frame.time * 6.2831853 / params.period);
    let a = clamp(1.0 - edge / params.thickness, 0.0, 1.0) * pulse;
    let border = params.color * a;
    return border + base * (1.0 - border.a);
}
