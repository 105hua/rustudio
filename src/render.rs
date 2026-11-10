//! wgpu viewport renderer. Draws the scene into a float (HDR, MSAA-resolved)
//! target, then a post pass tone-maps it into the texture egui shows as an
//! image, so the 3D view docks like any other panel. The sun's shadow map is
//! drawn first.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu;
use glam::{Mat4, Vec3};
use rbx_dom_weak::types::Ref;
use wgpu::util::DeviceExt;

use crate::math::Xform;

/// The scene is lit in linear light into this target...
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// ...and the post pass writes display values (gamma 2.0) here.
const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// EVSM moments of the local light shadow maps (Studio's warps of 5.54 keep
/// e^(2·5.54) under half float's 65504).
const LSHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// One face's uniforms for the light shadow pass, at a dynamic offset.
const LSHADOW_FACE_STRIDE: u64 = 256;

/// The local light shadow atlas (Studio's Future path: a cube map per
/// shadowed light, Studio measurements "Local light shadows").
pub struct LightShadows {
    moments_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    /// ShadowSlot per light (6 view-projections + position / 1 over Range).
    slots_buf: wgpu::Buffer,
    /// LFace per face, read at a dynamic offset.
    face_buf: wgpu::Buffer,
    face_layout: wgpu::BindGroupLayout,
    face_bg: wgpu::BindGroup,
    /// What the atlas holds: static scene generation + the lights; redrawn when it changes.
    key: Option<u64>,
}

impl LightShadows {
    fn new(device: &wgpu::Device) -> Self {
        let side = crate::mech::LSHADOW_ATLAS;
        let tex = |label: &str, format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let moments_view = tex(
            "light shadow moments",
            LSHADOW_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth_view = tex("light shadow depth", DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("light shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let n = crate::mech::LSHADOW_MAX_LIGHTS.max(1);
        let slots_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("light shadow slots"),
            size: (n * (6 * 64 + 16)) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let face_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("light shadow faces"),
            size: n as u64 * 6 * LSHADOW_FACE_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let face_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light shadow face"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(96),
                },
                count: None,
            }],
        });
        let face_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light shadow face"),
            layout: &face_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &face_buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(96),
                }),
            }],
        });
        Self { moments_view, depth_view, sampler, slots_buf, face_buf, face_layout, face_bg, key: None }
    }

    /// The six face view-projections of a light: 93° frusta (Studio's
    /// 1/0.94896), near LSHADOW_NEAR, far its Range.
    fn faces(pos: Vec3, range: f32) -> [(Mat4, Vec3); 6] {
        let axes = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        let fov = 2.0 * crate::mech::LSHADOW_TAN_HALF_FOV.atan();
        let proj = Mat4::perspective_rh(fov, 1.0, crate::mech::LSHADOW_NEAR, range.max(crate::mech::LSHADOW_NEAR * 2.0));
        axes.map(|a| {
            let up = if a.y.abs() > 0.5 { Vec3::Z } else { Vec3::Y };
            (proj * Mat4::look_to_rh(pos, a, up), a)
        })
    }
}
const SAMPLES: u32 = 4;
const SHADOW_SIZE: u32 = crate::mech::SHADOW_MAP_TEXELS;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub cam_pos: [f32; 4],
    pub sun_dir: [f32; 4],
    pub sun_color: [f32; 4],
    pub ambient: [f32; 4],
    pub outdoor: [f32; 4],
    pub sky_top: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub params: [f32; 4],
    pub fog: [f32; 4],
    pub fog_color: [f32; 4],
    pub cc: [f32; 4],
    pub cc_tint: [f32; 4],
    pub bloom: [f32; 4],
    /// The sun's shadow cascades (sun_shadow.rs): world to each tile's
    /// (u, v, depth), each sphere (centre, radius²), each tile in the atlas.
    pub sun_casc: [[[f32; 4]; 4]; 4],
    pub sun_sphere: [[f32; 4]; 4],
    pub sun_rect: [[f32; 4]; 4],
    pub sky_rot: [f32; 4],
    /// Skybox irradiance facing +X, −X, +Y, −Y, +Z, −Z, times EnvironmentDiffuseScale.
    pub env: [[f32; 4]; 6],
    /// The sky's brightness (and its light's) by time of day.
    pub sky_tint: [f32; 4],
    /// Light grid (lightgrid.rs): xyz its first voxel, w studs out along the normal (0 = none yet).
    pub lightgrid: [f32; 4],
    /// Atmosphere (round 26): rgb the haze band colour (linear), w the
    /// band strength at the horizon (0 = no Atmosphere).
    pub atmos: [f32; 4],
    /// x EnvironmentSpecularScale (0 = no sky reflection), y roughness (round 26).
    pub env_spec: [f32; 4],
    /// Parts blocking local lights (round 26, lightocc.rs): xyz the grid's
    /// first voxel, w studs per voxel (0 = no grid).
    pub lightocc: [f32; 4],
    /// Atmosphere haze toward +X, −X, +Y, −Y, +Z, −Z (AdvSkyFS irradiance × FogColor).
    pub atmos_cube: [[f32; 4]; 6],
    /// x fogOffset.x, y fogOffset.y, z Glare, w Offset.
    pub atmos_sky: [f32; 4],
    /// xyz the glare's exponents per channel (from Decay).
    pub atmos_glare: [f32; 4],
    /// Studio's indoor environment light (AmbientCube[6..11]): the
    /// irradiance of textures/sky/indoor512 × EnvironmentDiffuseScale,
    /// +X, −X, +Y, −Y, +Z, −Z (Studio measurements "Environment light and AO").
    pub env_indoor: [[f32; 4]; 6],
    /// SkyGradientTop / Bottom (CB0[30] / [31]): the time-of-day sky
    /// levels c0², c1²; w EnvironmentDiffuse / SpecularScale.
    pub sky_grad: [[f32; 4]; 2],
    /// Lamp1: the back light (faces turned from Lamp0), rgb.
    pub lamp1: [f32; 4],
    /// Screen-space AO: x, y half the projection diagonal, z near plane, w on.
    pub ssao: [f32; 4],
    pub dof: [f32; 4],
}

/// One local light as the shader reads it (see `Light` in shader.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct LightRaw {
    pub pos_range: [f32; 4],
    pub color: [f32; 4],
    pub dir: [f32; 4],
    pub extra: [f32; 4],
    pub surface_u: [f32; 4],
    pub surface_v: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    projection: [f32; 4],
    /// Per-vertex colour (sRGB 0..1); alpha 0 = use the part's Color.
    color: [f32; 4],
}

/// Box-projected UVs for built-in shapes. Each face is seen from outside
/// with its image upright: the front face (−Z) has image-left at +X (the
/// part's right), matching the classic clothing-template layout.
fn box_uv(p: Vec3, n: Vec3) -> [f32; 2] {
    let a = n.abs();
    if a.x >= a.y && a.x >= a.z {
        if n.x > 0.0 {
            [0.5 - p.z, 0.5 - p.y]
        } else {
            [p.z + 0.5, 0.5 - p.y]
        }
    } else if a.y >= a.z {
        if n.y > 0.0 {
            [0.5 - p.x, 0.5 - p.z]
        } else {
            [0.5 - p.x, p.z + 0.5]
        }
    } else if n.z < 0.0 {
        [0.5 - p.x, 0.5 - p.y]
    } else {
        [p.x + 0.5, 0.5 - p.y]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    r0: [f32; 4],
    r1: [f32; 4],
    r2: [f32; 4],
    size: [f32; 4],
    color: [f32; 4],
    extra: [f32; 4],
    uvt: [f32; 4],
    /// SurfaceAppearance emission: EmissiveTint² × EmissiveStrength.
    emissive: [f32; 4],
}

/// How a texture combines with the part (shader `extra.w`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexMode {
    None,
    /// Over the part colour (clothing, SurfaceAppearance AlphaMode Overlay):
    /// transparent texels show Color.
    Over,
    /// A decal: the texture's own alpha, tinted by the colour.
    Decal,
    /// The texture alone, alpha ignored (MeshPart TextureID, SpecialMesh
    /// TextureId: Studio shows neither Color nor alpha through them).
    Opaque,
    /// SpecialMesh TextureId multiplied by its VertexColor.
    OpaqueTinted,
    /// SurfaceAppearance AlphaMode Transparency: texels under half alpha are cut out.
    Cutout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeshKind {
    Block = 0,
    Ball = 1,
    Cylinder = 2,
    Wedge = 3,
    CornerWedge = 4,
}

/// One part as the renderer sees it.
#[derive(Debug, Clone, Copy)]
pub struct PartDraw {
    pub xf: Xform,
    pub size: Vec3,
    /// Linear RGB.
    pub color: [f32; 3],
    pub alpha: f32,
    pub mesh: MeshKind,
    /// 0 plastic, 1 neon, 2 smooth, 3 glass.
    pub material: f32,
    pub placeholder: bool,
    pub hover: bool,
    /// Reflectance (0..1): the sky's reflection mixed into the colour.
    pub reflectance: f32,
    /// A loaded mesh (index from [`Renderer::add_mesh`]) drawn instead of `mesh`.
    pub custom_mesh: Option<u32>,
    /// A texture from [`Renderer::add_texture`].
    pub texture: Option<u32>,
    pub tex_mode: TexMode,
    /// UV scale (x, y) and offset (z, w), e.g. Texture tiling.
    pub uvt: [f32; 4],
    /// Emission (linear): the texture times its emissive mask times this.
    pub emissive: [f32; 3],
    /// The Material's darkening of textures (the part colour already has it).
    pub tex_albedo: f32,
    /// Legacy surfaces, 3 bits per face in Enum.NormalId order (Right, Top,
    /// Back, Left, Bottom, Front): 0 smooth, 1 studs, 2 weld / glue,
    /// 3 inlet, 4 universal (bands of Studio's studs.dds).
    pub surfaces: u32,
    /// Enum.Material value.
    pub material_id: u32,
    /// Material texture array layer (-1: none, draw flat).
    pub mat_layer: i32,
    /// BasePart.CastShadow: false keeps it out of the sun's shadow map.
    pub cast_shadow: bool,
}

impl PartDraw {
    pub const NO_UVT: [f32; 4] = [1.0, 1.0, 0.0, 0.0];
}

pub struct Frame<'a> {
    pub globals: Globals,
    /// Ordinary (not reverse-Z) view-projection, for culling.
    pub cull_view_proj: Mat4,
    pub fov_y: f32,
    /// Local lights (at most MAX_LIGHTS).
    pub lights: &'a [LightRaw],
    /// Draw the sun's shadow cascades (`sun_cascades`).
    pub shadows: bool,
    pub sun_cascades: [crate::sun_shadow::Cascade; crate::sun_shadow::CASCADES],
    /// Drawn on top of what's already there (hover highlight).
    pub overlay: &'a [PartDraw],
    pub lines: &'a [LineVertex],
    /// Draw the sky (an orthographic game camera has none: Studio leaves
    /// black where nothing is).
    pub sky: bool,
    /// Terrain water's waves, animation and clearness (water.rs).
    pub water: crate::water::WaterLook,
}

/// Studs per side of the cells static parts are grouped into.
const CELL: f32 = crate::mech::RENDER_CELL;

/// Which pass a draw belongs to; None = blended part, sorted every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Pass {
    Opaque,
    Decal,
}

fn pass_of(p: &PartDraw) -> Option<Pass> {
    if p.tex_mode == TexMode::Decal && p.texture.is_some() {
        // Texture batching changes coplanar decal layer order. Keep decals
        // with the sorted translucent draws so their source order survives
        // equal distances, and translucent hosts draw behind their decals.
        None
    } else if p.alpha >= 0.999 && p.material_id != 1584 {
        Some(Pass::Opaque)
    } else {
        None
    }
}

fn transparent_order(a_kind: f32, a_distance: f32, b_kind: f32, b_distance: f32) -> std::cmp::Ordering {
    let a_glass = (a_kind as u32 & 7) == 3;
    let b_glass = (b_kind as u32 & 7) == 3;
    b_glass.cmp(&a_glass).then_with(|| b_distance.total_cmp(&a_distance))
}

#[cfg(test)]
mod transparent_order_tests {
    use super::transparent_order;
    use std::cmp::Ordering;

    #[test]
    fn glass_precedes_other_transparents_regardless_of_distance() {
        assert_eq!(transparent_order(3.0, 1.0, 0.0, 100.0), Ordering::Less);
        assert_eq!(transparent_order(6.0, 100.0, 3.0, 1.0), Ordering::Greater);
    }

    #[test]
    fn each_group_remains_back_to_front() {
        for kind in [0.0, 3.0, 6.0] {
            assert_eq!(transparent_order(kind, 100.0, kind, 1.0), Ordering::Less);
            assert_eq!(transparent_order(kind, 1.0, kind, 1.0), Ordering::Equal);
        }
    }
}

fn shows(p: &PartDraw) -> bool {
    p.alpha > 0.001
}

fn radius_of(p: &PartDraw) -> f32 {
    p.size.length() * 0.5
}

/// The camera is inside the part's box.
fn camera_inside(p: &PartDraw, cam: Vec3) -> bool {
    let local = p.xf.rot.transpose() * (cam - p.xf.pos);
    local.abs().cmple(p.size * 0.5).all()
}

#[derive(Debug, Clone, PartialEq)]
enum ItemState {
    /// New: drawn per frame until the next static rebuild.
    Pending,
    /// In the static buffer: slot per draw (u32::MAX = not drawn).
    Static(Vec<u32>),
    /// Moves (or is see-through): drawn per frame.
    Dynamic,
}

/// One part's draws (the part itself plus its decals).
struct Item {
    draws: Vec<PartDraw>,
    bias: f32,
    state: ItemState,
    /// Last time the part changed place or look (Dynamic parts quiet for
    /// DYNAMIC_SETTLE_S go back to the static batches).
    moved: Instant,
}

/// Same place within float noise: a CFrame write of the same value comes
/// back through the scene's transform maths with its low bits changed.
fn near_xf(a: &crate::math::Xform, b: &crate::math::Xform) -> bool {
    a.pos.abs_diff_eq(b.pos, 1e-4) && a.rot.abs_diff_eq(b.rot, 1e-5)
}

struct Batch {
    pass: Pass,
    key: (usize, usize),
    lo: Vec3,
    hi: Vec3,
    start: u32,
    count: u32,
}

/// Anchored, opaque parts: uploaded once, sorted by (pass, mesh/texture,
/// cell, size) so each cell is one instanced draw that can be culled and
/// trimmed of its smallest parts with distance.
#[derive(Default)]
struct StaticLayout {
    raw: Vec<InstanceRaw>,
    /// Bounding radius per instance (descending within a batch).
    radii: Vec<f32>,
    batches: Vec<Batch>,
    hidden: usize,
    /// Slots changed in place, uploaded next frame.
    writes: Vec<u32>,
}

/// Counters for the last frame (shown in the bench report).
#[derive(Default, Debug, Clone, Copy)]
pub struct RenderStats {
    pub items: usize,
    pub static_instances: usize,
    pub batches: usize,
    pub per_frame_items: usize,
    pub drawn_instances: usize,
    pub draw_calls: usize,
    pub culled_batches: usize,
    pub lod_skipped: usize,
    pub rebuilds: u64,
}

/// A small, stable depth nudge per part so exactly coplanar faces of two
/// parts always resolve the same way instead of flickering. From the part's
/// own size and placement, not its Ref (Refs are random per run: which of
/// two coplanar floors showed changed from run to run).
fn bias_of(draws: &[PartDraw]) -> f32 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    if let Some(d) = draws.first() {
        for v in d.size.to_array().into_iter().chain(d.xf.pos.to_array()) {
            v.to_bits().hash(&mut h);
        }
    }
    (h.finish() % 16) as f32 * crate::mech::DEPTH_BIAS_STEP
}

fn spread(v: u64) -> u64 {
    let mut x = v & 0x1f_ffff;
    x = (x | x << 32) & 0x1f_0000_0000_ffff;
    x = (x | x << 16) & 0x1f_0000_ff00_00ff;
    x = (x | x << 8) & 0x100f_00f0_0f00_f00f;
    x = (x | x << 4) & 0x10c3_0c30_c30c_30c3;
    (x | x << 2) & 0x1249_2492_4924_9249
}

/// Z-order of a cell, so neighbouring cells sit next to each other.
fn morton(c: [i32; 3]) -> u64 {
    let o = |v: i32| (v + (1 << 20)) as u64;
    spread(o(c[0])) | spread(o(c[1])) << 1 | spread(o(c[2])) << 2
}

/// World-space bounding box of a draw.
fn aabb_of(p: &PartDraw) -> (Vec3, Vec3) {
    let r = p.xf.rot;
    let h = p.size * 0.5;
    let ext = r.x_axis.abs() * h.x + r.y_axis.abs() * h.y + r.z_axis.abs() * h.z;
    (p.xf.pos - ext, p.xf.pos + ext)
}

/// Frustum planes (normal, d) from a view-projection matrix (depth 0..1).
pub fn frustum(m: Mat4) -> [glam::Vec4; 5] {
    let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
    let n = |p: glam::Vec4| p / p.truncate().length().max(1e-6);
    [n(r3 + r0), n(r3 - r0), n(r3 + r1), n(r3 - r1), n(r2)]
}

fn box_visible(planes: &[glam::Vec4; 5], lo: Vec3, hi: Vec3) -> bool {
    planes.iter().all(|p| {
        let v = Vec3::new(
            if p.x > 0.0 { hi.x } else { lo.x },
            if p.y > 0.0 { hi.y } else { lo.y },
            if p.z > 0.0 { hi.z } else { lo.z },
        );
        p.truncate().dot(v) + p.w >= 0.0
    })
}

pub fn sphere_visible(planes: &[glam::Vec4; 5], c: Vec3, r: f32) -> bool {
    planes.iter().all(|p| p.truncate().dot(c) + p.w >= -r)
}

fn box_distance(p: Vec3, lo: Vec3, hi: Vec3) -> f32 {
    (p.clamp(lo, hi) - p).length()
}

const HIDDEN: InstanceRaw = InstanceRaw {
    r0: [0.0; 4],
    r1: [0.0; 4],
    r2: [0.0; 4],
    size: [0.0; 4],
    color: [0.0; 4],
    extra: [0.0; 4],
    uvt: [0.0; 4],
    emissive: [0.0; 4],
};

/// Terrain palette entries: 23 SmoothGrid materials × top / side / bottom.
pub const TERRAIN_PALETTE: usize = 69;

/// One terrain chunk's triangles (round 21) and their bounds.
struct TerrainChunk {
    vbuf: wgpu::Buffer,
    count: u32,
    lo: Vec3,
    hi: Vec3,
}

/// Layers with their own Studio material constants in `MaterialArrays::params`.
const MAT_PARAM_LAYERS: usize = 256;

/// Two RGBA8 texture arrays of material maps (layer = pack), mipmapped, and
/// the part shader's per-layer material constants (shader `mat_params`):
/// 5 vec4 per layer: detail/tiling, noise/type, far tiling/emissive R,
/// cutoffs/emissive G, cutoff scales/emissive B; then Studio's 32²
/// noise and 128² Wang index, packed as exact integers in f32 lanes.
struct MaterialArrays {
    albedo: wgpu::Texture,
    surface: wgpu::Texture,
    emission: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    surface_view: wgpu::TextureView,
    emission_view: wgpu::TextureView,
    size: u32,
    capacity: u32,
    layers: std::collections::HashMap<u64, u32>,
    cpu: Vec<std::sync::Arc<crate::matpack::Maps>>,
    params: Vec<[f32; 4]>,
    params_buf: wgpu::Buffer,
}

impl MaterialArrays {
    /// The default constants: no detail map, textureTiling 1, tiled; noise.
    fn default_params() -> Vec<[f32; 4]> {
        let mut p = Vec::with_capacity(MAT_PARAM_LAYERS * 5 + 256 + 1024);
        for _ in 0..MAT_PARAM_LAYERS {
            p.push([-1.0, 0.0, 0.0, 1.0]);
            p.push([0.0, 0.0, 4.0, 1.0]);
            p.push([1.0, 1.0, 1.0, 0.0]);
            p.push([0.0; 4]);
            p.push([1.0, 1.0, 1.0, 0.0]);
        }
        p.extend(crate::matpack::detile_noise().chunks(4).map(|c| [c[0], c[1], c[2], c[3]]));
        p.extend(crate::matpack::wang_index());
        p
    }

    fn new(device: &wgpu::Device, size: u32, capacity: u32) -> Self {
        Self::with_params(device, size, capacity, Self::default_params())
    }

    fn with_params(device: &wgpu::Device, size: u32, capacity: u32, params: Vec<[f32; 4]>) -> Self {
        let make = |label, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: capacity,
                },
                mip_level_count: mip_levels(size, size),
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let view = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let albedo = make("material albedo", wgpu::TextureFormat::Rgba8Unorm);
        let surface = make("material surface", wgpu::TextureFormat::Rgba8Unorm);
        let emission = make("material emission", wgpu::TextureFormat::R8Unorm);
        let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material params"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        MaterialArrays {
            albedo_view: view(&albedo),
            surface_view: view(&surface),
            emission_view: view(&emission),
            albedo,
            surface,
            emission,
            size,
            capacity,
            layers: Default::default(),
            cpu: Vec::new(),
            params,
            params_buf,
        }
    }

    fn upload(&self, queue: &wgpu::Queue, layer: u32, maps: &crate::matpack::Maps) {
        for (tex, data, channels) in [(&self.albedo, &maps.albedo, 4), (&self.surface, &maps.surface, 4), (&self.emission, &maps.emissive, 1)] {
            for (mip, (at, size)) in crate::matpack::mip_levels().into_iter().enumerate() {
                let at = at / (4 / channels) as usize;
                let bytes = (size * size * channels) as usize;
                let Some(level) = data.get(at..at + bytes) else {
                    break;
                };
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: tex,
                        mip_level: mip as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    level,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size * channels),
                        rows_per_image: Some(size),
                    },
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
    }
}

struct Mesh {
    vbuf: wgpu::Buffer,
    count: u32,
}

struct Target {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    /// The resolved scene in linear light.
    hdr: wgpu::Texture,
    hdr_view: wgpu::TextureView,
    /// The opaque scene's colour, copied for the water pass to look through.
    scene_copy: wgpu::Texture,
    water_bg: wgpu::BindGroup,
    glass_bg: wgpu::BindGroup,
    glass_depth: wgpu::TextureView,
    hdr_bg: wgpu::BindGroup,
    dof_bgs: [wgpu::BindGroup; 2],
    dof_views: [wgpu::TextureView; 2],
    /// Linear bloom pyramid, from half resolution through 1/32 resolution.
    bloom_view: wgpu::TextureView,
    bloom_levels: Vec<wgpu::TextureView>,
    bloom_down_bgs: Vec<wgpu::BindGroup>,
    bloom_up_bgs: Vec<wgpu::BindGroup>,
    /// SSAO at half resolution: the AO depth, then (occlusion, depth) ping-
    /// ponged through the blurs; a bind group per pass (depth + its input).
    ssao_depth_view: wgpu::TextureView,
    ssao_views: [wgpu::TextureView; 2],
    ssao_bgs: [wgpu::BindGroup; 4],
    /// The tone-mapped image egui shows.
    display: wgpu::Texture,
    display_view: wgpu::TextureView,
}

pub struct Renderer {
    /// SurfaceGuis / BillboardGuis tested against the scene's depth (round 23).
    world_gui: Option<crate::world_gui::WorldGuiPass>,
    /// Terrain water (round 24, water.rs).
    water: crate::water::WaterPass,
    glass_layout: wgpu::BindGroupLayout,
    glass_resolve: wgpu::RenderPipeline,
    material_epoch: Instant,
    /// Fog for the world GUI pass (gui_fog).
    gui_fog: ([f32; 4], [f32; 3]),
    /// The cascades as the beams read them (matrices, spheres, tiles).
    gui_sun: crate::world_gui::SunCascades,
    gui_lightgrid: [f32; 4],
    gui_sun_params: [f32; 4],
    /// Local lights in lights_buf this frame (the effects read them).
    gui_n_lights: u32,
    sky: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    transparent_culled: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    glass_culled: wgpu::RenderPipeline,
    opaque_two_sided: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    post: wgpu::RenderPipeline,
    post_dof: bool,
    dof_h: wgpu::RenderPipeline,
    dof_v: wgpu::RenderPipeline,
    /// Studio's SSAO passes (shader.wgsl "SSAO"): depth down, occlusion,
    /// blur across, blur down, and the multiply into the HDR scene.
    ssao_layout: wgpu::BindGroupLayout,
    ssao_down: wgpu::RenderPipeline,
    ssao_main: wgpu::RenderPipeline,
    ssao_blur_x: wgpu::RenderPipeline,
    ssao_blur_y: wgpu::RenderPipeline,
    ssao_apply: wgpu::RenderPipeline,
    /// Bloom: bright pass, blur across (two targets), blur down.
    bloom_bright: wgpu::RenderPipeline,
    bloom_down: wgpu::RenderPipeline,
    bloom_up: wgpu::RenderPipeline,
    post_bloom_size: f32,
    shadow: wgpu::RenderPipeline,
    /// Local light shadow maps (Studio's Future path) and their pass.
    lshadow: LightShadows,
    lshadow_pipe: wgpu::RenderPipeline,
    /// Bumped whenever the static instances change (light shadow cache key).
    static_gen: u64,
    /// Smooth terrain (round 21): per-chunk triangle buffers, drawn with
    /// Studio's terrain shading, and their per-palette parameters.
    terrain: wgpu::RenderPipeline,
    terrain_shadow: wgpu::RenderPipeline,
    terrain_params_buf: wgpu::Buffer,
    terrain_bg: wgpu::BindGroup,
    terrain_chunks: HashMap<[i32; 3], TerrainChunk>,
    /// Decoration grass blades by chunk (drawn with the terrain, no shadow).
    terrain_grass: HashMap<[i32; 3], TerrainChunk>,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    /// The globals alone, for the shadow pass (its map can't be bound there).
    shadow_globals_bg: wgpu::BindGroup,
    globals_layout: wgpu::BindGroupLayout,
    lights_buf: wgpu::Buffer,
    /// The depth map the sun's cascades are drawn into, one at a time.
    shadow_view: wgpu::TextureView,
    /// The sun's shadow cascades: the atlas the shaders read (sun_shadow.rs).
    sun: crate::sun_shadow::SunShadows,
    sky_view: wgpu::TextureView,
    /// Studio's indoor environment (textures/sky/indoor512); white 1×1 until loaded.
    indoor_view: wgpu::TextureView,
    /// Studio's split-sum BRDF table (brdfLUT.dds, RG16F).
    brdf_view: wgpu::TextureView,
    sky_sampler: wgpu::Sampler,
    /// Studio's light grid: skylight and sun per 4-stud voxel (lightgrid.rs).
    lightgrid_tex: wgpu::Texture,
    lightgrid_view: wgpu::TextureView,
    localgrid_tex: wgpu::Texture,
    localgrid_view: wgpu::TextureView,
    /// How much of each voxel parts fill, for local lights (lightocc.rs).
    lightocc_tex: wgpu::Texture,
    lightocc_view: wgpu::TextureView,
    /// Studio's legacy surface atlas (studs.dds); grey (no change) until loaded.
    studs_view: wgpu::TextureView,
    studs_sampler: wgpu::Sampler,
    /// Material texture packs (matpack.rs): one array layer per pack,
    /// colour + height and normal + roughness + metalness.
    materials: MaterialArrays,
    /// Size of the loaded skybox faces (0 = none, draw the gradient).
    pub sky_size: u32,
    /// 1×1 black: the emissive mask of textures without one.
    black_view: wgpu::TextureView,
    meshes: Vec<Mesh>,
    tex_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Texture bind groups; index 0 is plain white.
    textures: Vec<wgpu::BindGroup>,
    overlay: wgpu::RenderPipeline,
    /// Per-frame instances: moving and see-through parts, overlay.
    instance_buf: wgpu::Buffer,
    instance_cap: usize,
    static_buf: wgpu::Buffer,
    static_cap: usize,
    items: HashMap<Ref, Item>,
    /// Items drawn from the per-frame list (pending and dynamic).
    per_frame: HashSet<Ref>,
    st: StaticLayout,
    pending: usize,
    pending_at: Option<Instant>,
    /// Last static rebuild / quiet-part scan (both rate-limited).
    last_rebuild: Instant,
    last_quiet_scan: Instant,
    pub stats: RenderStats,
    line_buf: wgpu::Buffer,
    line_cap: usize,
    target: Option<Target>,
    pub texture_id: Option<egui::TextureId>,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rustudio viewport"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let frag = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                uniform_entry,
                frag(
                    1,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                frag(
                    2,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                ),
                frag(
                    3,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                // The sun's shadow cascades: EVSM moments (sun_shadow.rs).
                frag(
                    4,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                frag(
                    7,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                frag(
                    8,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                frag(
                    9,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                frag(
                    10,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                frag(
                    11,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                ),
                frag(
                    12,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                ),
                // Studio's indoor environment cube (PrefilteredEnvIndoorTexture).
                frag(
                    13,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                ),
                // PrecomputedBRDFTexture (PlatformContent textures/brdfLUT.dds).
                frag(
                    14,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                // Local light shadows: EVSM moment atlas, its sampler, the slots.
                frag(
                    15,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                frag(
                    16,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                frag(
                    17,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                frag(
                    19,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                ),
                frag(20, wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3, multisampled: false,
                }),
                // Studio's per-material constants and detiling noise (MaterialArrays).
                frag(
                    18,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let shadow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow globals"),
            entries: &[uniform_entry],
        });
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lights_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lights"),
            size: (crate::mech::MAX_LIGHTS.max(1) * std::mem::size_of::<LightRaw>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun shadow map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_tex.create_view(&Default::default());
        let [lg_x, lg_y, lg_z] = crate::lightgrid::dims().map(|n| n as u32);
        let lightgrid_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("light grid"),
            size: wgpu::Extent3d {
                width: lg_x,
                height: lg_z,
                depth_or_array_layers: lg_y,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lightgrid_view = lightgrid_tex.create_view(&Default::default());
        let localgrid_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("voxel local lights"),
            size: wgpu::Extent3d { width: lg_x, height: lg_z, depth_or_array_layers: lg_y },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let localgrid_view = localgrid_tex.create_view(&Default::default());
        let [lo_x, lo_y, lo_z] = crate::lightocc::dims().map(|n| n as u32);
        let lightocc_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("light occupancy"),
            size: wgpu::Extent3d {
                width: lo_x,
                height: lo_y,
                depth_or_array_layers: lo_z,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let lightocc_view = lightocc_tex.create_view(&Default::default());
        let sky_view = Self::cube_view(
            device,
            queue,
            1,
            &[
                [255u8; 4].to_vec(),
                vec![255; 4],
                vec![255; 4],
                vec![255; 4],
                vec![255; 4],
                vec![255; 4],
            ],
        );
        let indoor_view = Self::cube_view(
            device,
            queue,
            1,
            &[vec![0u8, 0, 0, 255], vec![0, 0, 0, 255], vec![0, 0, 0, 255], vec![0, 0, 0, 255], vec![0, 0, 0, 255], vec![0, 0, 0, 255]],
        );
        let brdf_view = Self::brdf_lut_view(device, queue);
        let sky_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let studs_view = Self::studs_texture(device, queue, 1, 1, &[128, 128, 128, 255]);
        let materials = MaterialArrays::new(device, 1, 1);
        let studs_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("studs"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // Also the material maps' sampler: Studio's anisotropic filter
            // at the device default (mech::TEXTURE_ANISOTROPY).
            anisotropy_clamp: crate::mech::TEXTURE_ANISOTROPY,
            ..Default::default()
        });
        let lshadow = LightShadows::new(device);
        let sun = crate::sun_shadow::SunShadows::new(device, &shadow_view);
        let globals_bg = Self::globals_group(
            device,
            &bgl,
            &globals_buf,
            &lights_buf,
            &sky_view,
            &sky_sampler,
            &sun.atlas_view,
            &lightgrid_view,
            &lightocc_view,
            &localgrid_view,
            &studs_view,
            &studs_sampler,
            &materials,
            &indoor_view,
            &brdf_view,
            &lshadow,
        );
        let shadow_globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow globals"),
            layout: &shadow_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });
        let tex_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
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
                // SurfaceAppearance emissive mask (black = none).
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
                    }, count: None,
                },
            ],
        });
        let black_view = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("no emission"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[0, 0, 0, 255],
            )
            .create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("texture"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Mipmapped and anisotropic, like Studio: finely tiled textures
            // average out at a distance instead of sparkling.
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: crate::mech::TEXTURE_ANISOTROPY,
            ..Default::default()
        });
        let glass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass scene"),
            entries: &[
                frag(0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
                frag(1, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport"),
            bind_group_layouts: &[Some(&bgl), Some(&tex_layout)],
            immediate_size: 0,
        });

        let part_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("parts with glass scene"),
            bind_group_layouts: &[Some(&bgl), Some(&tex_layout), Some(&glass_layout)],
            immediate_size: 0,
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 8 => Float32x2, 12 => Float32x4, 11 => Float32x4],
        };
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceRaw>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 9 => Float32x4, 10 => Float32x4],
        };
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
        };

        let ms = wgpu::MultisampleState {
            count: SAMPLES,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    topology: wgpu::PrimitiveTopology,
                    cull: Option<wgpu::Face>,
                    depth_write: bool,
                    depth_compare: wgpu::CompareFunction,
                    blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(if fs == "fs_part" { &part_layout } else { &layout }),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(depth_compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: ms,
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::CompareFunction as C;
        use wgpu::PrimitiveTopology as T;
        let sky = make(
            "sky",
            "vs_sky",
            "fs_sky",
            &[],
            T::TriangleList,
            None,
            false,
            C::Always,
            None,
        );
        let opaque = make(
            "parts",
            "vs_part",
            "fs_part",
            &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            T::TriangleList,
            Some(wgpu::Face::Back),
            true,
            C::Greater,
            None,
        );
        let opaque_two_sided = make(
            "parts-two-sided",
            "vs_part",
            "fs_part",
            &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            T::TriangleList,
            None,
            true,
            C::Greater,
            None,
        );
        // fs_part writes premultiplied colour (round 26): see-through Neon
        // adds its glow with cover 0, everything else blends as before.
        let transparent = make(
            "parts-transparent",
            "vs_part",
            "fs_part",
            &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            T::TriangleList,
            None,
            false,
            C::Greater,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        // See-through built-in shapes show their front faces only, as in
        // Studio (round 19: from inside a translucent part it draws nothing).
        let transparent_culled = make(
            "parts-transparent-culled",
            "vs_part",
            "fs_part",
            &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            T::TriangleList,
            Some(wgpu::Face::Back),
            false,
            C::Greater,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        // GlassTransparentFS already composites the opaque scene. Replace RGB
        // and preserve its energy-derived alpha; ordinary transparents remain
        // premultiplied One / OneMinusSrcAlpha.
        let glass = make("glass-composite", "vs_part", "fs_part",
            &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            T::TriangleList, None, false, C::Greater, None);
        let glass_culled = make("glass-composite-culled", "vs_part", "fs_part",
            &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            T::TriangleList, Some(wgpu::Face::Back), false, C::Greater, None);
        let water = crate::water::WaterPass::new(
            device,
            queue,
            &shader,
            &bgl,
            &tex_layout,
            &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            HDR_FORMAT,
            SAMPLES,
        );
        // Hover highlight: the same geometry again, on top of itself.
        let overlay = make(
            "parts-overlay",
            "vs_part",
            "fs_part",
            &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            T::TriangleList,
            None,
            false,
            C::GreaterEqual,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        // Local light shadow faces: EVSM moments + depth, both sides.
        let lshadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("light shadow"),
            bind_group_layouts: &[Some(&shadow_bgl), Some(&lshadow.face_layout)],
            immediate_size: 0,
        });
        let lshadow_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("light shadow"),
            layout: Some(&lshadow_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_light_shadow"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout.clone()), Some(instance_layout.clone())],
            },
            primitive: wgpu::PrimitiveState {
                topology: T::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(C::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_light_shadow"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: LSHADOW_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // A cascade of the sun's shadow: depth only, both sides (thin
        // parts). No depth bias: the EVSM test's minimum variance is the bias.
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&shadow_bgl), Some(&sun.pass_layout)],
            immediate_size: 0,
        });
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout), Some(instance_layout)],
            },
            primitive: wgpu::PrimitiveState {
                topology: T::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(C::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        // Smooth terrain (round 21): its own vertices (terrain.rs
        // TerrainVertex) and per-palette parameters in group 1.
        let terrain_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain params"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let terrain_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain params"),
            size: (TERRAIN_PALETTE * 4 * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let terrain_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain params"),
            layout: &terrain_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: terrain_params_buf.as_entire_binding(),
            }],
        });
        let terrain_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::terrain::TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm8x4, 2 => Uint8x4, 3 => Uint8x4, 4 => Uint8x4],
        };
        let terrain_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(&bgl), Some(&terrain_bgl)],
            immediate_size: 0,
        });
        let terrain = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&terrain_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_terrain"),
                compilation_options: Default::default(),
                buffers: &[Some(terrain_vertex_layout.clone())],
            },
            primitive: wgpu::PrimitiveState {
                topology: T::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(C::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: ms,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_terrain"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let terrain_shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_terrain_shadow"),
                compilation_options: Default::default(),
                buffers: &[Some(terrain_vertex_layout)],
            },
            primitive: wgpu::PrimitiveState {
                topology: T::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(C::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        // Exposure, tone mapping, bloom and colour correction into the display texture.
        let fullscreen = |label: &str, fs: &str, targets: usize| {
            let t = Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &vec![t; targets],
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let dof_v = fullscreen("dof down", "fs_dof_v", 1);
        let bloom_bright = fullscreen("bloom bright", "fs_bloom_bright", 1);
        let bloom_down = fullscreen("bloom pyramid down", "fs_bloom_down", 1);
        let bloom_up_desc = wgpu::RenderPipelineDescriptor {
            label: Some("bloom pyramid up"), layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_post"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_bloom_up"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent::REPLACE }), write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(), multiview_mask: None, cache: None,
        };
        let bloom_up = device.create_render_pipeline(&bloom_up_desc);
        let ssao_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ssao"),
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post with depth"),
            bind_group_layouts: &[Some(&bgl), Some(&tex_layout), Some(&ssao_layout)],
            immediate_size: 0,
        });
        let post = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post"),
            layout: Some(&post_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_post"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_post"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: DISPLAY_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let dof_h = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("dof prepare"),
            layout: Some(&post_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_post"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_dof_h"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let ssao_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ssao"),
            bind_group_layouts: &[Some(&bgl), Some(&ssao_layout)],
            immediate_size: 0,
        });
        let ssao_pipe = |label: &str, fs: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&ssao_pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let glass_resolve = ssao_pipe("glass depth", "fs_glass_depth", wgpu::TextureFormat::R32Float, None);
        let ssao_down = ssao_pipe("ssao depth", "fs_ssao_down", wgpu::TextureFormat::R32Float, None);
        let ssao_main = ssao_pipe("ssao", "fs_ssao", wgpu::TextureFormat::Rg32Float, None);
        let ssao_blur_x = ssao_pipe("ssao blur across", "fs_ssao_blur_x", wgpu::TextureFormat::Rg32Float, None);
        let ssao_blur_y = ssao_pipe("ssao blur down", "fs_ssao_blur_y", wgpu::TextureFormat::Rg32Float, None);
        // SSAOApply: the scene × the occlusion.
        let ssao_apply = ssao_pipe(
            "ssao apply",
            "fs_ssao_apply",
            HDR_FORMAT,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::Src,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            }),
        );
        let lines = make(
            "lines",
            "vs_line",
            "fs_line",
            &[Some(line_layout)],
            T::LineList,
            None,
            false,
            C::GreaterEqual,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let meshes = build_meshes()
            .into_iter()
            .map(|verts| Mesh {
                count: verts.len() as u32,
                vbuf: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh"),
                    contents: bytemuck::cast_slice(&verts),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            })
            .collect();

        let instance_cap = 1024;
        let line_cap = 4096;
        let mut r = Renderer {
            gui_fog: ([0.0, 1e9, 0.0, 0.0], [0.0; 3]),
            gui_sun: Default::default(),
            gui_lightgrid: [0.0; 4],
            gui_sun_params: [0.0, 580.0, 0.05, 0.0],
            gui_n_lights: 0,
            world_gui: None,
            water,
            glass_layout,
            glass_resolve,
            material_epoch: Instant::now(),
            sky,
            opaque,
            opaque_two_sided,
            transparent_culled,
            glass,
            glass_culled,
            transparent,
            lines,
            post,
            post_dof: false,
            ssao_layout,
            ssao_down,
            ssao_main,
            dof_h,
            dof_v,
            ssao_blur_x,
            ssao_blur_y,
            ssao_apply,
            bloom_bright,
            bloom_down,
            bloom_up,
            post_bloom_size: 24.0,
            shadow,
            terrain,
            terrain_shadow,
            terrain_params_buf,
            terrain_bg,
            terrain_chunks: HashMap::new(),
            terrain_grass: HashMap::new(),
            globals_buf,
            globals_bg,
            shadow_globals_bg,
            lshadow,
            lshadow_pipe,
            static_gen: 0,
            globals_layout: bgl,
            lights_buf,
            shadow_view,
            sun,
            sky_view,
            indoor_view,
            brdf_view,
            sky_sampler,
            lightgrid_tex,
            lightgrid_view,
            localgrid_tex,
            localgrid_view,
            lightocc_tex,
            lightocc_view,
            studs_view,
            studs_sampler,
            materials,
            sky_size: 0,
            meshes,
            tex_layout,
            sampler,
            textures: Vec::new(),
            black_view,
            overlay,
            instance_buf: Self::make_buf(
                device,
                "instances",
                instance_cap * std::mem::size_of::<InstanceRaw>(),
            ),
            instance_cap,
            static_buf: Self::make_buf(
                device,
                "static instances",
                instance_cap * std::mem::size_of::<InstanceRaw>(),
            ),
            static_cap: instance_cap,
            items: HashMap::new(),
            per_frame: HashSet::new(),
            st: StaticLayout::default(),
            pending: 0,
            pending_at: None,
            last_rebuild: Instant::now(),
            last_quiet_scan: Instant::now(),
            stats: RenderStats::default(),
            line_buf: Self::make_buf(
                device,
                "lines",
                line_cap * std::mem::size_of::<LineVertex>(),
            ),
            line_cap,
            target: None,
            texture_id: None,
        };
        r.add_texture(device, queue, 1, 1, &[255, 255, 255, 255]);
        r
    }

    /// The last frame as shown (RGBA8, top row first): width, height,
    /// pixels. Waits for the GPU; for tests.
    pub fn read_frame(&self, rs: &egui_wgpu::RenderState) -> Option<(u32, u32, Vec<u8>)> {
        let target = self.target.as_ref()?;
        let [w, h] = target.size;
        let row = (w * 4).div_ceil(256) * 256;
        let buf = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read frame"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("read frame"),
            });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.display,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        rs.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        rs.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let o = (y * row) as usize;
            out.extend_from_slice(&data[o..o + (w * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        Some((w, h, out))
    }

    /// Raw RGBA16Float scene before bloom/tone, for render diagnostics.
    pub fn read_hdr(&self, rs: &egui_wgpu::RenderState) -> Option<(u32, u32, Vec<u8>)> {
        let target = self.target.as_ref()?;
        let [w, h] = target.size;
        let row = (w * 8).div_ceil(256) * 256;
        let buf = rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read HDR"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("read HDR"),
            });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.hdr,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        rs.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        rs.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 8) as usize);
        for y in 0..h {
            let o = (y * row) as usize;
            out.extend_from_slice(&data[o..o + (w * 8) as usize]);
        }
        drop(data);
        buf.unmap();
        Some((w, h, out))
    }

    /// Uploads an RGBA8 (sRGB) image; returns its index for [`PartDraw::texture`].
    pub fn add_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> u32 {
        self.add_texture_emissive(device, queue, width, height, rgba, None)
    }

    /// A texture with a SurfaceAppearance emissive mask (RGBA8, its own size;
    /// read as data, not sRGB).
    pub fn add_texture_emissive(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        rgba: &[u8],
        mask: Option<(u32, u32, &[u8])>,
    ) -> u32 {
        self.add_texture_surface(device, queue, width, height, rgba, mask, None)
    }

    pub fn add_texture_surface(
        &mut self, device: &wgpu::Device, queue: &wgpu::Queue,
        width: u32, height: u32, rgba: &[u8],
        mask: Option<(u32, u32, &[u8])>, surface: Option<(u32, u32, &[u8])>,
    ) -> u32 {
        let (sw, sh, sp) = surface.unwrap_or((1, 1, &[128, 128, 255, 0]));
        let surface_view = device.create_texture_with_data(queue, &wgpu::TextureDescriptor {
            label: Some("SurfaceAppearance normal roughness metalness"),
            size: wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
            mip_level_count: mip_levels(sw, sh), sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }, wgpu::util::TextureDataOrder::LayerMajor, &data_mip_chain(sw, sh, sp))
            .create_view(&Default::default());
        let mask_view = mask.map(|(w, h, px)| {
            device
                .create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some("emissive mask"),
                        size: wgpu::Extent3d {
                            width: w.max(1),
                            height: h.max(1),
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    px,
                )
                .create_view(&Default::default())
        });
        let (w, h) = (width.max(1), height.max(1));
        let mips = if rgba.len() == (w * h * 4) as usize {
            mip_chain(w, h, rgba)
        } else {
            rgba.to_vec()
        };
        let levels = if rgba.len() == (w * h * 4) as usize {
            mip_levels(w, h)
        } else {
            1
        };
        let tex = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("part texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &mips,
        );
        let view = tex.create_view(&Default::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("part texture"),
            layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        mask_view.as_ref().unwrap_or(&self.black_view),
                    ),
                },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&surface_view) },
            ],
        });
        // An EditableImage texture re-uploaded: into its old slot (the old
        // bind group and its textures are freed), not a new one.
        if let Some(i) = REUSE_TEXTURE_SLOT.with(|c| c.take()) {
            if let Some(slot) = self.textures.get_mut(i as usize) {
                *slot = bg;
                return i;
            }
        }
        self.textures.push(bg);
        (self.textures.len() - 1) as u32
    }

    #[allow(clippy::too_many_arguments)]
    fn globals_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        globals: &wgpu::Buffer,
        lights: &wgpu::Buffer,
        sky: &wgpu::TextureView,
        sky_sampler: &wgpu::Sampler,
        shadow: &wgpu::TextureView,
        lightgrid: &wgpu::TextureView,
        lightocc: &wgpu::TextureView,
        localgrid: &wgpu::TextureView,
        studs: &wgpu::TextureView,
        studs_sampler: &wgpu::Sampler,
        materials: &MaterialArrays,
        indoor: &wgpu::TextureView,
        brdf: &wgpu::TextureView,
        lshadow: &LightShadows,
    ) -> wgpu::BindGroup {
        use wgpu::BindingResource as B;
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: B::TextureView(sky),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: B::Sampler(sky_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: B::TextureView(shadow),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: B::TextureView(studs),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: B::Sampler(studs_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: B::TextureView(&materials.albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: B::TextureView(&materials.surface_view),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: B::TextureView(lightgrid),
                },
                wgpu::BindGroupEntry {
                    binding: 12,
                    resource: B::TextureView(lightocc),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: B::TextureView(indoor),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: B::TextureView(brdf),
                },
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: B::TextureView(&lshadow.moments_view),
                },
                wgpu::BindGroupEntry {
                    binding: 16,
                    resource: B::Sampler(&lshadow.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 17,
                    resource: lshadow.slots_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 18,
                    resource: materials.params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry { binding: 20, resource: B::TextureView(localgrid) },
                wgpu::BindGroupEntry {
                    binding: 19,
                    resource: B::TextureView(&materials.emission_view),
                },
            ],
        })
    }

    /// Studio's split-sum environment BRDF table: PlatformContent
    /// textures/brdfLUT.dds, 256 × 256 R16G16_FLOAT (DX10 header), read by the
    /// part shaders at (roughness, N·V) as PrecomputedBRDFTexture. Without the
    /// file a 1×1 (1, 0): Fresnel F0 alone.
    fn brdf_lut_view(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        let file = crate::assets::resolve("rbxasset://textures/brdfLUT.dds", None)
            .and_then(|p| std::fs::read(p).ok());
        let parsed = file.and_then(|b| {
            if b.len() < 148 || &b[0..4] != b"DDS " || &b[84..88] != b"DX10" {
                return None;
            }
            let h = u32::from_le_bytes(b[12..16].try_into().ok()?);
            let w = u32::from_le_bytes(b[16..20].try_into().ok()?);
            let fmt = u32::from_le_bytes(b[128..132].try_into().ok()?);
            let n = (w * h * 4) as usize;
            // DXGI_FORMAT_R16G16_FLOAT = 34.
            (fmt == 34 && b.len() >= 148 + n).then(|| (w, h, b[148..148 + n].to_vec()))
        });
        // (f16 1.0 = 0x3C00)
        let (w, h, data) = parsed.unwrap_or((1, 1, vec![0x00, 0x3C, 0, 0]));
        device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("brdf lut"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rg16Float,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &data,
            )
            .create_view(&Default::default())
    }

    /// Sets Studio's indoor environment cube (textures/sky/indoor512, faces
    /// in wgpu's cube order like `set_sky`).
    pub fn set_indoor(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, size: u32, faces: &[Vec<u8>; 6]) {
        self.indoor_view = Self::cube_view(device, queue, size, faces);
        self.rebind_globals(device);
    }

    /// A cube texture from six square RGBA8 faces in wgpu's order
    /// (+X, −X, +Y, −Y, +Z, −Z). Texels stay gamma-encoded; the shader
    /// squares them as Studio does.
    fn cube_view(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        faces: &[Vec<u8>; 6],
    ) -> wgpu::TextureView {
        // Each face with its mip chain (layer-major): the sky's reflection
        // on rough parts reads a blurred level (shader.wgsl sky_rough).
        let data = cube_mips(size, faces);
        Self::cube_view_mipped(device, queue, size, &data)
    }

    /// A cube texture from faces already with their mip chains (cube_mips).
    fn cube_view_mipped(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        data: &[u8],
    ) -> wgpu::TextureView {
        let tex = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("sky"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 6,
                },
                mip_level_count: mip_levels(size, size),
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            data,
        );
        tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        })
    }

    fn rebind_globals(&mut self, device: &wgpu::Device) {
        self.globals_bg = Self::globals_group(
            device,
            &self.globals_layout,
            &self.globals_buf,
            &self.lights_buf,
            &self.sky_view,
            &self.sky_sampler,
            &self.sun.atlas_view,
            &self.lightgrid_view,
            &self.lightocc_view,
            &self.localgrid_view,
            &self.studs_view,
            &self.studs_sampler,
            &self.materials,
            &self.indoor_view,
            &self.brdf_view,
            &self.lshadow,
        );
    }

    /// The legacy surface atlas (RGBA8, kept as data: the shader reads its
    /// grey 0.5 as "no change"), with mips averaged in linear light.
    fn studs_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> wgpu::TextureView {
        device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("studs"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: mip_levels(w, h),
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &mip_chain(w, h, rgba),
            )
            .create_view(&Default::default())
    }

    /// Studio's studs.dds (PlatformContent/pc/textures): 128 px wide, four
    /// 512 px bands (studs, weld, inlet, universal), 2 × 8 studs each.
    pub fn set_studs(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) {
        self.studs_view = Self::studs_texture(device, queue, w, h, rgba);
        self.rebind_globals(device);
    }

    /// Replaces one terrain chunk's triangles (none = the chunk goes).
    pub fn set_terrain_chunk(
        &mut self,
        device: &wgpu::Device,
        key: [i32; 3],
        verts: &[crate::terrain::TerrainVertex],
    ) {
        // (the sun's cascades and the lights' shadow maps hold the terrain)
        self.static_gen += 1;
        if verts.is_empty() {
            self.terrain_chunks.remove(&key);
            return;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for v in verts {
            lo = lo.min(Vec3::from(v.pos));
            hi = hi.max(Vec3::from(v.pos));
        }
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain chunk"),
            contents: bytemuck::cast_slice(verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        self.terrain_chunks.insert(
            key,
            TerrainChunk {
                vbuf,
                count: verts.len() as u32,
                lo,
                hi,
            },
        );
    }

    /// Drops every terrain chunk (another place, or no terrain).
    pub fn clear_terrain(&mut self) {
        self.terrain_chunks.clear();
        self.terrain_grass.clear();
        self.static_gen += 1;
    }

    /// Replaces one chunk's decoration grass (none = drop it).
    pub fn set_terrain_grass(
        &mut self,
        device: &wgpu::Device,
        key: [i32; 3],
        verts: &[crate::terrain::TerrainVertex],
    ) {
        let had = self.terrain_chunks.remove(&key);
        self.set_terrain_chunk(device, key, verts);
        match self.terrain_chunks.remove(&key) {
            Some(c) => {
                self.terrain_grass.insert(key, c);
            }
            None => {
                self.terrain_grass.remove(&key);
            }
        }
        if let Some(h) = had {
            self.terrain_chunks.insert(key, h);
        }
    }

    /// Triangles in the drawn terrain chunks.
    pub fn terrain_triangles(&self) -> usize {
        self.terrain_chunks.values().map(|c| c.count as usize / 3).sum()
    }

    /// The terrain shader's per-palette parameters (4 vec4s per entry, see
    /// shader.wgsl TerrainParams).
    pub fn set_terrain_params(&self, queue: &wgpu::Queue, params: &[[f32; 4]]) {
        let n = params.len().min(TERRAIN_PALETTE * 4);
        queue.write_buffer(&self.terrain_params_buf, 0, bytemuck::cast_slice(&params[..n]));
    }

    /// The texture array layer of a material pack, uploading it (and
    /// growing the arrays) the first time.
    pub fn material_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        maps: &std::sync::Arc<crate::matpack::Maps>,
    ) -> i32 {
        if let Some(l) = self.materials.layers.get(&maps.pack) {
            return *l as i32;
        }
        let layer = self.materials.cpu.len() as u32;
        self.materials.cpu.push(maps.clone());
        self.materials.layers.insert(maps.pack, layer);
        if layer >= self.materials.capacity || self.materials.size != crate::matpack::LAYER_SIZE {
            let cap = (layer + 1).next_power_of_two().max(4);
            let params = std::mem::take(&mut self.materials.params);
            let mut grown = MaterialArrays::with_params(device, crate::matpack::LAYER_SIZE, cap, params);
            grown.layers = std::mem::take(&mut self.materials.layers);
            grown.cpu = std::mem::take(&mut self.materials.cpu);
            for (i, m) in grown.cpu.iter().enumerate() {
                grown.upload(queue, i as u32, m);
            }
            self.materials = grown;
            self.rebind_globals(device);
        } else {
            self.materials.upload(queue, layer, maps);
        }
        layer as i32
    }

    /// Studio's material constants for the parts drawn with material layer
    /// `layer` (see `MaterialArrays`): written when they change.
    pub fn set_material_params(&mut self, queue: &wgpu::Queue, layer: i32, p: [[f32; 4]; 2]) {
        let Ok(l) = usize::try_from(layer) else {
            return;
        };
        if l >= MAT_PARAM_LAYERS { return; }
        let mut full = [p[0], p[1], [1.0, 1.0, 1.0, 0.0], [0.0; 4], [1.0, 1.0, 1.0, 0.0]];
        full[1][2] = 4.0;
        full[1][3] = 1.0;
        if let Some(c) = self.materials.cpu.get(l).and_then(|m| crate::matpack::full_consts(m.pack)) {
            full[1][2] = c[0];
            full[1][3] = c[16];
            full[2] = [c[6], c[7], c[8], c[17]];
            full[3] = [c[9], c[10], c[11], c[18]];
            full[4] = [c[13], c[14], c[15], c[19]];
        }
        if self.materials.params[l * 5..l * 5 + 5] == full { return; }
        self.materials.params[l * 5..l * 5 + 5].copy_from_slice(&full);
        queue.write_buffer(&self.materials.params_buf, (l * 5 * 16) as u64, bytemuck::cast_slice(&full));
    }

    /// Sets the skybox: six square RGBA8 faces of `size` in wgpu's cube
    /// order (+X, −X, +Y, −Y, +Z, −Z). `size` 0 = no skybox.
    /// `mipped`: the six faces with their mip chains, layer-major (cube_mips).
    pub fn set_sky(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        mipped: Option<&[u8]>,
    ) {
        match mipped {
            Some(f) if size > 0 => {
                self.sky_view = Self::cube_view_mipped(device, queue, size, f);
                self.sky_size = size;
            }
            _ => self.sky_size = 0,
        }
        self.rebind_globals(device);
    }

    /// Uploads the light grid (lightgrid.rs: skylight and sun bytes, X, then Z, then Y).
    pub fn set_lightgrid(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let [nx, ny, nz] = crate::lightgrid::dims().map(|n| n as u32);
        if data.len() != (2 * nx * ny * nz) as usize {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.lightgrid_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(2 * nx),
                rows_per_image: Some(nz),
            },
            wgpu::Extent3d {
                width: nx,
                height: nz,
                depth_or_array_layers: ny,
            },
        );
    }

    pub fn set_localgrid(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let [nx, ny, nz] = crate::lightgrid::dims().map(|n| n as u32);
        if data.len() != (nx * ny * nz * 4) as usize { return; }
        queue.write_texture(wgpu::TexelCopyTextureInfo {
            texture: &self.localgrid_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        }, data, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(nx * 4), rows_per_image: Some(nz) },
        wgpu::Extent3d { width: nx, height: nz, depth_or_array_layers: ny });
    }

    /// Uploads the parts' light occupancy (lightocc.rs: X, then Y, then Z).
    pub fn set_lightocc(&mut self, queue: &wgpu::Queue, data: &[u8]) {
        let [nx, ny, nz] = crate::lightocc::dims().map(|n| n as u32);
        if data.len() != (nx * ny * nz) as usize {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.lightocc_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(nx),
                rows_per_image: Some(ny),
            },
            wgpu::Extent3d {
                width: nx,
                height: ny,
                depth_or_array_layers: nz,
            },
        );
    }

    fn make_buf(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.max(64) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn ensure_target(&mut self, rs: &egui_wgpu::RenderState, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let device = &rs.device;
        let tex =
            |label: &str, format: wgpu::TextureFormat, samples: u32, usage: wgpu::TextureUsages| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
        let msaa = tex(
            "viewport-msaa",
            HDR_FORMAT,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth = tex(
            "viewport-depth",
            DEPTH_FORMAT,
            SAMPLES,
            // (read by the world GUI pass)
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let hdr = tex(
            "viewport-hdr",
            HDR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let hdr_view = hdr.create_view(&Default::default());
        let scene_copy = tex(
            "viewport-scene-copy",
            HDR_FORMAT,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let depth_view = depth.create_view(&Default::default());
        let water_bg = self.water.bind(device, &scene_copy.create_view(&Default::default()), &depth_view);
        let glass_depth = tex("glass depth snapshot", wgpu::TextureFormat::R32Float, 1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
        let glass_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass scene snapshot"), layout: &self.glass_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene_copy.create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&glass_depth) },
            ],
        });
        // Bloom's half-resolution targets.
        let half = [size[0].div_ceil(2).max(1), size[1].div_ceil(2).max(1)];
        let half_tex = |label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: half[0],
                        height: half[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: HDR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let bloom_levels: Vec<_> = (1..=5).map(|level| device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bloom pyramid"), size: wgpu::Extent3d { width: (size[0] >> level).max(1), height: (size[1] >> level).max(1), depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }).create_view(&Default::default())).collect();
        let bloom_view = bloom_levels[0].clone();
        let dof_views = [half_tex("dof across"), half_tex("dof down")];
        let pair = |label: &str, a: &wgpu::TextureView, b: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.tex_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(a),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sky_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(b),
                    },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.black_view) },
                ],
            })
        };
        let bloom_down_bgs = (0..5).map(|i| pair("bloom pyramid down", if i == 0 { &hdr_view } else { &bloom_levels[i-1] }, &hdr_view)).collect();
        let bloom_up_bgs = (1..5).map(|i| pair("bloom pyramid up", &bloom_levels[i], &hdr_view)).collect();
        let dof_bgs = [pair("dof across", &hdr_view, &self.black_view), pair("dof down", &dof_views[0], &self.black_view)];
        let hdr_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post inputs"), layout: &self.tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&hdr_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sky_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&bloom_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&dof_views[1]) },
            ],
        });
        let half_float = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: half[0], height: half[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let ssao_depth_view = half_float("ssao depth", wgpu::TextureFormat::R32Float);
        let ssao_views = [
            half_float("ssao a", wgpu::TextureFormat::Rg32Float),
            half_float("ssao b", wgpu::TextureFormat::Rg32Float),
        ];
        let ssao_bg = |label: &str, input: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.ssao_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth_view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(input) },
                ],
            })
        };
        // down (input unused) → a … occlusion reads the depth → a, blur
        // across a → b, blur down b → a, apply reads a.
        let ssao_bgs = [
            ssao_bg("ssao down", &ssao_views[1]),
            ssao_bg("ssao", &ssao_depth_view),
            ssao_bg("ssao blur across", &ssao_views[0]),
            ssao_bg("ssao blur down", &ssao_views[1]),
        ];
        let display = tex(
            "viewport",
            DISPLAY_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let display_view = display.create_view(&Default::default());
        let mut egui_renderer = rs.renderer.write();
        match self.texture_id {
            Some(id) => egui_renderer.update_egui_texture_from_wgpu_texture(
                device,
                &display_view,
                wgpu::FilterMode::Linear,
                id,
            ),
            None => {
                self.texture_id = Some(egui_renderer.register_native_texture(
                    device,
                    &display_view,
                    wgpu::FilterMode::Linear,
                ))
            }
        }
        self.target = Some(Target {
            size,
            msaa: msaa.create_view(&Default::default()),
            depth: depth_view,
            hdr,
            hdr_view,
            scene_copy,
            water_bg,
            glass_bg,
            glass_depth,
            hdr_bg,
            dof_views,
            dof_bgs,
            bloom_view,
            bloom_levels,
            bloom_down_bgs,
            bloom_up_bgs,
            ssao_depth_view,
            ssao_views,
            ssao_bgs,
            display,
            display_view,
        });
    }

    /// World GUIs and effects enter the HDR scene before bloom, DoF and
    /// colour correction (UILitFogFS / ParticleCustomFS). AlwaysOnTop and
    /// screen GUIs are composed separately by egui.
    pub fn draw_world_gui(
        &mut self,
        rs: &egui_wgpu::RenderState,
        batches: &[crate::world_gui::GuiBatch],
        fx: Option<&crate::world_gui::FxFrame>,
        tone_curve: Option<[f32; 5]>,
    ) {
        let Some(t) = self.target.as_ref() else { return };
        let pass = self
            .world_gui
            .get_or_insert_with(|| crate::world_gui::WorldGuiPass::new(&rs.device, &rs.queue, DISPLAY_FORMAT));
        pass.sun_atlas = self.sun.atlas_view.clone();
        pass.sun_grid = self.lightgrid_view.clone();
        pass.lightgrid = self.gui_lightgrid;
        pass.sun_params = self.gui_sun_params;
        pass.sun_cascades = self.gui_sun;
        // (effects light themselves from the frame's local lights)
        pass.lights = self.lights_buf.clone();
        pass.n_lights = self.gui_n_lights;
        if pass.effects(rs, &t.hdr_view, t.size, &t.depth, batches, fx, tone_curve, self.gui_fog.0, self.gui_fog.1) {
            self.finish_again(rs);
        }
    }

    /// The bloom and the final image again, from the HDR image as it is now
    /// (effects drawn into it after `render`; the frame's globals still
    /// bound).
    fn encode_bloom(&self, encoder: &mut wgpu::CommandEncoder, target: &Target, size: f32) {
        let scale = ((target.size[0] as f32 * target.size[1] as f32) / 2073600.0).sqrt();
        let levels = (3.0 + (scale.log2() + (size / 7.0).log2().max(0.0)).max(-1.0)).min(5.999) as usize;
        for i in 0..levels {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom pyramid down"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &target.bloom_levels[i], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(if i == 0 { &self.bloom_bright } else { &self.bloom_down });
            pass.set_bind_group(0, &self.globals_bg, &[]); pass.set_bind_group(1, &target.bloom_down_bgs[i], &[]); pass.draw(0..3, 0..1);
        }
        for i in (1..levels).rev() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom pyramid up"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &target.bloom_levels[i-1], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(&self.bloom_up); pass.set_bind_group(0, &self.globals_bg, &[]); pass.set_bind_group(1, &target.bloom_up_bgs[i-1], &[]); pass.draw(0..3, 0..1);
        }
    }

    fn finish_again(&self, rs: &egui_wgpu::RenderState) {
        let _b = crate::bench::scope("fx_post_again");
        let Some(target) = self.target.as_ref() else { return };
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("effects post") });
        self.encode_bloom(&mut encoder, target, self.post_bloom_size);
        let steps: [(&str, &wgpu::RenderPipeline, &wgpu::BindGroup, Vec<&wgpu::TextureView>); 3] = [
            ("dof across", &self.dof_h, &target.dof_bgs[0], vec![&target.dof_views[0]]),
            ("dof down", &self.dof_v, &target.dof_bgs[1], vec![&target.dof_views[1]]),
            ("post", &self.post, &target.hdr_bg, vec![&target.display_view]),
        ];
        for (label, pipeline, bg, views) in steps {
            if label.starts_with("dof ") && !self.post_dof { continue; }
            let attachments: Vec<_> = views
                .iter()
                .map(|v| {
                    Some(wgpu::RenderPassColorAttachment {
                        view: v,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })
                })
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(1, bg, &[]);
            if label == "post" || label == "dof across" { pass.set_bind_group(2, &target.ssao_bgs[0], &[]); }
            pass.draw(0..3, 0..1);
        }
        rs.queue.submit([encoder.finish()]);
    }

    /// The last frame's fog (`Globals::fog`) and its linear colour, as
    /// shader.wgsl fog_rgb works it out (world GUIs are fogged the same).
    fn gui_fog(g: &Globals) -> ([f32; 4], [f32; 3]) {
        let c = [g.fog_color[0], g.fog_color[1], g.fog_color[2]];
        if g.fog[2] > 1.5 && g.fog[2] < 2.5 {
            let k = |i: usize| g.ambient[i] + g.outdoor[i] + g.sun_color[i] * g.sun_dir[3] * g.sun_dir[1].max(0.0);
            return (g.fog, [c[0] * k(0), c[1] * k(1), c[2] * k(2)]);
        }
        if g.fog[2] > 2.5 {
            // Atmosphere: the haze band at the horizon (the sky behind a
            // world GUI is close to it at the distances fog shows).
            return (g.fog, [g.atmos[0], g.atmos[1], g.atmos[2]]);
        }
        (g.fog, c)
    }

    /// Renders a frame at `size` physical pixels; returns the texture to show.
    pub fn render(
        &mut self,
        rs: &egui_wgpu::RenderState,
        size: [u32; 2],
        frame: &Frame,
    ) -> Option<egui::TextureId> {
        let svc_t0 = Instant::now();
        let size = [size[0].clamp(1, 8192), size[1].clamp(1, 8192)];
        self.ensure_target(rs, size);
        let device = &rs.device;
        let queue = &rs.queue;
        // cam_pos.w is unused by the camera math. Keep material animation
        // continuous across scene rebuilds, independently of lighting time.
        let mut globals = frame.globals;
        self.post_bloom_size = globals.bloom[1];
        self.post_dof = globals.dof[2] > 0.0 || globals.dof[3] > 0.0;
        globals.cam_pos[3] = self.material_epoch.elapsed().as_secs_f32();
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        self.gui_fog = Self::gui_fog(&frame.globals);
        self.gui_lightgrid = frame.globals.lightgrid;
        self.gui_sun_params = [if frame.globals.sun_color[3] < 0.5 { 1.0 } else { 0.0 }, crate::mech::SUN_DIST, crate::mech::HYBRID_LERP_SLOPE, if frame.shadows { 1.0 } else { 0.0 }];
        self.gui_sun = crate::world_gui::SunCascades { local: frame.globals.sun_casc, sphere: frame.globals.sun_sphere, rect: frame.globals.sun_rect };
        let mut stats = RenderStats {
            rebuilds: self.stats.rebuilds,
            ..Default::default()
        };

        self.requeue_quiet();
        // New parts join the static batches once they've settled.
        let settled = self
            .pending_at
            .is_none_or(|t| t.elapsed().as_secs_f32() >= crate::mech::STATIC_SETTLE_S);
        let compact = self.st.hidden > 1024 && self.st.hidden * 4 > self.st.raw.len();
        if (self.pending > 0
            && (self.st.batches.is_empty()
                || ((settled || self.pending > crate::mech::PENDING_MAX)
                    && self.last_rebuild.elapsed().as_secs_f32() >= crate::mech::STATIC_REBUILD_MIN_S)))
            || compact
        {
            self.last_rebuild = Instant::now();
            let _p = crate::bench::scope("render_static_rebuild");
            self.rebuild_static(device, queue);
            stats.rebuilds += 1;
        }
        self.flush_static_writes(queue);

        let cam = Vec3::new(
            frame.globals.cam_pos[0],
            frame.globals.cam_pos[1],
            frame.globals.cam_pos[2],
        );
        let planes = frustum(frame.cull_view_proj);
        // A part whose bounding radius covers less than LOD_MIN_PX pixels is skipped.
        let focal = size[1] as f32 * 0.5 / (frame.fov_y * 0.5).tan().max(1e-3);
        let lod_k = crate::mech::LOD_MIN_PX / focal;

        // Static batches: frustum test per cell, then trim the smallest parts.
        let mut static_ranges: Vec<(Pass, (usize, usize), u32, u32)> = Vec::new();
        for b in &self.st.batches {
            if !box_visible(&planes, b.lo, b.hi) {
                stats.culled_batches += 1;
                continue;
            }
            let r_min = lod_k * box_distance(cam, b.lo, b.hi);
            let radii = &self.st.radii[b.start as usize..(b.start + b.count) as usize];
            let n = radii.partition_point(|r| *r >= r_min) as u32;
            stats.lod_skipped += (b.count - n) as usize;
            if n == 0 {
                continue;
            }
            match static_ranges.last_mut() {
                Some(last) if last.0 == b.pass && last.1 == b.key && last.3 == b.start => {
                    last.3 = b.start + n
                }
                _ => static_ranges.push((b.pass, b.key, b.start, b.start + n)),
            }
        }

        // Per-frame items: moving, new and see-through parts.
        let mut dyn_opaque: Vec<((usize, usize), InstanceRaw)> = Vec::new();
        let mut dyn_decal: Vec<((usize, usize), InstanceRaw)> = Vec::new();
        let mut blended: Vec<(f32, (usize, usize), InstanceRaw)> = Vec::new();
        let mut water: Vec<((usize, usize), InstanceRaw)> = Vec::new();
        for r in &self.per_frame {
            let Some(item) = self.items.get(r) else {
                continue;
            };
            for d in &item.draws {
                let rad = radius_of(d);
                if !shows(d) || !sphere_visible(&planes, d.xf.pos, rad) {
                    continue;
                }
                let dist = (d.xf.pos - cam).length();
                if rad < lod_k * (dist - rad).max(0.0) {
                    stats.lod_skipped += 1;
                    continue;
                }
                let raw = to_raw(d, item.bias);
                match pass_of(d) {
                    Some(Pass::Opaque) => dyn_opaque.push((draw_key(d), raw)),
                    Some(Pass::Decal) => dyn_decal.push((draw_key(d), raw)),
                    // A see-through mesh or union is drawn two-sided (its
                    // winding isn't trusted): from inside its box every face
                    // Studio would draw faces away, so nothing shows.
                    // (Not terrain water, material 5: one mesh whose box can
                    // span the map, its surface seen from anywhere.)
                    None if d.custom_mesh.is_some() && d.material != 5.0 && camera_inside(d, cam) => {}
                    // Terrain water: its own pass (water.rs).
                    None if d.custom_mesh.is_some() && d.material == 5.0 => water.push((draw_key(d), raw)),
                    None => blended.push((dist, draw_key(d), raw)),
                }
            }
        }
        dyn_opaque.sort_by_key(|e| e.0);
        dyn_decal.sort_by_key(|e| e.0);
        // Glass samples the opaque scene and must precede other transparents.
        // Preserve the existing back-to-front distance order within each group.
        blended.sort_by(|a, b| transparent_order(a.2.extra[0], a.0, b.2.extra[0], b.0));
        let overlay: Vec<((usize, usize), InstanceRaw)> = frame
            .overlay
            .iter()
            .filter(|d| shows(d))
            .map(|d| (draw_key(d), to_raw(&PartDraw { hover: true, ..*d }, 0.0)))
            .collect();
        let raw: Vec<InstanceRaw> = dyn_opaque
            .iter()
            .map(|e| e.1)
            .chain(dyn_decal.iter().map(|e| e.1))
            .chain(blended.iter().map(|e| e.2))
            .chain(overlay.iter().map(|e| e.1))
            .chain(water.iter().map(|e| e.1))
            .collect();
        if raw.len() > self.instance_cap {
            self.instance_cap = raw.len().next_power_of_two();
            self.instance_buf = Self::make_buf(
                device,
                "instances",
                self.instance_cap * std::mem::size_of::<InstanceRaw>(),
            );
        }
        if !raw.is_empty() {
            queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(&raw));
        }
        // Runs of equal keys in a per-frame list → (key, start, end) in `raw`.
        let runs = |keys: &mut dyn Iterator<Item = (usize, usize)>,
                    base: u32|
         -> Vec<((usize, usize), u32, u32)> {
            let mut out: Vec<((usize, usize), u32, u32)> = Vec::new();
            for (i, k) in keys.enumerate() {
                let at = base + i as u32;
                match out.last_mut() {
                    Some(l) if l.0 == k && l.2 == at => l.2 = at + 1,
                    _ => out.push((k, at, at + 1)),
                }
            }
            out
        };
        let mut base = 0u32;
        let opaque_runs = runs(&mut dyn_opaque.iter().map(|e| e.0), base);
        base += dyn_opaque.len() as u32;
        let decal_runs = runs(&mut dyn_decal.iter().map(|e| e.0), base);
        base += dyn_decal.len() as u32;
        let glass_count = blended.iter().take_while(|e| (e.2.extra[0] as u32 & 7) == 3).count();
        let glass_end = base + glass_count as u32;
        let mut blend_runs = runs(&mut blended[..glass_count].iter().map(|e| e.1), base);
        blend_runs.extend(runs(&mut blended[glass_count..].iter().map(|e| e.1), glass_end));
        base += blended.len() as u32;
        let overlay_runs = runs(&mut overlay.iter().map(|e| e.0), base);
        base += overlay.len() as u32;
        let water_runs = runs(&mut water.iter().map(|e| e.0), base);
        stats.items = self.items.len();
        stats.static_instances = self.st.raw.len() - self.st.hidden;
        stats.batches = self.st.batches.len();
        stats.per_frame_items = self.per_frame.len();
        stats.drawn_instances = static_ranges
            .iter()
            .map(|r| (r.3 - r.2) as usize)
            .sum::<usize>()
            + raw.len();
        if frame.lines.len() > self.line_cap {
            self.line_cap = frame.lines.len().next_power_of_two();
            self.line_buf = Self::make_buf(
                device,
                "lines",
                self.line_cap * std::mem::size_of::<LineVertex>(),
            );
        }
        if !frame.lines.is_empty() {
            queue.write_buffer(&self.line_buf, 0, bytemuck::cast_slice(frame.lines));
        }

        let n_lights = frame.lights.len().min(crate::mech::MAX_LIGHTS);
        self.gui_n_lights = n_lights as u32;
        if n_lights > 0 {
            queue.write_buffer(
                &self.lights_buf,
                0,
                bytemuck::cast_slice(&frame.lights[..n_lights]),
            );
        }

        // The sun's shadow cascades (sun_shadow.rs): which to redraw.
        let scene_gen = self.static_gen;
        let sun_due = if frame.shadows {
            self.sun.upload(queue, &frame.sun_cascades);
            self.sun.plan(&frame.sun_cascades, scene_gen, !opaque_runs.is_empty())
        } else {
            self.sun.forget();
            [false; crate::sun_shadow::CASCADES]
        };

        let target = self.target.as_ref()?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport"),
        });
        for (ci, cascade) in frame.sun_cascades.iter().enumerate() {
            if !sun_due[ci] {
                continue;
            }
            // Casters: static opaque cells that can reach the cascade's box
            // (trimmed like the view), plus this frame's moving opaque parts.
            let sp = frustum(cascade.caster_view_proj());
            let mut shadow_ranges: Vec<((usize, usize), u32, u32)> = Vec::new();
            for b in self.st.batches.iter().filter(|b| b.pass == Pass::Opaque) {
                if !box_visible(&sp, b.lo, b.hi) {
                    continue;
                }
                let r_min = lod_k * box_distance(cam, b.lo, b.hi) * crate::mech::SHADOW_LOD_SCALE;
                let radii = &self.st.radii[b.start as usize..(b.start + b.count) as usize];
                let n = radii.partition_point(|r| *r >= r_min) as u32;
                if n == 0 {
                    continue;
                }
                match shadow_ranges.last_mut() {
                    Some(last) if last.0 == b.key && last.2 == b.start => last.2 = b.start + n,
                    _ => shadow_ranges.push((b.key, b.start, b.start + n)),
                }
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sun shadow cascade"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let side = cascade.tile.min(SHADOW_SIZE);
                pass.set_viewport(0.0, 0.0, side as f32, side as f32, 0.0, 1.0);
                pass.set_scissor_rect(0, 0, side, side);
                pass.set_pipeline(&self.shadow);
                pass.set_bind_group(0, &self.shadow_globals_bg, &[]);
                self.sun.bind(&mut pass, ci);
                let cast =
                    |pass: &mut wgpu::RenderPass, key: (usize, usize), range: std::ops::Range<u32>| {
                        if let Some(mesh) = self.meshes.get(key.0) {
                            crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::SHADOW, 1, (mesh.count / 3) as u64 * (range.end - range.start) as u64);
                            pass.set_vertex_buffer(0, mesh.vbuf.slice(..));
                            pass.draw(0..mesh.count, range);
                        }
                    };
                pass.set_vertex_buffer(1, self.static_buf.slice(..));
                for r in &shadow_ranges {
                    cast(&mut pass, r.0, r.1..r.2);
                }
                pass.set_vertex_buffer(1, self.instance_buf.slice(..));
                for r in &opaque_runs {
                    cast(&mut pass, r.0, r.1..r.2);
                }
                if !self.terrain_chunks.is_empty() {
                    pass.set_pipeline(&self.terrain_shadow);
                    for c in self.terrain_chunks.values() {
                        if box_visible(&sp, c.lo, c.hi) {
                            crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::SHADOW, 1, (c.count / 3) as u64);
                            pass.set_vertex_buffer(0, c.vbuf.slice(..));
                            pass.draw(0..c.count, 0..1);
                        }
                    }
                }
            }
            self.sun.blur(&mut encoder, cascade, ci);
        }
        // Local light shadow maps (Studio's Future path): the lights the
        // light list gave a slot (LightRaw.extra.w), each a cube of EVSM
        // moments; redrawn only when those lights or the static scene change.
        {
            let slots: Vec<(usize, Vec3, f32)> = frame.lights[..n_lights]
                .iter()
                .filter(|l| l.extra[3] >= 0.0)
                .map(|l| (l.extra[3] as usize, Vec3::new(l.pos_range[0], l.pos_range[1], l.pos_range[2]), l.pos_range[3]))
                .filter(|s| s.0 < crate::mech::LSHADOW_MAX_LIGHTS)
                .collect();
            let key = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                self.static_gen.hash(&mut h);
                for (i, p, r) in &slots {
                    i.hash(&mut h);
                    p.to_array().map(f32::to_bits).hash(&mut h);
                    r.to_bits().hash(&mut h);
                }
                h.finish()
            };
            if !slots.is_empty() && self.lshadow.key != Some(key) {
                self.lshadow.key = Some(key);
                let n = crate::mech::LSHADOW_MAX_LIGHTS;
                let mut slot_data = vec![0f32; n * 100];
                let mut face_data = vec![0u8; n * 6 * LSHADOW_FACE_STRIDE as usize];
                for (slot, pos, range) in &slots {
                    let inv = 1.0 / range.max(1e-3);
                    for (f, (vp, axis)) in LightShadows::faces(*pos, *range).iter().enumerate() {
                        slot_data[slot * 100 + f * 16..slot * 100 + f * 16 + 16].copy_from_slice(&vp.to_cols_array());
                        let mut u = [0f32; 24];
                        u[..16].copy_from_slice(&vp.to_cols_array());
                        u[16..20].copy_from_slice(&[pos.x, pos.y, pos.z, inv]);
                        u[20..24].copy_from_slice(&[axis.x, axis.y, axis.z, 0.0]);
                        let o = (slot * 6 + f) * LSHADOW_FACE_STRIDE as usize;
                        face_data[o..o + 96].copy_from_slice(bytemuck::cast_slice(&u));
                    }
                    slot_data[slot * 100 + 96..slot * 100 + 100].copy_from_slice(&[pos.x, pos.y, pos.z, inv]);
                }
                queue.write_buffer(&self.lshadow.slots_buf, 0, bytemuck::cast_slice(&slot_data));
                queue.write_buffer(&self.lshadow.face_buf, 0, &face_data);
                let far_p = crate::mech::EVSM_POS_EXP.exp() as f64;
                let far_n = -(-crate::mech::EVSM_NEG_EXP).exp() as f64;
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("light shadows"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.lshadow.moments_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: far_p, g: far_p * far_p, b: far_n, a: far_n * far_n }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.lshadow.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.lshadow_pipe);
                pass.set_bind_group(0, &self.shadow_globals_bg, &[]);
                pass.set_vertex_buffer(1, self.static_buf.slice(..));
                let cols = crate::mech::LSHADOW_ATLAS / crate::mech::LSHADOW_FACE;
                let face_px = crate::mech::LSHADOW_FACE;
                for (slot, pos, range) in &slots {
                    // The static cells the light reaches.
                    let near: Vec<&Batch> = self
                        .st
                        .batches
                        .iter()
                        .filter(|b| b.pass == Pass::Opaque)
                        .filter(|b| (pos.clamp(b.lo, b.hi) - *pos).length_squared() <= range * range)
                        .collect();
                    for f in 0..6u32 {
                        let k = *slot as u32 * 6 + f;
                        let (x, y) = ((k % cols) * face_px, (k / cols) * face_px);
                        pass.set_viewport(x as f32, y as f32, face_px as f32, face_px as f32, 0.0, 1.0);
                        pass.set_scissor_rect(x, y, face_px, face_px);
                        pass.set_bind_group(1, &self.lshadow.face_bg, &[k * LSHADOW_FACE_STRIDE as u32]);
                        for b in &near {
                            if let Some(mesh) = self.meshes.get(b.key.0) {
                                crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::SHADOW, 1, (mesh.count / 3) as u64 * b.count as u64);
                                pass.set_vertex_buffer(0, mesh.vbuf.slice(..));
                                pass.draw(0..mesh.count, b.start..b.start + b.count);
                            }
                        }
                    }
                }
            }
        }
        // With terrain water in view the scene is drawn in three passes:
        // the opaque scene; then (after copying its colour) the water,
        // which looks through it; then decals and see-through parts.
        let has_glass = blended.iter().any(|e| (e.2.extra[0] as u32 & 7) == 3);
        let split = !water_runs.is_empty() || has_glass;
        let mut calls = 0usize;
        // One instanced draw per run of (mesh, texture).
        let draw = |pass: &mut wgpu::RenderPass,
                    key: (usize, usize),
                    range: std::ops::Range<u32>,
                    calls: &mut usize| {
            let Some(mesh) = self.meshes.get(key.0) else {
                return;
            };
            pass.set_bind_group(1, self.textures.get(key.1).unwrap_or(&self.textures[0]), &[]);
            pass.set_vertex_buffer(0, mesh.vbuf.slice(..));
            crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::SCENE, 1, (mesh.count / 3) as u64 * (range.end - range.start) as u64);
            pass.draw(0..mesh.count, range);
            *calls += 1;
        };
        // Decals, then see-through parts back to front, then the hover overlay.
        let see_through = |pass: &mut wgpu::RenderPass, calls: &mut usize| {
            pass.set_bind_group(2, &target.glass_bg, &[]);
            pass.set_pipeline(&self.transparent);
            pass.set_vertex_buffer(1, self.static_buf.slice(..));
            for r in static_ranges.iter().filter(|r| r.0 == Pass::Decal) {
                draw(pass, r.1, r.2..r.3, calls);
            }
            pass.set_vertex_buffer(1, self.instance_buf.slice(..));
            for r in &decal_runs {
                draw(pass, r.0, r.1..r.2, calls);
            }
            for r in &blend_runs {
                pass.set_pipeline(if r.1 < glass_end {
                    if r.0 .0 < BUILTIN_MESHES { &self.glass_culled } else { &self.glass }
                } else if r.0 .0 < BUILTIN_MESHES {
                    &self.transparent_culled
                } else {
                    &self.transparent
                });
                draw(pass, r.0, r.1..r.2, calls);
            }
            if !overlay_runs.is_empty() {
                pass.set_pipeline(&self.overlay);
                for r in &overlay_runs {
                    draw(pass, r.0, r.1..r.2, calls);
                }
            }
        };
        let scene_pass = |encoder: &mut wgpu::CommandEncoder, label: &str, first: bool, last: bool| {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa,
                    depth_slice: None,
                    resolve_target: Some(&target.hdr_view),
                    ops: wgpu::Operations {
                        load: if first { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load },
                        store: if last { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: if first { wgpu::LoadOp::Clear(0.0) } else { wgpu::LoadOp::Load },
                        // Kept: the world GUI pass tests against it (round 23).
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime()
        };
        {
            let mut pass = scene_pass(&mut encoder, "viewport", true, !split);
            pass.set_bind_group(2, &target.glass_bg, &[]);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(1, &self.textures[0], &[]);
            if frame.sky {
                pass.set_pipeline(&self.sky);
                pass.draw(0..3, 0..1);
            }

            // Loaded meshes may wind either way: draw them two-sided.
            let opaque_pipe = |key: (usize, usize)| {
                if key.0 < BUILTIN_MESHES {
                    &self.opaque
                } else {
                    &self.opaque_two_sided
                }
            };

            pass.set_vertex_buffer(1, self.static_buf.slice(..));
            for r in static_ranges.iter().filter(|r| r.0 == Pass::Opaque) {
                pass.set_pipeline(opaque_pipe(r.1));
                draw(&mut pass, r.1, r.2..r.3, &mut calls);
            }
            pass.set_vertex_buffer(1, self.instance_buf.slice(..));
            for r in &opaque_runs {
                pass.set_pipeline(opaque_pipe(r.0));
                draw(&mut pass, r.0, r.1..r.2, &mut calls);
            }
            // Terrain chunks in view.
            if !self.terrain_chunks.is_empty() {
                pass.set_pipeline(&self.terrain);
                pass.set_bind_group(1, &self.terrain_bg, &[]);
                for c in self.terrain_chunks.values().chain(self.terrain_grass.values()) {
                    if box_visible(&planes, c.lo, c.hi) {
                        crate::runtime::domains::services::perf::draw(crate::runtime::domains::services::perf::SCENE, 1, (c.count / 3) as u64);
                        pass.set_vertex_buffer(0, c.vbuf.slice(..));
                        pass.draw(0..c.count, 0..1);
                    }
                }
                pass.set_bind_group(1, &self.textures[0], &[]);
            }

            if !frame.lines.is_empty() {
                pass.set_bind_group(1, &self.textures[0], &[]);
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, self.line_buf.slice(..));
                pass.draw(0..frame.lines.len() as u32, 0..1);
            }
            if !split {
                see_through(&mut pass, &mut calls);
            }
        }
        if split {
            let u = self.water.uniforms(&frame.water, target.size);
            queue.write_buffer(&self.water.buf, 0, bytemuck::bytes_of(&u));
            encoder.copy_texture_to_texture(
                target.hdr.as_image_copy(),
                target.scene_copy.as_image_copy(),
                wgpu::Extent3d {
                    width: target.size[0],
                    height: target.size[1],
                    depth_or_array_layers: 1,
                },
            );
            if !water_runs.is_empty() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("water"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.msaa,
                        depth_slice: None,
                        resolve_target: Some(&target.hdr_view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.water.pipeline);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_bind_group(2, &target.water_bg, &[]);
                pass.set_vertex_buffer(1, self.instance_buf.slice(..));
                for r in &water_runs {
                    draw(&mut pass, r.0, r.1..r.2, &mut calls);
                }
            }
            // A separate immutable snapshot prevents sampling an active depth attachment.
            if has_glass {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("glass depth snapshot"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &target.glass_depth,
                        depth_slice: None, resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
                });
                pass.set_pipeline(&self.glass_resolve);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_bind_group(1, &target.ssao_bgs[0], &[]);
                pass.draw(0..3, 0..1);
            }
            let mut pass = scene_pass(&mut encoder, "see-through", false, true);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            see_through(&mut pass, &mut calls);
        }
        stats.draw_calls = calls;
        // Bloom: bright pass, blur across, blur down (half resolution).
        // SSAO (Studio's passes), then its multiply into the scene.
        {
            let steps: [(&str, &wgpu::RenderPipeline, usize, &wgpu::TextureView, bool); 5] = [
                ("ssao depth", &self.ssao_down, 0, &target.ssao_depth_view, true),
                ("ssao", &self.ssao_main, 1, &target.ssao_views[0], true),
                ("ssao blur across", &self.ssao_blur_x, 2, &target.ssao_views[1], true),
                ("ssao blur down", &self.ssao_blur_y, 3, &target.ssao_views[0], true),
                ("ssao apply", &self.ssao_apply, 2, &target.hdr_view, false),
            ];
            for (label, pipeline, bg, view, clear) in steps {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if clear { wgpu::LoadOp::Clear(wgpu::Color::WHITE) } else { wgpu::LoadOp::Load },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_bind_group(1, &target.ssao_bgs[bg], &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.encode_bloom(&mut encoder, target, globals.bloom[1]);
        let mut fullscreen = |label: &str,
                              pipeline: &wgpu::RenderPipeline,
                              bg: &wgpu::BindGroup,
                              views: &[&wgpu::TextureView]| {
            let attachments: Vec<_> = views
                .iter()
                .map(|v| {
                    Some(wgpu::RenderPassColorAttachment {
                        view: v,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                })
                .collect();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(1, bg, &[]);
            if label == "dof across" { pass.set_bind_group(2, &target.ssao_bgs[0], &[]); }
            pass.draw(0..3, 0..1);
        };
        if globals.dof[2] > 0.0 || globals.dof[3] > 0.0 {
            fullscreen("dof across", &self.dof_h, &target.dof_bgs[0], &[&target.dof_views[0]]);
            fullscreen("dof down", &self.dof_v, &target.dof_bgs[1], &[&target.dof_views[1]]);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.display_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.post);
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_bind_group(1, &target.hdr_bg, &[]);
            pass.set_bind_group(2, &target.ssao_bgs[0], &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
        self.stats = stats;
        crate::runtime::domains::services::perf::end_frame(svc_t0.elapsed().as_secs_f64(), 0.0);
        self.texture_id
    }
}

const BUILTIN_MESHES: usize = 5;

/// Index into `Renderer::meshes`: built-in shapes first, then loaded meshes.
fn mesh_slot(p: &PartDraw) -> usize {
    match p.custom_mesh {
        Some(i) => BUILTIN_MESHES + i as usize,
        None => p.mesh as usize,
    }
}

impl Renderer {
    /// Uploads a triangle list (positions in the unit box, normals scaled to
    /// match) and returns its index for [`PartDraw::custom_mesh`].
    pub fn add_mesh(
        &mut self,
        device: &wgpu::Device,
        positions: &[Vec3],
        normals: &[Vec3],
        uvs: &[glam::Vec2],
    ) -> u32 {
        self.add_mesh_colored(device, positions, normals, uvs, &[])
    }

    /// `add_mesh` with per-vertex colours (sRGB 0..1, alpha 1), or none.
    pub fn add_mesh_colored(
        &mut self,
        device: &wgpu::Device,
        positions: &[Vec3],
        normals: &[Vec3],
        uvs: &[glam::Vec2],
        colors: &[glam::Vec4],
    ) -> u32 {
        self.add_mesh_projected(device, positions, normals, uvs, colors, &[])
    }

    pub fn add_mesh_projected(
        &mut self, device: &wgpu::Device, positions: &[Vec3], normals: &[Vec3],
        uvs: &[glam::Vec2], colors: &[glam::Vec4], planes: &[glam::Vec4],
    ) -> u32 {
        let verts: Vec<Vertex> = positions
            .iter()
            .zip(normals)
            .enumerate()
            .map(|(i, (p, n))| Vertex {
                pos: p.to_array(),
                normal: n.to_array(),
                projection: planes.get(i).map(|p| p.to_array()).unwrap_or([0.0; 4]),
                uv: uvs.get(i).map(|u| u.to_array()).unwrap_or([0.0, 0.0]),
                color: colors.get(i).map(|c| c.to_array()).unwrap_or([0.0; 4]),
            })
            .collect();
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("loaded mesh"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        self.meshes.push(Mesh {
            vbuf,
            count: verts.len() as u32,
        });
        (self.meshes.len() - 1 - BUILTIN_MESHES) as u32
    }

    /// Rewrites a loaded mesh's vertices in place (skinned parts re-upload
    /// their deformed copy, skin.rs), or adds a new mesh when `index` is
    /// None or its size differs. Returns the index to draw with.
    pub fn set_mesh(
        &mut self, device: &wgpu::Device, queue: &wgpu::Queue, index: Option<u32>,
        positions: &[Vec3], normals: &[Vec3], uvs: &[glam::Vec2], colors: &[glam::Vec4],
    ) -> u32 {
        let slot = index.map(|i| i as usize + BUILTIN_MESHES);
        match slot.and_then(|s| self.meshes.get(s)) {
            Some(m) if m.count as usize == positions.len() => {
                let verts: Vec<Vertex> = positions
                    .iter()
                    .zip(normals)
                    .enumerate()
                    .map(|(i, (p, n))| Vertex {
                        pos: p.to_array(),
                        normal: n.to_array(),
                        projection: [0.0; 4],
                        uv: uvs.get(i).map(|u| u.to_array()).unwrap_or([0.0, 0.0]),
                        color: colors.get(i).map(|c| c.to_array()).unwrap_or([0.0; 4]),
                    })
                    .collect();
                queue.write_buffer(&m.vbuf, 0, bytemuck::cast_slice(&verts));
                index.unwrap_or(0)
            }
            _ => self.add_mesh_colored(device, positions, normals, uvs, colors),
        }
    }
}

/// Sort / batch key: mesh slot, then texture.
fn draw_key(p: &PartDraw) -> (usize, usize) {
    (
        mesh_slot(p),
        if p.tex_mode == TexMode::None {
            0
        } else {
            p.texture.unwrap_or(0) as usize
        },
    )
}

fn tex_mode(p: &PartDraw) -> f32 {
    match (p.tex_mode, p.texture) {
        (_, None) | (TexMode::None, _) => 0.0,
        (TexMode::Over, Some(_)) => 1.0,
        (TexMode::Decal, Some(_)) => 2.0,
        (TexMode::Opaque, Some(_)) => 3.0,
        (TexMode::Cutout, Some(_)) => 4.0,
        (TexMode::OpaqueTinted, Some(_)) => 5.0,
    }
}

impl Renderer {
    /// Adds or updates one part's draws. Parts that keep changing place move
    /// to the per-frame list; colour-only changes update the static slot.
    pub fn set_item(&mut self, r: Ref, mut draws: Vec<PartDraw>) {
        // Studio seeds material offsets from an allocation address. A DOM
        // referent supplies the equivalent stable per-instance identity here.
        let identity = u64::from_str_radix(&r.to_string()[16..], 16).unwrap_or(0);
        for d in &mut draws {
            if d.mat_layer >= 0 && d.custom_mesh.is_none() && d.tex_mode == TexMode::None {
                let off = crate::matpack::uv_offset(d.size, d.material_id, identity);
                d.uvt[2] = off[0];
                d.uvt[3] = off[1];
            }
        }
        let Renderer {
            items,
            st,
            per_frame,
            pending,
            pending_at,
            ..
        } = self;
        let static_ok = draws.iter().all(|d| !shows(d) || pass_of(d).is_some());
        let Some(item) = items.get_mut(&r) else {
            let bias = bias_of(&draws);
            items.insert(
                r,
                Item {
                    draws,
                    bias,
                    state: ItemState::Pending,
                    moved: Instant::now(),
                },
            );
            per_frame.insert(r);
            *pending += 1;
            // Settle time counts from the first waiting part: a game that
            // spawns something every few frames would otherwise never settle.
            pending_at.get_or_insert_with(Instant::now);
            return;
        };
        let same_place = item.draws.len() == draws.len()
            && item.draws.iter().zip(&draws).all(|(a, b)| {
                near_xf(&a.xf, &b.xf)
                    && a.size == b.size
                    && pass_of(a) == pass_of(b)
                    && draw_key(a) == draw_key(b)
                    && shows(a) == shows(b)
            });
        match &item.state {
            ItemState::Static(slots) if same_place && static_ok => {
                for (d, &s) in draws.iter().zip(slots) {
                    if s != u32::MAX {
                        st.raw[s as usize] = to_raw(d, item.bias);
                        st.writes.push(s);
                    }
                }
            }
            ItemState::Static(slots) => {
                for &s in slots {
                    if s != u32::MAX {
                        st.raw[s as usize] = HIDDEN;
                        st.writes.push(s);
                        st.hidden += 1;
                    }
                }
                item.state = ItemState::Dynamic;
                per_frame.insert(r);
            }
            ItemState::Pending if !same_place => {
                item.state = ItemState::Dynamic;
                *pending = pending.saturating_sub(1);
            }
            _ => {}
        }
        if !same_place {
            item.moved = Instant::now();
        }
        item.draws = draws;
    }

    /// Dynamic parts that stopped changing go back to Pending, so the next
    /// static rebuild batches them again (a part moved once by a script, or
    /// whose mesh finished loading, is not per-frame for the session).
    fn requeue_quiet(&mut self) {
        if self.last_quiet_scan.elapsed().as_secs_f32() < 1.0 {
            return;
        }
        self.last_quiet_scan = Instant::now();
        let mut n = 0;
        for r in &self.per_frame {
            let Some(item) = self.items.get_mut(r) else { continue };
            if item.state == ItemState::Dynamic
                && item.moved.elapsed().as_secs_f32() >= crate::mech::DYNAMIC_SETTLE_S
                && item.draws.iter().all(|d| !shows(d) || pass_of(d).is_some())
            {
                item.state = ItemState::Pending;
                n += 1;
            }
        }
        if n > 0 {
            self.pending += n;
            self.pending_at.get_or_insert_with(Instant::now);
        }
    }

    pub fn remove_item(&mut self, r: Ref) {
        let Some(item) = self.items.remove(&r) else {
            return;
        };
        match item.state {
            ItemState::Static(slots) => {
                for s in slots {
                    if s != u32::MAX {
                        self.st.raw[s as usize] = HIDDEN;
                        self.st.writes.push(s);
                        self.st.hidden += 1;
                    }
                }
            }
            ItemState::Pending => self.pending = self.pending.saturating_sub(1),
            ItemState::Dynamic => {}
        }
        self.per_frame.remove(&r);
    }

    pub fn clear_items(&mut self) {
        self.items.clear();
        self.per_frame.clear();
        self.st = StaticLayout::default();
        self.static_gen += 1;
        self.pending = 0;
        self.pending_at = None;
    }

    pub fn item_draws(&self, r: Ref) -> Option<&[PartDraw]> {
        self.items.get(&r).map(|i| i.draws.as_slice())
    }

    /// Re-sorts every settled part into the static buffer.
    fn rebuild_static(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        struct Row {
            pass: Pass,
            key: (usize, usize),
            cell: u64,
            radius: f32,
            raw: InstanceRaw,
            lo: Vec3,
            hi: Vec3,
            owner: Ref,
            index: u32,
        }
        let mut rows: Vec<Row> = Vec::with_capacity(self.items.len());
        for (r, item) in self.items.iter_mut() {
            if item.state == ItemState::Dynamic {
                continue;
            }
            if !item.draws.iter().all(|d| !shows(d) || pass_of(d).is_some()) {
                item.state = ItemState::Dynamic;
                continue;
            }
            item.state = ItemState::Static(vec![u32::MAX; item.draws.len()]);
            for (i, d) in item.draws.iter().enumerate() {
                let Some(pass) = pass_of(d).filter(|_| shows(d)) else {
                    continue;
                };
                let c = (d.xf.pos / CELL).floor();
                let (lo, hi) = aabb_of(d);
                rows.push(Row {
                    pass,
                    key: draw_key(d),
                    cell: morton([c.x as i32, c.y as i32, c.z as i32]),
                    radius: radius_of(d),
                    raw: to_raw(d, item.bias),
                    lo,
                    hi,
                    owner: *r,
                    index: i as u32,
                });
            }
        }
        rows.sort_by(|a, b| {
            (a.pass, a.key, a.cell)
                .cmp(&(b.pass, b.key, b.cell))
                .then(b.radius.total_cmp(&a.radius))
        });
        let mut st = StaticLayout {
            raw: Vec::with_capacity(rows.len()),
            radii: Vec::with_capacity(rows.len()),
            ..Default::default()
        };
        let mut cell_of_batch = u64::MAX;
        for (slot, row) in rows.iter().enumerate() {
            let slot = slot as u32;
            st.raw.push(row.raw);
            st.radii.push(row.radius);
            if let Some(ItemState::Static(slots)) =
                self.items.get_mut(&row.owner).map(|i| &mut i.state)
            {
                slots[row.index as usize] = slot;
            }
            match st.batches.last_mut() {
                Some(b) if b.pass == row.pass && b.key == row.key && cell_of_batch == row.cell => {
                    b.count += 1;
                    b.lo = b.lo.min(row.lo);
                    b.hi = b.hi.max(row.hi);
                }
                _ => {
                    cell_of_batch = row.cell;
                    st.batches.push(Batch {
                        pass: row.pass,
                        key: row.key,
                        lo: row.lo,
                        hi: row.hi,
                        start: slot,
                        count: 1,
                    });
                }
            }
        }
        if st.raw.len() > self.static_cap {
            self.static_cap = st.raw.len().next_power_of_two();
            self.static_buf = Self::make_buf(
                device,
                "static instances",
                self.static_cap * std::mem::size_of::<InstanceRaw>(),
            );
        }
        if !st.raw.is_empty() {
            queue.write_buffer(&self.static_buf, 0, bytemuck::cast_slice(&st.raw));
        }
        self.st = st;
        self.static_gen += 1;
        let items = &self.items;
        self.per_frame
            .retain(|r| items.get(r).is_some_and(|i| i.state == ItemState::Dynamic));
        self.pending = 0;
        self.pending_at = None;
    }

    /// Uploads static slots changed in place (merged into contiguous runs).
    fn flush_static_writes(&mut self, queue: &wgpu::Queue) {
        if self.st.writes.is_empty() {
            return;
        }
        self.static_gen += 1;
        let mut w = std::mem::take(&mut self.st.writes);
        w.sort_unstable();
        w.dedup();
        let size = std::mem::size_of::<InstanceRaw>() as u64;
        let mut i = 0;
        while i < w.len() {
            let start = w[i];
            let mut end = start + 1;
            i += 1;
            while i < w.len() && w[i] == end {
                end += 1;
                i += 1;
            }
            if (end as usize) <= self.st.raw.len() {
                queue.write_buffer(
                    &self.static_buf,
                    start as u64 * size,
                    bytemuck::cast_slice(&self.st.raw[start as usize..end as usize]),
                );
            }
        }
    }
}

/// InstanceRaw.extra.y bit for CastShadow false (vs_shadow drops the part).
const NO_SHADOW_BIT: u32 = 1 << 19;

// selects PlasticReflection only for 256/272; tiled
// materials (including Metal) keep their texture-driven metalness/roughness.
pub(crate) fn material_reflectance(material: u32, reflectance: f32) -> f32 {
    match material {
        256 | 272 | 1568 => reflectance.clamp(0.0, 1.0),
        _ => 0.0,
    }
}

fn to_raw(p: &PartDraw, bias: f32) -> InstanceRaw {
    let r = p.xf.rot;
    let pos = p.xf.pos;
    InstanceRaw {
        r0: [r.x_axis.x, r.x_axis.y, r.x_axis.z, pos.x],
        r1: [r.y_axis.x, r.y_axis.y, r.y_axis.z, pos.y],
        r2: [r.z_axis.x, r.z_axis.y, r.z_axis.z, pos.z],
        size: [p.size.x, p.size.y, p.size.z, bias],
        color: [
            p.color[0], p.color[1], p.color[2],
            // scene::part_draw currently halves ForceField opacity.
            if p.material_id == 1584 { (p.alpha * 2.0).min(1.0) } else { p.alpha },
        ],
        // y: placeholder (bit 0), the six surfaces (3 bits each above it)
        // and NO_SHADOW_BIT (CastShadow false).
        // x: the shading kind (0..7), 8 = map materials by the mesh UVs,
        // 16 × (material layer + 1).
        extra: [
            (if p.material_id == 1584 { 6.0 } else { p.material })
                + if p.custom_mesh.is_some() { 8.0 } else { 0.0 }
                + 16.0 * (p.mat_layer + 1) as f32,
            (p.placeholder as u32 | p.surfaces << 1 | if p.cast_shadow { 0 } else { NO_SHADOW_BIT })
                as f32,
            // z: Reflectance, + 2 when hovered.
            material_reflectance(p.material_id, p.reflectance) + if p.hover { 2.0 } else { 0.0 },
            tex_mode(p),
        ],
        uvt: p.uvt,
        emissive: [p.emissive[0], p.emissive[1], p.emissive[2], p.tex_albedo],
    }
}

/// shader.wgsl with the measured constants from mech.rs put in front.
fn shader_source() -> String {
    use crate::mech as m;
    let [a, b, c, d, e] = m::TONE_CURVE;
    format!(
        "const SUN_DIST: f32 = {:?};\nconst HYBRID_LERP_SLOPE: f32 = {:?};\nconst TONE_A: f32 = {a:?};\nconst TONE_B: f32 = {b:?};\nconst TONE_C: f32 = {c:?};\nconst TONE_D: f32 = {d:?};\nconst TONE_E: f32 = {e:?};\nconst BLOOM_SIGMA_NARROW: f32 = {:?};\nconst BLOOM_SIGMA_WIDE: f32 = {:?};\nconst BLOOM_WIDE_SHARE: f32 = {:?};\nconst BLOOM_GAIN: f32 = {:?};\nconst BLOOM_MAX_RADIUS: i32 = {};\nconst ATMOS_BAND_DEG: f32 = {:?};\nconst ATMOS_BAND_POW: f32 = {:?};\nconst ATMOS_BAND_FLOOR: f32 = {:?};\nconst ENV_SPEC_LOD_SPAN: f32 = {:?};\nconst LIGHTGRID_BORDER: f32 = {:?};\nconst LIGHTOCC_DENSITY: f32 = {:?};\nconst LIGHTOCC_START_VOXELS: f32 = {:?};\nconst LIGHTOCC_LIGHT_SKIP_VOXELS: f32 = {:?};\nconst LIGHTOCC_MAX_STEPS: f32 = {:?};\nconst NEON_SEE_THROUGH_CUT: f32 = {:?};\nconst NEON_SEE_THROUGH_MAX: f32 = {:?};\nconst ATMOS_T_EPS: f32 = {:?};\nconst ATMOS_FOG_MIP: f32 = {:?};\nconst ATMOS_ENV_CUBE_LEVELS: f32 = {:?};\nconst ENV_SPEC_FADE: f32 = {:?};\nconst ENV_ROUGH_BASE: f32 = {:?};\nconst ENV_ROUGH_SCALE: f32 = {:?};\nconst ENV_SPEC_MIP_PER_ROUGHNESS: f32 = {:?};\nconst ENV_REFLECT_UP_GAIN: f32 = {:?};\nconst SSAO_DEPTH_SCALE: f32 = {:?};\nconst SSAO_DEPTH_MAX: f32 = {:?};\nconst SSAO_RADIUS_K: f32 = {:?};\nconst SSAO_RADIUS_MIN_TEXELS: f32 = {:?};\nconst SSAO_RADIUS_MAX_TEXELS: f32 = {:?};\nconst SSAO_SAMPLES: u32 = {}u;\nconst SSAO_DEPTH_GAIN: f32 = {:?};\nconst SSAO_BLUR_K: f32 = {:?};\nconst SSAO_BLUR_DEPTH: f32 = {:?};\nconst SSAO_BLUR_DEPTH_MIN: f32 = {:?};\nconst SSAO_BLUR_DEPTH_MAX: f32 = {:?};\nconst SSAO_UPSAMPLE_DEPTH: f32 = {:?};\nconst EVSM_POS_EXP: f32 = {:?};\nconst EVSM_NEG_EXP: f32 = {:?};\nconst EVSM_SHADOW_BIAS: f32 = {:?};\nconst EVSM_BLEED_CUT: f32 = {:?};\nconst LSHADOW_COLS: u32 = {}u;\nconst LSHADOW_CELL: f32 = {:?};\n{}",
        m::SUN_DIST,
        m::HYBRID_LERP_SLOPE,
        m::BLOOM_SIGMAS_PX[0],
        m::BLOOM_SIGMAS_PX[1],
        m::BLOOM_WIDE_SHARE,
        m::BLOOM_GAIN,
        m::BLOOM_MAX_RADIUS,
        m::ATMOS_BAND_DEG,
        m::ATMOS_BAND_POW,
        m::ATMOS_BAND_FLOOR,
        m::ENV_SPEC_LOD_SPAN,
        m::LIGHTGRID_BORDER,
        m::LIGHTOCC_DENSITY,
        m::LIGHTOCC_START_VOXELS,
        m::LIGHTOCC_LIGHT_SKIP_VOXELS,
        m::LIGHTOCC_MAX_STEPS,
        m::NEON_SEE_THROUGH_CUT,
        m::NEON_SEE_THROUGH_MAX,
        m::ATMOS_T_EPS,
        m::ATMOS_FOG_MIP,
        m::ATMOS_ENV_CUBE_LEVELS,
        m::ENV_SPEC_FADE,
        m::ENV_ROUGH_BASE,
        m::ENV_ROUGH_SCALE,
        m::ENV_SPEC_MIP_PER_ROUGHNESS,
        m::ENV_REFLECT_UP_GAIN,
        m::SSAO_DEPTH_SCALE,
        m::SSAO_DEPTH_MAX,
        m::SSAO_RADIUS_K,
        m::SSAO_RADIUS_MIN_TEXELS,
        m::SSAO_RADIUS_MAX_TEXELS,
        m::SSAO_SAMPLES,
        m::SSAO_DEPTH_GAIN,
        m::SSAO_BLUR_K,
        m::SSAO_BLUR_DEPTH,
        m::SSAO_BLUR_DEPTH_MIN,
        m::SSAO_BLUR_DEPTH_MAX,
        m::SSAO_UPSAMPLE_DEPTH,
        m::EVSM_POS_EXP,
        m::EVSM_NEG_EXP,
        m::EVSM_SHADOW_BIAS,
        m::EVSM_BLEED_CUT,
        m::LSHADOW_ATLAS / m::LSHADOW_FACE,
        m::LSHADOW_FACE as f32 / m::LSHADOW_ATLAS as f32,
        format!("const DETILE_LEVELS: f32 = {:?};\nconst WANG_CELLS: f32 = {:?};\nconst MATERIAL_DIFFUSE_FADE_DEPTH: f32 = {:?};\nconst MATERIAL_LQ_DIFFUSE_CUTOFF: f32 = {:?};\n{}", m::DETILE_LEVELS, m::WANG_CELLS, m::MATERIAL_DIFFUSE_FADE_DEPTH, m::MATERIAL_LQ_DIFFUSE_CUTOFF, include_str!("shader.wgsl"))
    )
}

fn mip_levels(w: u32, h: u32) -> u32 {
    32 - w.max(h).leading_zeros()
}

/// An RGBA8 sRGB image followed by its mip levels (each half the size,
/// 2×2 texels averaged in linear light; alpha averaged as is).
/// PBR maps contain linear data; gamma-correct colour mips distort normals
/// and roughness. Keep the channels in their encoded data space.
fn data_mip_chain(mut w: u32, mut h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let mut prev = rgba.to_vec();
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0; (nw * nh * 4) as usize];
        for y in 0..nh { for x in 0..nw { for c in 0..4 {
            let mut sum = 0u16;
            for (dx, dy) in [(0,0), (1,0), (0,1), (1,1)] {
                sum += prev[(((y*2+dy).min(h-1)*w + (x*2+dx).min(w-1))*4) as usize + c] as u16;
            }
            next[((y*nw+x)*4) as usize+c] = ((sum+2)/4) as u8;
        }}}
        out.extend_from_slice(&next);
        prev = next;
        w = nw; h = nh;
    }
    out
}

#[test]
fn pbr_mips_preserve_linear_channels() {
    let px = [0,0,0,0, 255,255,255,255, 0,0,0,0, 255,255,255,255];
    assert_eq!(&data_mip_chain(2,2,&px)[16..], &[128;4]);
}

/// Six square faces, each with its mip chain, layer-major (a cube upload).
pub fn cube_mips(size: u32, faces: &[Vec<u8>; 6]) -> Vec<u8> {
    let mut data: Vec<u8> = faces.iter().flat_map(|f| mip_chain(size, size, f)).collect();
    ggx_prefilter_cube(size, &mut data);
    data
}

/// Studio's PrefilteredEnvTexture: the sky at ENV_PREFILTER_SIZE px, each
/// mip below it GGX-filtered for roughness mip / ENV_SPEC_MIP_PER_ROUGHNESS
/// (FastConvolutionPS, Manson & Sloan's fast filtering, run per face and
/// level; the part FS reads it at lod roughness × 5). The
/// shader reads those levels of the skybox's own chain (lod offset by the
/// levels above ENV_PREFILTER_SIZE), so they're replaced in place here:
/// box-filtered levels left rough parts with sharp, glossy sky reflections.
/// Importance-sampled GGX (α = roughness², as the FS's sun term), N = V,
/// reading the box chain at the sample's solid angle.
fn ggx_prefilter_cube(size: u32, data: &mut [u8]) {
    use crate::mech as m;
    let levels = mip_levels(size, size) as usize;
    let base = (size.max(1) / m::ENV_PREFILTER_SIZE.min(size.max(1))).trailing_zeros() as usize;
    if base + 1 >= levels {
        return;
    }
    let to_lin: [f32; 256] = std::array::from_fn(|i| (i as f32 / 255.0).powf(2.2));
    let th: [f32; 255] = std::array::from_fn(|i| ((i as f32 + 0.5) / 255.0).powf(2.2));
    let to_srgb = |v: f32| th.partition_point(|&t| t <= v) as u8;
    let face_len: usize = (0..levels).map(|l| ((size >> l).max(1) as usize).pow(2) * 4).sum();
    let level_off = |l: usize| -> usize { (0..l).map(|k| ((size >> k).max(1) as usize).pow(2) * 4).sum() };
    // The source: the box chain from the base level down, linear RGB.
    let src: Vec<(usize, Vec<[f32; 3]>)> = (base..levels)
        .map(|l| {
            let w = (size >> l).max(1) as usize;
            let mut v = Vec::with_capacity(6 * w * w);
            for f in 0..6 {
                let o = f * face_len + level_off(l);
                for t in 0..w * w {
                    let p = &data[o + t * 4..o + t * 4 + 3];
                    v.push([to_lin[p[0] as usize], to_lin[p[1] as usize], to_lin[p[2] as usize]]);
                }
            }
            (w, v)
        })
        .collect();
    let fetch = |level: usize, d: Vec3| -> [f32; 3] {
        let (w, px) = &src[level.min(src.len() - 1)];
        let w = *w;
        let (f, sc, tc) = cube_face_st(d);
        let x = ((sc * w as f32 - 0.5).max(0.0)).min(w as f32 - 1.0);
        let y = ((tc * w as f32 - 0.5).max(0.0)).min(w as f32 - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(w - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |xx: usize, yy: usize| px[f * w * w + yy * w + xx];
        let mut c = [0f32; 3];
        for (xx, yy, wt) in [(x0, y0, (1.0 - fx) * (1.0 - fy)), (x1, y0, fx * (1.0 - fy)), (x0, y1, (1.0 - fx) * fy), (x1, y1, fx * fy)] {
            let s = at(xx, yy);
            for ch in 0..3 {
                c[ch] += s[ch] * wt;
            }
        }
        c
    };
    let n_samples = m::ENV_PREFILTER_SAMPLES;
    let base_w = src[0].0 as f32;
    let texel_sa = 4.0 * std::f32::consts::PI / (6.0 * base_w * base_w);
    // One thread per face: each writes only its own face's levels.
    std::thread::scope(|scope| {
        for (f, face) in data.chunks_mut(face_len).enumerate() {
            let fetch = &fetch;
            let to_srgb = &to_srgb;
            let level_off = &level_off;
            scope.spawn(move || {
                for l in base + 1..levels {
                    let w = (size >> l).max(1) as usize;
                    let rough = ((l - base) as f32 / m::ENV_SPEC_MIP_PER_ROUGHNESS).min(1.0);
                    let a = rough * rough;
                    let a2 = a * a;
                    let o = level_off(l);
                    for ty in 0..w {
                        for tx in 0..w {
                            let n = cube_face_dir(f, (tx as f32 + 0.5) / w as f32, (ty as f32 + 0.5) / w as f32);
                            let up = if n.z.abs() < 0.999 { Vec3::Z } else { Vec3::X };
                            let tx_ = up.cross(n).normalize();
                            let ty_ = n.cross(tx_);
                            let mut acc = [0f32; 3];
                            let mut wsum = 0f32;
                            for i in 0..n_samples {
                                // Hammersley point → GGX half vector.
                                let u = (i as f32 + 0.5) / n_samples as f32;
                                let v = (i as u32).reverse_bits() as f32 / 4294967296.0;
                                let phi = 2.0 * std::f32::consts::PI * v;
                                let cos_t = ((1.0 - u) / (1.0 + (a2 - 1.0) * u)).sqrt();
                                let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
                                let h = tx_ * (sin_t * phi.cos()) + ty_ * (sin_t * phi.sin()) + n * cos_t;
                                let lv = h * (2.0 * cos_t) - n;
                                let nl = n.dot(lv);
                                if nl <= 0.0 {
                                    continue;
                                }
                                // pdf (N = V) = D / 4; the source mip whose
                                // texel covers this sample's solid angle.
                                let dd = cos_t * cos_t * (a2 - 1.0) + 1.0;
                                let d = a2 / (std::f32::consts::PI * dd * dd);
                                let sample_sa = 1.0 / (n_samples as f32 * d * 0.25 + 1e-4);
                                let mip = (0.5 * (sample_sa / texel_sa).log2() + 1.0).max(0.0);
                                let m0 = mip.floor() as usize;
                                let t = mip - m0 as f32;
                                let c0 = fetch(m0, lv);
                                let c1 = fetch(m0 + 1, lv);
                                for ch in 0..3 {
                                    acc[ch] += (c0[ch] * (1.0 - t) + c1[ch] * t) * nl;
                                }
                                wsum += nl;
                            }
                            let p = o + (ty * w + tx) * 4;
                            for ch in 0..3 {
                                face[p + ch] = to_srgb(acc[ch] / wsum.max(1e-6));
                            }
                        }
                    }
                }
            });
        }
    });
}

/// A unit direction's cube face (wgpu order +X, −X, +Y, −Y, +Z, −Z) and its
/// texture coordinates (s, t) in 0..1 (the Vulkan cube map selection).
fn cube_face_st(d: Vec3) -> (usize, f32, f32) {
    let a = d.abs();
    let (f, sc, tc, ma) = if a.x >= a.y && a.x >= a.z {
        if d.x > 0.0 { (0, -d.z, -d.y, a.x) } else { (1, d.z, -d.y, a.x) }
    } else if a.y >= a.z {
        if d.y > 0.0 { (2, d.x, d.z, a.y) } else { (3, d.x, -d.z, a.y) }
    } else if d.z > 0.0 {
        (4, d.x, -d.y, a.z)
    } else {
        (5, -d.x, -d.y, a.z)
    };
    (f, (sc / ma + 1.0) * 0.5, (tc / ma + 1.0) * 0.5)
}

/// The direction through (s, t) of cube face `f` (inverse of cube_face_st).
fn cube_face_dir(f: usize, s: f32, t: f32) -> Vec3 {
    let (a, b) = (2.0 * s - 1.0, 2.0 * t - 1.0);
    match f {
        0 => Vec3::new(1.0, -b, -a),
        1 => Vec3::new(-1.0, -b, a),
        2 => Vec3::new(a, 1.0, b),
        3 => Vec3::new(a, -1.0, -b),
        4 => Vec3::new(a, -b, 1.0),
        _ => Vec3::new(-a, -b, -1.0),
    }
    .normalize()
}

#[test]
fn cube_face_mapping_round_trips() {
    for f in 0..6 {
        for (s, t) in [(0.2, 0.7), (0.5, 0.5), (0.9, 0.1)] {
            let (g, s2, t2) = cube_face_st(cube_face_dir(f, s, t));
            assert_eq!(g, f);
            assert!((s - s2).abs() < 1e-5 && (t - t2).abs() < 1e-5);
        }
    }
}

fn mip_chain(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    use std::sync::OnceLock;
    static TO_LIN: OnceLock<[f32; 256]> = OnceLock::new();
    let to_lin = TO_LIN.get_or_init(|| std::array::from_fn(|i| (i as f32 / 255.0).powf(2.2)));
    // round(v^(1/2.2) * 255) without powf: the byte whose rounding interval
    // holds v (thresholds ((s - 0.5) / 255)^2.2). powf per channel per texel
    // made mip chains a large share of a map load's texture time.
    static TH: OnceLock<[f32; 255]> = OnceLock::new();
    let th = TH.get_or_init(|| std::array::from_fn(|i| ((i as f32 + 0.5) / 255.0).powf(2.2)));
    let to_srgb = |v: f32| th.partition_point(|&t| t <= v) as u8;
    let mut out = rgba.to_vec();
    let (mut cw, mut ch) = (w, h);
    let mut prev = rgba.to_vec();
    while cw > 1 || ch > 1 {
        let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(cw - 1);
                    let sy = (y * 2 + dy).min(ch - 1);
                    let o = ((sy * cw + sx) * 4) as usize;
                    for c in 0..3 {
                        acc[c] += to_lin[prev[o + c] as usize];
                    }
                    acc[3] += prev[o + 3] as f32;
                }
                let o = ((y * nw + x) * 4) as usize;
                for c in 0..3 {
                    next[o + c] = to_srgb(acc[c] * 0.25);
                }
                next[o + 3] = (acc[3] * 0.25 + 0.5) as u8;
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        cw = nw;
        ch = nh;
    }
    out
}

pub fn mat_cols(m: Mat4) -> [[f32; 4]; 4] {
    m.to_cols_array_2d()
}

// ----------------------------------------------------------------- meshes

/// Collects triangles for a convex shape centred on the origin, flipping any
/// triangle whose winding faces inward so back-face culling is correct.
#[derive(Default)]
struct MeshBuilder {
    verts: Vec<Vertex>,
    /// A point strictly inside the shape.
    center: Vec3,
}

impl MeshBuilder {
    fn tri(&mut self, a: (Vec3, Vec3), b: (Vec3, Vec3), c: (Vec3, Vec3)) {
        let face_n = (b.0 - a.0).cross(c.0 - a.0);
        let centroid = (a.0 + b.0 + c.0) / 3.0;
        let (b, c) = if face_n.dot(centroid - self.center) < 0.0 {
            (c, b)
        } else {
            (b, c)
        };
        let fnorm = face_n.normalize_or(Vec3::Y)
            * if face_n.dot(centroid - self.center) < 0.0 {
                -1.0
            } else {
                1.0
            };
        for (p, n) in [a, b, c] {
            self.verts.push(Vertex {
                pos: p.to_array(),
                normal: n.to_array(),
                uv: box_uv(p, fnorm),
                projection: [0.0; 4],
                color: [0.0; 4],
            });
        }
    }

    /// A flat polygon (convex, given in order).
    fn flat(&mut self, pts: &[Vec3]) {
        let n = (pts[1] - pts[0]).cross(pts[2] - pts[0]).normalize();
        let centroid = pts.iter().copied().sum::<Vec3>() / pts.len() as f32;
        let n = if n.dot(centroid - self.center) < 0.0 {
            -n
        } else {
            n
        };
        for i in 1..pts.len() - 1 {
            self.tri((pts[0], n), (pts[i], n), (pts[i + 1], n));
        }
    }
}

fn build_meshes() -> Vec<Vec<Vertex>> {
    let h = 0.5;
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);

    // Block
    let mut block = MeshBuilder::default();
    for axis in 0..3 {
        for s in [-h, h] {
            let corner = |a: f32, b: f32| {
                let mut p = Vec3::ZERO;
                p[axis] = s;
                p[(axis + 1) % 3] = a;
                p[(axis + 2) % 3] = b;
                p
            };
            block.flat(&[corner(-h, -h), corner(h, -h), corner(h, h), corner(-h, h)]);
        }
    }

    // Ball
    let mut ball = MeshBuilder::default();
    let (rings, segs) = (18, 32);
    let sp = |i: usize, j: usize| {
        let th = std::f32::consts::PI * i as f32 / rings as f32;
        let ph = std::f32::consts::TAU * j as f32 / segs as f32;
        let n = Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin());
        (n * h, n)
    };
    for i in 0..rings {
        for j in 0..segs {
            let (a, b, c, d) = (sp(i, j), sp(i + 1, j), sp(i + 1, j + 1), sp(i, j + 1));
            if i != 0 {
                ball.tri(a, b, d);
            }
            if i != rings - 1 {
                ball.tri(b, c, d);
            }
        }
    }

    // Cylinder (axis along X, like Roblox cylinders)
    let mut cyl = MeshBuilder::default();
    let segs = 32;
    let ring = |j: usize| {
        let a = std::f32::consts::TAU * j as f32 / segs as f32;
        Vec3::new(0.0, a.cos(), a.sin())
    };
    for j in 0..segs {
        let (n0, n1) = (ring(j), ring(j + 1));
        let (p0, p1) = (n0 * h, n1 * h);
        cyl.tri(
            (p0 + v(-h, 0.0, 0.0), n0),
            (p0 + v(h, 0.0, 0.0), n0),
            (p1 + v(h, 0.0, 0.0), n1),
        );
        cyl.tri(
            (p0 + v(-h, 0.0, 0.0), n0),
            (p1 + v(h, 0.0, 0.0), n1),
            (p1 + v(-h, 0.0, 0.0), n1),
        );
        for s in [-h, h] {
            let n = v(s.signum(), 0.0, 0.0);
            cyl.tri(
                (v(s, 0.0, 0.0), n),
                (p0 + v(s, 0.0, 0.0), n),
                (p1 + v(s, 0.0, 0.0), n),
            );
        }
    }

    // Wedge: full height at the back (+Z), sloping down to the front (-Z).
    let mut wedge = MeshBuilder {
        center: v(0.0, -0.2, 0.2),
        ..Default::default()
    };
    wedge.flat(&[v(-h, -h, -h), v(h, -h, -h), v(h, -h, h), v(-h, -h, h)]); // bottom
    wedge.flat(&[v(-h, -h, h), v(h, -h, h), v(h, h, h), v(-h, h, h)]); // back
    wedge.flat(&[v(-h, -h, -h), v(h, -h, -h), v(h, h, h), v(-h, h, h)]); // slope
    wedge.flat(&[v(-h, -h, -h), v(-h, -h, h), v(-h, h, h)]); // left
    wedge.flat(&[v(h, -h, -h), v(h, -h, h), v(h, h, h)]); // right

    // Corner wedge: apex above the right-front corner.
    let mut corner = MeshBuilder {
        center: v(0.25, -0.3, -0.25),
        ..Default::default()
    };
    let apex = v(h, h, -h);
    let (b0, b1, b2, b3) = (v(-h, -h, -h), v(h, -h, -h), v(h, -h, h), v(-h, -h, h));
    corner.flat(&[b0, b1, b2, b3]);
    corner.flat(&[apex, b1, b2]);
    corner.flat(&[apex, b0, b1]);
    corner.flat(&[apex, b2, b3]);
    corner.flat(&[apex, b3, b0]);

    vec![
        block.verts,
        ball.verts,
        cyl.verts,
        wedge.verts,
        corner.verts,
    ]
}

thread_local! {
    static REUSE_TEXTURE_SLOT: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// The next part texture upload on this thread replaces slot `index`
/// instead of taking a new one (assets.rs, for EditableImage updates).
pub fn reuse_texture_slot(index: u32) {
    REUSE_TEXTURE_SLOT.with(|c| c.set(Some(index)));
}
