//! ParticleEmitters, Beams and Trails (round 25): simulated on the CPU every
//! frame and drawn as extra triangles in the world GUI pass (world_gui.rs),
//! so the scene's depth hides them and LightInfluence / Brightness, the
//! scene's tone curve and its fog apply as in Studio's ParticleCustomFS and
//! TrailFSLight (docs/MECHANICS.md "Round 25").
//!
//! As in Studio, the CPU simulates and writes one record per particle (its
//! ParticleCustomVS makes the quad from it) and world-space strip vertices
//! for beams and trails (TrailVSLightUnified); the GPU projects and lights
//! them (world_gui.rs `fx_scene`: the light grid, the local lights and the
//! sun, the terms of `SceneLight::at`, which stays as the reference and for
//! RUSTUDIO_FX_LIGHT_CHECK). Colours are premultiplied and gamma-space like
//! egui's; LightEmission keeps the colour and scales the alpha by
//! (1 − LightEmission), which under premultiplied blending is Studio's
//! "additive" blend.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use egui::{Rect, TextureId};
use glam::{Mat4, Quat, Vec2, Vec3};
#[cfg(test)]
use glam::Mat3;
use rbx_dom_weak::types::{Ref, Variant};

use crate::camera::Camera;
use crate::doc::{self, Document};
use crate::math::Xform;
use crate::mech;
use crate::scene::{content_string, enum_u32, f32_of, vec3_of, xform_of};
use crate::world_gui::{FxFrame, FxLook, FxParticle, FxVertex};

#[path = "effects_constraints.rs"]
mod constraints;

const CLASSES: [&str; 6] = ["ParticleEmitter", "Beam", "Trail", "RopeConstraint", "RodConstraint", "SpringConstraint"];

// ------------------------------------------------------------ script calls

enum Req {
    Emit(u32),
    Clear,
    /// Beam:SetTextureOffset (domains/world_misc.rs).
    Offset(f32),
}

/// `ParticleEmitter:Emit` / `:Clear` from scripts, by document id, until
/// the viewport next draws that document.
static REQUESTS: Mutex<Vec<(u64, Ref, Req)>> = Mutex::new(Vec::new());

/// `emitter:Emit(n)`: n particles at once, whether or not it's Enabled.
pub fn emit(doc_id: u64, emitter: Ref, n: u32) {
    if let Ok(mut q) = REQUESTS.lock() {
        // (a script calling Emit every frame on a document nobody shows
        // doesn't grow the queue for ever)
        if q.len() < 4096 {
            q.push((doc_id, emitter, Req::Emit(n)));
        }
    }
}

/// `emitter:Clear()`: drops its live particles.
pub fn clear(doc_id: u64, emitter: Ref) {
    if let Ok(mut q) = REQUESTS.lock() {
        if q.len() < 4096 {
            q.push((doc_id, emitter, Req::Clear));
        }
    }
}

/// `beam:SetTextureOffset(offset)`: the texture cycle's phase is `offset`
/// now (Studio: 0.25 moves the texture a quarter of its length toward
/// Attachment0) and TextureSpeed carries on from there.
pub fn set_texture_offset(doc_id: u64, beam: Ref, offset: f32) {
    if let Ok(mut q) = REQUESTS.lock() {
        if q.len() < 4096 {
            q.push((doc_id, beam, Req::Offset(offset)));
        }
    }
}

// ------------------------------------------------------------- the light

/// The scene's light as the effects take it (round 26), kept by the
/// viewport from the frame's lighting. Studio lights a particle once, at
/// its centre moved FX_LIGHT_TOWARD_CAMERA toward the camera
/// (ParticleLightingVS / FS: min(LightMap + Ambient + SkyAmbient ×
/// skylight, cap) + Lamp0 × sun visibility, no N·L), and trails / beams per
/// pixel at the point (TrailFSLight: the same, the sun also × the shadow
/// map). rustudio reads the same terms on the GPU (world_gui.rs fx_scene,
/// from these values, the light-grid texture and the lights buffer; this
/// CPU copy is the reference for tests and RUSTUDIO_FX_LIGHT_CHECK), per
/// particle and per strip row: Ambient, OutdoorAmbient × the roof map × the terrain's
/// light grid, the sun × the roof map × the light grid (the voxel sun
/// visibility Studio keeps in LightGridSkylight.y, estimated by the roof
/// map; no shadow map on the CPU), the skybox's diffuse light × the light
/// grid (as parts take it: Place B F6's conveyor, under a roof 140 studs
/// up, reads 0.2 of the open sky in Studio, black with the roof map on
/// all of it), and the local lights without N·L.
#[derive(Default, Clone)]
pub struct SceneLight {
    /// Lighting.Ambient, squared (shader.wgsl g.ambient).
    pub ambient: Vec3,
    /// max(OutdoorAmbient − Ambient, 0), squared (g.outdoor).
    pub outdoor: Vec3,
    /// The sun: its colour × 0.5 × Brightness, 0 below the horizon.
    pub sun: Vec3,
    /// The skybox's diffuse light (EnvironmentDiffuseScale), the mean of
    /// the six axes (shader.wgsl env_diffuse with no normal).
    pub env: Vec3,
    /// LightingStyle Soft (local lights through the voxels).
    pub soft: bool,
    /// The frame's local lights (shader layout).
    pub lights: Vec<crate::render::LightRaw>,
    /// Studio's light grid (lightgrid.rs), as the GPU has it.
    pub grid: Option<Arc<crate::lightgrid::LightGrid>>,
}

impl SceneLight {
    /// The light grid at `p`, trilinear (shader.wgsl light_grid without
    /// the normal offset): channel 0 skylight, 1 sun; 1 beyond its border.
    fn grid(&self, p: Vec3, channel: usize) -> f32 {
        let Some(g) = &self.grid else { return 1.0 };
        let [nx, ny, nz] = crate::lightgrid::dims();
        let dim = Vec3::new(nx as f32, ny as f32, nz as f32);
        // Voxel coordinates, texel centres at +0.5.
        let q = p / crate::terrain::VOXEL - Vec3::new(g.origin[0] as f32, g.origin[1] as f32, g.origin[2] as f32);
        if ((q / dim) - Vec3::splat(0.5)).abs().max_element() >= mech::LIGHTGRID_BORDER {
            return 1.0;
        }
        let q = q - Vec3::splat(0.5);
        let b = q.floor();
        let f = q - b;
        let at = |x: i32, y: i32, z: i32| -> f32 {
            let x = x.clamp(0, nx as i32 - 1) as usize;
            let y = y.clamp(0, ny as i32 - 1) as usize;
            let z = z.clamp(0, nz as i32 - 1) as usize;
            g.data.get(2 * ((y * nz + z) * nx + x) + channel).copied().unwrap_or(255) as f32 / 255.0
        };
        let (x, y, z) = (b.x as i32, b.y as i32, b.z as i32);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let c00 = lerp(at(x, y, z), at(x + 1, y, z), f.x);
        let c10 = lerp(at(x, y + 1, z), at(x + 1, y + 1, z), f.x);
        let c01 = lerp(at(x, y, z + 1), at(x + 1, y, z + 1), f.x);
        let c11 = lerp(at(x, y + 1, z + 1), at(x + 1, y + 1, z + 1), f.x);
        lerp(lerp(c00, c01, f.z), lerp(c10, c11, f.z), f.y)
    }

    /// One local light at `p`, without N·L. Studio lights effects from
    /// its voxel light grid, not per pixel: the light falls off in a
    /// straight line, FX_LOCAL_SLOPE × Brightness per stud, from the light
    /// to Range + FX_LOCAL_REACH (calibration q: PointLights of Range 8, 16
    /// and 30 at Brightness 1 and 4, a white LightInfluence 1 particle's
    /// light read every 1.3 studs). Spot and surface lights keep their
    /// cone (shader.wgsl's).
    fn local(&self, l: &crate::render::LightRaw, p: Vec3) -> Vec3 {
        let pos = Vec3::new(l.pos_range[0], l.pos_range[1], l.pos_range[2]);
        let range = l.pos_range[3];
        let color = Vec3::new(l.color[0], l.color[1], l.color[2]) / mech::LIGHT_INTENSITY;
        let kind = l.color[3];
        let axis = Vec3::new(l.dir[0], l.dir[1], l.dir[2]);
        let cos_cut = l.dir[3];
        let to = pos - p;
        let d = to.length();
        let reach = range + mech::FX_LOCAL_REACH;
        if range <= 0.0 || d >= reach {
            return Vec3::ZERO;
        }
        let dir = to / d.max(1e-4);
        let mut k = mech::FX_LOCAL_SLOPE * (reach - d);
        if kind > 0.5 {
            let c = (-dir).dot(axis);
            let cone = ((c - cos_cut) / (1.0 - cos_cut).max(1e-3)).clamp(0.0, 1.0);
            k *= cone * cone;
            if kind > 1.5 && c < 0.0 {
                k = 0.0;
            }
        }
        color * k
    }

    /// The local lights that can reach a sphere (an effect's bounds).
    fn near(&self, c: Vec3, r: f32) -> Vec<&crate::render::LightRaw> {
        self.lights
            .iter()
            .filter(|l| {
                let reach = l.pos_range[3] + mech::FX_LOCAL_REACH;
                (Vec3::new(l.pos_range[0], l.pos_range[1], l.pos_range[2]) - c).length() < reach + r
            })
            .collect()
    }

    /// The light at `p` (linear RGB), from `lights` (`near`'s list).
    /// ParticleLightingFS: min(the grid's light + ambient + sky, cap) +
    /// the sun. The shader caps each RGB channel separately; the sun is
    /// added after that cap.
    fn at(&self, p: Vec3, lights: &[&crate::render::LightRaw]) -> Vec3 {
        let sky = self.grid(p, 0);
        let mut c = self.ambient + self.outdoor * sky + self.env * sky;
        for l in lights {
            c += self.local(l, p);
        }
        // ParticleLightingFS uses a component-wise min before adding Lamp0.
        // A luminance rescale preserves overbright coloured channels and
        // incorrectly reduces the other channels of a coloured local light.
        c.min(Vec3::splat(mech::FX_LIGHT_CAP)) + self.sun * self.grid(p, 1)
    }
}

/// A see-through part effects behind it are dimmed by (round 26): its box,
/// opacity and distance from the camera (Studio sorts blended parts and
/// effects together far to near by object).
#[derive(Clone, Copy)]
pub struct Glass {
    pub xf: Xform,
    pub half: Vec3,
    pub alpha: f32,
    /// Bounding sphere, and its centre's distance from the camera.
    pub centre: Vec3,
    pub radius: f32,
    pub dist: f32,
    pub part: Ref,
}

impl Glass {
    /// Where the segment from `o` (length `len` along unit `d`) enters the
    /// box, if it does.
    fn hit(&self, o: Vec3, d: Vec3, len: f32) -> bool {
        let r = self.xf.rot.transpose();
        let lo = r * (o - self.xf.pos);
        let ld = r * d;
        let (mut t0, mut t1) = (0.0f32, len);
        for i in 0..3 {
            if ld[i].abs() < 1e-6 {
                if lo[i].abs() > self.half[i] {
                    return false;
                }
            } else {
                let a = (-self.half[i] - lo[i]) / ld[i];
                let b = (self.half[i] - lo[i]) / ld[i];
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }
}

/// The see-through parts that may lie between the camera and an effect
/// (centre `c`, radius `r`), drawn after it in Studio's order: nearer the
/// camera than the effect, and not the effect's own host.
#[cfg(test)]
fn glass_between<'a>(glass: &'a [Glass], cam: Vec3, c: Vec3, r: f32, host: Option<Ref>) -> Vec<&'a Glass> {
    glass_in_front(glass, cam, c, r, host).map(|i| &glass[i]).collect()
}

/// `glass_between` as indices into `glass`.
fn glass_in_front(glass: &[Glass], cam: Vec3, c: Vec3, r: f32, host: Option<Ref>) -> impl Iterator<Item = usize> + '_ {
    let seg = c - cam;
    let len = seg.length();
    let d = seg / len.max(1e-6);
    // (`glass` is nearest first: what lies past the effect can stop)
    glass
        .iter()
        .enumerate()
        .take_while(move |(_, g)| g.dist < len)
        .filter(move |(_, g)| {
            if Some(g.part) == host {
                return false;
            }
            // The glass's sphere against the capsule from the camera to the effect.
            let t = (g.centre - cam).dot(d).clamp(0.0, len);
            (cam + d * t - g.centre).length() < g.radius + r
        })
        .map(|(i, _)| i)
}

/// The glass in front of one effect (`glass_between`), binned into screen
/// tiles when there's much of it (Place B's Snowfall box spans hundreds of
/// see-through parts), so each particle tests only what covers it. The
/// CPU reference of the GPU's test (world_gui.rs fx_transmit).
#[cfg(test)]
struct GlassSet<'a> {
    list: Vec<&'a Glass>,
    /// GLASS_TILES² lists of indices into `list`, by NDC tile.
    tiles: Option<Vec<Vec<u32>>>,
    vp: Mat4,
}

#[cfg(test)]
const GLASS_TILES: usize = 16;
/// More candidates than this get tiles.
#[cfg(test)]
const GLASS_TILE_MIN: usize = 8;

#[cfg(test)]
fn tile_of(ndc: f32) -> usize {
    (((ndc * 0.5 + 0.5) * GLASS_TILES as f32).floor().max(0.0) as usize).min(GLASS_TILES - 1)
}

#[cfg(test)]
impl<'a> GlassSet<'a> {
    fn new(list: Vec<&'a Glass>, vp: Mat4, cam: Vec3) -> GlassSet<'a> {
        let tiles = (list.len() > GLASS_TILE_MIN).then(|| {
            let mut t = vec![Vec::new(); GLASS_TILES * GLASS_TILES];
            // (the projection's scale per stud at 1 stud: x and y)
            let (sx, sy) = (vp.x_axis.length(), vp.y_axis.length());
            for (i, g) in list.iter().enumerate() {
                let c = vp * g.centre.extend(1.0);
                let (x0, x1, y0, y1) = if c.w <= g.radius || (g.centre - cam).length() <= g.radius {
                    (0, GLASS_TILES - 1, 0, GLASS_TILES - 1)
                } else {
                    let (nx, ny) = (c.x / c.w, c.y / c.w);
                    let (rx, ry) = (g.radius * sx / c.w * 1.5, g.radius * sy / c.w * 1.5);
                    (tile_of(nx - rx), tile_of(nx + rx), tile_of(ny - ry), tile_of(ny + ry))
                };
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        t[y * GLASS_TILES + x].push(i as u32);
                    }
                }
            }
            t
        });
        GlassSet { list, tiles, vp }
    }

    /// How much of an effect at `p` shows through the glass in front of it.
    fn transmit(&self, cam: Vec3, p: Vec3) -> f32 {
        let mut t = 1.0;
        if self.list.is_empty() {
            return t;
        }
        let seg = p - cam;
        let len = seg.length();
        let d = seg / len.max(1e-6);
        let mut test = |g: &Glass| {
            if g.hit(cam, d, len) {
                t *= 1.0 - g.alpha;
            }
        };
        match &self.tiles {
            Some(tiles) => {
                let c = self.vp * p.extend(1.0);
                if c.w <= 1e-4 {
                    return 1.0;
                }
                for &i in &tiles[tile_of(c.y / c.w) * GLASS_TILES + tile_of(c.x / c.w)] {
                    test(self.list[i as usize]);
                }
            }
            None => self.list.iter().for_each(|g| test(g)),
        }
        t
    }
}

// ------------------------------------------------------------- properties

/// A NumberSequence: (time, value, envelope) keypoints.
#[derive(Clone, Default)]
struct Seq(Vec<(f32, f32, f32)>);

impl Seq {
    fn read(v: Option<Variant>, dflt: f32) -> Seq {
        match v {
            Some(Variant::NumberSequence(s)) if !s.keypoints.is_empty() => {
                Seq(s.keypoints.iter().map(|k| (k.time, k.value, k.envelope)).collect())
            }
            Some(v) => Seq(vec![(0.0, f32_of(Some(v)).unwrap_or(dflt), 0.0)]),
            None => Seq(vec![(0.0, dflt, 0.0)]),
        }
    }

    /// The value at `t` (0..1), each keypoint moved by `r` (−1..1, fixed per
    /// particle) times its envelope.
    fn at(&self, t: f32, r: f32) -> f32 {
        let k = &self.0;
        let v = |i: usize| k[i].1 + k[i].2 * r;
        if k.len() == 1 || t <= k[0].0 {
            return v(0);
        }
        for i in 1..k.len() {
            if t <= k[i].0 {
                let span = (k[i].0 - k[i - 1].0).max(1e-6);
                return v(i - 1) + (v(i) - v(i - 1)) * ((t - k[i - 1].0) / span);
            }
        }
        v(k.len() - 1)
    }
}

/// A ColorSequence (time, rgb 0..1).
#[derive(Clone, Default)]
struct CSeq(Vec<(f32, Vec3)>);

impl CSeq {
    fn read(v: Option<Variant>) -> CSeq {
        match v {
            Some(Variant::ColorSequence(s)) if !s.keypoints.is_empty() => CSeq(
                s.keypoints
                    .iter()
                    .map(|k| (k.time, Vec3::new(k.color.r, k.color.g, k.color.b)))
                    .collect(),
            ),
            Some(Variant::Color3(c)) => CSeq(vec![(0.0, Vec3::new(c.r, c.g, c.b))]),
            _ => CSeq(vec![(0.0, Vec3::ONE)]),
        }
    }

    fn at(&self, t: f32) -> Vec3 {
        let k = &self.0;
        if k.len() == 1 || t <= k[0].0 {
            return k[0].1;
        }
        for i in 1..k.len() {
            if t <= k[i].0 {
                let span = (k[i].0 - k[i - 1].0).max(1e-6);
                return k[i - 1].1.lerp(k[i].1, (t - k[i - 1].0) / span);
            }
        }
        k[k.len() - 1].1
    }
}

fn range(v: Option<Variant>, dflt: f32) -> (f32, f32) {
    match v {
        Some(Variant::NumberRange(r)) => (r.min, r.max),
        v => {
            let x = f32_of(v).unwrap_or(dflt);
            (x, x)
        }
    }
}

fn num(d: &Document, r: Ref, k: &str, dflt: f32) -> f32 {
    f32_of(d.prop(r, k)).unwrap_or(dflt)
}

fn flag(d: &Document, r: Ref, k: &str, dflt: bool) -> bool {
    match d.prop(r, k) {
        Some(Variant::Bool(b)) => b,
        _ => dflt,
    }
}

fn reference(d: &Document, r: Ref, k: &str) -> Option<Ref> {
    match d.prop(r, k) {
        Some(Variant::Ref(a)) if d.exists(a) => Some(a),
        _ => None,
    }
}

fn texture_of(d: &Document, r: Ref) -> Option<String> {
    // What the instance stores wins over class defaults: TextureContent's
    // default is the sparkle, Texture's is "", and an emitter given only
    // Texture (Instance.new, then Texture = id) must draw that id.
    let stored = |k: &str| {
        d.dom
            .get_by_ref(r)
            .and_then(|i| i.properties.get(&rbx_dom_weak::ustr(k)).cloned())
    };
    let (tc, t) = (stored("TextureContent"), stored("Texture"));
    if let Some(s) = content_string(tc.clone()) {
        return Some(s);
    }
    if t.is_some() {
        return content_string(t);
    }
    if tc.is_some() {
        return None;
    }
    content_string(d.prop(r, "TextureContent")).or_else(|| content_string(d.prop(r, "Texture")))
}

/// What an effect hangs off: a part (its box) or an attachment (a point).
#[derive(Clone, Copy, PartialEq)]
enum Host {
    Part(Ref),
    Attachment(Ref),
    None,
}

fn host_of(d: &Document, r: Ref) -> Host {
    let Some(p) = d.dom.get_by_ref(r).map(|i| i.parent()) else {
        return Host::None;
    };
    match d.class_of(p) {
        Some(c) if doc::is_a(&c, "Attachment") => Host::Attachment(p),
        Some(c) if doc::is_a(&c, "BasePart") => Host::Part(p),
        _ => Host::None,
    }
}

/// An attachment's world transform: its parent part's CFrame × its own
/// (as physics.rs attachment_xform), through Bones parented to Bones.
fn attachment_world(d: &Document, a: Ref) -> Option<Xform> {
    crate::attachment::world(d, a, true)
}

/// What an attachment's world frame reads (attachment::world): it, and
/// its ancestors up to and including the first BasePart.
fn chain_of(d: &Document, a: Ref, out: &mut Vec<Ref>) {
    let mut cur = a;
    for _ in 0..64 {
        let Some(i) = d.dom.get_by_ref(cur) else { return };
        out.push(cur);
        if cur != a && doc::is_a(&i.class, "BasePart") {
            return;
        }
        cur = i.parent();
    }
}

impl Pose {
    fn same(&self, o: &Pose) -> bool {
        let x = |a: &Option<Xform>, b: &Option<Xform>| match (a, b) {
            (Some(a), Some(b)) => a.pos == b.pos && a.rot == b.rot,
            (None, None) => true,
            _ => false,
        };
        match (self, o) {
            (Pose::Ends(a0, a1), Pose::Ends(b0, b1)) => x(a0, b0) && x(a1, b1),
            (Pose::Host(a, va), Pose::Host(b, vb)) => {
                va == vb
                    && match (a, b) {
                        (Some(a), Some(b)) => a.1 == b.1 && x(&Some(a.0), &Some(b.0)),
                        (None, None) => true,
                        _ => false,
                    }
            }
            _ => false,
        }
    }
}

/// RUSTUDIO_FX_POSE_CHECK: every kept pose is read again and compared.
fn pose_check() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTUDIO_FX_POSE_CHECK").is_some())
}

/// Where an effect is (read through the document's properties), kept until
/// the journal marks it or something it hangs from (`Effects::pose`).
#[derive(Clone, Copy)]
enum Pose {
    /// A beam's or trail's attachments.
    Ends(Option<Xform>, Option<Xform>),
    /// An emitter's host frame and size, and an anchored host's set
    /// velocity.
    Host(Option<(Xform, Vec3)>, Option<Vec3>),
}

/// The world transform and box size an effect emits from.
fn host_xform(d: &Document, h: Host) -> Option<(Xform, Vec3)> {
    match h {
        Host::Part(p) => Some((xform_of(d, p)?, vec3_of(d.prop(p, "Size")).unwrap_or(Vec3::ONE))),
        Host::Attachment(a) => Some((attachment_world(d, a)?, Vec3::ZERO)),
        Host::None => None,
    }
}

/// How an effect is lit and blended (every class has these).
#[derive(Clone, Copy)]
struct Look {
    influence: f32,
    brightness: f32,
    emission: f32,
    alpha: f32,
}

impl Look {
    /// LightInfluence 0 = Brightness alone, 1 = the scene's light `lit`,
    /// mixed by √LightInfluence (calibration o, Studio at night: LI 0.1 /
    /// 0.25 / 0.5 / 0.75 / 0.9 weigh the scene's light 0.30 / 0.46 / 0.71
    /// / 0.87 / 0.95 and Brightness the rest, at Brightness 0, 1 and 3).
    fn light(&self, lit: Vec3) -> [f32; 3] {
        let w = self.influence.sqrt();
        (Vec3::splat(self.brightness * (1.0 - w)) + lit * w).to_array()
    }

    /// Whether the scene's light matters (LightInfluence above 0).
    fn lit(&self) -> bool {
        self.influence > 0.0
    }

    /// For the GPU (world_gui::FxLook): `light`'s two weights, the mode
    /// (0 unlit, 1 lit, 2 lit with the beam's per-pixel sun shadow) and
    /// 1 − LightEmission.
    fn gpu(&self, beam_shadow: bool) -> FxLook {
        let w = self.influence.sqrt();
        let mode = if !self.lit() { 0.0 } else if beam_shadow { 2.0 } else { 1.0 };
        [self.brightness * (1.0 - w), if self.lit() { w } else { 0.0 }, mode, 1.0 - self.emission]
    }
}

fn look(d: &Document, r: Ref) -> Look {
    Look {
        influence: num(d, r, "LightInfluence", 0.0).clamp(0.0, 1.0),
        brightness: num(d, r, "Brightness", 1.0).max(0.0),
        emission: num(d, r, "LightEmission", 0.0).clamp(0.0, 1.0),
        alpha: 1.0 - num(d, r, "LocalTransparencyModifier", 0.0).clamp(0.0, 1.0),
    }
}

#[derive(Clone)]
struct EmitterCfg {
    host: Host,
    enabled: bool,
    rate: f32,
    life: (f32, f32),
    speed: (f32, f32),
    spread: Vec2,
    dir: u32,
    shape: u32,
    style: u32,
    inout: u32,
    partial: f32,
    accel: Vec3,
    drag: f32,
    inherit: f32,
    locked: bool,
    /// WindAffectsDrag: Drag pulls toward Workspace.GlobalWind, not rest.
    wind: bool,
    time_scale: f32,
    size: Seq,
    transparency: Seq,
    color: CSeq,
    squash: Seq,
    rotation: (f32, f32),
    rot_speed: (f32, f32),
    texture: Option<String>,
    grid: (u32, u32),
    flip_mode: u32,
    flip_fps: (f32, f32),
    flip_random_start: bool,
    flip_blend: bool,
    orientation: u32,
    zoffset: f32,
    look: Look,
}

fn emitter_cfg(d: &Document, r: Ref) -> EmitterCfg {
    let spread = match d.prop(r, "SpreadAngle") {
        Some(Variant::Vector2(v)) => Vec2::new(v.x, v.y),
        _ => Vec2::ZERO,
    };
    // ParticleFlipbookLayout: None, Grid2x2, Grid4x4, Grid8x8, Custom
    // (FlipbookSizeX × FlipbookSizeY frames).
    let grid = match enum_u32(d.prop(r, "FlipbookLayout")).unwrap_or(0) {
        1 => (2, 2),
        2 => (4, 4),
        3 => (8, 8),
        4 => (
            num(d, r, "FlipbookSizeX", 1.0).max(1.0) as u32,
            num(d, r, "FlipbookSizeY", 1.0).max(1.0) as u32,
        ),
        _ => (1, 1),
    };
    EmitterCfg {
        host: host_of(d, r),
        enabled: flag(d, r, "Enabled", true),
        rate: num(d, r, "Rate", 20.0).max(0.0),
        life: range(d.prop(r, "Lifetime"), 5.0),
        speed: range(d.prop(r, "Speed"), 5.0),
        spread,
        dir: enum_u32(d.prop(r, "EmissionDirection")).unwrap_or(1),
        shape: enum_u32(d.prop(r, "Shape")).unwrap_or(0),
        style: enum_u32(d.prop(r, "ShapeStyle")).unwrap_or(0),
        inout: enum_u32(d.prop(r, "ShapeInOut")).unwrap_or(0),
        partial: num(d, r, "ShapePartial", 1.0).clamp(0.0, 1.0),
        accel: vec3_of(d.prop(r, "Acceleration")).unwrap_or(Vec3::ZERO),
        drag: num(d, r, "Drag", 0.0),
        inherit: num(d, r, "VelocityInheritance", 0.0),
        locked: flag(d, r, "LockedToPart", false),
        wind: flag(d, r, "WindAffectsDrag", false),
        time_scale: num(d, r, "TimeScale", 1.0).clamp(0.0, 1.0),
        size: Seq::read(d.prop(r, "Size"), 1.0),
        transparency: Seq::read(d.prop(r, "Transparency"), 0.0),
        color: CSeq::read(d.prop(r, "Color")),
        squash: Seq::read(d.prop(r, "Squash"), 0.0),
        rotation: range(d.prop(r, "Rotation"), 0.0),
        rot_speed: range(d.prop(r, "RotSpeed"), 0.0),
        texture: texture_of(d, r),
        grid,
        flip_mode: enum_u32(d.prop(r, "FlipbookMode")).unwrap_or(0),
        flip_fps: range(d.prop(r, "FlipbookFramerate"), 1.0),
        flip_random_start: flag(d, r, "FlipbookStartRandom", false),
        flip_blend: flag(d, r, "FlipbookBlendFrames", false),
        orientation: enum_u32(d.prop(r, "Orientation")).unwrap_or(0),
        zoffset: num(d, r, "ZOffset", 0.0),
        look: look(d, r),
    }
}

#[derive(Clone)]
struct BeamCfg {
    enabled: bool,
    a0: Option<Ref>,
    a1: Option<Ref>,
    curve: (f32, f32),
    segments: usize,
    width: (f32, f32),
    face_camera: bool,
    color: CSeq,
    transparency: Seq,
    texture: Option<String>,
    tex_length: f32,
    tex_mode: u32,
    tex_speed: f32,
    zoffset: f32,
    look: Look,
}

fn beam_cfg(d: &Document, r: Ref) -> BeamCfg {
    BeamCfg {
        enabled: flag(d, r, "Enabled", true),
        a0: reference(d, r, "Attachment0"),
        a1: reference(d, r, "Attachment1"),
        curve: (num(d, r, "CurveSize0", 0.0), num(d, r, "CurveSize1", 0.0)),
        segments: (num(d, r, "Segments", 10.0) as i32).clamp(1, mech::FX_BEAM_MAX_SEGMENTS) as usize,
        width: (num(d, r, "Width0", 1.0), num(d, r, "Width1", 1.0)),
        face_camera: flag(d, r, "FaceCamera", false),
        color: CSeq::read(d.prop(r, "Color")),
        transparency: Seq::read(d.prop(r, "Transparency"), 0.5),
        texture: texture_of(d, r),
        tex_length: num(d, r, "TextureLength", 1.0),
        tex_mode: enum_u32(d.prop(r, "TextureMode")).unwrap_or(0),
        tex_speed: num(d, r, "TextureSpeed", 1.0),
        zoffset: num(d, r, "ZOffset", 0.0),
        look: look(d, r),
    }
}

#[derive(Clone)]
struct TrailCfg {
    enabled: bool,
    a0: Option<Ref>,
    a1: Option<Ref>,
    life: f32,
    min_length: f32,
    max_length: f32,
    width_scale: Seq,
    face_camera: bool,
    color: CSeq,
    transparency: Seq,
    texture: Option<String>,
    tex_length: f32,
    tex_mode: u32,
    look: Look,
}

fn trail_cfg(d: &Document, r: Ref) -> TrailCfg {
    TrailCfg {
        enabled: flag(d, r, "Enabled", true),
        a0: reference(d, r, "Attachment0"),
        a1: reference(d, r, "Attachment1"),
        life: num(d, r, "Lifetime", 2.0).max(1e-3),
        min_length: num(d, r, "MinLength", 0.1).max(0.0),
        max_length: num(d, r, "MaxLength", 0.0).max(0.0),
        width_scale: Seq::read(d.prop(r, "WidthScale"), 1.0),
        face_camera: flag(d, r, "FaceCamera", false),
        color: CSeq::read(d.prop(r, "Color")),
        transparency: Seq::read(d.prop(r, "Transparency"), 0.5),
        texture: texture_of(d, r),
        tex_length: num(d, r, "TextureLength", 1.0),
        tex_mode: enum_u32(d.prop(r, "TextureMode")).unwrap_or(0),
        look: look(d, r),
    }
}

#[derive(Clone)]
enum Cfg {
    Constraint(Ref),
    Emitter(Box<EmitterCfg>),
    Beam(Box<BeamCfg>),
    Trail(Box<TrailCfg>),
}

// ------------------------------------------------------------- simulation

/// xorshift64*: cheap, and seeded per emitter so runs repeat.
struct Rng(u64);

impl Rng {
    fn seeded(r: Ref) -> Rng {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&r, &mut h);
        Rng(std::hash::Hasher::finish(&h) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// 0..1
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// −1..1
    fn sym(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
    fn in_range(&mut self, r: (f32, f32)) -> f32 {
        r.0 + (r.1 - r.0) * self.unit()
    }
}

struct Particle {
    /// World position, or the host's local one when LockedToPart.
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
    rot: f32,
    spin: f32,
    /// Envelope draws (−1..1) for Size, Transparency and Squash.
    env: [f32; 3],
    frame0: f32,
    fps: f32,
    seed: u32,
}

#[derive(Default)]
struct EmitterState {
    particles: Vec<Particle>,
    carry: f32,
    last: Option<Vec3>,
    rng: Option<Rng>,
}

struct TrailPoint {
    a: Vec3,
    b: Vec3,
    born: f64,
    /// Path length from the trail's first point (Static / Wrap textures).
    s: f32,
}

#[derive(Default)]
struct TrailState {
    points: VecDeque<TrailPoint>,
}

/// A NormalId as a unit vector: Right, Top, Back, Left, Bottom, Front.
fn normal_id(n: u32) -> Vec3 {
    match n {
        0 => Vec3::X,
        1 => Vec3::Y,
        2 => Vec3::Z,
        3 => -Vec3::X,
        4 => -Vec3::Y,
        _ => -Vec3::Z,
    }
}

/// Two axes across a direction (the SpreadAngle X / Y axes for a face).
fn across(n: Vec3) -> (Vec3, Vec3) {
    if n.y.abs() > 0.9 {
        (Vec3::X, Vec3::Z)
    } else if n.x.abs() > 0.9 {
        (Vec3::Y, Vec3::Z)
    } else if n.z.abs() > 0.9 {
        (Vec3::X, Vec3::Y)
    } else {
        let a = n.any_orthonormal_vector();
        (a, n.cross(a))
    }
}

/// The axes SpreadAngle turns a direction about: (Y's axis, X's axis).
/// X's is the part's X made square to the direction (Z when the direction
/// lies along X); Y's is the third.
fn spread_axes(n: Vec3) -> (Vec3, Vec3) {
    let x = if n.x.abs() > 0.9 { Vec3::Z } else { Vec3::X };
    let b = (x - n * n.dot(x)).normalize_or(n.any_orthonormal_vector());
    (n.cross(b).normalize_or(n.any_orthonormal_vector()), b)
}

/// A new particle in the host's frame: where on the Shape, which way.
fn spawn_local(c: &EmitterCfg, size: Vec3, rng: &mut Rng) -> (Vec3, Vec3) {
    let n = normal_id(c.dir);
    let (u, v) = across(n);
    let half = size * 0.5;
    let surface = c.style == 1;
    let (pos, base) = match c.shape {
        // Sphere: on / in the ellipsoid, outward along the radius;
        // ShapePartial keeps the cap within that share of the sphere
        // around EmissionDirection (1 whole, 0.5 a hemisphere).
        1 => {
            let z = 1.0 - 2.0 * c.partial * rng.unit();
            let phi = rng.unit() * std::f32::consts::TAU;
            let s = (1.0 - z * z).max(0.0).sqrt();
            let d = n * z + (u * phi.cos() + v * phi.sin()) * s;
            let k = if surface { 1.0 } else { rng.unit().cbrt() };
            (d * half * k, d)
        }
        // Cylinder: a Roblox cylinder along the part's X axis (Z when
        // EmissionDirection lies along X: the axis SpreadAngle.X turns
        // about), its radii the other two extents. Surface: the side only
        // (no caps); Volume: inside. Both move straight out from the axis.
        // ShapePartial: the radius at the +axis end (a cone at 0.5).
        // Calibration h (6 × 4 × 2 part seen from three sides): Top and
        // Front draw a 6-long block whose end view is a 4 × 2 ellipse (a
        // ring for Surface), Right one along Z; 0.5 narrows toward +X.
        2 => {
            let (a, b) = spread_axes(n);
            let ext = |ax: Vec3| (ax.abs() * half).element_sum();
            let phi = rng.unit() * std::f32::consts::TAU;
            let radial = n * phi.cos() + a * phi.sin();
            let ring = n * (phi.cos() * ext(n)) + a * (phi.sin() * ext(a));
            let h = rng.sym();
            let taper = 1.0 + (c.partial - 1.0) * (h + 1.0) * 0.5;
            let k = if surface { 1.0 } else { rng.unit().sqrt() };
            (b * (h * ext(b)) + ring * (taper * k), radial)
        }
        // Disc: lies across EmissionDirection with the part's other two
        // extents as radii, and moves along it. Surface: on the emitting
        // face; Volume: anywhere through the part (calibration h: Top
        // Surface draws a 6 × 2 ellipse at the part's top seen from above
        // and a line from the front; Volume fills the 6 × 4 front). Filled
        // either way; ShapePartial is the share of the radius that's filled,
        // from the rim in (0.5 a ring of half the radius).
        3 => {
            let (a, b) = spread_axes(n);
            let ext = |ax: Vec3| (ax.abs() * half).element_sum();
            let phi = rng.unit() * std::f32::consts::TAU;
            let inner = (1.0 - c.partial).clamp(0.0, 1.0).powi(2);
            let k = (inner + (1.0 - inner) * rng.unit()).sqrt();
            let along = if surface { 1.0 } else { rng.sym() };
            (b * (phi.cos() * ext(b) * k) + a * (phi.sin() * ext(a) * k) + n * (along * ext(n)), n)
        }
        // Box: anywhere in the part (Volume) or on the face EmissionDirection
        // names (Surface; calibration h: Top draws only the top face, Right
        // only the +X one), moving along EmissionDirection.
        _ => {
            let mut p = Vec3::new(rng.sym(), rng.sym(), rng.sym()) * half;
            if surface {
                p = p - n * n.dot(p) + n * (n.abs() * half).element_sum();
            }
            (p, n)
        }
    };
    // ShapeInOut: Outward, Inward, InAndOut.
    let base = match c.inout {
        1 => -base,
        2 if rng.unit() < 0.5 => -base,
        _ => base,
    };
    // SpreadAngle: X turns the direction about the part's X axis (Z when
    // the direction lies along X), Y about the third axis (calibration d:
    // Top spreads X into Z and Y into X, Front X into Y and Y into X, Right
    // X into Y and Y into Z).
    let (a, b) = spread_axes(base);
    let rx = (rng.sym() * c.spread.x).to_radians();
    let ry = (rng.sym() * c.spread.y).to_radians();
    let dir = Quat::from_axis_angle(a, ry) * (Quat::from_axis_angle(b, -rx) * base);
    (pos, dir.normalize_or(base))
}

impl EmitterState {
    /// `moving`: the host's velocity when it is set (an anchored part's
    /// AssemblyLinearVelocity), None to take it from the host's motion.
    /// `wind`: Workspace.GlobalWind.
    #[allow(clippy::too_many_arguments)]
    fn step(&mut self, c: &EmitterCfg, r: Ref, xf: &Xform, size: Vec3, dt: f32, emit: u32, moving: Option<Vec3>, wind: Vec3) {
        let rng = self.rng.get_or_insert_with(|| Rng::seeded(r));
        // VelocityInheritance takes the host's physics velocity: an anchored
        // part moved by CFrame passes on none (calibration s: Rate 10 off a
        // part slid 4 studs in 1 s leaves a 4-stud line at Inheritance 1
        // and at 0.5 alike).
        let host_vel = match (moving, self.last) {
            (Some(v), _) => v,
            (None, Some(p)) if dt > 0.0 => (xf.pos - p) / dt,
            _ => Vec3::ZERO,
        };
        self.last = Some(xf.pos);
        let dts = dt * c.time_scale;
        let mut n = emit as usize;
        if c.enabled && c.rate > 0.0 {
            // (Studio makes at most FX_MAX_RATE a second: effects-lanes)
            self.carry += c.rate.min(mech::FX_MAX_RATE) * dts;
            let k = self.carry.floor();
            self.carry -= k;
            n += k as usize;
        }
        let n = n.min(mech::FX_PARTICLES_PER_EMITTER.saturating_sub(self.particles.len()));
        let frames = (c.grid.0 * c.grid.1).max(1) as f32;
        for _ in 0..n {
            let (p, dir) = spawn_local(c, size, rng);
            let speed = rng.in_range(c.speed);
            let (pos, vel) = if c.locked {
                (p, dir * speed)
            } else {
                (xf.pos + xf.rot * p, xf.rot * (dir * speed) + host_vel * c.inherit)
            };
            let life = rng.in_range(c.life).max(0.0);
            if life <= 0.0 {
                continue;
            }
            self.particles.push(Particle {
                pos,
                vel,
                age: 0.0,
                life,
                rot: rng.in_range(c.rotation),
                spin: rng.in_range(c.rot_speed),
                env: [rng.sym(), rng.sym(), rng.sym()],
                frame0: if c.flip_random_start { (rng.unit() * frames).floor() } else { 0.0 },
                fps: rng.in_range(c.flip_fps),
                seed: rng.next() as u32,
            });
        }
        // Motion: Acceleration (world, or the part's frame when
        // LockedToPart: calibration s, +X on a part turned 90° about Z
        // moves the particle up), Drag (speed halves every 1 / Drag s,
        // toward GlobalWind with WindAffectsDrag).
        let accel = c.accel;
        let wind = if !c.wind { Vec3::ZERO } else if c.locked { xf.rot.transpose() * wind } else { wind };
        let keep = 0.5f32.powf(c.drag * dts);
        for p in &mut self.particles {
            p.age += dts;
            p.vel += accel * dts;
            if c.drag != 0.0 {
                p.vel = wind + (p.vel - wind) * keep;
            }
            p.pos += p.vel * dts;
            // (RotSpeed turns the other way from Rotation: calibration f,
            // +10°/s turns a particle clockwise on screen, Rotation +30 is
            // counter-clockwise)
            p.rot -= p.spin * dts;
        }
        self.particles.retain(|p| p.age < p.life);
    }
}

// ----------------------------------------------------------------- drawing

/// The camera as the effects need it.
struct View<'a> {
    vp: Mat4,
    pos: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    /// Screen points per stud at 1 stud away (pixel-size culling).
    px_per_stud: f32,
    /// The scene's light this frame.
    light: &'a SceneLight,
    /// See-through parts this frame (dimming what's behind them).
    glass: &'a [Glass],
    /// RUSTUDIO_FX_LIGHT_CHECK: the CPU's light goes along for comparison.
    check: bool,
    /// The frame's lists of glass in front of each effect (indices into
    /// `glass`), for the GPU (FxFrame::glass_idx).
    glass_idx: std::cell::RefCell<Vec<u32>>,
}

impl View<'_> {
    /// The see-through parts in front of an effect (centre `c`, radius
    /// `r`, `glass_between`), listed for the GPU: (first, count) in
    /// FxFrame::glass_idx. The vertex shader dims the effect by each one
    /// its point lies behind (world_gui.rs fx_transmit, as Glass::hit).
    fn glass_ids(&self, c: Vec3, r: f32, host: Option<Ref>) -> [u32; 2] {
        if self.glass.is_empty() {
            return [0, 0];
        }
        let mut idx = self.glass_idx.borrow_mut();
        let first = idx.len();
        idx.extend(glass_in_front(self.glass, self.pos, c, r, host).map(|i| i as u32));
        let n = idx.len() - first;
        if n == 0 {
            return [0, 0];
        }
        [first as u32, n as u32]
    }

    /// RUSTUDIO_FX_LIGHT_CHECK: the CPU's light × what shows through the
    /// glass in `list` (`glass_ids`), as the GPU should have it.
    fn check(&self, light: [f32; 3], list: [u32; 2], p: Vec3) -> [f32; 3] {
        let idx = self.glass_idx.borrow();
        let seg = p - self.pos;
        let len = seg.length();
        let d = seg / len.max(1e-6);
        let mut t = 1.0;
        for &i in &idx[list[0] as usize..(list[0] + list[1]) as usize] {
            let g = &self.glass[i as usize];
            if g.hit(self.pos, d, len) {
                t *= 1.0 - g.alpha;
            }
        }
        light.map(|c| c * t)
    }

    /// The light an effect takes at `p` (Brightness alone when
    /// LightInfluence is 0).
    fn light_at(&self, look: &Look, p: Vec3, lights: &[&crate::render::LightRaw]) -> [f32; 3] {
        if !look.lit() {
            return look.light(Vec3::ZERO);
        }
        look.light(self.light.at(p, lights))
    }
}

/// Premultiplied gamma-space colour.
fn premul(rgb: Vec3, a: f32) -> [u8; 4] {
    let a = a.clamp(0.0, 1.0);
    let c = (rgb.clamp(Vec3::ZERO, Vec3::ONE) * a * 255.0).round();
    [c.x as u8, c.y as u8, c.z as u8, (a * 255.0).round() as u8]
}

/// One effect's geometry, drawn far to near with the others.
struct Draw {
    dist: f32,
    texture: TextureId,
    wrap: bool,
    geom: Geom,
}

enum Geom {
    Strip(Vec<FxVertex>, Vec<u32>),
    Particles(Vec<FxParticle>),
    /// A beam's kept strip (`Effects::kept`).
    Kept(Ref),
}

/// A beam's strip kept from frame to frame: with the light and the glass on
/// the GPU, a beam that doesn't face the camera or scroll its texture
/// doesn't change until it, its attachments or its parts do (Studio's
/// BeamNode rebuilds on change). The glass list in front of it is redone
/// each frame.
struct Kept {
    tex: (TextureId, bool),
    vertices: Vec<FxVertex>,
    indices: Vec<u32>,
    /// Bounding sphere (the glass in front is picked by it).
    centre: Vec3,
    radius: f32,
}

impl Kept {
    fn new(tex: (TextureId, bool), vertices: Vec<FxVertex>, indices: Vec<u32>) -> Kept {
        // (as `strip`: a FaceCamera row reaches its half width every way)
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for v in &vertices {
            let reach = Vec3::splat(v.face[3].abs());
            lo = lo.min(Vec3::from(v.pos) - reach);
            hi = hi.max(Vec3::from(v.pos) + reach);
        }
        Kept { tex, vertices, indices, centre: (lo + hi) * 0.5, radius: (hi - lo).length() * 0.5 }
    }

    /// This frame's glass list and texture scroll, and how far it is
    /// (`strip`'s `far`).
    fn frame(&mut self, view: &View, scroll: f32) -> f32 {
        let glass = view.glass_ids(self.centre, self.radius, None);
        let scroll = if self.tex.1 { scroll } else { 0.0 };
        let mut far = 0.0f32;
        for v in self.vertices.iter_mut() {
            v.glass = glass;
            v.scroll = scroll;
        }
        // (a row's two vertices share its centre)
        for v in self.vertices.iter().step_by(2) {
            let p = Vec3::from(v.at);
            let shifted = if v.zoff != 0.0 { p + (view.pos - p).normalize_or_zero() * v.zoff } else { p };
            far = far.max((shifted - view.pos).dot(view.forward));
        }
        far
    }
}

/// The texture's id (white when the effect has none); None while loading
/// or failed (Studio draws nothing then).
fn texture_id(
    tex: &Option<String>,
    load: &mut dyn FnMut(&str) -> Option<TextureId>,
) -> Option<(TextureId, bool)> {
    match tex {
        Some(id) => load(id).map(|t| (t, true)),
        None => Some((TextureId::default(), false)),
    }
}

fn particles_draw(
    c: &EmitterCfg,
    st: &EmitterState,
    xf: &Xform,
    view: &View,
    tex: (TextureId, bool),
    budget: &mut usize,
) -> Option<Draw> {
    let white = egui::epaint::WHITE_UV;
    let (cols, rows) = (c.grid.0.max(1), c.grid.1.max(1));
    let frames = cols * rows;
    // (centre, distance) of each live particle, far first; and their
    // bounding sphere (lights and glass are picked per emitter).
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    let mut order: Vec<(f32, usize, Vec3)> = st
        .particles
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let w = if c.locked { xf.pos + xf.rot * p.pos } else { p.pos };
            lo = lo.min(w);
            hi = hi.max(w);
            ((w - view.pos).dot(view.forward), i, w)
        })
        .collect();
    order.sort_by(|a, b| b.0.total_cmp(&a.0));
    let max_size = c.size.0.iter().map(|k| (k.1 + k.2.abs()) * mech::FX_PARTICLE_HALF_SIZE).fold(0.0f32, f32::max);
    let centre = (lo + hi) * 0.5;
    let radius = (hi - lo).length() * 0.5 + max_size;
    // (RUSTUDIO_FX_LIGHT_CHECK: the lights the CPU reference reads)
    let lights = if view.check && c.look.lit() {
        view.light.near(centre, radius + mech::FX_LIGHT_TOWARD_CAMERA)
    } else {
        Vec::new()
    };
    let look = c.look.gpu(false);
    let host = match c.host {
        Host::Part(p) => Some(p),
        _ => None,
    };
    let glass = view.glass_ids(xf.pos, radius + (xf.pos - centre).length(), host);
    let mut out = Vec::with_capacity(order.len().min(*budget));
    let mut far = 0.0f32;
    for (depth, i, center) in order {
        if *budget == 0 {
            break;
        }
        let p = &st.particles[i];
        let t = (p.age / p.life.max(1e-6)).clamp(0.0, 1.0);
        let size = c.size.at(t, p.env[0]);
        if size <= 0.0 {
            continue;
        }
        let half = size * mech::FX_PARTICLE_HALF_SIZE;
        // Behind the camera, or too small to colour a pixel.
        if depth < -half * 2.0 || (depth > 0.0 && half * 2.0 * view.px_per_stud / depth < mech::FX_MIN_PIXELS) {
            continue;
        }
        // (the glass in front dims it on the GPU, at the centre)
        let alpha = (1.0 - c.transparency.at(t, p.env[1])).clamp(0.0, 1.0) * c.look.alpha;
        if alpha <= 0.0 {
            continue;
        }
        let color = premul(c.color.at(t), alpha);
        // Lit once per particle, FX_LIGHT_TOWARD_CAMERA toward the camera
        // (ParticleLightingVS), on the GPU; the CPU's for the check.
        let check = if view.check {
            let at = center + (view.pos - center).normalize_or_zero() * mech::FX_LIGHT_TOWARD_CAMERA;
            view.check(view.light_at(&c.look, at, &lights), glass, center)
        } else {
            [0.0; 3]
        };
        // Squash: x × 1/(1+s), y × (1+s) for s > 0, the inverse below (area
        // kept), then the rotation (ParticleCustomVS).
        let s = c.squash.at(t, p.env[2]);
        let (sa, sb) = if s <= 0.0 { (1.0 - s, 1.0 / (1.0 - s)) } else { (1.0 / (1.0 + s), 1.0 + s) };
        let (sin, cos) = p.rot.to_radians().sin_cos();
        let m = [cos * half * sa, -sin * half * sb, sin * half * sa, cos * half * sb];
        // ParticleOrientation: FacingCamera, FacingCameraWorldUp,
        // VelocityParallel, VelocityPerpendicular (ParticleCustomVS's axes).
        let vel = if c.locked { xf.rot * p.vel } else { p.vel };
        let speed = vel.length();
        // Calibration j / k (a texture of four coloured quarters): the
        // image's X runs along X, its Y along Y, and X × Y = −velocity.
        let (ax, ay) = match c.orientation {
            1 => (view.right, Vec3::Y),
            // VelocityParallel: X along the velocity, Y = forward × v (a
            // particle moving right shows its image upside down).
            2 if speed > 1e-4 => {
                let v = vel / speed;
                (v, view.forward.cross(v).normalize_or(view.up))
            }
            // VelocityPerpendicular: Y is world up made square to the
            // velocity, X = v × Y (moving away shows the image as is,
            // moving toward the camera mirrored left to right, whatever the
            // part's turn). Straight up or down: Y = v × the part's X.
            3 if speed > 1e-4 => {
                let v = vel / speed;
                let up = Vec3::Y - v * v.y;
                let y = if up.length_squared() > 1e-6 {
                    up.normalize()
                } else {
                    v.cross(xf.rot.x_axis).normalize_or(Vec3::Z)
                };
                (v.cross(y), y)
            }
            _ => (view.right, view.up),
        };
        // The flipbook frame (row-major from the top left), and with
        // FlipbookBlendFrames the next one and how far toward it
        // (ParticleCustomVS: TEXCOORD7 = frame, next frame, fraction).
        let (frame, next, blend) = if frames <= 1 {
            (0, 0, 0.0)
        } else {
            let x = (p.frame0 + p.age * p.fps).max(0.0);
            let k = x.floor() as u32;
            let f = x - x.floor();
            match c.flip_mode {
                // OneShot: once over the lifetime, the last frame reached at
                // its end (calibration i, Grid4x4: half way through the
                // life shows frames 7 and 8 mixed half and half, 0.625 of
                // it 9 → 10 at 0.375).
                1 => {
                    let x = t * (frames - 1) as f32;
                    let k = (x as u32).min(frames - 1);
                    (k, (k + 1).min(frames - 1), x - x.floor())
                }
                // PingPong: there and back.
                2 => {
                    let period = (2 * frames - 2).max(1);
                    let at = |j: u32| {
                        let j = j % period;
                        if j < frames { j } else { period - j }
                    };
                    (at(k), at(k + 1), f)
                }
                // Random: a new random frame at each tick of the frame rate
                // (not blended).
                3 => ((p.seed.wrapping_add(k.wrapping_mul(0x9e37_79b9)) >> 7) % frames, 0, 0.0),
                _ => (k % frames, (k + 1) % frames, f),
            }
        };
        let blend = if c.flip_blend && tex.1 { blend } else { 0.0 };
        let cell = Vec2::new(1.0 / cols as f32, 1.0 / rows as f32);
        let origin = Vec2::new((frame % cols) as f32, (frame / cols) as f32) * cell;
        let origin2 = Vec2::new((next % cols) as f32, (next / cols) as f32) * cell;
        let (uv0, uv1, cell) = if tex.1 {
            (origin.to_array(), origin2.to_array(), cell.to_array())
        } else {
            ([white.x, white.y], [white.x, white.y], [0.0, 0.0])
        };
        // Soft particles: only the camera-facing ones (ParticleCustomFS_Soft
        // tests the orientation for 0), faded over their half size.
        let soft = if c.orientation == 0 { 1.0 / half.max(1e-3) } else { 0.0 };
        // The quad's corners: centre ± e1 ± e2 (ParticleCustomVS).
        out.push(FxParticle {
            centre: center.to_array(),
            zoffset: c.zoffset,
            e1: (ax * m[0] + ay * m[2]).to_array(),
            soft,
            e2: (ax * m[1] + ay * m[3]).to_array(),
            blend,
            uv0,
            uv1,
            cell,
            color,
            look,
            check,
            glass,
        });
        far = far.max(depth);
        *budget -= 1;
    }
    if out.is_empty() {
        return None;
    }
    Some(Draw { dist: far, texture: tex.0, wrap: false, geom: Geom::Particles(out) })
}

/// A strip of (point, side half-vector, t along, u) rows as triangles;
/// Transparency is read `fade_back` earlier along than Color (trails).
/// `face`: per row the (tangent, half width) of a FaceCamera beam, whose
/// side the GPU makes across the camera (the rows' side unused); `scroll`
/// is taken off every u on the GPU (Beam.TextureSpeed).
#[allow(clippy::too_many_arguments)]
fn strip(
    rows: &[(Vec3, Vec3, f32, f32)],
    color: &CSeq,
    transparency: &Seq,
    fade_back: f32,
    look: &Look,
    zoffset_pos: f32,
    beam_shadow: bool,
    face: &[(Vec3, f32)],
    scroll: f32,
    view: &View,
    tex: (TextureId, bool),
) -> Option<Draw> {
    if rows.len() < 2 {
        return None;
    }
    let white = egui::epaint::WHITE_UV;
    let face = (face.len() == rows.len()).then_some(face);
    // Bounds: lights and glass are picked per strip (`Kept::bounds`).
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for (i, r) in rows.iter().enumerate() {
        let reach = match face {
            Some(f) => Vec3::splat(f[i].1.abs()),
            None => r.1.abs(),
        };
        lo = lo.min(r.0 - reach);
        hi = hi.max(r.0 + reach);
    }
    let centre = (lo + hi) * 0.5;
    let radius = (hi - lo).length() * 0.5;
    let lights = if view.check && look.lit() { view.light.near(centre, radius) } else { Vec::new() };
    let glass = view.glass_ids(centre, radius, None);
    let gpu = look.gpu(beam_shadow);
    let mut vertices = Vec::with_capacity(rows.len() * 2);
    let mut indices = Vec::with_capacity((rows.len() - 1) * 6);
    let mut far = 0.0f32;
    for (i, &(p, side, t, u)) in rows.iter().enumerate() {
        // Beams: ZOffset moves the whole beam toward the camera (TrailVS;
        // the GPU moves the row, as here).
        let shifted = if zoffset_pos != 0.0 { p + (view.pos - p).normalize_or_zero() * zoffset_pos } else { p };
        // (the glass in front dims it on the GPU, at the row's centre)
        let alpha = (1.0 - transparency.at((t - fade_back).max(0.0), 0.0)).clamp(0.0, 1.0) * look.alpha;
        let col = premul(color.at(t), alpha);
        // Lit at the row's centre (TrailFSLight reads the grid at the
        // pixel; here per row), on the GPU; the CPU's for the check.
        let check = if view.check { view.check(view.light_at(look, shifted, &lights), glass, shifted) } else { [0.0; 3] };
        // The image runs with its Y along the strip (Y 0 at Attachment0 /
        // the head) and its X across, X 0 on the +side edge (effects place:
        // the arrow cursor on a flat beam lies on its side in Studio).
        let (uv0, uv1) = if tex.1 { ([0.0, u], [1.0, u]) } else { ([white.x, white.y], [white.x, white.y]) };
        let scroll = if tex.1 { scroll } else { 0.0 };
        for (sign, uv) in [(1.0f32, uv0), (-1.0, uv1)] {
            let (pos, face) = match face {
                Some(f) => (p, [f[i].0.x, f[i].0.y, f[i].0.z, sign * f[i].1]),
                None => (p + side * sign, [0.0; 4]),
            };
            vertices.push(FxVertex {
                pos: pos.to_array(),
                zoff: zoffset_pos,
                at: p.to_array(),
                color: col,
                uv,
                look: gpu,
                check,
                glass,
                face,
                scroll,
            });
        }
        far = far.max((shifted - view.pos).dot(view.forward));
        if i > 0 {
            let b = (i as u32 - 1) * 2;
            indices.extend_from_slice(&[b, b + 1, b + 3, b, b + 3, b + 2]);
        }
    }
    // (clamped when untextured: repeat would filter egui's white texel at
    // the atlas corner with the far edges' empty ones)
    Some(Draw { dist: far, texture: tex.0, wrap: tex.1, geom: Geom::Strip(vertices, indices) })
}

fn bezier(p: [Vec3; 4], t: f32) -> (Vec3, Vec3) {
    let s = 1.0 - t;
    let pos = p[0] * (s * s * s) + p[1] * (3.0 * s * s * t) + p[2] * (3.0 * s * t * t) + p[3] * (t * t * t);
    let tan = (p[1] - p[0]) * (3.0 * s * s) + (p[2] - p[1]) * (6.0 * s * t) + (p[3] - p[2]) * (3.0 * t * t);
    (pos, tan)
}

/// Stable tangent, including the limiting direction at collapsed handles.
fn beam_tangent(ctrl: [Vec3; 4], t: f32, tan: Vec3, fallback: Vec3) -> Vec3 {
    let limit = if t < 0.5 { ctrl[2] - ctrl[0] } else { ctrl[3] - ctrl[1] };
    tan.normalize_or(limit.normalize_or((ctrl[3] - ctrl[0]).normalize_or(fallback)))
}

/// How far a beam's texture has scrolled toward Attachment1 (TextureSpeed).
fn beam_scroll(c: &BeamCfg, time: f64, phase: Option<(f32, f64)>) -> f32 {
    match phase {
        // SetTextureOffset: the cycle restarts from −offset then scrolls on.
        Some((o, at)) => ((time - at) * c.tex_speed as f64 - o as f64).rem_euclid(1024.0) as f32,
        None => (time * c.tex_speed as f64).rem_euclid(1024.0) as f32,
    }
}

fn beam_draw(c: &BeamCfg, ends: (Option<Xform>, Option<Xform>), view: &View, time: f64, tex: (TextureId, bool), phase: Option<(f32, f64)>) -> Option<Draw> {
    let (x0, x1) = (ends.0?, ends.1?);
    // Attachment.Axis (X) is the curve's direction at each end; CurveSize
    // places the bezier's control points along it.
    let ctrl = [
        x0.pos,
        x0.pos + x0.rot.x_axis * c.curve.0,
        x1.pos - x1.rot.x_axis * c.curve.1,
        x1.pos,
    ];
    let n = c.segments;
    let mut pts: Vec<(Vec3, Vec3, f32, f32)> = Vec::with_capacity(n + 1);
    let mut len = 0.0f32;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let (p, tan) = bezier(ctrl, t);
        if let Some(&(q, _, _, _)) = pts.last() {
            len += (p - q).length();
        }
        pts.push((p, tan, t, len));
    }
    let scroll = beam_scroll(c, time, phase);
    let mut face = Vec::new();
    let rows: Vec<(Vec3, Vec3, f32, f32)> = pts
        .iter()
        .map(|&(p, tan, t_curve, s)| {
            // Along the beam by arc length, not the curve parameter (a
            // straight beam's bezier bunches up at its ends: effects place,
            // the arrow on the flat beam).
            let t = if len > 1e-6 { s / len } else { t_curve };
            // At a zero-curve endpoint the derivative is zero, but its limit
            // points along the beam, not the attachment's unrelated X axis.
            // With two segments, the old fallback twisted the entire end quad.
            let tan = beam_tangent(ctrl, t_curve, tan, x0.rot.x_axis);
            // Width: across the camera (FaceCamera), else along the
            // attachments' SecondaryAxis (Y), blended along the beam and
            // squared to the curve (Place B's flat conveyor and waterfall
            // beams lie along their attachments' Y).
            // (FaceCamera: tan × (camera − p), made on the GPU: vs_strip)
            let w = c.width.0 + (c.width.1 - c.width.0) * t;
            let side = if c.face_camera {
                face.push((tan, w * 0.5));
                Vec3::ZERO
            } else {
                let y = x0.rot.y_axis.lerp(x1.rot.y_axis, t);
                (y - tan * y.dot(tan)).normalize_or(y.normalize_or(Vec3::Y))
            };
            // TextureMode: Stretch (TextureLength copies over the beam),
            // Wrap and Static (one every TextureLength studs from
            // Attachment0: effects-lanes, a 15-stud beam at TextureLength 4
            // cuts its fourth arrow at Attachment1 in both, at Width 2 and
            // 4); TextureSpeed scrolls it toward Attachment1.
            let tl = c.tex_length.max(1e-3);
            // (the scroll is taken off on the GPU)
            let u = match c.tex_mode {
                1 | 2 => s / tl,
                _ => t * c.tex_length,
            };
            (p, side * (w * 0.5), t, u)
        })
        .collect();
    strip(&rows, &c.color, &c.transparency, 0.0, &c.look, c.zoffset, true, &face, scroll, view, tex)
}

/// Where a trail point of this age reads its Color (and WidthScale and a
/// Stretch texture; 0 the head, 1 the tail): FX_TRAIL_AGE_LEAD older than
/// it is (effects-lanes: Studio's white → green trail is already 0.12 s
/// along at its head). Transparency leads by FX_TRAIL_FADE_LEAD (`strip`).
fn age_t(c: &TrailCfg, age: f32) -> f32 {
    ((age + mech::FX_TRAIL_AGE_LEAD) / c.life).clamp(0.0, 1.0)
}

impl TrailState {
    fn step(&mut self, c: &TrailCfg, a: Vec3, b: Vec3, now: f64) {
        let mid = (a + b) * 0.5;
        if c.enabled {
            match self.points.back() {
                Some(l) => {
                    let step = ((l.a + l.b) * 0.5 - mid).length();
                    if step >= c.min_length.max(1e-4) {
                        let s = l.s + step;
                        self.points.push_back(TrailPoint { a, b, born: now, s });
                    }
                }
                None => self.points.push_back(TrailPoint { a, b, born: now, s: 0.0 }),
            }
        }
        // (expired FX_TRAIL_AGE_LEAD early, as Studio's trails end short)
        let life = (c.life - mech::FX_TRAIL_AGE_LEAD).max(1e-3) as f64;
        while self.points.front().is_some_and(|p| now - p.born > life) {
            self.points.pop_front();
        }
        while self.points.len() > mech::FX_TRAIL_MAX_POINTS {
            self.points.pop_front();
        }
        if c.max_length > 0.0 {
            let head = self.points.back().map_or(0.0, |p| p.s);
            while self.points.front().is_some_and(|p| head - p.s > c.max_length) {
                self.points.pop_front();
            }
        }
    }

    fn draw(&self, c: &TrailCfg, head: Option<(Vec3, Vec3)>, view: &View, now: f64, tex: (TextureId, bool)) -> Option<Draw> {
        // Oldest first, then the attachments where they are now.
        let mut pts: Vec<(Vec3, Vec3, f32, f32)> = self
            .points
            .iter()
            .map(|p| (p.a, p.b, age_t(c, (now - p.born) as f32), p.s))
            .collect();
        if let (Some((a, b)), true) = (head, c.enabled) {
            let s = self.points.back().map_or(0.0, |l| l.s + ((l.a + l.b) * 0.5 - (a + b) * 0.5).length());
            pts.push((a, b, age_t(c, 0.0), s));
        }
        if pts.len() < 2 {
            return None;
        }
        let s_head = pts.last().map_or(0.0, |p| p.3);
        let tl = c.tex_length.max(1e-3);
        let rows: Vec<(Vec3, Vec3, f32, f32)> = (0..pts.len())
            .map(|i| {
                let (a, b, t, s) = pts[i];
                let mid = (a + b) * 0.5;
                // WidthScale over the lifetime, around the midpoint.
                let mut half = (a - b) * 0.5 * c.width_scale.at(t, 0.0);
                if c.face_camera {
                    let next = pts[(i + 1).min(pts.len() - 1)];
                    let prev = pts[i.saturating_sub(1)];
                    let tan = ((next.0 + next.1) - (prev.0 + prev.1)).normalize_or(view.right);
                    half = tan.cross(view.pos - mid).normalize_or(view.up) * half.length();
                }
                // TextureMode: Stretch (over the lifetime), Wrap (every
                // TextureLength studs back from the attachments), Static
                // (every TextureLength studs, fixed where it was laid).
                let u = match c.tex_mode {
                    1 => (s_head - s) / tl,
                    2 => s / tl,
                    _ => t * c.tex_length,
                };
                (mid, half, t, u)
            })
            .collect();
        // (Transparency leads by FX_TRAIL_FADE_LEAD, not FX_TRAIL_AGE_LEAD)
        let fade_back = (mech::FX_TRAIL_AGE_LEAD - mech::FX_TRAIL_FADE_LEAD) / c.life;
        strip(&rows, &c.color, &c.transparency, fade_back, &c.look, 0.0, false, &[], 0.0, view, tex)
    }
}

// -------------------------------------------------------------- the module

/// Every ParticleEmitter / Beam / Trail under Workspace, and their state.
#[derive(Default)]
pub struct Effects {
    doc_id: u64,
    list: Vec<Ref>,
    set: HashSet<Ref>,
    /// Properties, read again when the journal says one changed.
    cfg: HashMap<Ref, Cfg>,
    emitters: HashMap<Ref, EmitterState>,
    trails: HashMap<Ref, TrailState>,
    /// Beam:SetTextureOffset: (offset, effects time it was set).
    beam_phase: HashMap<Ref, (f32, f64)>,
    time: f64,
    /// `glass`: the see-through parts' indices in the scene, and the
    /// (document, revision, part count) they were found for.
    glass_parts: Vec<usize>,
    glass_key: Option<(u64, u64, usize)>,
    /// The last frame's geometry for the GPU, and whether `drawn` has not
    /// handed it out yet.
    out: FxFrame,
    fresh: bool,
    /// Where each effect is, and per instance the effects whose pose reads
    /// it (dropped when the journal marks it).
    poses: HashMap<Ref, Pose>,
    pose_deps: HashMap<Ref, Vec<Ref>>,
    /// Beams' kept strips (dropped with their pose).
    kept: HashMap<Ref, Kept>,
}

fn is_effect(d: &Document, r: Ref) -> bool {
    d.dom.get_by_ref(r).is_some_and(|i| CLASSES.contains(&i.class.as_str()))
}

/// RUSTUDIO_FX_LIGHT_CHECK: the CPU lights the effects too and the GPU
/// paints magenta where its light differs (world_gui.rs fx_checked).
fn light_check() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTUDIO_FX_LIGHT_CHECK").is_some())
}

/// What a frame needs from the viewport.
pub struct FrameIn<'a> {
    pub doc: &'a mut Document,
    pub cam: &'a Camera,
    pub rect: Rect,
    pub dt: f32,
    /// A texture id → its egui texture, None while loading or failed.
    pub texture: &'a mut dyn FnMut(&str) -> Option<TextureId>,
    /// The scene's light this frame (the viewport's `LightingState::fx`).
    pub light: &'a SceneLight,
    /// The see-through parts in view (`glass_of`).
    pub glass: &'a [Glass],
    /// Play, showing the client: the server DataModel's id and its
    /// instances' client copies, so a server script's `:Emit` shows as
    /// Roblox replicates it.
    pub server: Option<(u64, &'a dyn Fn(Ref) -> Option<Ref>)>,
}


impl Effects {
    /// The see-through parts (Transparency between FX_GLASS_MIN_ALPHA and
    /// opaque) of the scene in front of the camera, the nearest
    /// FX_GLASS_MAX within FX_GLASS_MAX_DIST, nearest first, for
    /// `FrameIn::glass`. Which parts are see-through is kept until the
    /// document's revision moves; where they are is read each frame.
    pub fn glass(&mut self, scene: &crate::scene::Scene, revision: u64, cam: &Camera) -> Vec<Glass> {
        let _p = crate::bench::scope("fx_glass");
        let key = (scene.doc_id, revision, scene.parts.len());
        if self.glass_key != Some(key) {
            self.glass_key = Some(key);
            self.glass_parts = (0..scene.parts.len())
                .filter(|&i| {
                    let a = scene.parts[i].draw.alpha;
                    a > mech::FX_GLASS_MIN_ALPHA && a < 0.999
                })
                .collect();
        }
        let fwd = cam.forward();
        let mut out: Vec<Glass> = self
            .glass_parts
            .iter()
            .filter_map(|&i| {
                let (p, b) = (scene.parts.get(i)?, scene.bounds.get(i)?);
                let centre = Vec3::new(b.x, b.y, b.z);
                let rel = centre - cam.pos;
                let dist = rel.length();
                (dist - b.w < mech::FX_GLASS_MAX_DIST && rel.dot(fwd) > -b.w).then_some(Glass {
                    xf: p.draw.xf,
                    half: p.draw.size * 0.5,
                    alpha: p.draw.alpha,
                    centre,
                    radius: b.w,
                    dist,
                    part: p.r,
                })
            })
            .collect();
        out.sort_by(|a, b| a.dist.total_cmp(&b.dist));
        out.truncate(mech::FX_GLASS_MAX);
        out
    }

    /// This frame's effects for the world GUI pass (once per `frame`).
    pub fn drawn(&mut self) -> Option<&FxFrame> {
        std::mem::take(&mut self.fresh).then_some(&self.out)
    }

    /// No effects under Workspace (as of the last frame).
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// `r` changed: the poses of the effects that read it (and its own)
    /// are read again.
    fn unpose(&mut self, r: Ref) {
        self.poses.remove(&r);
        self.kept.remove(&r);
        if let Some(list) = self.pose_deps.remove(&r) {
            for e in list {
                self.poses.remove(&e);
                self.kept.remove(&e);
            }
        }
    }

    /// Effect `r`'s pose, read from the document when not kept.
    fn pose(poses: &mut HashMap<Ref, Pose>, pose_deps: &mut HashMap<Ref, Vec<Ref>>, d: &Document, r: Ref, cfg: &Cfg) -> Pose {
        if let Some(p) = poses.get(&r) {
            if pose_check() {
                // (RUSTUDIO_FX_POSE_CHECK: the kept pose against a fresh read)
                let kept = *p;
                poses.remove(&r);
                let fresh = Self::pose(poses, pose_deps, d, r, cfg);
                if !kept.same(&fresh) {
                    let name = d.dom.get_by_ref(r).map(|i| i.name.clone()).unwrap_or_default();
                    eprintln!("[fx pose check] {name} {r:?} kept a stale pose");
                }
                return fresh;
            }
            return *p;
        }
        let mut deps = Vec::new();
        let pose = match cfg {
            Cfg::Beam(c) => {
                let mut end = |a: Option<Ref>| {
                    let a = a?;
                    chain_of(d, a, &mut deps);
                    attachment_world(d, a)
                };
                Pose::Ends(end(c.a0), end(c.a1))
            }
            Cfg::Trail(c) => {
                let mut end = |a: Option<Ref>| {
                    let a = a?;
                    chain_of(d, a, &mut deps);
                    attachment_world(d, a)
                };
                Pose::Ends(end(c.a0), end(c.a1))
            }
            Cfg::Emitter(c) => {
                // (an anchored host's velocity is its set one)
                let body = match c.host {
                    Host::Part(p) => {
                        deps.push(p);
                        Some(p)
                    }
                    Host::Attachment(a) => {
                        chain_of(d, a, &mut deps);
                        d.dom.get_by_ref(a).map(|i| i.parent())
                    }
                    Host::None => None,
                };
                if let Some(b) = body {
                    deps.push(b);
                }
                let moving = body
                    .filter(|&p| flag(d, p, "Anchored", false))
                    .map(|p| vec3_of(d.prop(p, "AssemblyLinearVelocity")).unwrap_or(Vec3::ZERO));
                Pose::Host(host_xform(d, c.host), moving)
            }
            Cfg::Constraint(_) => Pose::Ends(None, None),
        };
        for dep in deps {
            let list = pose_deps.entry(dep).or_default();
            if !list.contains(&r) {
                list.push(r);
            }
        }
        poses.insert(r, pose);
        pose
    }

    fn refresh(&mut self, d: &mut Document) {
        let marks = d.marks_for(doc::Reader::Effects);
        if let doc::Marks::Some(marks) = marks {
            if self.doc_id == d.id {
                let ws = d.workspace();
                let mut removed = false;
                for m in marks {
                    match m {
                        doc::Mark::Tree(t) => {
                            self.unpose(t);
                            for r in d.subtree(t) {
                                self.unpose(r);
                                let keep = is_effect(d, r) && ws.is_some_and(|w| d.is_ancestor(w, r));
                                self.cfg.remove(&r);
                                if keep && self.set.insert(r) {
                                    self.list.push(r);
                                } else if !keep && self.set.remove(&r) {
                                    removed = true;
                                }
                            }
                        }
                        doc::Mark::Prop(r, _) => {
                            self.unpose(r);
                            if self.set.contains(&r) {
                                self.cfg.remove(&r);
                            }
                        }
                    }
                }
                if removed {
                    let set = &self.set;
                    self.kept.retain(|r, _| set.contains(r));
                    self.list.retain(|r| set.contains(r));
                    self.emitters.retain(|r, _| set.contains(r));
                    self.trails.retain(|r, _| set.contains(r));
                }
                return;
            }
        }
        self.doc_id = d.id;
        self.list = match d.workspace() {
            Some(ws) => d
                .dom
                .descendants_of(ws)
                .filter(|i| CLASSES.contains(&i.class.as_str()))
                .map(|i| i.referent())
                .collect(),
            None => Vec::new(),
        };
        self.set = self.list.iter().copied().collect();
        self.cfg.clear();
        self.poses.clear();
        self.pose_deps.clear();
        self.kept.clear();
        self.emitters.clear();
        self.trails.clear();
    }

    /// Steps every effect by `dt` and lays out its geometry for the GPU
    /// (`drawn`), far to near. True while something is moving (keep
    /// repainting).
    pub fn frame(&mut self, f: FrameIn<'_>) -> bool {
        let _p = crate::bench::scope("fx");
        self.refresh(f.doc);
        let d: &Document = f.doc;
        let dt = f.dt.clamp(0.0, mech::FX_DT_MAX);
        self.time += dt as f64;
        let now = self.time;

        // Scripts' Emit / Clear for this document (and, on the client in
        // Play, the server's for their client copies).
        let mut emits: HashMap<Ref, u32> = HashMap::new();
        if let Ok(mut q) = REQUESTS.lock() {
            q.retain(|(id, r, req)| {
                let r = if *id == d.id {
                    *r
                } else if let Some(c) = f.server.and_then(|(sid, map)| (sid == *id).then(|| map(*r)).flatten()) {
                    c
                } else {
                    // (the server's own requests for an instance the client
                    // doesn't have go)
                    return !f.server.is_some_and(|(sid, _)| sid == *id);
                };
                let r = &r;
                match req {
                    Req::Emit(n) => *emits.entry(*r).or_default() += n,
                    Req::Clear => {
                        if let Some(s) = self.emitters.get_mut(r) {
                            s.particles.clear();
                        }
                        // Trail:Clear (domains/world_misc.rs).
                        if let Some(t) = self.trails.get_mut(r) {
                            t.points.clear();
                        }
                        emits.remove(r);
                    }
                    Req::Offset(o) => {
                        self.beam_phase.insert(*r, (*o, now));
                        self.kept.remove(r);
                    }
                }
                false
            });
        }

        let cam = f.cam;
        let view = View {
            light: f.light,
            glass: f.glass,
            vp: cam.view_proj_reverse_z(f.rect),
            pos: cam.pos,
            right: cam.right(),
            up: cam.up(),
            forward: cam.forward(),
            px_per_stud: f.rect.height() / (2.0 * (cam.fov_deg.to_radians() * 0.5).tan()),
            check: light_check(),
            glass_idx: std::cell::RefCell::new(std::mem::take(&mut self.out.glass_idx)),
        };
        view.glass_idx.borrow_mut().clear();

        let wind = d.workspace().and_then(|w| vec3_of(d.prop(w, "GlobalWind"))).unwrap_or(Vec3::ZERO);
        let mut draws: Vec<Draw> = Vec::new();
        let mut live = false;
        let mut n_particles = 0usize;
        let (mut t_sim, mut t_strip) = (0.0f64, 0.0f64);
        let (mut n_beams, mut n_kept) = (0u64, 0u64);
        // Emitters nearest first get the particle budget.
        let mut emitters: Vec<(f32, Ref, Xform, Vec3)> = Vec::new();
        for &r in &self.list {
            if !d.exists(r) {
                continue;
            }
            let cfg = self.cfg.entry(r).or_insert_with(|| {
                match d.class_of(r).as_deref() {
                    Some("ParticleEmitter") => Cfg::Emitter(Box::new(emitter_cfg(d, r))),
                    Some("Beam") => Cfg::Beam(Box::new(beam_cfg(d, r))),
                    Some("RopeConstraint" | "RodConstraint" | "SpringConstraint") => Cfg::Constraint(r),
                    _ => Cfg::Trail(Box::new(trail_cfg(d, r))),
                }
            });
            let pose = Self::pose(&mut self.poses, &mut self.pose_deps, d, r, cfg);
            match cfg {
                Cfg::Constraint(r) => {
                    if let Some(dr) = constraints::draw(d, *r, &view) {
                        if dr.dist < mech::FX_MAX_DIST { draws.push(dr); }
                    }
                }
                Cfg::Emitter(c) => {
                    let Pose::Host(Some((xf, size)), moving) = pose else { continue };
                    let dist = (xf.pos - view.pos).length() - size.length() * 0.5;
                    if dist > mech::FX_MAX_DIST {
                        // (out of range: nothing kept, as if not streamed in)
                        if let Some(s) = self.emitters.get_mut(&r) {
                            s.particles.clear();
                            s.last = None;
                        }
                        continue;
                    }
                    let st = self.emitters.entry(r).or_default();
                    let t0 = std::time::Instant::now();
                    st.step(c, r, &xf, size, dt, emits.remove(&r).unwrap_or(0), moving, wind);
                    t_sim += t0.elapsed().as_secs_f64();
                    if !st.particles.is_empty() {
                        live = true;
                        emitters.push((dist, r, xf, size));
                    }
                }
                Cfg::Beam(c) => {
                    if !c.enabled || c.a0.is_none() || c.a1.is_none() {
                        continue;
                    }
                    let Some(tex) = texture_id(&c.texture, f.texture) else { continue };
                    if c.tex_speed != 0.0 {
                        live = true;
                    }
                    let t0 = std::time::Instant::now();
                    let Pose::Ends(x0, x1) = pose else { continue };
                    // (kept while nothing it reads changes: the light, the
                    // glass, FaceCamera's side and the scroll are the GPU's)
                    let keep = !view.check;
                    n_beams += 1;
                    if let Some(k) = self.kept.get_mut(&r).filter(|k| keep && k.tex == tex) {
                        let far = k.frame(&view, beam_scroll(c, now, self.beam_phase.get(&r).copied()));
                        n_kept += 1;
                        if pose_check() {
                            // (the kept strip against a fresh one, glass aside)
                            let fresh = beam_draw(c, (x0, x1), &view, now, tex, self.beam_phase.get(&r).copied());
                            let same = match fresh.map(|d| d.geom) {
                                Some(Geom::Strip(v, i)) => {
                                    i == k.indices
                                        && v.len() == k.vertices.len()
                                        && v.iter().zip(&k.vertices).all(|(a, b)| {
                                            let mut a = *a;
                                            a.glass = b.glass;
                                            a.scroll = b.scroll;
                                            bytemuck::bytes_of(&a) == bytemuck::bytes_of(b)
                                        })
                                }
                                _ => false,
                            };
                            if !same {
                                eprintln!("[fx pose check] beam {r:?} kept a stale strip");
                            }
                        }
                        t_strip += t0.elapsed().as_secs_f64();
                        if far < mech::FX_MAX_DIST {
                            draws.push(Draw { dist: far, texture: tex.0, wrap: tex.1, geom: Geom::Kept(r) });
                        }
                        continue;
                    }
                    let mut dr = beam_draw(c, (x0, x1), &view, now, tex, self.beam_phase.get(&r).copied());
                    if keep {
                        if let Some(Draw { geom, .. }) = dr.as_mut() {
                            if let Geom::Strip(v, i) = std::mem::replace(geom, Geom::Kept(r)) {
                                self.kept.insert(r, Kept::new(tex, v, i));
                            }
                        }
                    }
                    t_strip += t0.elapsed().as_secs_f64();
                    if let Some(dr) = dr {
                        if dr.dist < mech::FX_MAX_DIST {
                            draws.push(dr);
                        }
                    }
                }
                Cfg::Trail(c) => {
                    let Pose::Ends(Some(x0), Some(x1)) = pose else { continue };
                    let st = self.trails.entry(r).or_default();
                    st.step(c, x0.pos, x1.pos, now);
                    if st.points.len() < 2 && !c.enabled {
                        continue;
                    }
                    if !st.points.is_empty() {
                        live = true;
                    }
                    let Some(tex) = texture_id(&c.texture, f.texture) else { continue };
                    if let Some(dr) = st.draw(c, Some((x0.pos, x1.pos)), &view, now, tex) {
                        if dr.dist < mech::FX_MAX_DIST {
                            draws.push(dr);
                        }
                    }
                }
            }
        }
        crate::bench::add("fx_sim", t_sim * 1e3, 1);
        crate::bench::add("fx_beams", t_strip * 1e3, n_beams);
        crate::bench::add("fx_beams_kept", 0.0, n_kept);
        let _pd = crate::bench::scope("fx_pdraw");
        emitters.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut budget = mech::FX_PARTICLES_MAX;
        for (_, r, xf, _) in emitters {
            let (Some(Cfg::Emitter(c)), Some(st)) = (self.cfg.get(&r), self.emitters.get(&r)) else {
                continue;
            };
            n_particles += st.particles.len();
            // (a ParticleEmitter with no Texture draws nothing in Studio,
            // effects place; Beams and Trails without one draw flat colour)
            if c.texture.is_none() {
                continue;
            }
            let Some(tex) = texture_id(&c.texture, f.texture) else { continue };
            if let Some(dr) = particles_draw(c, st, &xf, &view, tex, &mut budget) {
                draws.push(dr);
            }
        }
        crate::bench::add("fx_particles", 0.0, n_particles as u64);
        drop(_pd);
        let _push = crate::bench::scope("fx_push");
        // Far to near, so nearer effects blend over farther ones.
        draws.sort_by(|a, b| b.dist.total_cmp(&a.dist));
        let out = &mut self.out;
        out.clear();
        out.glass_idx = view.glass_idx.take();
        out.glass.extend(view.glass.iter().map(|g| {
            let (r, h) = (g.xf.rot, g.half);
            crate::world_gui::FxGlass {
                x: [r.x_axis.x, r.x_axis.y, r.x_axis.z, h.x],
                y: [r.y_axis.x, r.y_axis.y, r.y_axis.z, h.y],
                z: [r.z_axis.x, r.z_axis.y, r.z_axis.z, h.z],
                pos: [g.xf.pos.x, g.xf.pos.y, g.xf.pos.z, g.alpha],
            }
        }));
        out.vp = view.vp.to_cols_array_2d();
        out.cam = [view.pos.x, view.pos.y, view.pos.z, if view.check { 1.0 } else { 0.0 }];
        out.right = [view.right.x, view.right.y, view.right.z, 0.0];
        let l = f.light;
        out.light = [l.ambient, l.outdoor, l.sun, l.env].map(|v| [v.x, v.y, v.z, 0.0]);
        for dr in draws {
            match dr.geom {
                Geom::Strip(v, i) => out.push_strip(dr.texture, dr.wrap, &v, &i),
                Geom::Particles(p) => out.push_particles(dr.texture, &p),
                Geom::Kept(r) => {
                    if let Some(k) = self.kept.get(&r) {
                        out.push_strip(dr.texture, dr.wrap, &k.vertices, &k.indices);
                    }
                }
            }
        }
        self.fresh = true;
        live
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_beam_endpoint_tangent_limit_follows_chord() {
        let a = Vec3::new(11.26, 24.52, 9023.88);
        let b = Vec3::new(-2.428, 1.679, 9010.0625);
        let ctrl = [a, a, b, b];
        for t in [0.0, 0.5, 1.0] {
            let (_, tan) = bezier(ctrl, t);
            let dir = beam_tangent(ctrl, t, tan, Vec3::X);
            assert!(dir.dot((b - a).normalize()) > 0.99999);
        }
    }

    #[test]
    fn sequences_interpolate_with_envelopes() {
        let s = Seq(vec![(0.0, 1.0, 0.5), (1.0, 3.0, 0.0)]);
        assert!((s.at(0.5, 0.0) - 2.0).abs() < 1e-5);
        assert!((s.at(0.0, 1.0) - 1.5).abs() < 1e-5);
        assert!((s.at(1.0, -1.0) - 3.0).abs() < 1e-5);
        let c = CSeq(vec![(0.0, Vec3::ZERO), (1.0, Vec3::ONE)]);
        assert!((c.at(0.25).x - 0.25).abs() < 1e-5);
    }

    #[test]
    fn spawn_respects_shape_and_spread() {
        let mut c = emitter_test_cfg();
        let mut rng = Rng(12345);
        // Box, Top, no spread: inside the box, straight up.
        for _ in 0..200 {
            let (p, d) = spawn_local(&c, Vec3::new(4.0, 2.0, 6.0), &mut rng);
            assert!(p.x.abs() <= 2.0 && p.y.abs() <= 1.0 && p.z.abs() <= 3.0);
            assert!((d - Vec3::Y).length() < 1e-4);
        }
        // Sphere surface, outward: on the sphere, along the radius.
        c.shape = 1;
        c.style = 1;
        for _ in 0..200 {
            let (p, d) = spawn_local(&c, Vec3::splat(4.0), &mut rng);
            assert!((p.length() - 2.0).abs() < 1e-3);
            assert!((p.normalize() - d).length() < 1e-3);
        }
        // Spread 30° about Top: within 30·√2° of up.
        c.shape = 0;
        c.style = 0;
        c.spread = Vec2::new(30.0, 30.0);
        for _ in 0..200 {
            let (_, d) = spawn_local(&c, Vec3::ONE, &mut rng);
            assert!(d.dot(Vec3::Y) >= (43.0f32).to_radians().cos());
        }
        // SpreadAngle X turns Top toward Z, Y toward X; Front: X toward Y
        // (calibration d).
        for (dir, spread, off) in [(1, Vec2::new(40.0, 0.0), Vec3::X), (1, Vec2::new(0.0, 40.0), Vec3::Z), (5, Vec2::new(40.0, 0.0), Vec3::X)] {
            c.dir = dir;
            c.spread = spread;
            for _ in 0..100 {
                let (_, d) = spawn_local(&c, Vec3::ONE, &mut rng);
                assert!(d.dot(off).abs() < 1e-4, "{dir} {spread:?} {d:?}");
            }
        }
    }

    #[test]
    fn particles_age_move_and_drag() {
        let mut c = emitter_test_cfg();
        c.rate = 10.0;
        c.speed = (4.0, 4.0);
        c.life = (2.0, 2.0);
        c.drag = 1.0;
        let xf = Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY };
        let mut st = EmitterState::default();
        st.step(&c, Ref::new(), &xf, Vec3::ZERO, 0.1, 0, None, Vec3::ZERO);
        assert_eq!(st.particles.len(), 1);
        for _ in 0..10 {
            st.step(&c, Ref::new(), &xf, Vec3::ZERO, 0.1, 0, None, Vec3::ZERO);
        }
        // The first one: 1.1 s old, its speed halved per second.
        let p = &st.particles[0];
        assert!((p.vel.length() - 4.0 * 0.5f32.powf(1.1)).abs() < 1e-3);
        assert_eq!(st.particles.len(), 11);
        // Emit adds at once; past the lifetime they go.
        st.step(&c, Ref::new(), &xf, Vec3::ZERO, 0.0, 5, None, Vec3::ZERO);
        assert_eq!(st.particles.len(), 16);
        c.enabled = false;
        for _ in 0..25 {
            st.step(&c, Ref::new(), &xf, Vec3::ZERO, 0.1, 0, None, Vec3::ZERO);
        }
        assert!(st.particles.is_empty());
    }

    #[test]
    fn trail_lays_points_and_expires() {
        let c = TrailCfg {
            enabled: true,
            a0: None,
            a1: None,
            life: 1.0,
            min_length: 0.5,
            max_length: 3.0,
            width_scale: Seq(vec![(0.0, 1.0, 0.0)]),
            face_camera: false,
            color: CSeq(vec![(0.0, Vec3::ONE)]),
            transparency: Seq(vec![(0.0, 0.0, 0.0)]),
            texture: None,
            tex_length: 1.0,
            tex_mode: 0,
            look: Look { influence: 0.0, brightness: 1.0, emission: 0.0, alpha: 1.0 },
        };
        let mut st = TrailState::default();
        for i in 0..20 {
            let x = i as f32 * 0.25;
            st.step(&c, Vec3::new(x, 1.0, 0.0), Vec3::new(x, -1.0, 0.0), i as f64 * 0.05);
        }
        // A point every 0.5 studs (MinLength), at most 3 studs of them (MaxLength).
        let s: Vec<f32> = st.points.iter().map(|p| p.s).collect();
        assert!(s.windows(2).all(|w| (w[1] - w[0] - 0.5).abs() < 1e-4), "{s:?}");
        assert!(s.last().unwrap() - s[0] <= 3.0 + 1e-4);
        // A second later, all gone.
        st.step(&c, Vec3::new(4.75, 1.0, 0.0), Vec3::new(4.75, -1.0, 0.0), 2.5);
        assert_eq!(st.points.len(), 0);
    }

    #[test]
    fn cylinder_and_disc_follow_the_part() {
        let mut c = emitter_test_cfg();
        let mut rng = Rng(777);
        let size = Vec3::new(6.0, 4.0, 2.0);
        // Cylinder, EmissionDirection Top or Front: along the part's X,
        // radii Y / Z, moving straight out from the axis (calibration h).
        c.shape = 2;
        for dir in [1, 5] {
            c.dir = dir;
            for style in [0, 1] {
                c.style = style;
                for _ in 0..300 {
                    let (p, d) = spawn_local(&c, size, &mut rng);
                    assert!(p.x.abs() <= 3.0 + 1e-4);
                    let r = (p.y / 2.0).powi(2) + (p.z / 1.0).powi(2);
                    assert!(r <= 1.0 + 1e-3);
                    if style == 1 {
                        assert!((r - 1.0).abs() < 1e-3, "surface: the side only");
                    }
                    assert!(d.x.abs() < 1e-4, "out from the axis");
                }
            }
        }
        // EmissionDirection Right: the axis is Z.
        c.dir = 0;
        c.style = 1;
        for _ in 0..100 {
            let (p, d) = spawn_local(&c, size, &mut rng);
            assert!(((p.x / 3.0).powi(2) + (p.y / 2.0).powi(2) - 1.0).abs() < 1e-3 && d.z.abs() < 1e-4);
        }
        // ShapePartial 0.5: the radius halves toward +X.
        c.dir = 1;
        c.partial = 0.5;
        for _ in 0..300 {
            let (p, _) = spawn_local(&c, size, &mut rng);
            let k = 1.0 - 0.5 * (p.x / 3.0 + 1.0) * 0.5;
            assert!((((p.y / 2.0).powi(2) + p.z.powi(2)).sqrt() - k).abs() < 1e-3);
        }
        // Disc, EmissionDirection Right: across X (radii Z / Y), moving +X;
        // Surface on the +X face, Volume anywhere along X; ShapePartial 0.5
        // leaves the inner half empty.
        c.shape = 3;
        c.dir = 0;
        for style in [0, 1] {
            c.style = style;
            for _ in 0..300 {
                let (p, d) = spawn_local(&c, size, &mut rng);
                let r = (p.z / 1.0).powi(2) + (p.y / 2.0).powi(2);
                assert!(r <= 1.0 + 1e-3 && r >= 0.25 - 1e-3);
                assert!(if style == 1 { (p.x - 3.0).abs() < 1e-4 } else { p.x.abs() <= 3.0 + 1e-4 });
                assert!((d - Vec3::X).length() < 1e-4);
            }
        }
        // Box Surface: only the face EmissionDirection names.
        c.shape = 0;
        c.dir = 1;
        c.style = 1;
        for _ in 0..100 {
            let (p, d) = spawn_local(&c, size, &mut rng);
            assert!((p.y - 2.0).abs() < 1e-4 && (d - Vec3::Y).length() < 1e-4);
        }
    }

    #[test]
    fn rate_is_capped() {
        let mut c = emitter_test_cfg();
        c.rate = 5000.0;
        c.life = (10.0, 10.0);
        let xf = Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY };
        let mut st = EmitterState::default();
        for _ in 0..10 {
            st.step(&c, Ref::new(), &xf, Vec3::ONE, 0.1, 0, None, Vec3::ZERO);
        }
        assert_eq!(st.particles.len(), mech::FX_MAX_RATE as usize);
    }

    #[test]
    fn glass_dims_what_is_behind_it() {
        let g = Glass {
            xf: Xform { pos: Vec3::new(0.0, 0.0, -5.0), rot: Mat3::IDENTITY },
            half: Vec3::new(4.0, 4.0, 0.1),
            alpha: 0.6,
            centre: Vec3::new(0.0, 0.0, -5.0),
            radius: 5.7,
            dist: 5.0,
            part: Ref::new(),
        };
        let all = [g];
        let cam = Vec3::ZERO;
        let list = glass_between(&all, cam, Vec3::new(0.0, 0.0, -10.0), 1.0, None);
        assert_eq!(list.len(), 1);
        let vp = Mat4::perspective_infinite_reverse_rh(1.2, 1.0, 0.1);
        // (and with tiles: the same pane many times over)
        let many = GlassSet::new(vec![&all[0]; 9], vp, cam);
        assert!(many.tiles.is_some());
        assert!((many.transmit(cam, Vec3::new(0.5, 0.0, -10.0)) - 0.4f32.powi(9)).abs() < 1e-5);
        let set = GlassSet::new(list, vp, cam);
        assert!((set.transmit(cam, Vec3::new(0.5, 0.0, -10.0)) - 0.4).abs() < 1e-5);
        // Beside it, or in front of it: untouched.
        assert_eq!(set.transmit(cam, Vec3::new(9.0, 0.0, -10.0)), 1.0);
        assert!(glass_between(&all, cam, Vec3::new(0.0, 0.0, -3.0), 1.0, None).is_empty());
    }

    #[test]
    fn scene_light_reads_roofs_and_lights() {
        let mut l = SceneLight { ambient: Vec3::splat(0.1), outdoor: Vec3::splat(0.2), sun: Vec3::splat(0.5), ..Default::default() };
        // Open sky: ambient + outdoor + sun.
        assert!((l.at(Vec3::ZERO, &[]).x - 0.8).abs() < 1e-5);
        // Under a wide roof: the sky's share nearly gone (the sun channel
        // isn't computed: 1).
        let origin = [-128, -32, -128];
        let mut occ = crate::lightgrid::Occupancy::new(origin);
        occ.add_prim(&crate::lightgrid::Prim::new(
            crate::lightgrid::Kind::Block,
            Vec3::new(200.0, 4.0, 200.0),
            &crate::math::Xform { pos: Vec3::new(0.0, 12.0, 0.0), rot: glam::Mat3::IDENTITY },
            0.0,
        ));
        l.grid = Some(Arc::new(crate::lightgrid::skylight(&occ)));
        assert!((l.at(Vec3::ZERO, &[]).x - 0.6).abs() < 0.01, "{}", l.at(Vec3::ZERO, &[]).x);
        // A point light 2 studs away adds to it, nothing past its range.
        l.lights.push(crate::render::LightRaw {
            surface_u: [0.0; 4], surface_v: [0.0; 4],
            pos_range: [2.0, 0.0, 0.0, 8.0],
            color: [1.0, 0.0, 0.0, 0.0],
            dir: [0.0, 1.0, 0.0, -1.0],
            extra: [0.0; 4],
        });
        let near = l.near(Vec3::ZERO, 1.0);
        assert!(l.at(Vec3::ZERO, &near).x > 0.5);
        let far = Vec3::new(20.0, 0.0, 0.0);
        assert!((l.at(far, &near).x - l.at(far, &[]).x).abs() < 1e-5);
        // (calibration q) the light falls in a straight line to Range + 4,
        // FX_LOCAL_SLOPE per stud per unit of Brightness.
        let one = |d: f32| l.local(&l.lights[0], Vec3::new(2.0 - d, 0.0, 0.0)).x * mech::LIGHT_INTENSITY;
        assert!((one(4.0) - mech::FX_LOCAL_SLOPE * 8.0).abs() < 1e-4);
        assert!((one(8.0) - one(10.0) - mech::FX_LOCAL_SLOPE * 2.0).abs() < 1e-4);
        assert_eq!(one(12.5), 0.0);
        // The ambient and local light are held to luminance 1, the sun added on top.
        let mut white = SceneLight { ambient: Vec3::splat(3.0), sun: Vec3::splat(0.5), ..Default::default() };
        assert!((white.at(Vec3::ZERO, &[]).x - 1.5).abs() < 1e-4);
        white.ambient = Vec3::splat(0.4);
        assert!((white.at(Vec3::ZERO, &[]).x - 0.9).abs() < 1e-4);
    }

    #[test]
    fn light_influence_and_spin_follow_studio() {
        // (calibration o) LightInfluence weighs the scene's light by its
        // square root.
        let look = Look { influence: 0.25, brightness: 1.0, emission: 0.0, alpha: 1.0 };
        assert!((look.light(Vec3::ZERO)[0] - 0.5).abs() < 1e-5);
        assert!((look.light(Vec3::ONE)[0] - 1.0).abs() < 1e-5);
        // (calibration f) a positive RotSpeed turns clockwise: the angle
        // falls while Rotation's sign is counter-clockwise.
        let mut c = emitter_test_cfg();
        c.rot_speed = (90.0, 90.0);
        c.life = (5.0, 5.0);
        let mut st = EmitterState::default();
        let xf = Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY };
        st.step(&c, Ref::new(), &xf, Vec3::ZERO, 0.0, 1, None, Vec3::ZERO);
        for _ in 0..10 {
            st.step(&c, Ref::new(), &xf, Vec3::ZERO, 0.1, 0, None, Vec3::ZERO);
        }
        assert!((st.particles[0].rot + 90.0).abs() < 1e-3);
        // (calibration s) Drag with WindAffectsDrag pulls toward the wind;
        // an anchored host passes on its set velocity, not its motion.
        c.rot_speed = (0.0, 0.0);
        c.drag = 1.0;
        c.wind = true;
        c.inherit = 1.0;
        let mut st = EmitterState::default();
        st.step(&c, Ref::new(), &xf, Vec3::ZERO, 0.0, 1, Some(Vec3::ZERO), Vec3::X * 10.0);
        let moved = Xform { pos: Vec3::X * 4.0, rot: Mat3::IDENTITY };
        st.step(&c, Ref::new(), &moved, Vec3::ZERO, 1.0, 0, Some(Vec3::ZERO), Vec3::X * 10.0);
        assert!((st.particles[0].vel.x - 5.0).abs() < 1e-3, "{:?}", st.particles[0].vel);
    }

    fn emitter_test_cfg() -> EmitterCfg {
        EmitterCfg {
            host: Host::None,
            enabled: true,
            rate: 0.0,
            life: (1.0, 1.0),
            speed: (1.0, 1.0),
            spread: Vec2::ZERO,
            dir: 1,
            shape: 0,
            style: 0,
            inout: 0,
            partial: 1.0,
            accel: Vec3::ZERO,
            drag: 0.0,
            inherit: 0.0,
            locked: false,
            wind: false,
            time_scale: 1.0,
            size: Seq(vec![(0.0, 1.0, 0.0)]),
            transparency: Seq(vec![(0.0, 0.0, 0.0)]),
            color: CSeq(vec![(0.0, Vec3::ONE)]),
            squash: Seq(vec![(0.0, 0.0, 0.0)]),
            rotation: (0.0, 0.0),
            rot_speed: (0.0, 0.0),
            texture: None,
            grid: (1, 1),
            flip_mode: 0,
            flip_fps: (1.0, 1.0),
            flip_random_start: false,
            flip_blend: false,
            orientation: 0,
            zoffset: 0.0,
            look: Look { influence: 0.0, brightness: 1.0, emission: 0.0, alpha: 1.0 },
        }
    }
}

#[cfg(test)]
mod texture_tests {
    use super::*;
    use rbx_dom_weak::types::{Content, ContentId};
    use rbx_dom_weak::InstanceBuilder;

    /// Place A's Vfx.emitter: Instance.new("ParticleEmitter"), then Texture = id.
    /// TextureContent's class default (the sparkle) must not win over it.
    #[test]
    fn emitter_texture_prefers_what_is_stored() {
        let mut d = Document::new_place();
        let ws = d.workspace().unwrap();
        let set = d.dom.insert(ws, InstanceBuilder::new("ParticleEmitter").with_property("Texture", ContentId::from("rbxassetid://123")));
        assert_eq!(texture_of(&d, set).as_deref(), Some("rbxassetid://123"));
        let fresh = d.dom.insert(ws, InstanceBuilder::new("ParticleEmitter"));
        assert_eq!(texture_of(&d, fresh).as_deref(), Some("rbxasset://textures/particles/sparkles_main.dds"));
        let cleared = d.dom.insert(ws, InstanceBuilder::new("ParticleEmitter").with_property("Texture", ContentId::from("")));
        assert_eq!(texture_of(&d, cleared), None);
        let twin = d.dom.insert(ws, InstanceBuilder::new("ParticleEmitter").with_property("TextureContent", Content::from_uri("rbxassetid://9")));
        assert_eq!(texture_of(&d, twin).as_deref(), Some("rbxassetid://9"));
    }
}
