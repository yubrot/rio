// Draws a fading trail along the cursor's recent positions.
//
// Parameters:
//   color    = [r, g, b, a]  Linear premultiplied trail color
//   width    = 0.0-1.0       Trail thickness relative to the cursor height
//   duration = seconds       How long a trail segment takes to fade out

struct EffectParams {
    color: vec4<f32>,
    width: f32,
    duration: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;

fn center(rect: vec4<f32>) -> vec2<f32> {
    return rect.xy + rect.zw * 0.5;
}

fn segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-4), 0.0, 1.0);
    return length(p - (a + ab * t));
}

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let base = textureLoad(rio_input, vec2<i32>(pos), 0);
    let now = rio_frame.time;
    // Nothing to draw once the newest segment has faded.
    if (rio_cursor.current.visible == 0u || now - rio_cursor.move_time > params.duration) {
        return base;
    }
    let half_width = rio_cursor.current.rect.w * params.width * 0.5;
    var alpha = 0.0;
    for (var i = 0u; i + 1u < rio_cursor.history_len; i++) {
        let newer = rio_cursor.history[i];
        // A segment starts fading when the cursor arrives at its newer end.
        let age = now - newer.time;
        if (age > params.duration) {
            break;
        }
        let fade = 1.0 - age / params.duration;
        let older = rio_cursor.history[i + 1u];
        let d = segment_distance(pos, center(older.rect), center(newer.rect));
        // Tapers as it fades; the +0.5 antialiases the edge.
        alpha = max(alpha, clamp(half_width * fade - d + 0.5, 0.0, 1.0) * fade);
    }
    let trail = params.color * alpha;
    return trail + base * (1.0 - trail.a);
}
