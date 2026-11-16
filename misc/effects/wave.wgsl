// Spatial distortion: shifts each row horizontally along a sine wave.
// Animates with `rio_frame.time`.
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let amplitude = 4.0 * rio_frame.scale;
    let wavelength = 6.0 * rio_frame.cell_size.y;
    let shift = amplitude * sin(pos.y / wavelength * 6.2831853 + rio_frame.time * 2.0);
    let uv = (pos + vec2<f32>(shift, 0.0)) / rio_frame.resolution;
    return textureSampleLevel(rio_input, rio_sampler, uv, 0.0);
}
