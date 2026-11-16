//! External WGSL post-processing chains for terminal panels.
//!
//! Each user file implements `fn effect_main(pos: vec2<f32>) -> vec4<f32>`.
//! Rio appends [`PRELUDE`] after the user source (so user line numbers stay
//! intact in diagnostics), which declares the host bindings and entry points.

#[cfg(target_os = "macos")]
pub(crate) mod metal;
#[cfg(any(target_os = "macos", test))]
mod msl;
#[cfg(feature = "wgpu")]
pub(crate) mod webgpu;

use naga::valid::{Capabilities, ValidationFlags, Validator};

/// Version of the WGSL ABI implemented by this build.
pub const API_VERSION: u32 = 1;

/// Upper bound on passes per chain.
pub const MAX_PASSES: usize = 16;

/// Host declarations appended to every user shader.
pub const PRELUDE: &str = include_str!("effects/prelude.wgsl");

pub(crate) const VERTEX_ENTRY: &str = "rio_effect_vs";
pub(crate) const FRAGMENT_ENTRY: &str = "rio_effect_fs";

/// One external shader file, already read from disk by the embedder.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectSource {
    /// Display name used in diagnostics (normally the file path).
    pub name: String,
    pub source: String,
}

/// A user shader that passed validation, with the prelude attached.
#[derive(Debug, Clone)]
pub struct ValidatedEffect {
    pub name: String,
    pub wgsl: String,
    pub module: naga::Module,
    pub info: naga::valid::ModuleInfo,
}

/// Per-panel data exposed to shaders as `rio_frame`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EffectFrameUniform {
    /// Panel size in physical pixels.
    pub resolution: [f32; 2],
    /// Seconds since the chain was installed.
    pub time: f32,
    /// Seconds since the previous rendered frame.
    pub time_delta: f32,
    /// Rendered frame counter.
    pub frame: u32,
    /// DPI scale factor.
    pub scale: f32,
    /// Terminal cell size in physical pixels.
    pub cell_size: [f32; 2],
}

/// Where a panel sits in the window, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectPanel {
    /// Image-overlay key (`rich_text_id`) of the panel.
    pub id: usize,
    /// `[x, y, width, height]`, integer-aligned.
    pub rect: [f32; 4],
    pub cell_size: [f32; 2],
}

impl EffectPanel {
    /// The rect clipped to the window, or `None` if nothing is visible.
    pub(crate) fn clipped_rect(&self, window: (f32, f32)) -> Option<[u32; 4]> {
        let x0 = self.rect[0].max(0.0);
        let y0 = self.rect[1].max(0.0);
        let x1 = (self.rect[0] + self.rect[2]).min(window.0);
        let y1 = (self.rect[1] + self.rect[3]).min(window.1);
        if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
            return None;
        }
        let (x0, y0) = (x0.round() as u32, y0.round() as u32);
        let (x1, y1) = (x1.round() as u32, y1.round() as u32);
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }
}

/// How the captured surface stores alpha, as seen by the host passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum SurfaceAlpha {
    Premultiplied = 0,
    #[cfg_attr(not(feature = "wgpu"), allow(dead_code))]
    Straight = 1,
    Opaque = 2,
}

/// Uniform for the decode/encode host passes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct HostPassUniform {
    pub origin: [f32; 2],
    pub alpha_mode: u32,
    pub _pad: u32,
}

/// Monotonic clock shared by every panel of a chain.
#[derive(Debug)]
pub(crate) struct EffectClock {
    start: std::time::Instant,
    last: Option<std::time::Instant>,
    frame: u32,
}

impl Default for EffectClock {
    fn default() -> Self {
        Self {
            start: std::time::Instant::now(),
            last: None,
            frame: 0,
        }
    }
}

impl EffectClock {
    /// Advance one rendered frame: `(time, time_delta, frame)`.
    pub fn tick(&mut self) -> (f32, f32, u32) {
        let now = std::time::Instant::now();
        let delta = self.last.map_or(0.0, |l| (now - l).as_secs_f32());
        self.last = Some(now);
        let frame = self.frame;
        self.frame = self.frame.wrapping_add(1);
        ((now - self.start).as_secs_f32(), delta, frame)
    }
}

/// Column-major orthographic projection mapping the window-space rect
/// `[x, y, w, h]` onto a target of size `w × h`.
pub(crate) fn panel_projection(rect: [f32; 4]) -> [f32; 16] {
    let mut m = crate::components::core::orthographic_projection(rect[2], rect[3]);
    m[12] = -1.0 - 2.0 * rect[0] / rect[2];
    m[13] = 1.0 + 2.0 * rect[1] / rect[3];
    m
}

/// Parse and validate a user shader against the host ABI.
pub fn validate(effect: &EffectSource) -> Result<ValidatedEffect, String> {
    let name = effect.name.as_str();
    if !effect.source.contains("effect_main") {
        return Err(format!(
            "{name}: missing `fn effect_main(pos: vec2<f32>) -> vec4<f32>`"
        ));
    }
    if effect.source.contains("rio_effect_") {
        return Err(format!(
            "{name}: identifiers starting with `rio_` are reserved"
        ));
    }
    let wgsl = format!("{}\n{}", effect.source, PRELUDE);
    let module = naga::front::wgsl::parse_str(&wgsl)
        .map_err(|e| e.emit_to_string_with_path(&wgsl, name))?;

    check_abi(&module).map_err(|e| format!("{name}: {e}"))?;

    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| e.emit_to_string_with_path(&wgsl, name))?;

    Ok(ValidatedEffect {
        name: name.to_string(),
        wgsl,
        module,
        info,
    })
}

fn check_abi(module: &naga::Module) -> Result<(), String> {
    let effect_main = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some("effect_main"))
        .map(|(_, f)| f)
        .ok_or("missing `fn effect_main(pos: vec2<f32>) -> vec4<f32>`")?;
    let is_vec = |ty: naga::Handle<naga::Type>, size: naga::VectorSize| {
        matches!(
            module.types[ty].inner,
            naga::TypeInner::Vector { size: s, scalar } if s == size && scalar == naga::Scalar::F32
        )
    };
    let args_ok = effect_main.arguments.len() == 1
        && is_vec(effect_main.arguments[0].ty, naga::VectorSize::Bi);
    let result_ok = effect_main
        .result
        .as_ref()
        .is_some_and(|r| is_vec(r.ty, naga::VectorSize::Quad));
    if !args_ok || !result_ok {
        return Err("`effect_main` must have the signature `fn effect_main(pos: vec2<f32>) -> vec4<f32>`".into());
    }

    for ep in &module.entry_points {
        if ep.name != VERTEX_ENTRY && ep.name != FRAGMENT_ENTRY {
            return Err(format!(
                "entry point `{}` is not allowed; implement `effect_main` instead",
                ep.name
            ));
        }
    }

    for (_, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else {
            continue;
        };
        let host = var.name.as_deref().is_some_and(|n| n.starts_with("rio_"));
        if !host || binding.group != 0 {
            return Err(format!(
                "resource binding `{}` (@group({}) @binding({})) is not supported",
                var.name.as_deref().unwrap_or("?"),
                binding.group,
                binding.binding
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(source: &str) -> EffectSource {
        EffectSource {
            name: "test.wgsl".into(),
            source: source.into(),
        }
    }

    const IDENTITY: &str = r#"
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    return textureLoad(rio_input, vec2<i32>(pos), 0);
}
"#;

    #[test]
    fn identity_validates() {
        validate(&src(IDENTITY)).unwrap();
    }

    #[test]
    fn sampling_with_frame_data_validates() {
        validate(&src(r#"
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    let uv = pos / rio_frame.resolution;
    let o = textureSampleLevel(rio_original, rio_sampler, uv, 0.0);
    let s = sin(rio_frame.time) * 0.01;
    return textureSampleLevel(rio_input, rio_sampler, uv + vec2<f32>(s, 0.0), 0.0) * o.a;
}
"#))
        .unwrap();
    }

    #[test]
    fn missing_effect_main_is_rejected() {
        let err = validate(&src("fn main() {}")).unwrap_err();
        assert!(err.contains("effect_main"), "{err}");
    }

    #[test]
    fn wrong_signature_is_rejected() {
        let err = validate(&src(
            "fn effect_main(pos: vec4<f32>) -> vec4<f32> { return pos; }",
        ))
        .unwrap_err();
        assert!(err.contains("signature"), "{err}");
    }

    #[test]
    fn syntax_error_reports_user_line() {
        let err = validate(&src(
            "\n\nfn effect_main(pos: vec2<f32>) -> vec4<f32> {\n  return oops;\n}\n",
        ))
        .unwrap_err();
        assert!(err.contains("test.wgsl:4"), "{err}");
    }

    #[test]
    fn extra_entry_points_are_rejected() {
        let err = validate(&src(&format!(
            "{IDENTITY}\n@fragment fn other() -> @location(0) vec4<f32> {{ return vec4<f32>(0.0); }}"
        )))
        .unwrap_err();
        assert!(err.contains("entry point"), "{err}");
    }

    #[test]
    fn user_bindings_are_rejected() {
        let err = validate(&src(&format!(
            "{IDENTITY}\n@group(2) @binding(0) var<uniform> x: vec4<f32>;"
        )))
        .unwrap_err();
        assert!(err.contains("binding"), "{err}");
    }

    #[test]
    fn panel_projection_maps_corners() {
        let m = panel_projection([100.0, 50.0, 200.0, 100.0]);
        let ndc = |x: f32, y: f32| (m[0] * x + m[12], m[5] * y + m[13]);
        assert_eq!(ndc(100.0, 50.0), (-1.0, 1.0));
        assert_eq!(ndc(300.0, 150.0), (1.0, -1.0));
    }

    #[test]
    fn clipped_rect_rounds_and_clips() {
        let p = EffectPanel {
            id: 0,
            rect: [10.0, 20.0, 1000.0, 30.0],
            cell_size: [1.0, 1.0],
        };
        assert_eq!(p.clipped_rect((500.0, 500.0)), Some([10, 20, 490, 30]));
        assert_eq!(p.clipped_rect((5.0, 5.0)), None);
    }
}
