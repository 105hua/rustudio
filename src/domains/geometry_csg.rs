//! Solid geometry: BSP-tree CSG over the parts' polygons (union, subtract,
//! intersect), the parts' polygon soups (blocks, wedges, corner wedges,
//! balls, cylinders, MeshParts and earlier unions), splitting results into
//! connected pieces, and the union mesh blob (CSGMDL version 5, the
//! PartOperation.MeshData2 format mesh.rs reads) that makes a result draw,
//! collide (by its CollisionFidelity) and ray-test through the ordinary
//! union path.

use glam::{DVec3, Vec3};

use crate::math::Xform;

const EPS: f64 = 1e-5;

#[derive(Clone, Copy, Debug)]
pub struct Plane {
    n: DVec3,
    w: f64,
}

impl Plane {
    fn from_points(a: DVec3, b: DVec3, c: DVec3) -> Option<Plane> {
        let n = (b - a).cross(c - a);
        let l = n.length();
        if l < 1e-12 {
            return None;
        }
        let n = n / l;
        Some(Plane { n, w: n.dot(a) })
    }
    fn flip(&mut self) {
        self.n = -self.n;
        self.w = -self.w;
    }
}

#[derive(Clone, Debug)]
pub struct Poly {
    pub v: Vec<DVec3>,
    plane: Plane,
    /// The source part's colour (sRGB 0..1).
    pub color: [f32; 3],
}

impl Poly {
    pub fn new(v: Vec<DVec3>, color: [f32; 3]) -> Option<Poly> {
        if v.len() < 3 {
            return None;
        }
        // Newell's normal: robust for any planar polygon.
        let mut n = DVec3::ZERO;
        for i in 0..v.len() {
            let a = v[i];
            let b = v[(i + 1) % v.len()];
            n += DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
        let l = n.length();
        if l < 1e-12 {
            return None;
        }
        let n = n / l;
        let plane = Plane { n, w: n.dot(v[0]) };
        let _ = Plane::from_points;
        Some(Poly { v, plane, color })
    }
    fn flip(&mut self) {
        self.v.reverse();
        self.plane.flip();
    }
}

const COPLANAR: u8 = 0;
const FRONT: u8 = 1;
const BACK: u8 = 2;
const SPANNING: u8 = 3;

fn split(
    plane: &Plane,
    p: &Poly,
    cf: &mut Vec<Poly>,
    cb: &mut Vec<Poly>,
    front: &mut Vec<Poly>,
    back: &mut Vec<Poly>,
) {
    let mut kind = 0u8;
    let types: Vec<u8> = p
        .v
        .iter()
        .map(|v| {
            let t = plane.n.dot(*v) - plane.w;
            let k = if t < -EPS {
                BACK
            } else if t > EPS {
                FRONT
            } else {
                COPLANAR
            };
            kind |= k;
            k
        })
        .collect();
    match kind {
        COPLANAR => {
            if plane.n.dot(p.plane.n) > 0.0 {
                cf.push(p.clone())
            } else {
                cb.push(p.clone())
            }
        }
        FRONT => front.push(p.clone()),
        BACK => back.push(p.clone()),
        _ => {
            let mut f = Vec::new();
            let mut b = Vec::new();
            let n = p.v.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (ti, tj) = (types[i], types[j]);
                let (vi, vj) = (p.v[i], p.v[j]);
                if ti != BACK {
                    f.push(vi);
                }
                if ti != FRONT {
                    b.push(vi);
                }
                if (ti | tj) == SPANNING {
                    let t = (plane.w - plane.n.dot(vi)) / plane.n.dot(vj - vi);
                    let v = vi.lerp(vj, t);
                    f.push(v);
                    b.push(v);
                }
            }
            if f.len() >= 3 {
                front.push(Poly { v: f, plane: p.plane, color: p.color });
            }
            if b.len() >= 3 {
                back.push(Poly { v: b, plane: p.plane, color: p.color });
            }
        }
    }
}

#[derive(Default)]
struct Node {
    plane: Option<Plane>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
    polys: Vec<Poly>,
}

impl Node {
    fn new(polys: Vec<Poly>) -> Node {
        let mut n = Node::default();
        n.build(polys);
        n
    }

    fn invert(&mut self) {
        for p in &mut self.polys {
            p.flip();
        }
        if let Some(pl) = &mut self.plane {
            pl.flip();
        }
        if let Some(f) = &mut self.front {
            f.invert();
        }
        if let Some(b) = &mut self.back {
            b.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    fn clip_polygons(&self, polys: Vec<Poly>) -> Vec<Poly> {
        let Some(plane) = &self.plane else { return polys };
        let mut front = Vec::new();
        let mut back = Vec::new();
        for p in &polys {
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            split(plane, p, &mut cf, &mut cb, &mut front, &mut back);
            front.extend(cf);
            back.extend(cb);
        }
        let front = match &self.front {
            Some(f) => f.clip_polygons(front),
            None => front,
        };
        let back = match &self.back {
            Some(b) => b.clip_polygons(back),
            None => Vec::new(),
        };
        let mut out = front;
        out.extend(back);
        out
    }

    fn clip_to(&mut self, other: &Node) {
        self.polys = other.clip_polygons(std::mem::take(&mut self.polys));
        if let Some(f) = &mut self.front {
            f.clip_to(other);
        }
        if let Some(b) = &mut self.back {
            b.clip_to(other);
        }
    }

    fn all(&self, out: &mut Vec<Poly>) {
        out.extend(self.polys.iter().cloned());
        if let Some(f) = &self.front {
            f.all(out);
        }
        if let Some(b) = &self.back {
            b.all(out);
        }
    }

    fn build(&mut self, polys: Vec<Poly>) {
        if polys.is_empty() {
            return;
        }
        let plane = *self.plane.get_or_insert(polys[0].plane);
        let mut front = Vec::new();
        let mut back = Vec::new();
        for p in &polys {
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            split(&plane, p, &mut cf, &mut cb, &mut front, &mut back);
            self.polys.extend(cf);
            self.polys.extend(cb);
        }
        if !front.is_empty() {
            self.front.get_or_insert_with(Default::default).build(front);
        }
        if !back.is_empty() {
            self.back.get_or_insert_with(Default::default).build(back);
        }
    }
}

fn polys_of(n: &Node) -> Vec<Poly> {
    let mut v = Vec::new();
    n.all(&mut v);
    v
}

pub fn union(a: Vec<Poly>, b: Vec<Poly>) -> Vec<Poly> {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    let mut a = Node::new(a);
    let mut b = Node::new(b);
    a.clip_to(&b);
    b.clip_to(&a);
    b.invert();
    b.clip_to(&a);
    b.invert();
    a.build(polys_of(&b));
    polys_of(&a)
}

pub fn subtract(a: Vec<Poly>, b: Vec<Poly>) -> Vec<Poly> {
    if a.is_empty() || b.is_empty() {
        return a;
    }
    let mut a = Node::new(a);
    let mut b = Node::new(b);
    a.invert();
    a.clip_to(&b);
    b.clip_to(&a);
    b.invert();
    b.clip_to(&a);
    b.invert();
    a.build(polys_of(&b));
    a.invert();
    polys_of(&a)
}

pub fn intersect(a: Vec<Poly>, b: Vec<Poly>) -> Vec<Poly> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut a = Node::new(a);
    let mut b = Node::new(b);
    a.invert();
    b.clip_to(&a);
    b.invert();
    a.clip_to(&b);
    b.clip_to(&a);
    a.build(polys_of(&b));
    a.invert();
    polys_of(&a)
}

// ------------------------------------------------------------ primitives

/// A part's shape for CSG.
#[derive(Clone, Debug)]
pub enum Shape {
    Block,
    Wedge,
    CornerWedge,
    Ball,
    Cylinder,
    /// Triangles in the unit box (-0.5..0.5 per axis), stretched to Size,
    /// then shifted by the offset × Size (an off-centre union).
    Mesh(std::sync::Arc<Vec<[Vec3; 3]>>, Vec3),
}

fn faces_from_hull(pts: &[DVec3], faces: &[&[usize]], color: [f32; 3]) -> Vec<Poly> {
    let c = pts.iter().copied().sum::<DVec3>() / pts.len() as f64;
    faces
        .iter()
        .filter_map(|f| {
            let mut v: Vec<DVec3> = f.iter().map(|&i| pts[i]).collect();
            let p = Poly::new(v.clone(), color)?;
            if p.plane.n.dot(v[0] - c) < 0.0 {
                v.reverse();
            }
            Poly::new(v, color)
        })
        .collect()
}

/// The part's polygons in world space.
pub fn part_polys(shape: &Shape, xf: &Xform, size: Vec3, color: [f32; 3]) -> Vec<Poly> {
    let h = (size * 0.5).as_dvec3();
    let rot = xf.rot.as_dmat3();
    let pos = xf.pos.as_dvec3();
    let w = |p: DVec3| pos + rot * p;
    match shape {
        Shape::Block | Shape::Wedge | Shape::CornerWedge => {
            let s = |x: f64, y: f64, z: f64| w(DVec3::new(x * h.x, y * h.y, z * h.z));
            match shape {
                Shape::Block => {
                    let p: Vec<DVec3> = (0..8)
                        .map(|i| s(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 }))
                        .collect();
                    faces_from_hull(&p, &[&[0, 2, 6, 4], &[1, 5, 7, 3], &[0, 4, 5, 1], &[2, 3, 7, 6], &[0, 1, 3, 2], &[4, 6, 7, 5]], color)
                }
                Shape::Wedge => {
                    // Full height at the back (+Z), as physics.rs.
                    let p = [
                        s(-1.0, -1.0, -1.0),
                        s(1.0, -1.0, -1.0),
                        s(1.0, -1.0, 1.0),
                        s(-1.0, -1.0, 1.0),
                        s(-1.0, 1.0, 1.0),
                        s(1.0, 1.0, 1.0),
                    ];
                    faces_from_hull(&p, &[&[0, 1, 2, 3], &[3, 2, 5, 4], &[0, 4, 5, 1], &[0, 3, 4], &[1, 5, 2]], color)
                }
                _ => {
                    let p = [s(-1.0, -1.0, -1.0), s(1.0, -1.0, -1.0), s(1.0, -1.0, 1.0), s(-1.0, -1.0, 1.0), s(1.0, 1.0, -1.0)];
                    faces_from_hull(&p, &[&[0, 1, 2, 3], &[1, 2, 4], &[0, 1, 4], &[3, 0, 4], &[2, 3, 4]], color)
                }
            }
        }
        Shape::Ball => {
            let r = h.min_element();
            let (segs, rings) = (16usize, 8usize);
            let pt = |i: usize, j: usize| {
                let th = std::f64::consts::PI * j as f64 / rings as f64;
                let ph = std::f64::consts::TAU * i as f64 / segs as f64;
                w(DVec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()) * r)
            };
            let mut out = Vec::new();
            for j in 0..rings {
                for i in 0..segs {
                    let mut v = vec![pt(i, j)];
                    if j > 0 {
                        v.push(pt(i + 1, j));
                    }
                    v.push(pt(i + 1, j + 1));
                    if j < rings - 1 {
                        v.push(pt(i, j + 1));
                    }
                    if let Some(mut p) = Poly::new(v, color) {
                        if p.plane.n.dot(p.v[0] - pos) < 0.0 {
                            p.flip();
                        }
                        out.push(p);
                    }
                }
            }
            out
        }
        Shape::Cylinder => {
            // Along local X, radius the smaller of Y / Z.
            let r = h.y.min(h.z);
            let segs = 20usize;
            let ring = |x: f64| -> Vec<DVec3> {
                (0..segs)
                    .map(|i| {
                        let a = std::f64::consts::TAU * i as f64 / segs as f64;
                        w(DVec3::new(x, a.cos() * r, a.sin() * r))
                    })
                    .collect()
            };
            let (a, b) = (ring(-h.x), ring(h.x));
            let mut out = Vec::new();
            for i in 0..segs {
                let j = (i + 1) % segs;
                if let Some(mut p) = Poly::new(vec![a[i], a[j], b[j], b[i]], color) {
                    let mid = (a[i] + b[j]) * 0.5;
                    let axis_pt = pos + rot * DVec3::new(rot.transpose().mul_vec3(mid - pos).x, 0.0, 0.0);
                    if p.plane.n.dot(mid - axis_pt) < 0.0 {
                        p.flip();
                    }
                    out.push(p);
                }
            }
            for (cap, x) in [(a, -1.0), (b, 1.0)] {
                if let Some(mut p) = Poly::new(cap, color) {
                    if p.plane.n.dot(rot * DVec3::X) * x < 0.0 {
                        p.flip();
                    }
                    out.push(p);
                }
            }
            out
        }
        Shape::Mesh(tris, off) => tris
            .iter()
            .filter_map(|t| {
                let v = t.iter().map(|p| w((*p + *off).as_dvec3() * size.as_dvec3())).collect();
                Poly::new(v, color)
            })
            .collect(),
    }
}

// ------------------------------------------------------------ output

/// Triangles (world space) with their colour.
pub fn triangles(polys: &[Poly]) -> Vec<([DVec3; 3], [f32; 3])> {
    let polys = close_t_junctions(polys);
    let mut out = Vec::new();
    for p in &polys {
        let n = p.v.len();
        // Collinear corners (T-junction vertices): a fan from the centroid
        // keeps every edge in a real triangle; else a plain fan.
        let collinear = (0..n).any(|i| {
            let (a, b, c) = (p.v[(i + n - 1) % n], p.v[i], p.v[(i + 1) % n]);
            (b - a).cross(c - b).length() <= 1e-9 * (b - a).length().max(1.0) * (c - b).length().max(1.0)
        });
        if collinear {
            let c = p.v.iter().copied().sum::<DVec3>() / n as f64;
            for i in 0..n {
                let t = [c, p.v[i], p.v[(i + 1) % n]];
                if (t[1] - t[0]).cross(t[2] - t[0]).length() > 1e-12 {
                    out.push((t, p.color));
                }
            }
        } else {
            for i in 1..n - 1 {
                let t = [p.v[0], p.v[i], p.v[i + 1]];
                if (t[1] - t[0]).cross(t[2] - t[0]).length() > 1e-9 {
                    out.push((t, p.color));
                }
            }
        }
    }
    out
}

/// BSP splits leave T-junctions (a vertex of one polygon in the middle of
/// a neighbour's edge), so the triangle soup isn't watertight and convex
/// decomposition's voxel fill leaks (a union's corner had no collider).
/// Every such vertex is added to the edge it lies on; vertices closer than
/// a tolerance are welded first.
fn close_t_junctions(polys: &[Poly]) -> Vec<Poly> {
    const WELD: f64 = 1e-6;
    let key = |p: DVec3| ((p.x / WELD).round() as i64, (p.y / WELD).round() as i64, (p.z / WELD).round() as i64);
    let mut canon: std::collections::HashMap<(i64, i64, i64), DVec3> = std::collections::HashMap::new();
    let polys: Vec<Poly> = polys
        .iter()
        .map(|p| {
            let mut q = p.clone();
            for v in q.v.iter_mut() {
                *v = *canon.entry(key(*v)).or_insert(*v);
            }
            q.v.dedup_by(|a, b| a == b);
            if q.v.len() > 1 && q.v.first() == q.v.last() {
                q.v.pop();
            }
            q
        })
        .filter(|q| q.v.len() >= 3)
        .collect();
    let mut verts: Vec<DVec3> = canon.into_values().collect();
    verts.sort_by(|a, b| a.x.total_cmp(&b.x));
    let xs: Vec<f64> = verts.iter().map(|v| v.x).collect();
    polys
        .into_iter()
        .map(|p| {
            let n = p.v.len();
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let (a, b) = (p.v[i], p.v[(i + 1) % n]);
                out.push(a);
                let d = b - a;
                let len2 = d.length_squared();
                if len2 < 1e-18 {
                    continue;
                }
                let (x0, x1) = (a.x.min(b.x) - 1e-7, a.x.max(b.x) + 1e-7);
                let start = xs.partition_point(|x| *x < x0);
                let mut mids: Vec<(f64, DVec3)> = Vec::new();
                for v in &verts[start..] {
                    if v.x > x1 {
                        break;
                    }
                    let t = (*v - a).dot(d) / len2;
                    if t <= 1e-9 || t >= 1.0 - 1e-9 {
                        continue;
                    }
                    if (a + d * t - *v).length_squared() < 1e-12 {
                        mids.push((t, *v));
                    }
                }
                mids.sort_by(|x, y| x.0.total_cmp(&y.0));
                out.extend(mids.into_iter().map(|m| m.1));
            }
            Poly { v: out, ..p }
        })
        .collect()
}

/// Connected pieces (triangles sharing a vertex position) of a result.
pub fn split_apart(tris: Vec<([DVec3; 3], [f32; 3])>) -> Vec<Vec<([DVec3; 3], [f32; 3])>> {
    let key = |p: DVec3| ((p.x * 1e4).round() as i64, (p.y * 1e4).round() as i64, (p.z * 1e4).round() as i64);
    let mut parent: Vec<usize> = (0..tris.len()).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut i = i;
        while p[i] != r {
            let n = p[i];
            p[i] = r;
            i = n;
        }
        r
    }
    let mut first: std::collections::HashMap<(i64, i64, i64), usize> = std::collections::HashMap::new();
    for (i, (t, _)) in tris.iter().enumerate() {
        for v in t {
            match first.get(&key(*v)) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a] = b;
                    }
                }
                None => {
                    first.insert(key(*v), i);
                }
            }
        }
    }
    let mut groups: Vec<(usize, Vec<([DVec3; 3], [f32; 3])>)> = Vec::new();
    for (i, t) in tris.into_iter().enumerate() {
        let r = find(&mut parent, i);
        match groups.iter_mut().find(|g| g.0 == r) {
            Some(g) => g.1.push(t),
            None => groups.push((r, vec![t])),
        }
    }
    groups.into_iter().map(|g| g.1).collect()
}

fn csgmdl_key() -> [u8; 31] {
    let mut x: u32 = 1337;
    let mut k = [0u8; 31];
    for b in k.iter_mut() {
        x = x.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        *b = (((x >> 16) & 0x7fff) % 127) as u8;
    }
    k
}

fn projection_plane(a: Vec3, b: Vec3, c: Vec3) -> u8 {
    let cross = (b - a).cross(c - a);
    let n = cross / (cross.length() + f32::MIN_POSITIVE);
    let biased = n.abs() + Vec3::new(0.01, 0.02, 0.0);
    let axis = if biased.x >= biased.y && biased.x >= biased.z {
        0
    } else if biased.y >= biased.z {
        1
    } else {
        2
    };
    (axis + if n[axis] < 0.0 { 3 } else { 0 }) as u8
}

fn pack_unit(out: &mut Vec<u8>, v: Vec3) {
    for c in [v.x, v.y, v.z] {
        let q = ((c.clamp(-1.0, 1.0) * 32767.0).round() as i32 + 0x7fff).clamp(0, 0xffff) as u16;
        out.extend_from_slice(&q.to_le_bytes());
    }
}

fn delta(out: &mut Vec<u8>, d: i32) {
    if (-64..64).contains(&d) {
        out.push((d & 0x7f) as u8);
    } else {
        out.push(0x80 | ((d >> 16) & 0x7f) as u8);
        out.push(((d >> 8) & 0xff) as u8);
        out.push((d & 0xff) as u8);
    }
}

/// Max vertices one CSGMDL blob holds (its counts are u16, ≤ 60000).
pub const MAX_VERTS: usize = 60000;

/// Local triangles (relative to the result's CFrame) as a CSGMDL v5 blob;
/// vertices are shared per (position, normal, colour, plane).
pub fn csgmdl(tris: &[([Vec3; 3], [f32; 3])]) -> Result<Vec<u8>, String> {
    let mut pos: Vec<Vec3> = Vec::new();
    let mut nrm: Vec<Vec3> = Vec::new();
    let mut col: Vec<[u8; 4]> = Vec::new();
    let mut tag: Vec<u8> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    let mut map: std::collections::HashMap<([u32; 3], [u8; 4], u8), u32> = std::collections::HashMap::new();
    for (t, c) in tris {
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        let plane = projection_plane(t[0], t[1], t[2]);
        let c8 = [
            (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
            255,
        ];
        for p in t {
            let k = (p.to_array().map(f32::to_bits), c8, plane);
            let i = *map.entry(k).or_insert_with(|| {
                pos.push(*p);
                nrm.push(n);
                col.push(c8);
                tag.push(plane);
                (pos.len() - 1) as u32
            });
            idx.push(i);
        }
    }
    if pos.len() > MAX_VERTS {
        return Err(format!("{} vertices", pos.len()));
    }
    let n = pos.len() as u16;
    let k = csgmdl_key();
    let mut out: Vec<u8> = b"CSGMDL".to_vec();
    out.extend_from_slice(&5u32.to_le_bytes());
    for (i, b) in out.iter_mut().enumerate() {
        *b ^= k[i % 31];
    }
    out.extend_from_slice(&n.to_le_bytes());
    for p in &pos {
        for c in p.to_array() {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out.extend_from_slice(&n.to_le_bytes());
    if n > 0 {
        out.extend_from_slice(&(n as u32 * 6).to_le_bytes());
        for v in &nrm {
            pack_unit(&mut out, *v);
        }
    }
    out.extend_from_slice(&n.to_le_bytes());
    for c in &col {
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&tag);
    // UVs: the planar projection the renderer derives from the tags.
    out.extend_from_slice(&n.to_le_bytes());
    for (p, t) in pos.iter().zip(&tag) {
        let uv = match t % 3 {
            0 => [p.z, p.y],
            1 => [p.x, p.z],
            _ => [p.x, p.y],
        };
        for c in uv {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out.extend_from_slice(&0u16.to_le_bytes()); // no tangents
    out.extend_from_slice(&(idx.len() as u32).to_le_bytes());
    let mut body = Vec::new();
    let mut prev = 0i64;
    for (i, v) in idx.iter().enumerate() {
        let d = if i == 0 { *v as i64 } else { *v as i64 - prev };
        delta(&mut body, d as i32);
        prev = *v as i64;
    }
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out.push(2);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(idx.len() as u32).to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(at: Vec3, s: f32) -> Vec<Poly> {
        part_polys(&Shape::Block, &Xform { pos: at, rot: glam::Mat3::IDENTITY }, Vec3::splat(s), [1.0, 0.0, 0.0])
    }

    fn volume(polys: &[Poly]) -> f64 {
        triangles(polys).iter().map(|(t, _)| t[0].dot(t[1].cross(t[2])) / 6.0).sum()
    }

    #[test]
    fn csg_volumes() {
        let a = cube(Vec3::ZERO, 4.0);
        let b = cube(Vec3::new(2.0, 0.0, 0.0), 4.0);
        assert!((volume(&a) - 64.0).abs() < 1e-6);
        assert!((volume(&union(a.clone(), b.clone())) - 96.0).abs() < 1e-6);
        assert!((volume(&subtract(a.clone(), b.clone())) - 32.0).abs() < 1e-6);
        assert!((volume(&intersect(a.clone(), b.clone())) - 32.0).abs() < 1e-6);
        let far = cube(Vec3::new(20.0, 0.0, 0.0), 2.0);
        assert!(intersect(a.clone(), far.clone()).is_empty());
        let u = union(a.clone(), far);
        assert_eq!(split_apart(triangles(&u)).len(), 2);
        // Watertight: every edge (welded) is used by exactly two triangles,
        // once each way.
        let b2 = part_polys(&Shape::Block, &Xform { pos: Vec3::new(2.0, 1.0, 0.0), rot: glam::Mat3::from_rotation_y(0.5) }, Vec3::splat(4.0), [0.0; 3]);
        let tris = triangles(&union(a, b2));
        let k = |p: DVec3| ((p.x * 1e5).round() as i64, (p.y * 1e5).round() as i64, (p.z * 1e5).round() as i64);
        let mut edges: std::collections::HashMap<_, i32> = std::collections::HashMap::new();
        for (t, _) in &tris {
            for i in 0..3 {
                *edges.entry((k(t[i]), k(t[(i + 1) % 3]))).or_default() += 1;
            }
        }
        let open = edges.iter().filter(|((a, b), n)| edges.get(&(*b, *a)).copied().unwrap_or(0) != **n).count();
        assert_eq!(open, 0, "open edges");
    }

    #[test]
    fn csgmdl_round_trip() {
        let a = cube(Vec3::ZERO, 4.0);
        let tris: Vec<([Vec3; 3], [f32; 3])> = triangles(&a)
            .into_iter()
            .map(|(t, c)| (t.map(|p| p.as_vec3()), c))
            .collect();
        let blob = csgmdl(&tris).unwrap();
        let m = crate::mesh::parse(&blob).unwrap();
        assert_eq!(m.positions.len(), 36);
        assert_eq!(m.min, Vec3::splat(-2.0));
        assert_eq!(m.max, Vec3::splat(2.0));
        assert_eq!(m.colors[0].x, 1.0);
    }
}
