// ---- Rio effect host prelude (ABI v1), appended after the user source ----

struct RioEffectFrame {
    // Panel size in physical pixels.
    resolution: vec2<f32>,
    // Seconds since effects started in this window; continues across reloads.
    time: f32,
    // Seconds since the previous rendered frame.
    time_delta: f32,
    // Rendered frame counter.
    frame: u32,
    // DPI scale factor.
    scale: f32,
    // Terminal cell size in physical pixels.
    cell_size: vec2<f32>,
    // 1 while the panel has keyboard focus (focused window, active split).
    focused: u32,
    // `time` at which `focused` last changed.
    focus_time: f32,
}

// Colors are linear premultiplied, matching the sampled terminal image.
struct RioEffectTheme {
    foreground: vec4<f32>,
    background: vec4<f32>,
    cursor: vec4<f32>,
    selection_foreground: vec4<f32>,
    selection_background: vec4<f32>,
    // The 256-color palette, including the terminal's OSC 4 overrides.
    palette: array<vec4<f32>, 256>,
}

@group(0) @binding(0) var<uniform> rio_frame: RioEffectFrame;
// Output of the previous pass (the captured terminal for the first pass).
@group(0) @binding(1) var rio_input: texture_2d<f32>;
// The captured terminal, unchanged by any pass.
@group(0) @binding(2) var rio_original: texture_2d<f32>;
// Linear filtering, clamped to the edge.
@group(0) @binding(3) var rio_sampler: sampler;
@group(0) @binding(4) var<uniform> rio_theme: RioEffectTheme;

// Cursor geometry is in panel-local physical pixels, top-left origin.
struct RioCursor {
    // x, y, width, height of the cursor cell.
    rect: vec4<f32>,
    // Linear premultiplied.
    color: vec4<f32>,
    // 0 = block, 1 = underline, 2 = beam, 3 = hidden.
    shape: u32,
    // 1 unless the terminal hides the cursor or the view is scrolled back.
    visible: u32,
    // 1 during the visible half of a blink cycle (always 1 without blink).
    blink_on: u32,
    // `rio_frame.time` at which this state began.
    time: f32,
}

struct RioCursorSample {
    rect: vec4<f32>,
    // `rio_frame.time` at which the cursor arrived at `rect`.
    time: f32,
}

struct RioEffectCursor {
    current: RioCursor,
    // The state before the last position or appearance change.
    previous: RioCursor,
    // Time of the last position change.
    move_time: f32,
    // Time of the last shape, color, or visibility change.
    change_time: f32,
    // Valid entries in `history`.
    history_len: u32,
    // Recent positions, newest first. Cleared on resize and scrollback.
    history: array<RioCursorSample, 64>,
}

@group(0) @binding(5) var<uniform> rio_cursor: RioEffectCursor;

struct RioTypingEvent {
    // Cursor cell displayed when the input happened.
    rect: vec4<f32>,
    // `rio_frame.time` of the input.
    time: f32,
    // Increases by one per event in this panel.
    seq: u32,
    // Random bits for this event.
    seed: u32,
    // 0 = text, 1 = Enter, 2 = Backspace/Delete, 3 = IME commit.
    kind: u32,
}

struct RioEffectEvents {
    // Valid entries in `typing`.
    typing_len: u32,
    // Recent inputs sent to the terminal, newest first. Cleared on resize
    // and scrollback, like the cursor history.
    typing: array<RioTypingEvent, 64>,
}

@group(0) @binding(6) var<uniform> rio_events: RioEffectEvents;

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
