// Cuts a diagonal slash across the cursor cell on each keystroke; Enter and
// IME commits cut longer ones.
//
// Parameters:
//   color    = [r, g, b, a]  Linear premultiplied slash color
//   length   = cells         Slash length relative to the cell height
//   duration = seconds       How long a slash takes to fade out

struct EffectParams {
    color: vec4<f32>,
    length: f32,
    duration: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-4), 0.0, 1.0);
    return length(p - (a + ab * t));
}

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let base = textureLoad(rio_input, vec2<i32>(pos), 0);
    var alpha = 0.0;
    for (var e = 0u; e < rio_events.typing_len; e++) {
        let event = rio_events.typing[e];
        let age = rio_frame.time - event.time;
        if (age > params.duration) {
            break;
        }
        let t = age / params.duration;
        let long_cut = event.kind == 1u || event.kind == 3u;
        let len = event.rect.w * params.length * select(1.0, 3.0, long_cut);
        // The cut extends quickly, then thins out.
        let reach = len * 0.5 * min(t * 4.0, 1.0);
        let center = event.rect.xy + event.rect.zw * 0.5;
        let dir = normalize(vec2<f32>(1.0, -1.6));
        let d = segment_distance(pos, center - dir * reach, center + dir * reach);
        let width = 1.2 * rio_frame.scale * (1.0 - t);
        alpha = max(alpha, clamp(width - d + 0.5, 0.0, 1.0) * (1.0 - t));
    }
    let slash = params.color * alpha;
    return slash + base * (1.0 - slash.a);
}
