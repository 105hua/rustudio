//! MeshPart collision geometry, by Enum.CollisionFidelity:
//!
//! - Box (2): the part's box (physics.rs builds it, no mesh needed).
//! - Hull (1): one convex hull of the mesh's vertices.
//! - Default (0) and PreciseConvexDecomposition (3): an approximate convex
//!   decomposition of the mesh (parry's V-HACD). Studio decomposes with its
//!   own HACD, tuned by fast-ints read from Studio:
//!   HACDConcavityMillionth 10 000 000 against HACDNormalizationDistanceMillionth
//!   1 000 000 000, i.e. pieces may be at most 1% of the mesh's size concave;
//!   rustudio decomposes at that concavity (docs/MECHANICS.md, "Collision").
//!
//! Hulls are kept in the mesh's unit box (−0.5..0.5, the same space as
//! assets::mesh_triangles), so one decomposition serves every part using the
//! mesh: each part stretches the hull points to its Size (a stretched convex
//! set stays convex). Decompositions run on background threads; results are
//! kept in memory and in ~/.cache/rustudio/collision (asset ids never change,
//! so a second run doesn't decompose again).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use glam::Vec3;
use rapier3d::parry::transformation::vhacd::{VHACDParameters, VHACD};
use rapier3d::prelude as rp;

/// Enum.CollisionFidelity values (Roblox docs).
pub const FIDELITY_DEFAULT: u32 = 0;
pub const FIDELITY_HULL: u32 = 1;
pub const FIDELITY_BOX: u32 = 2;
pub const FIDELITY_PRECISE: u32 = 3;

/// Convex pieces in the mesh's unit box.
pub type Hulls = Arc<Vec<Vec<Vec3>>>;

#[derive(Clone)]
enum State {
    Pending,
    Ready(Hulls),
    Failed,
}

struct Store {
    states: Mutex<HashMap<(String, u32), State>>,
    queue: Mutex<VecDeque<(String, u32)>>,
    wake: Condvar,
}

/// Bump when the decomposition settings change (old cache files are ignored).
const CACHE_VERSION: &str = "hacd-c0.01-v1";

fn store() -> &'static Store {
    static S: OnceLock<Store> = OnceLock::new();
    S.get_or_init(|| {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .saturating_sub(2)
            .clamp(1, 8);
        for _ in 0..workers {
            std::thread::Builder::new()
                .name("collision".into())
                .spawn(|| loop {
                    let job = {
                        let s = store();
                        let mut q = s.queue.lock().expect("collision queue");
                        loop {
                            if let Some(j) = q.pop_front() {
                                break j;
                            }
                            q = s.wake.wait(q).expect("collision queue");
                        }
                    };
                    let result = compute(&job.0, job.1);
                    let st = match result {
                        Some(h) => State::Ready(Arc::new(h)),
                        None => State::Failed,
                    };
                    store()
                        .states
                        .lock()
                        .expect("collision states")
                        .insert(job, st);
                })
                .ok();
        }
        Store {
            states: Mutex::new(HashMap::new()),
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
        }
    })
}

/// V-HACD settings for a fidelity: concavity 0.01 from Studio's HACD flags;
/// the voxel grid is V-HACD's own default 64³ for Precise and half that for
/// Default, which Roblox documents as the coarser of the two.
pub fn params(fidelity: u32) -> VHACDParameters {
    VHACDParameters {
        concavity: 0.01,
        resolution: if fidelity == FIDELITY_PRECISE { 64 } else { 32 },
        ..Default::default()
    }
}

fn cache_file(id: &str, fidelity: u32) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (CACHE_VERSION, id, fidelity).hash(&mut h);
    crate::assets::cache_dir()
        .join("collision")
        .join(format!("{:016x}.bin", h.finish()))
}

fn load_cached(id: &str, fidelity: u32) -> Option<Vec<Vec<Vec3>>> {
    let b = std::fs::read(cache_file(id, fidelity)).ok()?;
    let mut at = 0usize;
    let mut u32_at = |b: &[u8]| -> Option<u32> {
        let v = u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?);
        at += 4;
        Some(v)
    };
    let n = u32_at(&b)? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let m = u32_at(&b)? as usize;
        let mut pts = Vec::with_capacity(m);
        for _ in 0..m {
            let x = f32::from_bits(u32_at(&b)?);
            let y = f32::from_bits(u32_at(&b)?);
            let z = f32::from_bits(u32_at(&b)?);
            pts.push(Vec3::new(x, y, z));
        }
        out.push(pts);
    }
    Some(out)
}

fn save_cached(id: &str, fidelity: u32, hulls: &[Vec<Vec3>]) {
    let mut b = Vec::new();
    b.extend_from_slice(&(hulls.len() as u32).to_le_bytes());
    for h in hulls {
        b.extend_from_slice(&(h.len() as u32).to_le_bytes());
        for p in h {
            for c in [p.x, p.y, p.z] {
                b.extend_from_slice(&c.to_bits().to_le_bytes());
            }
        }
    }
    let path = cache_file(id, fidelity);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, b);
}

/// The mesh's vertices (welded) and triangles in its unit box.
fn mesh_geometry(id: &str) -> Option<(Vec<rp::Vector>, Vec<[u32; 3]>)> {
    let tris = crate::assets::mesh_triangles(id)?;
    let mut index: HashMap<[u32; 3], u32> = HashMap::new();
    let mut verts = Vec::new();
    let mut faces = Vec::with_capacity(tris.len());
    for t in tris.iter() {
        let mut f = [0u32; 3];
        for (k, p) in t.iter().enumerate() {
            let key = [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
            f[k] = *index.entry(key).or_insert_with(|| {
                verts.push(rp::Vector::new(p.x, p.y, p.z));
                (verts.len() - 1) as u32
            });
        }
        if f[0] != f[1] && f[1] != f[2] && f[0] != f[2] {
            faces.push(f);
        }
    }
    (!faces.is_empty()).then_some((verts, faces))
}

/// How far (unit box) a mesh's surface may dip under its convex hull and
/// still be one convex piece: Studio's HACD concavity, 1% of the size.
const CONVEX_TOLERANCE: f32 = 0.01;

/// Whether the mesh's surface lies on its own convex hull (to within
/// `CONVEX_TOLERANCE`): its vertices, edge midpoints and face centres. HACD
/// measures concavity on the surface, so such a mesh is one piece in Studio
/// even when it isn't closed. Converted brushes often aren't (the source game's brush
/// hulls drop faces: Map 3's spawn-stair ramp has no lower slope), and
/// V-HACD's flood fill leaks through the hole and leaves most of the solid
/// out (the ramp's lower half was hollow: players walked into its middle).
fn is_convex(verts: &[rp::Vector], faces: &[[u32; 3]]) -> bool {
    use rapier3d::parry::transformation::try_convex_hull;
    // (flat or degenerate meshes fail here and go to V-HACD as before)
    let Ok(Ok((hp, hf))) = crate::crash::recoverable(|| try_convex_hull(verts)) else {
        return false;
    };
    if hf.len() < 4 {
        return false;
    }
    let centre = hp.iter().fold(rp::Vector::ZERO, |a, p| a + *p) / hp.len() as f32;
    let planes: Vec<(rp::Vector, rp::Vector)> = hf
        .iter()
        .filter_map(|f| {
            let [a, b, c] = f.map(|i| hp[i as usize]);
            let n = (b - a).cross(c - a);
            let len = n.length();
            (len > 1e-9).then(|| {
                let n = n / len;
                // Outward, away from the hull's middle.
                (a, if n.dot(a - centre) < 0.0 { -n } else { n })
            })
        })
        .collect();
    if planes.len() < 4 {
        return false;
    }
    // How deep a point lies under the hull's surface.
    let depth = |p: rp::Vector| planes.iter().map(|(a, n)| n.dot(*a - p)).fold(f32::INFINITY, f32::min);
    faces.iter().all(|f| {
        let [a, b, c] = f.map(|i| verts[i as usize]);
        [a, b, c, (a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5, (a + b + c) / 3.0]
            .into_iter()
            .all(|p| depth(p) <= CONVEX_TOLERANCE)
    })
}

/// Computes the hulls for a mesh (no caching; runs on a worker).
fn compute(id: &str, fidelity: u32) -> Option<Vec<Vec<Vec3>>> {
    // A convex mesh is its own hull at every fidelity (checked before the
    // disk cache: older cache files hold V-HACD's pieces of these).
    let geometry = (fidelity != FIDELITY_HULL).then(|| mesh_geometry(id)).flatten();
    if let Some((verts, faces)) = &geometry {
        if is_convex(verts, faces) {
            return Some(vec![verts.iter().map(|v| Vec3::new(v.x, v.y, v.z)).collect()]);
        }
    }
    if let Some(h) = load_cached(id, fidelity) {
        return Some(h);
    }
    let (verts, faces) = geometry.or_else(|| mesh_geometry(id))?;
    let hulls: Vec<Vec<Vec3>> = if fidelity == FIDELITY_HULL {
        vec![verts.iter().map(|v| Vec3::new(v.x, v.y, v.z)).collect()]
    } else {
        // parry's VHACD can panic on some meshes (an unwrap in its convex
        // hull: "exiting an unfinished work", seen on a live Place A session Map 1
        // mesh). That killed the collision worker, so every later mesh had
        // no collider and the character fell through the map. One bad mesh
        // now falls back to its single convex hull.
        let decomposed = crate::crash::recoverable(|| {
            let decomp = VHACD::decompose(&params(fidelity), &verts, &faces, true);
            decomp
                .compute_exact_convex_hulls(&verts, &faces)
                .into_iter()
                .map(|(pts, _)| {
                    pts.iter()
                        .map(|v| Vec3::new(v.x, v.y, v.z))
                        .collect::<Vec<_>>()
                })
                .filter(|p| p.len() >= 4)
                .collect::<Vec<_>>()
        });
        match decomposed {
            Ok(h) => h,
            Err(_) => {
                eprintln!("collision: decomposition of {id} failed; using its convex hull");
                vec![verts.iter().map(|v| Vec3::new(v.x, v.y, v.z)).collect()]
            }
        }
    };
    if hulls.is_empty() {
        return None;
    }
    save_cached(id, fidelity, &hulls);
    Some(hulls)
}

/// What a part's collider can use right now.
pub enum Lookup {
    Ready(Hulls),
    /// Being computed: ask again later.
    Pending,
    /// The mesh isn't available here (or can't be decomposed).
    Unavailable,
}

/// The convex pieces for a mesh at a fidelity (Hull, Default or Precise),
/// starting the work in the background the first time.
pub fn hulls(id: &str, fidelity: u32) -> Lookup {
    let s = store();
    let key = (id.to_string(), fidelity);
    {
        let mut states = s.states.lock().expect("collision states");
        match states.get(&key) {
            Some(State::Ready(h)) => return Lookup::Ready(h.clone()),
            Some(State::Pending) => return Lookup::Pending,
            Some(State::Failed) => return Lookup::Unavailable,
            None => {}
        }
        // Hull is cheap: do it here; decompositions go to the workers.
        if fidelity == FIDELITY_HULL {
            let st = match compute(id, fidelity) {
                Some(h) => State::Ready(Arc::new(h)),
                None => State::Failed,
            };
            states.insert(key, st.clone());
            return match st {
                State::Ready(h) => Lookup::Ready(h),
                _ => Lookup::Unavailable,
            };
        }
        if crate::assets::mesh_triangles(id).is_none() {
            states.insert(key, State::Failed);
            return Lookup::Unavailable;
        }
        states.insert(key.clone(), State::Pending);
    }
    s.queue.lock().expect("collision queue").push_back(key);
    s.wake.notify_one();
    Lookup::Pending
}

/// Waits (up to `timeout`) for a pending decomposition: for tests.
#[allow(dead_code)]
pub fn wait_ready(id: &str, fidelity: u32, timeout: std::time::Duration) -> Lookup {
    let start = std::time::Instant::now();
    loop {
        match hulls(id, fidelity) {
            Lookup::Pending if start.elapsed() < timeout => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            other => return other,
        }
    }
}

/// A collider shape from unit-box hulls stretched to a part's size.
pub fn shape_from_hulls(hulls: &[Vec<Vec3>], size: Vec3) -> Option<rp::SharedShape> {
    let mut parts = Vec::new();
    for h in hulls {
        let pts: Vec<rp::Vector> = h
            .iter()
            .map(|p| rp::Vector::new(p.x * size.x, p.y * size.y, p.z * size.z))
            .collect();
        if let Some(s) = rp::SharedShape::convex_hull(&pts) {
            parts.push((rp::Pose::IDENTITY, s));
        }
    }
    match parts.len() {
        0 => None,
        1 => parts.pop().map(|(_, s)| s),
        _ => Some(rp::SharedShape::compound(parts)),
    }
}

/// A convex hull reduced to its extreme points along 50 fixed directions
/// (the 26 of a cube's faces, edges and corners plus 24 between them): the
/// touch shape of a moving non-colliding mesh, which is re-bounded every
/// step. Hulls with few points are returned as they are.
pub fn touch_hull(s: &rp::SharedShape) -> rp::SharedShape {
    let Some(c) = s.as_convex_polyhedron() else { return s.clone() };
    let pts = c.points();
    if pts.len() <= 50 {
        return s.clone();
    }
    let mut dirs: Vec<rp::Vector> = Vec::new();
    for x in -1i32..=1 {
        for y in -1i32..=1 {
            for z in -1i32..=1 {
                if (x, y, z) != (0, 0, 0) {
                    dirs.push(rp::Vector::new(x as f32, y as f32, z as f32));
                }
            }
        }
    }
    for (a, b) in [(1.0f32, 0.5f32), (0.5, 1.0)] {
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                dirs.push(rp::Vector::new(sx * a, sy * b, 0.0));
                dirs.push(rp::Vector::new(0.0, sx * a, sy * b));
                dirs.push(rp::Vector::new(sy * b, 0.0, sx * a));
            }
        }
    }
    let mut keep: Vec<rp::Vector> = Vec::new();
    for d in dirs {
        if let Some(p) = pts.iter().copied().max_by(|a, b| a.dot(d).total_cmp(&b.dot(d))) {
            if !keep.iter().any(|k| (*k - p).length_squared() < 1e-8) {
                keep.push(p);
            }
        }
    }
    rp::SharedShape::convex_hull(&keep).unwrap_or_else(|| s.clone())
}

/// The mesh's own triangles stretched to a part's size (an anchored part's
/// shape while its decomposition is being computed).
pub fn trimesh(id: &str, size: Vec3) -> Option<rp::SharedShape> {
    let (verts, faces) = mesh_geometry(id)?;
    let verts: Vec<rp::Vector> = verts
        .iter()
        .map(|v| rp::Vector::new(v.x * size.x, v.y * size.y, v.z * size.z))
        .collect();
    rp::SharedShape::trimesh(verts, faces).ok()
}

/// An L-shaped prism as OBJ text (our own geometry, for the tests): the
/// L fills x, y in 0..2 except the 1..2 × 1..2 corner, z in 0..1. In the
/// unit box that empty corner is x, y in 0..0.5.
pub fn l_prism_obj() -> String {
    let l = [
        [0.0f32, 0.0],
        [2.0, 0.0],
        [2.0, 1.0],
        [1.0, 1.0],
        [1.0, 2.0],
        [0.0, 2.0],
    ];
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut tri = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| pos.extend([a, b, c]);
    // Caps: the L as two rectangles, wound outward.
    for (z, flip) in [(0.0f32, true), (1.0, false)] {
        for quad in [[0usize, 1, 2, 3], [0, 3, 4, 5]] {
            let p = |i: usize| [l[quad[i]][0], l[quad[i]][1], z];
            if flip {
                tri(p(0), p(2), p(1));
                tri(p(0), p(3), p(2));
            } else {
                tri(p(0), p(1), p(2));
                tri(p(0), p(2), p(3));
            }
        }
    }
    for i in 0..6 {
        let (a, b) = (l[i], l[(i + 1) % 6]);
        tri([a[0], a[1], 0.0], [b[0], b[1], 0.0], [b[0], b[1], 1.0]);
        tri([a[0], a[1], 0.0], [b[0], b[1], 1.0], [a[0], a[1], 1.0]);
    }
    let mut obj = String::new();
    for p in &pos {
        obj += &format!("v {} {} {}\n", p[0], p[1], p[2]);
    }
    for i in 0..pos.len() / 3 {
        obj += &format!("f {} {} {}\n", i * 3 + 1, i * 3 + 2, i * 3 + 3);
    }
    obj
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A moving sensor's touch hull: at most 50 points, the same extent
    /// along the axes, and inside the full hull.
    #[test]
    fn touch_hull_keeps_the_extremes_with_few_points() {
        let mut pts = Vec::new();
        let mut s: u32 = 12345;
        for _ in 0..400 {
            let mut r = || { s ^= s << 13; s ^= s >> 17; s ^= s << 5; (s % 10000) as f32 / 10000.0 - 0.5 };
            let v = rp::Vector::new(r(), r(), r());
            pts.push(v.normalize() * 5.5);
        }
        let full = rp::SharedShape::convex_hull(&pts).unwrap();
        let n_full = full.as_convex_polyhedron().unwrap().points().len();
        assert!(n_full > 50, "{n_full}");
        let t = touch_hull(&full);
        let c = t.as_convex_polyhedron().unwrap();
        assert!(c.points().len() <= 50, "{}", c.points().len());
        let ext = |pts: &[rp::Vector], d: rp::Vector| pts.iter().map(|p| p.dot(d)).fold(f32::MIN, f32::max);
        for d in [rp::Vector::X, rp::Vector::NEG_X, rp::Vector::Y, rp::Vector::NEG_Y, rp::Vector::Z, rp::Vector::NEG_Z] {
            assert!((ext(c.points(), d) - ext(&pts, d)).abs() < 1e-4, "extent along {d:?}");
        }
        // A sphere-ish hull loses at most a little volume.
        let v = |s: &rp::SharedShape| s.mass_properties(1.0).mass();
        assert!(v(&t) > 0.8 * v(&full) && v(&t) <= v(&full) + 1e-3, "{} vs {}", v(&t), v(&full));
    }

    #[test]
    fn hull_and_decomposition_of_an_l_shape() {
        let id = "rbxtemp://rustudio/test/l-shape.obj";
        crate::assets::register_generated(id, l_prism_obj().into_bytes());
        std::env::set_var(
            "RUSTUDIO_CACHE_DIR",
            std::env::temp_dir().join("rustudio-collision-test"),
        );
        let Lookup::Ready(hull) = hulls(id, FIDELITY_HULL) else {
            panic!("hull")
        };
        assert_eq!(hull.len(), 1);
        let Lookup::Ready(dec) =
            wait_ready(id, FIDELITY_PRECISE, std::time::Duration::from_secs(60))
        else {
            panic!("decomposition")
        };
        assert!(
            dec.len() >= 2,
            "an L needs at least two convex pieces, got {}",
            dec.len()
        );
        // The notch (x, y in the empty quarter) is outside every piece.
        let shape = shape_from_hulls(&dec, Vec3::splat(2.0)).unwrap();
        let notch = rp::Vector::new(0.3, 0.3, 0.0); // unit box ×2: in the empty corner, inside the hull
        let inside = shape.contains_local_point(notch);
        assert!(!inside, "decomposition fills the notch");
        let hull_shape = shape_from_hulls(&hull, Vec3::splat(2.0)).unwrap();
        assert!(
            hull_shape.contains_local_point(notch),
            "a hull does fill it"
        );
        // The solid bar under the corner is inside a piece.
        let bar = rp::Vector::new(0.5, -0.5, 0.0);
        assert!(
            shape.contains_local_point(bar),
            "decomposition misses the bar"
        );
    }

    /// Map 3's spawn-stair ramp clip (the source game brush hull H2019 in the live place's
    /// fbx pack_025): a wedge missing its lower slope face. Studio collides
    /// with the whole wedge; V-HACD's flood fill leaked through the hole
    /// and left the lower half hollow.
    #[test]
    fn open_convex_mesh_is_one_solid_piece() {
        let v: [[f32; 3]; 12] = [
            [0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, -0.437, -0.5], [0.5, -0.058, -0.112],
            [0.5, 0.5, 0.471], [0.5, 0.5, 0.5], [-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5],
            [-0.5, -0.437, -0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, 0.471], [-0.5, -0.058, -0.112],
        ];
        let f: [[usize; 3]; 18] = [
            [0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5], [6, 1, 0], [6, 0, 7], [2, 1, 6], [2, 6, 8], [7, 0, 5],
            [7, 5, 9], [10, 9, 5], [10, 5, 4], [11, 10, 4], [11, 4, 3], [9, 10, 11], [9, 11, 8], [9, 8, 6], [9, 6, 7],
        ];
        let mut obj = String::new();
        for p in v {
            obj += &format!("v {} {} {}\n", p[0], p[1], p[2]);
        }
        for t in f {
            obj += &format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1);
        }
        let id = "rbxtemp://rustudio/test/open-ramp.obj";
        crate::assets::register_generated(id, obj.into_bytes());
        std::env::set_var(
            "RUSTUDIO_CACHE_DIR",
            std::env::temp_dir().join("rustudio-collision-test"),
        );
        let Lookup::Ready(dec) = wait_ready(id, FIDELITY_DEFAULT, std::time::Duration::from_secs(60)) else {
            panic!("decomposition")
        };
        assert_eq!(dec.len(), 1, "a convex brush is one piece");
        // Sized as in the map: 9.6 × 7.68 × 11.04. Inside the ramp, just
        // under the missing slope (z near the low end) and deep inside.
        let size = Vec3::new(9.6, 7.68, 11.04);
        let shape = shape_from_hulls(&dec, size).unwrap();
        for p in [[0.0f32, -0.3, -0.3], [0.3, -0.45, -0.45], [0.0, 0.0, 0.2]] {
            let q = rp::Vector::new(p[0] * size.x, p[1] * size.y, p[2] * size.z);
            assert!(shape.contains_local_point(q), "{p:?} is inside the ramp");
        }
        // Above the slope stays out.
        let above = rp::Vector::new(0.0, 0.2 * size.y, -0.3 * size.z);
        assert!(!shape.contains_local_point(above));
    }
}
