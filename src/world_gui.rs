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
    /// What alpha is scaled by (1 for GUIs).
    pub alpha_mul: f32,
}

/// How an effect is lit, per vertex / particle: x Brightness × (1 − w),
/// y w = √LightInfluence (0: unlit, Brightness alone), z the mode (0 unlit,
/// 1 lit, 2 lit with the sun's shadow per pixel: beams), w 1 − LightEmission
/// (alpha scale: additive toward 1 under premultiplied blending). The GPU
/// lights it from the light grid, the local lights and the sun
/// (`fx_light`, the same terms as effects::SceneLight::at).
pub type FxLook = [f32; 4];

/// A beam / trail / constraint strip vertex in world space (Studio's
/// TrailVSLightUnified takes world positions and lights at the pixel's
/// light-grid coordinate): the GPU projects and lights it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FxVertex {
    /// World position before the strip's ZOffset.
    pub pos: [f32; 3],
    /// Studs the row moves toward the camera (Beam.ZOffset, TrailVS).
    pub zoff: f32,
    /// Where the light is read: the row's centre (before ZOffset).
    pub at: [f32; 3],
    pub color: [u8; 4],
    pub uv: [f32; 2],
    pub look: FxLook,
    /// RUSTUDIO_FX_LIGHT_CHECK: the CPU's light (SceneLight), compared.
    pub check: [f32; 3],
    /// The glass in front (first, count in FxFrame::glass_idx).
    pub glass: [u32; 2],
    /// FaceCamera beams: the row's tangent and ± half width; the GPU puts
    /// the vertex at `pos` + normalize(tangent × (camera − pos)) × w
    /// (0: `pos` is the vertex).
    pub face: [f32; 4],
    /// Taken off the texture's v (Beam.TextureSpeed).
    pub scroll: f32,
}

/// One particle (Studio's ParticleCustomVS: the CPU writes the particle,
/// the vertex shader makes its quad): the four corners are centre ± e1 ±
/// e2 (size, squash, rotation and orientation already in e1 / e2).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FxParticle {
    pub centre: [f32; 3],
    /// ParticleEmitter.ZOffset: moves the depth only (ParticleCustomVS).
    pub zoffset: f32,
    pub e1: [f32; 3],
    /// Soft particles (ParticleCustomFS_Soft): 1 / the fade distance (the
    /// half size), 0 = hard.
    pub soft: f32,
    pub e2: [f32; 3],
    /// FlipbookBlendFrames: how far toward the next frame (ParticleCustomFS
    /// mixes the two frames' texels).
    pub blend: f32,
    /// The flipbook frame's and the next frame's top-left UV, and the cell
    /// size (0 when untextured: both UVs egui's white texel).
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub cell: [f32; 2],
    pub color: [u8; 4],
    pub look: FxLook,
    pub check: [f32; 3],
    /// The glass in front (first, count in FxFrame::glass_idx).
    pub glass: [u32; 2],
}

/// A see-through part effects behind it are dimmed by (effects::Glass):
/// its rotation's columns with the half size in w, and its position with
/// the opacity in w.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FxGlass {
    pub x: [f32; 4],
    pub y: [f32; 4],
    pub z: [f32; 4],
    pub pos: [f32; 4],
}

/// One draw of effects, far to near: a texture and a range of `FxFrame`'s
/// strip indices or particles.
#[derive(Clone, Copy, Debug)]
pub struct FxDraw {
    pub texture: egui::TextureId,
    pub wrap: bool,
    pub particles: bool,
    pub start: u32,
    pub end: u32,
}

/// The frame's particles, beams and trails (effects.rs), and what the GPU
/// needs to project and light them.
#[derive(Default)]
pub struct FxFrame {
    /// Reverse-Z view-projection the effects were laid out for.
    pub vp: [[f32; 4]; 4],
    /// Camera position; w 1 = RUSTUDIO_FX_LIGHT_CHECK.
    pub cam: [f32; 4],
    /// The camera's right (FaceCamera's fallback side).
    pub right: [f32; 4],
    /// effects::SceneLight: ambient, outdoor, sun, env (linear RGB).
    pub light: [[f32; 4]; 4],
    pub draws: Vec<FxDraw>,
    pub strips: Vec<FxVertex>,
    pub indices: Vec<u32>,
    pub particles: Vec<FxParticle>,
    /// The see-through parts in view, and per effect the ones in front of
    /// it (FxVertex / FxParticle::glass).
    pub glass: Vec<FxGlass>,
    pub glass_idx: Vec<u32>,
}

impl FxFrame {
    pub fn clear(&mut self) {
        self.glass.clear();
        self.glass_idx.clear();
        self.draws.clear();
        self.strips.clear();
        self.indices.clear();
        self.particles.clear();
    }

    /// Appends a strip's triangles, joining the last draw when it has the
    /// same texture.
    pub fn push_strip(&mut self, texture: egui::TextureId, wrap: bool, vertices: &[FxVertex], indices: &[u32]) {
        let base = self.strips.len() as u32;
        self.strips.extend_from_slice(vertices);
        let start = self.indices.len() as u32;
        self.indices.extend(indices.iter().map(|i| base + i));
        self.join(FxDraw { texture, wrap, particles: false, start, end: self.indices.len() as u32 });
    }

    /// Appends particles (one draw, joined like `push_strip`).
    pub fn push_particles(&mut self, texture: egui::TextureId, particles: &[FxParticle]) {
        let start = self.particles.len() as u32;
        self.particles.extend_from_slice(particles);
        self.join(FxDraw { texture, wrap: false, particles: true, start, end: self.particles.len() as u32 });
    }

    fn join(&mut self, d: FxDraw) {
        if d.start == d.end {
            return;
        }
        if let Some(l) = self.draws.last_mut() {
            if l.texture == d.texture && l.wrap == d.wrap && l.particles == d.particles && l.end == d.start {
                l.end = d.end;
                return;
            }
        }
        self.draws.push(d);
    }
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
    @location(11) beam_sun: vec3<f32>,
    @location(12) world: vec3<f32>,
    @location(13) beam_grid: f32,
};
// Studio's soft-particle depth texture holds view depth / 500
// (ParticleCustomFS_Soft: depth × 500; ParticleCustomVS_Soft: min(w − ZOffset,
// 500 − fade)).
const SOFT_DEPTH_MAX: f32 = 500.0;
// fog: the scene's (shader.wgsl g.fog: start, end or exp2 terms, mode);
// fog_rgb: its colour, linear; e_on: tone curve e, on, the near plane.
// casc / sphere / rect: the sun's shadow cascades (shader.wgsl g.sun_*);
// sun: x Realistic, y sunDist, z hybridLerpSlope, w shadows on.
// Effects (FxFrame): vp the reverse-Z view-projection, cam the camera (w 1:
// light check), amb (w: local lights), outdoor, lamp (the sun), env:
// effects::SceneLight's terms.
struct Tone { curve: vec4<f32>, e_on: vec4<f32>, fog: vec4<f32>, fog_rgb: vec4<f32>, casc: array<mat4x4<f32>, 4>, sphere: array<vec4<f32>, 4>, rect: array<vec4<f32>, 4>, grid: vec4<f32>, sun: vec4<f32>,
    vp: mat4x4<f32>, cam: vec4<f32>, amb: vec4<f32>, outdoor: vec4<f32>, lamp: vec4<f32>, env: vec4<f32>, right: vec4<f32> };
// render.rs LightRaw / shader.wgsl Light.
struct Light { pos_range: vec4<f32>, color: vec4<f32>, dir: vec4<f32>, extra: vec4<f32>, surface_u: vec4<f32>, surface_v: vec4<f32> };
@group(0) @binding(0) var scene_depth: texture_depth_multisampled_2d;
@group(0) @binding(1) var<uniform> tone: Tone;
@group(0) @binding(2) var sun_atlas: texture_2d<f32>;
@group(0) @binding(3) var sun_grid: texture_3d<f32>;
@group(0) @binding(4) var sun_sampler: sampler;
@group(0) @binding(5) var<storage, read> lights: array<Light>;
// effects::Glass (FxGlass) and each effect's list of the ones in front.
struct Glass { x: vec4<f32>, y: vec4<f32>, z: vec4<f32>, pos: vec4<f32> };
@group(0) @binding(6) var<storage, read> glass: array<Glass>;
@group(0) @binding(7) var<storage, read> glass_idx: array<u32>;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    o.pos = vec4<f32>(v.ndc, 0.5, 1.0);
    o.fx = 0.0;
    o.alpha_mul = v.alpha_mul;
    o.uv2 = v.uv;
    o.blend = 0.0;
    o.soft = vec2<f32>(0.0);
    o.vw = 0.0;
    o.uv = v.uv;
    o.color = v.color;
    o.depth = v.depth;
    o.light = v.light;
    o.beam_sun = vec3<f32>(0.0);
    o.beam_grid = 0.0;
    o.world = vec3<f32>(0.0);
    o.dist = v.dist;
    return o;
}

// ------------------------------------------------------------ effect light
// effects::SceneLight on the GPU (Studio lights effects on the GPU:
// ParticleLightingVS / FS into the "Emitter Lighting Atlas", TrailFSLight
// at the pixel). Same terms, same order as the CPU reference.

// The light grid at p, trilinear (SceneLight::grid): x skylight, y sun;
// 1 beyond its border or before the first grid.
fn fx_grid(p: vec3<f32>) -> vec2<f32> {
    if (tone.grid.w <= 0.0) { return vec2<f32>(1.0); }
    let q = (p / FX_VOXEL - tone.grid.xyz).xzy / vec3<f32>(textureDimensions(sun_grid));
    if (any(abs(q - vec3<f32>(0.5)) >= vec3<f32>(FX_GRID_BORDER))) { return vec2<f32>(1.0); }
    return textureSampleLevel(sun_grid, sun_sampler, q, 0.0).rg;
}

// One local light without N·L (SceneLight::local).
fn fx_local(l: Light, p: vec3<f32>) -> vec3<f32> {
    let range = l.pos_range.w;
    let to = l.pos_range.xyz - p;
    let d = length(to);
    let reach = range + FX_LOCAL_REACH;
    if (range <= 0.0 || d >= reach) { return vec3<f32>(0.0); }
    let dir = to / max(d, 1e-4);
    var k = FX_LOCAL_SLOPE * (reach - d);
    if (l.color.w > 0.5) {
        let c = dot(-dir, l.dir.xyz);
        let cone = clamp((c - l.dir.w) / max(1.0 - l.dir.w, 1e-3), 0.0, 1.0);
        k *= cone * cone;
        if (l.color.w > 1.5 && c < 0.0) { k = 0.0; }
    }
    return l.color.rgb / FX_LIGHT_INTENSITY * k;
}

// The light at p (SceneLight::at): min(ambient + sky + local, cap) per
// channel (ParticleLightingFS), then the sun × the voxel sun.
fn fx_scene(p: vec3<f32>, g: vec2<f32>) -> vec3<f32> {
    var c = tone.amb.rgb + tone.outdoor.rgb * g.x + tone.env.rgb * g.x;
    let n = u32(tone.amb.w);
    for (var i = 0u; i < n; i++) {
        c += fx_local(lights[i], p);
    }
    return min(c, vec3<f32>(FX_LIGHT_CAP)) + tone.lamp.rgb * g.y;
}

// Look::light: Brightness × (1 − w) + the scene's light × w.
fn fx_look(look: vec4<f32>, lit: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(look.x) + lit * look.y;
}

// RUSTUDIO_FX_LIGHT_CHECK: the CPU's light against the GPU's.
fn fx_checked(o: ptr<function, VOut>, gpu: vec3<f32>, cpu: vec3<f32>) {
    if (tone.cam.w > 0.5 && any(abs(gpu - cpu) > vec3<f32>(2e-3) + abs(cpu) * 2e-3)) {
        (*o).color = vec4<f32>(1.0, 0.0, 1.0, 1.0);
        (*o).light = vec3<f32>(1.0);
        (*o).beam_sun = vec3<f32>(0.0);
        (*o).uv = vec2<f32>(FX_WHITE_UV);
        (*o).uv2 = vec2<f32>(FX_WHITE_UV);
    }
}

// Whether the segment from o (length len along unit d) enters the box
// (effects::Glass::hit).
fn fx_hit(g: Glass, o: vec3<f32>, d: vec3<f32>, len: f32) -> bool {
    let rel = o - g.pos.xyz;
    let lo = vec3<f32>(dot(g.x.xyz, rel), dot(g.y.xyz, rel), dot(g.z.xyz, rel));
    let ld = vec3<f32>(dot(g.x.xyz, d), dot(g.y.xyz, d), dot(g.z.xyz, d));
    let half = vec3<f32>(g.x.w, g.y.w, g.z.w);
    var t0 = 0.0;
    var t1 = len;
    for (var i = 0; i < 3; i++) {
        if (abs(ld[i]) < 1e-6) {
            if (abs(lo[i]) > half[i]) { return false; }
        } else {
            let a = (-half[i] - lo[i]) / ld[i];
            let b = (half[i] - lo[i]) / ld[i];
            t0 = max(t0, min(a, b));
            t1 = min(t1, max(a, b));
            if (t0 > t1) { return false; }
        }
    }
    return true;
}

// How much of an effect at p shows through the glass in front of it
// (GlassSet::transmit; Studio sorts see-through parts and effects together).
fn fx_transmit(p: vec3<f32>, list: vec2<u32>) -> f32 {
    var t = 1.0;
    if (list.y == 0u) { return t; }
    let seg = p - tone.cam.xyz;
    let len = length(seg);
    let d = seg / max(len, 1e-6);
    for (var i = 0u; i < list.y; i++) {
        let g = glass[glass_idx[list.x + i]];
        if (fx_hit(g, tone.cam.xyz, d, len)) { t *= 1.0 - g.pos.w; }
    }
    return t;
}

fn fx_clip(p: vec3<f32>) -> vec4<f32> {
    var c = tone.vp * vec4<f32>(p, 1.0);
    if (c.w == 0.0) { c.w = -1e-6; }
    return c;
}

fn toward_cam(p: vec3<f32>) -> vec3<f32> {
    let d = tone.cam.xyz - p;
    let l = length(d);
    return select(vec3<f32>(0.0), d / l, l > 0.0);
}

struct StripIn {
    @location(0) pos: vec3<f32>,
    @location(1) zoff: f32,
    @location(2) at: vec3<f32>,
    @location(3) color: vec4<f32>,
    @location(4) uv: vec2<f32>,
    @location(5) look: vec4<f32>,
    @location(6) check: vec3<f32>,
    @location(7) glass: vec2<u32>,
    @location(8) face: vec4<f32>,
    @location(9) scroll: f32,
};

// Beams, trails and constraints (TrailVSLightUnified): world position, the
// row moved ZOffset toward the camera, lit at the row's centre.
@vertex
fn vs_strip(v: StripIn) -> VOut {
    var o: VOut;
    var p = v.pos;
    var at = v.at;
    if (v.face.w != 0.0) {
        // FaceCamera: across tangent × (camera − p), else the camera's right
        // (glam normalize_or).
        let c = cross(v.face.xyz, tone.cam.xyz - p);
        let rcp = 1.0 / length(c);
        let side = select(tone.right.xyz, c * rcp, rcp > 0.0 && rcp < 3.0e38);
        p += side * v.face.w;
    }
    if (v.zoff != 0.0) {
        let s = toward_cam(at) * v.zoff;
        p += s;
        at += s;
    }
    let c = fx_clip(p);
    o.pos = c;
    o.depth = c.z;
    o.vw = c.w;
    o.fx = 1.0;
    o.alpha_mul = v.look.w;
    o.uv = vec2<f32>(v.uv.x, v.uv.y - v.scroll);
    o.uv2 = o.uv;
    o.blend = 0.0;
    o.soft = vec2<f32>(0.0);
    let t = fx_transmit(at, v.glass);
    o.color = v.color * t;
    o.dist = length(p - tone.cam.xyz);
    o.world = p;
    o.beam_sun = vec3<f32>(0.0);
    o.beam_grid = 0.0;
    if (v.look.y > 0.0) {
        let g = fx_grid(at);
        o.light = fx_look(v.look, fx_scene(at, g));
        if (v.look.z > 1.5) {
            // TrailFSLight: Lamp0 × the sun's visibility per pixel (beam_light).
            o.beam_sun = tone.lamp.rgb * v.look.y;
            o.beam_grid = g.y;
        }
    } else {
        o.light = vec3<f32>(v.look.x);
    }
    fx_checked(&o, o.light * t, v.check);
    return o;
}

struct ParticleIn {
    @location(0) centre: vec3<f32>,
    @location(1) zoffset: f32,
    @location(2) e1: vec3<f32>,
    @location(3) soft: f32,
    @location(4) e2: vec3<f32>,
    @location(5) blend: f32,
    @location(6) uv0: vec2<f32>,
    @location(7) uv1: vec2<f32>,
    @location(8) cell: vec2<f32>,
    @location(9) color: vec4<f32>,
    @location(11) look: vec4<f32>,
    @location(12) check: vec3<f32>,
    @location(13) glass: vec2<u32>,
};

// Particles (ParticleCustomVS): the quad made from the particle, six
// vertices per instance; ZOffset moves the depth only; lit once, at the
// centre FX_LIGHT_TOWARD_CAMERA toward the camera (ParticleLightingVS).
@vertex
fn vs_particle(v: ParticleIn, @builtin(vertex_index) vi: u32) -> VOut {
    var o: VOut;
    var corners = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let k = corners[vi];
    let cx = select(-1.0, 1.0, k == 1u || k == 2u);
    let cy = select(-1.0, 1.0, k >= 2u);
    let p = v.centre + (v.e1 * cx + v.e2 * cy);
    let c = fx_clip(p);
    var z = c.z;
    if (v.zoffset != 0.0) {
        let q = p + toward_cam(p) * v.zoffset;
        let cz = tone.vp * vec4<f32>(q, 1.0);
        if (abs(cz.w) > 1e-6) { z = cz.z * c.w / cz.w; }
    }
    o.pos = vec4<f32>(c.xy, z, c.w);
    o.depth = z;
    o.vw = c.w;
    o.fx = 1.0;
    o.alpha_mul = v.look.w;
    let corner = vec2<f32>((cx + 1.0) * 0.5, 1.0 - (cy + 1.0) * 0.5) * v.cell;
    o.uv = v.uv0 + corner;
    o.uv2 = v.uv1 + corner;
    o.blend = v.blend;
    o.soft = vec2<f32>(v.soft, select(0.0, v.zoffset, v.soft > 0.0));
    let t = fx_transmit(v.centre, v.glass);
    o.color = v.color * t;
    o.dist = length(p - tone.cam.xyz);
    o.world = p;
    o.beam_sun = vec3<f32>(0.0);
    o.beam_grid = 0.0;
    if (v.look.y > 0.0) {
        let at = v.centre + toward_cam(v.centre) * FX_LIGHT_TOWARD_CAMERA;
        o.light = fx_look(v.look, fx_scene(at, fx_grid(at)));
    } else {
        o.light = vec3<f32>(v.look.x);
    }
    fx_checked(&o, o.light * t, v.check);
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
            // As Studio's lit GUI shader: fogged like the
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
// As Studio's lit trail shader: Lamp0 alone is cut by the
// sun's visibility, the parts' own (shader.wgsl sun_shadow): the shadow
// cascade whose sphere holds the pixel, its EVSM moments tested with
// Chebyshev on both warps, handed to the voxel sun (light grid .y) past
// sunDist. Skylight, local light and the effect's own parameters stay.
fn beam_light(in: VOut) -> vec3<f32> {
    if (!any(in.beam_sun > vec3<f32>(0.0))) { return in.light; }
    var visible = -1.0;
    if (tone.sun.w > 0.5) {
        let d0 = in.world - tone.sphere[0].xyz;
        let d1 = in.world - tone.sphere[1].xyz;
        let d2 = in.world - tone.sphere[2].xyz;
        var i = 3;
        if (dot(d0, d0) < tone.sphere[0].w) {
            i = 0;
        } else if (dot(d1, d1) < tone.sphere[1].w) {
            i = 1;
        } else if (dot(d2, d2) < tone.sphere[2].w) {
            i = 2;
        }
        let p = tone.casc[i] * vec4<f32>(in.world, 1.0);
        if (all(p.xy > vec2<f32>(0.0)) && all(p.xy < vec2<f32>(1.0))) {
            let r = tone.rect[i];
            let m = textureSampleLevel(sun_atlas, sun_sampler, r.xy + p.xy * r.zw, 0.0);
            let z = 2.0 * p.z - 1.0;
            let ep = exp(EVSM_POS_EXP * z);
            let en = -exp(-EVSM_NEG_EXP * z);
            let mv = vec2<f32>(EVSM_POS_EXP, EVSM_NEG_EXP) * EVSM_SHADOW_BIAS * vec2<f32>(ep, en);
            let vp = max(m.y - m.x * m.x, mv.x * mv.x);
            let dp = ep - m.x;
            let vn = max(m.w - m.z * m.z, mv.y * mv.y);
            let dn = en - m.z;
            let pp = select(clamp((vp / (vp + dp * dp) - EVSM_BLEED_CUT) / (1.0 - EVSM_BLEED_CUT), 0.0, 1.0), 1.0, ep <= m.x);
            let pn = select(clamp((vn / (vn + dn * dn) - EVSM_BLEED_CUT) / (1.0 - EVSM_BLEED_CUT), 0.0, 1.0), 1.0, en <= m.z);
            visible = min(pp, pn);
        }
    }
    var voxel = 1.0;
    let q = (in.world * 0.25 - tone.grid.xyz).xzy / vec3<f32>(textureDimensions(sun_grid));
    if (tone.grid.w > 0.0 && all(abs(q - vec3<f32>(0.5)) < vec3<f32>(0.4921875))) {
        voxel = textureSampleLevel(sun_grid, samp, q, 0.0).g;
    }
    var visibility = voxel;
    if (visible >= 0.0) {
        visibility = select(voxel * visible,
            mix(visible, voxel, clamp((in.dist - tone.sun.y) * tone.sun.z, 0.0, 1.0)), tone.sun.x > 0.5);
    }
    return max(in.light + in.beam_sun * (visibility - in.beam_grid), vec3<f32>(0.0));
}

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
    var lin = (g.rgb / g.a) * (g.rgb / g.a) * beam_light(in);
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

/// The sun's shadow cascades as the beams' shader reads them
/// (render.rs Globals sun_casc / sun_sphere / sun_rect).
#[derive(Clone, Copy, Default)]
pub struct SunCascades {
    pub local: [[[f32; 4]; 4]; 4],
    pub sphere: [[f32; 4]; 4],
    pub rect: [[f32; 4]; 4],
}

pub struct WorldGuiPass {
    tone_buf: wgpu::Buffer,
    /// The sun's shadow cascades (sun_shadow.rs): the EVSM atlas.
    pub sun_atlas: wgpu::TextureView,
    sun_sampler: wgpu::Sampler,
    pub sun_grid: wgpu::TextureView,
    pub lightgrid: [f32; 4],
    pub sun_params: [f32; 4],
    pub sun_cascades: SunCascades,
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
    /// Effects (FxFrame): beams / trails, and particles (instanced).
    strip_pipeline: wgpu::RenderPipeline,
    particle_pipeline: wgpu::RenderPipeline,
    /// The frame's local lights (render.rs lights_buf) and how many.
    pub lights: wgpu::Buffer,
    pub n_lights: u32,
    /// Vertex / index buffers kept from frame to frame, grown as needed
    /// (Studio's particle buffer is one 4 MB dynamic vertex buffer).
    bufs: [Option<wgpu::Buffer>; 5],
    /// The glass and its lists (FxFrame::glass / glass_idx), never empty.
    glass: [Option<wgpu::Buffer>; 2],
    /// Texture bind groups by egui id and wrap: the egui texture they were
    /// made for, and the frame each was last used.
    tex_groups: std::collections::HashMap<(egui::TextureId, bool), (wgpu::Texture, wgpu::BindGroup, u64)>,
    /// Untextured effects: one white texel.
    white: wgpu::BindGroup,
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
    /// GUI vertices / indices, strip vertices / indices, particles
    /// (`WorldGuiPass::bufs`).
    draws: Vec<(Kind, wgpu::BindGroup, u32, u32)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Gui,
    Strip,
    Particles,
}

/// The effects' shader constants (mech.rs).
fn fx_constants() -> String {
    let w = egui::epaint::WHITE_UV;
    format!(
        "const FX_VOXEL: f32 = {:?};\nconst FX_GRID_BORDER: f32 = {:?};\nconst FX_LOCAL_REACH: f32 = {:?};\nconst FX_LOCAL_SLOPE: f32 = {:?};\nconst FX_LIGHT_INTENSITY: f32 = {:?};\nconst FX_LIGHT_CAP: f32 = {:?};\nconst FX_LIGHT_TOWARD_CAMERA: f32 = {:?};\nconst FX_WHITE_UV = vec2<f32>({:?}, {:?});\n",
        crate::terrain::VOXEL,
        crate::mech::LIGHTGRID_BORDER,
        crate::mech::FX_LOCAL_REACH,
        crate::mech::FX_LOCAL_SLOPE,
        crate::mech::LIGHT_INTENSITY,
        crate::mech::FX_LIGHT_CAP,
        crate::mech::FX_LIGHT_TOWARD_CAMERA,
        w.x,
        w.y,
    )
}

/// Writes `data` into the kept buffer `slot`, growing it (to the next power
/// of two) when it doesn't fit.
fn fill(device: &wgpu::Device, queue: &wgpu::Queue, slot: &mut Option<wgpu::Buffer>, usage: wgpu::BufferUsages, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    let need = (data.len() as u64).next_power_of_two().max(4096);
    if slot.as_ref().is_none_or(|b| b.size() < data.len() as u64) {
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world gui / effects"),
            size: need,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    if let Some(b) = slot {
        queue.write_buffer(b, 0, data);
    }
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
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, target: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world gui"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "const EVSM_POS_EXP: f32 = {:?};\nconst EVSM_NEG_EXP: f32 = {:?};\nconst EVSM_SHADOW_BIAS: f32 = {:?};\nconst EVSM_BLEED_CUT: f32 = {:?};\n{}{SHADER}",
                    crate::mech::EVSM_POS_EXP,
                    crate::mech::EVSM_NEG_EXP,
                    crate::mech::EVSM_SHADOW_BIAS,
                    crate::mech::EVSM_BLEED_CUT,
                    fx_constants(),
                )
                .into(),
            ),
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None,
                },
                // The frame's local lights (render.rs lights_buf).
                wgpu::BindGroupLayoutEntry {
                    binding: 5, visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None,
                },
                // Effects: the glass in view, and each effect's list of it.
                wgpu::BindGroupLayoutEntry {
                    binding: 6, visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7, visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None,
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
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Float32x2, 3 => Unorm8x4, 4 => Float32x3, 5 => Float32, 6 => Float32, 7 => Float32];
        let strip_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Unorm8x4, 4 => Float32x2, 5 => Float32x4, 6 => Float32x3, 7 => Uint32x2, 8 => Float32x4, 9 => Float32];
        let particle_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Float32, 4 => Float32x3, 5 => Float32, 6 => Float32x2, 7 => Float32x2, 8 => Float32x2, 9 => Unorm8x4, 11 => Float32x4, 12 => Float32x3, 13 => Uint32x2];
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
            size: 640,
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
        // Into the effects' buffer (`effects`): world GUIs, strips and
        // particles, premultiplied, alpha kept (round 27).
        let fx_into = |label: &str, entry: &str, stride: usize, step: wgpu::VertexStepMode, attributes: &[wgpu::VertexAttribute]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: stride as u64, step_mode: step, attributes })],
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
            })
        };
        let fx_pipeline = fx_into("world gui effects", "vs", std::mem::size_of::<GuiVertex>(), wgpu::VertexStepMode::Vertex, &attrs);
        let strip_pipeline = fx_into("effects strips", "vs_strip", std::mem::size_of::<FxVertex>(), wgpu::VertexStepMode::Vertex, &strip_attrs);
        let particle_pipeline = fx_into("effects particles", "vs_particle", std::mem::size_of::<FxParticle>(), wgpu::VertexStepMode::Instance, &particle_attrs);
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
        let no_lights = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effects no lights"),
            size: std::mem::size_of::<crate::render::LightRaw>() as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let white = {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("effects white"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                tex.as_image_copy(),
                &[255u8; 4],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: None },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
            let view = tex.create_view(&Default::default());
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("effects white"),
                layout: &tex_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                ],
            })
        };
        WorldGuiPass {
            white,
            fx_pipeline,
            strip_pipeline,
            particle_pipeline,
            lights: no_lights,
            n_lights: 0,
            bufs: Default::default(),
            glass: Default::default(),
            tex_groups: Default::default(),
            fx_bgl,
            fx_in,
            fx_out,
            fx_tex: None,
            sun_atlas: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("unshadowed GUI fallback"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
            }).create_view(&Default::default()),
            sun_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("sun shadow cascades"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            sun_grid: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("unshadowed beam grid fallback"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::Rg8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
            }).create_view(&Default::default()),
            lightgrid: [0.0; 4],
            sun_params: [0.0, 580.0, 0.05, 0.0],
            sun_cascades: Default::default(),
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

    /// The world GUIs (`batches`) and, into the scene, the effects (`fx`),
    /// uploaded into the kept buffers: the depth / tone bind group, and per
    /// draw its kind, its texture's bind group and its index / instance
    /// range.
    #[allow(clippy::too_many_arguments)]
    fn upload(
        &mut self,
        rs: &egui_wgpu::RenderState,
        depth: &wgpu::TextureView,
        batches: &[GuiBatch],
        fx: Option<&FxFrame>,
        tone_curve: Option<[f32; 5]>,
        fog: [f32; 4],
        fog_rgb: [f32; 3],
    ) -> Option<Upload> {
        let fx_any = fx.is_some_and(|f| !f.draws.is_empty());
        if !fx_any && batches.iter().all(|b| b.indices.is_empty()) {
            return None;
        }
        let device = &rs.device;
        let t = tone_curve.unwrap_or([0.0; 5]);
        let u: [f32; 16] = [
            t[0], t[1], t[2], t[3], t[4], if tone_curve.is_some() { 1.0 } else { 0.0 }, crate::mech::NEAR, crate::mech::FX_LIGHT_CLAMP,
            fog[0], fog[1], fog[2], fog[3], fog_rgb[0], fog_rgb[1], fog_rgb[2], 0.0,
        ];
        let mut u = u.to_vec();
        u.extend(self.sun_cascades.local.iter().flatten().flatten().copied());
        u.extend(self.sun_cascades.sphere.iter().flatten().copied());
        u.extend(self.sun_cascades.rect.iter().flatten().copied());
        u.extend(self.lightgrid);
        u.extend(self.sun_params);
        let empty = FxFrame::default();
        let f = fx.unwrap_or(&empty);
        u.extend(f.vp.iter().flatten().copied());
        u.extend(f.cam);
        let mut amb = f.light[0];
        amb[3] = self.n_lights as f32;
        u.extend(amb);
        u.extend(f.light[1]);
        u.extend(f.light[2]);
        u.extend(f.light[3]);
        u.extend(f.right);
        rs.queue.write_buffer(&self.tone_buf, 0, bytemuck::cast_slice(&u));
        // (storage bindings can't be empty)
        let st = wgpu::BufferUsages::STORAGE;
        let pad = [FxGlass::default()];
        let [g0, g1] = &mut self.glass;
        fill(device, &rs.queue, g0, st, bytemuck::cast_slice(if f.glass.is_empty() { &pad } else { &f.glass }));
        fill(device, &rs.queue, g1, st, bytemuck::cast_slice(if f.glass_idx.is_empty() { &[0u32][..] } else { &f.glass_idx }));
        let (Some(g0), Some(g1)) = (&self.glass[0], &self.glass[1]) else { return None };
        let depth_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world gui depth"),
            layout: &self.depth_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 1, resource: self.tone_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.sun_atlas) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.sun_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.sun_grid) },
                wgpu::BindGroupEntry { binding: 5, resource: self.lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: g0.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: g1.as_entire_binding() },
            ],
        });
        // A bind group per egui texture (and sampler).
        self.frame += 1;
        // (effects' textures unused for a while are let go)
        let frame = self.frame;
        self.mips.retain(|_, (_, _, used)| frame - *used < 600);
        let egui = rs.renderer.read();
        self.tex_groups.retain(|_, (_, _, used)| frame - *used < 600);
        // (kept from frame to frame while egui's texture for the id is the
        // same one: making the bind groups again each frame was most of
        // this pass's CPU time)
        let group = |this: &mut Self, id: egui::TextureId, wrap: bool| -> Option<wgpu::BindGroup> {
            let tex = egui.texture(&id).and_then(|t| t.texture.clone())?;
            let mipped = tex.size().width.max(tex.size().height) > 1;
            // (the font atlas changes in place: its copy is refilled once a frame)
            let live = id == egui::TextureId::default();
            let kept = match this.tex_groups.get_mut(&(id, wrap)) {
                Some((t, bg, used)) if *t == tex => Some((bg.clone(), std::mem::replace(used, frame) != frame)),
                _ => None,
            };
            if let Some((bg, first)) = kept {
                if first && live && mipped {
                    this.mipped(device, &rs.queue, &tex, true);
                }
                return Some(bg);
            }
            let (view, sampler) = if mipped {
                (this.mipped(device, &rs.queue, &tex, live), if wrap { &this.sampler_mip_wrap } else { &this.sampler_mip })
            } else {
                (tex.create_view(&Default::default()), if wrap { &this.sampler_wrap } else { &this.sampler })
            };
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("world gui texture"),
                layout: &this.tex_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                ],
            });
            this.tex_groups.insert((id, wrap), (tex, bg.clone(), frame));
            Some(bg)
        };
        let mut vertices: Vec<GuiVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws: Vec<(Kind, wgpu::BindGroup, u32, u32)> = Vec::new();
        for b in batches {
            let Some(id) = b.texture else { continue };
            if b.indices.is_empty() {
                continue;
            }
            let Some(bg) = group(self, id, b.wrap) else { continue };
            let base = vertices.len() as u32;
            let start = indices.len() as u32;
            vertices.extend_from_slice(&b.vertices);
            indices.extend(b.indices.iter().map(|i| base + i));
            draws.push((Kind::Gui, bg, start, indices.len() as u32));
        }
        // Effects after the world GUIs, far to near. (An untextured effect
        // reads egui's white texel: a 1×1 white texture, not the font
        // atlas, whose mips would be refilled every frame for it.)
        for d in &f.draws {
            let bg = if d.texture == egui::TextureId::default() { Some(self.white.clone()) } else { group(self, d.texture, d.wrap) };
            let Some(bg) = bg else { continue };
            draws.push((if d.particles { Kind::Particles } else { Kind::Strip }, bg, d.start, d.end));
        }
        drop(egui);
        if draws.is_empty() {
            return None;
        }
        let (q, v, i) = (&rs.queue, wgpu::BufferUsages::VERTEX, wgpu::BufferUsages::INDEX);
        let [b0, b1, b2, b3, b4] = &mut self.bufs;
        fill(device, q, b0, v, bytemuck::cast_slice(&vertices));
        fill(device, q, b1, i, bytemuck::cast_slice(&indices));
        fill(device, q, b2, v, bytemuck::cast_slice(&f.strips));
        fill(device, q, b3, i, bytemuck::cast_slice(&f.indices));
        fill(device, q, b4, v, bytemuck::cast_slice(&f.particles));
        Some(Upload { depth_bg, draws })
    }

    fn record(&self, pass: &mut wgpu::RenderPass<'_>, gui: &wgpu::RenderPipeline, up: &Upload) {
        pass.set_bind_group(0, &up.depth_bg, &[]);
        let mut bound = None;
        for (kind, bg, start, end) in &up.draws {
            if bound != Some(*kind) {
                let [b0, b1, b2, b3, b4] = &self.bufs;
                match kind {
                    Kind::Gui => {
                        let (Some(vb), Some(ib)) = (b0, b1) else { continue };
                        pass.set_pipeline(gui);
                        pass.set_vertex_buffer(0, vb.slice(..));
                        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    }
                    Kind::Strip => {
                        let (Some(vb), Some(ib)) = (b2, b3) else { continue };
                        pass.set_pipeline(&self.strip_pipeline);
                        pass.set_vertex_buffer(0, vb.slice(..));
                        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    }
                    Kind::Particles => {
                        let Some(vb) = b4 else { continue };
                        pass.set_pipeline(&self.particle_pipeline);
                        pass.set_vertex_buffer(0, vb.slice(..));
                    }
                }
                bound = Some(*kind);
            }
            pass.set_bind_group(1, bg, &[]);
            let tris = match kind {
                Kind::Particles => 2 * (end - start),
                _ => (end - start) / 3,
            };
            crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::UI3D, 1, tris as u64);
            match kind {
                Kind::Particles => pass.draw(0..6, *start..*end),
                _ => pass.draw_indexed(*start..*end, 0, 0..1),
            }
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
        let Some(up) = self.upload(rs, depth, batches, None, tone_curve, fog, fog_rgb) else { return };
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
        fx: Option<&FxFrame>,
        tone_curve: Option<[f32; 5]>,
        fog: [f32; 4],
        fog_rgb: [f32; 3],
    ) -> bool {
        let up = {
            let _b = crate::bench::scope("fx_upload");
            self.upload(rs, depth, batches, fx, tone_curve, fog, fog_rgb)
        };
        let Some(up) = up else { return false };
        let _b = crate::bench::scope("fx_encode");
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
