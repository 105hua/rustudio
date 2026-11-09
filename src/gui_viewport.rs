//! Offscreen ViewportFrame pass on the editor's wgpu device.
use egui_wgpu::{wgpu, RenderState};
use wgpu::util::DeviceExt;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub clip: [f32; 4],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}
pub struct Target {
    pub size: [u32; 2],
    pub id: egui::TextureId,
    /// When it was last drawn.
    pub at: std::time::Instant,
    /// When it was last shown (targets unused for a while are freed).
    pub used: std::time::Instant,
    /// Needs drawing again: something it shows changed (the document's
    /// journal, read in `Gui::sync_roots`), or a mesh / texture it wanted
    /// was still loading. A static frame is drawn once, not 15 times a
    /// second: a tile of gun silhouettes was ~7 ms a draw (Place A's kits page).
    pub dirty: bool,
    /// The camera and parts it drew (a camera outside the frame, or a part
    /// taken out of it, dirties it too).
    pub cam: Option<rbx_dom_weak::types::Ref>,
    pub parts: std::collections::HashSet<rbx_dom_weak::types::Ref>,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
}
pub struct Gpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
}
impl Gpu {
    pub fn new(d: &wgpu::Device) -> Self {
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ViewportFrame texture"),
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
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("ViewportFrame"), source: wgpu::ShaderSource::Wgsl(r#"
struct V { @builtin(position) p: vec4f, @location(0) uv: vec2f, @location(1) c: vec4f }
@vertex fn vs(@location(0) p: vec4f, @location(1) uv: vec2f, @location(2) c: vec4f) -> V { return V(p, uv, c); }
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@fragment fn fs(v: V) -> @location(0) vec4f {
 let t = textureSample(tex,samp,v.uv); let a=t.a*v.c.a;
 if a < 0.001 { discard; }
 return vec4f(t.rgb*v.c.rgb*a,a);
}
"#.into()) });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ViewportFrame"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 40,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x2,2=>Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let white = d
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Self {
            pipeline,
            layout,
            sampler: d.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            white,
        }
    }
    pub fn target(&self, rs: &RenderState, size: [u32; 2]) -> Target {
        let make = |format, usage| {
            rs.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("ViewportFrame"),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let color = make(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth = make(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let id = rs.renderer.write().register_native_texture(
            &rs.device,
            &color,
            wgpu::FilterMode::Linear,
        );
        Target {
            size,
            id,
            at: std::time::Instant::now(),
            used: std::time::Instant::now(),
            dirty: true,
            cam: None,
            parts: Default::default(),
            color,
            depth,
        }
    }
    pub fn render(
        &self,
        rs: &RenderState,
        target: &mut Target,
        vertices: &[Vertex],
        batches: &[(std::ops::Range<u32>, egui::TextureId)],
    ) {
        let d = &rs.device;
        let mut encoder = d.create_command_encoder(&Default::default());
        // The untextured primitive fallback is a one-pixel white texture.
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.white,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        let renderer = rs.renderer.read();
        let groups: Vec<_> = batches
            .iter()
            .map(|(_, id)| {
                let view = renderer
                    .texture(id)
                    .and_then(|t| t.texture.as_ref())
                    .map(|t| t.create_view(&Default::default()));
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                if *id == egui::TextureId::default() {
                                    &self.white
                                } else {
                                    view.as_ref().unwrap_or(&self.white)
                                },
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            })
            .collect();
        let data = if vertices.is_empty() {
            &[Vertex {
                clip: [0.0; 4],
                uv: [0.0; 2],
                color: [0.0; 4],
            }][..]
        } else {
            vertices
        };
        let buffer = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::VERTEX,
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ViewportFrame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, buffer.slice(..));
            for ((range, _), group) in batches.iter().zip(&groups) {
                pass.set_bind_group(0, group, &[]);
                pass.draw(range.clone(), 0..1);
            }
        }
        rs.queue.submit([encoder.finish()]);
        target.at = std::time::Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_gpu_pipeline() {
        // Exercise the real WGSL and depth/texture pipeline on a headless adapter.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("headless wgpu adapter");
        let (device, _) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
        let _gpu = Gpu::new(&device);
    }
}
