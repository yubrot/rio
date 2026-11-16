// ---- Rio effect host prelude (ABI v1), appended after the user source ----

struct RioEffectFrame {
    // Panel size in physical pixels.
    resolution: vec2<f32>,
    // Seconds since the chain was installed.
    time: f32,
    // Seconds since the previous rendered frame.
    time_delta: f32,
    // Rendered frame counter.
    frame: u32,
    // DPI scale factor.
    scale: f32,
    // Terminal cell size in physical pixels.
    cell_size: vec2<f32>,
}

@group(0) @binding(0) var<uniform> rio_frame: RioEffectFrame;
// Output of the previous pass (the captured terminal for the first pass).
@group(0) @binding(1) var rio_input: texture_2d<f32>;
// The captured terminal, unchanged by any pass.
@group(0) @binding(2) var rio_original: texture_2d<f32>;
// Linear filtering, clamped to the edge.
@group(0) @binding(3) var rio_sampler: sampler;

struct RioEffectVertexOut {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn rio_effect_vs(@builtin(vertex_index) vid: u32) -> RioEffectVertexOut {
    var out: RioEffectVertexOut;
    let x = select(-1.0, 3.0, vid == 2u);
    let y = select(1.0, -3.0, vid == 0u);
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn rio_effect_fs(in: RioEffectVertexOut) -> @location(0) vec4<f32> {
    return effect_main(in.position.xy);
}
