//! wgpu execution of effect chains: per-panel targets, the decode →
//! user passes → encode pipeline, and pipeline creation with error scopes.

use super::{
    HostPassUniform, PanelUniforms, SurfaceAlpha, ValidatedEffect, FRAGMENT_ENTRY,
    UNIFORM_BINDINGS, UNIFORM_SIZES, VERTEX_ENTRY,
};
use crate::context::webgpu::WgpuContext;
use crate::renderer::Renderer;
use rustc_hash::FxHashMap;

/// Linear premultiplied working format for chain intermediates.
const CHAIN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub(crate) struct WgpuEffects {
    passes: Vec<wgpu::RenderPipeline>,
    effect_bgl: wgpu::BindGroupLayout,
    host_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    decode: wgpu::RenderPipeline,
    encode: wgpu::RenderPipeline,
    panels: FxHashMap<usize, PanelTargets>,
}

struct PanelTargets {
    size: [u32; 2],
    capture: wgpu::TextureView,
    /// Bind group for drawing window-space content into `capture`.
    constant: wgpu::BindGroup,
    transform: wgpu::Buffer,
    /// Per-panel uniform blocks, in [`UNIFORM_BINDINGS`] order.
    uniforms: Vec<wgpu::Buffer>,
    decode_uniform: wgpu::Buffer,
    encode_uniform: wgpu::Buffer,
    original: wgpu::TextureView,
    decode_bg: wgpu::BindGroup,
    pass_bgs: Vec<wgpu::BindGroup>,
    /// Ping-pong targets; pass `i` writes `chain[i % 2]`.
    chain: Vec<wgpu::TextureView>,
    encode_bg: wgpu::BindGroup,
    /// Clipped window rect `[x, y, w, h]` for this frame.
    rect: [u32; 4],
}

impl WgpuEffects {
    pub fn new(ctx: &WgpuContext) -> Self {
        let device = &ctx.device;
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let effect_entries: Vec<_> = UNIFORM_BINDINGS
            .iter()
            .map(|&b| uniform_entry(b))
            .chain([
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ])
            .collect();
        let effect_bgl =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("effects::bgl"),
                entries: &effect_entries,
            });
        let host_bgl =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("effects::host_bgl"),
                entries: &[texture_entry(0), uniform_entry(1)],
            });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("effects::sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let host_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("effects::host"),
            source: wgpu::ShaderSource::Wgsl(include_str!("host.wgsl").into()),
        });
        let host_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("effects::host_layout"),
                bind_group_layouts: &[Some(&host_bgl)],
                immediate_size: 0,
            });
        let decode = fullscreen_pipeline(
            device,
            "effects::decode",
            &host_layout,
            &host_module,
            ("vs_main", "fs_decode"),
            CHAIN_FORMAT,
        );
        let encode = fullscreen_pipeline(
            device,
            "effects::encode",
            &host_layout,
            &host_module,
            ("vs_main", "fs_encode"),
            ctx.format,
        );

        Self {
            passes: Vec::new(),
            effect_bgl,
            host_bgl,
            sampler,
            decode,
            encode,
            panels: FxHashMap::default(),
        }
    }

    /// Build pipelines for `effects`, skipping (and reporting) any that
    /// wgpu rejects.
    pub fn set_passes(
        &mut self,
        ctx: &WgpuContext,
        effects: &[ValidatedEffect],
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let device = &ctx.device;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effects::layout"),
            bind_group_layouts: &[Some(&self.effect_bgl)],
            immediate_size: 0,
        });
        let mut passes = Vec::with_capacity(effects.len());
        for effect in effects {
            // wgpu reports these errors to the device error handler
            // (a panic by default), so capture them in a scope.
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&effect.name),
                source: wgpu::ShaderSource::Wgsl(effect.wgsl.as_str().into()),
            });
            let pipeline = fullscreen_pipeline(
                device,
                &effect.name,
                &layout,
                &module,
                (VERTEX_ENTRY, FRAGMENT_ENTRY),
                CHAIN_FORMAT,
            );
            match futures::executor::block_on(scope.pop()) {
                Some(err) => errors.push(format!("{}: {err}", effect.name)),
                None => passes.push(pipeline),
            }
        }
        self.passes = passes;
        // Pass bind groups depend on the pass count.
        self.panels.clear();
        errors
    }

    pub fn is_active(&self) -> bool {
        !self.passes.is_empty()
    }

    /// Allocate or update `panel`'s targets for this frame.
    pub fn prepare_panel(
        &mut self,
        ctx: &WgpuContext,
        renderer: &Renderer,
        id: usize,
        rect: [u32; 4],
        uniforms: &PanelUniforms,
        alpha: SurfaceAlpha,
    ) -> bool {
        let size = [rect[2], rect[3]];
        if self.panels.get(&id).is_none_or(|t| t.size != size) {
            let Some(targets) = self.create_targets(ctx, renderer, size) else {
                return false;
            };
            self.panels.insert(id, targets);
        }
        let Some(t) = self.panels.get_mut(&id) else {
            return false;
        };
        t.rect = rect;
        let rectf = rect.map(|v| v as f32);
        let queue = &ctx.queue;
        queue.write_buffer(
            &t.transform,
            0,
            bytemuck::cast_slice(&super::panel_projection(rectf)),
        );
        for (buffer, data) in t.uniforms.iter().zip(&uniforms.blocks) {
            queue.write_buffer(buffer, 0, data);
        }
        let host = |origin: [f32; 2]| HostPassUniform {
            origin,
            alpha_mode: alpha as u32,
            _pad: 0,
        };
        queue.write_buffer(&t.decode_uniform, 0, bytemuck::bytes_of(&host([0.0; 2])));
        queue.write_buffer(
            &t.encode_uniform,
            0,
            bytemuck::bytes_of(&host([rectf[0], rectf[1]])),
        );
        true
    }

    /// Drop targets of panels not in `ids`.
    pub fn retain_panels(&mut self, ids: &[usize]) {
        self.panels.retain(|id, _| ids.contains(id));
    }

    pub fn capture_view(&self, id: usize) -> Option<&wgpu::TextureView> {
        self.panels.get(&id).map(|t| &t.capture)
    }

    pub fn constant_bind_group(&self, id: usize) -> Option<&wgpu::BindGroup> {
        self.panels.get(&id).map(|t| &t.constant)
    }

    /// Record decode + every user pass for `id` after its capture.
    pub fn run_chain(&self, encoder: &mut wgpu::CommandEncoder, id: usize) {
        let Some(t) = self.panels.get(&id) else {
            return;
        };
        fullscreen_pass(
            encoder,
            "effects::decode",
            &t.original,
            &self.decode,
            &t.decode_bg,
        );
        for (i, pipeline) in self.passes.iter().enumerate() {
            fullscreen_pass(
                encoder,
                "effects::pass",
                &t.chain[i % 2],
                pipeline,
                &t.pass_bgs[i],
            );
        }
    }

    /// Write `id`'s chain output into its rect of the window pass.
    pub fn composite(&self, rpass: &mut wgpu::RenderPass<'_>, id: usize) {
        let Some(t) = self.panels.get(&id) else {
            return;
        };
        let [x, y, w, h] = t.rect;
        rpass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        rpass.set_scissor_rect(x, y, w, h);
        rpass.set_pipeline(&self.encode);
        rpass.set_bind_group(0, &t.encode_bg, &[]);
        rpass.draw(0..3, 0..1);
    }

    fn create_targets(
        &self,
        ctx: &WgpuContext,
        renderer: &Renderer,
        size: [u32; 2],
    ) -> Option<PanelTargets> {
        let device = &ctx.device;
        let texture = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let uniform = |label, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let host_bg = |src: &wgpu::TextureView, buf: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("effects::host_bg"),
                layout: &self.host_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buf.as_entire_binding(),
                    },
                ],
            })
        };

        let capture = texture("effects::capture", ctx.format);
        let transform = uniform("effects::transform", std::mem::size_of::<[f32; 16]>());
        let constant = renderer.constant_bind_group_wgpu(ctx, &transform)?;
        let uniforms: Vec<_> = UNIFORM_SIZES
            .iter()
            .map(|&size| uniform("effects::uniform", size))
            .collect();
        let decode_uniform =
            uniform("effects::decode", std::mem::size_of::<HostPassUniform>());
        let encode_uniform =
            uniform("effects::encode", std::mem::size_of::<HostPassUniform>());
        let original = texture("effects::original", CHAIN_FORMAT);
        let chain_len = self.passes.len().min(2);
        let chain: Vec<_> = (0..chain_len)
            .map(|_| texture("effects::chain", CHAIN_FORMAT))
            .collect();
        let decode_bg = host_bg(&capture, &decode_uniform);
        let pass_bgs = (0..self.passes.len())
            .map(|i| {
                let input = if i == 0 {
                    &original
                } else {
                    &chain[(i - 1) % 2]
                };
                let blocks = UNIFORM_BINDINGS.iter().zip(&uniforms).map(|(b, buf)| {
                    wgpu::BindGroupEntry {
                        binding: *b,
                        resource: buf.as_entire_binding(),
                    }
                });
                let entries: Vec<_> = blocks
                    .chain([
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(input),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&original),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ])
                    .collect();
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("effects::pass_bg"),
                    layout: &self.effect_bgl,
                    entries: &entries,
                })
            })
            .collect();
        let output = &chain[(self.passes.len() - 1) % 2];
        let encode_bg = host_bg(output, &encode_uniform);

        Some(PanelTargets {
            size,
            capture,
            constant,
            transform,
            uniforms,
            decode_uniform,
            encode_uniform,
            original,
            decode_bg,
            pass_bgs,
            chain,
            encode_bg,
            rect: [0; 4],
        })
    }
}

/// How `ctx`'s surface stores alpha.
pub(crate) fn surface_alpha(ctx: &WgpuContext) -> SurfaceAlpha {
    match ctx.alpha_mode {
        wgpu::CompositeAlphaMode::PreMultiplied => SurfaceAlpha::Premultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied => SurfaceAlpha::Straight,
        _ => SurfaceAlpha::Opaque,
    }
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    (vs, fs): (&str, &str),
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vs),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            // Passes replace their target; shaders compose themselves.
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
) {
    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rpass.set_pipeline(pipeline);
    rpass.set_bind_group(0, bind_group, &[]);
    rpass.draw(0..3, 0..1);
}
