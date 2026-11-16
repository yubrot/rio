# PLAN

## External WGSL post-processing for terminal visuals

### Remaining implementation work

- Handle device-loss recovery.
- Reset cursor history on alternate-screen switches.
- Report the full width of wide-cell cursors.

### Remaining regression checks

- Verify that removing all trail/spark/slash examples leaves the application build and generic infrastructure tests functional, with no effect-specific Rust or embedded shaders.

### Performance measurements

- Performance: compare disabled, identity-only, a focus-gated identity branch, trail-only, and trail-plus-typing on the same hardware. Record input-to-present latency, CPU/GPU time, and idle redraws. Initially target at most 1ms additional GPU time at 60fps; tune history size, drawing area, and formulas if necessary. Continuous idle redraws are intentional for enabled visible chains and must respect the FPS cap. Verify no continuing shader redraws when disabled, empty, or hidden. Measure idle/unfocused CPU, GPU, bandwidth, and power costs; shader-side early returns do not imply zero cost. Measure intermediate texture memory and cached-terminal reuse as well.

### Follow-up work

- Native Linux/Vulkan execution via runtime WGSL-to-SPIR-V translation.
- Persistent previous-frame feedback, user-provided textures, reduced-resolution pass targets, and compute/storage-based effects.
- A separately specified Ghostty/Shadertoy GLSL compatibility layer.
- Preset distribution/selection UI. Stationary effects are already supported by the initial continuous animation API.
- Parameter defaults, richer types, and external parameter schemas if needed.
