//! BasePart physics (mass, assemblies, network ownership, impulses), Model
//! (ScaleTo, identity orientation, persistent players), WorldRoot
//! (ArePartsTouchingOthers, IsRegion3Empty) and Workspace members, as
//! Studio 0.705 does them (measured in an offline Studio; numbers in
//! checks/parity/world.md).
//!
//! Mass: PhysicalProperties density (CustomPhysicalProperties, else the
//! material's) × the shape's volume (Ball: the smallest side is the
//! diameter; Cylinder: X long, min(Y, Z) across; Wedge ½, CornerWedge ⅓ of
//! the box; MeshParts / unions their box), each side at least 0.05 stud. A
//! Massless part weighs nothing unless it is its assembly's root.
//! Assemblies: parts joined by enabled rigid joints (JointInstances other
//! than the Rotate* / VelocityMotor hinges, WeldConstraints) under
//! Workspace. Root: an anchored part is its own root (and every unanchored
//! part joined to one takes the first anchored part); otherwise
//! Massless=false first, then RootPriority, then a part named
//! HumanoidRootPart, then the largest volume (ties: the longest side).
//! Inertia is the shape's (box for wedges) about the assembly's centre of
//! mass; AngularAccelerationToTorque = I·α + ω×(I·ω), as Studio returns.
//!
//! Network ownership: no clients own anything in a Run session; the API
//! keeps Studio's checks and messages, remembers SetNetworkOwner /
//! SetNetworkOwnershipAuto per assembly, and a player's character answers
//! its player under automatic ownership; loose assemblies go to the closest
//! player within its simulation radius (`auto_owner`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use glam::{Mat3, Vec3};
use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::ustr;

use super::util::*;
use crate::doc::{self, Document};
use crate::ltypes::*;
use crate::math::Xform;
use crate::runtime::*;

// ------------------------------------------------------------------ mass

/// The PhysicalProperties a part simulates with.
pub fn phys_props(d: &Document, r: Ref) -> LPhysicalProperties {
    if let Some(Variant::PhysicalProperties(rbx_dom_weak::types::PhysicalProperties::Custom(c))) =
        d.prop(r, "CustomPhysicalProperties")
    {
        let mut p = LPhysicalProperties::for_material("Plastic");
        p.density = c.density();
        p.friction = c.friction();
        p.elasticity = c.elasticity();
        p.friction_weight = c.friction_weight();
        p.elasticity_weight = c.elasticity_weight();
        return p;
    }
    let mat = crate::scene::enum_u32(d.prop(r, "Material"))
        .and_then(|v| LEnumItem::from_value("Material", v))
        .map(|e| e.name)
        .unwrap_or_else(|| "Plastic".into());
    LPhysicalProperties::for_material(&mat)
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Block,
    Ball,
    Cylinder,
    Wedge,
    CornerWedge,
}

fn shape(d: &Document, r: Ref, class: &str) -> Shape {
    match class {
        "WedgePart" => return Shape::Wedge,
        "CornerWedgePart" => return Shape::CornerWedge,
        "Part" | "Seat" | "SpawnLocation" | "SkateboardPlatform" | "VehicleSeat" | "Platform" => {}
        _ => return Shape::Block,
    }
    match crate::scene::enum_u32(d.prop(r, "Shape")) {
        Some(0) => Shape::Ball,
        Some(2) => Shape::Cylinder,
        Some(3) => Shape::Wedge,
        Some(4) => Shape::CornerWedge,
        _ => Shape::Block,
    }
}

/// One part's own mass (Massless ignored), centre of mass in its frame and
/// principal inertia about that centre (its axes).
#[derive(Clone, Copy)]
pub struct PMass {
    pub mass: f32,
    pub com: Vec3,
    pub inertia: Vec3,
}

pub fn part_mass(d: &Document, r: Ref) -> PMass {
    let class = d.class_of(r).map(|c| c.to_string()).unwrap_or_default();
    if class == "Terrain" {
        return PMass { mass: 0.0, com: Vec3::ZERO, inertia: Vec3::ZERO };
    }
    let s = v3p(d, r, "Size").unwrap_or(Vec3::ONE).max(Vec3::splat(0.05));
    let rho = phys_props(d, r).density;
    let box_i = |m: f32, s: Vec3| {
        Vec3::new(s.y * s.y + s.z * s.z, s.x * s.x + s.z * s.z, s.x * s.x + s.y * s.y) * (m / 12.0)
    };
    match shape(d, r, &class) {
        Shape::Block => {
            let m = rho * s.x * s.y * s.z;
            PMass { mass: m, com: Vec3::ZERO, inertia: box_i(m, s) }
        }
        Shape::Ball => {
            let rr = s.min_element() * 0.5;
            let m = rho * 4.0 / 3.0 * std::f32::consts::PI * rr * rr * rr;
            PMass { mass: m, com: Vec3::ZERO, inertia: Vec3::splat(0.4 * m * rr * rr) }
        }
        Shape::Cylinder => {
            let rr = s.y.min(s.z) * 0.5;
            let m = rho * std::f32::consts::PI * rr * rr * s.x;
            let side = m * (3.0 * rr * rr + s.x * s.x) / 12.0;
            PMass { mass: m, com: Vec3::ZERO, inertia: Vec3::new(0.5 * m * rr * rr, side, side) }
        }
        Shape::Wedge => {
            let m = rho * s.x * s.y * s.z * 0.5;
            PMass { mass: m, com: Vec3::new(0.0, -s.y / 6.0, s.z / 6.0), inertia: box_i(m, s) }
        }
        Shape::CornerWedge => {
            let m = rho * s.x * s.y * s.z / 3.0;
            PMass { mass: m, com: Vec3::new(s.x / 8.0, -s.y / 4.0, -s.z / 8.0), inertia: box_i(m, s) }
        }
    }
}

// ------------------------------------------------------------- assemblies

#[derive(Default)]
pub struct Graph {
    /// Part → its group (rigidly joined parts).
    group: HashMap<Ref, usize>,
    groups: Vec<Vec<Ref>>,
    /// Part → parts it is rigidly joined to directly.
    rigid: HashMap<Ref, Vec<Ref>>,
    /// Part → joints and constraints naming it (GetJoints).
    joints: HashMap<Ref, Vec<Ref>>,
    /// Group → its root and whether it holds an anchored part.
    roots: Vec<(Ref, bool)>,
    /// Joints and constraints that made an edge or named a part, and the
    /// attachments constraints name (moving one moves the constraint).
    joint_set: HashSet<Ref>,
}

fn rigid_joint(class: &str) -> bool {
    class == "WeldConstraint"
        || doc::is_a(class, "JointInstance")
            && !matches!(class, "Rotate" | "RotateP" | "RotateV" | "VelocityMotor")
}

fn anchored(d: &Document, r: Ref) -> bool {
    boolp(d, r, "Anchored", false) || d.class_of(r).is_some_and(|c| c == "Terrain")
}

fn build(d: &Document) -> Graph {
    let _b = crate::bench::scope("joint_graph");
    let mut g = Graph::default();
    let Some(ws) = d.workspace() else { return g };
    // Per class: 0 other, 1 part, 2 joint, 3 constraint (classes repeat; is_a interns its names).
    let mut kinds: rustc_hash::FxHashMap<rbx_dom_weak::Ustr, u8> = Default::default();
    let mut edges: Vec<(Ref, Ref)> = Vec::new();
    for inst in d.dom.descendants_of(ws) {
        let class = inst.class;
        let kind = *kinds.entry(class).or_insert_with(|| {
            let c = class.as_str();
            if doc::is_a(c, "BasePart") {
                1
            } else if rigid_joint(c) || doc::is_a(c, "JointInstance") {
                2
            } else if doc::is_a(c, "Constraint") && c != "NoCollisionConstraint" {
                3
            } else {
                0
            }
        });
        if kind < 2 {
            continue;
        }
        let r = inst.referent();
        let class = class.as_str();
        let pair = if kind == 2 {
            (refp(d, r, "Part0"), refp(d, r, "Part1"))
        } else {
            for k in ["Attachment0", "Attachment1"] {
                if let Some(a) = refp(d, r, k) {
                    g.joint_set.insert(a);
                }
            }
            let pp = |k: &str| refp(d, r, k).and_then(|a| d.dom.get_by_ref(a)).map(|a| a.parent());
            (pp("Attachment0"), pp("Attachment1"))
        };
        for p in [pair.0, pair.1].into_iter().flatten() {
            if is_part(d, p) {
                g.joints.entry(p).or_default().push(r);
                g.joint_set.insert(r);
            }
        }
        if let (Some(a), Some(b)) = pair {
            if a != b
                && rigid_joint(class)
                && boolp(d, r, "Enabled", true)
                && is_part(d, a)
                && is_part(d, b)
                && d.is_ancestor(ws, a)
                && d.is_ancestor(ws, b)
            {
                edges.push((a, b));
                g.rigid.entry(a).or_default().push(b);
                g.rigid.entry(b).or_default().push(a);
            }
        }
    }
    // Union-find over the rigid edges. Parts no rigid joint names are
    // single-part assemblies: the lookups' fallback, so they get no entry.
    let mut parts: Vec<Ref> = Vec::new();
    let mut idx: HashMap<Ref, usize> = HashMap::new();
    let mut id_of = |r: Ref, parts: &mut Vec<Ref>| {
        *idx.entry(r).or_insert_with(|| {
            parts.push(r);
            parts.len() - 1
        })
    };
    let edges: Vec<(usize, usize)> = edges.into_iter().map(|(a, b)| (id_of(a, &mut parts), id_of(b, &mut parts))).collect();
    let mut up: Vec<usize> = (0..parts.len()).collect();
    fn find(up: &mut [usize], mut i: usize) -> usize {
        while up[i] != i {
            up[i] = up[up[i]];
            i = up[i];
        }
        i
    }
    for (ia, ib) in edges {
        let (ra, rb) = (find(&mut up, ia), find(&mut up, ib));
        if ra != rb {
            up[ra] = rb;
        }
    }
    let mut slot: HashMap<usize, usize> = HashMap::new();
    for (i, r) in parts.iter().enumerate() {
        let root = find(&mut up, i);
        let gi = *slot.entry(root).or_insert_with(|| {
            g.groups.push(Vec::new());
            g.groups.len() - 1
        });
        g.groups[gi].push(*r);
        g.group.insert(*r, gi);
    }
    for members in &g.groups {
        g.roots.push(pick_root(d, members));
    }
    g
}

fn pick_root(d: &Document, members: &[Ref]) -> (Ref, bool) {
    if let Some(a) = members.iter().find(|r| anchored(d, **r)) {
        return (*a, true);
    }
    let key = |r: Ref| {
        let s = v3p(d, r, "Size").unwrap_or(Vec3::ONE);
        let hrp = d.dom.get_by_ref(r).is_some_and(|i| i.name == "HumanoidRootPart");
        (
            !boolp(d, r, "Massless", false),
            crate::scene::f32_of(d.prop(r, "RootPriority")).unwrap_or(0.0) as i32,
            hrp,
            s.x * s.y * s.z,
            s.max_element(),
        )
    };
    let mut best = members[0];
    let mut bk = key(best);
    for r in &members[1..] {
        let k = key(*r);
        let better = (k.0, k.1, k.2) > (bk.0, bk.1, bk.2)
            || (k.0, k.1, k.2) == (bk.0, bk.1, bk.2)
                && (k.3 > bk.3 + 1e-6 || (k.3 - bk.3).abs() <= 1e-6 && k.4 > bk.4);
        if better {
            best = *r;
            bk = k;
        }
    }
    (best, false)
}

thread_local! {
    /// Document id → its joint graph (server and client documents share a thread).
    static GRAPHS: RefCell<Vec<(u64, Graph)>> = const { RefCell::new(Vec::new()) };
}

/// Properties that change which parts are joined, or which part is a root.
fn structural_prop(name: &str) -> bool {
    matches!(
        name,
        "Part0" | "Part1" | "Attachment0" | "Attachment1" | "Enabled" | "Anchored" | "Massless" | "RootPriority" | "Parent"
    )
}

impl Graph {
    fn in_multi(&self, r: Ref) -> bool {
        self.group.get(&r).is_some_and(|gi| self.groups[*gi].len() > 1)
    }

    /// Whether the journal marks since the last look can change the graph.
    /// Moving parts (CFrame, Velocity, ...) can't; new parts with no joints are
    /// single-part assemblies (the lookups' fallback), so they don't either.
    fn stale(&self, d: &Document, marks: &[doc::Mark]) -> bool {
        for m in marks {
            match *m {
                doc::Mark::Prop(r, name) => {
                    let n = name.as_str();
                    // (a part no rigid joint names is its own assembly whatever these say)
                    if structural_prop(n) && (self.joint_set.contains(&r) || self.group.contains_key(&r)) {
                        return true;
                    }
                    // the root pick weighs these among joined parts
                    if matches!(n, "Size" | "Name") && self.in_multi(r) {
                        return true;
                    }
                    if structural_prop(n) && d.class_of(r).is_some_and(|c| joint_class(c.as_str())) {
                        return true;
                    }
                }
                doc::Mark::Tree(r) => {
                    if self.joint_set.contains(&r) || self.in_multi(r) {
                        return true;
                    }
                    for x in d.subtree(r) {
                        if self.joint_set.contains(&x) || self.in_multi(x) {
                            return true;
                        }
                        // (a removed instance's class is in limbo; one freed outright is unknown here
                        // and was neither a known joint nor a joined part)
                        if d.class_of(x).is_some_and(|c| joint_class(c.as_str())) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

fn joint_class(class: &str) -> bool {
    doc::is_a(class, "JointInstance")
        || class == "WeldConstraint"
        || (doc::is_a(class, "Constraint") && class != "NoCollisionConstraint")
}

/// Runs `f` on the joint graph of `d`, rebuilt only when the document's
/// change journal holds a change that can alter it.
pub fn with_graph<R>(d: &Document, f: impl FnOnce(&Graph) -> R) -> R {
    GRAPHS.with(|gs| {
        let mut gs = gs.borrow_mut();
        let marks = d.graph_marks();
        let i = match gs.iter().position(|(id, _)| *id == d.id) {
            Some(i) => {
                let rebuild = match &marks {
                    doc::Marks::All => true,
                    doc::Marks::Some(m) => gs[i].1.stale(d, m),
                };
                if rebuild {
                    gs[i].1 = build(d);
                }
                i
            }
            None => {
                if gs.len() >= 4 {
                    gs.remove(0);
                }
                gs.push((d.id, build(d)));
                gs.len() - 1
            }
        };
        f(&gs[i].1)
    })
}

impl Graph {
    /// The parts rigidly joined with `r` (itself included).
    pub fn assembly(&self, r: Ref) -> Vec<Ref> {
        match self.group.get(&r) {
            Some(gi) => self.groups[*gi].clone(),
            None => vec![r],
        }
    }
    pub fn root(&self, d: &Document, r: Ref) -> Ref {
        if anchored(d, r) {
            return r;
        }
        self.group.get(&r).map(|gi| self.roots[*gi].0).unwrap_or(r)
    }
    pub fn grounded(&self, d: &Document, r: Ref) -> bool {
        anchored(d, r) || self.group.get(&r).is_some_and(|gi| self.roots[*gi].1)
    }
    pub fn joints_of(&self, r: Ref) -> Vec<Ref> {
        self.joints.get(&r).cloned().unwrap_or_default()
    }
}

/// A part's effective mass (0 for a Massless part that isn't the root).
fn eff_mass(d: &Document, g: &Graph, r: Ref) -> f32 {
    if boolp(d, r, "Massless", false) && g.root(d, r) != r && !anchored(d, r) {
        return 0.0;
    }
    part_mass(d, r).mass
}

/// Assembly mass, centre of mass (world) and world inertia about it.
pub fn assembly_mass(d: &Document, r: Ref) -> (f32, Vec3, Mat3) {
    with_graph(d, |g| {
        let parts = g.assembly(r);
        let mut m = 0.0;
        let mut c = Vec3::ZERO;
        let mut info = Vec::new();
        for p in &parts {
            let pm = part_mass(d, *p);
            let em = eff_mass(d, g, *p);
            let x = xf(d, *p, "CFrame").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY });
            let wc = x.apply(pm.com);
            m += em;
            c += wc * em;
            info.push((em, pm, x, wc));
        }
        let com = if m > 0.0 {
            c / m
        } else {
            xf(d, r, "CFrame").map(|x| x.pos).unwrap_or(Vec3::ZERO)
        };
        let mut i = Mat3::ZERO;
        for (em, pm, x, wc) in info {
            if em <= 0.0 || pm.mass <= 0.0 {
                continue;
            }
            let k = em / pm.mass;
            let local = x.rot * Mat3::from_diagonal(pm.inertia * k) * x.rot.transpose();
            let o = wc - com;
            let shift = (Mat3::IDENTITY * o.length_squared()) - outer(o, o);
            i += local + shift * em;
        }
        (m, com, i)
    })
}

fn outer(a: Vec3, b: Vec3) -> Mat3 {
    Mat3::from_cols(a * b.x, a * b.y, a * b.z)
}

// ------------------------------------------------------ network ownership

#[derive(Clone, Copy)]
struct Owner {
    player: Ref,
    auto: bool,
}

thread_local! {
    /// (document id, assembly root) → what SetNetworkOwner* last said.
    static OWNERS: RefCell<HashMap<(u64, Ref), Owner>> = RefCell::new(HashMap::new());
    /// (document id, model) → SetIdentityOrientation's rotation.
    static IDENTITY: RefCell<HashMap<(u64, Ref), Mat3>> = RefCell::new(HashMap::new());
}

const NO_WS: &str = "Can only call Network Ownership API on a part that is descendent of Workspace";
const NO_ANCHORED: &str =
    "Network Ownership API cannot be called on Anchored parts or parts welded to Anchored parts.";

/// Studio's checks before any ownership call: Err(message).
fn ownership_check(d: &Document, r: Ref) -> Result<Ref, &'static str> {
    if !in_workspace(d, r) {
        return Err(NO_WS);
    }
    with_graph(d, |g| if g.grounded(d, r) { Err(NO_ANCHORED) } else { Ok(g.root(d, r)) })
}

thread_local! {
    /// (document id, player) → the run time it was first seen with a character.
    static JOINED: RefCell<HashMap<(u64, Ref), f64>> = RefCell::new(HashMap::new());
}

/// Automatic ownership (Studio, Play): each player simulates loose
/// assemblies within its SimulationRadius of its character, the closest
/// player winning; the radius grows from 0 after joining to
/// MaximumSimulationRadius (1000). Studio's growth adapts to the client's
/// connection (50 studs reached 16.7 s and 27.5 s after joining in two
/// runs, 100 at 30.8 s, 1000 by 152 s): rustudio grows it 4 studs/s.
fn auto_owner(w: &World, d: &Document, root: Ref) -> Option<Ref> {
    let at = xf(d, root, "CFrame")?.pos;
    let players = d.find_child(d.dom.root_ref(), "Players")?;
    let now = w.time;
    let mut best: Option<(f32, Ref)> = None;
    for p in d.dom.get_by_ref(players)?.children() {
        let Some(c) = refp(d, *p, "Character") else { continue };
        let Some(hrp) = d.dom.get_by_ref(c).and_then(|i| i.children().iter().copied().find(|k| d.dom.get_by_ref(*k).is_some_and(|x| x.name == "HumanoidRootPart"))) else { continue };
        let Some(hx) = xf(d, hrp, "CFrame") else { continue };
        let since = JOINED.with(|j| *j.borrow_mut().entry((d.id, *p)).or_insert(now));
        let radius = ((now - since) * 4.0).min(1000.0) as f32;
        let dist = (hx.pos - at).length();
        if dist <= radius && best.is_none_or(|b| dist < b.0) {
            best = Some((dist, *p));
        }
    }
    best.map(|b| b.1)
}

/// The player whose character holds `r`, if any.
fn character_owner(d: &Document, r: Ref) -> Option<Ref> {
    let players = d.find_child(d.dom.root_ref(), "Players")?;
    for p in d.dom.get_by_ref(players)?.children() {
        if let Some(c) = refp(d, *p, "Character") {
            if d.is_ancestor(c, r) {
                return Some(*p);
            }
        }
    }
    None
}

// ------------------------------------------------------------------ hooks

pub fn call(lua: &Lua, name: &str, this: Ref, class: &str, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let part = doc::is_a(class, "BasePart");
    Some(match name {
        "GetMass" if part => {
            let m = {
                let w = world(lua).ok()?;
                let d = w.d();
                with_graph(d, |g| eff_mass(d, g, this))
            };
            one(Value::Number(m as f64))
        }
        "AngularAccelerationToTorque" | "TorqueToAngularAcceleration" if part => {
            (|| {
                let v = vec3_arg(args, 1)?;
                let wv = match arg(args, 2) {
                    Value::Nil => Vec3::ZERO,
                    x => ud::<LVector3>(&x).map(|v| v.0).ok_or_else(|| rt_err("Unable to cast value to Vector3"))?,
                };
                let (grounded, inertia) = {
                    let w = world(lua)?;
                    let d = w.d();
                    (with_graph(d, |g| g.grounded(d, this)), assembly_mass(d, this).2)
                };
                let gyro = wv.cross(inertia * wv);
                let out = if name == "AngularAccelerationToTorque" {
                    if grounded { Vec3::splat(f32::INFINITY) } else { inertia * v + gyro }
                } else if grounded {
                    Vec3::ZERO
                } else {
                    inertia.inverse() * (v - gyro)
                };
                one(vec3(lua, out)?)
            })()
        }
        "IsGrounded" if part => {
            let g = {
                let w = world(lua).ok()?;
                let d = w.d();
                with_graph(d, |g| g.grounded(d, this))
            };
            one(Value::Boolean(g))
        }
        "GetRootPart" if part => {
            let r = {
                let w = world(lua).ok()?;
                let d = w.d();
                with_graph(d, |g| g.root(d, this))
            };
            inst_value(lua, r).and_then(one)
        }
        "GetConnectedParts" if part => {
            let recursive = matches!(arg(args, 1), Value::Boolean(true));
            let list_refs = {
                let w = world(lua).ok()?;
                let d = w.d();
                with_graph(d, |g| {
                    if recursive {
                        g.assembly(this)
                    } else {
                        let mut v = vec![this];
                        for p in g.rigid.get(&this).into_iter().flatten() {
                            if !v.contains(p) {
                                v.push(*p);
                            }
                        }
                        v
                    }
                })
            };
            list(lua, list_refs).map(Value::Table).and_then(one)
        }
        "GetJoints" if part => {
            let js = {
                let w = world(lua).ok()?;
                let d = w.d();
                with_graph(d, |g| g.joints_of(this))
            };
            list(lua, js).map(Value::Table).and_then(one)
        }
        "ApplyImpulse" | "ApplyAngularImpulse" | "ApplyImpulseAtPosition" if part => {
            (|| {
                let j = vec3_arg(args, 1)?;
                let at = if name == "ApplyImpulseAtPosition" { Some(vec3_arg(args, 2)?) } else { None };
                let mut w = world(lua)?;
                let (m, com, inertia, grounded, root, doc_id) = {
                    let d = w.d();
                    let (m, com, i) = assembly_mass(d, this);
                    let (gr, root) = with_graph(d, |g| (g.grounded(d, this), g.root(d, this)));
                    (m, com, i, gr, root, d.id)
                };
                if grounded || m <= 0.0 {
                    return none();
                }
                let (lin, ang) = match (name, at) {
                    ("ApplyAngularImpulse", _) => (Vec3::ZERO, j),
                    (_, Some(p)) => (j, (p - com).cross(j)),
                    _ => (j, Vec3::ZERO),
                };
                if lin != Vec3::ZERO {
                    // Pending requests this frame stack, as impulses do.
                    let v = w
                        .velocity_requests
                        .iter()
                        .rev()
                        .find(|(r, _)| *r == root || *r == this)
                        .map(|(_, v)| *v)
                        .or_else(|| w.velocities.get(&this).copied())
                        .unwrap_or(Vec3::ZERO);
                    w.velocity_requests.push((this, v + lin / m));
                }
                if ang != Vec3::ZERO && inertia.determinant().abs() > 1e-12 {
                    crate::physics::live::queue_spin(doc_id, this, inertia.inverse() * ang);
                }
                none()
            })()
        }
        "GetVelocityAtPosition" if part => {
            (|| {
                let p = vec3_arg(args, 1)?;
                let w = world(lua)?;
                let d = w.d();
                let v = w.velocities.get(&this).copied().unwrap_or(Vec3::ZERO);
                let (spin, com) = crate::physics::live::with_stats(d.id, |s| s.spin.get(&this).copied())
                    .flatten()
                    .unwrap_or((Vec3::ZERO, Vec3::ZERO));
                one(vec3(lua, v + spin.cross(p - com))?)
            })()
        }
        "SetNetworkOwner" if part => {
            (|| {
                let who = match arg(args, 1) {
                    Value::Nil => Ref::none(),
                    v => opt_inst(&v).ok_or_else(|| rt_err("Unable to cast value to Object"))?,
                };
                let w = world(lua)?;
                let d = w.d();
                let root = ownership_check(d, this).map_err(rt_err)?;
                let player = if d.class_of(who).is_some_and(|c| c == "Player") { who } else { Ref::none() };
                OWNERS.with(|o| o.borrow_mut().insert((d.id, root), Owner { player, auto: false }));
                none()
            })()
        }
        "SetNetworkOwnershipAuto" if part => {
            (|| {
                let w = world(lua)?;
                let d = w.d();
                let root = ownership_check(d, this).map_err(rt_err)?;
                OWNERS.with(|o| o.borrow_mut().insert((d.id, root), Owner { player: Ref::none(), auto: true }));
                none()
            })()
        }
        "GetNetworkOwner" | "GetNetworkOwnershipAuto" if part => {
            (|| {
                let (owner, doc_id, root) = {
                    let w = world(lua)?;
                    let d = w.d();
                    let root = ownership_check(d, this).map_err(rt_err)?;
                    let set = OWNERS.with(|o| o.borrow().get(&(d.id, root)).copied());
                    let owner = match set {
                        Some(o) if !o.auto => o.player,
                        _ => character_owner(d, this)
                            .or_else(|| auto_owner(&w, d, root))
                            .unwrap_or_else(Ref::none),
                    };
                    (owner, d.id, root)
                };
                if name == "GetNetworkOwner" {
                    inst_value(lua, owner).and_then(one)
                } else {
                    let auto = OWNERS.with(|o| o.borrow().get(&(doc_id, root)).is_none_or(|o| o.auto));
                    one(Value::Boolean(auto))
                }
            })()
        }
        "CanSetNetworkOwnership" if part => {
            let w = world(lua).ok()?;
            let r = match ownership_check(w.d(), this) {
                Ok(_) => Ok(MultiValue::from_iter([Value::Boolean(true), Value::Nil])),
                Err(m) => lua
                    .create_string(m)
                    .map(|s| MultiValue::from_iter([Value::Boolean(false), Value::String(s)])),
            };
            r
        }
        // A connection that Studio never calls back in a Run session
        // (collision summaries come from replicated client contacts).
        "BindToCollisionSummaries" if part => (|| {
            let f = match arg(args, 1) {
                Value::Function(f) => f,
                Value::Nil => return Err(missing(1)),
                _ => return Err(rt_err("Unable to cast value to function")),
            };
            let sig: mlua::Function = lua.globals().get("__rs_new_signal")?;
            let s: mlua::AnyUserData = sig.call((inst_value(lua, this)?, "__CollisionSummaries"))?;
            let connect: mlua::Function = lua.load("return function(s, f) return s:Connect(f) end").eval()?;
            let conn: Value = connect.call((s, f))?;
            one(conn)
        })(),

        // ------------------------------------------------------- Model
        "ScaleTo" if doc::is_a(class, "Model") => scale_to(lua, this, args),
        "GetScale" if doc::is_a(class, "Model") => {
            let s = world(lua).ok()?.d().prop(this, "Scale");
            one(Value::Number(crate::scene::f32_of(s).unwrap_or(1.0) as f64))
        }
        "SetIdentityOrientation" if doc::is_a(class, "Model") => (|| {
            let w = world(lua)?;
            let d = w.d();
            let rot = model_pivot(&w, this).rot;
            IDENTITY.with(|m| m.borrow_mut().insert((d.id, this), rot));
            none()
        })(),
        "ResetOrientationToIdentity" if doc::is_a(class, "Model") => (|| {
            let (id, p) = {
                let w = world(lua)?;
                let id = IDENTITY.with(|m| m.borrow().get(&(w.d().id, this)).copied());
                (id, model_pivot(&w, this))
            };
            if let Some(rot) = id {
                pivot_to(lua, this, Xform { pos: p.pos, rot })?;
            }
            none()
        })(),
        // Studio (no streaming in Edit / Run): the player argument is
        // checked first and only a Player passes; there is nobody to keep.
        "AddPersistentPlayer" | "RemovePersistentPlayer" if doc::is_a(class, "Model") => {
            let ok = opt_inst(&arg(args, 1))
                .is_some_and(|p| world(lua).is_ok_and(|w| w.d().class_of(p).is_some_and(|c| c == "Player")));
            if ok { none() } else { Err(rt_err("PersistentPerPlayer player not specified.")) }
        }
        "GetPersistentPlayers" if doc::is_a(class, "Model") => {
            list(lua, Vec::new()).map(Value::Table).and_then(one)
        }

        // --------------------------------------------- WorldRoot / Workspace
        "ArePartsTouchingOthers" if doc::is_a(class, "WorldRoot") => touching_others(lua, args),
        "IsRegion3Empty" | "IsRegion3EmptyWithIgnoreList" if doc::is_a(class, "WorldRoot") => (|| {
            let reg = match arg(args, 1) {
                Value::Nil => return Err(missing(1)),
                v => ud::<LRegion3>(&v).ok_or_else(|| rt_err("Unable to cast value to Region3"))?,
            };
            let ignore: HashSet<Ref> = match arg(args, 2) {
                Value::Table(t) => t.sequence_values::<Value>().flatten().filter_map(|v| opt_inst(&v)).collect(),
                v => opt_inst(&v).into_iter().collect(),
            };
            let empty = {
                let w = world(lua)?;
                let d = w.d();
                !parts_overlapping_box(d, reg.min, reg.max, &ignore)
            };
            one(Value::Boolean(empty))
        })(),
        "GetNumAwakeParts" if class == "Workspace" => {
            let n = world(lua)
                .ok()
                .and_then(|w| crate::physics::live::with_stats(w.d().id, |s| s.awake_parts))
                .unwrap_or(0);
            one(Value::Number(n as f64))
        }
        // Studio never throttles a local session.
        "GetPhysicsThrottling" if class == "Workspace" => one(Value::Number(100.0)),
        "PGSIsEnabled" if class == "Workspace" => one(Value::Boolean(true)),
        "JoinToOutsiders" if class == "Workspace" => join_outsiders(lua, args),
        "UnjoinFromOutsiders" if class == "Workspace" => unjoin_outsiders(lua, args),
        _ => return None,
    })
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if doc::is_a(class, "BasePart") {
        return Ok(Some(match key {
            "Mass" => {
                let w = world(lua)?;
                let d = w.d();
                Value::Number(with_graph(d, |g| eff_mass(d, g, r)) as f64)
            }
            "AssemblyMass" => {
                let w = world(lua)?;
                Value::Number(assembly_mass(w.d(), r).0 as f64)
            }
            "AssemblyCenterOfMass" => {
                let c = assembly_mass(world(lua)?.d(), r).1;
                vec3(lua, c)?
            }
            "CenterOfMass" => {
                let w = world(lua)?;
                let d = w.d();
                let p = part_mass(d, r);
                let x = xf(d, r, "CFrame").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY });
                vec3(lua, x.apply(p.com))?
            }
            "AssemblyRootPart" => {
                let root = {
                    let w = world(lua)?;
                    let d = w.d();
                    if !in_workspace(d, r) {
                        Ref::none()
                    } else {
                        with_graph(d, |g| g.root(d, r))
                    }
                };
                inst_value(lua, root)?
            }
            "AssemblyAngularVelocity" | "RotVelocity" => {
                let w = world(lua)?;
                let s = crate::physics::live::with_stats(w.d().id, |s| s.spin.get(&r).map(|x| x.0))
                    .flatten()
                    .unwrap_or(Vec3::ZERO);
                vec3(lua, s)?
            }
            "CurrentPhysicalProperties" => {
                let p = phys_props(world(lua)?.d(), r);
                Value::UserData(lua.create_userdata(p)?)
            }
            "SpecificGravity" => Value::Number(phys_props(world(lua)?.d(), r).density as f64),
            // The server simulates everything in Studio: no replicated age.
            "ReceiveAge" => Value::Number(0.0),
            "ResizeIncrement" => Value::Number(if class == "TrussPart" { 2.0 } else { 1.0 }),
            "ResizeableFaces" => Value::UserData(lua.create_userdata(LFaces(0b11_1111))?),
            _ => return Ok(None),
        }));
    }
    if class == "Workspace" {
        return Ok(Some(match key {
            // Always on (the property is deprecated; plugins alone may write it).
            "FilteringEnabled" => Value::Boolean(true),
            "InsertPoint" => vec3(lua, v3p(world(lua)?.d(), r, "InsertPoint").unwrap_or(Vec3::ZERO))?,
            "InterpolationThrottling" => enum_item(lua, "InterpolationThrottlingMode", "Default")?,
            "ReplicationFrequency" | "ReplicationThroughput" => {
                let v = crate::scene::enum_u32(world(lua)?.d().prop(r, key)).unwrap_or(0);
                match LEnumItem::from_value("ReplicationMode", v) {
                    Some(e) => enum_value(lua, e)?,
                    None => enum_item(lua, "ReplicationMode", "Automatic")?,
                }
            }
            "SignalBehavior" => return Err(not_member(lua, r, class, key)?),
            _ => return Ok(None),
        }));
    }
    // NotScriptable in Studio: reading them fails the same way.
    if doc::is_a(class, "PVInstance") && matches!(key, "Origin" | "Pivot Offset") {
        return Err(not_member(lua, r, class, key)?);
    }
    Ok(None)
}

fn not_member(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<mlua::Error> {
    let full = world(lua)?.full_name(r);
    Ok(rt_err(format!("{key} is not a valid member of {class} \"{full}\"")))
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    match (class, key) {
        ("Workspace", "InsertPoint") => {
            let v = ud::<LVector3>(value).ok_or_else(|| rt_err(format!(
                "Unable to assign property InsertPoint. Vector3 expected, got {}",
                value.type_name()
            )))?;
            store_prop(lua, r, "InsertPoint", Variant::Vector3(rbx_dom_weak::types::Vector3::new(v.0.x, v.0.y, v.0.z)))?;
            Ok(true)
        }
        ("Workspace", "ReplicationFrequency" | "ReplicationThroughput") => {
            let v = enum_of(value, "ReplicationMode").ok_or_else(|| {
                rt_err(format!("Unable to assign property {key}. EnumItem expected, got {}", value.type_name()))
            })?;
            store_prop(lua, r, key, Variant::Enum(rbx_dom_weak::types::Enum::from_u32(v)))?;
            Ok(true)
        }
        _ if doc::is_a(class, "PVInstance") && matches!(key, "Origin" | "Pivot Offset") => {
            Err(not_member(lua, r, class, key)?)
        }
        // The simulation takes the new spin; the property is stored as
        // well (falls through: an anchored part's spin moves what touches it).
        _ if doc::is_a(class, "BasePart") && matches!(key, "AssemblyAngularVelocity" | "RotVelocity") => {
            if let Some(v) = ud::<LVector3>(value) {
                let id = world(lua)?.d().id;
                crate::physics::live::queue_spin_set(id, r, v.0);
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

// ------------------------------------------------------------------ Model

/// Model:ScaleTo(s): parts' sizes and offsets from the pivot, attachments,
/// joint offsets and the rest scaled by s / GetScale() (Studio: s must be
/// positive; NaN warns and is ignored).
fn scale_to(lua: &Lua, model: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let s = num_arg(args, 1).ok_or_else(|| missing(1))? as f32;
    if s.is_nan() {
        world(lua)?.output.push((
            crate::app::Level::Warn,
            "Ignoring ScaleTo(NaN) call, check your code for divide by zero errors".into(),
        ));
        return none();
    }
    if s <= 0.0 {
        return Err(rt_err("Only positive non-zero scale values are allowed."));
    }
    let (writes, old) = {
        let w = world(lua)?;
        let d = w.d();
        let old = crate::scene::f32_of(d.prop(model, "Scale")).unwrap_or(1.0);
        let k = s / old;
        let pivot = model_pivot(&w, model);
        let mut writes: Vec<(Ref, &'static str, Variant)> = Vec::new();
        let scale_xf = |x: Xform| Xform { pos: x.pos * k, rot: x.rot };
        let v3 = |v: Vec3| Variant::Vector3(rbx_dom_weak::types::Vector3::new(v.x, v.y, v.z));
        for inst in d.dom.descendants_of(model) {
            let r = inst.referent();
            let c = inst.class.as_str();
            if doc::is_a(c, "BasePart") {
                if let (Some(x), Some(size)) = (xf(d, r, "CFrame"), v3p(d, r, "Size")) {
                    let pos = pivot.pos + (x.pos - pivot.pos) * k;
                    writes.push((r, "Size", v3((size * k).clamp(Vec3::splat(0.001), Vec3::splat(2048.0)))));
                    writes.push((r, "CFrame", Variant::CFrame(Xform { pos, rot: x.rot }.to_cframe())));
                }
            } else if c == "Attachment" || c == "Bone" {
                if let Some(x) = xf(d, r, "CFrame") {
                    writes.push((r, "CFrame", Variant::CFrame(scale_xf(x).to_cframe())));
                }
            } else if doc::is_a(c, "JointInstance") {
                for key in ["C0", "C1"] {
                    if let Some(x) = xf(d, r, key) {
                        writes.push((r, key, Variant::CFrame(scale_xf(x).to_cframe())));
                    }
                }
            } else if c == "SpecialMesh" {
                if let Some(v) = v3p(d, r, "Scale") {
                    writes.push((r, "Scale", v3(v * k)));
                }
                if let Some(v) = v3p(d, r, "Offset") {
                    writes.push((r, "Offset", v3(v * k)));
                }
            } else {
                let lengths: &[&'static str] = match c {
                    "RopeConstraint" => &["Length", "Thickness"],
                    "RodConstraint" => &["Length", "Thickness"],
                    "SpringConstraint" => &["FreeLength", "MinLength", "MaxLength", "Radius", "Thickness"],
                    "PrismaticConstraint" | "CylindricalConstraint" => &["LowerLimit", "UpperLimit", "Size"],
                    "Humanoid" => &["HipHeight"],
                    "Beam" => &["Width0", "Width1", "CurveSize0", "CurveSize1"],
                    "PointLight" | "SpotLight" | "SurfaceLight" => &["Range"],
                    _ => &[],
                };
                for key in lengths {
                    if let Some(f) = crate::scene::f32_of(d.prop(r, key)) {
                        writes.push((r, key, Variant::Float32(f * k)));
                    }
                }
            }
        }
        (writes, old)
    };
    for (r, key, v) in writes {
        store_prop(lua, r, key, v)?;
    }
    store_prop(lua, model, "Scale", Variant::Float32(s))?;
    let _ = old;
    none()
}

// -------------------------------------------------------------- WorldRoot

fn part_obb(d: &Document, r: Ref) -> Option<(Xform, Vec3)> {
    Some((xf(d, r, "CFrame")?, v3p(d, r, "Size")?))
}

/// Separating-axis test between two boxes, `pad` shrinking both (positive)
/// or growing them (negative).
fn obb_overlap(a: &(Xform, Vec3), b: &(Xform, Vec3), pad: f32) -> bool {
    let ha = (a.1 * 0.5 - Vec3::splat(pad)).max(Vec3::ZERO);
    let hb = (b.1 * 0.5 - Vec3::splat(pad)).max(Vec3::ZERO);
    let ax = [a.0.rot.x_axis, a.0.rot.y_axis, a.0.rot.z_axis];
    let bx = [b.0.rot.x_axis, b.0.rot.y_axis, b.0.rot.z_axis];
    let t = b.0.pos - a.0.pos;
    let proj = |axis: Vec3, h: Vec3, xs: &[Vec3; 3]| {
        h.x * xs[0].dot(axis).abs() + h.y * xs[1].dot(axis).abs() + h.z * xs[2].dot(axis).abs()
    };
    let mut axes: Vec<Vec3> = Vec::with_capacity(15);
    axes.extend(ax);
    axes.extend(bx);
    for i in ax {
        for j in bx {
            let c = i.cross(j);
            if c.length_squared() > 1e-8 {
                axes.push(c.normalize());
            }
        }
    }
    axes.iter().all(|axis| t.dot(*axis).abs() <= proj(*axis, ha, &ax) + proj(*axis, hb, &bx))
}

fn parts_overlapping_box(d: &Document, lo: Vec3, hi: Vec3, ignore: &HashSet<Ref>) -> bool {
    let Some(ws) = d.workspace() else { return false };
    let probe = (Xform { pos: (lo + hi) * 0.5, rot: Mat3::IDENTITY }, hi - lo);
    d.dom.descendants_of(ws).any(|i| {
        let r = i.referent();
        doc::is_a(&i.class, "BasePart")
            && i.class != "Terrain"
            && !ignore.iter().any(|g| *g == r || d.is_ancestor(*g, r))
            && part_obb(d, r).is_some_and(|b| obb_overlap(&probe, &b, 0.0))
    })
}

/// WorldRoot:ArePartsTouchingOthers(parts, overlapIgnored = 0.000001):
/// whether any of `parts` overlaps a part that isn't in the list.
fn touching_others(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let t = match arg(args, 1) {
        Value::Table(t) => t,
        Value::Nil => return Err(missing(1)),
        _ => return Err(rt_err("Unable to cast value to Objects")),
    };
    let pad = num_arg(args, 2).unwrap_or(0.000001) as f32;
    let w = world(lua)?;
    let d = w.d();
    let mut set: Vec<Ref> = Vec::new();
    for v in t.sequence_values::<Value>() {
        let v = v?;
        let r = opt_inst(&v);
        match r.filter(|r| is_part(d, *r)) {
            Some(r) => set.push(r),
            None => {
                let what = r.and_then(|r| d.class_of(r)).map(|c| c.to_string()).unwrap_or_else(|| v.type_name().to_string());
                return Err(rt_err(format!(
                    "All objects passed to ArePartsTouchingOthers must be BaseParts, got a {what}."
                )));
            }
        }
    }
    let Some(ws) = d.workspace() else { return one(Value::Boolean(false)) };
    let others: Vec<(Xform, Vec3)> = d
        .dom
        .descendants_of(ws)
        .filter(|i| doc::is_a(&i.class, "BasePart") && i.class != "Terrain" && !set.contains(&i.referent()))
        .filter_map(|i| part_obb(d, i.referent()))
        .collect();
    let hit = set.iter().filter_map(|r| part_obb(d, *r)).any(|a| others.iter().any(|b| obb_overlap(&a, b, pad)));
    one(Value::Boolean(hit))
}

// --------------------------------------------------------------- Workspace

const JOINABLE: &[u32] = &[1, 2, 3, 4, 5, 6]; // Glue, Weld, Studs, Inlet, Universal, Hinge…

/// Surfaces of `a` and `b` that touch face to face: (face of a, face of b).
fn touching_faces(a: &(Xform, Vec3), b: &(Xform, Vec3)) -> Option<(usize, usize)> {
    let faces = |x: &(Xform, Vec3)| {
        let h = x.1 * 0.5;
        [
            (x.0.rot.x_axis, h.x),
            (x.0.rot.y_axis, h.y),
            (x.0.rot.z_axis, h.z),
            (-x.0.rot.x_axis, h.x),
            (-x.0.rot.y_axis, h.y),
            (-x.0.rot.z_axis, h.z),
        ]
    };
    let fa = faces(a);
    let fb = faces(b);
    for (i, (na, da)) in fa.iter().enumerate() {
        for (j, (nb, db)) in fb.iter().enumerate() {
            if na.dot(*nb) > -0.999 {
                continue;
            }
            let pa = a.0.pos + *na * *da;
            let pb = b.0.pos + *nb * *db;
            if (pb - pa).dot(*na).abs() > 0.01 {
                continue;
            }
            if obb_overlap(a, b, -0.01) {
                return Some((i, j));
            }
        }
    }
    None
}

/// Face index (0 +X, 1 +Y, 2 +Z, 3 −X, 4 −Y, 5 −Z) → its surface property.
fn surface_prop(i: usize) -> &'static str {
    ["RightSurface", "TopSurface", "BackSurface", "LeftSurface", "BottomSurface", "FrontSurface"][i]
}

/// The parts at or under `objs`.
fn parts_of(d: &Document, objs: &[Ref]) -> Vec<Ref> {
    let mut set: Vec<Ref> = Vec::new();
    for o in objs {
        if is_part(d, *o) && !set.contains(o) {
            set.push(*o);
        }
        for i in d.dom.descendants_of(*o) {
            if doc::is_a(&i.class, "BasePart") && !set.contains(&i.referent()) {
                set.push(i.referent());
            }
        }
    }
    set
}

/// Welds parts of `set` to the `others` whose faces they sit on (mode 0
/// All: any touching faces; 1 Surface: a joinable surface on either side).
fn weld_touching(lua: &Lua, set: &[Ref], others: &[Ref], mode: u32) -> mlua::Result<()> {
    let pairs: Vec<(Ref, Ref)> = {
        let w = world(lua)?;
        let d = w.d();
        let mut v = Vec::new();
        for a in set {
            let Some(oa) = part_obb(d, *a) else { continue };
            for b in others {
                if set.contains(b) {
                    continue;
                }
                let Some(ob) = part_obb(d, *b) else { continue };
                if let Some((fa, fb)) = touching_faces(&oa, &ob) {
                    let st = |r: Ref, f: usize| crate::scene::enum_u32(d.prop(r, surface_prop(f))).unwrap_or(0);
                    if mode == 0 || JOINABLE.contains(&st(*a, fa)) || JOINABLE.contains(&st(*b, fb)) {
                        v.push((*a, *b));
                    }
                }
            }
        }
        v
    };
    for (a, b) in pairs {
        let j = crate::runtime::ext::create_any(lua, "Weld")?;
        let c1 = {
            let w = world(lua)?;
            let d = w.d();
            let id = Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY };
            let xa = xf(d, a, "CFrame").unwrap_or(id);
            let xb = xf(d, b, "CFrame").unwrap_or(id);
            mul(&xb.inverse(), &xa)
        };
        store_prop(lua, j, "Part0", Variant::Ref(a))?;
        store_prop(lua, j, "Part1", Variant::Ref(b))?;
        store_prop(lua, j, "C1", Variant::CFrame(c1.to_cframe()))?;
        set_parent(lua, j, a)?;
    }
    Ok(())
}

/// JointsService's join-after-move: `inst`'s parts welded (surface rules)
/// to `target`'s, or to anything under Workspace when there's no target.
pub fn join_between(lua: &Lua, inst: Ref, target: Option<Ref>) -> mlua::Result<()> {
    let (set, others) = {
        let w = world(lua)?;
        let d = w.d();
        let set = parts_of(d, &[inst]);
        let others = match target {
            Some(t) => parts_of(d, &[t]),
            None => d.workspace().map(|ws| parts_of(d, &[ws])).unwrap_or_default(),
        };
        (set, others)
    };
    weld_touching(lua, &set, &others, 1)
}

/// Workspace:JoinToOutsiders(objects, jointType): welds the given parts to
/// parts outside them whose surfaces they sit on.
fn join_outsiders(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let objs: Vec<Ref> = match arg(args, 1) {
        Value::Table(t) => t.sequence_values::<Value>().flatten().filter_map(|v| opt_inst(&v)).collect(),
        Value::Nil => return Err(missing(1)),
        _ => return Err(rt_err("Unable to cast value to Objects")),
    };
    let mode = enum_of(&arg(args, 2), "JointCreationMode").ok_or_else(|| missing(2))?;
    if mode == 2 {
        return none(); // JointCreationMode.None
    }
    let (set, others) = {
        let w = world(lua)?;
        let d = w.d();
        let set = parts_of(d, &objs);
        let others: Vec<Ref> = d
            .workspace()
            .map(|ws| parts_of(d, &[ws]))
            .unwrap_or_default()
            .into_iter()
            .filter(|r| !set.contains(r) && d.class_of(*r).is_some_and(|c| c != "Terrain"))
            .collect();
        (set, others)
    };
    weld_touching(lua, &set, &others, mode)?;
    none()
}

/// Workspace:UnjoinFromOutsiders(objects): removes joints between the
/// given parts and parts outside them.
fn unjoin_outsiders(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let objs: Vec<Ref> = match arg(args, 1) {
        Value::Table(t) => t.sequence_values::<Value>().flatten().filter_map(|v| opt_inst(&v)).collect(),
        Value::Nil => return Err(missing(1)),
        _ => return Err(rt_err("Unable to cast value to Objects")),
    };
    let doomed: Vec<Ref> = {
        let w = world(lua)?;
        let d = w.d();
        let mut set: HashSet<Ref> = HashSet::new();
        for o in &objs {
            if is_part(d, *o) {
                set.insert(*o);
            }
            set.extend(d.dom.descendants_of(*o).filter(|i| doc::is_a(&i.class, "BasePart")).map(|i| i.referent()));
        }
        let mut v = Vec::new();
        with_graph(d, |g| {
            for p in &set {
                for j in g.joints_of(*p) {
                    let c = d.class_of(j).map(|c| c.to_string()).unwrap_or_default();
                    if !rigid_joint(&c) {
                        continue;
                    }
                    let (a, b) = (refp(d, j, "Part0"), refp(d, j, "Part1"));
                    let inside = |x: Option<Ref>| x.is_some_and(|x| set.contains(&x));
                    if inside(a) != inside(b) && !v.contains(&j) {
                        v.push(j);
                    }
                }
            }
        });
        v
    };
    for j in doomed {
        destroy(lua, j)?;
    }
    none()
}

fn destroy(lua: &Lua, r: Ref) -> mlua::Result<()> {
    destroy_inst(lua, r)
}
