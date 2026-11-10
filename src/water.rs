//! Terrain water drawn like Studio's high-quality water shader (the
//! SmoothWater FS with GBufferColor / GBufferDepth; see
//! docs/MECHANICS.md "Round 24: water"): the scene behind the surface shows
//! through, bent by the ripples and blended toward the lit WaterColor by
//! depth; the sky reflects by the Fresnel term; the sun glints off the
//! animated normal maps (PlatformContent textures/water/normal_01..25.dds);
//! two crossing sine waves move the surface.
//!
//! The pass runs after the opaque scene, reading a copy of its colour and
//! its (multisampled) depth, so it tests depth itself and has no depth
//! attachment. See-through parts are drawn after it.

use egui_wgpu::wgpu;
use wgpu::util::DeviceExt;

/// The pass's uniforms (`WaterParams` in shader.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterRaw {
    /// x wave number (rad / stud), y phase (rad), z amplitude (studs), w frame blend.
    pub wave: [f32; 4],
    /// x frame a, y frame b (array layers), z depth scale (/ stud), w depth offset.
    pub frames: [f32; 4],
    /// xy target size (pixels), z wave fade (/ stud), w the near plane.
    pub view: [f32; 4],
}

/// What a frame's water needs from the place (Terrain and time).
#[derive(Clone, Copy, Debug, Default)]
pub struct WaterLook {
    /// Seconds since the view started (animation).
    pub time: f32,
    pub wave_size: f32,
    pub wave_speed: f32,
    pub transparency: f32,
}

pub const FRAMES: u32 = 25;
const SIZE: u32 = 256;

pub struct WaterPass {
    pub pipeline: wgpu::RenderPipeline,
    pub bgl: wgpu::BindGroupLayout,
    pub buf: wgpu::Buffer,
    normals: wgpu::TextureView,
    sampler: wgpu::Sampler,
    layers: u32,
}

impl WaterPass {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shader: &wgpu::ShaderModule,
        globals: &wgpu::BindGroupLayout,
        textures: &wgpu::BindGroupLayout,
        buffers: &[Option<wgpu::VertexBufferLayout>],
        hdr: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let frag = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
            ty,
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water"),
            entries: &[
                frag(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                frag(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                frag(2, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                frag(
                    3,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                frag(
                    4,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: samples > 1,
                    },
                ),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water"),
            bind_group_layouts: &[Some(globals), Some(textures), Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_water"),
                compilation_options: Default::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water"),
            size: std::mem::size_of::<WaterRaw>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (normals, layers) = load_normals(device, queue);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("water normals"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Mipmapped: far ripples average out instead of shimmering.
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: crate::mech::TEXTURE_ANISOTROPY,
            ..Default::default()
        });
        WaterPass {
            pipeline,
            bgl,
            buf,
            normals,
            sampler,
            layers,
        }
    }

    /// The bind group for one render target (its scene copy and depth).
    pub fn bind(
        &self,
        device: &wgpu::Device,
        scene: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        use wgpu::BindingResource as B;
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: B::TextureView(&self.normals) },
                wgpu::BindGroupEntry { binding: 2, resource: B::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: B::TextureView(scene) },
                wgpu::BindGroupEntry { binding: 4, resource: B::TextureView(depth) },
            ],
        })
    }

    /// This frame's uniforms.
    pub fn uniforms(&self, look: &WaterLook, size: [u32; 2]) -> WaterRaw {
        // The normal maps play through their frames (an estimate of Studio's
        // rate: WaterWaveSpeed 10 → 8 frames / s), blending between two.
        let t = look.time * look.wave_speed.max(0.0) * crate::mech::WATER_FRAMES_PER_SPEED;
        let n = self.layers.max(1) as f32;
        let a = t.floor().rem_euclid(n);
        let b = (a + 1.0).rem_euclid(n);
        // Waves (SmoothWater VS): y += (sin((z − x)k − φ) + sin((z + x)k + φ)) × amplitude.
        let phase = look.time * look.wave_speed * crate::mech::WATER_WAVE_PHASE_PER_SPEED;
        // Depth blend toward the water colour: clear water shows deeper.
        let clear = look.transparency.clamp(0.0, 1.0);
        let scale = crate::mech::WATER_DEPTH_SCALE * (1.0 - clear) / (clear + crate::mech::WATER_DEPTH_SOFT);
        WaterRaw {
            wave: [
                crate::mech::WATER_WAVE_NUMBER,
                phase,
                look.wave_size.max(0.0) * crate::mech::WATER_WAVE_AMPLITUDE,
                t.fract(),
            ],
            frames: [a, b, scale, 0.0],
            view: [size[0] as f32, size[1] as f32, crate::mech::WATER_WAVE_FADE, crate::mech::NEAR],
        }
    }
}

/// The 25 normal map frames as one mipmapped array (a flat map if missing).
fn load_normals(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::TextureView, u32) {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    for i in 1..=FRAMES {
        let id = format!("rbxasset://textures/water/normal_{i:02}.dds");
        let img = crate::assets::resolve(&id, None)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| crate::texture::decode(&b).ok())
            .filter(|img| img.width == SIZE && img.height == SIZE);
        match img {
            Some(img) => frames.push(img.pixels),
            None => break,
        }
    }
    if frames.len() != FRAMES as usize {
        // Flat: x, y at 0.5 (.wy), smooth.
        frames = vec![[0u8, 128, 128, 128].repeat((SIZE * SIZE) as usize)];
    }
    let layers = frames.len() as u32;
    let mips = 32 - SIZE.leading_zeros();
    let mut data = Vec::new();
    for f in &frames {
        let mut level = f.clone();
        let mut s = SIZE;
        for _ in 0..mips {
            data.extend_from_slice(&level);
            if s == 1 {
                break;
            }
            level = halve(&level, s);
            s /= 2;
        }
    }
    let tex = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("water normals"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: layers,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (view, layers)
}

/// A 2× box-filtered mip of a square RGBA8 image (linear: normal data).
fn halve(src: &[u8], s: u32) -> Vec<u8> {
    let h = s / 2;
    let mut out = vec![0u8; (h * h * 4) as usize];
    for y in 0..h {
        for x in 0..h {
            for c in 0..4 {
                let at = |xx: u32, yy: u32| src[((yy * s + xx) * 4 + c) as usize] as u32;
                let v = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                out[((y * h + x) * 4 + c) as usize] = ((v + 2) / 4) as u8;
            }
        }
    }
    out
}
