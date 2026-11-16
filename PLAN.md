# PLAN

## External WGSL post-processing for terminal visuals

### Goals and assumptions

- Build a general terminal post-processing API, with terminal image access, ordered shader chaining, and continuous animation in the initial scope. Preserve the expressive capabilities of Ghostty's shader model without requiring source/configuration compatibility.
- Reproduce cursor trails and typing sparks/slashes throughout Rio, inspired by [this article](https://techblog.spiderplus.co.jp/entry/2026/02/05/120000), without shell- or editor-specific plugins.
- Load effect implementations from external files referenced in configuration. Adding an effect or changing its appearance must require neither Rust changes nor rebuilding Rio. Reload files on save.
- **Cursor trails, sparks, and slashes are implemented entirely outside Rio.** Do not embed their algorithms, shaders, or dedicated presets in the application. User-provided WGSL and configuration implement all visual behavior.
- **Installing the execution infrastructure initially requires building/updating Rio once.** Existing binaries cannot acquire this capability through configuration alone. After installation, users need neither Cargo nor an external shader compiler.
- Support Windows through wgpu and macOS through native Metal. Native Linux/Vulkan support follows later; disable custom effects on the CPU renderer.
- Disabled by default. When disabled, allocate no effect history and introduce no extra GPU passes or animation redraws.
- When enabled, continuously execute visible shader chains, including unfocused panels. The recurring CPU/GPU and power costs are accepted by design. Do not add trigger/animation selectors or automatic idle/focus throttling; retain the shared FPS cap and suspension for hidden panels.

### Existing integration points

| Location | Current behavior and intended use |
| --- | --- |
| `rio-backend/src/config/effects.rs` | Defines `[effects]` and existing `trail-cursor-*` options. Extend it with external shader configuration. |
| `frontends/rioterm/src/renderer/trail_cursor.rs` | Implements a built-in four-corner smear in Rust. Reference its geometry, reset handling, and scheduling, but do not use it to implement external effects. |
| `frontends/rioterm/src/screen/mod.rs` | Handles keys and obtains rendered cursor positions. Connect input collection, panel coordinate conversion, and animation scheduling here. |
| `frontends/rioterm/src/application.rs` | Routes IME commits and modal input. Collect commits without duplicating ordinary key events. |
| `frontends/rioterm/src/watcher.rs` | Watches the configuration directory non-recursively. Additional watches are required for shader subdirectories and external paths. |
| `sugarloaf/src/components/filters/` | Loads external RetroArch presets through wgpu-only librashader. It exposes neither typing/cursor events nor direct Ghostty GLSL support. |
| `sugarloaf/src/sugarloaf.rs`, `sugarloaf/src/renderer/` | Separate wgpu, Metal, Vulkan, and CPU rendering paths. Separate shared effect data from backend-specific execution. |

### Execution model and ownership

The initial implementation is **a chain of external WGSL image-processing passes**. Each pass samples the preceding pass's output and returns the complete replacement image, including alpha. The first pass receives the rendered terminal. Overlay-style effects sample and preserve that image themselves; they are one use of this general model.

Terminal image sampling, ordered pass chaining, continuous animation, cursor state, focus state, and theme colors are initial requirements. Do not freeze an API limited to cursor/typing overlays. Previous-frame feedback is a separate feature from passing the current frame between shaders and remains follow-up work.

Do not add branches keyed by effect names or application state/calculations specific to an effect's appearance.

| Generic infrastructure owned by Rio | Visual behavior owned by external WGSL/configuration |
| --- | --- |
| Cursor geometry, input events, time, and bounded history | Trail geometry, interpolation, following behavior, width, color, and fading |
| Buffer capacity, continuous frame scheduling, and FPS limits | Particle count, initial velocity, trajectories, changes over time, and slash geometry/movement |
| File loading, validation, reload, GPU execution, image inputs, and pass chaining | Mapping events to visuals, image distortion, recoloring, blur, glow, and combining effects |

Distribute examples as independent files alongside configuration examples. Never embed them using `include_str!` or equivalent. Creating trails/sparks/slashes in the milestones below means creating these external examples. Without installing and configuring them, none of these new visuals exists. Preserve the existing built-in `trail-cursor` as a separate compatibility feature; external examples must not call its implementation.

- Rio owns the vertex shader, bindings, render targets, clipping, and event buffers. External files implement a fixed `effect_main` signature returning the complete output pixel as premultiplied RGBA. Disable blending for image replacement passes; shaders implement any overlay composition. Version the ABI; a shader's `api-version` defaults to the latest when omitted.
- Render each panel's terminal into a sampleable offscreen texture, including background, text, selection, images, and the normal cursor. Run its configured passes in order and composite the final image before tabs/modals and other application UI.
- Capture boundary: the panel's full allocated rectangle including its padding, containing everything that area shows without effects — the window clear color (with opacity), the matching slice of the background image, cell backgrounds/text, kitty images, and the cursor. Exclude split dividers, scrollbar, and window-level UI (titlebar, island, search, assistant, modals). Write the final pass into that rectangle with blending disabled, so the chain output fully owns it (matching Ghostty, whose per-surface shader input includes the background).
- This requires splitting today's single render pass: wgpu currently draws all panels' grids, kitty images (inside `BrushRenderer::render`), and UI overlays into one swapchain pass, and Metal is similar. Separate per-panel terminal content (grid + images) from window UI on both backends before freezing the ABI.
- Expose both the current pass input and the original unmodified terminal image, plus a sampler. Use distinct read/write textures and explicit render-to-sample transitions. Ping-pong intermediate targets; never sample a texture while writing it. Reuse allocations across frames and recreate them on size/format changes.
- Run passes over the full panel so displaced pixels and neighborhood sampling work beyond the original cursor/damage bounds. Define pixel centers, UV orientation, edge clamping, and filtering. Preserve alpha through all passes.
- Color space: shaders see and return premultiplied linear values in the surface's primaries (sRGB or DisplayP3); palette/semantic color uniforms use the same space. Today's surfaces are non-sRGB 8-bit formats (`Bgra8Unorm` on wgpu outside macOS and on Metal) with gamma-space blending, so captures hold gamma-encoded premultiplied values. Decode at capture and re-encode at final composite; use `Rgba16Float` (or sRGB-encoded views) for intermediates to avoid banding in 8-bit linear. Decoding gamma-premultiplied pixels is inexact for partially transparent ones; measure the error with transparent windows in M1 and document it.
- Cache the terminal image while its content is unchanged; rerun the chain for animation without regenerating glyphs or reading pixels back to the CPU. With the chain disabled/empty, keep the existing direct-render path and allocate no offscreen targets.
- Compile and validate WGSL at runtime on wgpu. First validate Naga WGSL-to-MSL translation and runtime Metal compilation for the native Metal path. Promote Naga from dev-dependencies to normal dependencies with `wgsl-in` plus `msl-out` on macOS, pinned to the same Naga version wgpu uses on Windows so it is not built twice.
- wgpu reports `create_shader_module`/pipeline errors to the device error handler (a panic by default), not as `Result`s. Always validate with Naga first, then wrap module/pipeline creation in `push_error_scope`/`pop_error_scope` and treat a captured error as a failed load. This is what keeps invalid shaders from terminating Rio.
- Define a WGSL subset usable by both backends. Prove binding, layout, coordinate, and color consistency before freezing the ABI. Do not require changing macOS's default backend.
- Preserve `trail-cursor`; examples set it to `false` to avoid overlapping trails. Allow coexistence. The captured image includes the normal cursor; cursor-aware shaders may replace its pixels using image processing. A separate cursor-free texture/control is outside the initial scope.
- Keep this independent of `renderer.filters`. Target ordering when combined: terminal content → effects → UI → existing screen filters. Verify this against the actual wgpu compositing path.

### Proposed configuration

This is a new configuration proposal, not configuration supported by the current binary.

```toml
[effects]
trail-cursor = false

[effects.custom]
enabled = true
hot-reload = true
max-fps = 60

[[effects.custom.shaders]]
path = "effects/cursor-trail.wgsl"
api-version = 1

[effects.custom.shaders.params]
color = [0.75, 0.4, 1.0, 1.0]
opacity = 0.65
width = 1.0
duration = 0.3 # Seconds; interpreted by this external shader

[[effects.custom.shaders]]
path = "effects/typing-sparks.wgsl"
api-version = 1

[effects.custom.shaders.params]
color = [1.0, 0.8, 0.3, 1.0]
intensity = 0.5
speed = 120.0
duration = 0.24

# This shader transforms the input image on every rendered frame.
[[effects.custom.shaders]]
path = "effects/animated-distortion.wgsl"
api-version = 1
```

- Resolve relative paths against the directory containing the loaded `config.toml`; allow absolute paths. On Windows, `%LOCALAPPDATA%\rio\effects\` is a typical location. Preserve case, spaces, and non-ASCII characters.
- Do not add `trigger`, `animation`, or host-owned effect duration settings. While custom shaders are enabled and the panel is visible, run the entire chain continuously, even without terminal damage or focus. Focus, event timestamps, and histories are shader inputs; external WGSL decides when an effect is visible and how long it lasts.
- `max-fps` is a single chain-wide cap, coordinated with presentation timing. Validate its range and cap shader count. Do not maintain per-shader timers or redraw policies.
- Suspend the loop for hidden/minimized/occluded panels and inactive tabs; resume it when visible. Visible unfocused panels continue running by default. Disabled/empty chains retain ordinary event-driven rendering.
- Precedence with existing renderer settings: when `disable-unfocused-render` or `disable-occluded-render` stops a window's rendering, the effect loop stops with it. With `strategy = "game"`, the existing unthrottled loop drives effects and `max-fps` does not apply; `max-fps` only paces the `events` strategy. Document this.
- Windows' `rio-window` emits no `Occluded` events (only macOS and X11 do), so on Windows suspension initially covers minimized windows (detected separately) and inactive tabs, not occlusion.
- `params` is a **key-value table with names defined by each external shader**, never a positional list. Rio must not hard-code names such as `color`, `width`, or `speed`.
- Load only WGSL and configuration values. Do not introduce DLLs, Rust plugins, external command execution, or file includes.

### Named parameter contract

Use a reserved uniform binding for a shader-defined `EffectParams` struct. Reflect its members and layout through Naga, match configuration keys by name, and pack values using the reflected GPU offsets/alignment. Configuration order must not affect packing.

For example, the trail shader declares these fields and reads `params.opacity` in its own visual calculations:

```wgsl
struct EffectParams {
    color: vec4<f32>,
    opacity: f32,
    width: f32,
    duration: f32,
}

@group(1) @binding(0)
var<uniform> params: EffectParams;
```

- Initially support `f32` and `vec2/3/4<f32>` members. TOML numbers map to scalars, and numeric arrays of matching length map to vectors. Reject non-finite/out-of-range values during conversion to `f32`.
- Require values for all declared members; reject missing keys, unknown keys, type mismatches, and unsupported layouts with the shader path and parameter name. A shader with no parameters may omit the binding and table. Default values and richer types can be added through an explicit future schema.
- Bound parameter count and uniform byte size. Account for vector alignment and trailing padding; never use table iteration order as a GPU layout.
- Changing values updates the uniform buffer without recompiling WGSL. Changing member names/types reloads and validates the shader and configuration together. Keep the previous valid shader/parameter pair on failure.
- Do not interpolate values into shader source. Each external example documents its parameter names, units, and useful ranges. Parameter packing is generic host infrastructure; parameter semantics remain external.

### Host-provided data

Define CPU representations and a versioned GPU ABI for `EffectFrame` / `EffectEvent`; pass values from the frontend to Sugarloaf.

- Current pass input texture, original terminal texture, sampler, and input/output dimensions. These are GPU resources, not CPU screenshots.
- Monotonic relative time, frame delta, actual frame counter, panel resolution, DPI scale, and cell dimensions. Keep time advancing during suspension; document resumed delta behavior so shaders can handle gaps.
- Current/previous cursor rectangles, shapes, colors, visibility, and change timestamps. Distinguish position changes from appearance changes, so shaders can react to either.
- Panel focus state and last focus-change time for focus/blur transitions and animation restarts.
- The 256-color palette and semantic foreground/background, cursor, and selection colors (Rio has no cursor-text color). Normalize them to the shader color space, matching the sampled image: the cell shaders' gamut mapping into DisplayP3 (on both backends), then linear premultiplied. Update theme/palette changes without rebuilding shaders.
- Timestamped cursor history and typing events in bounded ring buffers, initially 64 entries each. Overwrite the oldest on overflow; do not expire events using a host-owned visual duration. External shaders filter entries by age. Preserve multiple keystrokes between frames.
- Each typing event carries its occurrence position/time, sequence number, and random seed, but no text. Use the displayed cursor position at input time without waiting for a PTY response.
- Use panel-local physical pixels with a top-left origin and `x, y, width, height` rectangles. Normalize WGSL/MSL fragment coordinates, margins, splits, scrolling, and wide-cell geometry in the host.
- Own state per route/panel. Reset discontinuous history on tab/panel switches, resize, DPI changes, scrollback, and screen switches so trails cannot cross unrelated views.
- Continue supplying real input events and cursor visibility independently. A hidden cursor must not disable full-screen typing effects; external shaders decide whether visibility affects their visuals. Distinguish blink from movement/input. Event age does not control host scheduling.

### Input and redraw scheduling

- Collect ordinary input after routing, from key presses actually sent to the terminal. Ignore releases, standalone modifiers, keys consumed by modals/search, and PTY output alone.
- Initially, `typing` covers text input, Enter, and Backspace/Delete. Cursor movement keys produce cursor events. Include repeat events while retaining bounded history.
- Do not trigger on IME preedit; emit once per committed string delivered to the terminal. IME currently shares `paste` handling, so preserve explicit input provenance to distinguish commits from ordinary paste. Ordinary paste does not trigger typing initially.
- Detect cursor movement from terminal display state, including movement caused by TUI applications or program output. Keep typing and cursor events distinct.
- Integrate one continuous chain loop into `renderer.needs_redraw()`, `screen`'s `should_present`, and continuation scheduling. Start when a nonempty enabled chain becomes visible; stop when it is hidden, disabled, or empty. Do not infer effect completion from input activity. Removing/disabling the chain requests a clearing frame.
- Present effects even without terminal damage. Preserve row-level updates rather than rebuilding terminal content every animation frame.
- Suspend continuation redraws for hidden/minimized/occluded panels. Keep the monotonic clock advancing so external shaders can evaluate old events correctly after resuming. Time-based effects must work with a stationary or hidden cursor.
- A uniform focus/time branch in WGSL can avoid expensive visual calculations, but it cannot cancel host wakeups, command submission, full-panel rasterization, input texture reads/output writes, or presentation. Returning unchanged input still executes an image pass. These baseline costs are accepted; measurements are for implementation efficiency, not for introducing scheduling controls or inferring a shader's no-op condition.

### Hot reload and failure behavior

- Watch parent directories of configured files, filtering events to relevant paths. Support atomic rename saves, deletion/recreation, and watch-set changes after configuration edits.
- Debounce saves and use generation numbers so stale compile results cannot replace newer ones. Move file reads, validation, and translation off the render loop; obey backend constraints for GPU pipeline creation/exchange.
- Exchange validated pipelines at frame boundaries. Preserve the last successful shader/parameter pair on partial saves, syntax errors, or ABI violations. Disable only the failing effect if there is no valid previous version.
- Report file, line where available, and cause through logs and existing notifications. Avoid repeating identical errors every frame. Explicit removal/disabling must release the previous pipeline.
- Accept configuration on unsupported/CPU backends, notify once, and continue ordinary terminal rendering. Recreate resources correctly after device loss or surface recreation.
- Validation cannot guarantee execution time for arbitrary GPU code. Limit the ABI and keep example loops/history bounded; do not promise complete isolation from GPU hangs.

### Capability baseline and Ghostty relationship

Source/configuration compatibility is not a goal. Expressive capability is: use WGSL and TOML while retaining image processing, pass composition, animation, and contextual inputs. The official [configuration reference](https://ghostty.org/docs/config/reference#custom-shader) and [shader declarations](https://github.com/ghostty-org/ghostty/blob/main/src/renderer/shaders/shadertoy_prefix.glsl) inform the following initial requirements:

| Capability to preserve | Rio design and what it enables |
| --- | --- |
| Read terminal pixels | Sample the input image at arbitrary coordinates: distortion, blur, channel displacement, recoloring, CRT effects, and overlays that react to text. |
| Chain shaders | Every pass reads the previous pass's current-frame output. Build multi-pass glow/blur without host-specific effect implementations. |
| Animate without terminal changes | Continuously execute visible chains; focus and event age are shader-side conditions. Stationary cursor effects need no fake typing. |
| Time, frame, dimensions | Explicit shared frame data for time-based formulas, animation progression, and resolution-independent rendering. |
| Cursor state | Current/previous geometry, appearance, visibility, and change times. |
| Focus state | Focus value and transition time for dimming and focus-reactive animation. |
| Theme context | Palette and semantic colors for effects that follow the terminal theme. |

Ghostty has a chain/surface-level `custom-shader-animation` option: `true` animates focused surfaces (the default), `false` renders on terminal updates, and `always` includes unfocused surfaces. It does not document per-shader triggers or event durations. Rio deliberately uses one continuous visible-chain policy, comparable to `always`, without reproducing that configuration selector. Its recurring execution cost is an accepted tradeoff.

Ghostty supplies GLSL `mainImage`; Rio uses WGSL `effect_main`. Named parameters and explicit typing/history are Rio extensions. Define a native ABI rather than copying uniform names or relying on documentation's declared types; for example, Ghostty's source declares cursor styles/visibility as integers.

Do not mistake current-frame shader chaining for previous-frame feedback. Ghostty's documented shader API does not expose a previous-frame image or additional active texture channels. Mouse/date/audio fields listed as unsupported or inapplicable are not working features to match. Persistent image feedback, user textures, and general compute/storage execution are separate future scope, not prerequisites inferred from Shadertoy terminology.

Porting a shader still requires WGSL translation and input/color/coordinate mapping, but no forced reduction from image processing to additive decoration. Validate licensing and attribution before distributing ports. A source-compatible GLSL adapter is optional future work, independent of this capability baseline.

### Milestones and completion criteria

1. [x] **Validate the shared ABI on both GPU paths**
   - Split per-panel terminal content from window UI on both backends and implement the capture boundary above.
   - Run external WGSL image passes on Windows/wgpu and macOS/Metal: identity, spatial distortion, and a two-pass chain. Verify each pass reads the correct input and the original texture remains unchanged.
   - Verify WGSL-to-MSL translation, event/uniform layouts, named parameter reflection/packing, alpha, sRGB/DisplayP3, transparent windows, neighborhood sampling, and panel boundaries. Confirm focus/theme uniforms update correctly.
   - Freeze the ABI and insertion points only after both backends work.
   - Status: ABI v1 is frozen as group 0 = `rio_frame` (incl. `focused`/`focus_time`), `rio_input`, `rio_original`, `rio_sampler`, `rio_theme`. Later milestones only append (new `rio_*` bindings, `@group(1)` params, new trailing fields), so v1 shaders keep working. Event layouts and parameter packing are defined in M2–M4 under that rule. Metal runs of focus/theme uniforms are pending a Mac check.
2. [x] **Configuration, loader, parameters, and recovery**
   - Extend `config/effects.rs`, including platform-specific overrides.
   - Add shared models, validation, parameter packing, and backend pipelines under a module such as `sugarloaf/src/effects/`.
   - Implement startup/configuration loading, watches, and preservation of the last valid state.
   - Complete when saving shader code or named values updates the same running process, invalid parameters produce useful errors, and fixing errors restores operation.
   - Status: a loader thread reads, validates, and packs shaders, watching their parent directories (debounced); windows install the latest chain on a `CustomEffectsChanged` event, reusing pipelines whose WGSL is unchanged. Errors go to the log and a non-modal warning toast, cleared on recovery. File watching does not see changes under `\\wsl.localhost` paths from Windows. Device-loss recovery is not handled yet.
3. [x] **Continuous execution and cursor history**
   - Add per-panel frontend state and supply actual cursor geometry/history.
   - Create external `cursor-trail.wgsl` with configurable width, fading, and color.
   - Add an external stationary/time-based example and shader-side focus gating. Complete when visible chains animate without terminal damage, unfocused visible panels continue executing, hidden panels suspend, all passes remain applied, and tab/split changes never mix histories.
   - Status: `rio_cursor` (binding 5) carries the current/previous cursor, move/change times, and a 64-entry history, tracked per panel and reset on resize and scrollback. Effect frames are scheduled through `RenderRoute` at `max-fps` (so unfocused/occluded gates and vsync throttling apply) and stop while minimized; `game` renders every frame itself. Examples: `cursor-trail.wgsl`, `focus-pulse.wgsl`. Not yet: alternate-screen switches do not reset history, and wide-cell cursors report one cell.
4. [x] **Typing events and external examples**
   - Collect ordinary input and IME commits, distinguishing paste and PTY output.
   - Create external `typing-sparks.wgsl` and `typing-slash.wgsl`, and tune their composition with the trail example.
   - Complete when rapid input, repeat, and Japanese IME produce appropriate events, external shaders end their visuals according to their own duration parameters, and text/cursor readability is preserved.
   - Status: `rio_events` (binding 6) holds the last 64 inputs per panel, newest first, each with the displayed cursor cell, time, sequence number, seed, and kind (text, Enter, Backspace/Delete, IME commit). Key presses are recorded where bytes go to the PTY (not in search, hints, Vi mode, or bindings); IME commits once each; paste and PTY output never. Examples: `typing-sparks.wgsl`, `typing-slash.wgsl`. Japanese IME has not been tried interactively yet.
5. [ ] **Regression checks and user documentation**
   - Document configuration, named parameter contracts, WGSL ABI, installation, reload, diagnostics, and unsupported backends.
   - Demonstrate: launch the installed binary → add a new external shader → configure it → save and display it → edit its visual formula, without rebuilding or restarting Rio.
   - Demonstrate image distortion/recoloring, a two-pass blur/glow, focus/theme reactions, and cursor-independent animation entirely in external WGSL. Implement an additional effect unknown to Rio using the same external API. Removing all trail/spark/slash examples must leave the application build and generic infrastructure tests functional, with no effect-specific Rust or embedded shaders.
   - Verify the same shader files work on Windows and macOS.

### Validation

- Unit tests: defaults/ranges, path resolution, ABI checks, parameter names/types/layout/padding and ordering independence, history capacity/overwrite, visible/hidden loop transitions, input provenance, reload failures, and generation ordering.
- GPU checks: compare small deterministic renders for identity preservation, transformed sampling, chain order, original-input stability, coordinates, color conversion, alpha, and clipping; verify invalid shaders do not terminate the terminal. Compile examples at runtime on both backends.
- Manual checks: shells/TUIs, newlines/wrapping/wide cells, cursor shapes/hide/blink, splits/windows, scrolling, DPI/resize, transparency, existing trail/filter coexistence, minimize/restore, rapid saves, rename saves, and deletion/recreation.
- Performance: compare disabled, identity-only, a focus-gated identity branch, trail-only, and trail-plus-typing on the same hardware. Record input-to-present latency, CPU/GPU time, and idle redraws. Initially target at most 1ms additional GPU time at 60fps; tune history size, drawing area, and formulas if necessary. Continuous idle redraws are intentional for enabled visible chains and must respect the FPS cap. Verify no continuing shader redraws when disabled, empty, or hidden. Measure idle/unfocused CPU, GPU, bandwidth, and power costs; shader-side early returns do not imply zero cost. Measure intermediate texture memory and cached-terminal reuse as well.
- Windows/WSL: `mise run test:windows`, `mise run check:windows`, and `mise run dev:windows`. macOS: corresponding `test:macos`, `check:macos`, and `dev:macos` tasks. Fix lints in changed files and distinguish inherited upstream failures. Install only when explicitly requested.

### Follow-up work

- Native Linux/Vulkan execution via runtime WGSL-to-SPIR-V translation.
- Persistent previous-frame feedback, user-provided textures, reduced-resolution pass targets, and compute/storage-based effects.
- A separately specified Ghostty/Shadertoy GLSL compatibility layer.
- Preset distribution/selection UI. Stationary effects are already supported by the initial continuous animation API.
- Parameter defaults, richer types, and external parameter schemas if needed.

### References

- [Article: cursor trail](https://github.com/hondazn/dotfiles/blob/master/config/ghostty/shaders/cursor_blaze.glsl)
- [Article: sparks](https://github.com/hondazn/dotfiles/blob/master/config/ghostty/shaders/sparks.glsl)
- [Article: slash](https://github.com/hondazn/dotfiles/blob/master/config/ghostty/shaders/slash.glsl)
- [Ghostty: custom-shader](https://ghostty.org/docs/config/reference#custom-shader)
- [Ghostty: custom-shader-animation](https://ghostty.org/docs/config/reference#custom-shader-animation)
- [Ghostty: shader ABI declarations](https://github.com/ghostty-org/ghostty/blob/main/src/renderer/shaders/shadertoy_prefix.glsl)
