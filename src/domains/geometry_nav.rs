//! PathfindingService / Path / PathWaypoint: a navmesh-style path solver
//! over the place's collidable parts and terrain.
//!
//! The world around the request is rasterised into a heightfield of
//! columns (1 stud cells, coarser for long requests): every collidable part
//! (CanCollide, not under a Humanoid's model; a PathfindingModifier with
//! PassThrough lets agents through its part) and solid terrain gives solid
//! spans per column. A span top with AgentHeight of headroom is a walkable
//! surface; surfaces within a step (1 stud, Studio walks over a 1-stud
//! wall and jumps a 1.5-stud one) of a neighbour connect; the walkable
//! area is eroded by AgentRadius. With AgentCanJump, obstacles up to ~6.6
//! studs high (Studio: 6 jumps, 7 is NoPath) are crossed by jump links.
//! PathfindingLinks add their Attachment0 → Attachment1 links. A* runs with
//! Costs (by material name, modifier label, link label; math.huge closes
//! it), the cell path is string-pulled, and waypoints are laid every
//! WaypointSpacing along each straight run, with the corners, the jump
//! (Action Jump, Label "Jump") and the ends (Studio's shapes; measured in
//! checks/parity/geometry.md).
//!
//! Studio behaviour kept: Path.Name "Instance", Status NoPath until
//! computed, "PathfindingService: path request is too long." beyond 3000
//! studs, CheckOcclusionAsync(start) checks the waypoints after `start` and
//! errors "Start point value is invalid" out of range, Blocked / Unblocked
//! fire as the world changes under a computed path.

use std::collections::{BinaryHeap, HashMap};

use glam::{Vec2, Vec3};
use mlua::{Lua, MultiValue, UserData, UserDataFields, UserDataMethods, Value};
use rbx_dom_weak::types::{Ref, Variant};

use crate::doc::{self, Document};
use crate::ltypes::*;
use crate::math::Xform;
use crate::runtime::*;

const STEP: f32 = 1.05;
const JUMP: f32 = 6.6;
const MAX_REQUEST: f32 = 3000.0;
const SNAP_BELOW: f32 = 40.0;
/// Studio drops off ledges up to here (100 studs found a path, 110 not).
const MAX_DROP: f32 = 105.0;

// ------------------------------------------------------------ PathWaypoint

#[derive(Clone, Debug, PartialEq)]
pub struct LPathWaypoint {
    pub pos: Vec3,
    pub action: u32,
    pub label: String,
}

fn action_item(v: u32) -> LEnumItem {
    LEnumItem::from_value("PathWaypointAction", v).unwrap_or(LEnumItem {
        enum_type: "PathWaypointAction".into(),
        name: "Walk".into(),
        value: 0,
    })
}

fn fmt_f(f: f32) -> String {
    let s = format!("{f}");
    if s == "-0" { "0".into() } else { s }
}

impl UserData for LPathWaypoint {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "PathWaypoint");
        f.add_field_method_get("Position", |_, w| Ok(LVector3(w.pos)));
        f.add_field_method_get("Action", |lua, w| enum_value(lua, action_item(w.action)));
        f.add_field_method_get("Label", |_, w| Ok(w.label.clone()));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method("__tostring", |_, w, ()| {
            Ok(format!(
                "{{{}, {}, {}}}, {}, {}",
                fmt_f(w.pos.x),
                fmt_f(w.pos.y),
                fmt_f(w.pos.z),
                action_item(w.action).name,
                w.label
            ))
        });
        m.add_meta_function("__eq", |_, (a, b): (Value, Value)| {
            Ok(match (ud_of::<LPathWaypoint>(&a), ud_of::<LPathWaypoint>(&b)) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            })
        });
        m.add_meta_method("__newindex", |_, _, (k, _v): (String, Value)| -> mlua::Result<()> {
            Err(rt_err(format!("{k} member of PathWaypoint is read-only and cannot be assigned to")))
        });
    }
}

fn ud_of<T: Clone + 'static>(v: &Value) -> Option<T> {
    if let Some(x) = crate::ltypes::vec_any::<T>(v) {
        return Some(x);
    }
    match v {
        Value::UserData(u) => u.borrow::<T>().ok().map(|x| x.clone()),
        _ => None,
    }
}

// ------------------------------------------------------------ agent / path

#[derive(Clone, Debug)]
pub struct Agent {
    pub radius: f32,
    pub height: f32,
    pub can_jump: bool,
    pub can_climb: bool,
    pub spacing: f32,
    pub costs: HashMap<String, f32>,
    pub partial: bool,
}

impl Default for Agent {
    fn default() -> Self {
        Agent { radius: 2.0, height: 5.0, can_jump: true, can_climb: false, spacing: 4.0, costs: HashMap::new(), partial: false }
    }
}

#[derive(Clone, Debug)]
struct PathData {
    agent: Agent,
    status: u32,
    waypoints: Vec<LPathWaypoint>,
    blocked: Option<usize>,
    checked_rev: u64,
    next_check: f64,
}

/// The computed paths of one Lua state (each runtime has its own).
#[derive(Default)]
struct Paths(HashMap<Ref, PathData>);

fn paths<R>(lua: &Lua, f: impl FnOnce(&mut HashMap<Ref, PathData>) -> R) -> R {
    if lua.app_data_ref::<Paths>().is_none() {
        lua.set_app_data(Paths::default());
    }
    let mut g = lua.app_data_mut::<Paths>().expect("paths");
    f(&mut g.0)
}

const S_SUCCESS: u32 = 0;
const S_CLOSEST_NO_PATH: u32 = 1;
const S_NO_PATH: u32 = 5;

// ------------------------------------------------------------ heightfield

#[derive(Clone, Copy, Debug)]
struct Span {
    lo: f32,
    hi: f32,
    /// Index into `Field::labels` of the surface on top.
    label: u16,
    /// Passes agents (a PathfindingModifier with PassThrough) but labels.
    pass: bool,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    h: f32,
    label: u16,
    /// Distance (studs) to the nearest edge or wall.
    clear: f32,
}

pub struct Field {
    origin: Vec2,
    cs: f32,
    nx: usize,
    nz: usize,
    /// Per cell: walkable surfaces, lowest first.
    cells: Vec<Vec<Node>>,
    /// Per cell: merged solid spans (headroom / occlusion checks).
    solids: Vec<Vec<(f32, f32)>>,
    labels: Vec<String>,
    /// Modifier volumes: (inverse xform, half size, label).
    modifiers: Vec<(Xform, Vec3, u16)>,
}

#[derive(Clone)]
struct Obstacle {
    xf: Xform,
    half: Vec3,
    shape: u8, // 0 box, 1 ball, 2 cylinder, 3 wedge, 4 corner wedge
    label: u16,
    pass: bool,
    /// A MeshPart's / union's own triangles (unit box, centred on `xf`):
    /// Studio's navmesh follows that geometry whatever the CollisionFidelity
    /// (a doorway cut in a wall is walked through with Box or Hull).
    mesh: Option<std::sync::Arc<Vec<[Vec3; 3]>>>,
}

/// The id of a MeshPart's / union's mesh.
fn mesh_id(doc: &Document, r: Ref, class: &str) -> Option<String> {
    if class == "MeshPart" {
        crate::scene::content_string(doc.prop(r, "MeshId")).or_else(|| crate::scene::content_string(doc.prop(r, "MeshContent")))
    } else if doc::is_a(class, "PartOperation") {
        crate::scene::union_mesh(doc, r).map(|m| m.id)
    } else {
        None
    }
}

/// Solid spans of a mesh obstacle per column: each triangle crossing a
/// column's vertical line is an entry (facing down) or an exit (facing up).
fn mesh_spans(o: &Obstacle, tris: &[[Vec3; 3]], lo: Vec2, cs: f32, nx: usize, nz: usize, out: &mut [Vec<Span>]) {
    let size = o.half * 2.0;
    let mut crossings: HashMap<usize, Vec<(f32, bool)>> = HashMap::new();
    for t in tris {
        let w = t.map(|p| o.xf.apply(p * size));
        let n = (w[1] - w[0]).cross(w[2] - w[0]);
        if n.y.abs() < 1e-9 {
            continue;
        }
        let mn = w[0].min(w[1]).min(w[2]);
        let mx = w[0].max(w[1]).max(w[2]);
        let x0 = (((mn.x - lo.x) / cs - 0.5).ceil() as i64).max(0);
        let x1 = (((mx.x - lo.x) / cs - 0.5).floor() as i64).min(nx as i64 - 1);
        let z0 = (((mn.z - lo.y) / cs - 0.5).ceil() as i64).max(0);
        let z1 = (((mx.z - lo.y) / cs - 0.5).floor() as i64).min(nz as i64 - 1);
        for iz in z0..=z1 {
            for ix in x0..=x1 {
                // Nudged off the cell centre so a line through a shared edge
                // isn't counted twice.
                let p = lo + Vec2::new((ix as f32 + 0.5) * cs + 1.3e-4, (iz as f32 + 0.5) * cs + 0.7e-4);
                let e = |a: Vec3, b: Vec3| (b.x - a.x) * (p.y - a.z) - (b.z - a.z) * (p.x - a.x);
                let (e0, e1, e2) = (e(w[0], w[1]), e(w[1], w[2]), e(w[2], w[0]));
                if !((e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0)) {
                    continue;
                }
                // The plane's height at p.
                let y = w[0].y - (n.x * (p.x - w[0].x) + n.z * (p.y - w[0].z)) / n.y;
                crossings.entry(iz as usize * nx + ix as usize).or_default().push((y, n.y < 0.0));
            }
        }
    }
    for (i, mut c) in crossings {
        c.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut depth = 0i32;
        let mut start = 0.0;
        let balanced = c.iter().filter(|x| x.1).count() * 2 == c.len();
        if balanced {
            for (y, enter) in c {
                if enter {
                    if depth == 0 {
                        start = y;
                    }
                    depth += 1;
                } else if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        out[i].push(Span { lo: start, hi: y, label: o.label, pass: o.pass });
                    }
                }
            }
        } else {
            // An open mesh: pair the crossings bottom-up.
            for pair in c.chunks_exact(2) {
                out[i].push(Span { lo: pair[0].0, hi: pair[1].0, label: o.label, pass: o.pass });
            }
        }
    }
}

fn label_id(labels: &mut Vec<String>, s: &str) -> u16 {
    if let Some(i) = labels.iter().position(|l| l == s) {
        return i as u16;
    }
    labels.push(s.to_string());
    (labels.len() - 1) as u16
}

fn material_name(doc: &Document, r: Ref) -> String {
    let v = crate::scene::enum_u32(doc.prop(r, "Material")).unwrap_or(256);
    LEnumItem::from_value("Material", v).map(|e| e.name).unwrap_or_else(|| "Plastic".into())
}

fn bool_prop(doc: &Document, r: Ref, k: &str, default: bool) -> bool {
    match doc.prop(r, k) {
        Some(Variant::Bool(b)) => b,
        _ => default,
    }
}

fn under_humanoid(doc: &Document, r: Ref) -> bool {
    let mut at = doc.dom.get_by_ref(r).map(|i| i.parent()).unwrap_or(Ref::none());
    for _ in 0..4 {
        let Some(i) = doc.dom.get_by_ref(at) else { return false };
        if i.children().iter().any(|c| doc.class_of(*c).is_some_and(|k| k == "Humanoid")) {
            return true;
        }
        at = i.parent();
    }
    false
}

fn shape_of(doc: &Document, r: Ref, class: &str) -> u8 {
    match class {
        "WedgePart" => 3,
        "CornerWedgePart" => 4,
        c if doc::is_a(c, "Part") => match crate::scene::enum_u32(doc.prop(r, "Shape")) {
            Some(0) => 1,
            Some(2) => 2,
            Some(3) => 3,
            Some(4) => 4,
            _ => 0,
        },
        _ => 0,
    }
}

/// The interval of t where the vertical line (x, t, z) is inside the shape.
fn column_interval(o: &Obstacle, x: f32, z: f32) -> Option<(f32, f32)> {
    let inv = o.xf.rot.transpose();
    let base = inv * (Vec3::new(x, 0.0, z) - o.xf.pos);
    let dir = inv * Vec3::Y;
    let h = o.half;
    let mut lo = f32::NEG_INFINITY;
    let mut hi = f32::INFINITY;
    let mut slab = |b: f32, d: f32, e: f32| -> bool {
        if d.abs() < 1e-7 {
            return b.abs() <= e;
        }
        let (a, c) = ((-e - b) / d, (e - b) / d);
        let (a, c) = if a < c { (a, c) } else { (c, a) };
        lo = lo.max(a);
        hi = hi.min(c);
        lo <= hi
    };
    match o.shape {
        1 => {
            // Ball: sphere of the smallest half size.
            let r = h.min_element();
            let c = o.xf.pos;
            let dx = x - c.x;
            let dz = z - c.z;
            let s = r * r - dx * dx - dz * dz;
            if s < 0.0 {
                return None;
            }
            let s = s.sqrt();
            return Some((c.y - s, c.y + s));
        }
        2 => {
            // Cylinder along local X: |x| ≤ hx and y² + z² ≤ r².
            if !slab(base.x, dir.x, h.x) {
                return None;
            }
            let r = h.y.min(h.z);
            let (a, b, c) = (
                dir.y * dir.y + dir.z * dir.z,
                2.0 * (base.y * dir.y + base.z * dir.z),
                base.y * base.y + base.z * base.z - r * r,
            );
            if a < 1e-9 {
                if c > 0.0 {
                    return None;
                }
            } else {
                let disc = b * b - 4.0 * a * c;
                if disc < 0.0 {
                    return None;
                }
                let s = disc.sqrt();
                lo = lo.max((-b - s) / (2.0 * a));
                hi = hi.min((-b + s) / (2.0 * a));
            }
            return (lo <= hi).then_some((lo, hi));
        }
        _ => {}
    }
    if !(slab(base.x, dir.x, h.x) && slab(base.y, dir.y, h.y) && slab(base.z, dir.z, h.z)) {
        return None;
    }
    if o.shape == 3 || o.shape == 4 {
        // Wedge: solid below the plane through (y=-h, z=-h) .. (y=+h, z=+h)
        // (full height at the back, +Z): n·p ≤ 0 with n = (0, hz, -hy).
        let n = Vec3::new(0.0, h.z, -h.y);
        let nb = n.dot(base);
        let nd = n.dot(dir);
        if nd.abs() < 1e-9 {
            if nb > 0.0 {
                return None;
            }
        } else {
            let t = -nb / nd;
            if nd > 0.0 {
                hi = hi.min(t);
            } else {
                lo = lo.max(t);
            }
        }
        if o.shape == 4 {
            // Corner wedge: also below the plane rising along -X.
            let n = Vec3::new(h.y, h.x, 0.0);
            let nb = n.dot(base) - h.x * h.y;
            let nd = n.dot(dir);
            if nd.abs() > 1e-9 {
                let t = -nb / nd;
                if nd > 0.0 {
                    hi = hi.min(t);
                } else {
                    lo = lo.max(t);
                }
            }
        }
    }
    (lo <= hi).then_some((lo, hi))
}

fn obstacles(doc: &Document, labels: &mut Vec<String>, lo: Vec2, hi: Vec2) -> (Vec<Obstacle>, Vec<(Xform, Vec3, u16)>) {
    let mut out = Vec::new();
    let mut mods = Vec::new();
    let Some(ws) = doc.workspace() else { return (out, mods) };
    for r in doc.subtree(ws) {
        let Some(class) = doc.class_of(r) else { continue };
        if !doc::is_a(&class, "BasePart") || class == "Terrain" {
            continue;
        }
        let Some(xf) = crate::scene::xform_of(doc, r) else { continue };
        let size = crate::scene::vec3_of(doc.prop(r, "Size")).unwrap_or(Vec3::new(4.0, 1.0, 2.0));
        let half = size.abs() * 0.5;
        let ext = xf.rot.x_axis.abs() * half.x + xf.rot.y_axis.abs() * half.y + xf.rot.z_axis.abs() * half.z;
        if xf.pos.x + ext.x < lo.x || xf.pos.x - ext.x > hi.x || xf.pos.z + ext.z < lo.y || xf.pos.z - ext.z > hi.y {
            continue;
        }
        // A PathfindingModifier child labels the part's volume.
        let modifier = doc.dom.get_by_ref(r).and_then(|i| {
            i.children().iter().copied().find(|c| doc.class_of(*c).is_some_and(|k| k == "PathfindingModifier"))
        });
        let (label, pass) = match modifier {
            Some(m) => {
                let l = match doc.prop(m, "Label") {
                    Some(Variant::String(s)) => s,
                    _ => String::new(),
                };
                let pass = bool_prop(doc, m, "PassThrough", false);
                let l = if l.is_empty() { material_name(doc, r) } else { l };
                let id = label_id(labels, &l);
                mods.push((xf, half, id));
                (id, pass)
            }
            None => (label_id(labels, &material_name(doc, r)), false),
        };
        let collide = bool_prop(doc, r, "CanCollide", true);
        if !collide && !pass {
            continue;
        }
        if under_humanoid(doc, r) {
            continue;
        }
        let mut xf = xf;
        let mesh = mesh_id(doc, r, &class).and_then(|id| {
            let tris = crate::assets::mesh_triangles(&id)?;
            // An off-centre union's geometry (GeometryService).
            xf.pos += xf.rot * (crate::mesh_bvh::union_offset(&id) * size.abs());
            Some(tris)
        });
        out.push(Obstacle { xf, half, shape: shape_of(doc, r, &class), label, pass, mesh });
    }
    (out, mods)
}

impl Field {
    fn index(&self, ix: i64, iz: i64) -> Option<usize> {
        (ix >= 0 && iz >= 0 && (ix as usize) < self.nx && (iz as usize) < self.nz).then(|| iz as usize * self.nx + ix as usize)
    }

    fn cell_of(&self, p: Vec3) -> (i64, i64) {
        (((p.x - self.origin.x) / self.cs).floor() as i64, ((p.z - self.origin.y) / self.cs).floor() as i64)
    }

    fn center(&self, ix: i64, iz: i64) -> Vec2 {
        self.origin + Vec2::new((ix as f32 + 0.5) * self.cs, (iz as f32 + 0.5) * self.cs)
    }

    /// Builds the heightfield for [lo, hi] (horizontal x, z) with cell `cs`.
    pub fn build(doc: &Document, lo: Vec2, hi: Vec2, cs: f32, ylo: f32, yhi: f32, agent: &Agent) -> Field {
        let nx = (((hi.x - lo.x) / cs).ceil() as usize).max(1);
        let nz = (((hi.y - lo.y) / cs).ceil() as usize).max(1);
        let mut labels = Vec::new();
        let (obs, modifiers) = obstacles(doc, &mut labels, lo, hi);
        let mut spans: Vec<Vec<Span>> = vec![Vec::new(); nx * nz];
        for o in &obs {
            let ext = o.xf.rot.x_axis.abs() * o.half.x + o.xf.rot.y_axis.abs() * o.half.y + o.xf.rot.z_axis.abs() * o.half.z;
            if o.xf.pos.y + ext.y < ylo || o.xf.pos.y - ext.y > yhi {
                continue;
            }
            if let Some(tris) = &o.mesh {
                mesh_spans(o, tris, lo, cs, nx, nz, &mut spans);
                continue;
            }
            let x0 = (((o.xf.pos.x - ext.x - lo.x) / cs).floor() as i64).max(0);
            let x1 = (((o.xf.pos.x + ext.x - lo.x) / cs).floor() as i64).min(nx as i64 - 1);
            let z0 = (((o.xf.pos.z - ext.z - lo.y) / cs).floor() as i64).max(0);
            let z1 = (((o.xf.pos.z + ext.z - lo.y) / cs).floor() as i64).min(nz as i64 - 1);
            for iz in z0..=z1 {
                for ix in x0..=x1 {
                    let c = lo + Vec2::new((ix as f32 + 0.5) * cs, (iz as f32 + 0.5) * cs);
                    if let Some((a, b)) = column_interval(o, c.x, c.y) {
                        spans[iz as usize * nx + ix as usize].push(Span { lo: a, hi: b, label: o.label, pass: o.pass });
                    }
                }
            }
        }
        // Terrain: solid voxels (and water, walkable, labelled Water).
        if let Some(Ok(v)) = crate::terrain::Voxels::from_dom(&doc.dom) {
            if v.count() > 0 {
                let vy0 = (ylo / crate::terrain::VOXEL).floor() as i32;
                let vy1 = (yhi / crate::terrain::VOXEL).ceil() as i32;
                for iz in 0..nz {
                    for ix in 0..nx {
                        let c = lo + Vec2::new((ix as f32 + 0.5) * cs, (iz as f32 + 0.5) * cs);
                        let vx = (c.x / crate::terrain::VOXEL).floor() as i32;
                        let vz = (c.y / crate::terrain::VOXEL).floor() as i32;
                        let mut run: Option<(f32, f32, u8)> = None;
                        let col = &mut spans[iz * nx + ix];
                        for vy in vy0..=vy1 {
                            let (m, occ) = v.get(vx, vy, vz);
                            let y = vy as f32 * crate::terrain::VOXEL;
                            if m != 0 {
                                let top = y + crate::terrain::VOXEL * occ.clamp(0.25, 1.0);
                                run = Some(match run {
                                    Some((a, _, _)) => (a, top, m),
                                    None => (y, top, m),
                                });
                            } else if let Some((a, b, m)) = run.take() {
                                let name = crate::terrain::material_name(m);
                                let l = label_id(&mut labels, name);
                                col.push(Span { lo: a, hi: b, label: l, pass: false });
                            }
                        }
                        if let Some((a, b, m)) = run {
                            let l = label_id(&mut labels, crate::terrain::material_name(m));
                            col.push(Span { lo: a, hi: b, label: l, pass: false });
                        }
                    }
                }
            }
        }
        // Merge solids; surfaces are span tops with headroom.
        let mut cells = vec![Vec::new(); nx * nz];
        let mut solids = vec![Vec::new(); nx * nz];
        for i in 0..nx * nz {
            let col = &mut spans[i];
            col.sort_by(|a, b| a.lo.total_cmp(&b.lo));
            let mut merged: Vec<Span> = Vec::new();
            for s in col.iter().filter(|s| !s.pass) {
                match merged.last_mut() {
                    Some(m) if s.lo <= m.hi + 0.05 => {
                        if s.hi >= m.hi {
                            m.hi = s.hi;
                            m.label = s.label;
                        }
                    }
                    _ => merged.push(*s),
                }
            }
            for (k, s) in merged.iter().enumerate() {
                let ceiling = merged.get(k + 1).map(|n| n.lo).unwrap_or(f32::INFINITY);
                if ceiling - s.hi >= agent.height * 0.999 {
                    cells[i].push(Node { h: s.hi, label: s.label, clear: f32::INFINITY });
                }
            }
            solids[i] = merged.iter().map(|s| (s.lo, s.hi)).collect();
        }
        let mut f = Field { origin: lo, cs, nx, nz, cells, solids, labels, modifiers };
        f.relabel_modifiers();
        f.erode(agent.radius);
        f
    }

    /// Surfaces inside a modifier volume take its label.
    fn relabel_modifiers(&mut self) {
        if self.modifiers.is_empty() {
            return;
        }
        let mods = self.modifiers.clone();
        for iz in 0..self.nz {
            for ix in 0..self.nx {
                let c = self.center(ix as i64, iz as i64);
                for n in self.cells[iz * self.nx + ix].iter_mut() {
                    let p = Vec3::new(c.x, n.h + 0.5, c.y);
                    for (xf, half, l) in &mods {
                        let q = xf.rot.transpose() * (p - xf.pos);
                        if q.abs().cmple(*half + Vec3::splat(0.6)).all() {
                            n.label = *l;
                        }
                    }
                }
            }
        }
    }

    /// The node at (ix, iz) within `step` of height h.
    fn node_near(&self, ix: i64, iz: i64, h: f32, step: f32) -> Option<(usize, usize)> {
        let i = self.index(ix, iz)?;
        let mut best: Option<(usize, f32)> = None;
        for (k, n) in self.cells[i].iter().enumerate() {
            let d = (n.h - h).abs();
            if d <= step && best.is_none_or(|b| d < b.1) {
                best = Some((k, d));
            }
        }
        best.map(|b| (i, b.0))
    }

    /// Distance to the nearest edge / wall per node (two-pass chamfer over
    /// the connected surfaces), compared against the agent radius by users.
    fn erode(&mut self, _radius: f32) {
        const D: [(i64, i64, f32); 8] = [
            (1, 0, 1.0), (-1, 0, 1.0), (0, 1, 1.0), (0, -1, 1.0),
            (1, 1, std::f32::consts::SQRT_2), (-1, 1, std::f32::consts::SQRT_2),
            (1, -1, std::f32::consts::SQRT_2), (-1, -1, std::f32::consts::SQRT_2),
        ];
        // Border: a node with a 4-neighbour missing at its height.
        let mut dist: Vec<Vec<f32>> = self.cells.iter().map(|c| vec![f32::INFINITY; c.len()]).collect();
        let mut heap = BinaryHeap::new();
        for iz in 0..self.nz as i64 {
            for ix in 0..self.nx as i64 {
                let i = iz as usize * self.nx + ix as usize;
                for k in 0..self.cells[i].len() {
                    let h = self.cells[i][k].h;
                    let border = D[..4].iter().any(|(dx, dz, _)| self.node_near(ix + dx, iz + dz, h, STEP).is_none());
                    if border {
                        dist[i][k] = 0.5 * self.cs;
                        heap.push(HeapItem { f: -dist[i][k], node: (i, k) });
                    }
                }
            }
        }
        while let Some(HeapItem { f, node: (i, k) }) = heap.pop() {
            let d = -f;
            if d > dist[i][k] {
                continue;
            }
            let (ix, iz) = ((i % self.nx) as i64, (i / self.nx) as i64);
            let h = self.cells[i][k].h;
            for (dx, dz, w) in D {
                if let Some((j, l)) = self.node_near(ix + dx, iz + dz, h, STEP) {
                    let nd = d + w * self.cs;
                    if nd < dist[j][l] {
                        dist[j][l] = nd;
                        heap.push(HeapItem { f: -nd, node: (j, l) });
                    }
                }
            }
        }
        for (i, c) in self.cells.iter_mut().enumerate() {
            for (k, n) in c.iter_mut().enumerate() {
                n.clear = dist[i][k];
            }
        }
    }

    fn label(&self, n: &Node) -> &str {
        self.labels.get(n.label as usize).map(String::as_str).unwrap_or("")
    }

    /// Whether something solid now stands in an agent's way at p (its
    /// cylinder from a step above the ground to its height).
    fn obstructed(&self, p: Vec3, agent: &Agent) -> bool {
        let r = (agent.radius * 0.5).max(self.cs * 0.5);
        let (cx, cz) = self.cell_of(p);
        let n = (r / self.cs).ceil() as i64;
        for dz in -n..=n {
            for dx in -n..=n {
                let Some(i) = self.index(cx + dx, cz + dz) else { continue };
                let c = self.center(cx + dx, cz + dz);
                if Vec2::new(c.x - p.x, c.y - p.z).length() > r + self.cs * 0.5 {
                    continue;
                }
                for (lo, hi) in &self.solids[i] {
                    if *hi > p.y + STEP && *lo < p.y + agent.height * 0.9 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[derive(PartialEq)]
struct HeapItem {
    f: f32,
    node: (usize, usize),
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.f.total_cmp(&o.f).then(self.node.cmp(&o.node))
    }
}

// ------------------------------------------------------------ solver

#[derive(Clone, Copy, Debug, PartialEq)]
enum Edge {
    Walk,
    Jump,
    /// A PathfindingLink (index into the link list).
    Link(usize),
    /// A TrussPart climb (index into the climb points).
    Climb(usize),
}

struct Link {
    a: Vec3,
    b: Vec3,
    label: String,
}

fn links(doc: &Document) -> Vec<(Link, bool)> {
    let mut out = Vec::new();
    let Some(ws) = doc.workspace() else { return out };
    for r in doc.subtree(ws) {
        if doc.class_of(r).is_none_or(|c| c != "PathfindingLink") {
            continue;
        }
        let att = |k: &str| match doc.prop(r, k) {
            Some(Variant::Ref(a)) if !a.is_none() => crate::attachment::world(doc, a, true).map(|x| x.pos),
            _ => None,
        };
        let (Some(a), Some(b)) = (att("Attachment0"), att("Attachment1")) else { continue };
        let label = match doc.prop(r, "Label") {
            Some(Variant::String(s)) => s,
            _ => String::new(),
        };
        out.push((Link { a, b, label }, bool_prop(doc, r, "IsBidirectional", true)));
    }
    out
}

/// TrussParts near the request: (CFrame, half size).
fn trusses(doc: &Document, lo: Vec2, hi: Vec2) -> Vec<(Xform, Vec3)> {
    let mut out = Vec::new();
    let Some(ws) = doc.workspace() else { return out };
    for r in doc.subtree(ws) {
        if doc.class_of(r).is_none_or(|c| c != "TrussPart") {
            continue;
        }
        let (Some(xf), Some(size)) = (crate::scene::xform_of(doc, r), crate::scene::vec3_of(doc.prop(r, "Size"))) else { continue };
        if xf.pos.x < lo.x || xf.pos.x > hi.x || xf.pos.z < lo.y || xf.pos.z > hi.y {
            continue;
        }
        out.push((xf, size.abs() * 0.5));
    }
    out
}

pub struct Solved {
    pub status: u32,
    pub waypoints: Vec<LPathWaypoint>,
}

fn usable(f: &Field, agent: &Agent, i: usize, k: usize) -> bool {
    let n = &f.cells[i][k];
    if n.clear + 1e-3 < agent.radius {
        return false;
    }
    !agent.costs.get(f.label(n)).is_some_and(|c| !c.is_finite())
}

fn node_cost(f: &Field, agent: &Agent, n: &Node) -> f32 {
    agent.costs.get(f.label(n)).copied().unwrap_or(1.0).max(0.0)
}

/// The surface below (or just above) p that the request snaps to: the
/// usable node nearest p within a few cells (any standable one in the
/// start's own column first, as Studio keeps the start's x, z).
fn snap(f: &Field, agent: &Agent, p: Vec3) -> Option<(usize, usize)> {
    let (cx, cz) = f.cell_of(p);
    let reach = ((agent.radius + 4.0) / f.cs).ceil() as i64;
    let mut best: Option<((usize, usize), f32)> = None;
    for dz in -reach..=reach {
        for dx in -reach..=reach {
            let Some(i) = f.index(cx + dx, cz + dz) else { continue };
            for (k, n) in f.cells[i].iter().enumerate() {
                if n.h > p.y + agent.height || n.h < p.y - SNAP_BELOW || !usable(f, agent, i, k) {
                    continue;
                }
                let c = f.center(cx + dx, cz + dz);
                let d = Vec2::new(c.x - p.x, c.y - p.z).length() + (n.h - p.y).max(0.0) * 0.5 + (p.y - n.h).max(0.0) * 0.05;
                if best.is_none_or(|b| d < b.1) {
                    best = Some(((i, k), d));
                }
            }
        }
    }
    best.map(|b| b.0)
}

/// Standing height at p's column nearest `h` (any surface, not eroded).
fn ground(f: &Field, p: Vec2, h: f32) -> Option<f32> {
    let (cx, cz) = f.cell_of(Vec3::new(p.x, 0.0, p.y));
    let i = f.index(cx, cz)?;
    f.cells[i].iter().map(|n| n.h).min_by(|a, b| (a - h).abs().total_cmp(&(b - h).abs()))
}

pub fn solve(doc: &Document, agent: &Agent, start: Vec3, finish: Vec3) -> Solved {
    let fail = Solved { status: S_NO_PATH, waypoints: Vec::new() };
    let d = Vec2::new(finish.x - start.x, finish.z - start.z).length();
    let margin = 48.0 + d * 0.25;
    let lo = Vec2::new(start.x.min(finish.x), start.z.min(finish.z)) - Vec2::splat(margin);
    let hi = Vec2::new(start.x.max(finish.x), start.z.max(finish.z)) + Vec2::splat(margin);
    let span = (hi - lo).max_element();
    let cs = (span / 512.0).max(1.0);
    let ylo = start.y.min(finish.y) - SNAP_BELOW - 64.0;
    let yhi = start.y.max(finish.y) + 96.0;
    let f = Field::build(doc, lo, hi, cs, ylo, yhi, agent);
    let Some(s) = snap(&f, agent, start) else { return fail };
    let Some(g) = snap(&f, agent, finish) else { return fail };

    // Links snapped onto the field.
    let all_links = links(doc);
    let mut link_edges: HashMap<(usize, usize), Vec<((usize, usize), usize)>> = HashMap::new();
    for (li, (l, bi)) in all_links.iter().enumerate() {
        if let (Some(a), Some(b)) = (snap(&f, agent, l.a), snap(&f, agent, l.b)) {
            link_edges.entry(a).or_default().push((b, li));
            if *bi {
                link_edges.entry(b).or_default().push((a, li));
            }
        }
    }

    // TrussPart climbs (AgentCanClimb): usable ground at the truss's foot
    // to usable ground at its top, both ways. Studio's waypoint for it is
    // Action Custom, Label "Climb", at the truss top on the approach side.
    let climb_cost = agent.costs.get("Climb").copied().unwrap_or(1.0);
    let mut climb_edges: HashMap<(usize, usize), Vec<((usize, usize), usize)>> = HashMap::new();
    let mut climb_points: Vec<Vec3> = Vec::new();
    if agent.can_climb && climb_cost.is_finite() {
        for (xf, half) in trusses(doc, lo, hi) {
            let ext = xf.rot.x_axis.abs() * half.x + xf.rot.y_axis.abs() * half.y + xf.rot.z_axis.abs() * half.z;
            let (bottom, top) = (xf.pos.y - ext.y, xf.pos.y + ext.y);
            let foot = Vec2::new(ext.x, ext.z).max_element();
            let near = |h_lo: f32, h_hi: f32, reach: f32| -> Vec<(usize, usize)> {
                let mut out = Vec::new();
                let (cx, cz) = f.cell_of(xf.pos);
                let n = (reach / f.cs).ceil() as i64 + 1;
                for dz in -n..=n {
                    for dx in -n..=n {
                        let Some(i) = f.index(cx + dx, cz + dz) else { continue };
                        let c = f.center(cx + dx, cz + dz);
                        let d = Vec2::new((c.x - xf.pos.x).abs() - ext.x, (c.y - xf.pos.z).abs() - ext.z).max(Vec2::ZERO).length();
                        if d > reach {
                            continue;
                        }
                        for (k, nd) in f.cells[i].iter().enumerate() {
                            if nd.h >= h_lo && nd.h <= h_hi && usable(&f, agent, i, k) {
                                out.push((i, k));
                            }
                        }
                    }
                }
                out
            };
            let reach = agent.radius + 1.5;
            let feet = near(bottom - STEP, bottom + STEP * 2.0, reach);
            let tops = near(top - STEP * 1.5, top + STEP, reach + foot);
            if feet.is_empty() || tops.is_empty() {
                continue;
            }
            for a in &feet {
                let ap = f.center((a.0 % f.nx) as i64, (a.0 / f.nx) as i64);
                let toward = (ap - Vec2::new(xf.pos.x, xf.pos.z)).normalize_or_zero() * 0.5;
                let up = Vec3::new(xf.pos.x + toward.x, top, xf.pos.z + toward.y);
                let down = Vec3::new(xf.pos.x + toward.x, bottom, xf.pos.z + toward.y);
                for b in &tops {
                    climb_points.push(up);
                    climb_edges.entry(*a).or_default().push((*b, climb_points.len() - 1));
                    climb_points.push(down);
                    climb_edges.entry(*b).or_default().push((*a, climb_points.len() - 1));
                }
            }
        }
    }

    // A* over (cell, surface).
    let pos = |(i, k): (usize, usize)| -> Vec3 {
        let c = f.center((i % f.nx) as i64, (i / f.nx) as i64);
        Vec3::new(c.x, f.cells[i][k].h, c.y)
    };
    let goal = pos(g);
    let mut best: HashMap<(usize, usize), (f32, (usize, usize), Edge)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    best.insert(s, (0.0, s, Edge::Walk));
    heap.push(HeapItem { f: -(pos(s).distance(goal)), node: s });
    const D: [(i64, i64); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)];
    let mut closest = (s, pos(s).distance(goal));
    let mut found = false;
    let mut expanded = 0usize;
    while let Some(HeapItem { node, .. }) = heap.pop() {
        if node == g {
            found = true;
            break;
        }
        expanded += 1;
        if expanded > 2_000_000 {
            break;
        }
        let gcost = best[&node].0;
        let here = pos(node);
        let hd = here.distance(goal);
        if hd < closest.1 {
            closest = (node, hd);
        }
        let (i, k) = node;
        let (ix, iz) = ((i % f.nx) as i64, (i / f.nx) as i64);
        let h = f.cells[i][k].h;
        let mut relax = |to: (usize, usize), cost: f32, e: Edge, heap: &mut BinaryHeap<HeapItem>| {
            let ng = gcost + cost;
            if best.get(&to).is_none_or(|b| ng < b.0) {
                best.insert(to, (ng, node, e));
                heap.push(HeapItem { f: -(ng + pos(to).distance(goal)), node: to });
            }
        };
        for (dx, dz) in D {
            let Some(to) = f.node_near(ix + dx, iz + dz, h, STEP) else { continue };
            if !usable(&f, agent, to.0, to.1) {
                continue;
            }
            if dx != 0 && dz != 0 {
                // No corner cutting.
                let a = f.node_near(ix + dx, iz, h, STEP).is_some_and(|n| usable(&f, agent, n.0, n.1));
                let b = f.node_near(ix, iz + dz, h, STEP).is_some_and(|n| usable(&f, agent, n.0, n.1));
                if !(a && b) {
                    continue;
                }
            }
            let w = (dx as f32).hypot(dz as f32) * f.cs + (f.cells[to.0][to.1].h - h).abs();
            let c = node_cost(&f, agent, &f.cells[to.0][to.1]);
            relax(to, w * c, Edge::Walk, &mut heap);
        }
        let jump_cost = agent.costs.get("Jump").copied().unwrap_or(1.0);
        if agent.can_jump && jump_cost.is_finite() {
            // Jump links over low obstacles to the next usable surface, and
            // drops off ledges (Studio: up to ~100 studs; both "Jump").
            let kmax = ((agent.radius * 2.0 + 6.0) / f.cs).ceil() as i64;
            for (dx, dz) in D {
                let mut crossed = false;
                for k2 in 1..=kmax {
                    let (jx, jz) = (ix + dx * k2, iz + dz * k2);
                    let Some(j) = f.index(jx, jz) else { break };
                    let level = f.node_near(jx, jz, h, STEP);
                    if !crossed && level.is_some_and(|t| usable(&f, agent, t.0, t.1)) {
                        // Still on reachable ground: not a jump.
                        break;
                    }
                    // The best landing in this column: level (after an
                    // obstacle), up to the jump height, else the highest
                    // surface below within the drop.
                    let land = f.cells[j]
                        .iter()
                        .enumerate()
                        .filter(|(l, n)| n.h <= h + JUMP && n.h >= h - MAX_DROP && usable(&f, agent, j, *l))
                        .filter(|(_, n)| crossed || n.h < h - STEP || n.h > h + STEP)
                        .max_by(|a, b| {
                            // Prefer the surface nearest the agent's height.
                            (b.1.h - h).abs().total_cmp(&(a.1.h - h).abs())
                        })
                        .map(|(l, _)| (j, l));
                    if let Some(to) = land {
                        let th = f.cells[to.0][to.1].h;
                        let w = (dx as f32).hypot(dz as f32) * k2 as f32 * f.cs + (h - th).max(0.0) * 0.25;
                        let c = node_cost(&f, agent, &f.cells[to.0][to.1]);
                        relax(to, w * c + 2.0 * jump_cost.max(0.0), Edge::Jump, &mut heap);
                        break;
                    }
                    // The column in between: anything above the jump height
                    // or no headroom over it stops the jump.
                    let tall = f.solids[j].iter().any(|(lo, hi)| *hi > h + JUMP && *lo < h + JUMP + agent.height);
                    if tall {
                        break;
                    }
                    crossed = true;
                }
            }
        }
        if let Some(cs) = climb_edges.get(&node) {
            for (to, ci) in cs.clone() {
                let rise = (f.cells[to.0][to.1].h - h).abs();
                relax(to, rise * climb_cost.max(0.0) + 1.0, Edge::Climb(ci), &mut heap);
            }
        }
        if let Some(ls) = link_edges.get(&node) {
            for (to, li) in ls.clone() {
                let l = &all_links[li].0;
                let w = l.a.distance(l.b);
                let c = agent.costs.get(&l.label).copied().unwrap_or(1.0);
                if c.is_finite() {
                    relax(to, w * c.max(0.0), Edge::Link(li), &mut heap);
                }
            }
        }
    }
    let (end, status) = if found {
        (g, S_SUCCESS)
    } else if agent.partial && closest.0 != s {
        (closest.0, S_CLOSEST_NO_PATH)
    } else {
        return fail;
    };

    // Back to a node list with the edge used to reach each.
    let mut chain: Vec<((usize, usize), Edge)> = Vec::new();
    let mut at = end;
    loop {
        let (_, prev, e) = best[&at];
        chain.push((at, e));
        if at == s {
            break;
        }
        at = prev;
    }
    chain.reverse();

    // Exact ends: the start's / finish's own x, z on their surface.
    let start_pos = {
        let h = f.cells[s.0][s.1].h;
        let gh = ground(&f, Vec2::new(start.x, start.z), h).filter(|y| (y - h).abs() <= STEP).unwrap_or(h);
        Vec3::new(start.x, gh, start.z)
    };
    let end_pos = if status == S_SUCCESS {
        let h = f.cells[end.0][end.1].h;
        let gh = ground(&f, Vec2::new(finish.x, finish.z), h).filter(|y| (y - h).abs() <= STEP).unwrap_or(h);
        Vec3::new(finish.x, gh, finish.z)
    } else {
        pos(end)
    };

    // Runs of walk edges, split at jumps / links.
    let mut runs: Vec<(Vec<Vec3>, Edge)> = Vec::new();
    let mut cur: Vec<Vec3> = vec![start_pos];
    for (idx, (n, e)) in chain.iter().enumerate().skip(1) {
        let p = if idx == chain.len() - 1 { end_pos } else { pos(*n) };
        match e {
            Edge::Walk => cur.push(p),
            other => {
                runs.push((std::mem::take(&mut cur), *other));
                // A climb's waypoint is on the truss; walking on starts there.
                if let Edge::Climb(ci) = other {
                    cur.push(climb_points[*ci]);
                }
                cur.push(p);
            }
        }
    }
    if chain.len() == 1 {
        cur.push(end_pos);
    }
    runs.push((cur, Edge::Walk));

    let label_at = |p: Vec3| -> String {
        let (cx, cz) = f.cell_of(p);
        f.index(cx, cz)
            .and_then(|i| f.cells[i].iter().min_by(|a, b| (a.h - p.y).abs().total_cmp(&(b.h - p.y).abs())))
            .map(|n| f.label(n).to_string())
            .unwrap_or_default()
    };
    let mut wps: Vec<LPathWaypoint> = Vec::new();
    let spacing = if agent.spacing.is_finite() && agent.spacing > 0.0 { agent.spacing } else { f32::INFINITY };
    let n_runs = runs.len();
    for (ri, (pts, edge_after)) in runs.iter().enumerate() {
        let after_climb = ri > 0 && matches!(runs[ri - 1].1, Edge::Climb(_));
        let corners = string_pull(&f, agent, pts, if after_climb { agent.radius + 2.0 } else { 0.0 });
        for (ci, c) in corners.iter().enumerate() {
            // The first point of a run after a jump / link is that jump's
            // landing, already emitted.
            if ci == 0 && ri > 0 {
                continue;
            }
            wps.push(LPathWaypoint { pos: *c, action: 0, label: label_at(*c) });
            if let Some(next) = corners.get(ci + 1) {
                let seg = *next - *c;
                let len = Vec2::new(seg.x, seg.z).length();
                let mut t = spacing;
                while t < len - 1e-3 {
                    let q = *c + seg * (t / len);
                    let y = ground(&f, Vec2::new(q.x, q.z), q.y).filter(|y| (y - q.y).abs() <= STEP * 2.0).unwrap_or(q.y);
                    let q = Vec3::new(q.x, y, q.z);
                    wps.push(LPathWaypoint { pos: q, action: 0, label: label_at(q) });
                    t += spacing;
                }
            }
        }
        if ri + 1 < n_runs {
            let landing = runs[ri + 1].0[0];
            match edge_after {
                Edge::Jump => wps.push(LPathWaypoint { pos: landing, action: 1, label: "Jump".into() }),
                Edge::Link(li) => wps.push(LPathWaypoint { pos: landing, action: 2, label: all_links[*li].0.label.clone() }),
                Edge::Climb(_) => wps.push(LPathWaypoint { pos: landing, action: 2, label: "Climb".into() }),
                Edge::Walk => {}
            }
        }
    }
    Solved { status, waypoints: wps }
}

/// The straight-line test for string pulling: every cell along a→b has a
/// usable surface within a step of the last.
/// `relax`: within this distance of `a` (a climb's top, on the truss's
/// edge) the ground needn't be clear of edges.
fn line_clear(f: &Field, agent: &Agent, a: Vec3, b: Vec3, relax: f32) -> bool {
    let d = Vec2::new(b.x - a.x, b.z - a.z);
    let len = d.length();
    let n = ((len / (f.cs * 0.5)).ceil() as usize).max(1);
    let mut h = a.y;
    for s in 1..n {
        let p = a + (b - a) * (s as f32 / n as f32);
        let (cx, cz) = f.cell_of(p);
        let Some((i, k)) = f.node_near(cx, cz, h, STEP) else { return false };
        if !usable(f, agent, i, k) && Vec2::new(p.x - a.x, p.z - a.z).length() > relax {
            return false;
        }
        h = f.cells[i][k].h;
    }
    (h - b.y).abs() <= STEP * 1.5
}

fn string_pull(f: &Field, agent: &Agent, pts: &[Vec3], relax_first: f32) -> Vec<Vec3> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    let mut anchor = 0;
    while anchor < pts.len() - 1 {
        let mut far = anchor + 1;
        for j in (anchor + 2..pts.len()).rev() {
            if line_clear(f, agent, pts[anchor], pts[j], if anchor == 0 { relax_first } else { 0.0 }) {
                far = j;
                break;
            }
        }
        out.push(pts[far]);
        anchor = far;
    }
    out
}

/// CheckOcclusionAsync: the first waypoint after `start` (1-based) that is
/// now obstructed, or -1.
fn occlusion(doc: &Document, data: &PathData, start: usize) -> i64 {
    let w = &data.waypoints;
    if w.is_empty() {
        return -1;
    }
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    let (mut ylo, mut yhi) = (f32::MAX, f32::MIN);
    for p in w {
        lo = lo.min(Vec2::new(p.pos.x, p.pos.z));
        hi = hi.max(Vec2::new(p.pos.x, p.pos.z));
        ylo = ylo.min(p.pos.y);
        yhi = yhi.max(p.pos.y);
    }
    let pad = data.agent.radius + 2.0;
    let lo = lo - Vec2::splat(pad);
    let hi = hi + Vec2::splat(pad);
    let cs = ((hi - lo).max_element() / 512.0).max(1.0);
    let f = Field::build(doc, lo, hi, cs, ylo - 8.0, yhi + data.agent.height + 8.0, &data.agent);
    for (i, p) in w.iter().enumerate().skip(start) {
        if f.obstructed(p.pos, &data.agent) {
            return i as i64 + 1;
        }
    }
    -1
}

// ------------------------------------------------------------ Lua surface

pub const METHODS: &[(&str, &str)] = &[
    ("CreatePath", "PathfindingService"),
    ("GetWaypoints", "Path"),
    ("GetPointCoordinates", "Path"),
];

pub const LUA_METHODS: &[(&str, &str)] = &[
    ("FindPathAsync", "PathfindingService"),
    ("ComputeRawPathAsync", "PathfindingService"),
    ("ComputeSmoothPathAsync", "PathfindingService"),
    ("ComputeAsync", "Path"),
    ("CheckOcclusionAsync", "Path"),
];

pub const EVENTS: &[(&str, &str)] = &[("Blocked", "Path"), ("Unblocked", "Path")];

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

fn cast_float(v: &Value) -> mlua::Result<Option<f32>> {
    match v {
        Value::Nil => Ok(None),
        Value::Number(n) => Ok(Some(*n as f32)),
        Value::Integer(n) => Ok(Some(*n as f32)),
        Value::String(_) => Err(rt_err("Unable to cast string to float")),
        Value::Boolean(_) => Err(rt_err("Unable to cast bool to float")),
        _ => Err(rt_err("Unable to cast value to float")),
    }
}

fn cast_bool(v: &Value) -> mlua::Result<Option<bool>> {
    match v {
        Value::Nil => Ok(None),
        Value::Boolean(b) => Ok(Some(*b)),
        Value::String(_) => Err(rt_err("Unable to cast string to bool")),
        _ => Err(rt_err("Unable to cast value to bool")),
    }
}

fn agent_of(v: &Value) -> mlua::Result<Agent> {
    let mut a = Agent::default();
    let Value::Table(t) = v else { return Ok(a) };
    if let Some(x) = cast_float(&t.get::<Value>("AgentRadius")?)? {
        a.radius = x.max(0.0);
    }
    if let Some(x) = cast_float(&t.get::<Value>("AgentHeight")?)? {
        a.height = x.max(0.0);
    }
    if let Some(x) = cast_bool(&t.get::<Value>("AgentCanJump")?)? {
        a.can_jump = x;
    }
    if let Some(x) = cast_bool(&t.get::<Value>("AgentCanClimb")?)? {
        a.can_climb = x;
    }
    if let Some(x) = cast_float(&t.get::<Value>("WaypointSpacing")?)? {
        a.spacing = x;
    }
    if let Some(x) = cast_bool(&t.get::<Value>("SupportPartialPath")?)? {
        a.partial = x;
    }
    if let Value::Table(ps) = t.get::<Value>("PathSettings")? {
        if let Some(x) = cast_bool(&ps.get::<Value>("SupportPartialPath")?)? {
            a.partial = x;
        }
    }
    if let Value::Table(c) = t.get::<Value>("Costs")? {
        for pair in c.pairs::<Value, Value>() {
            let (k, v) = pair?;
            let key = match &k {
                Value::String(s) => s.to_str()?.to_string(),
                Value::UserData(_) => ud_of::<LEnumItem>(&k).map(|e| e.name).unwrap_or_default(),
                _ => continue,
            };
            if let Some(x) = cast_float(&v)? {
                a.costs.insert(key, x);
            }
        }
    }
    Ok(a)
}

fn new_path(lua: &Lua, agent: Agent) -> mlua::Result<Ref> {
    let r = crate::runtime::ext::create_any(lua, "Path")?;
    if let Some(i) = world(lua)?.dm().dom.get_by_ref_mut(r) {
        i.name = "Instance".to_string();
    }
    paths(lua, |p| {
        p.insert(r, PathData { agent, status: S_NO_PATH, waypoints: Vec::new(), blocked: None, checked_rev: 0, next_check: 0.0 })
    });
    Ok(r)
}

fn v3(v: &Value) -> Option<Vec3> {
    ud_of::<LVector3>(v).map(|v| v.0)
}

fn compute(lua: &Lua, r: Ref, start: Vec3, finish: Vec3) -> mlua::Result<()> {
    let agent = paths(lua, |p| p.get(&r).map(|d| d.agent.clone())).unwrap_or_default();
    if start.distance(finish) > MAX_REQUEST {
        paths(lua, |p| {
            if let Some(d) = p.get_mut(&r) {
                d.status = S_NO_PATH;
                d.waypoints.clear();
            }
        });
        return Err(rt_err("PathfindingService: path request is too long."));
    }
    let (solved, rev) = {
        let w = world(lua)?;
        let d = w.d();
        (solve(d, &agent, start, finish), d.revision)
    };
    paths(lua, |p| {
        if let Some(d) = p.get_mut(&r) {
            d.status = solved.status;
            d.waypoints = solved.waypoints;
            d.blocked = None;
            d.checked_rev = rev;
        }
    });
    Ok(())
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([v]))
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if !METHODS.iter().any(|(m, _)| *m == name) {
        return None;
    }
    let class = world(lua).ok()?.d().class_of(this)?;
    Some((|| match (class.as_str(), name) {
        ("PathfindingService", "CreatePath") => {
            let a = agent_of(&arg(args, 1))?;
            one(inst_value(lua, new_path(lua, a)?)?)
        }
        ("Path", "GetWaypoints") => {
            let w = paths(lua, |p| p.get(&this).map(|d| d.waypoints.clone())).unwrap_or_default();
            let t = lua.create_table()?;
            for (i, x) in w.into_iter().enumerate() {
                t.raw_set(i + 1, x)?;
            }
            one(Value::Table(t))
        }
        ("Path", "GetPointCoordinates") => {
            let w = paths(lua, |p| p.get(&this).map(|d| d.waypoints.clone())).unwrap_or_default();
            let t = lua.create_table()?;
            for (i, x) in w.into_iter().enumerate() {
                t.raw_set(i + 1, LVector3(x.pos))?;
            }
            one(Value::Table(t))
        }
        _ => Ok(MultiValue::new()),
    })())
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if class == "Path" && key == "Status" {
        let s = paths(lua, |p| p.get(&r).map(|d| d.status)).unwrap_or(S_NO_PATH);
        let item = LEnumItem::from_value("PathStatus", s).ok_or_else(|| rt_err("PathStatus"))?;
        return Ok(Some(enum_value(lua, item)?));
    }
    Ok(None)
}

/// `__geo_nav(name, self, ...)`: the yielding methods' work (the Luau side
/// yields a frame after it, as Studio's resume on a later step).
fn nav_call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    match name {
        "FindPathAsync" | "ComputeRawPathAsync" | "ComputeSmoothPathAsync" => {
            let s = v3(&arg(args, 0)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let e = v3(&arg(args, 1)).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            if name != "FindPathAsync" {
                let max = cast_float(&arg(args, 2))?.ok_or_else(|| rt_err("Argument 3 missing or nil"))?;
                if s.distance(e) > max.min(MAX_REQUEST) {
                    return Err(rt_err("PathfindingService: path request is too long."));
                }
            }
            let r = new_path(lua, Agent::default())?;
            compute(lua, r, s, e)?;
            one(inst_value(lua, r)?)
        }
        "ComputeAsync" => {
            let s = v3(&arg(args, 0)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let e = v3(&arg(args, 1)).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            compute(lua, this, s, e)?;
            Ok(MultiValue::new())
        }
        "CheckOcclusionAsync" => {
            let start = cast_float(&arg(args, 0))?.ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let data = paths(lua, |p| p.get(&this).cloned()).ok_or_else(|| rt_err("Start point value is invalid"))?;
            if start < 1.0 || start as usize > data.waypoints.len() {
                return Err(rt_err("Start point value is invalid"));
            }
            let w = world(lua)?;
            let v = occlusion(w.d(), &data, start as usize);
            one(Value::Number(v as f64))
        }
        _ => Ok(MultiValue::new()),
    }
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let pw = lua.create_table()?;
    pw.set(
        "new",
        lua.create_function(|lua, (p, a, l): (Value, Value, Value)| {
            // Studio: nil is the origin, anything else must be a Vector3.
            let pos = match (&p, v3(&p)) {
                (_, Some(v)) => v,
                (Value::Nil, None) => Vec3::ZERO,
                (other, None) => {
                    return Err(crate::cmeta::located(format!(
                        "invalid argument #1 to 'new' (Vector3 expected, got {})",
                        crate::cmeta::type_name(other)
                    )))
                }
            };
            let action = ud_of::<LEnumItem>(&a).filter(|e| e.enum_type == "PathWaypointAction").map(|e| e.value).unwrap_or(0);
            let label = match l {
                Value::String(s) => s.to_str()?.to_string(),
                _ => String::new(),
            };
            lua.create_userdata(LPathWaypoint { pos, action, label })
        })?,
    )?;
    g.set("PathWaypoint", pw)?;
    g.set(
        "__geo_nav",
        lua.create_function(|lua, (name, this, rest): (String, Value, MultiValue)| {
            let r = match &this {
                Value::UserData(_) => opt_inst(&this),
                _ => None,
            }
            .ok_or_else(|| rt_err(format!("Expected ':' not '.' calling member function {name}")))?;
            // (true, results…) or (false, Studio's message) for the Luau side.
            Ok(match nav_call(lua, &name, r, &rest) {
                Ok(v) => std::iter::once(Value::Boolean(true)).chain(v).collect::<MultiValue>(),
                Err(e) => MultiValue::from_iter([Value::Boolean(false), Value::String(lua.create_string(super::plain_error(&e))?)]),
            })
        })?,
    )?;
    lua.load(
        r#"
local M = __rs_lua_methods
local nav = __geo_nav
for _, name in {"FindPathAsync", "ComputeRawPathAsync", "ComputeSmoothPathAsync", "ComputeAsync", "CheckOcclusionAsync"} do
    M[name] = function(self, ...)
        local r = table.pack(nav(name, self, ...))
        task.wait()
        if not r[1] then error(r[2], 0) end
        return table.unpack(r, 2, r.n)
    end
end
"#,
    )
    .set_name("geometry_nav")
    .exec()?;
    Ok(())
}

/// Blocked / Unblocked: computed paths are re-checked (every half second,
/// when the world changed) and fire on a change of their first obstructed
/// waypoint.
pub fn step(lua: &Lua, _dt: f64) -> mlua::Result<()> {
    let (rev, clock) = {
        let w = world(lua)?;
        (w.d().revision, w.time)
    };
    let due: Vec<(Ref, PathData)> = paths(lua, |p| {
        p.iter()
            .filter(|(_, d)| d.status == S_SUCCESS && !d.waypoints.is_empty() && d.checked_rev != rev && d.next_check <= clock)
            .map(|(r, d)| (*r, d.clone()))
            .take(8)
            .collect()
    });
    for (r, d) in due {
        let exists = world(lua)?.d().exists(r);
        if !exists {
            paths(lua, |p| p.remove(&r));
            continue;
        }
        let occ = {
            let w = world(lua)?;
            occlusion(w.d(), &d, 0)
        };
        let now_blocked = (occ > 0).then_some(occ as usize);
        paths(lua, |p| {
            if let Some(x) = p.get_mut(&r) {
                x.checked_rev = rev;
                x.next_check = clock + 0.5;
                x.blocked = now_blocked.or(x.blocked.filter(|_| false));
            }
        });
        match (d.blocked, now_blocked) {
            (None, Some(i)) => world(lua)?.fire(r, "Blocked", vec![Value::Number(i as f64)]),
            (Some(i), None) => world(lua)?.fire(r, "Unblocked", vec![Value::Number(i as f64)]),
            (Some(a), Some(b)) if a != b => {
                world(lua)?.fire(r, "Unblocked", vec![Value::Number(a as f64)]);
                world(lua)?.fire(r, "Blocked", vec![Value::Number(b as f64)]);
            }
            _ => {}
        }
    }
    Ok(())
}
