// Debug: the left half shows this pass's input, the right half the
// original capture, separated by a red line. Put it last in a chain to
// check that earlier passes do not alter `rio_original`.
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let p = vec2<i32>(pos);
    let mid = rio_frame.resolution.x * 0.5;
    if (abs(pos.x - mid) < rio_frame.scale) {
        return vec4<f32>(1.0, 0.0, 0.0, 1.0);
    }
    if (pos.x < mid) {
        return textureLoad(rio_input, p, 0);
    }
    return textureLoad(rio_original, p, 0);
}
