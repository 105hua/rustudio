//! A bounding-volume hierarchy over a mesh's unit-space triangles, so rays
//! and overlap probes against MeshParts test a handful of triangles instead
//! of all of them (round 27: Place A's nav-grid build on the source game terrain meshes spent
//! ~19 s in brute-force Raycast / GetPartsInPart; Studio takes ~1.5 s).
//! Answers are the same as the brute-force tests: the BVH only skips
//! triangles whose bounds the query can't touch.

use glam::Vec3;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const LEAF: usize = 4;

#[derive(Clone, Copy)]
struct Node {
    lo: Vec3,
    hi: Vec3,
    /// Leaf: first triangle index in `order`, count > 0. Inner: left child
    /// index in `first` (right = left + 1), count 0.
    first: u32,
    count: u32,
}

pub struct MeshBvh {
    pub tris: Arc<Vec<[Vec3; 3]>>,
    nodes: Vec<Node>,
    order: Vec<u32>,
}

impl MeshBvh {
    pub fn build(tris: Arc<Vec<[Vec3; 3]>>) -> MeshBvh {
        let n = tris.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        let bounds: Vec<(Vec3, Vec3, Vec3)> = tris
            .iter()
            .map(|t| {
                let lo = t[0].min(t[1]).min(t[2]);
                let hi = t[0].max(t[1]).max(t[2]);
                (lo, hi, (lo + hi) * 0.5)
            })
            .collect();
        let mut nodes = Vec::with_capacity(2 * n / LEAF + 1);
        nodes.push(Node { lo: Vec3::ZERO, hi: Vec3::ZERO, first: 0, count: n as u32 });
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let (first, count) = (nodes[i].first as usize, nodes[i].count as usize);
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            let (mut clo, mut chi) = (lo, hi);
            for &k in &order[first..first + count] {
                let b = bounds[k as usize];
                lo = lo.min(b.0);
                hi = hi.max(b.1);
                clo = clo.min(b.2);
                chi = chi.max(b.2);
            }
            nodes[i].lo = lo;
            nodes[i].hi = hi;
            if count <= LEAF {
                continue;
            }
            let ext = chi - clo;
            let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
            if ext[axis] <= 0.0 {
                continue; // all centres coincide: keep as a (big) leaf
            }
            let mid = count / 2;
            order[first..first + count].select_nth_unstable_by(mid, |a, b| {
                bounds[*a as usize].2[axis].total_cmp(&bounds[*b as usize].2[axis])
            });
            let left = nodes.len();
            nodes.push(Node { lo, hi, first: first as u32, count: mid as u32 });
            nodes.push(Node { lo, hi, first: (first + mid) as u32, count: (count - mid) as u32 });
            nodes[i].first = left as u32;
            nodes[i].count = 0;
            stack.push(left);
            stack.push(left + 1);
        }
        MeshBvh { tris, nodes, order }
    }

    /// Visits every triangle whose node bounds pass `keep(lo, hi)`;
    /// `visit` returns true to stop early.
    fn walk(&self, keep: impl Fn(Vec3, Vec3) -> bool, mut visit: impl FnMut(&[Vec3; 3]) -> bool) {
        if self.tris.is_empty() {
            return;
        }
        let mut stack = [0u32; 64];
        let mut sp = 1usize;
        while sp > 0 {
            sp -= 1;
            let nd = self.nodes[stack[sp] as usize];
            if !keep(nd.lo, nd.hi) {
                continue;
            }
            if nd.count > 0 {
                for &k in &self.order[nd.first as usize..(nd.first + nd.count) as usize] {
                    if visit(&self.tris[k as usize]) {
                        return;
                    }
                }
            } else if sp + 2 <= stack.len() {
                stack[sp] = nd.first;
                stack[sp + 1] = nd.first + 1;
                sp += 2;
            } else {
                // Too deep for the stack (degenerate input): test the subtree
                // by brute force rather than miss it.
                for t in self.tris.iter() {
                    if visit(t) {
                        return;
                    }
                }
                return;
            }
        }
    }

    /// Nearest hit along origin + t·dir (t > 0): (t, normal facing the ray),
    /// as `assets::ray_triangles_normal`.
    pub fn ray_normal(&self, origin: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
        let inv = Vec3::ONE / dir;
        let best = std::cell::Cell::new(f32::INFINITY);
        let mut out: Option<(f32, Vec3)> = None;
        self.walk(
            |lo, hi| match slab(origin, inv, lo, hi) {
                Some((t0, _)) => t0 <= best.get(),
                None => false,
            },
            |t| {
                if let Some((tt, n)) = ray_tri(origin, dir, t) {
                    if tt > 0.0 && tt < best.get() {
                        best.set(tt);
                        out = Some((tt, if n.dot(dir) > 0.0 { -n } else { n }));
                    }
                }
                false
            },
        );
        out
    }

    /// Whether any triangle's bounds meet the box [lo, hi] and `test` says the
    /// triangle really touches the query.
    pub fn any_in_box(&self, lo: Vec3, hi: Vec3, mut test: impl FnMut(&[Vec3; 3]) -> bool) -> bool {
        let mut found = false;
        self.walk(
            |a, b| a.cmple(hi).all() && b.cmpge(lo).all(),
            |t| {
                let tl = t[0].min(t[1]).min(t[2]);
                let th = t[0].max(t[1]).max(t[2]);
                if tl.cmple(hi).all() && th.cmpge(lo).all() && test(t) {
                    found = true;
                    return true;
                }
                false
            },
        );
        found
    }

    /// Crossings of the ray p + t·dir (t > 0) with the triangles (parity test).
    pub fn crossings(&self, p: Vec3, dir: Vec3) -> usize {
        let inv = Vec3::ONE / dir;
        let mut n = 0;
        self.walk(
            |lo, hi| slab(p, inv, lo, hi).is_some(),
            |t| {
                if ray_tri(p, dir, t).is_some_and(|(tt, _)| tt > 0.0) {
                    n += 1;
                }
                false
            },
        );
        n
    }
}

/// The ray's entry / exit parameters through a box (exit ≥ 0), if it meets it.
fn slab(o: Vec3, inv: Vec3, lo: Vec3, hi: Vec3) -> Option<(f32, f32)> {
    let a = (lo - o) * inv;
    let b = (hi - o) * inv;
    let tmin = a.min(b);
    let tmax = a.max(b);
    // NaN (0 · inf on a flat box edge) must not reject: treat as unbounded.
    let t0 = [tmin.x, tmin.y, tmin.z].into_iter().filter(|v| !v.is_nan()).fold(f32::NEG_INFINITY, f32::max);
    let t1 = [tmax.x, tmax.y, tmax.z].into_iter().filter(|v| !v.is_nan()).fold(f32::INFINITY, f32::min);
    (t1 >= t0.max(0.0) - 1e-6).then_some((t0, t1))
}

/// Möller–Trumbore, as in `assets::ray_triangles_normal`: (t, unnormalised
/// face normal → normalised).
fn ray_tri(o: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<(f32, Vec3)> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some((e2.dot(q) * inv, e1.cross(e2).normalize_or_zero()))
}

/// The BVH of a mesh content id (built once, shared).
pub fn for_mesh(id: &str) -> Option<Arc<MeshBvh>> {
    static CACHE: Mutex<Option<HashMap<String, Option<Arc<MeshBvh>>>>> = Mutex::new(None);
    if let Some(hit) = CACHE.lock().ok()?.get_or_insert_with(HashMap::new).get(id) {
        return hit.clone();
    }
    let tris = crate::assets::mesh_triangles(id);
    // A mesh still downloading isn't cached as missing.
    let tris = tris?;
    let bvh = Some(Arc::new(MeshBvh::build(tris)));
    if let Ok(mut g) = CACHE.lock() {
        g.get_or_insert_with(HashMap::new).insert(id.to_string(), bvh.clone());
    }
    bvh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soup(n: usize) -> Vec<[Vec3; 3]> {
        // A pseudo-random triangle soup in [-0.5, 0.5]^3.
        let mut s = 12345u32;
        let mut r = || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        (0..n)
            .map(|_| {
                let c = Vec3::new(r(), r(), r());
                [c, c + Vec3::new(r(), r(), r()) * 0.2, c + Vec3::new(r(), r(), r()) * 0.2]
            })
            .collect()
    }

    #[test]
    fn rays_match_brute_force() {
        let tris = Arc::new(soup(2000));
        let bvh = MeshBvh::build(tris.clone());
        let mut s = 7u32;
        let mut r = || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for _ in 0..500 {
            let o = Vec3::new(r(), r(), r()) * 3.0;
            let d = Vec3::new(r(), r(), r()).normalize_or(Vec3::Y);
            let a = crate::assets::ray_triangles_normal(o, d, &tris).map(|x| x.0);
            let b = bvh.ray_normal(o, d).map(|x| x.0);
            assert_eq!(a.is_some(), b.is_some());
            if let (Some(a), Some(b)) = (a, b) {
                assert!((a - b).abs() < 1e-5);
            }
            // axis-aligned rays (the nav build's downward casts)
            let a = crate::assets::ray_triangles_normal(o, -Vec3::Y, &tris).map(|x| x.0);
            let b = bvh.ray_normal(o, -Vec3::Y).map(|x| x.0);
            assert_eq!(a.is_some(), b.is_some());
            if let (Some(a), Some(b)) = (a, b) {
                assert!((a - b).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn boxes_match_brute_force() {
        use crate::math::Xform;
        let tris = Arc::new(soup(1500));
        let bvh = MeshBvh::build(tris.clone());
        let mesh = Xform { pos: Vec3::new(3.0, -1.0, 2.0), rot: glam::Mat3::from_rotation_y(0.7) };
        let size = Vec3::new(40.0, 10.0, 25.0);
        let mut s = 99u32;
        let mut r = || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let mut hits = 0;
        for _ in 0..800 {
            let b = Xform {
                pos: mesh.pos + Vec3::new(r() * 50.0, r() * 14.0, r() * 30.0),
                rot: glam::Mat3::from_rotation_x(r()) * glam::Mat3::from_rotation_z(r()),
            };
            let bs = Vec3::new(r().abs() * 3.0 + 0.1, r().abs() * 6.0 + 0.1, r().abs() * 3.0 + 0.1);
            let a = crate::query_shapes::obb_mesh(&b, bs, &mesh, size, &tris);
            let c = crate::query_shapes::obb_mesh_bvh(&b, bs, &mesh, size, &bvh);
            assert_eq!(a, c);
            hits += a as u32;
        }
        assert!(hits > 20);
    }
}

/// Where a union's geometry sits in its part, as a fraction of Size: the
/// centre of the mesh's bounds over their extent. Studio-made unions (and
/// BasePart:UnionAsync results) are centred (zero); GeometryService keeps
/// the main part's CFrame, so its results' geometry sits off-centre and
/// moves with Size (measured: Size doubled → the offset doubles). Drawing,
/// colliders and world queries fit the mesh to the Size box, then shift it
/// by this × Size. Only union meshes (`rbxtemp://rustudio/union/…`) and
/// GeometryService's MeshParts (`rbxtemp://rustudio/geometry/…`: fragments
/// keep the source part's CFrame the same way).
pub fn union_offset(id: &str) -> Vec3 {
    if !id.starts_with("rbxtemp://rustudio/union/") && !id.starts_with("rbxtemp://rustudio/geometry/") {
        return Vec3::ZERO;
    }
    static CACHE: Mutex<Option<HashMap<String, Vec3>>> = Mutex::new(None);
    if let Some(v) = CACHE.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(id).copied())) {
        return v;
    }
    let Some(bytes) = crate::assets::generated(id) else { return Vec3::ZERO };
    // Text (an OBJ in MeshData: test fixtures, imports) is fitted as before.
    let text = bytes.iter().take(64).all(|b| b.is_ascii_graphic() || b.is_ascii_whitespace());
    let v = match crate::mesh::parse(&bytes).ok().filter(|_| !text) {
        Some(m) => {
            let ext = m.max - m.min;
            let c = (m.max + m.min) * 0.5;
            let f = |c: f32, e: f32| if e > 1e-6 && (c / e).abs() > 1e-5 { c / e } else { 0.0 };
            Vec3::new(f(c.x, ext.x), f(c.y, ext.y), f(c.z, ext.z))
        }
        None => Vec3::ZERO,
    };
    if let Ok(mut g) = CACHE.lock() {
        g.get_or_insert_with(HashMap::new).insert(id.to_string(), v);
    }
    v
}
