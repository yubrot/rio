// Host passes around an effect chain: `fs_decode` turns a captured panel
// (surface encoding) into linear premultiplied RGBA, `fs_encode` writes
// the chain output back into the surface at the panel's origin.

struct HostPass {
    origin: vec2<f32>,
    // 0 = premultiplied, 1 = straight, 2 = opaque
    alpha_mode: u32,
    _pad: u32,
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> host: HostPass;

struct VsOut {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var out: VsOut;
    let x = select(-1.0, 3.0, vid == 2u);
    let y = select(1.0, -3.0, vid == 0u);
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn to_encoded(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn load(pos: vec4<f32>) -> vec4<f32> {
    return textureLoad(src, vec2<i32>(pos.xy - host.origin), 0);
}

@fragment
fn fs_decode(in: VsOut) -> @location(0) vec4<f32> {
    let c = load(in.position);
    if (host.alpha_mode == 2u) {
        return vec4<f32>(to_linear(c.rgb), 1.0);
    }
    if (c.a <= 0.0) {
        return vec4<f32>(0.0);
    }
    var straight = c.rgb;
    if (host.alpha_mode == 0u) {
        straight = clamp(c.rgb / c.a, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(to_linear(straight) * c.a, c.a);
}

@fragment
fn fs_encode(in: VsOut) -> @location(0) vec4<f32> {
    let c = load(in.position);
    if (host.alpha_mode == 2u) {
        return vec4<f32>(to_encoded(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
    }
    let a = clamp(c.a, 0.0, 1.0);
    if (a <= 0.0) {
        return vec4<f32>(0.0);
    }
    let straight = to_encoded(clamp(c.rgb / a, vec3<f32>(0.0), vec3<f32>(1.0)));
    if (host.alpha_mode == 0u) {
        return vec4<f32>(straight * a, a);
    }
    return vec4<f32>(straight, a);
}
