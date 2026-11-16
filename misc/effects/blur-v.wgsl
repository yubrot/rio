// Second half of a separable Gaussian blur (vertical).
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let weights = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    let step = vec2<f32>(0.0, 1.5 * rio_frame.scale) / rio_frame.resolution;
    let uv = pos / rio_frame.resolution;
    var sum = textureSampleLevel(rio_input, rio_sampler, uv, 0.0) * weights[0];
    for (var i = 1; i < 5; i++) {
        let o = step * f32(i);
        sum += textureSampleLevel(rio_input, rio_sampler, uv + o, 0.0) * weights[i];
        sum += textureSampleLevel(rio_input, rio_sampler, uv - o, 0.0) * weights[i];
    }
    return sum;
}
