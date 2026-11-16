# Custom effect shaders

Rio can run a chain of WGSL shaders over each terminal panel. Each shader is
an external file: Rio supplies the panel image, time, focus, theme colors,
cursor history, and typing events, and the shader decides what to draw and
for how long. Shaders reload while Rio runs.

Supported on the wgpu (Windows) and Metal (macOS) renderers. Other renderer
backends accept the configuration, show a warning, and render normally.

## Configuration

```toml
[effects.custom]
enabled = true
hot-reload = true # Reload shader files when they change (default: true)
max-fps = 60      # Frame cap while effects run, 1-240 (default: 60)

[[effects.custom.shaders]]
path = "effects/cursor-trail.wgsl" # Relative to the directory of config.toml
api-version = 1                    # Optional; defaults to the latest ABI

[effects.custom.shaders.params]
color = [0.75, 0.4, 1.0, 1.0]
width = 1.0
duration = 0.3

[[effects.custom.shaders]]
path = "effects/typing-sparks.wgsl"

[effects.custom.shaders.params]
color = [1.0, 0.8, 0.3, 1.0]
intensity = 0.5
speed = 120.0
duration = 0.24
```

- Shaders run in order; each one reads the previous one's output. At most 16.
- `path` may be absolute. On Windows, `%LOCALAPPDATA%\rio\effects\` is a
  typical place for shader files.
- `[platform.<os>.effects.custom]` overrides `enabled`, `hot-reload`,
  `max-fps`, and `shaders` (which replaces the whole chain) per platform.
- `api-version` pins a shader to an ABI version. Rio skips a shader whose
  version it does not implement. An omitted version means the latest, so a
  shader written for an older ABI may stop working after an ABI change.
  Shared shaders should state their version.

### Parameters

`params` sets the fields of the shader's own parameter struct by name. Rio
reads the struct from the shader, so the names, units, and meaning are up to
each shader:

```wgsl
struct EffectParams {
    color: vec4<f32>,
    width: f32,
}

@group(1) @binding(0) var<uniform> params: EffectParams;
```

- Fields must be `f32` or `vec2/3/4<f32>`: TOML numbers and arrays of 2–4
  numbers. At most 32 fields and 1024 bytes.
- Every field needs a value; unknown names, wrong types, and non-finite
  values are errors. Shaders without parameters omit both the struct and
  `params`.
- Changing only values updates the running shader without recompiling it.

## Writing a shader

A shader file defines one function:

```wgsl
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    return textureLoad(rio_input, vec2<i32>(pos), 0);
}
```

`pos` is the pixel center in panel-local physical pixels (top-left origin).
The result replaces the pixel: linear, premultiplied RGBA. To overlay
something, blend it yourself, for example `overlay + base * (1.0 - overlay.a)`.

Rio appends declarations after the file, so error line numbers match the file.
Identifiers starting with `rio_` are reserved. The file must not declare entry
points or bindings other than its parameter struct.

### Host data (ABI v1)

All geometry is in panel-local physical pixels. All times are seconds on one
clock (`rio_frame.time`). All colors are linear premultiplied, in the same
color space as the sampled image.

| Name | Contents |
| --- | --- |
| `rio_input: texture_2d<f32>` | The previous shader's output; the captured panel for the first shader |
| `rio_original: texture_2d<f32>` | The captured panel, unchanged by every shader |
| `rio_sampler: sampler` | Linear filtering, clamped to the edge |
| `rio_frame.resolution` | Panel size |
| `rio_frame.time`, `time_delta`, `frame` | Clock, time since the previous frame, frame counter |
| `rio_frame.scale`, `cell_size` | DPI scale factor, cell size |
| `rio_frame.focused`, `focus_time` | 1 while the panel has keyboard focus (focused window, active split); when that last changed |
| `rio_theme.foreground`, `background`, `cursor`, `selection_foreground`, `selection_background` | Theme colors, including the terminal's OSC overrides |
| `rio_theme.palette[256]` | The 256-color palette, including OSC 4 overrides |
| `rio_cursor.current`, `previous` | `rect` (x, y, w, h), `color`, `shape` (0 block, 1 underline, 2 beam, 3 hidden), `visible`, `blink_on`, `time` the state began. `previous` is the state before the last change |
| `rio_cursor.move_time`, `change_time` | Last position change; last shape, color, or visibility change |
| `rio_cursor.history[64]`, `history_len` | Recent positions (`rect`, arrival `time`), newest first |
| `rio_events.typing[64]`, `typing_len` | Recent inputs, newest first: `rect` (cursor cell displayed at input time), `time`, `seq`, `seed` (random bits), `kind` (0 text, 1 Enter, 2 Backspace/Delete, 3 IME commit) |

See `sugarloaf/src/effects/prelude.wgsl` for the exact declarations.

- The capture is the panel's whole area: background, background image slice,
  cells, images, and cursor. It does not include split dividers, the
  scrollbar, tabs, or other window UI.
- Sampling outside the panel clamps to its edge.
- The cursor history and typing events are per panel. They reset when the
  panel is resized or scrolled back. Switching tabs or splits never mixes
  histories.
- Typing events come from keys actually sent to the terminal (text, Enter,
  Backspace/Delete) and from IME commits (once per committed string). Paste,
  program output, key bindings, search, and Vi mode add none. A hidden cursor
  does not stop typing events.

## When shaders run

- While a chain is enabled, Rio renders visible panels every frame up to
  `max-fps`, even when the terminal is idle and the window is unfocused. A
  shader returning its input unchanged still costs a full pass, so idle
  windows with effects use some GPU and power.
- Rendering pauses for minimized windows (and occluded ones on macOS) and for
  inactive tabs. The clock keeps running, so compare event times with
  `rio_frame.time` instead of counting frames.
- `renderer.disable-unfocused-render` and `disable-occluded-render` stop
  effects along with the rest of the window's rendering.
  `renderer.strategy = "game"` renders every frame itself, and `max-fps` does
  not apply.

## Reloading and errors

- Saving a shader file or `config.toml` reloads the chain in place. Rio
  watches the shaders' directories, so editors that save by renaming work.
  Changes to files under `\\wsl.localhost` paths are not detected.
- A shader that fails to load (missing file, syntax or validation error,
  bad parameters, unsupported `api-version`) is reported in a warning toast
  and in the log, with file and line where available. If it worked before,
  its last working version and parameters keep running; otherwise only that
  shader is skipped. Fixing the file clears the warning.
- Validation cannot bound a shader's GPU time; keep loops bounded.

## Examples

| File | What it does | Parameters |
| --- | --- | --- |
| `identity.wgsl` | Passes the panel through | — |
| `wave.wgsl` | Shifts rows along an animated sine wave | — |
| `blur-h.wgsl`, `blur-v.wgsl` | Two-pass Gaussian blur; use both in this order | — |
| `glow.wgsl` | Adds the blurred input over the original; use after the blur pair | — |
| `split-compare.wgsl` | Debug: left half shows the chain so far, right half the original | — |
| `focus-dim.wgsl` | Fades unfocused panels toward the theme background | — |
| `tint.wgsl` | Multiplies the panel by a color | `color` (vec3), `strength` (0–1) |
| `focus-pulse.wgsl` | Pulsing border on the focused panel | `color`, `period` (s), `thickness` (px) |
| `cursor-trail.wgsl` | Fading trail along recent cursor positions | `color`, `width` (0–1 of cell height), `duration` (s) |
| `typing-sparks.wgsl` | Sparks from the cursor on each keystroke | `color`, `intensity` (0–1), `speed` (px/s), `duration` (s) |
| `typing-slash.wgsl` | Diagonal slash per keystroke; longer for Enter and IME | `color`, `length` (cells), `duration` (s) |
