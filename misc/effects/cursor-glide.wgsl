// Glides the cursor between positions: the edges facing the movement arrive
// first and the trailing edges catch up. A shape or color change (e.g. a
// mode switch in an editor) sends a ring out from the cursor.
//
// Parameters:
//   duration = seconds   Glide length; the ring lasts twice as long
//   opacity  = 0.0-1.0   Opacity of the gliding cursor

struct EffectParams {
    duration: f32,
    opacity: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;

// Approximates the drawn cursor within its cell.
fn shape_rect(rect: vec4<f32>, shape: u32) -> vec4<f32> {
    let line = 2.0 * rio_frame.scale;
    switch shape {
        case 1u: {
            let h = max(line, rect.w * 0.1);
            return vec4<f32>(rect.x, rect.y + rect.w - h, rect.z, h);
        }
        case 2u: { return vec4<f32>(rect.x, rect.y, max(line, rect.z * 0.15), rect.w); }
        default: { return rect; }
    }
}

fn ease_out(t: f32) -> f32 {
    let u = 1.0 - clamp(t, 0.0, 1.0);
    return 1.0 - u * u * u;
}

// Antialiased coverage of a convex quad given in either winding order.
fn quad_coverage(p: vec2<f32>, q: array<vec2<f32>, 4>) -> f32 {
    var area = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let a = q[i];
        let b = q[(i + 1u) % 4u];
        area += a.x * b.y - b.x * a.y;
    }
    let s = sign(area);
    var d = 1e9;
    for (var i = 0u; i < 4u; i++) {
        let a = q[i];
        let e = q[(i + 1u) % 4u] - a;
        let w = p - a;
        d = min(d, s * (e.x * w.y - e.y * w.x) / max(length(e), 1e-4));
    }
    return clamp(d + 0.5, 0.0, 1.0);
}

fn box_distance(p: vec2<f32>, rect: vec4<f32>) -> f32 {
    let half = rect.zw * 0.5;
    let q = abs(p - rect.xy - half) - half;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
}

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let base = textureLoad(rio_input, vec2<i32>(pos), 0);
    let cur = rio_cursor.current;
    if (cur.visible == 0u || cur.shape == 3u) {
        return base;
    }
    let now = rio_frame.time;
    var overlay = vec4<f32>(0.0);

    // history[0] is the current position; history[1] where it came from.
    let glide_age = now - rio_cursor.move_time;
    if (rio_cursor.history_len >= 2u && glide_age < params.duration) {
        let dst = shape_rect(cur.rect, cur.shape);
        let src = shape_rect(rio_cursor.history[1].rect, cur.shape);
        let dir = (dst.xy + dst.zw * 0.5) - (src.xy + src.zw * 0.5);
        let t = glide_age / params.duration;
        let offsets = array<vec2<f32>, 4>(
            vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0),
            vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
        );
        var quad: array<vec2<f32>, 4>;
        for (var i = 0u; i < 4u; i++) {
            let o = offsets[i];
            let facing = dot(o - 0.5, dir);
            let speed = select(select(1.5, 1.0, facing < 0.0), 2.5, facing > 0.0);
            quad[i] = mix(src.xy + src.zw * o, dst.xy + dst.zw * o, ease_out(t * speed));
        }
        overlay = cur.color * (quad_coverage(pos, quad) * params.opacity);
    }

    // `previous` holds the appearance before the last change.
    let ring_age = now - rio_cursor.change_time;
    let ring_time = params.duration * 2.0;
    let changed = cur.shape != rio_cursor.previous.shape
        || any(cur.color != rio_cursor.previous.color);
    if (changed && ring_age < ring_time) {
        let t = ring_age / ring_time;
        let radius = ease_out(t) * rio_frame.cell_size.y;
        let d = box_distance(pos, shape_rect(cur.rect, cur.shape));
        let ring = clamp(1.5 * rio_frame.scale - abs(d - radius), 0.0, 1.0) * (1.0 - t);
        let c = cur.color * ring;
        overlay = c + overlay * (1.0 - c.a);
    }
    return overlay + base * (1.0 - overlay.a);
}
