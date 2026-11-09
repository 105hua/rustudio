//! SurfaceGuis and BillboardGuis in the world (round 23): their egui
//! triangles drawn onto the finished view, each pixel tested against the
//! scene's depth, so parts in front hide them as in Studio (AlwaysOnTop
//! false). AlwaysOnTop ones and ScreenGuis are still painted over the view
//! by egui.
//!
//! The scene's depth is 4× multisampled and reverse-Z (1 near, 0 far); the
//! fragment shader reads sample 0 and drops what lies behind it. Colours are
//! egui's: premultiplied, gamma-space, on an Rgba8Unorm target (egui's
//! "gamma framebuffer" path), so they come out as egui would paint them.

use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

/// One vertex: NDC position, reverse-Z depth, egui's UV and colour, and
/// the light it takes (linear RGB
/// factor: Brightness × (1 − LightInfluence) + LightInfluence × the light).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GuiVertex {
    pub ndc: [f32; 2],
    pub depth: f32,
    pub uv: [f32; 2],
    pub color: [u8; 4],
    pub light: [f32; 3],
    /// Studs from the camera (fog).
    pub dist: f32,
    /// Particles, beams and trails (round 25, src/effects.rs): the clip-space
    /// w, with `ndc` and `depth` holding clip x, y and z, so the GPU clips at
    /// the near plane and interpolates perspective-correctly. 0 for GUIs.
    pub clip_w: f32,
    /// What alpha is scaled by: 1 − LightEmission for effects (additive
    /// toward 1 under premultiplied blending), 1 for GUIs.
    pub alpha_mul: f32,
    /// Particles with FlipbookBlendFrames (round 26): the next frame's UV
    /// and how far toward it (ParticleCustomFS mixes the two frames'
    /// premultiplied texels by TEXCOORD7.z). 0 = the one frame.
    pub uv2: [f32; 2],
    pub blend: f32,
    /// Soft particles (round 26, ParticleCustomFS_Soft): 1 / the fade
    /// distance (the particle's half size; 0 = hard) and its ZOffset.
    pub soft: [f32; 2],
}

/// Triangles drawn with one egui texture, in drawing order.
#[derive(Default, Clone)]
pub struct GuiBatch {
    pub texture: Option<egui::TextureId>,
    /// Sampled repeating (beam and trail textures tile), else clamped.
    pub wrap: bool,
    pub vertices: Vec<GuiVertex>,
    pub indices: Vec<u32>,
}

/// Appends a mesh's triangles, starting a new batch when the texture changes
/// (so later items stay over earlier ones).
pub fn push(batches: &mut Vec<GuiBatch>, texture: egui::TextureId, vertices: Vec<GuiVertex>, indices: &[u32]) {
    push_wrap(batches, texture, false, vertices, indices);
}

/// `push`, with the texture sampled repeating (`wrap`) or clamped.
pub fn push_wrap(batches: &mut Vec<GuiBatch>, texture: egui::TextureId, wrap: bool, vertices: Vec<GuiVertex>, indices: &[u32]) {
    if batches.last().is_none_or(|b| b.texture != Some(texture) || b.wrap != wrap) {
        batches.push(GuiBatch { texture: Some(texture), wrap, ..Default::default() });
    }
    let b = batches.last_mut().expect("batch");
    let base = b.vertices.len() as u32;
    b.vertices.extend(vertices);
    b.indices.extend(indices.iter().map(|i| base + i));
}

const SHADER: &str = r#"
struct VIn {
    @location(0) ndc: vec2<f32>,
    @location(1) depth: f32,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) light: vec3<f32>,
    @location(5) dist: f32,
    @location(6) clip_w: f32,
    @location(7) alpha_mul: f32,
    @location(8) uv2: vec2<f32>,
    @location(9) blend: f32,
    @location(10) soft: vec2<f32>,
};
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) depth: f32,
    @location(3) light: vec3<f32>,
    @location(4) dist: f32,
    // 1 for particles / beams / trails (clip-space input), 0 for GUIs.
    @location(5) @interpolate(flat) fx: f32,
    @location(6) @interpolate(flat) alpha_mul: f32,
    @location(7) uv2: vec2<f32>,
    @location(8) @interpolate(flat) blend: f32,
    @location(9) @interpolate(flat) soft: vec2<f32>,
    // The clip w (view depth) at the pixel, for soft particles.
    @location(10) vw: f32,
};
// Studio's soft-particle depth texture holds view depth / 500
// (ParticleCustomFS_Soft: depth × 500; ParticleCustomVS_Soft: min(w − ZOffset,
// 500 − fade)).
const SOFT_DEPTH_MAX: f32 = 500.0;
// fog: the scene's (shader.wgsl g.fog: start, end or exp2 terms, mode);
// fog_rgb: its colour, linear; e_on: tone curve e, on, the near plane.
struct Tone { curve: vec4<f32>, e_on: vec4<f32>, fog: vec4<f32>, fog_rgb: vec4<f32> };
@group(0) @binding(0) var scene_depth: texture_depth_multisampled_2d;
@group(0) @binding(1) var<uniform> tone: Tone;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    o.pos = vec4<f32>(v.ndc, 0.5, 1.0);
    o.fx = 0.0;
    if (v.clip_w != 0.0) {
        o.pos = vec4<f32>(v.ndc, v.depth, v.clip_w);
        o.fx = 1.0;
    }
    o.alpha_mul = v.alpha_mul;
    o.uv2 = v.uv2;
    o.blend = v.blend;
    o.soft = v.soft;
    o.vw = v.clip_w;
    o.uv = v.uv;
    o.color = v.color;
    o.depth = v.depth;
    o.light = v.light;
    o.dist = v.dist;
    return o;
}

// shader.wgsl fog_amount: Lighting's linear fog, under water Studio's exp2, or
// Atmosphere's exp2 (fog.z 3).
fn fog_amount(d: f32) -> f32 {
    if (tone.fog.z > 2.5) {
        return 1.0 - clamp(exp2(tone.fog.x - tone.fog.y * d), 0.0, 1.0);
    }
    if (tone.fog.z > 1.5) {
        return 1.0 - clamp(exp2(tone.fog.x - tone.fog.y * d) - 1.0, 0.0, 1.0);
    }
    return clamp((d - tone.fog.x) / max(tone.fog.y - tone.fog.x, 1e-3), 0.0, 1.0);
}

// The scene's tone curve (shader.wgsl tonemap: y = x(ax+b)/(x(ax+c)+d)·e),
// off for Legacy / Voxel / Compatibility.
fn tonemap(x: vec3<f32>) -> vec3<f32> {
    var y = max(x, vec3<f32>(0.0));
    if (tone.e_on.y > 0.5) {
        let a = tone.curve.x; let b = tone.curve.y; let c = tone.curve.z; let d = tone.curve.w;
        y = y * (a * y + b) / (y * (a * y + c) + d) * tone.e_on.x;
    }
    return sqrt(clamp(y, vec3<f32>(0.0), vec3<f32>(1.0)));
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // (sampled first: textureSample needs uniform control flow)
    // FlipbookBlendFrames: the two frames' premultiplied texels mixed
    // (ParticleCustomFS; egui's textures are premultiplied already).
    let t = mix(textureSample(tex, samp, in.uv), textureSample(tex, samp, in.uv2), in.blend);
    let g = in.color * t;
    let s = textureLoad(scene_depth, vec2<i32>(floor(in.pos.xy)), 0);
    // Studio lights world GUIs in the scene: the colour as linear light ×
    // the GUI's light factor, through the tone curve, back to display.
    var c = g;
    if (g.a > 0.0) {
        var lin = (g.rgb / g.a) * (g.rgb / g.a) * in.light;
        if (in.fx > 0.5) {
            // Studio's ParticleCustomFS / TrailFSLight (round 25): fog fades
            // the effect out (alpha × the fog factor) instead of tinting it,
            // and LightEmission scales only the alpha.
            var a = g.a * (1.0 - fog_amount(in.dist));
            // Soft particles (ParticleCustomFS_Soft, camera-facing ones):
            // faded by how far the scene behind is, over the particle's
            // half size. Reverse-Z infinite: view depth = near / depth.
            if (in.soft.x > 0.0) {
                let sd = select(SOFT_DEPTH_MAX, min(tone.e_on.z / max(s, 1e-9), SOFT_DEPTH_MAX), s > 0.0);
                let pd = min(in.vw - in.soft.y, SOFT_DEPTH_MAX - 1.0 / in.soft.x);
                a *= clamp(abs(sd - pd) * in.soft.x, 0.0, 1.0);
            }
            // Studio blends before its tone curve: the curve of the
            // premultiplied colour (exact over black and when opaque;
            // effects place: red at Transparency 0.5, LightEmission 1 reads
            // 166 in Studio, 126 as tone(c) × a).
            c = vec4<f32>(tonemap(lin * a * a), a * in.alpha_mul);
        } else {
            // Studio's UILitFogFS (shaders_glsl3.pack 0x3efa69): fogged like the
            // scene, after the light and before the exposure.
            lin = mix(lin, tone.fog_rgb.rgb, fog_amount(in.dist));
            c = vec4<f32>(tonemap(lin) * g.a, g.a);
        }
    }
    // (effects: their own clip z / w)
    let depth = select(in.depth, in.pos.z, in.fx > 0.5);
    // Reverse-Z: nearer is larger. A sliver of slack: a SurfaceGui lies on
    // its part's face.
    if (depth < s * (1.0 - 0.0015)) {
        discard;
    }
    return c;
}

// Effects into the scene's light, blended as Studio blends them (round 27):
// in a buffer holding the square root of linear light (WorldGuiPass::effects
// fills it from the scene's HDR image and squares it back), before the tone
// curve and the bloom. ParticleCustomFS: rgb = sqrt(clamp(lin)) × alpha,
// alpha × (1 − LightEmission). Overlapping additive particles add up in
// that space; tone-mapping each one first (round 25) crushed them in the
// curve's toe.
@fragment
fn fs_fx(in: VOut) -> @location(0) vec4<f32> {
    let t = mix(textureSample(tex, samp, in.uv), textureSample(tex, samp, in.uv2), in.blend);
    let g = in.color * t;
    let s = textureLoad(scene_depth, vec2<i32>(floor(in.pos.xy)), 0);
    let depth = select(in.depth, in.pos.z, in.fx > 0.5);
    if (g.a <= 0.0 || depth < s * (1.0 - 0.0015)) {
        discard;
    }
    // (e_on.w: mech::FX_LIGHT_CLAMP)
    var lin = (g.rgb / g.a) * (g.rgb / g.a) * in.light;
    if (in.fx < 0.5) {
        // UILitFogFS: lit world GUI colour enters the scene before post.
        lin = mix(lin, tone.fog_rgb.rgb, fog_amount(in.dist));
        return vec4<f32>(sqrt(max(lin, vec3<f32>(0.0))) * g.a, g.a);
    }
    lin = min(lin, vec3<f32>(tone.e_on.w));
    var a = g.a * (1.0 - fog_amount(in.dist));
    if (in.soft.x > 0.0) {
        let sd = select(SOFT_DEPTH_MAX, min(tone.e_on.z / max(s, 1e-9), SOFT_DEPTH_MAX), s > 0.0);
        let pd = min(in.vw - in.soft.y, SOFT_DEPTH_MAX - 1.0 / in.soft.x);
        a *= clamp(abs(sd - pd) * in.soft.x, 0.0, 1.0);
    }
    return vec4<f32>(sqrt(lin) * a, a * in.alpha_mul);
}
"#;

/// The effects' buffer: filled with the square root of the scene's linear
/// light, and squared back into it once the effects are in.
const FX_SHADER: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
struct VOut { @builtin(position) pos: vec4<f32> };
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}
@fragment
fn fs_in(in: VOut) -> @location(0) vec4<f32> {
    let c = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    return vec4<f32>(sqrt(max(c.rgb, vec3<f32>(0.0))), c.a);
}
@fragment
fn fs_out(in: VOut) -> @location(0) vec4<f32> {
    let c = textureLoad(src, vec2<i32>(in.pos.xy), 0);
    return vec4<f32>(c.rgb * c.rgb, c.a);
}
"#;

pub struct WorldGuiPass {
    tone_buf: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    depth_bgl: wgpu::BindGroupLayout,
    tex_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Repeating, for beams' and trails' tiled textures (round 25).
    sampler_wrap: wgpu::Sampler,
    /// Effects' textures with mip levels (Studio samples particle, beam and
    /// trail images mipmapped: far or small particles are the image's
    /// average, not a sparse sample of its texels), made from egui's copy
    /// on first use. Keyed by egui's texture; the copy, its view, and the
    /// frame each was last used (and, for the font atlas, refilled).
    mips: std::collections::HashMap<wgpu::Texture, (wgpu::Texture, wgpu::TextureView, u64)>,
    mip_pipeline: wgpu::RenderPipeline,
    mip_bgl: wgpu::BindGroupLayout,
    mip_sampler: wgpu::Sampler,
    sampler_mip: wgpu::Sampler,
    sampler_mip_wrap: wgpu::Sampler,
    frame: u64,
    /// Effects (`effects`): their pipeline into the square-root buffer,
    /// the fill / square-back passes, and the buffer sized to the view.
    fx_pipeline: wgpu::RenderPipeline,
    fx_bgl: wgpu::BindGroupLayout,
    fx_in: wgpu::RenderPipeline,
    fx_out: wgpu::RenderPipeline,
    fx_tex: Option<([u32; 2], wgpu::TextureView)>,
}

/// The scene's HDR format (render.rs HDR_FORMAT) and the effects' buffer.
const FX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// `WorldGuiPass::upload`'s result.
struct Upload {
    depth_bg: wgpu::BindGroup,
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    draws: Vec<(wgpu::BindGroup, u32, u32)>,
}

/// Downsamples one mip level into the next (each texel the average of the
/// 2 × 2 under it: a bilinear tap at their shared corner).
const MIP_SHADER: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VOut;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(src, samp, in.uv, 0.0);
}
"#;

impl WorldGuiPass {
    pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world gui"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let depth_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world gui depth"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: true,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world gui texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world gui"),
            bind_group_layouts: &[Some(&depth_bgl), Some(&tex_bgl)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x2, 3 => Unorm8x4, 4 => Float32x3, 5 => Float32, 6 => Float32, 7 => Float32, 8 => Float32x2, 9 => Float32, 10 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world gui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GuiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target,
                    // egui's premultiplied blending.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("world gui"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sampler_wrap = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("world gui wrap"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let tone_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world gui tone"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mip_sampler_desc = |wrap: bool| wgpu::SamplerDescriptor {
            label: Some("world gui effects"),
            address_mode_u: if wrap { wgpu::AddressMode::Repeat } else { wgpu::AddressMode::ClampToEdge },
            address_mode_v: if wrap { wgpu::AddressMode::Repeat } else { wgpu::AddressMode::ClampToEdge },
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        };
        let sampler_mip = device.create_sampler(&mip_sampler_desc(false));
        let sampler_mip_wrap = device.create_sampler(&mip_sampler_desc(true));
        let mip_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("world gui mip build"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mip_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world gui mips"),
            source: wgpu::ShaderSource::Wgsl(MIP_SHADER.into()),
        });
        let mip_bgl = tex_bgl.clone();
        let mip_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world gui mips"),
            bind_group_layouts: &[Some(&mip_bgl)],
            immediate_size: 0,
        });
        let mip_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world gui mips"),
            layout: Some(&mip_layout),
            vertex: wgpu::VertexState {
                module: &mip_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mip_module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let fx_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world gui effects"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GuiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_fx"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FX_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let fx_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world gui effects buffer"),
            source: wgpu::ShaderSource::Wgsl(FX_SHADER.into()),
        });
        let fx_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world gui effects buffer"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let fx_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world gui effects buffer"),
            bind_group_layouts: &[Some(&fx_bgl)],
            immediate_size: 0,
        });
        let full = |label: &str, entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&fx_layout),
                vertex: wgpu::VertexState {
                    module: &fx_module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &fx_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: FX_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let fx_in = full("world gui effects in", "fs_in");
        let fx_out = full("world gui effects out", "fs_out");
        WorldGuiPass {
            fx_pipeline,
            fx_bgl,
            fx_in,
            fx_out,
            fx_tex: None,
            tone_buf,
            pipeline,
            depth_bgl,
            tex_bgl,
            sampler,
            sampler_wrap,
            mips: Default::default(),
            mip_pipeline,
            mip_bgl,
            mip_sampler,
            sampler_mip,
            sampler_mip_wrap,
            frame: 0,
        }
    }

    /// A mipmapped copy of an egui texture (egui's own have one level),
    /// made once: level 0 drawn from egui's, each next level downsampled
    /// from the one above. Gamma-space premultiplied texels averaged, as
    /// the effects' fragment shader mixes them.
    ///
    /// `live`: egui's texture changes in place (the font atlas gains glyphs
    /// as new text sizes are laid out), so the copy is refilled once a
    /// frame, not kept from the first frame: a SurfaceGui whose text size
    /// was first laid out after the copy was made drew nothing (Place A's
    /// Liquid Divinium counter).
    fn mipped(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture, live: bool) -> wgpu::TextureView {
        let frame = self.frame;
        let reuse = match self.mips.get_mut(tex) {
            Some((_, view, used)) if !live || *used == frame => {
                *used = frame;
                return view.clone();
            }
            Some((copy, view, used)) => {
                *used = frame;
                Some((copy.clone(), view.clone()))
            }
            None => None,
        };
        let size = tex.size();
        let levels = 32 - size.width.max(size.height).max(1).leading_zeros();
        let mipped = if let Some((copy, _)) = &reuse { copy.clone() } else { device.create_texture(&wgpu::TextureDescriptor {
            label: Some("world gui effect texture"),
            size: wgpu::Extent3d { width: size.width, height: size.height, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        }) };
        let level_view = |l: u32| {
            mipped.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: l,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("world gui mips") });
        for l in 0..levels {
            let src = if l == 0 { tex.create_view(&Default::default()) } else { level_view(l - 1) };
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("world gui mips"),
                layout: &self.mip_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.mip_sampler) },
                ],
            });
            let dst = level_view(l);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world gui mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.mip_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([enc.finish()]);
        if let Some((_, view)) = reuse {
            return view;
        }
        let view = mipped.create_view(&Default::default());
        self.mips.insert(tex.clone(), (mipped, view.clone(), frame));
        view
    }

    /// The batches that are effects (`fx`) or not, uploaded: the depth /
    /// tone bind group, one vertex and index buffer, and per draw its
    /// texture's bind group and index range.
    #[allow(clippy::too_many_arguments)]
    fn upload(
        &mut self,
        rs: &egui_wgpu::RenderState,
        depth: &wgpu::TextureView,
        batches: &[GuiBatch],
        scene: bool,
        tone_curve: Option<[f32; 5]>,
        fog: [f32; 4],
        fog_rgb: [f32; 3],
    ) -> Option<Upload> {
        // (effect vertices carry a clip w; GUIs' are 0)
        let is_fx = |b: &GuiBatch| b.vertices.first().is_some_and(|v| v.clip_w != 0.0);
        if batches.iter().all(|b| b.indices.is_empty() || (!scene && is_fx(b))) {
            return None;
        }
        let device = &rs.device;
        let t = tone_curve.unwrap_or([0.0; 5]);
        let u: [f32; 16] = [
            t[0], t[1], t[2], t[3], t[4], if tone_curve.is_some() { 1.0 } else { 0.0 }, crate::mech::NEAR, crate::mech::FX_LIGHT_CLAMP,
            fog[0], fog[1], fog[2], fog[3], fog_rgb[0], fog_rgb[1], fog_rgb[2], 0.0,
        ];
        rs.queue.write_buffer(&self.tone_buf, 0, bytemuck::cast_slice(&u));
        let depth_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world gui depth"),
            layout: &self.depth_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 1, resource: self.tone_buf.as_entire_binding() },
            ],
        });
        // One buffer pair; a bind group per egui texture (and sampler).
        self.frame += 1;
        // (effects' textures unused for a while are let go)
        let frame = self.frame;
        self.mips.retain(|_, (_, _, used)| frame - *used < 600);
        let egui = rs.renderer.read();
        let mut groups: std::collections::HashMap<(egui::TextureId, bool), Option<wgpu::BindGroup>> = Default::default();
        let mut vertices: Vec<GuiVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws: Vec<(wgpu::BindGroup, u32, u32)> = Vec::new();
        for b in batches {
            let Some(id) = b.texture else { continue };
            if b.indices.is_empty() || (!scene && is_fx(b)) {
                continue;
            }
            let key = (id, b.wrap);
            if !groups.contains_key(&key) {
                let bg = egui.texture(&id).and_then(|t| t.texture.clone()).map(|tex| {
                    let (view, sampler) = if scene && tex.size().width.max(tex.size().height) > 1 {
                        (self.mipped(device, &rs.queue, &tex, id == egui::TextureId::default()), if b.wrap { &self.sampler_mip_wrap } else { &self.sampler_mip })
                    } else {
                        (tex.create_view(&Default::default()), if b.wrap { &self.sampler_wrap } else { &self.sampler })
                    };
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("world gui texture"),
                        layout: &self.tex_bgl,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                        ],
                    })
                });
                groups.insert(key, bg);
            }
            let Some(Some(bg)) = groups.get(&key) else { continue };
            let base = vertices.len() as u32;
            let start = indices.len() as u32;
            vertices.extend_from_slice(&b.vertices);
            indices.extend(b.indices.iter().map(|i| base + i));
            draws.push((bg.clone(), start, indices.len() as u32));
        }
        drop(egui);
        if draws.is_empty() {
            return None;
        }
        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("world gui vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("world gui indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Some(Upload { depth_bg, vb, ib, draws })
    }

    fn record(&self, pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline, up: &Upload) {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &up.depth_bg, &[]);
        pass.set_vertex_buffer(0, up.vb.slice(..));
        pass.set_index_buffer(up.ib.slice(..), wgpu::IndexFormat::Uint32);
        for (bg, start, end) in &up.draws {
            crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::UI3D, 1, ((end - start) / 3) as u64);
            pass.set_bind_group(1, bg, &[]);
            pass.draw_indexed(*start..*end, 0, 0..1);
        }
    }

    /// Draws the world GUIs among `batches` onto `target` (the shown view),
    /// tested against `depth`. Effects go through `effects`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        rs: &egui_wgpu::RenderState,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        batches: &[GuiBatch],
        tone_curve: Option<[f32; 5]>,
        fog: [f32; 4],
        fog_rgb: [f32; 3],
    ) {
        let Some(up) = self.upload(rs, depth, batches, false, tone_curve, fog, fog_rgb) else { return };
        let mut enc = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("world gui") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world gui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.record(&mut pass, &self.pipeline, &up);
        }
        rs.queue.submit([enc.finish()]);
    }

    /// Draws world GUIs and effects into the scene's HDR image `hdr`
    /// (linear light, before postprocessing), blended in the
    /// square root of linear light as Studio blends them: the image is
    /// copied into that space, the effects drawn over it far to near, and
    /// squared back. False when there were none (the view needn't be
    /// finished again).
    #[allow(clippy::too_many_arguments)]
    pub fn effects(
        &mut self,
        rs: &egui_wgpu::RenderState,
        hdr: &wgpu::TextureView,
        size: [u32; 2],
        depth: &wgpu::TextureView,
        batches: &[GuiBatch],
        tone_curve: Option<[f32; 5]>,
        fog: [f32; 4],
        fog_rgb: [f32; 3],
    ) -> bool {
        let Some(up) = self.upload(rs, depth, batches, true, tone_curve, fog, fog_rgb) else { return false };
        let device = &rs.device;
        if self.fx_tex.as_ref().is_none_or(|(s, _)| *s != size) {
            let view = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("world gui effects buffer"),
                    size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FX_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            self.fx_tex = Some((size, view));
        }
        let Some((_, buf)) = self.fx_tex.as_ref() else { return false };
        let source = |v: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("world gui effects buffer"),
                layout: &self.fx_bgl,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v) }],
            })
        };
        let (from_hdr, from_buf) = (source(hdr), source(buf));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("world gui effects") });
        let mut step = |label: &str, view: &wgpu::TextureView, f: &mut dyn FnMut(&mut wgpu::RenderPass<'_>)| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            f(&mut pass);
        };
        step("effects in", buf, &mut |p| {
            p.set_pipeline(&self.fx_in);
            p.set_bind_group(0, &from_hdr, &[]);
            p.draw(0..3, 0..1);
        });
        step("effects", buf, &mut |p| self.record(p, &self.fx_pipeline, &up));
        step("effects out", hdr, &mut |p| {
            p.set_pipeline(&self.fx_out);
            p.set_bind_group(0, &from_buf, &[]);
            p.draw(0..3, 0..1);
        });
        drop(step);
        rs.queue.submit([enc.finish()]);
        true
    }
}
