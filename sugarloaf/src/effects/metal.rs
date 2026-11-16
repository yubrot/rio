//! Native Metal execution of effect chains. The same WGSL (user passes
//! and `host.wgsl`) is translated to MSL by Naga and compiled at runtime.

use super::msl::{effect_resources, host_resources, to_msl, validator};
use super::{
    EffectFrameUniform, HostPassUniform, SurfaceAlpha, ValidatedEffect, FRAGMENT_ENTRY,
    VERTEX_ENTRY,
};
use metal::{
    CommandBufferRef, CompileOptions, Device, MTLClearColor, MTLLoadAction,
    MTLPixelFormat, MTLPrimitiveType, MTLSamplerAddressMode, MTLSamplerMinMagFilter,
    MTLScissorRect, MTLStorageMode, MTLStoreAction, MTLTextureType, MTLTextureUsage,
    MTLViewport, RenderCommandEncoderRef, RenderPassDescriptor, RenderPipelineDescriptor,
    RenderPipelineState, SamplerDescriptor, SamplerState, Texture, TextureDescriptor,
    TextureRef,
};
use rustc_hash::FxHashMap;

/// Linear premultiplied working format for chain intermediates.
const CHAIN_FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA16Float;
/// Must match the drawable format in `context/metal.rs`.
const SURFACE_FORMAT: MTLPixelFormat = MTLPixelFormat::BGRA8Unorm;

pub(crate) struct MetalEffects {
    passes: Vec<RenderPipelineState>,
    decode: RenderPipelineState,
    encode: RenderPipelineState,
    sampler: SamplerState,
    panels: FxHashMap<usize, PanelTargets>,
}

struct PanelTargets {
    size: [u32; 2],
    capture: Texture,
    original: Texture,
    /// Ping-pong targets; pass `i` writes `chain[i % 2]`.
    chain: Vec<Texture>,
    /// Clipped window rect `[x, y, w, h]` for this frame.
    rect: [u32; 4],
    frame: EffectFrameUniform,
    alpha: SurfaceAlpha,
}

impl MetalEffects {
    pub fn new(device: &Device) -> Result<Self, String> {
        let host = naga::front::wgsl::parse_str(include_str!("host.wgsl"))
            .map_err(|e| e.to_string())?;
        let info = validator()
            .validate(&host)
            .map_err(|e| e.into_inner().to_string())?;
        let (msl, names) = to_msl(&host, &info, host_resources())?;
        let decode = pipeline(
            device,
            "effects::decode",
            &msl,
            &names,
            "vs_main",
            "fs_decode",
            CHAIN_FORMAT,
        )?;
        let encode = pipeline(
            device,
            "effects::encode",
            &msl,
            &names,
            "vs_main",
            "fs_encode",
            SURFACE_FORMAT,
        )?;

        let desc = SamplerDescriptor::new();
        desc.set_min_filter(MTLSamplerMinMagFilter::Linear);
        desc.set_mag_filter(MTLSamplerMinMagFilter::Linear);
        desc.set_address_mode_s(MTLSamplerAddressMode::ClampToEdge);
        desc.set_address_mode_t(MTLSamplerAddressMode::ClampToEdge);
        let sampler = device.new_sampler(&desc);

        Ok(Self {
            passes: Vec::new(),
            decode,
            encode,
            sampler,
            panels: FxHashMap::default(),
        })
    }

    /// Build pipelines for `effects`, skipping (and reporting) any that
    /// fail translation or Metal compilation.
    pub fn set_passes(
        &mut self,
        device: &Device,
        effects: &[ValidatedEffect],
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let mut passes = Vec::with_capacity(effects.len());
        for effect in effects {
            let built = to_msl(&effect.module, &effect.info, effect_resources())
                .and_then(|(src, names)| {
                    pipeline(
                        device,
                        &effect.name,
                        &src,
                        &names,
                        VERTEX_ENTRY,
                        FRAGMENT_ENTRY,
                        CHAIN_FORMAT,
                    )
                });
            match built {
                Ok(p) => passes.push(p),
                Err(e) => errors.push(format!("{}: {e}", effect.name)),
            }
        }
        self.passes = passes;
        self.panels.clear();
        errors
    }

    pub fn is_active(&self) -> bool {
        !self.passes.is_empty()
    }

    /// Allocate or update `panel`'s targets for this frame.
    pub fn prepare_panel(
        &mut self,
        device: &Device,
        id: usize,
        rect: [u32; 4],
        frame: EffectFrameUniform,
        alpha: SurfaceAlpha,
    ) {
        let size = [rect[2], rect[3]];
        let chain_len = self.passes.len().min(2);
        let t = self
            .panels
            .entry(id)
            .and_modify(|t| {
                if t.size != size {
                    *t = PanelTargets::new(device, size, chain_len);
                }
            })
            .or_insert_with(|| PanelTargets::new(device, size, chain_len));
        t.rect = rect;
        t.frame = frame;
        t.alpha = alpha;
    }

    /// Drop targets of panels not in `ids`.
    pub fn retain_panels(&mut self, ids: &[usize]) {
        self.panels.retain(|id, _| ids.contains(id));
    }

    pub fn capture(&self, id: usize) -> Option<&TextureRef> {
        self.panels.get(&id).map(|t| t.capture.as_ref())
    }

    /// Encode decode + every user pass for `id` after its capture.
    pub fn run_chain(&self, command_buffer: &CommandBufferRef, id: usize) {
        let Some(t) = self.panels.get(&id) else {
            return;
        };
        let host = HostPassUniform {
            origin: [0.0; 2],
            alpha_mode: t.alpha as u32,
            _pad: 0,
        };
        let enc = begin_pass(command_buffer, &t.original, "effects::decode");
        enc.set_render_pipeline_state(&self.decode);
        enc.set_fragment_texture(0, Some(&t.capture));
        set_fragment_bytes(enc, 0, &host);
        enc.draw_primitives(MTLPrimitiveType::Triangle, 0, 3);
        enc.end_encoding();

        for (i, pass) in self.passes.iter().enumerate() {
            let input = if i == 0 {
                &t.original
            } else {
                &t.chain[(i - 1) % 2]
            };
            let enc = begin_pass(command_buffer, &t.chain[i % 2], "effects::pass");
            enc.set_render_pipeline_state(pass);
            set_fragment_bytes(enc, 0, &t.frame);
            enc.set_fragment_texture(0, Some(input));
            enc.set_fragment_texture(1, Some(&t.original));
            enc.set_fragment_sampler_state(0, Some(&self.sampler));
            enc.draw_primitives(MTLPrimitiveType::Triangle, 0, 3);
            enc.end_encoding();
        }
    }

    /// Write `id`'s chain output into its rect of the window pass.
    pub fn composite(&self, enc: &RenderCommandEncoderRef, id: usize) {
        let Some(t) = self.panels.get(&id) else {
            return;
        };
        let [x, y, w, h] = t.rect;
        let host = HostPassUniform {
            origin: [x as f32, y as f32],
            alpha_mode: t.alpha as u32,
            _pad: 0,
        };
        set_viewport(enc, [x, y, w, h]);
        enc.set_render_pipeline_state(&self.encode);
        enc.set_fragment_texture(0, Some(&t.chain[(self.passes.len() - 1) % 2]));
        set_fragment_bytes(enc, 0, &host);
        enc.draw_primitives(MTLPrimitiveType::Triangle, 0, 3);
    }
}

impl PanelTargets {
    fn new(device: &Device, size: [u32; 2], chain_len: usize) -> Self {
        Self {
            size,
            capture: texture(device, size, SURFACE_FORMAT),
            original: texture(device, size, CHAIN_FORMAT),
            chain: (0..chain_len)
                .map(|_| texture(device, size, CHAIN_FORMAT))
                .collect(),
            rect: [0; 4],
            frame: EffectFrameUniform::default(),
            alpha: SurfaceAlpha::Opaque,
        }
    }
}

/// Set both the viewport and scissor to `rect` (`[x, y, w, h]`).
pub(crate) fn set_viewport(enc: &RenderCommandEncoderRef, [x, y, w, h]: [u32; 4]) {
    enc.set_viewport(MTLViewport {
        originX: x as f64,
        originY: y as f64,
        width: w as f64,
        height: h as f64,
        znear: 0.0,
        zfar: 1.0,
    });
    enc.set_scissor_rect(MTLScissorRect {
        x: x as u64,
        y: y as u64,
        width: w as u64,
        height: h as u64,
    });
}

/// Begin a render pass into `target`, cleared to transparent.
pub(crate) fn begin_pass<'a>(
    command_buffer: &'a CommandBufferRef,
    target: &TextureRef,
    label: &str,
) -> &'a RenderCommandEncoderRef {
    let desc = RenderPassDescriptor::new();
    let color = desc.color_attachments().object_at(0).unwrap();
    color.set_texture(Some(target));
    color.set_load_action(MTLLoadAction::Clear);
    color.set_store_action(MTLStoreAction::Store);
    color.set_clear_color(MTLClearColor::new(0.0, 0.0, 0.0, 0.0));
    let enc = command_buffer.new_render_command_encoder(desc);
    enc.set_label(label);
    enc
}

fn set_fragment_bytes<T: bytemuck::Pod>(
    enc: &RenderCommandEncoderRef,
    index: u64,
    value: &T,
) {
    let bytes = bytemuck::bytes_of(value);
    enc.set_fragment_bytes(
        index,
        bytes.len() as u64,
        bytes.as_ptr() as *const std::ffi::c_void,
    );
}

fn texture(device: &Device, size: [u32; 2], format: MTLPixelFormat) -> Texture {
    let desc = TextureDescriptor::new();
    desc.set_texture_type(MTLTextureType::D2);
    desc.set_pixel_format(format);
    desc.set_width(size[0] as u64);
    desc.set_height(size[1] as u64);
    desc.set_storage_mode(MTLStorageMode::Private);
    desc.set_usage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
    device.new_texture(&desc)
}

fn pipeline(
    device: &Device,
    label: &str,
    msl_source: &str,
    names: &[(String, String)],
    vs: &str,
    fs: &str,
    format: MTLPixelFormat,
) -> Result<RenderPipelineState, String> {
    let library = device
        .new_library_with_source(msl_source, &CompileOptions::new())
        .map_err(|e| format!("Metal compilation failed: {e}"))?;
    let function = |wgsl_name: &str| {
        let name = names
            .iter()
            .find(|(n, _)| n == wgsl_name)
            .map(|(_, m)| m.as_str())
            .ok_or_else(|| format!("missing entry point `{wgsl_name}`"))?;
        library.get_function(name, None)
    };
    let desc = RenderPipelineDescriptor::new();
    desc.set_label(label);
    let (vs, fs) = (function(vs)?, function(fs)?);
    desc.set_vertex_function(Some(&vs));
    desc.set_fragment_function(Some(&fs));
    let color = desc.color_attachments().object_at(0).unwrap();
    color.set_pixel_format(format);
    // Passes replace their target; shaders compose themselves.
    color.set_blending_enabled(false);
    device
        .new_render_pipeline_state(&desc)
        .map_err(|e| format!("Metal pipeline creation failed: {e}"))
}

/// How the layer stores alpha: a non-opaque `CAMetalLayer` composites
/// premultiplied content.
pub(crate) fn surface_alpha(ctx: &crate::context::metal::MetalContext) -> SurfaceAlpha {
    if ctx.layer.is_opaque() {
        SurfaceAlpha::Opaque
    } else {
        SurfaceAlpha::Premultiplied
    }
}
