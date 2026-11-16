// Throws sparks from the cursor on each keystroke.
//
// Parameters:
//   color     = [r, g, b, a]    Linear premultiplied spark color
//   intensity = 0.0-1.0         Spark brightness
//   speed     = pixels/second   Initial spark speed
//   duration  = seconds         Spark lifetime

struct EffectParams {
    color: vec4<f32>,
    intensity: f32,
    speed: f32,
    duration: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;

const SPARKS: u32 = 6u;

fn hash(x: u32) -> u32 {
    var h = x;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return h;
}

fn rand(seed: u32, i: u32) -> f32 {
    return f32(hash(seed + i * 0x9e3779b9u) & 0xffffu) / 65535.0;
}

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let base = textureLoad(rio_input, vec2<i32>(pos), 0);
    var glow = 0.0;
    for (var e = 0u; e < rio_events.typing_len; e++) {
        let event = rio_events.typing[e];
        let age = rio_frame.time - event.time;
        // Newest first: every later event is older still.
        if (age > params.duration) {
            break;
        }
        let life = 1.0 - age / params.duration;
        let origin = event.rect.xy + event.rect.zw * 0.5;
        for (var k = 0u; k < SPARKS; k++) {
            let angle = rand(event.seed, k) * 6.2831853;
            let speed = params.speed * (0.4 + 0.6 * rand(event.seed, k + SPARKS));
            let p = origin + vec2<f32>(cos(angle), sin(angle)) * speed * age;
            let radius = 1.5 * rio_frame.scale * life;
            glow += clamp(radius - length(pos - p) + 0.5, 0.0, 1.0) * life;
        }
    }
    let spark = params.color * clamp(glow * params.intensity, 0.0, 1.0);
    return spark + base * (1.0 - spark.a);
}
