// Recolors the panel with two theme colors: the background and one palette
// entry. Text is mapped by how far its luminance sits from the background
// toward the foreground, so it works with dark and light themes alike.
//
// Parameters:
//   palette_index = 0-255    Palette entry for text (e.g. 4 for ANSI blue)
//   strength      = 0.0-1.0  Blend between the original and the duotone image

struct EffectParams {
    palette_index: f32,
    strength: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let c = textureLoad(rio_input, vec2<i32>(pos), 0);
    let bg = rio_theme.background.rgb;
    let index = u32(clamp(round(params.palette_index), 0.0, 255.0));
    let accent = rio_theme.palette[index].rgb;
    var span = luma(rio_theme.foreground.rgb) - luma(bg);
    span = select(span, 1e-3, abs(span) < 1e-3);
    let t = clamp((luma(c.rgb) - luma(bg)) / span, 0.0, 1.0);
    let duotone = mix(bg, accent, t) * c.a;
    return vec4<f32>(mix(c.rgb, duotone, params.strength), c.a);
}
