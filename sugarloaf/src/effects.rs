//! External WGSL post-processing chains for terminal panels.
//!
//! Each user file implements `fn effect_main(pos: vec2<f32>) -> vec4<f32>`.
//! Rio appends [`PRELUDE`] after the user source (so user line numbers stay
//! intact in diagnostics), which declares the host bindings and entry points.
#![cfg_attr(not(any(feature = "wgpu", target_os = "macos")), allow(dead_code))]

#[cfg(target_os = "macos")]
pub(crate) mod metal;
#[cfg(any(target_os = "macos", test))]
mod msl;
#[cfg(feature = "wgpu")]
pub(crate) mod webgpu;

use crate::sugarloaf::Colorspace;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use rustc_hash::FxHashMap;

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
    /// The shader's `@group(1) @binding(0)` parameter struct, if any.
    pub params: Option<ParamsLayout>,
}

/// Upper bounds on a shader's parameter struct.
pub const MAX_PARAMS: usize = 32;
pub const MAX_PARAMS_SIZE: u32 = 1024;

/// Reflected layout of a shader-defined parameter struct.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamsLayout {
    /// Struct size in bytes.
    pub size: u32,
    pub members: Vec<ParamMember>,
}

/// One `f32` / `vecN<f32>` member of a parameter struct.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamMember {
    pub name: String,
    pub offset: u32,
    /// 1 for `f32`, N for `vecN<f32>`.
    pub components: u32,
}

/// A configured parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Number(f64),
    Vector(Vec<f64>),
}

impl ValidatedEffect {
    /// Pack `values` by member name into the reflected uniform layout.
    /// Every member must be given; unknown names are rejected.
    pub fn pack_params(
        &self,
        values: &[(String, ParamValue)],
    ) -> Result<Vec<u8>, String> {
        let Some(layout) = &self.params else {
            return match values.first() {
                Some((key, _)) => Err(format!(
                    "unknown parameter `{key}`: the shader declares no parameters"
                )),
                None => Ok(Vec::new()),
            };
        };
        for (key, _) in values {
            if !layout.members.iter().any(|m| &m.name == key) {
                return Err(format!("unknown parameter `{key}`"));
            }
        }
        let mut bytes = vec![0u8; layout.size.next_multiple_of(16) as usize];
        for member in &layout.members {
            let name = &member.name;
            let value = values
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, v)| v)
                .ok_or_else(|| format!("missing parameter `{name}`"))?;
            let numbers: &[f64] = match value {
                ParamValue::Number(n) if member.components == 1 => {
                    std::slice::from_ref(n)
                }
                ParamValue::Vector(v)
                    if member.components > 1 && v.len() == member.components as usize =>
                {
                    v
                }
                _ if member.components == 1 => {
                    return Err(format!("parameter `{name}` must be a number"))
                }
                _ => {
                    return Err(format!(
                        "parameter `{name}` must be an array of {} numbers",
                        member.components
                    ))
                }
            };
            for (i, n) in numbers.iter().enumerate() {
                let v = *n as f32;
                if !v.is_finite() {
                    return Err(format!("parameter `{name}` is out of range: {n}"));
                }
                let at = member.offset as usize + i * 4;
                bytes[at..at + 4].copy_from_slice(&v.to_ne_bytes());
            }
        }
        Ok(bytes)
    }
}

/// A validated shader with its packed parameters, ready to install.
#[derive(Debug, Clone)]
pub struct EffectPass {
    pub effect: std::sync::Arc<ValidatedEffect>,
    pub params: Vec<u8>,
}

/// Per-panel data exposed to shaders as `rio_frame`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EffectFrameUniform {
    /// Panel size in physical pixels.
    pub resolution: [f32; 2],
    /// Seconds since effects started in this window; continues across reloads.
    pub time: f32,
    /// Seconds since the previous rendered frame.
    pub time_delta: f32,
    /// Rendered frame counter.
    pub frame: u32,
    /// DPI scale factor.
    pub scale: f32,
    /// Terminal cell size in physical pixels.
    pub cell_size: [f32; 2],
    /// 1 while the panel has keyboard focus.
    pub focused: u32,
    /// `time` at which `focused` last changed.
    pub focus_time: f32,
    pub _pad: [u32; 2],
}

/// Number of palette entries exposed as `rio_theme.palette`.
pub const PALETTE_SIZE: usize = 256;

/// Theme colors of a panel as configured (straight alpha, in the
/// configured input colorspace); converted for shaders by the host.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectTheme {
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub cursor: [f32; 4],
    pub selection_foreground: [f32; 4],
    pub selection_background: [f32; 4],
    pub palette: [[f32; 4]; PALETTE_SIZE],
}

impl Default for EffectTheme {
    fn default() -> Self {
        Self {
            foreground: [0.0; 4],
            background: [0.0; 4],
            cursor: [0.0; 4],
            selection_foreground: [0.0; 4],
            selection_background: [0.0; 4],
            palette: [[0.0; 4]; PALETTE_SIZE],
        }
    }
}

impl EffectTheme {
    /// Bytes of `rio_theme`: every color in the shader color space.
    fn to_uniform(&self, colorspace: Colorspace) -> Vec<u8> {
        let named = [
            self.foreground,
            self.background,
            self.cursor,
            self.selection_foreground,
            self.selection_background,
        ];
        let colors: Vec<[f32; 4]> = named
            .iter()
            .chain(self.palette.iter())
            .map(|c| shader_color(*c, colorspace))
            .collect();
        bytemuck::cast_slice(&colors).to_vec()
    }
}

/// Size of `rio_theme` in bytes.
#[cfg(any(feature = "wgpu", test))]
const THEME_SIZE: usize = (5 + PALETTE_SIZE) * 16;

/// A configured color as the shaders see the sampled image: the cell
/// shaders' gamut mapping into DisplayP3, then linear and premultiplied.
fn shader_color(c: [f32; 4], colorspace: Colorspace) -> [f32; 4] {
    const SRGB_TO_P3: [[f32; 3]; 3] = [
        [0.822_461_97, 0.177_538_03, 0.0],
        [0.033_194_2, 0.966_805_8, 0.0],
        [0.017_082_63, 0.072_397_44, 0.910_519_93],
    ];
    const REC2020_TO_P3: [[f32; 3]; 3] = [
        [1.343_578_3, -0.282_179_67, -0.061_398_58],
        [-0.065_297_45, 1.087_822_3, -0.022_524_81],
        [0.002_821_79, -0.025_988_07, 1.023_166_3],
    ];
    let to_linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let lin = [to_linear(c[0]), to_linear(c[1]), to_linear(c[2])];
    let rgb = match colorspace {
        Colorspace::Srgb => {
            SRGB_TO_P3.map(|r| r[0] * lin[0] + r[1] * lin[1] + r[2] * lin[2])
        }
        Colorspace::Rec2020 => {
            REC2020_TO_P3.map(|r| r[0] * lin[0] + r[1] * lin[1] + r[2] * lin[2])
        }
        Colorspace::DisplayP3 => lin,
    };
    let a = c[3].clamp(0.0, 1.0);
    [rgb[0] * a, rgb[1] * a, rgb[2] * a, a]
}

/// Bindings of the per-panel uniform blocks, in [`PanelUniforms::blocks`]
/// order. On Metal block `i` is buffer slot `i`.
pub(crate) const UNIFORM_BINDINGS: [u32; 3] = [0, 4, 5];
#[cfg(any(feature = "wgpu", test))]
pub(crate) const UNIFORM_SIZES: [usize; 3] = [
    std::mem::size_of::<EffectFrameUniform>(),
    THEME_SIZE,
    std::mem::size_of::<CursorUniform>(),
];

/// Metal buffer slot of the `@group(1)` parameter struct.
#[cfg(any(target_os = "macos", test))]
pub(crate) const PARAMS_SLOT: usize = UNIFORM_BINDINGS.len();

/// Per-panel uniform data for one frame.
pub(crate) struct PanelUniforms {
    pub blocks: [Vec<u8>; 3],
}

/// Entries kept in `rio_cursor.history`.
pub const CURSOR_HISTORY: usize = 64;

/// Displayed cursor of a panel, in panel-local physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EffectCursor {
    /// `[x, y, width, height]` of the cursor cell.
    pub rect: [f32; 4],
    /// As configured (straight alpha, configured colorspace).
    pub color: [f32; 4],
    /// 0 = block, 1 = underline, 2 = beam, 3 = hidden.
    pub shape: u32,
    pub visible: bool,
    pub blink_on: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct CursorState {
    rect: [f32; 4],
    color: [f32; 4],
    shape: u32,
    visible: u32,
    blink_on: u32,
    time: f32,
}

impl CursorState {
    /// Whether `other` differs in anything but blink phase and time.
    fn appearance_differs(&self, other: &CursorState) -> bool {
        (self.color, self.shape, self.visible)
            != (other.color, other.shape, other.visible)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct CursorSample {
    rect: [f32; 4],
    time: f32,
    _pad: [f32; 3],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CursorUniform {
    current: CursorState,
    previous: CursorState,
    move_time: f32,
    change_time: f32,
    history_len: u32,
    _pad: u32,
    history: [CursorSample; CURSOR_HISTORY],
}

/// Per-panel cursor tracking behind `rio_cursor`.
#[derive(Debug)]
struct CursorTrack {
    /// Changes of this (scroll position, panel size) start a new history.
    view: (u64, [u32; 2]),
    current: CursorState,
    previous: CursorState,
    move_time: f32,
    change_time: f32,
    /// Newest first.
    history: std::collections::VecDeque<CursorSample>,
}

impl CursorTrack {
    fn new(view: (u64, [u32; 2]), state: CursorState, time: f32) -> Self {
        let mut history = std::collections::VecDeque::with_capacity(CURSOR_HISTORY);
        history.push_front(CursorSample {
            rect: state.rect,
            time,
            _pad: [0.0; 3],
        });
        Self {
            view,
            current: state,
            previous: state,
            move_time: time,
            change_time: time,
            history,
        }
    }

    fn update(&mut self, view: (u64, [u32; 2]), mut state: CursorState, time: f32) {
        state.time = time;
        if view != self.view {
            *self = Self::new(view, state, time);
            return;
        }
        let moved = state.rect != self.current.rect;
        let changed = state.appearance_differs(&self.current);
        if moved || changed {
            self.previous = self.current;
            self.current = state;
        } else {
            self.current.blink_on = state.blink_on;
        }
        if moved {
            self.move_time = time;
            if self.history.len() == CURSOR_HISTORY {
                self.history.pop_back();
            }
            self.history.push_front(CursorSample {
                rect: state.rect,
                time,
                _pad: [0.0; 3],
            });
        }
        if changed {
            self.change_time = time;
        }
    }

    fn to_uniform(&self) -> CursorUniform {
        let mut history = [CursorSample::default(); CURSOR_HISTORY];
        for (slot, sample) in history.iter_mut().zip(&self.history) {
            *slot = *sample;
        }
        CursorUniform {
            current: self.current,
            previous: self.previous,
            move_time: self.move_time,
            change_time: self.change_time,
            history_len: self.history.len() as u32,
            _pad: 0,
            history,
        }
    }
}

/// Where a panel sits in the window, in physical pixels, plus its state.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectPanel {
    /// Image-overlay key (`rich_text_id`) of the panel.
    pub id: usize,
    /// `[x, y, width, height]`, integer-aligned.
    pub rect: [f32; 4],
    pub cell_size: [f32; 2],
    /// Whether the panel has keyboard focus (window focused, active split).
    pub focused: bool,
    pub theme: EffectTheme,
    pub cursor: EffectCursor,
    /// Changes when the view jumps (e.g. scrollback), resetting cursor history.
    pub view: u64,
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

/// Clock and per-panel state shared by both GPU backends.
#[derive(Debug, Default)]
pub(crate) struct EffectHost {
    clock: EffectClock,
    panels: FxHashMap<usize, PanelState>,
}

#[derive(Debug)]
struct PanelState {
    focused: bool,
    focus_time: f32,
    /// Last converted theme, reused while the input is unchanged.
    theme: (EffectTheme, Vec<u8>),
    cursor: Option<CursorTrack>,
}

/// Timing of one rendered frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FrameTime {
    time: f32,
    delta: f32,
    frame: u32,
}

impl EffectHost {
    pub fn begin_frame(&mut self) -> FrameTime {
        let (time, delta, frame) = self.clock.tick();
        FrameTime { time, delta, frame }
    }

    /// Uniforms of `panel`, drawn into a target of `rect`'s size.
    pub fn uniforms(
        &mut self,
        t: FrameTime,
        panel: &EffectPanel,
        rect: [u32; 4],
        scale: f32,
        colorspace: Colorspace,
    ) -> PanelUniforms {
        let state = self.panels.entry(panel.id).or_insert_with(|| PanelState {
            focused: panel.focused,
            focus_time: t.time,
            theme: (panel.theme.clone(), panel.theme.to_uniform(colorspace)),
            cursor: None,
        });
        if state.focused != panel.focused {
            state.focused = panel.focused;
            state.focus_time = t.time;
        }
        if state.theme.0 != panel.theme {
            state.theme = (panel.theme.clone(), panel.theme.to_uniform(colorspace));
        }
        let frame = EffectFrameUniform {
            resolution: [rect[2] as f32, rect[3] as f32],
            time: t.time,
            time_delta: t.delta,
            frame: t.frame,
            scale,
            cell_size: panel.cell_size,
            focused: panel.focused as u32,
            focus_time: state.focus_time,
            _pad: [0; 2],
        };
        let c = &panel.cursor;
        let cursor = CursorState {
            rect: c.rect,
            color: shader_color(c.color, colorspace),
            shape: c.shape,
            visible: c.visible as u32,
            blink_on: c.blink_on as u32,
            time: t.time,
        };
        let view = (panel.view, [rect[2], rect[3]]);
        match &mut state.cursor {
            Some(track) => track.update(view, cursor, t.time),
            None => state.cursor = Some(CursorTrack::new(view, cursor, t.time)),
        }
        let cursor = state.cursor.as_ref().map(CursorTrack::to_uniform);
        PanelUniforms {
            blocks: [
                bytemuck::bytes_of(&frame).to_vec(),
                state.theme.1.clone(),
                cursor.map_or_else(Vec::new, |c| bytemuck::bytes_of(&c).to_vec()),
            ],
        }
    }

    /// Forget panels not in `ids`.
    pub fn retain_panels(&mut self, ids: &[usize]) {
        self.panels.retain(|id, _| ids.contains(id));
    }
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

    let params = check_abi(&module).map_err(|e| format!("{name}: {e}"))?;

    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|e| e.emit_to_string_with_path(&wgsl, name))?;

    Ok(ValidatedEffect {
        name: name.to_string(),
        wgsl,
        module,
        info,
        params,
    })
}

fn check_abi(module: &naga::Module) -> Result<Option<ParamsLayout>, String> {
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

    let mut params = None;
    for (_, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else {
            continue;
        };
        let var_name = var.name.as_deref().unwrap_or("?");
        let host = var_name.starts_with("rio_");
        if host && binding.group == 0 {
            continue;
        }
        if !host
            && binding.group == 1
            && binding.binding == 0
            && var.space == naga::AddressSpace::Uniform
        {
            params = Some(params_layout(module, var.ty)?);
            continue;
        }
        return Err(format!(
            "resource binding `{var_name}` (@group({}) @binding({})) is not supported; \
             parameters go in `@group(1) @binding(0) var<uniform>`",
            binding.group, binding.binding
        ));
    }
    Ok(params)
}

fn params_layout(
    module: &naga::Module,
    ty: naga::Handle<naga::Type>,
) -> Result<ParamsLayout, String> {
    let naga::TypeInner::Struct { members, span } = &module.types[ty].inner else {
        return Err("the parameter binding must be a struct".into());
    };
    if members.len() > MAX_PARAMS || *span > MAX_PARAMS_SIZE {
        return Err(format!(
            "parameter structs are limited to {MAX_PARAMS} members and {MAX_PARAMS_SIZE} bytes"
        ));
    }
    let members = members
        .iter()
        .map(|m| {
            let name = m.name.clone().unwrap_or_default();
            let components = match module.types[m.ty].inner {
                naga::TypeInner::Scalar(naga::Scalar::F32) => 1,
                naga::TypeInner::Vector {
                    size,
                    scalar: naga::Scalar::F32,
                } => size as u32,
                _ => {
                    return Err(format!(
                        "parameter `{name}` must be `f32` or `vecN<f32>`"
                    ))
                }
            };
            Ok(ParamMember {
                name,
                offset: m.offset,
                components,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(ParamsLayout {
        size: *span,
        members,
    })
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

    const PARAMS: &str = r#"
struct EffectParams {
    color: vec4<f32>,
    opacity: f32,
    offset: vec2<f32>,
}
@group(1) @binding(0) var<uniform> params: EffectParams;
fn effect_main(pos: vec2<f32>) -> vec4<f32> {
    return params.color * params.opacity + vec4<f32>(params.offset, 0.0, 0.0);
}
"#;

    fn values(pairs: &[(&str, ParamValue)]) -> Vec<(String, ParamValue)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn params_are_reflected_and_packed_by_name() {
        let effect = validate(&src(PARAMS)).unwrap();
        let layout = effect.params.as_ref().unwrap();
        let offsets: Vec<_> = layout
            .members
            .iter()
            .map(|m| (m.offset, m.components))
            .collect();
        assert_eq!(offsets, [(0, 4), (16, 1), (24, 2)]);
        // Configuration order does not matter.
        let bytes = effect
            .pack_params(&values(&[
                ("offset", ParamValue::Vector(vec![5.0, 6.0])),
                ("opacity", ParamValue::Number(0.5)),
                ("color", ParamValue::Vector(vec![1.0, 2.0, 3.0, 4.0])),
            ]))
            .unwrap();
        let floats: &[f32] = bytemuck::cast_slice(&bytes);
        assert_eq!(floats, [1.0, 2.0, 3.0, 4.0, 0.5, 0.0, 5.0, 6.0]);
    }

    #[test]
    fn bad_params_are_rejected() {
        let effect = validate(&src(PARAMS)).unwrap();
        let ok = [
            ("color", ParamValue::Vector(vec![0.0; 4])),
            ("opacity", ParamValue::Number(1.0)),
            ("offset", ParamValue::Vector(vec![0.0; 2])),
        ];
        let err = |pairs: &[(&str, ParamValue)]| {
            effect.pack_params(&values(pairs)).unwrap_err()
        };
        assert!(err(&ok[..2]).contains("missing parameter `offset`"));
        let mut extra = ok.to_vec();
        extra.push(("width", ParamValue::Number(1.0)));
        assert!(err(&extra).contains("unknown parameter `width`"));
        let mut wrong = ok.to_vec();
        wrong[1].1 = ParamValue::Vector(vec![1.0]);
        assert!(err(&wrong).contains("must be a number"));
        wrong = ok.to_vec();
        wrong[0].1 = ParamValue::Vector(vec![1.0; 3]);
        assert!(err(&wrong).contains("4 numbers"));
        wrong = ok.to_vec();
        wrong[1].1 = ParamValue::Number(1e300);
        assert!(err(&wrong).contains("out of range"));

        let plain = validate(&src(IDENTITY)).unwrap();
        assert!(plain.pack_params(&[]).unwrap().is_empty());
        assert!(plain.pack_params(&values(&ok[..1])).is_err());
    }

    #[test]
    fn unsupported_param_types_are_rejected() {
        let err =
            validate(&src(&PARAMS.replace("opacity: f32", "opacity: u32"))).unwrap_err();
        assert!(err.contains("`opacity` must be"), "{err}");
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
    fn uniform_sizes_match_prelude() {
        let effect = validate(&src(IDENTITY)).unwrap();
        let module = &effect.module;
        for (binding, size) in UNIFORM_BINDINGS.iter().zip(UNIFORM_SIZES) {
            let (_, var) = module
                .global_variables
                .iter()
                .find(|(_, v)| v.binding.as_ref().is_some_and(|b| b.binding == *binding))
                .unwrap();
            let wgsl = module.types[var.ty].inner.size(module.to_ctx()) as usize;
            assert!(
                wgsl <= size && size % 16 == 0,
                "binding {binding}: {wgsl} vs {size}"
            );
        }
    }

    #[test]
    fn shader_colors_are_linear_premultiplied_p3() {
        let p3 = |c| shader_color(c, Colorspace::DisplayP3);
        assert_eq!(p3([1.0, 0.0, 1.0, 0.5]), [0.5, 0.0, 0.5, 0.5]);
        let gray = p3([0.5, 0.5, 0.5, 1.0]);
        assert!((gray[0] - 0.214).abs() < 1e-3, "{gray:?}");
        let white = shader_color([1.0; 4], Colorspace::Srgb);
        assert!(white.iter().all(|v| (v - 1.0).abs() < 1e-5), "{white:?}");
    }

    #[test]
    fn focus_changes_are_timestamped() {
        let mut host = EffectHost::default();
        let mut panel = EffectPanel {
            id: 1,
            rect: [0.0, 0.0, 10.0, 10.0],
            cell_size: [1.0, 1.0],
            focused: false,
            theme: EffectTheme::default(),
            cursor: EffectCursor::default(),
            view: 0,
        };
        let frame = |host: &mut EffectHost, panel: &EffectPanel, time| {
            let t = FrameTime {
                time,
                delta: 0.0,
                frame: 0,
            };
            let u = host.uniforms(t, panel, [0, 0, 10, 10], 1.0, Colorspace::Srgb);
            bytemuck::pod_read_unaligned::<EffectFrameUniform>(&u.blocks[0])
        };
        assert_eq!(frame(&mut host, &panel, 1.0).focus_time, 1.0);
        assert_eq!(frame(&mut host, &panel, 2.0).focus_time, 1.0);
        panel.focused = true;
        let f = frame(&mut host, &panel, 3.0);
        assert_eq!((f.focused, f.focus_time), (1, 3.0));
    }

    #[test]
    fn cursor_history_tracks_moves_and_resets_on_view_changes() {
        let mut host = EffectHost::default();
        let mut panel = EffectPanel {
            id: 1,
            rect: [0.0, 0.0, 10.0, 10.0],
            cell_size: [1.0, 1.0],
            focused: true,
            theme: EffectTheme::default(),
            cursor: EffectCursor {
                visible: true,
                ..Default::default()
            },
            view: 0,
        };
        let frame = |host: &mut EffectHost, panel: &EffectPanel, time| {
            let t = FrameTime {
                time,
                delta: 0.0,
                frame: 0,
            };
            let u = host.uniforms(t, panel, [0, 0, 10, 10], 1.0, Colorspace::DisplayP3);
            bytemuck::pod_read_unaligned::<CursorUniform>(&u.blocks[2])
        };
        frame(&mut host, &panel, 1.0);
        panel.cursor.rect = [1.0, 0.0, 1.0, 1.0];
        frame(&mut host, &panel, 2.0);
        // Blink alone is not a change.
        panel.cursor.blink_on = true;
        let c = frame(&mut host, &panel, 3.0);
        assert_eq!(c.history_len, 2);
        assert_eq!((c.history[0].rect[0], c.history[0].time), (1.0, 2.0));
        assert_eq!((c.history[1].rect[0], c.history[1].time), (0.0, 1.0));
        assert_eq!((c.move_time, c.change_time), (2.0, 1.0));
        assert_eq!((c.current.time, c.current.blink_on), (2.0, 1));
        assert_eq!(c.previous.rect[0], 0.0);

        panel.cursor.shape = 2;
        let c = frame(&mut host, &panel, 4.0);
        assert_eq!((c.move_time, c.change_time, c.history_len), (2.0, 4.0, 2));

        panel.view = 1;
        let c = frame(&mut host, &panel, 5.0);
        assert_eq!((c.history_len, c.move_time), (1, 5.0));
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
            focused: false,
            theme: EffectTheme::default(),
            cursor: EffectCursor::default(),
            view: 0,
        };
        assert_eq!(p.clipped_rect((500.0, 500.0)), Some([10, 20, 490, 30]));
        assert_eq!(p.clipped_rect((5.0, 5.0)), None);
    }
}
