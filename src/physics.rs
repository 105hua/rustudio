//! rapier3d physics for Play / Run.
//!
//! Parts joined by welds (Weld, WeldConstraint, Snap, Glue, Motor6D…) form an
//! assembly, as in Roblox: an assembly with an anchored part is anchored and
//! its parts become fixed colliders built once; any other assembly is one
//! dynamic rigid body carrying a compound collider (one collider per part).
//! CanCollide=false parts are sensors (they still report touches) and the
//! player's character is a kinematic capsule. Changes come from the
//! document's change journal, so only the assemblies that changed are rebuilt.
//!
//! Body movers (BodyForce, BodyVelocity, BodyPosition, BodyGyro, …) and the
//! mover constraints (VectorForce, LinearVelocity, AlignPosition, …) push
//! dynamic assemblies with forces and torques recomputed before every step.
//! Terrain is a set of fixed triangle-mesh colliders built from its voxels.

#[path = "physics_rope.rs"]
mod rope;
#[path = "physics_client.rs"]
pub mod client;
#[path = "physics_joints.rs"]
mod joints;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use glam::{Mat3, Quat, Vec3};
use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::prelude as rp;
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::ustr;

use crate::doc::{self, Document};
use crate::math::Xform;
use crate::mech;
use crate::scene::{enum_u32, f32_of, vec3_of, xform_of};
use crate::terrain;

fn rv(v: Vec3) -> rp::Vector {
    rp::Vector::new(v.x, v.y, v.z)
}

fn gv(v: rp::Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn pose(xf: &Xform) -> rp::Pose {
    let q = Quat::from_mat3(&xf.rot).normalize();
    rp::Pose::from_parts(rv(xf.pos), rp::Rotation::from_xyzw(q.x, q.y, q.z, q.w))
}

fn yaw_rot(yaw: f32) -> rp::Rotation {
    let q = Quat::from_rotation_y(yaw);
    rp::Rotation::from_xyzw(q.x, q.y, q.z, q.w)
}

fn xform(p: &rp::Pose) -> Xform {
    let q = p.rotation;
    Xform {
        pos: gv(p.translation),
        rot: Mat3::from_quat(Quat::from_xyzw(q.x, q.y, q.z, q.w).normalize()),
    }
}

/// What a part's collider depends on; a change means rebuilding its assembly.
#[derive(Debug, Clone, PartialEq)]
struct Sig {
    anchored: bool,
    collide: bool,
    size: Vec3,
    /// 0 block, 1 ball, 2 cylinder, 3 wedge, 4 corner wedge.
    kind: u8,
    /// A MeshPart's mesh and its Enum.CollisionFidelity.
    mesh: Option<std::sync::Arc<str>>,
    fidelity: u32,
}

fn part_sig(doc: &Document, r: Ref) -> Option<(Xform, Sig)> {
    let class = doc.class_of(r)?;
    let xf = xform_of(doc, r)?;
    let size = vec3_of(doc.prop(r, "Size"))
        .unwrap_or(Vec3::new(4.0, 1.2, 2.0))
        .max(Vec3::splat(0.05));
    let kind = match class.as_str() {
        "WedgePart" => 3,
        "CornerWedgePart" => 4,
        c if doc::is_a(c, "Part") => match enum_u32(doc.prop(r, "Shape")) {
            Some(0) => 1,
            Some(2) => 2,
            Some(3) => 3,
            Some(4) => 4,
            _ => 0,
        },
        _ => 0,
    };
    let anchored = matches!(doc.prop(r, "Anchored"), Some(Variant::Bool(true)));
    let collide = !matches!(doc.prop(r, "CanCollide"), Some(Variant::Bool(false)));
    let (mesh, fidelity) = if class == "MeshPart" {
        let id = match doc.prop(r, "MeshId") {
            Some(Variant::ContentId(c)) => c.as_str().to_string(),
            Some(Variant::Content(c)) => c.as_uri().unwrap_or("").to_string(),
            Some(Variant::String(s)) => s,
            _ => String::new(),
        };
        let fid = enum_u32(doc.prop(r, "CollisionFidelity"))
            .unwrap_or(crate::collision::FIDELITY_DEFAULT);
        (
            (!id.is_empty()).then(|| std::sync::Arc::from(id.as_str())),
            fid,
        )
    } else if doc::is_a(class.as_str(), "PartOperation") {
        let mesh = crate::scene::union_mesh(doc, r).map(|m| std::sync::Arc::from(m.id));
        let fid = enum_u32(doc.prop(r, "CollisionFidelity"))
            .unwrap_or(crate::collision::FIDELITY_DEFAULT);
        (mesh, fid)
    } else {
        (None, crate::collision::FIDELITY_BOX)
    };
    Some((
        xf,
        Sig {
            anchored,
            collide,
            size,
            kind,
            mesh,
            fidelity,
        },
    ))
}

/// A MeshPart's collider by its CollisionFidelity (collision.rs): Box → its
/// box; Hull → the mesh's convex hull; Default / PreciseConvexDecomposition →
/// the convex decomposition. While a decomposition is still being computed
/// an anchored part collides with the mesh's own triangles and a moving one
/// with the hull; `pending` asks for a rebuild once it is ready.
fn mesh_shape(sig: &Sig) -> Option<(rp::SharedShape, bool)> {
    use crate::collision::{self as col, Lookup};
    let id = sig.mesh.as_deref()?;
    if sig.fidelity == col::FIDELITY_BOX {
        return None;
    }
    // Built shapes by (mesh, fidelity, exact size, anchored): a map's copies
    // of one asset share their collider shape (SharedShape is an Arc).
    // Building each one (convex hulls / a trimesh BVH) was ~2.7 s when the live place's
    // Giant map (48k parts) was cloned in.
    type Key = (String, u32, [u32; 3], bool);
    // An interim shape (decomposition still running) is reused until the
    // decomposition is ready.
    thread_local! {
        static SHAPES: std::cell::RefCell<HashMap<Key, (rp::SharedShape, bool)>> = Default::default();
    }
    let key: Key = (id.to_string(), sig.fidelity, sig.size.to_array().map(f32::to_bits), sig.anchored);
    if let Some((s, pending)) = SHAPES.with(|m| m.borrow().get(&key).cloned()) {
        if !pending || matches!(col::hulls(id, sig.fidelity), Lookup::Pending) {
            return Some((s, pending));
        }
    }
    let built = mesh_shape_uncached(sig, id);
    if let Some(b) = &built {
        SHAPES.with(|m| {
            let mut m = m.borrow_mut();
            if m.len() > 50_000 {
                m.clear();
            }
            m.insert(key, b.clone());
        });
    }
    built
}

fn mesh_shape_uncached(sig: &Sig, id: &str) -> Option<(rp::SharedShape, bool)> {
    use crate::collision::{self as col, Lookup};
    match col::hulls(id, sig.fidelity) {
        Lookup::Ready(h) => col::shape_from_hulls(&h, sig.size).map(|s| (s, false)),
        Lookup::Unavailable => None,
        Lookup::Pending => {
            let interim = if sig.anchored {
                col::trimesh(id, sig.size)
            } else {
                match col::hulls(id, col::FIDELITY_HULL) {
                    Lookup::Ready(h) => col::shape_from_hulls(&h, sig.size),
                    _ => None,
                }
            };
            interim.map(|s| (s, true))
        }
    }
}

/// The collider shape for a part, the shape's own offset inside the part,
/// and whether a better (mesh) shape is still on its way.
fn collider(sig: &Sig) -> (rp::ColliderBuilder, rp::Pose, bool) {
    let h = sig.size * 0.5;
    let pts = |list: &[[f32; 3]]| -> Vec<rp::Vector> {
        list.iter()
            .map(|p| rp::Vector::new(p[0] * h.x, p[1] * h.y, p[2] * h.z))
            .collect()
    };
    let mut offset = rp::Pose::IDENTITY;
    let mut pending = false;
    // A moving CanCollide=false mesh only reports touches (it never pushes
    // anything): it touches as its convex hull (reduced to 50 extreme
    // points), not its decomposition. A compound of convex pieces on a moving
    // body is re-bounded piece by piece every step: Place B's 25–60 client
    // tumbleweeds (an 11-hull non-colliding mesh each) cost ~10 ms a step
    // that way, 0.6 ms as hulls.
    let mesh = if !sig.collide
        && !sig.anchored
        && sig.mesh.is_some()
        && sig.fidelity != crate::collision::FIDELITY_BOX
    {
        let mut hull = sig.clone();
        hull.fidelity = crate::collision::FIDELITY_HULL;
        mesh_shape(&hull).map(|(s, p)| (crate::collision::touch_hull(&s), p))
    } else {
        mesh_shape(sig)
    };
    let b = match sig.kind {
        _ if mesh.is_some() => {
            let (shape, p) = mesh.expect("mesh shape");
            pending = p;
            rp::ColliderBuilder::new(shape)
        }
        1 => rp::ColliderBuilder::ball(h.min_element()),
        2 => {
            // Roblox cylinders run along X; rapier's along Y.
            offset = rp::Pose::from_parts(
                rp::Vector::ZERO,
                rp::Rotation::from_rotation_z(-std::f32::consts::FRAC_PI_2),
            );
            rp::ColliderBuilder::cylinder(h.x, h.y.min(h.z))
        }
        3 => rp::ColliderBuilder::convex_hull(&pts(&[
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
            [-1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
        ]))
        .unwrap_or_else(|| rp::ColliderBuilder::cuboid(h.x, h.y, h.z)),
        4 => rp::ColliderBuilder::convex_hull(&pts(&[
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, -1.0, 1.0],
            [-1.0, -1.0, 1.0],
            [1.0, 1.0, -1.0],
        ]))
        .unwrap_or_else(|| rp::ColliderBuilder::cuboid(h.x, h.y, h.z)),
        _ => rp::ColliderBuilder::cuboid(h.x, h.y, h.z),
    };
    // An off-centre union (GeometryService keeps the main part's CFrame):
    // its shape (mesh or Box fidelity) sits where its geometry is.
    let off = sig.mesh.as_deref().map(crate::mesh_bvh::union_offset).unwrap_or(Vec3::ZERO) * sig.size;
    if off != Vec3::ZERO {
        offset = rp::Pose::from_parts(rp::Vector::new(off.x, off.y, off.z), rp::Rotation::IDENTITY);
    }
    let b = b
        .friction(mech::PART_FRICTION)
        .restitution(mech::PART_ELASTICITY)
        .density(mech::PART_DENSITY)
        .sensor(!sig.collide)
        .active_events(rp::ActiveEvents::COLLISION_EVENTS);
    (b, offset, pending)
}

/// Whether a joint or constraint is on. WeldConstraint never saves or
/// replicates `Enabled`: a file and the Team Create wire carry `State`
/// (Studio saves 3 for an enabled weld and 0 for a disabled one; the live
/// wire sends 1), so bit 0 is Enabled. A script's write lands in `Enabled`.
pub fn joint_enabled(doc: &Document, j: Ref) -> bool {
    let Some(i) = doc.dom.get_by_ref(j) else { return false };
    match i.properties.get(&ustr("Enabled")) {
        Some(Variant::Bool(b)) => return *b,
        _ => {}
    }
    match i.properties.get(&ustr("State")) {
        Some(Variant::Int32(s)) if i.class == "WeldConstraint" => s & 1 != 0,
        _ => !matches!(doc.prop(j, "Enabled"), Some(Variant::Bool(false))),
    }
}

/// The part an attachment belongs to (its nearest BasePart ancestor).
pub fn attachment_part(doc: &Document, a: Ref) -> Option<Ref> {
    let mut cur = doc.dom.get_by_ref(a)?.parent();
    while let Some(i) = doc.dom.get_by_ref(cur) {
        if doc::is_a(&i.class, "BasePart") {
            return Some(cur);
        }
        cur = i.parent();
    }
    None
}

/// The two parts a rigid joint holds: Part0 / Part1 (alias-aware: files and
/// the live wire keep a WeldConstraint's as Part0Internal / Part1Internal),
/// or a RigidConstraint's attachments' parts.
pub fn joint_parts(doc: &Document, j: Ref) -> Option<(Ref, Ref)> {
    let class = doc.class_of(j)?;
    let pr = |k: &str| match doc.prop(j, k) {
        Some(Variant::Ref(x)) if x.is_some() => Some(x),
        _ => None,
    };
    if class == "RigidConstraint" {
        return Some((
            attachment_part(doc, pr("Attachment0")?)?,
            attachment_part(doc, pr("Attachment1")?)?,
        ));
    }
    Some((pr("Part0")?, pr("Part1")?))
}

/// In the client world a fixed collider moved by less than this (studs, and
/// 1 − |q·q'|) stays put; the server's follow every write exactly.
pub(super) const FOLLOWER_EPS_POS: f32 = 0.05;
pub(super) const FOLLOWER_EPS_ROT: f32 = 1e-6;

/// Joint classes that hold two parts rigidly together (one assembly).
pub fn is_rigid_joint(class: &str) -> bool {
    matches!(
        class,
        "RigidConstraint"
            | "WeldConstraint"
            | "Weld"
            | "ManualWeld"
            | "Motor6D"
            | "Motor"
            | "AnimationConstraint"
            | "Snap"
            | "Glue"
            | "ManualGlue"
    )
}

// ------------------------------------------------------------------ movers

/// Legacy BodyMovers act on their parent part; the mover constraints act on
/// Attachment0's parent part.
fn is_mover_class(class: &str) -> bool {
    matches!(
        class,
        "BodyForce"
            | "BodyThrust"
            | "BodyVelocity"
            | "BodyPosition"
            | "BodyGyro"
            | "BodyAngularVelocity"
            | "VectorForce"
            | "Torque"
            | "LinearVelocity"
            | "AlignPosition"
            | "AlignOrientation"
            | "AngularVelocity"
            | "RocketPropulsion"
    )
}

fn is_legacy_mover(class: &str) -> bool {
    class.starts_with("Body") || class == "RocketPropulsion"
}

/// Enum.ActuatorRelativeTo (and BodyThrust's part space).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Frame {
    World,
    Att0,
    Att1,
    Part,
}

fn actuator_frame(v: Option<u32>) -> Frame {
    match v {
        Some(0) => Frame::Att0,
        Some(1) => Frame::Att1,
        _ => Frame::World,
    }
}

/// How a constraint's force (or torque) is capped.
#[derive(Debug, Clone, Copy)]
enum Limit {
    None,
    Magnitude(f32),
    /// Per-axis caps along the axes of a frame (ForceLimitMode.PerAxis).
    PerAxis(Vec3, Frame),
}

#[derive(Debug, Clone, Copy)]
enum Law {
    /// BodyForce (world, centre of mass), BodyThrust (part space, at
    /// `at` in part space), VectorForce (frame; at Attachment0 unless
    /// ApplyAtCenterOfMass).
    Force {
        force: Vec3,
        frame: Frame,
        at: Option<Vec3>,
        at_att0: bool,
    },
    Torque {
        torque: Vec3,
        frame: Frame,
    },
    BodyVelocity {
        vel: Vec3,
        max: Vec3,
        p: f32,
    },
    LinearVelocity {
        mode: u32,
        vel: Vec3,
        line: Vec3,
        line_vel: f32,
        plane_vel: [f32; 2],
        t1: Vec3,
        t2: Vec3,
        planar_max: Option<[f32; 2]>,
        frame: Frame,
        limit: Limit,
    },
    BodyPosition {
        pos: Vec3,
        max: Vec3,
        p: f32,
        d: f32,
    },
    /// `goal` None = Attachment1's position (TwoAttachment).
    AlignPosition {
        goal: Option<Vec3>,
        limit: Limit,
        max_vel: f32,
        resp: f32,
        rigid: bool,
    },
    BodyGyro {
        rot: Quat,
        max: Vec3,
        p: f32,
        d: f32,
    },
    /// `goal` None = Attachment1's orientation (TwoAttachment).
    AlignOrientation {
        goal: Option<Quat>,
        primary_only: bool,
        max_torque: f32,
        max_angvel: f32,
        resp: f32,
        rigid: bool,
    },
    BodyAngularVelocity {
        w: Vec3,
        max: Vec3,
        p: f32,
    },
    AngularVelocity {
        w: Vec3,
        frame: Frame,
        max: f32,
    },
    /// RocketPropulsion: idle until Fire() (`live::rocket_fired`), then flies
    /// at MaxSpeed toward Target + TargetOffset (thrust capped by MaxThrust)
    /// and turns its front to it (TurnP / TurnD, MaxTorque); within
    /// TargetRadius it stops and hovers where it is (Studio 0.705: the part
    /// halts, ReachedTarget fires once) until Abort().
    Rocket {
        target: Ref,
        offset: Vec3,
        radius: f32,
        max_speed: f32,
        max_thrust: f32,
        max_torque: Vec3,
        turn_p: f32,
        turn_d: f32,
    },
}

/// An unlimited LinearVelocity is a rigid velocity constraint in Roblox:
/// it holds its velocity exactly whatever the floor's friction, where a
/// force added before rapier's solver loses friction's share every step
/// (a box driven at 10 on a Plastic floor ran at 9.02). Each step the
/// velocity the drive meant to reach (`v_pred`) is compared with what came
/// out of the solver; the loss along the constrained axes that isn't a
/// contact holding the body back (wall / floor normals) is friction, and is
/// added to the next step's push (`make_up`, a velocity), so the body moves
/// at the target speed while touching the floor.
/// The most friction make-up per step, in g·h (Roblox's friction tops out at
/// 2; a load stacked on the body presses harder).
const LV_MAKE_UP_MAX_G: f32 = 4.0;

#[derive(Debug, Clone, Copy, Default)]
struct LvTrack {
    /// Orthonormal constrained axes (world), `n` of them.
    axes: [Vec3; 3],
    n: usize,
    /// The velocity the forces alone would give at the end of the step.
    v_pred: Vec3,
    make_up: Vec3,
    /// Seen in this step's apply_movers.
    live: bool,
}

#[derive(Debug, Clone, Copy)]
struct Mover {
    /// The mover instance itself.
    inst: Ref,
    /// The part it pushes (its assembly's body takes the force).
    part: Ref,
    att0: Option<Ref>,
    att1: Option<Ref>,
    law: Law,
}

fn ref_prop(doc: &Document, r: Ref, name: &str) -> Option<Ref> {
    match doc.prop(r, name)? {
        Variant::Ref(x) if x.is_some() => Some(x),
        _ => None,
    }
}

fn bool_prop(doc: &Document, r: Ref, name: &str, default: bool) -> bool {
    match doc.prop(r, name) {
        Some(Variant::Bool(b)) => b,
        _ => default,
    }
}

fn num(doc: &Document, r: Ref, name: &str, default: f32) -> f32 {
    f32_of(doc.prop(r, name))
        .filter(|v| !v.is_nan())
        .unwrap_or(default)
}

fn vec(doc: &Document, r: Ref, name: &str) -> Vec3 {
    vec3_of(doc.prop(r, name))
        .filter(|v| !v.is_nan())
        .unwrap_or(Vec3::ZERO)
}

fn rot_prop(doc: &Document, r: Ref, name: &str) -> Quat {
    match doc.prop(r, name) {
        Some(Variant::CFrame(cf)) => {
            Quat::from_mat3(&Xform::from_cframe(&cf).orthonormalized().rot).normalize()
        }
        _ => Quat::IDENTITY,
    }
}

/// Reads a mover's settings. None = not a mover, outside Workspace,
/// disabled, or a constraint without Attachment0.
fn read_mover(doc: &Document, ws: Ref, r: Ref) -> Option<Mover> {
    let class = doc.class_of(r)?;
    let class = class.as_str();
    if !is_mover_class(class) || !doc.is_ancestor(ws, r) {
        return None;
    }
    let (part, att0) = if is_legacy_mover(class) {
        (doc.dom.get_by_ref(r)?.parent(), None)
    } else {
        if !bool_prop(doc, r, "Enabled", true) {
            return None;
        }
        let a0 = ref_prop(doc, r, "Attachment0")?;
        (doc.dom.get_by_ref(a0)?.parent(), Some(a0))
    };
    let att1 = if is_legacy_mover(class) {
        None
    } else {
        ref_prop(doc, r, "Attachment1")
    };
    let e = |name: &str| enum_u32(doc.prop(r, name));
    let inf = f32::INFINITY;
    // ForceLimitMode: Magnitude 0, PerAxis 1.
    let limit = |mag: &str, axes: &str, frame: Frame| -> Limit {
        if e("ForceLimitMode") == Some(1) {
            Limit::PerAxis(vec(doc, r, axes).abs(), frame)
        } else {
            Limit::Magnitude(num(doc, r, mag, inf).abs())
        }
    };
    let law = match class {
        "BodyForce" => Law::Force {
            force: vec(doc, r, "Force"),
            frame: Frame::World,
            at: None,
            at_att0: false,
        },
        "BodyThrust" => Law::Force {
            force: vec(doc, r, "Force"),
            frame: Frame::Part,
            at: Some(vec(doc, r, "Location")),
            at_att0: false,
        },
        "VectorForce" => Law::Force {
            force: vec(doc, r, "Force"),
            frame: actuator_frame(e("RelativeTo")),
            at: None,
            at_att0: !bool_prop(doc, r, "ApplyAtCenterOfMass", false),
        },
        "Torque" => Law::Torque {
            torque: vec(doc, r, "Torque"),
            frame: actuator_frame(e("RelativeTo")),
        },
        "BodyVelocity" => Law::BodyVelocity {
            vel: vec(doc, r, "Velocity"),
            max: vec(doc, r, "MaxForce").abs(),
            p: num(doc, r, "P", 1250.0),
        },
        "LinearVelocity" => {
            let frame = actuator_frame(e("RelativeTo"));
            let limited = bool_prop(doc, r, "ForceLimitsEnabled", true);
            let per_axis = e("ForceLimitMode") == Some(1);
            let plane = match doc.prop(r, "PlaneVelocity") {
                Some(Variant::Vector2(v)) => [v.x, v.y],
                _ => [0.0, 0.0],
            };
            let planar_max = match doc.prop(r, "MaxPlanarAxesForce") {
                Some(Variant::Vector2(v)) if limited && per_axis => Some([v.x.abs(), v.y.abs()]),
                _ => None,
            };
            Law::LinearVelocity {
                // VelocityConstraintMode: Line 0, Plane 1, Vector 2.
                mode: e("VelocityConstraintMode").unwrap_or(2),
                vel: vec(doc, r, "VectorVelocity"),
                line: vec(doc, r, "LineDirection"),
                line_vel: num(doc, r, "LineVelocity", 0.0),
                plane_vel: plane,
                t1: vec(doc, r, "PrimaryTangentAxis"),
                t2: vec(doc, r, "SecondaryTangentAxis"),
                planar_max,
                frame,
                limit: if limited {
                    limit("MaxForce", "MaxAxesForce", frame)
                } else {
                    Limit::None
                },
            }
        }
        "BodyPosition" => Law::BodyPosition {
            pos: vec(doc, r, "Position"),
            max: vec(doc, r, "MaxForce").abs(),
            p: num(doc, r, "P", 10000.0),
            d: num(doc, r, "D", 1250.0),
        },
        "AlignPosition" => {
            // PositionAlignmentMode: OneAttachment 0, TwoAttachment 1.
            let one = e("Mode") == Some(0);
            let frame = actuator_frame(e("ForceRelativeTo"));
            Law::AlignPosition {
                goal: one.then(|| vec(doc, r, "Position")),
                limit: limit("MaxForce", "MaxAxesForce", frame),
                max_vel: num(doc, r, "MaxVelocity", inf).abs(),
                resp: num(doc, r, "Responsiveness", 10.0).max(0.0),
                rigid: bool_prop(doc, r, "RigidityEnabled", false),
            }
        }
        "BodyGyro" => Law::BodyGyro {
            rot: rot_prop(doc, r, "CFrame"),
            max: vec(doc, r, "MaxTorque").abs(),
            p: num(doc, r, "P", 3000.0),
            d: num(doc, r, "D", 500.0),
        },
        "AlignOrientation" => {
            // OrientationAlignmentMode: OneAttachment 0, TwoAttachment 1.
            let one = e("Mode") == Some(0);
            Law::AlignOrientation {
                goal: one.then(|| rot_prop(doc, r, "CFrame")),
                // AlignType.PrimaryAxisParallel (2), or the deprecated
                // PrimaryAxisOnly: only Attachment0's X axis is aligned.
                // Perpendicular / LookAt types are treated as AllAxes.
                primary_only: bool_prop(doc, r, "PrimaryAxisOnly", false)
                    || e("AlignType") == Some(2),
                max_torque: num(doc, r, "MaxTorque", inf).abs(),
                max_angvel: num(doc, r, "MaxAngularVelocity", inf).abs(),
                resp: num(doc, r, "Responsiveness", 10.0).max(0.0),
                rigid: bool_prop(doc, r, "RigidityEnabled", false),
            }
        }
        "BodyAngularVelocity" => Law::BodyAngularVelocity {
            w: vec(doc, r, "AngularVelocity"),
            max: vec(doc, r, "MaxTorque").abs(),
            p: num(doc, r, "P", 1250.0),
        },
        "AngularVelocity" => Law::AngularVelocity {
            w: vec(doc, r, "AngularVelocity"),
            frame: actuator_frame(e("RelativeTo")),
            max: num(doc, r, "MaxTorque", inf).abs(),
        },
        "RocketPropulsion" => Law::Rocket {
            target: ref_prop(doc, r, "Target").unwrap_or_else(Ref::none),
            offset: vec(doc, r, "TargetOffset"),
            radius: num(doc, r, "TargetRadius", 4.0).max(0.0),
            max_speed: num(doc, r, "MaxSpeed", 30.0).max(0.0),
            max_thrust: num(doc, r, "MaxThrust", 4000.0).abs(),
            max_torque: vec3_of(doc.prop(r, "MaxTorque"))
                .filter(|v| !v.is_nan())
                .unwrap_or(Vec3::new(4e5, 4e5, 0.0))
                .abs(),
            turn_p: num(doc, r, "TurnP", 3000.0),
            turn_d: num(doc, r, "TurnD", 500.0),
        },
        _ => return None,
    };
    Some(Mover {
        inst: r,
        part,
        att0,
        att1,
        law,
    })
}

/// A dynamic body's state as the movers see it.
struct BodyState {
    mass: f32,
    com: Vec3,
    v: Vec3,
    w: Vec3,
    /// World-space inertia tensor about the centre of mass.
    inertia: Mat3,
}

impl BodyState {
    fn of(b: &rp::RigidBody) -> Option<BodyState> {
        let mp = &b.mass_properties().local_mprops;
        let mass = mp.mass();
        if !(mass > 0.0 && mass.is_finite()) {
            return None;
        }
        let pose = xform(b.position());
        let local = mp.reconstruct_inertia_matrix();
        let col = |c: rp::Vector| Vec3::new(c.x, c.y, c.z);
        let il = Mat3::from_cols(col(local.x_axis), col(local.y_axis), col(local.z_axis));
        let inertia = pose.rot * il * pose.rot.transpose();
        if !inertia.is_finite() {
            return None;
        }
        Some(BodyState {
            mass,
            com: pose.pos + pose.rot * gv(mp.local_com),
            v: gv(b.linvel()),
            w: gv(b.angvel()),
            inertia,
        })
    }

    /// Torque that changes the angular velocity to `w_new` over one step.
    fn torque_for(&self, w_new: Vec3, h: f32) -> Vec3 {
        self.inertia * (w_new - self.w) / h
    }

    /// Implicit (backward Euler) angular servo: torque = p·θ + c·(ω_goal − ω_new) − d·ω_new,
    /// solved for ω_new so stiff gains stay stable at the step rate.
    fn angular_servo(&self, theta: Vec3, p: f32, d: f32, c: f32, w_goal: Vec3, h: f32) -> Vec3 {
        let a = self.inertia / h + Mat3::from_diagonal(Vec3::splat(p * h + d + c));
        let rhs = self.inertia * self.w / h + p * theta + c * w_goal;
        if a.determinant().abs() < 1e-12 {
            return Vec3::ZERO;
        }
        let w_new = a.inverse() * rhs;
        self.torque_for(w_new, h)
    }
}

/// Rotation vector (axis × angle, shortest way) taking `from` to `to`.
fn rot_error(from: Quat, to: Quat) -> Vec3 {
    let mut q = (to * from.inverse()).normalize();
    if q.w < 0.0 {
        q = -q;
    }
    q.to_scaled_axis()
}

/// One step of a critically damped spring of natural frequency `w` toward an
/// error `e` (implicit, so any `w` is stable): the new relative velocity.
fn critical(v_rel: Vec3, e: Vec3, w: f32, h: f32) -> Vec3 {
    (v_rel + h * w * w * e) / (1.0 + 2.0 * mech::ALIGN_DAMPING_RATIO * h * w + h * h * w * w)
}

fn clamp_axes(v: Vec3, max: Vec3) -> Vec3 {
    v.clamp(-max, max)
}

// ------------------------------------------------------------------ events

/// Collects contact start/stop events during a step.
#[derive(Default)]
struct Collector(Mutex<Vec<(rp::ColliderHandle, rp::ColliderHandle, bool)>>);

impl rp::EventHandler for Collector {
    fn handle_collision_event(
        &self,
        _: &rp::RigidBodySet,
        _: &rp::ColliderSet,
        event: rp::CollisionEvent,
        _: Option<&rp::ContactPair>,
    ) {
        let mut v = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match event {
            rp::CollisionEvent::Started(a, b, _) => v.push((a, b, true)),
            rp::CollisionEvent::Stopped(a, b, _) => v.push((a, b, false)),
        }
    }

    fn handle_contact_force_event(
        &self,
        _: rp::Real,
        _: &rp::RigidBodySet,
        _: &rp::ColliderSet,
        _: &rp::ContactPair,
        _: rp::Real,
    ) {
    }

    fn handle_soft_body_tear_event(&self, _: &rp::SoftBodySet, _: &rp::SoftBodyTearEvent) {}
}

struct PartBody {
    /// The dynamic assembly this part rides on (None = fixed collider).
    asm: Option<u32>,
    collider: rp::ColliderHandle,
    /// The part's pose relative to its assembly's body.
    local: rp::Pose,
}

struct Assembly {
    body: rp::RigidBodyHandle,
    parts: Vec<Ref>,
}

/// The player's capsule (position = capsule centre).
#[derive(Debug, Clone, Copy, Default)]
pub struct CharBody {
    pub pos: Vec3,
    pub vel_y: f32,
    pub grounded: bool,
    /// The Humanoid controller's velocity (round 22; vel.y is its own).
    pub vel: Vec3,
    /// The floor rays found a floor last step (they reach further then).
    pub had_floor: bool,
    /// Still rising from a jump (the floor doesn't hold it until it falls).
    pub jump_rise: bool,
    /// Seconds since the floor last held it (coyote time: a jump still
    /// takes off this soon after walking off a ledge).
    pub air_time: f32,
    /// The wall its body box last pushed against (outward normal).
    pub wall: Option<Vec3>,
    /// That wall's friction against the character's Plastic (Roblox's mix).
    pub wall_friction: f32,
    /// It walked into that wall on a floor and bounces off it (the mean
    /// friction), rather than being held flush (round 26).
    pub wall_bounce: bool,
    /// Jumping: the take-off speed is held for this long more against a
    /// wall (Studio's Jumping step re-applies it, round 26).
    pub jump_hold: f32,
    /// … the take-off speed.
    pub jump_speed: f32,
    /// After a jump against a wall: its friction brakes the rise this long.
    pub wall_grip: f32,
    /// The walk is a Plane LinearVelocity's (unlimited force, world): the
    /// root's horizontal velocity is held to it exactly, on slopes, against
    /// walls and in the air, as Roblox's solver does (Place A's pmove reads it
    /// back each frame; a Humanoid walk turned into a ramp's plane came
    /// back slower and with any sideways part grown, and fed back in).
    pub walk_exact: bool,
}

#[derive(Default)]
pub struct StepResult {
    /// Parts that moved (their CFrame is already written to the doc).
    pub moved: Vec<(Ref, Xform)>,
    /// (part, other part, started) contact changes.
    pub touches: Vec<(Ref, Ref, bool)>,
    /// Parts that fell below FallenPartsDestroyHeight.
    pub fallen: Vec<Ref>,
}

pub struct Physics {
    world: rp::PhysicsWorld,
    parts: rustc_hash::FxHashMap<Ref, PartBody>,
    asms: HashMap<u32, Assembly>,
    next_asm: u32,
    by_collider: rustc_hash::FxHashMap<rp::ColliderHandle, Ref>,
    /// Rigid joints: joint → (Part0, Part1), and part → its joints.
    joints: HashMap<Ref, (Ref, Ref)>,
    adj: HashMap<Ref, Vec<Ref>>,
    collector: Collector,
    accum: f32,
    /// Parts the physics leaves alone (the character's own parts).
    pub skip: HashSet<Ref>,
    /// Parts another simulation moves (the client's copies of what the
    /// server simulates): fixed colliders here, moved by their CFrame
    /// writes (see physics_client.rs).
    external: HashSet<Ref>,
    /// Bumped whenever a dynamic body is added or removed.
    dyn_version: u64,
    /// How far a fixed collider may lag its part's CFrame (studs, rotation):
    /// exact on the server, FOLLOWER_EPS_* in the client world.
    pub(super) follower_eps: (f32, f32),
    /// Parts built since last taken (recorded only when Some: the client).
    fresh: Option<Vec<Ref>>,
    /// Parts to re-check on the next sync (e.g. after `skip` changed).
    queued: HashSet<Ref>,
    /// Mesh parts colliding with a stand-in until their decomposition is ready.
    mesh_pending: HashMap<Ref, (std::sync::Arc<str>, u32)>,
    /// Off-centre unions' shape offsets (mesh_bvh::union_offset), kept for
    /// teleports.
    union_offsets: HashMap<rp::ColliderHandle, rp::Pose>,
    mesh_checked: Option<std::time::Instant>,
    controller: KinematicCharacterController,
    /// The Humanoid's body box mover (no steps, no snapping: the floor
    /// rays and the hip spring do that, as in Studio).
    hum_controller: KinematicCharacterController,
    /// Velocities scripts gave anchored parts (a conveyor's
    /// AssemblyLinearVelocity carries what stands on it).
    anchored_vel: HashMap<Ref, Vec3>,
    /// Velocities scripts gave parts the physics hasn't built yet (made
    /// this frame): applied once their body exists (frames left to wait).
    pending_vel: HashMap<Ref, (Vec3, u8)>,
    /// The same for angular impulses (accumulated spin, frames waited).
    pending_spin: HashMap<Ref, (Vec3, u8)>,
    /// Anchored parts whose collider is where (and how big) the client has
    /// them, by server part: the client's pose and Size (follow_client;
    /// build() keeps them through rebuilds).
    client_over: HashMap<Ref, (Xform, Vec3)>,
    /// Parts the last sync moved for the server (their client marks are
    /// the same moves, replicated: follow_client skips them).
    server_moved: HashSet<Ref>,
    pub gravity: f32,
    pub steps: u64,
    /// Assemblies rebuilt by journal syncs (for the perf check).
    pub rebuilt: u64,
    /// Body movers and mover constraints under Workspace, by instance.
    movers: HashMap<Ref, Mover>,
    /// RopeConstraints seen under Workspace (apply_ropes walks only these).
    pub(super) ropes: HashSet<Ref>,
    /// Hinge / BallSocket / Prismatic / … constraints under Workspace and
    /// the joints made for them (physics_joints.rs).
    joint_cons: HashSet<Ref>,
    live_joints: HashMap<Ref, joints::LiveJoint>,
    /// The fixed body anchored constraint ends hang on.
    ground: Option<rp::RigidBodyHandle>,
    /// Bodies that carry mover forces from the last step (reset each step).
    pushed: HashSet<rp::RigidBodyHandle>,
    /// Bodies driven by an unlimited LinearVelocity, for its friction
    /// make-up (see `track_velocity_constraints`).
    lv_track: HashMap<rp::RigidBodyHandle, LvTrack>,
    /// Workspace.Terrain and its fixed triangle-mesh colliders.
    terrain: Option<Ref>,
    terrain_colliders: Vec<rp::ColliderHandle>,
    terrain_tris: usize,
    /// The terrain's voxels, kept for water (Humanoid swimming).
    voxels: Option<terrain::Voxels>,
    /// PhysicsService collision groups (shared with the session's runtimes)
    /// and the version last applied to the colliders.
    pub groups: crate::collision_groups::Shared,
    groups_seen: u64,
    /// The player character's CollisionGroup (its HumanoidRootPart's).
    pub char_group: String,
}

fn new_world(gravity: f32) -> rp::PhysicsWorld {
    let mut world = rp::PhysicsWorld::new();
    world.gravity = rp::Vector::new(0.0, -gravity, 0.0);
    world.integration_parameters.dt = 1.0 / mech::PHYSICS_HZ;
    world.integration_parameters.num_solver_iterations = mech::PHYSICS_SUBSTEPS;
    // rapier's tolerances and sleep thresholds assume metre-sized objects;
    // Roblox parts are measured in studs with 20× Earth gravity.
    world.integration_parameters.length_unit = mech::PHYSICS_LENGTH_UNIT;
    world.physics_pipeline.counters.enable();
    world
}

impl Physics {
    pub fn new(doc: &mut Document) -> Self {
        Self::new_skipping(doc, HashSet::new())
    }

    /// A world that leaves `skip` out from the start (the client's: its
    /// replicated parts come in only near its own bodies, physics_client.rs).
    pub fn new_skipping(doc: &mut Document, skip: HashSet<Ref>) -> Self {
        let gravity = doc
            .workspace()
            .and_then(|w| f32_of(doc.prop(w, "Gravity")))
            .unwrap_or(mech::GRAVITY);
        let controller = KinematicCharacterController {
            up: rp::Vector::Y,
            offset: CharacterLength::Absolute(mech::CHARACTER_HOVER),
            slide: true,
            autostep: Some(CharacterAutostep {
                max_height: CharacterLength::Absolute(mech::CHARACTER_STEP_HEIGHT),
                min_width: CharacterLength::Absolute(0.3),
                include_dynamic_bodies: true,
            }),
            max_slope_climb_angle: mech::MAX_SLOPE_DEG.to_radians(),
            min_slope_slide_angle: mech::MAX_SLOPE_DEG.to_radians(),
            snap_to_ground: Some(CharacterLength::Absolute(0.5)),
            ..Default::default()
        };
        let hum_controller = KinematicCharacterController {
            up: rp::Vector::Y,
            offset: CharacterLength::Absolute(mech::HUMANOID_BOX_SKIN),
            slide: true,
            autostep: None,
            max_slope_climb_angle: mech::HUMANOID_BOX_CLIMB_DEG.to_radians(),
            // (any slope: at 90° a wall counted as one not to slide down, and a
            // body pressed into it hung there — round 23)
            min_slope_slide_angle: 1f32.to_radians(),
            snap_to_ground: None,
            ..Default::default()
        };
        let mut p = Physics {
            world: new_world(gravity),
            hum_controller,
            anchored_vel: HashMap::new(),
            pending_vel: HashMap::new(),
            pending_spin: HashMap::new(),
            client_over: HashMap::new(),
            server_moved: HashSet::new(),
            parts: Default::default(),
            asms: HashMap::new(),
            next_asm: 0,
            by_collider: Default::default(),
            joints: HashMap::new(),
            adj: HashMap::new(),
            collector: Collector::default(),
            accum: 0.0,
            skip: HashSet::new(),
            external: HashSet::new(),
            dyn_version: 0,
            follower_eps: (1e-5, 1e-9),
            fresh: None,
            queued: HashSet::new(),
            mesh_pending: HashMap::new(),
            union_offsets: HashMap::new(),
            mesh_checked: None,
            controller,
            gravity,
            steps: 0,
            rebuilt: 0,
            movers: HashMap::new(),
            ropes: HashSet::new(),
            joint_cons: HashSet::new(),
            live_joints: HashMap::new(),
            ground: None,
            pushed: HashSet::new(),
            lv_track: HashMap::new(),
            terrain: None,
            terrain_colliders: Vec::new(),
            terrain_tris: 0,
            voxels: None,
            groups: crate::collision_groups::shared(),
            groups_seen: 0,
            char_group: crate::collision_groups::DEFAULT.to_string(),
        };
        p.skip = skip;
        // Start reading the journal from here.
        let _ = doc.marks_for(doc::Reader::Physics);
        p.reconcile(doc);
        p
    }

    fn workspace_of(doc: &Document) -> Option<Ref> {
        doc.workspace()
    }

    /// A part the simulation should hold: a BasePart under Workspace, not skipped.
    fn wanted(&self, doc: &Document, ws: Ref, r: Ref) -> bool {
        !self.skip.contains(&r)
            && doc
                .class_of(r)
                .is_some_and(|c| c != "Terrain" && doc::is_a(&c, "BasePart"))
            && doc.is_ancestor(ws, r)
    }

    /// Full rebuild from the document.
    pub fn reconcile(&mut self, doc: &Document) {
        self.world = new_world(self.gravity);
        self.parts.clear();
        self.asms.clear();
        self.dyn_version += 1;
        self.by_collider.clear();
        self.joints.clear();
        self.adj.clear();
        self.movers.clear();
        self.ropes.clear();
        self.joint_cons.clear();
        self.live_joints.clear();
        self.ground = None;
        self.pushed.clear();
        self.terrain_colliders.clear();
        self.terrain_tris = 0;
        self.terrain = None;
        let Some(ws) = Self::workspace_of(doc) else {
            return;
        };
        let mut seeds = HashSet::new();
        for i in doc.dom.descendants_of(ws) {
            let r = i.referent();
            if i.class == "RopeConstraint" || i.class == "RodConstraint" {
                self.ropes.insert(r);
            } else if joints::is_joint_constraint(&i.class) {
                self.joint_cons.insert(r);
            }
            if is_rigid_joint(&i.class) {
                self.read_joint(doc, ws, r);
            } else if is_mover_class(&i.class) {
                if let Some(m) = read_mover(doc, ws, r) {
                    self.movers.insert(r, m);
                }
            } else if i.class != "Terrain"
                && doc::is_a(&i.class, "BasePart")
                && !self.skip.contains(&r)
            {
                seeds.insert(r);
            }
        }
        self.build(doc, ws, seeds);
        self.build_terrain(doc);
        // Queries only see colliders after a step: prime the broad phase so
        // the first character move already stands on the ground.
        self.prime();
    }

    fn prime(&mut self) {
        self.world.integration_parameters.dt = 1e-6;
        self.world.step_with_events(&(), &self.collector);
        self.world.integration_parameters.dt = 1.0 / mech::PHYSICS_HZ;
        self.collector
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// (Re)reads a joint's parts; returns the parts it joined before and after.
    fn read_joint(&mut self, doc: &Document, ws: Ref, j: Ref) -> Vec<Ref> {
        let mut touched = Vec::new();
        if let Some((a, b)) = self.joints.remove(&j) {
            for p in [a, b] {
                if let Some(list) = self.adj.get_mut(&p) {
                    list.retain(|x| *x != j);
                }
                touched.push(p);
            }
        }
        let class = doc.class_of(j);
        // A kinematic AnimationConstraint (Studio's upgraded avatar joint)
        // holds its parts like a Motor6D; its parts come from its attachments.
        let animc = class.is_some_and(|c| c == "AnimationConstraint")
            && !matches!(doc.prop(j, "IsKinematic"), Some(Variant::Bool(false)));
        let rigid = if class.is_some_and(|c| c == "AnimationConstraint") {
            animc
        } else {
            class.is_some_and(|c| is_rigid_joint(&c))
        };
        let live = rigid && doc.is_ancestor(ws, j) && joint_enabled(doc, j);
        if live {
            let parts = if animc {
                crate::rig::joint_frames(doc, j).map(|(a, b, _, _)| (a, b))
            } else {
                joint_parts(doc, j)
            };
            if let Some((a, b)) = parts
            {
                if a.is_some() && b.is_some() && a != b {
                    self.joints.insert(j, (a, b));
                    self.adj.entry(a).or_default().push(j);
                    self.adj.entry(b).or_default().push(j);
                    touched.extend([a, b]);
                }
            }
        }
        touched
    }

    fn neighbours(&self, r: Ref) -> impl Iterator<Item = Ref> + '_ {
        self.adj.get(&r).into_iter().flatten().filter_map(move |j| {
            self.joints
                .get(j)
                .map(|(a, b)| if *a == r { *b } else { *a })
        })
    }

    /// Removes a part's collider (and its assembly's body once empty).
    fn drop_part(&mut self, r: Ref) {
        let Some(pb) = self.parts.remove(&r) else {
            return;
        };
        self.by_collider.remove(&pb.collider);
        self.world.remove_collider(pb.collider);
        if let Some(id) = pb.asm {
            self.dyn_version += 1;
            if let Some(a) = self.asms.get_mut(&id) {
                a.parts.retain(|x| *x != r);
                if a.parts.is_empty() {
                    let body = a.body;
                    self.asms.remove(&id);
                    self.world.remove_body(body);
                }
            }
        }
    }

    /// Rebuilds the assemblies around `seeds`: everything welded to them and
    /// everything that shared an assembly with them.
    fn build(&mut self, doc: &Document, ws: Ref, seeds: HashSet<Ref>) {
        // 1. Close over welds and old assemblies.
        let b1 = crate::bench::scope("pb_close");
        let mut all: HashSet<Ref> = HashSet::new();
        let mut stack: Vec<Ref> = seeds.into_iter().collect();
        while let Some(r) = stack.pop() {
            if !all.insert(r) {
                continue;
            }
            if let Some(id) = self.parts.get(&r).and_then(|p| p.asm) {
                if let Some(a) = self.asms.get(&id) {
                    stack.extend(a.parts.iter().copied());
                }
            }
            stack.extend(self.neighbours(r));
        }
        drop(b1);
        let b2 = crate::bench::scope("pb_drop");
        // 2. Drop what's there now.
        for r in &all {
            self.drop_part(*r);
        }
        drop(b2);
        let b3 = crate::bench::scope("pb_sig");
        // 3. Group the parts that should exist into assemblies.
        let mut info: HashMap<Ref, (Xform, Sig)> = HashMap::new();
        for r in &all {
            if self.wanted(doc, ws, *r) {
                if let Some(mut v) = part_sig(doc, *r) {
                    if self.external.contains(r) {
                        v.1.anchored = true;
                    }
                    // A part the client moved or resized collides as the client has it.
                    if let Some((xf, size)) = self.client_over.get(r) {
                        if v.1.anchored {
                            v.0 = *xf;
                            v.1.size = *size;
                        } else {
                            self.client_over.remove(r);
                        }
                    }
                    info.insert(*r, v);
                }
            }
        }
        drop(b3);
        let _b4 = crate::bench::scope("pb_asm");
        let mut done: HashSet<Ref> = HashSet::new();
        let keys: Vec<Ref> = info.keys().copied().collect();
        for start in keys {
            if done.contains(&start) {
                continue;
            }
            let mut group = Vec::new();
            let mut stack = vec![start];
            while let Some(r) = stack.pop() {
                if !info.contains_key(&r) || !done.insert(r) {
                    continue;
                }
                group.push(r);
                stack.extend(self.neighbours(r));
            }
            self.add_assembly(doc, &group, &info);
            self.rebuilt += 1;
        }
    }

    fn add_assembly(&mut self, doc: &Document, group: &[Ref], info: &HashMap<Ref, (Xform, Sig)>) {
        let anchored = group.iter().any(|r| info[r].1.anchored);
        if anchored {
            // Anchored assembly: plain fixed colliders, never simulated.
            for r in group {
                let (xf, sig) = &info[r];
                let (b, offset, pending) = collider(sig);
                if let (true, Some(m)) = (pending, sig.mesh.clone()) {
                    self.mesh_pending.insert(*r, (m, sig.fidelity));
                }
                let b = b.collision_groups(self.part_groups(doc, *r));
                // Where the client has it (follow_client): only the
                // character's queries see it.
                let b = if self.client_over.contains_key(r) {
                    b.active_collision_types(rp::ActiveCollisionTypes::empty())
                } else {
                    b
                };
                let c = self
                    .world
                    .insert_collider(b.position(pose(xf) * offset), None);
                self.by_collider.insert(c, *r);
                if sig.mesh.as_deref().is_some_and(|m| crate::mesh_bvh::union_offset(m) != Vec3::ZERO) {
                    self.union_offsets.insert(c, offset);
                }
                if let Some(f) = self.fresh.as_mut() {
                    f.push(*r);
                }
                self.parts.insert(
                    *r,
                    PartBody {
                        asm: None,
                        collider: c,
                        local: rp::Pose::IDENTITY,
                    },
                );
            }
            return;
        }
        // The biggest part is the assembly's root.
        let root = *group
            .iter()
            .max_by(|a, b| {
                let va = info[*a].1.size;
                let vb = info[*b].1.size;
                (va.x * va.y * va.z).total_cmp(&(vb.x * vb.y * vb.z))
            })
            .expect("non-empty assembly");
        let root_pose = pose(&info[&root].0);
        // Humanoid characters stay upright.
        let upright = group.iter().any(|r| {
            doc.dom
                .get_by_ref(*r)
                .is_some_and(|i| i.name == "HumanoidRootPart")
        });
        let mut rb = rp::RigidBodyBuilder::dynamic().pose(root_pose);
        if upright {
            rb = rb.lock_rotations();
        }
        let body = self.world.insert_body(rb.build());
        self.dyn_version += 1;
        let id = self.next_asm;
        self.next_asm += 1;
        let inv = root_pose.inverse();
        for r in group {
            let (xf, sig) = &info[r];
            let local = inv * pose(xf);
            let (b, offset, pending) = collider(sig);
            if let (true, Some(m)) = (pending, sig.mesh.clone()) {
                self.mesh_pending.insert(*r, (m, sig.fidelity));
            }
            let b = b.collision_groups(self.part_groups(doc, *r));
            let c = self
                .world
                .insert_collider(b.position(local * offset), Some(body));
            self.by_collider.insert(c, *r);
            if let Some(f) = self.fresh.as_mut() {
                f.push(*r);
            }
            self.parts.insert(
                *r,
                PartBody {
                    asm: Some(id),
                    collider: c,
                    local,
                },
            );
        }
        // Movers need the mass before the next step computes it.
        if let Some(b) = self.world.bodies.get_mut(body) {
            b.recompute_mass_properties_from_colliders(&self.world.colliders);
        }
        self.asms.insert(
            id,
            Assembly {
                body,
                parts: group.to_vec(),
            },
        );
    }

    /// Queues parts to re-check (call after changing `skip`).
    pub fn refresh(&mut self, refs: impl IntoIterator<Item = Ref>) {
        self.queued.extend(refs);
    }

    /// Round 23: anchored parts a LocalScript moved on the client (a
    /// tweened platform) collide where the client has them. The character
    /// is the client's, and Studio's client simulates it against its own
    /// copy of the world; the server's copy stays where it was. Only parts
    /// whose client CFrame or Size differs from the server's: most client
    /// marks are the server's own moves replicated (Place A places whole maps),
    /// and following those put colliders a frame behind the server. A part
    /// is put back when the two agree again. Moving parts (with an
    /// assembly) stay the server's.
    ///
    /// Size counts too (Place B's spawn Teleporter: a LocalScript turns a 1×1×1
    /// part at the spawn into a 2048×1×2048 void catcher under the map). A
    /// journal read that comes back All (the client's start-up flood)
    /// compares every part instead of dropping the batch.
    pub fn follow_client(&mut self, server: &Document, client: &mut Document, c2s: &HashMap<Ref, Ref>) {
        let _b = crate::bench::scope("ps_follow_client");
        let cframe = ustr("CFrame");
        let size = ustr("Size");
        let changed: Vec<Ref> = match client.marks_for(doc::Reader::ClientPhysics) {
            doc::Marks::All => c2s.keys().copied().collect(),
            doc::Marks::Some(marks) => marks
                .into_iter()
                .filter_map(|m| match m {
                    doc::Mark::Prop(c, name) if name == cframe || name == size => Some(c),
                    _ => None,
                })
                .collect(),
        };
        let mut seen: HashSet<Ref> = HashSet::new();
        let mut resized: HashSet<Ref> = HashSet::new();
        for c in changed {
            let Some(&s) = c2s.get(&c) else { continue };
            if self.server_moved.contains(&s) && !self.client_over.contains_key(&s) {
                continue;
            }
            if !seen.insert(c) || !self.parts.get(&s).is_some_and(|pb| pb.asm.is_none()) {
                continue;
            }
            let (Some((cx, csig)), Some((sx, ssig))) = (part_sig(client, c), part_sig(server, s)) else {
                continue;
            };
            let near = |a: Vec3, b: Vec3| (a - b).abs().max_element() < 1e-3;
            let same = near(cx.pos, sx.pos) && cx.rot.abs_diff_eq(sx.rot, 1e-4) && near(csig.size, ssig.size);
            // The size the collider has now.
            let had = self.client_over.get(&s).map_or(ssig.size, |o| o.1);
            if !same {
                self.client_over.insert(s, (cx, csig.size));
                if near(had, csig.size) {
                    self.set_followed(s, true);
                    self.teleport(s, &cx);
                } else {
                    resized.insert(s);
                }
            } else if self.client_over.remove(&s).is_some() {
                if near(had, ssig.size) {
                    self.set_followed(s, false);
                    self.teleport(s, &sx);
                } else {
                    resized.insert(s);
                }
            }
        }
        if !resized.is_empty() {
            if let Some(ws) = Self::workspace_of(server) {
                self.build(server, ws, resized);
            }
        }
    }

    /// A collider placed where the client has its part stays out of the
    /// server's contacts: in Roblox the server never sees a client-side
    /// move, so its bodies can't react to one. The character (simulated
    /// here with queries, which still see it) stands on it as the client's
    /// would. Without this, a replicated part a LocalScript moves through
    /// the server's resting bodies shoves them awake (Place E fixture: its spawn
    /// lantern moved on the client woke the server's ~580 book parts every
    /// frame, ≈1250 contact pairs, 50–90 ms steps).
    fn set_followed(&mut self, r: Ref, on: bool) {
        let Some(pb) = self.parts.get(&r) else { return };
        if let Some(c) = self.world.colliders.get_mut(pb.collider) {
            let want = if on { rp::ActiveCollisionTypes::empty() } else { rp::ActiveCollisionTypes::default() };
            if c.active_collision_types() != want {
                c.set_active_collision_types(want);
            }
        }
    }

    /// Applies what changed in the document since the last sync: moved
    /// parts are teleported, anything else rebuilds just their assemblies.
    pub fn sync(&mut self, doc: &mut Document) {
        self.server_moved.clear();
        self.reapply_groups(doc);
        let marks = doc.marks_for(doc::Reader::Physics);
        let Some(ws) = Self::workspace_of(doc) else {
            return;
        };
        let marks = match marks {
            doc::Marks::All => {
                let _p = crate::bench::scope("ps_reconcile");
                self.queued.clear();
                self.reconcile(doc);
                return;
            }
            doc::Marks::Some(m) => m,
        };
        // Decompositions that finished since: rebuild those parts with them.
        if !self.mesh_pending.is_empty()
            && self
                .mesh_checked
                .is_none_or(|t| t.elapsed().as_secs_f32() > 0.25)
        {
            self.mesh_checked = Some(std::time::Instant::now());
            let done: Vec<Ref> = self
                .mesh_pending
                .iter()
                .filter(|(_, (id, fid))| {
                    !matches!(
                        crate::collision::hulls(id, *fid),
                        crate::collision::Lookup::Pending
                    )
                })
                .map(|(r, _)| *r)
                .collect();
            for r in done {
                self.mesh_pending.remove(&r);
                self.queued.insert(r);
            }
        }
        let mut seeds: HashSet<Ref> = std::mem::take(&mut self.queued);
        let mut moved: Vec<Ref> = Vec::new();
        let cframe = ustr("CFrame");
        let structural = |n: &str| {
            matches!(
                n,
                "Anchored"
                    | "CanCollide"
                    | "Size"
                    | "Shape"
                    | "MeshId"
                    | "CollisionFidelity"
                    | "SolidMeshHolder"
                    | "MeshData2"
                    | "MeshData"
                    | "Part0"
                    | "Part1"
                    | "Attachment0"
                    | "Attachment1"
                    | "IsKinematic"
                    | "Enabled"
                    | "CollisionGroup"
                    | "CollisionGroupId"
            )
        };
        // Movers to re-read, attachments that moved between parts, terrain.
        let mut movers: HashSet<Ref> = HashSet::new();
        let mut attachments: HashSet<Ref> = HashSet::new();
        let mut terrain = false;
        // Queued refs lost their property names: re-check them all ways.
        for r in &seeds {
            self.note_other(doc, *r, &mut movers, &mut attachments, &mut terrain);
        }
        let marks_scope = crate::bench::scope("ps_marks");
        crate::bench::add("ps_mark_count", 0.0, marks.len() as u64);
        for m in marks {
            match m {
                doc::Mark::Prop(r, name) => {
                    if name == cframe {
                        if self.parts.contains_key(&r) {
                            moved.push(r);
                        }
                        continue;
                    }
                    if self.movers.contains_key(&r)
                        || doc.class_of(r).is_some_and(|c| is_mover_class(&c))
                    {
                        movers.insert(r);
                    } else if name == "SmoothGrid"
                        && (Some(r) == self.terrain
                            || doc.class_of(r).is_some_and(|c| c == "Terrain"))
                    {
                        terrain = true;
                    }
                    if structural(name.as_str()) {
                        if self.joints.contains_key(&r)
                            || doc.class_of(r).is_some_and(|c| is_rigid_joint(&c))
                        {
                            seeds.extend(self.read_joint(doc, ws, r));
                        } else {
                            seeds.insert(r);
                        }
                    }
                }
                doc::Mark::Tree(r) => {
                    for d in doc.subtree(r) {
                        self.note_other(doc, d, &mut movers, &mut attachments, &mut terrain);
                        if self.joints.contains_key(&d)
                            || doc.class_of(d).is_some_and(|c| is_rigid_joint(&c))
                        {
                            seeds.extend(self.read_joint(doc, ws, d));
                        } else {
                            seeds.insert(d);
                        }
                    }
                }
            }
        }
        drop(marks_scope);
        if !attachments.is_empty() {
            for (r, m) in &self.movers {
                if m.att0.is_some_and(|a| attachments.contains(&a))
                    || m.att1.is_some_and(|a| attachments.contains(&a))
                {
                    movers.insert(*r);
                }
            }
        }
        for r in movers {
            match read_mover(doc, ws, r) {
                Some(m) => {
                    self.movers.insert(r, m);
                }
                None => {
                    self.movers.remove(&r);
                }
            }
        }
        if terrain {
            let _p = crate::bench::scope("ps_terrain");
            self.build_terrain(doc);
        }
        seeds.retain(|r| {
            self.parts.contains_key(r)
                || doc.class_of(*r).is_some_and(|c| doc::is_a(&c, "BasePart"))
        });
        if !seeds.is_empty() {
            let _p = crate::bench::scope("ps_build");
            crate::bench::add("ps_seeds", 0.0, seeds.len() as u64);
            self.build(doc, ws, seeds.clone());
        }
        for r in moved {
            self.server_moved.insert(r);
            if seeds.contains(&r) {
                continue;
            }
            let Some(xf) = xform_of(doc, r) else { continue };
            self.teleport(r, &xf);
        }
    }

    /// Sorts an instance from a Tree mark (or a queued ref) into the mover,
    /// attachment and terrain work lists.
    fn note_other(
        &mut self,
        doc: &Document,
        r: Ref,
        movers: &mut HashSet<Ref>,
        attachments: &mut HashSet<Ref>,
        terrain: &mut bool,
    ) {
        let class = doc.class_of(r);
        if class.is_some_and(|c| c == "RopeConstraint" || c == "RodConstraint") {
            self.ropes.insert(r);
        } else if class.is_some_and(|c| joints::is_joint_constraint(&c)) {
            self.joint_cons.insert(r);
        }
        if self.movers.contains_key(&r) || class.is_some_and(|c| is_mover_class(&c)) {
            movers.insert(r);
        }
        // Removed instances have no class any more: they may be attachments.
        if class.is_none_or(|c| c == "Attachment" || c == "Bone") {
            attachments.insert(r);
        }
        if Some(r) == self.terrain || class.is_some_and(|c| c == "Terrain") {
            *terrain = true;
        }
    }

    /// How many body movers / mover constraints are tracked.
    pub fn mover_count(&self) -> usize {
        self.movers.len()
    }

    /// Triangles in the terrain colliders.
    pub fn terrain_triangles(&self) -> usize {
        self.terrain_tris
    }

    /// (Re)builds Workspace.Terrain's colliders: its non-water surface as fixed
    /// triangle meshes, one per TERRAIN_COLLIDER_CELL cube so each stays small.
    fn build_terrain(&mut self, doc: &Document) {
        // Same solid voxels as the colliders were made from: keep them.
        let terrain = Self::workspace_of(doc).and_then(|ws| doc.find_child(ws, "Terrain"));
        if let (Some(old), Some(t)) = (self.voxels.as_ref(), terrain) {
            if Some(t) == self.terrain {
                if let Some(Ok(v)) = terrain::Voxels::from_dom(&doc.dom) {
                    if v.same_solid(old) {
                        self.voxels = Some(v);
                        return;
                    }
                }
            }
        }
        for c in self.terrain_colliders.drain(..) {
            self.by_collider.remove(&c);
            self.world.remove_collider(c);
        }
        self.terrain_tris = 0;
        self.voxels = None;
        self.terrain = Self::workspace_of(doc).and_then(|ws| doc.find_child(ws, "Terrain"));
        let Some(t) = self.terrain else { return };
        let dec = crate::bench::scope("pt_decode");
        let voxels = match terrain::Voxels::from_dom(&doc.dom) {
            Some(Ok(v)) => v,
            Some(Err(e)) => {
                eprintln!("physics: terrain not loaded: {e}");
                return;
            }
            None => return,
        };
        if voxels.count() == 0 {
            return;
        }
        drop(dec);
        let mesh_scope = crate::bench::scope("pt_mesh");
        let (pos, idx) = terrain::collision_mesh(&terrain::build_meshes(&voxels));
        drop(mesh_scope);
        let _col = crate::bench::scope("pt_colliders");
        self.voxels = Some(voxels);
        // Bucket triangles by the cell holding their centroid.
        let cell = mech::TERRAIN_COLLIDER_CELL.max(terrain::VOXEL);
        let mut buckets: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        for tri in idx.chunks_exact(3) {
            let p = [0, 1, 2].map(|k| Vec3::from(pos[tri[k] as usize]));
            let c = (p[0] + p[1] + p[2]) / 3.0;
            let key = (c / cell).floor();
            buckets
                .entry([key.x as i32, key.y as i32, key.z as i32])
                .or_default()
                .extend_from_slice(tri);
        }
        let groups: Vec<Vec<u32>> = buckets.into_values().collect();
        use rapier3d::rayon::prelude::*;
        let built: Vec<(rp::ColliderBuilder, usize)> = groups
            .par_iter()
            .filter_map(|tris| {
                let mut remap: HashMap<u32, u32> = HashMap::new();
                let mut verts: Vec<rp::Vector> = Vec::new();
                let mut faces: Vec<[u32; 3]> = Vec::with_capacity(tris.len() / 3);
                for t in tris.chunks_exact(3) {
                    let f = [0, 1, 2].map(|k| {
                        *remap.entry(t[k]).or_insert_with(|| {
                            verts.push(rv(Vec3::from(pos[t[k] as usize])));
                            verts.len() as u32 - 1
                        })
                    });
                    faces.push(f);
                }
                let n = faces.len();
                // Fixing internal edges stops balls catching on triangle
                // seams; fall back to a plain mesh if the topology is odd.
                let flags = rapier3d::parry::shape::TriMeshFlags::FIX_INTERNAL_EDGES
                    | rapier3d::parry::shape::TriMeshFlags::DELETE_DEGENERATE_TRIANGLES;
                let b =
                    rp::ColliderBuilder::trimesh_with_flags(verts.clone(), faces.clone(), flags)
                        .or_else(|_| rp::ColliderBuilder::trimesh(verts, faces))
                        .ok()?;
                Some((b, n))
            })
            .collect();
        for (b, n) in built {
            let b = b
                .friction(mech::TERRAIN_FRICTION)
                .restitution(mech::TERRAIN_ELASTICITY)
                .active_events(rp::ActiveEvents::COLLISION_EVENTS);
            let c = self.world.insert_collider(b, None);
            self.by_collider.insert(c, t);
            self.terrain_colliders.push(c);
            self.terrain_tris += n;
        }
    }

    /// A part's pose: its body's for simulated parts, else the document's.
    fn part_xform(&self, doc: &Document, r: Ref) -> Option<Xform> {
        if let Some(pb) = self.parts.get(&r) {
            if let Some(b) = pb
                .asm
                .and_then(|id| self.asms.get(&id))
                .and_then(|a| self.world.bodies.get(a.body))
            {
                return Some(xform(&(*b.position() * pb.local)));
            }
        }
        xform_of(doc, r)
    }

    /// An attachment's world frame (its parent part's pose times its CFrame).
    fn attachment_xform(&self, doc: &Document, a: Ref) -> Option<Xform> {
        let part = doc.dom.get_by_ref(a)?.parent();
        let pxf = self.part_xform(doc, part)?;
        let local = match doc.prop(a, "CFrame") {
            Some(Variant::CFrame(cf)) => Xform::from_cframe(&cf).orthonormalized(),
            _ => Xform {
                pos: Vec3::ZERO,
                rot: Mat3::IDENTITY,
            },
        };
        Some(Xform {
            pos: pxf.pos + pxf.rot * local.pos,
            rot: pxf.rot * local.rot,
        })
    }

    /// Linear and angular velocity of the body an attachment's part rides on.
    fn attachment_motion(&self, doc: &Document, a: Ref) -> (Vec3, Vec3) {
        let part = doc
            .dom
            .get_by_ref(a)
            .map(|i| i.parent())
            .unwrap_or_else(Ref::none);
        match self.body_of(part).and_then(|h| self.world.bodies.get(h)) {
            Some(b) => (gv(b.linvel()), gv(b.angvel())),
            None => (Vec3::ZERO, Vec3::ZERO),
        }
    }

    fn frame_rot(&self, doc: &Document, m: &Mover, part: &Xform, frame: Frame) -> Option<Mat3> {
        Some(match frame {
            Frame::World => Mat3::IDENTITY,
            Frame::Part => part.rot,
            Frame::Att0 => self.attachment_xform(doc, m.att0?)?.rot,
            Frame::Att1 => self.attachment_xform(doc, m.att1?)?.rot,
        })
    }

    fn limit(&self, doc: &Document, m: &Mover, part: &Xform, f: Vec3, limit: Limit) -> Vec3 {
        match limit {
            Limit::None => f,
            Limit::Magnitude(max) => f.clamp_length_max(max),
            Limit::PerAxis(max, frame) => {
                let rot = self
                    .frame_rot(doc, m, part, frame)
                    .unwrap_or(Mat3::IDENTITY);
                rot * clamp_axes(rot.transpose() * f, max)
            }
        }
    }

    /// The world axes an unlimited LinearVelocity (ForceLimitsEnabled off, or
    /// infinite MaxForce) holds rigidly; None for limited ones and others.
    fn unlimited_velocity_axes(&self, doc: &Document, m: &Mover, part: &Xform) -> Option<Vec<Vec3>> {
        let Law::LinearVelocity {
            mode,
            line,
            t1,
            t2,
            planar_max,
            frame,
            limit,
            ..
        } = m.law
        else {
            return None;
        };
        let unlimited = match limit {
            Limit::None => true,
            Limit::Magnitude(x) => x.is_infinite(),
            Limit::PerAxis(v, _) => v.min_element().is_infinite(),
        } && planar_max.is_none_or(|p| p[0].is_infinite() && p[1].is_infinite());
        if !unlimited {
            return None;
        }
        let rot = self.frame_rot(doc, m, part, frame)?;
        Some(match mode {
            0 => vec![rot * line],
            1 => vec![rot * t1, rot * t2],
            _ => vec![Vec3::X, Vec3::Y, Vec3::Z],
        })
    }

    /// After a solver step: each unlimited LinearVelocity body's velocity
    /// lost along its axes to friction becomes next step's make-up (LvTrack).
    fn track_velocity_constraints(&mut self) {
        if self.lv_track.is_empty() {
            return;
        }
        let h = self.world.integration_parameters.dt;
        let cap = gv(self.world.gravity).length().max(1.0) * h * LV_MAKE_UP_MAX_G;
        let bodies: Vec<rp::RigidBodyHandle> = self.lv_track.keys().copied().collect();
        for body in bodies {
            let Some(b) = self.world.bodies.get(body) else {
                self.lv_track.remove(&body);
                continue;
            };
            let v = gv(b.linvel());
            // Contact normals: what holds the body back along them is the
            // contact (walls, slopes), not friction.
            let mut normals: Vec<Vec3> = Vec::new();
            for &c in b.colliders() {
                for pair in self.world.narrow_phase.contact_pairs_with(c) {
                    if !pair.has_any_active_contact() {
                        continue;
                    }
                    for mf in pair.manifolds() {
                        if mf.data.solver_contacts.is_empty() {
                            continue;
                        }
                        let mut n = gv(mf.data.normal);
                        for o in &normals {
                            n -= *o * o.dot(n);
                        }
                        if n.length_squared() > 1e-4 {
                            normals.push(n.normalize());
                        }
                    }
                }
            }
            let Some(t) = self.lv_track.get_mut(&body) else { continue };
            if normals.is_empty() {
                t.make_up = Vec3::ZERO;
                continue;
            }
            let loss = t.v_pred - v;
            let mut l: Vec3 = t.axes[..t.n].iter().map(|a| *a * a.dot(loss)).sum();
            for n in &normals {
                l -= *n * n.dot(l);
            }
            // (What's left may have tilted off the axes: keep it on them.)
            let l: Vec3 = t.axes[..t.n].iter().map(|a| *a * a.dot(l)).sum();
            t.make_up = if l.is_finite() { l.clamp_length_max(cap) } else { Vec3::ZERO };
        }
    }

    /// The (force, torque) a mover puts on its body this step. Forces act at
    /// the centre of mass unless the law names a point; `g` is gravity.
    fn mover_effort(
        &self,
        doc: &Document,
        m: &Mover,
        st: &BodyState,
        part: &Xform,
        g: Vec3,
        h: f32,
    ) -> Option<(Vec3, Vec3)> {
        let mass = st.mass;
        // Force that brings the body to `v_new` in one step, gravity included.
        let force_for = |v_new: Vec3| mass * (v_new - st.v) / h - mass * g;
        Some(match m.law {
            Law::Force {
                force,
                frame,
                at,
                at_att0,
            } => {
                let f = self.frame_rot(doc, m, part, frame)? * force;
                let point = match (at, at_att0) {
                    (Some(loc), _) => part.pos + part.rot * loc,
                    (None, true) => self.attachment_xform(doc, m.att0?)?.pos,
                    (None, false) => st.com,
                };
                (f, (point - st.com).cross(f))
            }
            Law::Torque { torque, frame } => {
                (Vec3::ZERO, self.frame_rot(doc, m, part, frame)? * torque)
            }
            Law::BodyVelocity { vel, max, p } => {
                // Velocity servo: force P·(goal − v) (P scaled by
                // BODY_MOVER_P_SCALE) solved implicitly, plus the weight it
                // holds up; capped per world axis by MaxForce.
                let p = p * mech::BODY_MOVER_P_SCALE;
                if p <= 0.0 {
                    return None;
                }
                let v_new = (mass * st.v / h + p * vel) / (mass / h + p);
                (clamp_axes(force_for(v_new), max), Vec3::ZERO)
            }
            Law::LinearVelocity {
                mode,
                vel,
                line,
                line_vel,
                plane_vel,
                t1,
                t2,
                planar_max,
                frame,
                limit,
            } => {
                // A velocity constraint: closes CONSTRAINT_RIGID_FRACTION of
                // the velocity error each step along its constrained axes.
                let rot = self.frame_rot(doc, m, part, frame)?;
                let k = mech::CONSTRAINT_RIGID_FRACTION;
                let along = |axis: Vec3, goal: f32| {
                    axis * (mass * k * (goal - st.v.dot(axis)) / h - mass * g.dot(axis))
                };
                let f = match mode {
                    0 => {
                        let d = (rot * line).normalize_or_zero();
                        if d == Vec3::ZERO {
                            return None;
                        }
                        self.limit(doc, m, part, along(d, line_vel), limit)
                    }
                    1 => {
                        let a = (rot * t1).normalize_or_zero();
                        let b = rot * t2;
                        let b = (b - a * a.dot(b)).normalize_or_zero();
                        if a == Vec3::ZERO || b == Vec3::ZERO {
                            return None;
                        }
                        let (fa, fb) = (along(a, plane_vel[0]), along(b, plane_vel[1]));
                        match (planar_max, limit) {
                            (Some(pm), _) => {
                                fa.clamp_length_max(pm[0]) + fb.clamp_length_max(pm[1])
                            }
                            (None, Limit::Magnitude(x)) => (fa + fb).clamp_length_max(x),
                            _ => fa + fb,
                        }
                    }
                    _ => {
                        let goal = rot * vel;
                        let f = mass * k * (goal - st.v) / h - mass * g;
                        self.limit(doc, m, part, f, limit)
                    }
                };
                (f, Vec3::ZERO)
            }
            Law::BodyPosition { pos, max, p, d } => {
                // Studio's BodyPosition closes the gap at a first-order
                // rate independent of mass, λ = 386·P / max(D, 196)²
                // (fitted to Studio 0.744: P 1e4 D 1250 → 2.47/s, P 4e4 →
                // 9.88, D 5000 → 0.154, P 1e3 D 100 → ≈10), holding the
                // part's weight; the force is capped per world axis by
                // MaxForce (a 4-stud cube at the default 4000 falls, as in
                // Studio). ReachedTarget: within 0.1 stud (
                // compares the squared gap with 0.01).
                let dd = d.max(196.0);
                let lambda = (386.0 * p.max(0.0) / (dd * dd)).min(0.9 / h);
                let e = pos - part.pos;
                (clamp_axes(force_for(e * lambda), max), Vec3::ZERO)
            }
            Law::AlignPosition {
                goal,
                limit,
                max_vel,
                resp,
                rigid,
            } => {
                // Critically damped spring with natural frequency
                // Responsiveness (1/s) on Attachment0's position, holding its
                // weight, speed capped by MaxVelocity, force by MaxForce.
                // RigidityEnabled: reach the goal in one step, no limits.
                let p0 = self.attachment_xform(doc, m.att0?)?.pos;
                let (target, vt) = match goal {
                    Some(p) => (p, Vec3::ZERO),
                    None => {
                        let a1 = m.att1?;
                        (
                            self.attachment_xform(doc, a1)?.pos,
                            self.attachment_motion(doc, a1).0,
                        )
                    }
                };
                let e = target - p0;
                if rigid {
                    (
                        force_for(vt + mech::CONSTRAINT_RIGID_FRACTION * e / h),
                        Vec3::ZERO,
                    )
                } else {
                    let vr = critical(st.v - vt, e, resp, h).clamp_length_max(max_vel);
                    (
                        self.limit(doc, m, part, force_for(vt + vr), limit),
                        Vec3::ZERO,
                    )
                }
            }
            Law::Rocket {
                target,
                offset,
                max_speed,
                max_thrust,
                max_torque,
                turn_p,
                turn_d,
                ..
            } => {
                let Some(state) = live::rocket(m.inst) else {
                    return None;
                };
                let goal = self.part_xform(doc, target).map(|x| x.pos + offset)?;
                let f = match state {
                    // Hovering where it reached the target.
                    Some(hold) => force_for((hold - part.pos) * 0.5 / h),
                    None => {
                        let dir = (goal - part.pos).normalize_or_zero();
                        force_for(dir * max_speed)
                    }
                };
                let f = f.clamp_length_max(max_thrust + mass * g.length());
                // Turn the front (−Z) toward the goal.
                let look = (goal - part.pos).normalize_or_zero();
                let t = if look != Vec3::ZERO {
                    let q = Quat::from_mat3(&part.rot).normalize();
                    let want = Quat::from_rotation_arc(q * Vec3::NEG_Z, look) * q;
                    let p = (turn_p * mech::BODY_MOVER_P_SCALE).max(0.0);
                    let d = (turn_d * mech::BODY_MOVER_P_SCALE).max(0.0);
                    clamp_axes(st.angular_servo(rot_error(q, want), p, d, 0.0, Vec3::ZERO, h), max_torque)
                } else {
                    Vec3::ZERO
                };
                (f, t)
            }
            Law::BodyGyro { rot, max, p, d } => {
                // Angular spring-damper: torque P·θ − D·ω (scaled by
                // BODY_MOVER_P_SCALE), solved implicitly with the world
                // inertia; capped per world axis by MaxTorque.
                let q = Quat::from_mat3(&part.rot).normalize();
                let theta = rot_error(q, rot);
                let p = (p * mech::BODY_MOVER_P_SCALE).max(0.0);
                let d = (d * mech::BODY_MOVER_P_SCALE).max(0.0);
                (
                    Vec3::ZERO,
                    clamp_axes(st.angular_servo(theta, p, d, 0.0, Vec3::ZERO, h), max),
                )
            }
            Law::AlignOrientation {
                goal,
                primary_only,
                max_torque,
                max_angvel,
                resp,
                rigid,
            } => {
                let q0 = Quat::from_mat3(&self.attachment_xform(doc, m.att0?)?.rot).normalize();
                let (qt, wt) = match goal {
                    Some(q) => (q, Vec3::ZERO),
                    None => {
                        let a1 = m.att1?;
                        (
                            Quat::from_mat3(&self.attachment_xform(doc, a1)?.rot).normalize(),
                            self.attachment_motion(doc, a1).1,
                        )
                    }
                };
                let theta = if primary_only {
                    let arc = Quat::from_rotation_arc(q0 * Vec3::X, qt * Vec3::X);
                    rot_error(Quat::IDENTITY, arc)
                } else {
                    rot_error(q0, qt)
                };
                if rigid {
                    (
                        Vec3::ZERO,
                        st.torque_for(wt + mech::CONSTRAINT_RIGID_FRACTION * theta / h, h),
                    )
                } else {
                    let wr = critical(st.w - wt, theta, resp, h).clamp_length_max(max_angvel);
                    (
                        Vec3::ZERO,
                        st.torque_for(wt + wr, h).clamp_length_max(max_torque),
                    )
                }
            }
            Law::BodyAngularVelocity { w, max, p } => {
                // Angular velocity servo: torque P·(goal − ω), implicit.
                let p = (p * mech::BODY_MOVER_P_SCALE).max(0.0);
                (
                    Vec3::ZERO,
                    clamp_axes(st.angular_servo(Vec3::ZERO, 0.0, 0.0, p, w, h), max),
                )
            }
            Law::AngularVelocity { w, frame, max } => {
                let goal = self.frame_rot(doc, m, part, frame)? * w;
                let wn = st.w + mech::CONSTRAINT_RIGID_FRACTION * (goal - st.w);
                (Vec3::ZERO, st.torque_for(wn, h).clamp_length_max(max))
            }
        })
    }

    /// Puts every mover's force and torque on its body for the next step.
    /// Forces from the previous step are cleared first; bodies that get a
    /// non-zero push are woken.
    fn apply_movers(&mut self, doc: &Document) {
        for h in self.pushed.drain() {
            if let Some(b) = self.world.bodies.get_mut(h) {
                b.reset_forces(false);
                b.reset_torques(false);
            }
        }
        for t in self.lv_track.values_mut() {
            t.live = false;
        }
        if self.movers.is_empty() {
            self.lv_track.clear();
            return;
        }
        let h = self.world.integration_parameters.dt;
        let g = gv(self.world.gravity);
        let mut out: Vec<(rp::RigidBodyHandle, Vec3, Vec3)> = Vec::new();
        let mut states: HashMap<rp::RigidBodyHandle, BodyState> = HashMap::new();
        let mut lv_axes: HashMap<rp::RigidBodyHandle, Vec<Vec3>> = HashMap::new();
        for m in self.movers.values() {
            let Some(pb) = self.parts.get(&m.part) else {
                continue;
            };
            let Some(body) = pb.asm.and_then(|id| self.asms.get(&id)).map(|a| a.body) else {
                continue;
            };
            let Some(b) = self.world.bodies.get(body) else {
                continue;
            };
            if !states.contains_key(&body) {
                let Some(st) = BodyState::of(b) else { continue };
                states.insert(body, st);
            }
            let st = &states[&body];
            let part = xform(&(*b.position() * pb.local));
            if let Some((f, t)) = self.mover_effort(doc, m, st, &part, g, h) {
                if f.is_finite() && t.is_finite() {
                    out.push((body, f, t));
                    if let Some(axes) = self.unlimited_velocity_axes(doc, m, &part) {
                        lv_axes.entry(body).or_default().extend(axes);
                    }
                }
            }
        }
        // Unlimited LinearVelocity: last step's friction loss made up, and
        // the velocity the push should give recorded for after the step.
        let mut totals: HashMap<rp::RigidBodyHandle, Vec3> = HashMap::new();
        for (body, f, _) in &out {
            *totals.entry(*body).or_default() += *f;
        }
        for (body, list) in lv_axes {
            let Some(st) = states.get(&body) else { continue };
            let mut axes = [Vec3::ZERO; 3];
            let mut n = 0;
            for a in list {
                let mut a = a;
                for b in &axes[..n] {
                    a -= *b * b.dot(a);
                }
                if n < 3 && a.length_squared() > 1e-6 {
                    axes[n] = a.normalize();
                    n += 1;
                }
            }
            let t = self.lv_track.entry(body).or_default();
            let make_up = if t.n == n { t.make_up } else { Vec3::ZERO };
            let f_make_up = st.mass * make_up / h;
            out.push((body, f_make_up, Vec3::ZERO));
            let total = totals.get(&body).copied().unwrap_or(Vec3::ZERO) + f_make_up;
            *t = LvTrack {
                axes,
                n,
                v_pred: st.v + (total / st.mass + g) * h,
                make_up,
                live: true,
            };
        }
        self.lv_track.retain(|_, t| t.live);
        for (body, f, t) in out {
            if let Some(b) = self.world.bodies.get_mut(body) {
                b.add_force(rv(f), true);
                b.add_torque(rv(t), true);
                self.pushed.insert(body);
            }
        }
    }

    /// A script set a part's CFrame: move its collider, or its whole assembly.
    fn teleport(&mut self, r: Ref, xf: &Xform) {
        let Some(pb) = self.parts.get(&r) else { return };
        let target = pose(xf);
        match pb.asm.and_then(|id| self.asms.get(&id)) {
            None => {
                // Fixed collider: keep the shape's own turn (cylinders).
                // A pose within `follower_eps` of where the collider is isn't
                // set again: setting it flags the collider modified (a
                // broad-phase leaf update every step). Place B's client moves
                // ~850 far-off NPC limbs by 0.0001–0.03 studs every frame.
                // Compared against the collider itself, so a slow real move
                // still lands once it passes the tolerance.
                let shape_offset = self.shape_offset(pb.collider);
                let (ep, er) = self.follower_eps;
                if let Some(c) = self.world.colliders.get_mut(pb.collider) {
                    let want = target * shape_offset;
                    let cur = *c.position();
                    let same = (cur.translation - want.translation).length_squared() < ep * ep
                        && cur.rotation.dot(want.rotation).abs() > 1.0 - er;
                    if !same {
                        c.set_position(want);
                    }
                }
            }
            Some(a) => {
                let body_pose = target * pb.local.inverse();
                let body = a.body;
                if let Some(b) = self.world.bodies.get_mut(body) {
                    let cur = xform(b.position());
                    let want = xform(&body_pose);
                    if (cur.pos - want.pos).length() > 1e-4 || !cur.rot.abs_diff_eq(want.rot, 1e-4)
                    {
                        b.set_position(body_pose, true);
                    }
                }
            }
        }
    }

    /// The cylinder's turn inside its part (identity for other shapes).
    fn shape_offset(&self, c: rp::ColliderHandle) -> rp::Pose {
        let Some(col) = self.world.colliders.get(c) else {
            return rp::Pose::IDENTITY;
        };
        if let Some(o) = self.union_offsets.get(&c) {
            *o
        } else if col.shape().as_cylinder().is_some() {
            rp::Pose::from_parts(
                rp::Vector::ZERO,
                rp::Rotation::from_rotation_z(-std::f32::consts::FRAC_PI_2),
            )
        } else {
            rp::Pose::IDENTITY
        }
    }

    fn body_of(&self, r: Ref) -> Option<rp::RigidBodyHandle> {
        let id = self.parts.get(&r)?.asm?;
        self.asms.get(&id).map(|a| a.body)
    }

    pub fn set_velocity(&mut self, r: Ref, v: Vec3) {
        // A part made this frame has no body until the next sync.
        if !self.parts.contains_key(&r) {
            self.pending_vel.insert(r, (v, 30));
            return;
        }
        if let Some(body) = self.body_of(r) {
            if let Some(b) = self.world.bodies.get_mut(body) {
                if b.is_dynamic() {
                    b.set_linvel(rv(v), true);
                    return;
                }
            }
        }
        // Anchored: kept for what stands on it (a conveyor).
        self.anchored_vel.insert(r, v);
    }

    /// Linear velocities of the parts on moving assemblies.
    pub fn velocities(&self) -> HashMap<Ref, Vec3> {
        let mut out = HashMap::new();
        for a in self.asms.values() {
            let Some(b) = self.world.bodies.get(a.body) else {
                continue;
            };
            let v = gv(b.linvel());
            for r in &a.parts {
                out.insert(*r, v);
            }
        }
        out
    }

    /// rapier's own per-stage timings of the last step (ms), plus how many
    /// movers and terrain triangles the world holds.
    pub fn stage_times(&self) -> String {
        let c = &self.world.physics_pipeline.counters;
        format!(
            "collision {:.2} islands {:.2} solver {:.2} ccd {:.2} total {:.2} movers {} terrain_tris {}",
            c.collision_detection_time_ms(),
            c.island_construction_time_ms(),
            c.solver_time_ms(),
            c.ccd_time_ms(),
            c.step_time_ms(),
            self.mover_count(),
            self.terrain_triangles()
        )
    }

    /// Dynamic assemblies that are awake right now.
    pub fn awake(&self) -> usize {
        self.asms
            .values()
            .filter(|a| {
                self.world
                    .bodies
                    .get(a.body)
                    .is_some_and(|b| !b.is_sleeping())
            })
            .count()
    }

    /// A few awake assemblies: (first part, position, velocity, part count).
    pub fn awake_sample(&self, n: usize) -> Vec<(Ref, Vec3, Vec3, usize)> {
        self.asms
            .values()
            .filter_map(|a| {
                let b = self.world.bodies.get(a.body)?;
                (!b.is_sleeping()).then(|| {
                    (
                        a.parts[0],
                        gv(b.translation()),
                        gv(b.linvel()),
                        a.parts.len(),
                    )
                })
            })
            .take(n)
            .collect()
    }

    /// How many dynamic assemblies and fixed colliders exist.
    pub fn counts(&self) -> (usize, usize) {
        (
            self.asms.len(),
            self.parts.values().filter(|p| p.asm.is_none()).count(),
        )
    }

    /// Workspace.Gravity as it is now. Roblox applies a script's change at
    /// once (Place A sets 64 = the source game's 800 units/s² from Server/Main after Play has
    /// started); read only at `new`, the change never reached the solver or
    /// the Humanoid, which then fell at the place's 196.2.
    pub fn sync_gravity(&mut self, doc: &Document) {
        let g = doc
            .workspace()
            .and_then(|w| f32_of(doc.prop(w, "Gravity")))
            .unwrap_or(mech::GRAVITY);
        if g.is_finite() && g != self.gravity {
            self.gravity = g;
            self.world.gravity = rp::Vector::new(0.0, -g, 0.0);
        }
    }

    /// Advances the simulation in fixed steps and writes moved parts' CFrames
    /// back into the document.
    pub fn step(&mut self, doc: &mut Document, dt: f32) -> StepResult {
        self.sync_gravity(doc);
        self.take_script_impulses(doc.id);
        for (r, (v, left)) in std::mem::take(&mut self.pending_vel) {
            if self.parts.contains_key(&r) {
                self.set_velocity(r, v);
            } else if left > 0 {
                self.pending_vel.insert(r, (v, left - 1));
            }
        }
        let mut out = StepResult::default();
        let h = 1.0 / mech::PHYSICS_HZ;
        self.accum = (self.accum + dt).min(h * mech::MAX_PHYSICS_STEPS as f32);
        let _solve = crate::bench::scope("physics_solve");
        let jsc = crate::bench::scope("ph_joints");
        self.apply_joints(doc);
        drop(jsc);
        while self.accum >= h - 1e-6 {
            let msc = crate::bench::scope("ph_movers");
            self.apply_movers(doc);
            self.apply_drags(doc.id);
            drop(msc);
            let rsc = crate::bench::scope("ph_ropes_springs");
            self.apply_ropes(doc, h);
            self.apply_springs(doc, h);
            drop(rsc);
            let wsc = crate::bench::scope("ph_world_step");
            self.world.step_with_events(&(), &self.collector);
            drop(wsc);
            // Stats.ContactsCount: collider pairs touching after the step.
            crate::runtime::domains::services::perf::set_contacts(doc.id, self.world.narrow_phase.contact_pairs().filter(|p| p.has_any_active_contact()).count());
            self.track_velocity_constraints();
            self.accum -= h;
            self.steps += 1;
        }
        drop(_solve);
        self.mover_events(doc);
        self.publish_live(doc.id);
        let events: Vec<_> =
            std::mem::take(&mut *self.collector.0.lock().unwrap_or_else(|e| e.into_inner()));
        for (a, b, started) in events {
            if let (Some(ra), Some(rb)) = (self.by_collider.get(&a), self.by_collider.get(&b)) {
                out.touches.push((*ra, *rb, started));
            }
        }
        let cframe = ustr("CFrame");
        for a in self.asms.values() {
            let Some(b) = self.world.bodies.get(a.body) else {
                continue;
            };
            if b.is_sleeping() {
                continue;
            }
            let body_pose = *b.position();
            for r in &a.parts {
                let Some(pb) = self.parts.get(r) else {
                    continue;
                };
                let xf = xform(&(body_pose * pb.local));
                if xf.pos.y < mech::FALLEN_PARTS_HEIGHT {
                    out.fallen.push(*r);
                }
                if let Some(inst) = doc.dom.get_by_ref_mut(*r) {
                    inst.properties
                        .insert(cframe, Variant::CFrame(xf.to_cframe()));
                }
                doc.mark_prop(*r, cframe);
                out.moved.push((*r, xf));
            }
        }
        // Our own CFrame writes aren't news to us.
        if !out.moved.is_empty() {
            if let doc::Marks::Some(rest) = doc.marks_for(doc::Reader::Physics) {
                let others: Vec<doc::Mark> = rest
                    .into_iter()
                    .filter(|m| !matches!(m, doc::Mark::Prop(_, n) if *n == cframe))
                    .collect();
                if !others.is_empty() {
                    // Keep anything else for the next sync.
                    for m in others {
                        match m {
                            doc::Mark::Prop(r, _) | doc::Mark::Tree(r) => {
                                self.queued.insert(r);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Moves the character capsule: walking, gravity and jumping, sliding
    /// along walls and stepping up small ledges. Returns the parts it touches.
    /// `own`: the character's own parts (tools and anything welded into it
    /// later), which never block it.
    pub fn move_character(
        &mut self,
        c: &mut CharBody,
        walk: Vec3,
        jump_velocity: Option<f32>,
        dt: f32,
        own: &HashSet<Ref>,
    ) -> Vec<Ref> {
        let shape = rp::Capsule::new_y(mech::CHARACTER_HALF_HEIGHT, mech::CHARACTER_RADIUS);
        if let (Some(v), true) = (jump_velocity, c.grounded) {
            c.vel_y = v;
            c.grounded = false;
        }
        // Average of start/end velocity is the exact path under constant
        // gravity; Studio steps at 240 Hz moving by the velocity before
        // gravity, which lands g·h/2 per second higher (its jump apex is
        // (1.06·v)²/2g + 1.06·v·h/2: 7.27 studs at JumpPower 50, 28.86 at 100).
        let v0 = c.vel_y;
        c.vel_y -= self.gravity * dt;
        let char_groups = self.interaction(&self.char_group);
        let step_lag = if c.grounded {
            0.0
        } else {
            self.gravity * 0.5 / mech::STUDIO_PHYSICS_HZ
        };
        let desired = walk * dt + Vec3::Y * ((v0 + c.vel_y) * 0.5 + step_lag) * dt;
        let at = rp::Pose::from_translation(rv(c.pos));
        let by_collider = &self.by_collider;
        let not_own = |h: rp::ColliderHandle, _: &rp::Collider| {
            by_collider.get(&h).is_none_or(|r| !own.contains(r))
        };
        let qp = self.world.query_pipeline_with_filter(
            rp::QueryFilter::default()
                .exclude_sensors()
                .groups(char_groups)
                .predicate(&not_own),
        );
        let mv = self
            .controller
            .move_shape(dt, &qp, &shape, &at, rv(desired), |_| {});
        let mut moved = gv(mv.translation);
        let mut grounded = mv.grounded;
        // Resting inside its own skin, the controller now and then refuses a
        // whole frame's walk on open floor (a lost frame of walking, which
        // the fidelity walk speed shows): try again from just above it.
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z).length();
        if flat(desired) > 1e-3 && flat(moved) < flat(desired) * 0.1 {
            let lift = Vec3::Y * mech::CHARACTER_HOVER;
            let at2 = rp::Pose::from_translation(rv(c.pos + lift));
            let mv2 = self
                .controller
                .move_shape(dt, &qp, &shape, &at2, rv(desired - lift), |_| {});
            let moved2 = gv(mv2.translation) + lift;
            if flat(moved2) > flat(moved) {
                moved = moved2;
                grounded = mv2.grounded;
            }
        }
        c.pos += moved;
        c.grounded = grounded;
        if (c.grounded && c.vel_y < 0.0) || (c.vel_y > 0.0 && moved.y < desired.y * 0.5) {
            c.vel_y = 0.0;
        }
        // Touches include sensors (CanCollide = false parts).
        let probe = rp::Capsule::new_y(
            mech::CHARACTER_HALF_HEIGHT + 0.1,
            mech::CHARACTER_RADIUS + 0.1,
        );
        let qp = self.world.query_pipeline();
        let at = rp::Pose::from_translation(rv(c.pos));
        let touched: HashSet<Ref> = qp
            .intersect_shape(at, &probe)
            .filter_map(|(h, _)| self.by_collider.get(&h).copied())
            .collect();
        touched.into_iter().collect()
    }

    /// A part's interaction groups from its CollisionGroup.
    fn part_groups(&self, doc: &Document, r: Ref) -> rp::InteractionGroups {
        let name = match doc.prop(r, "CollisionGroup") {
            Some(Variant::String(s)) if !s.is_empty() => s,
            _ => crate::collision_groups::DEFAULT.to_string(),
        };
        self.interaction(&name)
    }

    fn interaction(&self, group: &str) -> rp::InteractionGroups {
        let (m, f) = self.groups.lock().map(|g| g.bits(group)).unwrap_or((1, u32::MAX));
        rp::InteractionGroups::new(
            rp::Group::from_bits_truncate(m),
            rp::Group::from_bits_truncate(f),
            rp::InteractionTestMode::And,
        )
    }

    /// Scripts changed the collision groups: every collider takes its
    /// group's new bits.
    fn reapply_groups(&mut self, doc: &Document) {
        let v = self.groups.lock().map(|g| g.version).unwrap_or(0);
        if v == self.groups_seen {
            return;
        }
        self.groups_seen = v;
        let all: Vec<(Ref, rp::ColliderHandle)> =
            self.parts.iter().map(|(r, p)| (*r, p.collider)).collect();
        for (r, c) in all {
            let g = self.part_groups(doc, r);
            if let Some(col) = self.world.colliders.get_mut(c) {
                col.set_collision_groups(g);
            }
        }
    }

    /// Like `move_character` with gravity off: what a swimming Humanoid
    /// does (its own vertical speed is set by the caller).
    pub fn move_character_floating(
        &mut self,
        c: &mut CharBody,
        walk: Vec3,
        dt: f32,
        own: &HashSet<Ref>,
    ) -> Vec<Ref> {
        let g = std::mem::replace(&mut self.gravity, 0.0);
        let touched = self.move_character(c, walk, None, dt, own);
        self.gravity = g;
        touched
    }

    /// The solid terrain material under a point (the first solid voxel at
    /// most three voxels down), as Humanoid.FloorMaterial reports it.
    pub fn terrain_material_under(&self, p: Vec3) -> Option<u8> {
        let v = self.voxels.as_ref()?;
        let q = (p / terrain::VOXEL).floor();
        (0..=3).find_map(|dy| match v.get(q.x as i32, q.y as i32 - dy, q.z as i32) {
            (m, o) if m != 0 && m != terrain::WATER && o > 0.0 => Some(m),
            _ => None,
        })
    }

    /// Terrain water (0..1) at a point: the occupancy of the Water voxel
    /// holding it, shoreline water included; 0 without terrain.
    pub fn water_at(&self, p: Vec3) -> f32 {
        let Some(v) = &self.voxels else { return 0.0 };
        let q = (p / terrain::VOXEL).floor();
        v.water(q.x as i32, q.y as i32, q.z as i32)
    }

    /// The height of the water's surface above a point in water: the top of
    /// its column of water voxels, a part-filled top voxel's share of it.
    pub fn water_top(&self, p: Vec3) -> Option<f32> {
        let v = self.voxels.as_ref()?;
        let q = (p / terrain::VOXEL).floor();
        let (x, z) = (q.x as i32, q.z as i32);
        let mut y = q.y as i32;
        if v.water(x, y, z) <= mech::SWIM_ENTER_WATER {
            return None;
        }
        for _ in 0..64 {
            let o = v.water(x, y, z);
            if o < 0.999 {
                let bottom = y as f32 * terrain::VOXEL;
                return Some(if o > mech::SWIM_ENTER_WATER { bottom + o * terrain::VOXEL } else { bottom });
            }
            y += 1;
        }
        Some((y as f32) * terrain::VOXEL)
    }

    /// The first part a horizontal ray from `from` along `dir` hits within
    /// `dist` (the character's own parts and sensors left out).
    pub fn part_ahead(&self, from: Vec3, dir: Vec3, dist: f32, own: &HashSet<Ref>) -> Option<Ref> {
        let by_collider = &self.by_collider;
        let not_own = |h: rp::ColliderHandle, _: &rp::Collider| {
            by_collider.get(&h).is_none_or(|r| !own.contains(r))
        };
        let qp = self.world.query_pipeline_with_filter(
            rp::QueryFilter::default()
                .exclude_sensors()
                .predicate(&not_own),
        );
        let ray = rp::Ray::new(rv(from), rv(dir.normalize_or_zero()));
        qp.cast_ray(&ray, dist, true)
            .and_then(|(h, _)| self.by_collider.get(&h).copied())
    }

    /// The part the character stands on (Humanoid.FloorMaterial comes from
    /// it): a short ray down from the bottom of its capsule; None in the air.
    pub fn floor_under(&self, c: &CharBody, own: &HashSet<Ref>) -> Option<Ref> {
        if !c.grounded {
            return None;
        }
        let by_collider = &self.by_collider;
        let not_own = |h: rp::ColliderHandle, _: &rp::Collider| {
            by_collider.get(&h).is_none_or(|r| !own.contains(r))
        };
        let qp = self.world.query_pipeline_with_filter(
            rp::QueryFilter::default()
                .exclude_sensors()
                .predicate(&not_own),
        );
        let bottom = c.pos - Vec3::Y * (mech::CHARACTER_HALF_HEIGHT + mech::CHARACTER_RADIUS - 0.05);
        let ray = rp::Ray::new(rv(bottom), rv(Vec3::NEG_Y));
        qp.cast_ray(&ray, 0.6, true)
            .and_then(|(h, _)| self.by_collider.get(&h).copied())
    }
}

/// What Studio's Humanoid controller needs to know about its character
/// (round 22): HipHeight (R6's legs count as its hip), the root part's
/// size, the box its colliding parts fill (root frame, root at the origin;
/// its bottom is the root's, so anything lower than the hip passes under
/// it and the floor rays lift the character over it), and how
/// far the root sits above `CharBody::pos`.
impl Physics {
    /// Angular impulses scripts asked for (BasePart:ApplyAngularImpulse,
    /// ApplyImpulseAtPosition's turning part): added to the bodies' spin.
    fn take_script_impulses(&mut self, doc_id: u64) {
        let mut items: Vec<(Ref, Vec3, u8)> = live::take_spins(doc_id).into_iter().map(|(r, dw)| (r, dw, 0)).collect();
        items.extend(std::mem::take(&mut self.pending_spin).into_iter().map(|(r, (dw, n))| (r, dw, n)));
        for (r, dw, n) in items {
            // A part made this frame has no body yet: keep it a while.
            if !self.parts.contains_key(&r) {
                if n < 30 {
                    let e = self.pending_spin.entry(r).or_insert((Vec3::ZERO, n + 1));
                    e.0 += dw;
                }
                continue;
            }
            if let Some(body) = self.body_of(r) {
                if let Some(b) = self.world.bodies.get_mut(body) {
                    if b.is_dynamic() && dw.is_finite() {
                        let w = gv(b.angvel()) + dw;
                        b.set_angvel(rv(w), true);
                    }
                }
            }
        }
        // AssemblyAngularVelocity writes replace the spin (a part made
        // this frame starts still, so the whole value waits as a delta).
        for (r, w) in live::take_spin_sets(doc_id) {
            if !w.is_finite() {
                continue;
            }
            if !self.parts.contains_key(&r) {
                self.pending_spin.insert(r, (w, 1));
                continue;
            }
            if let Some(b) = self.body_of(r).and_then(|body| self.world.bodies.get_mut(body)) {
                if b.is_dynamic() {
                    b.set_angvel(rv(w), true);
                }
            }
        }
    }

    /// BodyPosition / RocketPropulsion ReachedTarget (Studio 0.705): a
    /// BodyPosition fires once its part comes within 0.1 stud of Position,
    /// again after Position changes; a fired RocketPropulsion fires once
    /// inside TargetRadius of its target, then hovers there.
    fn mover_events(&mut self, doc: &Document) {
        let mut reached = Vec::new();
        for m in self.movers.values() {
            match m.law {
                Law::BodyPosition { pos, .. } => {
                    let Some(at) = self.part_xform(doc, m.part).map(|x| x.pos) else { continue };
                    if live::bp_check(m.inst, pos, (at - pos).length() < 0.1) {
                        reached.push(m.inst);
                    }
                }
                Law::Rocket { target, offset, radius, .. } => {
                    let Some(Some(None)) = Some(live::rocket(m.inst)) else { continue };
                    let Some(at) = self.part_xform(doc, m.part).map(|x| x.pos) else { continue };
                    let Some(goal) = self.part_xform(doc, target).map(|x| x.pos + offset) else { continue };
                    if (goal - at).length() <= radius {
                        live::rocket_hold(m.inst, at);
                        reached.push(m.inst);
                    }
                }
                _ => {}
            }
        }
        live::push_reached(doc.id, reached);
    }

    /// DragDetector Physical drags (domains/world_drag.rs): each dragged
    /// part's grab point is pulled toward its goal as a critically damped
    /// spring of rate Responsiveness (not holding its weight), the force capped by
    /// MaxForce (at the centre of mass with ApplyAtCenterOfMass, else at the
    /// grab point); rotation styles also turn the part toward its goal
    /// orientation at that rate, the torque capped by MaxTorque.
    fn apply_drags(&mut self, doc_id: u64) {
        let drags = live::drags(doc_id);
        if drags.is_empty() {
            return;
        }
        let h = self.world.integration_parameters.dt;
        for (part, goal) in drags {
            let Some(pb) = self.parts.get(&part) else { continue };
            let local = pb.local;
            let Some(body) = self.body_of(part) else { continue };
            let Some(b) = self.world.bodies.get(body) else { continue };
            if !b.is_dynamic() {
                continue;
            }
            let Some(st) = BodyState::of(b) else { continue };
            let px = xform(&(*b.position() * local));
            let grab = px.apply(goal.grab_local);
            let at = if goal.at_com { st.com } else { grab };
            let v_at = st.v + st.w.cross(at - st.com);
            let w = goal.responsiveness.max(0.0);
            let v_new = critical(v_at, goal.goal - grab, w, h);
            // (The floor carries the weight: Studio's dragged box stays on
            // the ground and only turns about its up axis.)
            let mut f = st.mass * (v_new - v_at) / h;
            if f.length() > goal.max_force {
                f = f.normalize_or_zero() * goal.max_force;
            }
            let mut t = (at - st.com).cross(f);
            if let Some(q) = goal.goal_rot {
                let cur = Quat::from_mat3(&px.rot).normalize();
                let theta = rot_error(cur, q);
                let tw = st.torque_for(theta * w, h);
                t += tw.clamp_length_max(goal.max_torque);
            }
            if f.is_finite() && t.is_finite() {
                if let Some(b) = self.world.bodies.get_mut(body) {
                    b.add_force(rv(f), true);
                    b.add_torque(rv(t), true);
                    self.pushed.insert(body);
                }
            }
        }
    }

    /// What scripts read off the simulation (live::Stats) for this document.
    fn publish_live(&self, doc_id: u64) {
        let mut st = live::Stats::default();
        for a in self.asms.values() {
            let Some(b) = self.world.bodies.get(a.body) else { continue };
            if !b.is_dynamic() {
                continue;
            }
            if !b.is_sleeping() {
                st.awake_assemblies += 1;
                st.awake_parts += a.parts.len();
            }
            let w = gv(b.angvel());
            let com = gv(b.center_of_mass());
            for r in &a.parts {
                st.spin.insert(*r, (w, com));
            }
        }
        live::publish(doc_id, st);
    }
}

/// Simulation facts shared with the runtimes (src/domains/world.rs) and
/// script requests going the other way, keyed by the simulated document's
/// id (the server's DataModel in Play).
pub mod live {
    use super::*;

    #[derive(Default, Clone)]
    pub struct Stats {
        pub awake_assemblies: usize,
        pub awake_parts: usize,
        /// Part → (assembly angular velocity, assembly centre of mass).
        pub spin: HashMap<Ref, (Vec3, Vec3)>,
    }

    /// A DragDetector Physical drag's goal for one part.
    #[derive(Clone, Copy, Debug)]
    pub struct DragGoal {
        /// The grab point in the part's frame.
        pub grab_local: Vec3,
        /// Where the grab point should be (world).
        pub goal: Vec3,
        /// The part's goal orientation (rotation styles).
        pub goal_rot: Option<Quat>,
        pub max_force: f32,
        pub max_torque: f32,
        pub responsiveness: f32,
        pub at_com: bool,
    }

    #[derive(Default)]
    struct Shared {
        stats: HashMap<u64, Stats>,
        spins: HashMap<u64, Vec<(Ref, Vec3)>>,
        /// AssemblyAngularVelocity writes: the assembly's spin becomes this.
        spin_sets: HashMap<u64, Vec<(Ref, Vec3)>>,
        reached: HashMap<u64, Vec<Ref>>,
        /// RocketPropulsion fired: None flying, Some(point) hovering.
        rockets: HashMap<Ref, Option<Vec3>>,
        /// BodyPosition: the goal it last reached (no repeat until it changes).
        bp_done: HashMap<Ref, Vec3>,
        /// DragDetector Physical drags per document.
        drags: HashMap<u64, HashMap<Ref, DragGoal>>,
    }

    pub fn set_drag(doc: u64, part: Ref, goal: Option<DragGoal>) {
        let mut s = shared();
        match goal {
            Some(g) => {
                s.drags.entry(doc).or_default().insert(part, g);
            }
            None => {
                if let Some(m) = s.drags.get_mut(&doc) {
                    m.remove(&part);
                }
            }
        }
    }

    pub(super) fn drags(doc: u64) -> Vec<(Ref, DragGoal)> {
        shared().drags.get(&doc).map(|m| m.iter().map(|(k, v)| (*k, *v)).collect()).unwrap_or_default()
    }

    fn shared() -> std::sync::MutexGuard<'static, Shared> {
        static S: std::sync::OnceLock<Mutex<Shared>> = std::sync::OnceLock::new();
        S.get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn publish(doc: u64, st: Stats) {
        shared().stats.insert(doc, st);
    }

    /// The client's copy of the server's stats (Play): parts mapped to the
    /// client's refs, so its scripts read angular velocities and awake counts.
    pub fn mirror(from: u64, to: u64, s2c: &HashMap<Ref, Ref>) {
        let mut sh = shared();
        let Some(st) = sh.stats.get(&from) else { return };
        let copy = Stats {
            awake_assemblies: st.awake_assemblies,
            awake_parts: st.awake_parts,
            spin: st.spin.iter().filter_map(|(k, v)| s2c.get(k).map(|c| (*c, *v))).collect(),
        };
        sh.stats.insert(to, copy);
    }

    /// The client's own simulation on top of the mirror (Play): its
    /// parts' angular velocities and centres of mass (physics_client.rs).
    pub fn overlay_spins(doc: u64, spins: impl IntoIterator<Item = (Ref, (Vec3, Vec3))>) {
        let mut sh = shared();
        let st = sh.stats.entry(doc).or_default();
        st.spin.extend(spins);
    }

    pub fn with_stats<R>(doc: u64, f: impl FnOnce(&Stats) -> R) -> Option<R> {
        shared().stats.get(&doc).map(f)
    }

    pub fn queue_spin(doc: u64, r: Ref, dw: Vec3) {
        shared().spins.entry(doc).or_default().push((r, dw));
    }

    pub(super) fn take_spins(doc: u64) -> Vec<(Ref, Vec3)> {
        shared().spins.remove(&doc).unwrap_or_default()
    }

    /// A script set a part's AssemblyAngularVelocity (its assembly spins at
    /// `w` from the next step; an anchored part keeps the property instead).
    pub fn queue_spin_set(doc: u64, r: Ref, w: Vec3) {
        shared().spin_sets.entry(doc).or_default().push((r, w));
    }

    pub(super) fn take_spin_sets(doc: u64) -> Vec<(Ref, Vec3)> {
        shared().spin_sets.remove(&doc).unwrap_or_default()
    }

    pub(super) fn push_reached(doc: u64, v: Vec<Ref>) {
        if !v.is_empty() {
            shared().reached.entry(doc).or_default().extend(v);
        }
    }

    /// Movers that reached their target since the last call.
    pub fn take_reached(doc: u64) -> Vec<Ref> {
        shared().reached.remove(&doc).unwrap_or_default()
    }

    pub fn rocket_fire(r: Ref) {
        shared().rockets.insert(r, None);
    }

    pub fn rocket_abort(r: Ref) {
        shared().rockets.remove(&r);
    }

    /// None = not fired; Some(None) flying; Some(Some(p)) hovering at p.
    pub fn rocket(r: Ref) -> Option<Option<Vec3>> {
        shared().rockets.get(&r).copied()
    }

    pub(super) fn rocket_hold(r: Ref, at: Vec3) {
        shared().rockets.insert(r, Some(at));
    }

    /// True once per goal when `near`.
    pub(super) fn bp_check(r: Ref, goal: Vec3, near: bool) -> bool {
        let mut s = shared();
        if s.bp_done.get(&r).is_some_and(|g| *g != goal) {
            s.bp_done.remove(&r);
        }
        if near && !s.bp_done.contains_key(&r) {
            s.bp_done.insert(r, goal);
            return true;
        }
        false
    }

    /// A BodyPosition's Position was set: it may fire again (Studio fires
    /// even for a goal it already sits at).
    pub fn bp_rearm(r: Ref) {
        shared().bp_done.remove(&r);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HumShape {
    pub hip: f32,
    pub root_size: Vec3,
    pub box_lo: Vec3,
    pub box_hi: Vec3,
    pub root_lift: f32,
    /// The character's mass and how many parts it has (water drag).
    pub mass: f32,
    pub parts: u32,
    /// An R6 rig (Torso, no UpperTorso): its wall contact differs (round 26).
    pub r6: bool,
}

/// The floor the rays found: mean hit height, normal and point, and the
/// part of the first hit.
#[derive(Debug, Clone, Copy)]
pub struct HumFloor {
    pub y: f32,
    pub normal: Vec3,
    pub point: Vec3,
    pub part: Option<Ref>,
}

/// One frame of `move_humanoid`.
#[derive(Debug, Default)]
pub struct HumMove {
    pub touched: Vec<Ref>,
    /// Standing on a floor (rays) or resting on the body box.
    pub grounded: bool,
    pub jumped: bool,
    /// Still Jumping: holding the take-off speed against a wall.
    pub jumping: bool,
    pub floor: Option<Ref>,
    /// The root's velocity (what it really moved, per second).
    pub velocity: Vec3,
    /// A wall it walked into (horizontal normal, pointing out of the wall).
    pub wall: Option<Vec3>,
}

impl Physics {
    /// Studio's floor finder (, called by
    ///): straight down from the root's local
    /// (0, −0.4·size.y, k·0.4·size.z), k = 0, 1, −1, 2, −2 (
    ///); if none hits, from the corners (±0.4·size.x,
    /// ±0.4·size.z). Length 0.3 × (HipHeight + size.y)
    /// (FInt ExtraFloorRaycastDistanceHipHeightPercent = 30) + HipHeight ×
    /// 1.5 with a floor already, 1.1 without.
    /// Sizes count at least 0.05. The hits are averaged. A ray that starts
    /// inside a part doesn't hit that part (Roblox raycasts).
    pub fn find_floor(&self, root: Vec3, rot: Quat, sh: &HumShape, had_floor: bool, own: &HashSet<Ref>) -> Option<HumFloor> {
        let size = sh.root_size.max(Vec3::splat(mech::FLOOR_RAY_MIN_SIZE));
        let s = size * mech::FLOOR_RAY_SPREAD;
        let len = mech::FLOOR_RAY_EXTRA * (sh.hip + size.y)
            + sh.hip * if had_floor { mech::FLOOR_RAY_HIP_HELD } else { mech::FLOOR_RAY_HIP_FREE };
        let groups = self.interaction(&self.char_group);
        let by_collider = &self.by_collider;
        let cast = |o: Vec3| -> Option<(f32, Vec3, rp::ColliderHandle)> {
            let mut inside: Vec<rp::ColliderHandle> = Vec::new();
            for _ in 0..4 {
                let keep = |h: rp::ColliderHandle, _: &rp::Collider| {
                    by_collider.get(&h).is_none_or(|r| !own.contains(r)) && !inside.contains(&h)
                };
                let qp = self.world.query_pipeline_with_filter(
                    rp::QueryFilter::default().exclude_sensors().groups(groups).predicate(&keep),
                );
                let ray = rp::Ray::new(rv(o), rp::Vector::new(0.0, -1.0, 0.0));
                let (h, hit) = qp.cast_ray_and_get_normal(&ray, len, true)?;
                if hit.time_of_impact <= 1e-5 {
                    inside.push(h);
                    continue;
                }
                return Some((hit.time_of_impact, gv(hit.normal), h));
            }
            None
        };
        let mut hits = Vec::new();
        for k in mech::FLOOR_RAYS_ALONG {
            if let Some(h) = cast(root + rot * Vec3::new(0.0, -s.y, k * s.z)) {
                hits.push((root + rot * Vec3::new(0.0, -s.y, k * s.z), h));
            }
        }
        if hits.is_empty() {
            for (cx, cz) in mech::FLOOR_RAYS_CORNERS {
                let o = root + rot * Vec3::new(cx * s.x, -s.y, cz * s.z);
                if let Some(h) = cast(o) {
                    hits.push((o, h));
                }
            }
        }
        let first = hits.first()?.1 .2;
        let n = hits.len() as f32;
        let mut y = 0.0;
        let mut normal = Vec3::ZERO;
        let mut point = Vec3::ZERO;
        for (o, (t, nrm, _)) in &hits {
            y += o.y - t;
            normal += *nrm;
            point += *o - Vec3::Y * *t;
        }
        let normal = normal.normalize_or(Vec3::Y);
        Some(HumFloor { y: y / n, normal, point: point / n, part: self.by_collider.get(&first).copied() })
    }

    /// A floor part's velocity at a point: its body's if it moves, else what
    /// a script gave the anchored part (AssemblyLinearVelocity / Velocity,
    /// AssemblyAngularVelocity: a conveyor or a turntable).
    fn floor_velocity(&self, doc: &Document, part: Ref, at: Vec3) -> (Vec3, Vec3) {
        if let Some(b) = self.body_of(part).and_then(|b| self.world.bodies.get(b)) {
            if b.is_dynamic() {
                let w = gv(b.angvel());
                let c = gv(b.center_of_mass());
                return (gv(b.linvel()) + w.cross(at - c), w);
            }
        }
        let lin = self
            .anchored_vel
            .get(&part)
            .copied()
            .or_else(|| vec3_of(doc.prop(part, "AssemblyLinearVelocity")))
            .or_else(|| vec3_of(doc.prop(part, "Velocity")))
            .unwrap_or(Vec3::ZERO);
        let ang = vec3_of(doc.prop(part, "AssemblyAngularVelocity"))
            .or_else(|| vec3_of(doc.prop(part, "RotVelocity")))
            .unwrap_or(Vec3::ZERO);
        let c = xform_of(doc, part).map(|x| x.pos).unwrap_or(at);
        (lin + ang.cross(at - c), ang)
    }

    /// The friction between a character and its floor: Roblox's weighted
    /// mix (f₁w₁ + f₂w₂) / (w₁ + w₂), the character's parts being Plastic
    /// (0.3, weight 1).
    fn floor_friction(&self, doc: &Document, part: Ref) -> f32 {
        if doc.class_of(part).is_some_and(|c| c == "Terrain") {
            return (mech::TERRAIN_FRICTION + mech::PART_FRICTION) * 0.5;
        }
        let (f, w) = match doc.prop(part, "CustomPhysicalProperties") {
            Some(Variant::PhysicalProperties(rbx_dom_weak::types::PhysicalProperties::Custom(c))) => {
                (c.friction(), c.friction_weight())
            }
            _ => (mech::PART_FRICTION, 1.0),
        };
        (f * w + mech::PART_FRICTION) / (w + 1.0)
    }

    /// Studio's Humanoid on its feet or in the air (round 22): the floor
    /// rays each 1/240 s step (Studio's rate), a spring lifting the root to
    /// floor + HipHeight + half the root (v = 39 × the gap, fitted to
    /// Studio's step-ups and landings) while gravity alone brings it down,
    /// the walk turned into the floor's plane at full WalkSpeed, the floor's
    /// own velocity carried (and its spin), braking limited by friction,
    /// and the body box (yaw-turned) colliding and pushing loose parts.
    #[allow(clippy::too_many_arguments)]
    pub fn move_humanoid(
        &mut self,
        doc: &Document,
        c: &mut CharBody,
        yaw: &mut f32,
        sh: &HumShape,
        walk: Vec3,
        jump_velocity: Option<f32>,
        support: bool,
        dt: f32,
        own: &HashSet<Ref>,
    ) -> HumMove {
        let n = ((dt * mech::STUDIO_PHYSICS_HZ).ceil() as usize).clamp(1, 12);
        let h = dt / n as f32;
        let half = (sh.box_hi - sh.box_lo) * 0.5;
        let mid = (sh.box_hi + sh.box_lo) * 0.5;
        // The body box with its vertical edges cut off (round 26): at a wall's
        // end Studio's box pivots round the edge and gets free ~0.3 studs
        // before a square box would (movement-wall wall_jump_diag); the cut
        // edge slides round it the same way and its slanted normal turns the
        // body as the pivot does.
        let cut = mech::HUMANOID_BOX_CHAMFER.min(half.x.min(half.z) * 0.6);
        let mut hull = Vec::with_capacity(16);
        for sy in [-1.0f32, 1.0] {
            for (sx, sz) in [(1.0f32, 1.0f32), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                let y = sy * half.y;
                hull.push(rp::Vector::new(sx * half.x, y, sz * (half.z - cut)));
                hull.push(rp::Vector::new(sx * (half.x - cut), y, sz * half.z));
            }
        }
        let shape = rp::SharedShape::convex_hull(&hull).unwrap_or_else(|| rp::SharedShape::cuboid(half.x, half.y, half.z));
        let groups = self.interaction(&self.char_group);
        let mut out = HumMove::default();
        let start = c.pos;
        let mut touched: HashSet<Ref> = HashSet::new();
        let mut pushes: Vec<(rp::ColliderHandle, Vec3)> = Vec::new();
        let rest = sh.hip + sh.root_size.y * 0.5;
        for _ in 0..n {
            let root = c.pos + Vec3::Y * sh.root_lift;
            let rot = Quat::from_rotation_y(*yaw);
            // PlatformStand: no hip support, the body drops onto its box.
            let floor = support.then(|| self.find_floor(root, rot, sh, c.had_floor, own)).flatten();
            let target = floor.map(|f| f.y + rest);
            // Rising from a jump: the floor no longer holds it.
            if c.jump_rise && c.vel.y <= 0.0 {
                c.jump_rise = false;
            }
            let mut on_floor = target.is_some() && !c.jump_rise;
            if on_floor {
                c.air_time = 0.0;
            } else {
                c.air_time += h;
            }
            // Coyote time: just off a ledge (falling, not from a jump) a jump
            // still takes off.
            let coyote = !on_floor
                && !c.jump_rise
                && c.vel.y <= 0.0
                && c.air_time <= mech::HUMANOID_COYOTE_S;
            if let (Some(v), true, false) = (jump_velocity, on_floor || coyote, out.jumped) {
                c.vel.y = v;
                c.jump_rise = true;
                out.jumped = true;
                on_floor = false;
                // Against a wall (walking into it) the wall's friction keeps
                // Studio's Jumping step from reaching the take-off speed, so
                // it re-applies it each step (: until 1.06 ×
                // GetJumpVelocity − v·up ≤ 1e-6), and brakes the rise after.
                c.jump_speed = v;
                c.jump_hold = 0.0;
                c.wall_grip = 0.0;
                // Straight in Studio's Jumping still lasts one step (the body
                // bounces straight off), only the brake: wall_jump 0.013 s.
                if let Some(wn) = c.wall {
                    let flat = Vec3::new(walk.x, 0.0, walk.z);
                    let into = -flat.dot(wn);
                    if into > 0.0 {
                        let slide = (flat + wn * into).length();
                        if slide > mech::HUMANOID_JUMP_WALL_SLIDE_MIN * flat.length() {
                            c.jump_hold = mech::HUMANOID_JUMP_WALL_HOLD_S;
                        }
                        c.wall_grip = mech::HUMANOID_WALL_AIR_GRIP_S;
                    }
                }
            }
            if on_floor {
                c.jump_hold = 0.0;
                c.wall_grip = 0.0;
            }
            // Walking: on a floor, in its plane at the same speed; into a
            // wall, its friction slows the slide along it on a floor (Studio:
            // 8.62 instead of 11.31 studs/s at 45° into Plastic) and in the
            // air the push into it speeds the slide up (round 25).
            let mut want = Vec3::new(walk.x, 0.0, walk.z);
            let mut pressed = false;
            if let Some(wn) = c.wall.filter(|_| !c.walk_exact) {
                let into = -want.dot(wn);
                pressed = into > 0.0;
                if into > 0.0 {
                    let tangent = want + wn * into;
                    let t_len = tangent.length();
                    if t_len > 1e-4 {
                        let dir = tangent / t_len;
                        // Held flush the slide loses the wall's Coulomb
                        // friction (Plastic 0.3: corner 7.92, R6 wall_jump_diag
                        // after landing 2.87); walked into, the body bounces
                        // off it and loses less on average (round 25's 0.24).
                        let f = if c.wall_friction > 0.0 { c.wall_friction } else { mech::PART_FRICTION };
                        let mu = if c.wall_bounce { f * mech::HUMANOID_WALL_BOUNCE_FRICTION } else { f };
                        let goal = if on_floor {
                            (t_len - mu * into).max(0.0)
                        } else {
                            let push = if sh.r6 { mech::HUMANOID_WALL_AIR_PUSH_R6 } else { mech::HUMANOID_WALL_AIR_PUSH };
                            t_len + push * into
                        };
                        // The air's faster slide builds up at a limited rate,
                        // and wears off after landing. Right after a jump
                        // into the wall it waits: held Jumping it keeps the
                        // floor's slide, then the wall's grip slows it (Studio
                        // R6 2.89 for 0.04 s, ~20 studs/s² for 0.1 s, then
                        // up to 10.6 at ~75; round 26).
                        let have = Vec3::new(c.vel.x, 0.0, c.vel.z).dot(dir);
                        let rate = if on_floor {
                            mech::HUMANOID_WALL_LAND_DECEL
                        } else if c.jump_hold > 0.0 {
                            0.0
                        } else if c.wall_grip > 0.0 {
                            mech::HUMANOID_WALL_GRIP_SLIDE_ACCEL
                        } else {
                            mech::HUMANOID_WALL_AIR_ACCEL
                        };
                        let step = rate * h;
                        let along = if on_floor { goal.max(have - step) } else { goal.min(have.max(0.0) + step) };
                        want = dir * along - wn * into;
                    }
                }
            }
            let mut spin = 0.0;
            if let (true, Some(f), false) = (on_floor, floor, c.walk_exact) {
                let nrm = f.normal;
                if nrm.y > 1e-3 && want.length_squared() > 1e-8 {
                    let p = want - nrm * nrm.dot(want);
                    if p.length_squared() > 1e-8 {
                        let p = p.normalize() * want.length();
                        want = Vec3::new(p.x, 0.0, p.z);
                    }
                }
                if let Some(part) = f.part {
                    let (fv, w) = self.floor_velocity(doc, part, f.point);
                    want += Vec3::new(fv.x, 0.0, fv.z);
                    spin = w.y;
                }
            }
            let cur = Vec3::new(c.vel.x, 0.0, c.vel.z);
            let next = match (on_floor, floor) {
                _ if c.walk_exact => want,
                // On a floor the velocity changes at most as fast as friction
                // allows, speeding up and braking alike (Studio: 0 → 16 in
                // 0.13 s and back on Friction 0.05; at once on Plastic).
                (true, Some(f)) => {
                    let mu = f.part.map(|p| self.floor_friction(doc, p)).unwrap_or(mech::PART_FRICTION);
                    let a = mech::HUMANOID_BRAKE_BASE + mech::HUMANOID_BRAKE_PER_FRICTION * mu;
                    let d = want - cur;
                    let step = a * h;
                    if d.length() <= step { want } else { cur + d.normalize() * step }
                }
                // Wall slide above already limits acceleration, including
                // the Jumping hold/grip phases. Do not limit it a second time.
                _ if pressed => want,
                // Freefall applies a bounded force to close velocity error,
                // rather than replacing horizontal velocity each substep.
                // gain getter = 150, cap = 143.
                _ => {
                    let acceleration = ((want - cur) * mech::HUMANOID_VELOCITY_GAIN)
                        .clamp_length_max(mech::HUMANOID_AIR_ACCEL);
                    cur + acceleration * h
                },
            };
            c.vel.x = next.x;
            c.vel.z = next.z;
            *yaw += spin * h;
            // Up: the spring to the floor; down: gravity (moved by the
            // velocity before gravity, as Studio's 240 Hz step).
            let spring = matches!((on_floor, target), (true, Some(t)) if root.y < t);
            if spring {
                c.vel.y = mech::HUMANOID_ALTITUDE_GAIN * (target.unwrap_or(root.y) - root.y);
            }
            // Jumping into a wall: the take-off speed held (Jumping), then the
            // wall's friction braking the rise for a moment (round 26).
            let mut held = false;
            if !on_floor && c.jump_hold > 0.0 {
                if pressed {
                    c.vel.y = c.jump_speed;
                    held = true;
                    out.jumping = true;
                    c.jump_hold -= h;
                } else {
                    c.jump_hold = 0.0;
                }
            } else if !on_floor && c.wall_grip > 0.0 {
                if pressed && c.vel.y > 0.0 {
                    c.vel.y = (c.vel.y - mech::HUMANOID_WALL_AIR_GRIP * h).max(0.0);
                }
                c.wall_grip -= h;
            }
            let desired = c.vel * h;
            if !spring && !held {
                c.vel.y -= self.gravity * h;
            }
            // The body box.
            let center = root + rot * mid;
            let at = rp::Pose::from_parts(rv(center), yaw_rot(*yaw));
            let by_collider = &self.by_collider;
            let not_own = |hd: rp::ColliderHandle, _: &rp::Collider| by_collider.get(&hd).is_none_or(|r| !own.contains(r));
            let qp = self.world.query_pipeline_with_filter(
                rp::QueryFilter::default().exclude_sensors().groups(groups).predicate(&not_own),
            );
            // Of the walls it pushes into, the one it already faces most (in
            // an inside corner Studio's body stays flush to the wall it slid
            // along; the last contact rapier reports changes from step to
            // step and turned it to the other one: round 25, corner lane).
            let facing = rot * Vec3::NEG_Z;
            let walk_flat = Vec3::new(walk.x, 0.0, walk.z);
            let mut wall_now: Option<(Vec3, f32, rp::ColliderHandle)> = None;
            let colliders = &self.world.colliders;
            let mv = self.hum_controller.move_shape(h, &qp, &*shape, &at, rv(desired), |hit| {
                let nrm = -gv(hit.hit.normal1);
                pushes.push((hit.handle, nrm));
                // A box wall counts by the face it was met on: the cut edges
                // meeting a wall's end edge give slanted contact normals, which
                // would turn the body (two walls' ends either side of a gap).
                let nrm = match colliders.get(hit.handle).filter(|c| c.shape().as_cuboid().is_some()) {
                    Some(col) => {
                        let x = xform(col.position());
                        let l = x.rot.transpose() * nrm;
                        let a = l.abs();
                        let axis = if a.x >= a.y && a.x >= a.z {
                            Vec3::X * l.x.signum()
                        } else if a.y >= a.z {
                            Vec3::Y * l.y.signum()
                        } else {
                            Vec3::Z * l.z.signum()
                        };
                        x.rot * axis
                    }
                    None => nrm,
                };
                if nrm.y.abs() < mech::HUMANOID_WALL_NORMAL_Y {
                    let wn = Vec3::new(-nrm.x, 0.0, -nrm.z).normalize_or_zero();
                    let faced = -facing.dot(wn);
                    if walk_flat.dot(wn) < 0.0 && wall_now.is_none_or(|(_, f, _)| faced > f + 1e-3) {
                        wall_now = Some((wn, faced, hit.handle));
                    }
                }
            });
            let moved = gv(mv.translation);
            c.pos += moved;
            // A wall met on a floor is walked into (the body bounces off it);
            // one it already pressed into in the air it lands against flush.
            match wall_now {
                None => c.wall_bounce = false,
                Some((_, _, hd)) => {
                    if c.wall.is_none() {
                        c.wall_bounce = on_floor;
                        c.wall_friction = self
                            .by_collider
                            .get(&hd)
                            .map(|p| self.floor_friction(doc, *p))
                            .unwrap_or(mech::PART_FRICTION);
                    } else if !on_floor {
                        c.wall_bounce = false;
                    }
                }
            }
            c.wall = wall_now.map(|(wn, _, _)| wn);
            if c.wall.is_some() {
                out.wall = c.wall;
            }
            if desired.y > 1e-5 && moved.y < desired.y * 0.5 {
                // Head against a ceiling: bounces back down.
                c.vel.y = -mech::HUMANOID_CEILING_BOUNCE * c.vel.y.max(0.0);
            } else if desired.y < -1e-5 && moved.y > desired.y * 0.5 {
                // The body box rests on something.
                c.vel.y = c.vel.y.max(0.0);
                if !on_floor {
                    out.grounded = true;
                }
            }
            c.had_floor = on_floor;
            if on_floor {
                out.grounded = true;
                out.floor = floor.and_then(|f| f.part);
            } else if !out.grounded {
                out.floor = None;
            }
            let _ = &qp;
        }
        // The last step decides.
        let root = c.pos + Vec3::Y * sh.root_lift;
        let floor = c.had_floor.then(|| self.find_floor(root, Quat::from_rotation_y(*yaw), sh, true, own)).flatten();
        out.grounded = c.had_floor || out.grounded && c.vel.y.abs() < 1e-3;
        out.floor = floor.and_then(|f| f.part).or(if out.grounded { out.floor } else { None });
        out.velocity = (c.pos - start) / dt.max(1e-6);
        c.grounded = out.grounded;
        // Loose parts it walked into are pushed along at its speed.
        let push_v = Vec3::new(c.vel.x, 0.0, c.vel.z);
        for (hd, dir) in pushes {
            let Some(body) = self.world.colliders.get(hd).and_then(|col| col.parent()) else { continue };
            let Some(b) = self.world.bodies.get_mut(body) else { continue };
            if !b.is_dynamic() {
                continue;
            }
            let d = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
            let along = push_v.dot(d);
            if along <= 0.0 {
                continue;
            }
            let v = gv(b.linvel());
            let have = v.dot(d);
            if have < along {
                b.set_linvel(rv(v + d * (along - have)), true);
            }
        }
        // Touches include sensors (CanCollide = false parts), and reach down
        // to the feet: the legs touch what they don't collide with (a lava
        // pad on the floor fires Touched in Studio).
        let lo = Vec3::new(sh.box_lo.x, sh.box_lo.y - sh.hip, sh.box_lo.z);
        let t_half = (sh.box_hi - lo) * 0.5;
        let t_mid = (sh.box_hi + lo) * 0.5;
        let probe = rp::Cuboid::new(rv(t_half + Vec3::splat(0.1)));
        let qp = self.world.query_pipeline();
        let at = rp::Pose::from_parts(rv(root + Quat::from_rotation_y(*yaw) * t_mid), yaw_rot(*yaw));
        touched.extend(qp.intersect_shape(at, &probe).filter_map(|(hd, _)| self.by_collider.get(&hd).copied()));
        // Its floor too (the box floats above it).
        if let Some(f) = out.floor {
            touched.insert(f);
        }
        out.touched = touched.into_iter().collect();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::types::{CFrame, Vector3};
    use rbx_dom_weak::InstanceBuilder;

    /// Terrain water is found by point (what starts Humanoid Swimming), and
    /// a floating move doesn't fall.
    #[test]
    fn water_and_floating() {
        let mut doc = Document::new_place();
        let mut p = Physics::new(&mut doc);
        let mut v = terrain::Voxels::empty();
        for x in 0..4 {
            for y in 0..3 {
                for z in 0..4 {
                    v.set([x, y, z], terrain::WATER, 1.0);
                }
            }
        }
        p.voxels = Some(v);
        assert!(p.water_at(Vec3::new(6.0, 6.0, 6.0)) > 0.99);
        assert_eq!(p.water_at(Vec3::new(6.0, 13.0, 6.0)), 0.0);
        assert_eq!(p.water_at(Vec3::new(-1.0, 6.0, 6.0)), 0.0);
        // The surface over a point: the column's top (three voxels: 12).
        assert_eq!(p.water_top(Vec3::new(6.0, 1.0, 6.0)), Some(12.0));
        assert_eq!(p.water_top(Vec3::new(6.0, 13.0, 6.0)), None);
        let mut c = CharBody {
            pos: Vec3::new(6.0, 60.0, 6.0),
            vel_y: 0.0,
            grounded: false,
            ..Default::default()
        };
        let g = p.gravity;
        for _ in 0..30 {
            p.move_character_floating(&mut c, Vec3::X * 10.0, 1.0 / 60.0, &HashSet::new());
        }
        assert!((c.pos.y - 60.0).abs() < 1e-3, "no gravity while floating: {c:?}");
        assert!((c.pos.x - 11.0).abs() < 0.1, "walks: {c:?}");
        assert_eq!(p.gravity, g);
    }

    fn hum_shape_r15() -> HumShape {
        HumShape {
            hip: 2.19,
            root_size: Vec3::new(2.0, 2.0, 1.0),
            box_lo: Vec3::new(-1.0, -1.0, -0.58),
            box_hi: Vec3::new(1.0, 2.27, 0.58),
            root_lift: 0.0,
            mass: 14.0,
            parts: 16,
            r6: false,
        }
    }

    #[test]
    fn floor_reach_clamps_small_root_height() {
        let (_, p) = hum_world(vec![]);
        let mut sh = hum_shape_r15();
        sh.root_size = Vec3::splat(0.01);
        sh.hip = 0.0;
        // Origin is .02 below the root, reach is .3 * .05 = .015.
        assert!(p.find_floor(Vec3::new(60.0, 0.03, 60.0), Quat::IDENTITY, &sh, false, &HashSet::new()).is_some());
        assert!(p.find_floor(Vec3::new(60.0, 0.04, 60.0), Quat::IDENTITY, &sh, false, &HashSet::new()).is_none());
    }

    /// A world with one big floor part (top at y = 0) and what else is given.
    fn hum_world(extra: Vec<InstanceBuilder>) -> (Document, Physics) {
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        if let Some(bp) = doc.find_named(ws, "Baseplate") {
            doc.dom.destroy(bp);
        }
        doc.dom.insert(
            ws,
            part("Floor", Vec3::new(0.0, -1.0, 0.0), true).with_property("Size", Vector3::new(400.0, 2.0, 400.0)),
        );
        for b in extra {
            doc.dom.insert(ws, b);
        }
        let mut p = Physics::new(&mut doc);
        p.sync(&mut doc);
        (doc, p)
    }

    fn run_hum(doc: &Document, p: &mut Physics, c: &mut CharBody, yaw: &mut f32, walk: Vec3, secs: f32) -> HumMove {
        let sh = hum_shape_r15();
        let mut last = HumMove::default();
        for _ in 0..(secs * 60.0) as usize {
            last = p.move_humanoid(doc, c, yaw, &sh, walk, None, true, 1.0 / 60.0, &HashSet::new());
        }
        last
    }

    /// Studio's rest height (HipHeight + half the root) on a flat floor,
    /// steps up to the hip climbed, higher ones stop it (round 22).
    #[test]
    fn humanoid_stands_and_steps_like_studio() {
        let step = |h: f32| {
            part("Step", Vec3::new(60.0, h / 2.0, 44.0), true).with_property("Size", Vector3::new(16.0, h, 20.0))
        };
        let (doc, mut p) = hum_world(vec![]);
        let mut c = CharBody { pos: Vec3::new(60.0, 4.0, 60.0), ..Default::default() };
        let mut yaw = 0.0;
        let m = run_hum(&doc, &mut p, &mut c, &mut yaw, Vec3::ZERO, 1.0);
        assert!((c.pos.y - 3.19).abs() < 0.02 && m.grounded, "rests at {:?}", c.pos);
        for (h, climbs) in [(1.0, true), (2.0, true), (2.5, false)] {
            let (doc, mut p) = hum_world(vec![step(h)]);
            let mut c = CharBody { pos: Vec3::new(60.0, 3.2, 60.0), ..Default::default() };
            let mut yaw = 0.0;
            run_hum(&doc, &mut p, &mut c, &mut yaw, Vec3::new(0.0, 0.0, -16.0), 1.5);
            assert_eq!(c.pos.y > 3.19 + h - 0.1, climbs, "step {h}: {:?}", c.pos);
        }
    }

    /// A Plane LinearVelocity's walk (Place A's pmove) goes up a ramp at exactly
    /// its horizontal velocity, sideways part included: turned into the
    /// ramp's plane it came back slower with the sideways part grown, which
    /// Place A fed back in (Map 3's spawn stairs: 15 → 9 studs/s, into the
    /// side wall).
    #[test]
    fn plane_velocity_walk_climbs_a_ramp_exactly() {
        let wedge = InstanceBuilder::new("WedgePart")
            .with_name("Ramp")
            .with_property("Anchored", true)
            .with_property("Size", Vector3::new(16.0, 8.0, 12.0))
            .with_property("CFrame", CFrame::new(Vector3::new(60.0, 4.0, 40.0), CFrame::identity().orientation));
        let top = part("Top", Vec3::new(60.0, 4.0, 56.0), true).with_property("Size", Vector3::new(16.0, 8.0, 20.0));
        let (doc, mut p) = hum_world(vec![wedge, top]);
        let mut c = CharBody { pos: Vec3::new(60.0, 3.2, 28.0), walk_exact: true, ..Default::default() };
        let mut yaw = std::f32::consts::PI;
        let walk = Vec3::new(0.6, 0.0, 15.0);
        let sh = hum_shape_r15();
        let mut on_ramp = 0;
        for _ in 0..90 {
            let m = p.move_humanoid(&doc, &mut c, &mut yaw, &sh, walk, None, true, 1.0 / 60.0, &HashSet::new());
            if c.pos.z > 35.0 && c.pos.z < 45.0 {
                on_ramp += 1;
                let flat = Vec3::new(m.velocity.x, 0.0, m.velocity.z);
                assert!((flat - walk).length() < 0.05, "held to the walk on the ramp: {m:?} at {:?}", c.pos);
            }
        }
        assert!(on_ramp > 20, "crossed the ramp ({on_ramp} frames)");
        assert!(c.pos.y > 8.0 + 3.0, "up on top: {:?}", c.pos);
    }

    /// Jumping into a wall while holding forward: the body slides down it
    /// and lands (round 23: it hung on the wall).
    #[test]
    fn humanoid_falls_down_walls() {
        let wall = part("Wall", Vec3::new(63.5, 15.0, 60.0), true).with_property("Size", Vector3::new(1.0, 30.0, 40.0));
        let (doc, mut p) = hum_world(vec![wall]);
        // Mid-air against the wall, facing it, holding forward (+X).
        let mut c = CharBody { pos: Vec3::new(62.0, 12.0, 60.0), vel: Vec3::new(0.0, 10.0, 0.0), ..Default::default() };
        let mut yaw = -std::f32::consts::FRAC_PI_2;
        let walk = Vec3::new(16.0, 0.0, 0.0);
        let m = run_hum(&doc, &mut p, &mut c, &mut yaw, walk, 2.0);
        assert!(c.pos.y < 3.5 && m.grounded, "still on the wall: {:?} {:?}", c.pos, m);
    }

    /// Jumping while walking into a wall at an angle stays Jumping for a
    /// moment and peaks higher than a free jump; straight in, one step and
    /// lower (Studio: wall_jump_diag 8.0–8.4 (R6) / 8.7–9.1 (R15), wall_jump
    /// 6.55–6.76, free 7.27; round 26).
    #[test]
    fn humanoid_jumps_at_walls() {
        // walk: 0 = none (free), 1 = straight in, 2 = 26.6° off the normal.
        let peak = |walk_kind: u8| {
            let wall = part("Wall", Vec3::new(63.5, 15.0, 60.0), true).with_property("Size", Vector3::new(1.0, 30.0, 80.0));
            let (doc, mut p) = hum_world(if walk_kind > 0 { vec![wall] } else { vec![] });
            let mut c = CharBody { pos: Vec3::new(61.5, 3.2, 60.0), ..Default::default() };
            let mut yaw = -std::f32::consts::FRAC_PI_2;
            let walk = match walk_kind {
                0 => Vec3::ZERO,
                1 => Vec3::new(16.0, 0.0, 0.0),
                _ => Vec3::new(16.0, 0.0, 8.0).normalize() * 16.0,
            };
            run_hum(&doc, &mut p, &mut c, &mut yaw, walk, 0.5);
            let y0 = c.pos.y;
            let sh = hum_shape_r15();
            let mut top = y0;
            let mut held = 0;
            for i in 0..60 {
                let jump = (i == 0).then_some(53.0);
                let m = p.move_humanoid(&doc, &mut c, &mut yaw, &sh, walk, jump, true, 1.0 / 60.0, &HashSet::new());
                held += m.jumping as i32;
                top = top.max(c.pos.y);
            }
            (top - y0, held)
        };
        let (free, free_held) = peak(0);
        let (straight, straight_held) = peak(1);
        let (diag, diag_held) = peak(2);
        assert!(free_held == 0 && straight_held == 0 && diag_held >= 1, "Jumping held {free_held} / {straight_held} / {diag_held} frames");
        assert!((free - 7.27).abs() < 0.2, "free peak {free}");
        assert!(straight < free - 0.4 && straight > 6.2, "straight-in peak {straight}");
        assert!(diag > free + 0.6 && diag < 9.0, "diagonal peak {diag}");
    }

    /// Walking at 45° into a wall: flush to it, sliding at Studio's mean 8.62 (wall_slide).
    #[test]
    fn humanoid_slides_along_walls() {
        let wall = part("Wall", Vec3::new(63.5, 5.0, 22.0), true).with_property("Size", Vector3::new(1.0, 10.0, 80.0));
        let (doc, mut p) = hum_world(vec![wall]);
        let mut c = CharBody { pos: Vec3::new(60.0, 3.2, 60.0), ..Default::default() };
        let mut yaw = -std::f32::consts::FRAC_PI_2; // facing the wall, as AutoRotate leaves it
        let walk = Vec3::new(1.0, 0.0, -1.0).normalize() * 16.0;
        run_hum(&doc, &mut p, &mut c, &mut yaw, walk, 0.5);
        let z0 = c.pos.z;
        let m = run_hum(&doc, &mut p, &mut c, &mut yaw, walk, 1.0);
        let speed = z0 - c.pos.z;
        assert!(m.wall.is_some(), "wall seen");
        assert!((c.pos.x - 62.5).abs() < 0.1, "flush: {:?}", c.pos);
        assert!((speed - 8.62).abs() < 0.3, "slides at {speed}");
    }

    #[test]
    fn jump_reaches_jump_height() {
        let mut doc = Document::new_place();
        let mut p = Physics::new(&mut doc);
        let dt = 1.0 / 60.0;
        let mut c = CharBody {
            pos: Vec3::new(40.0, 2.6, 40.0),
            vel_y: 0.0,
            grounded: false,
            ..Default::default()
        };
        for _ in 0..60 {
            p.move_character(&mut c, Vec3::ZERO, None, dt, &HashSet::new());
            p.step(&mut doc, dt);
        }
        assert!(c.grounded, "settled: {c:?}");
        let ground = c.pos.y;
        let v = (2.0 * p.gravity * mech::JUMP_HEIGHT).sqrt();
        let mut top = ground;
        let mut trace = Vec::new();
        p.move_character(&mut c, Vec3::ZERO, Some(v), dt, &HashSet::new());
        for _ in 0..40 {
            p.move_character(&mut c, Vec3::ZERO, None, dt, &HashSet::new());
            p.step(&mut doc, dt);
            top = top.max(c.pos.y);
            trace.push((c.pos.y - ground, c.vel_y));
        }
        let h = top - ground;
        assert!(
            (h - mech::JUMP_HEIGHT).abs() < 0.4,
            "jump {h:.2}: {trace:?}"
        );
    }

    fn part(name: &str, pos: Vec3, anchored: bool) -> InstanceBuilder {
        InstanceBuilder::new("Part")
            .with_name(name)
            .with_property("Anchored", anchored)
            .with_property("Size", Vector3::new(2.0, 2.0, 2.0))
            .with_property(
                "CFrame",
                CFrame::new(
                    Vector3::new(pos.x, pos.y, pos.z),
                    CFrame::identity().orientation,
                ),
            )
    }

    const DT: f32 = 1.0 / 60.0;

    fn run(p: &mut Physics, doc: &mut Document, steps: usize) {
        for _ in 0..steps {
            p.sync(doc);
            p.step(doc, DT);
        }
    }

    fn pos_of(doc: &Document, r: Ref) -> Vec3 {
        xform_of(doc, r).unwrap().pos
    }

    fn vel_of(p: &Physics, r: Ref) -> Vec3 {
        p.velocities().get(&r).copied().unwrap_or(Vec3::ZERO)
    }

    fn v3(v: Vec3) -> Vector3 {
        Vector3::new(v.x, v.y, v.z)
    }

    /// Place B's spawn Teleporter: a LocalScript turns a 1×1×1 part at the
    /// spawn into a 2048×1×2048 plate under the map. The collider follows
    /// the client's Size and pose, also from an All journal read, and comes
    /// back when the client agrees with the server again.
    #[test]
    fn client_resized_anchored_part_collides_where_the_client_has_it() {
        let mut server = Document::new_place();
        let sws = server.workspace().unwrap();
        let s = server.dom.insert(sws, part("Teleporter", Vec3::new(0.0, 150.0, 0.0), true));
        let mut client = Document::new_place();
        let cws = client.workspace().unwrap();
        let c = client.dom.insert(cws, part("Teleporter", Vec3::new(0.0, 150.0, 0.0), true));
        let c2s: HashMap<Ref, Ref> = [(c, s)].into();
        let mut p = Physics::new(&mut server);
        let aabb = |p: &Physics| {
            let col = &p.world.colliders[p.parts[&s].collider];
            let a = col.compute_aabb();
            (gv(a.mins), gv(a.maxs))
        };
        let size = ustr("Size");
        let cf = ustr("CFrame");
        client.set_prop_live(c, size, Variant::Vector3(Vector3::new(2048.0, 1.0, 2048.0)));
        client.set_prop_live(
            c,
            cf,
            Variant::CFrame(CFrame::new(Vector3::new(65.0, -3.5, 336.0), CFrame::identity().orientation)),
        );
        // First read of the client journal is All.
        p.follow_client(&server, &mut client, &c2s);
        let (lo, hi) = aabb(&p);
        assert!((hi.x - lo.x - 2048.0).abs() < 0.5 && (hi.y + lo.y) * 0.5 < 0.0, "{lo:?} {hi:?}");
        // A rebuild from the server's copy keeps the client's collider.
        p.refresh([s]);
        p.sync(&mut server);
        let (lo, hi) = aabb(&p);
        assert!((hi.x - lo.x - 2048.0).abs() < 0.5, "after rebuild {lo:?} {hi:?}");
        // Back to the server's size and pose: the collider follows.
        client.set_prop_live(c, size, Variant::Vector3(Vector3::new(2.0, 2.0, 2.0)));
        client.set_prop_live(
            c,
            cf,
            Variant::CFrame(CFrame::new(Vector3::new(0.0, 150.0, 0.0), CFrame::identity().orientation)),
        );
        p.follow_client(&server, &mut client, &c2s);
        let (lo, hi) = aabb(&p);
        assert!((hi.x - lo.x - 2.0).abs() < 0.1 && (lo.y - 149.0).abs() < 0.1, "restored {lo:?} {hi:?}");
    }

    #[test]
    fn mesh_part_collides_by_its_decomposition() {
        use rbx_dom_weak::types::{ContentId, Enum};
        let id = "rbxtemp://rustudio/test/physics-l.obj";
        crate::assets::register_generated(id, crate::collision::l_prism_obj().into_bytes());
        std::env::set_var(
            "RUSTUDIO_CACHE_DIR",
            std::env::temp_dir().join("rustudio-physics-mesh-test"),
        );
        let _ = crate::collision::wait_ready(
            id,
            crate::collision::FIDELITY_PRECISE,
            std::time::Duration::from_secs(60),
        );
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        doc.dom.insert(
            ws,
            InstanceBuilder::new("MeshPart")
                .with_name("L")
                .with_property("Anchored", true)
                .with_property("Size", Vector3::new(8.0, 8.0, 8.0))
                .with_property(
                    "CFrame",
                    CFrame::new(
                        Vector3::new(10.0, 4.01, 0.0),
                        CFrame::identity().orientation,
                    ),
                )
                .with_property("MeshId", ContentId::from(id))
                .with_property("CollisionFidelity", Enum::from_u32(3)),
        );
        // A box dropped from just above the L's top into its empty corner
        // lands on the corner's floor (the bar's top at the part's middle).
        let ball = floating(&mut doc, Vec3::new(12.0, 9.5, 0.0));
        let mut p = Physics::new(&mut doc);
        run(&mut p, &mut doc, 240);
        let at = pos_of(&doc, ball);
        assert!(
            (at.y - 5.0).abs() < 0.6,
            "the box should rest on the bar of the L at y≈5, got {at:?}"
        );
    }

    #[test]
    fn union_collision_fidelity_preserves_empty_corner() {
        use rbx_dom_weak::types::{BinaryString, Enum};
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let union = doc.dom.insert(ws, InstanceBuilder::new("UnionOperation")
            .with_property("Anchored", true)
            .with_property("Size", Vector3::new(8.0, 8.0, 8.0))
            .with_property("CFrame", CFrame::new(Vector3::new(10.0, 4.01, 0.0), CFrame::identity().orientation))
            .with_property("MeshData", BinaryString::from(crate::collision::l_prism_obj().into_bytes()))
            .with_property("CollisionFidelity", Enum::from_u32(3)));
        let (_, sig) = part_sig(&doc, union).unwrap();
        let id = sig.mesh.as_deref().expect("union must supply collision geometry");
        assert!(matches!(crate::collision::wait_ready(id, 3, std::time::Duration::from_secs(60)), crate::collision::Lookup::Ready(_)));
        let ball = floating(&mut doc, Vec3::new(12.0, 9.5, 0.0));
        let mut physics = Physics::new(&mut doc);
        run(&mut physics, &mut doc, 240);
        let decomposed_y = pos_of(&doc, ball).y;
        assert!((decomposed_y - 5.0).abs() < 0.6, "union corner: {decomposed_y}");
        doc.set_prop_live(union, ustr("CollisionFidelity"), Variant::Enum(Enum::from_u32(2)));
        let (_, boxed) = part_sig(&doc, union).unwrap();
        assert_eq!(boxed.fidelity, crate::collision::FIDELITY_BOX);
        assert!(mesh_shape(&boxed).is_none());
        doc.set_prop_live(ball, ustr("CFrame"), Variant::CFrame(CFrame::new(
            Vector3::new(12.0, 12.0, 0.0), CFrame::identity().orientation)));
        run(&mut physics, &mut doc, 240);
        let boxed_y = pos_of(&doc, ball).y;
        assert!((boxed_y - 9.01).abs() < 0.2, "changed fidelity must rebuild: {boxed_y}");
    }

    /// A free 2×2×2 part (mass 0.7 × 8 = 5.6) high above the baseplate.
    fn floating(doc: &mut Document, at: Vec3) -> Ref {
        let ws = doc.workspace().unwrap();
        doc.dom.insert(ws, part("Box", at, false))
    }

    #[test]
    fn body_position_hovers_near_its_target() {
        let mut doc = Document::new_place();
        let a = floating(&mut doc, Vec3::new(0.0, 20.0, 0.0));
        let goal = Vec3::new(3.0, 26.0, -2.0);
        doc.dom.insert(
            a,
            InstanceBuilder::new("BodyPosition").with_property("Position", v3(goal)),
        );
        let mut p = Physics::new(&mut doc);
        assert_eq!(p.mover_count(), 1);
        run(&mut p, &mut doc, 180);
        let at = pos_of(&doc, a);
        // Studio holds the weight: it sits on the goal (ReachedTarget needs
        // 0.1 stud).
        assert!(
            (at - goal).length() < 0.05,
            "hovers at {at:?}, goal {goal:?}"
        );
        assert!(vel_of(&p, a).length() < 0.5, "settled: {:?}", vel_of(&p, a));
        // Still holding a few seconds later (forces keep being applied).
        run(&mut p, &mut doc, 120);
        assert!((pos_of(&doc, a) - goal).length() < 0.5);
        // Anchored: the mover does nothing.
        doc.set_prop_live(a, ustr("Anchored"), Variant::Bool(true));
        doc.set_prop_live(
            a,
            ustr("CFrame"),
            Variant::CFrame(CFrame::new(
                Vector3::new(0.0, 40.0, 0.0),
                CFrame::identity().orientation,
            )),
        );
        run(&mut p, &mut doc, 30);
        assert_eq!(pos_of(&doc, a), Vec3::new(0.0, 40.0, 0.0));
    }

    #[test]
    fn body_velocity_drives_at_target_velocity() {
        let mut doc = Document::new_place();
        let a = floating(&mut doc, Vec3::new(0.0, 30.0, 0.0));
        let mut p = Physics::new(&mut doc);
        assert_eq!(p.mover_count(), 0);
        // Added mid-run: picked up from the journal.
        let bv = doc.dom.insert(
            a,
            InstanceBuilder::new("BodyVelocity")
                .with_property("Velocity", Vector3::new(12.0, 0.0, -5.0)),
        );
        doc.mark_tree(bv);
        run(&mut p, &mut doc, 60);
        assert_eq!(p.mover_count(), 1);
        let v = vel_of(&p, a);
        assert!(
            (v - Vec3::new(12.0, 0.0, -5.0)).length() < 0.3,
            "velocity {v:?}"
        );
        // Changing Velocity is seen next step.
        doc.set_prop_live(
            bv,
            ustr("Velocity"),
            Variant::Vector3(Vector3::new(0.0, 4.0, 0.0)),
        );
        run(&mut p, &mut doc, 30);
        let v = vel_of(&p, a);
        assert!(
            (v - Vec3::new(0.0, 4.0, 0.0)).length() < 0.3,
            "velocity {v:?}"
        );
        // Removing it lets the part fall.
        doc.dom.transfer_within(bv, Ref::none());
        doc.mark_tree(bv);
        run(&mut p, &mut doc, 20);
        assert_eq!(p.mover_count(), 0);
        assert!(vel_of(&p, a).y < -20.0, "falls: {:?}", vel_of(&p, a));
    }

    #[test]
    fn linear_velocity_holds_vector_velocity() {
        let mut doc = Document::new_place();
        let a = floating(&mut doc, Vec3::new(0.0, 30.0, 0.0));
        let att = doc.dom.insert(a, InstanceBuilder::new("Attachment"));
        doc.dom.insert(
            a,
            InstanceBuilder::new("LinearVelocity")
                .with_property("Attachment0", att)
                .with_property("VectorVelocity", Vector3::new(0.0, 0.0, 8.0))
                .with_property("MaxForce", 1.0e6f32),
        );
        let mut p = Physics::new(&mut doc);
        run(&mut p, &mut doc, 30);
        let v = vel_of(&p, a);
        assert!(
            (v - Vec3::new(0.0, 0.0, 8.0)).length() < 0.2,
            "velocity {v:?}"
        );
    }

    /// Place A's Movement drive: an unlimited planar LinearVelocity on a box
    /// resting on a Plastic floor holds 10 exactly (rapier's friction took
    /// ~10% off a plain push), and covers the distance at that speed.
    #[test]
    fn unlimited_linear_velocity_beats_floor_friction() {
        use rbx_dom_weak::types::{Enum, Vector2};
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        doc.dom.insert(
            ws,
            part("Floor", Vec3::new(0.0, -1.0, 0.0), true).with_property("Size", Vector3::new(400.0, 2.0, 400.0)),
        );
        let a = floating(&mut doc, Vec3::new(0.0, 1.0, 0.0));
        let att = doc.dom.insert(a, InstanceBuilder::new("Attachment"));
        doc.dom.insert(
            a,
            InstanceBuilder::new("LinearVelocity")
                .with_property("Attachment0", att)
                .with_property("RelativeTo", Enum::from_u32(2))
                .with_property("VelocityConstraintMode", Enum::from_u32(1))
                .with_property("PrimaryTangentAxis", Vector3::new(1.0, 0.0, 0.0))
                .with_property("SecondaryTangentAxis", Vector3::new(0.0, 0.0, 1.0))
                .with_property("PlaneVelocity", Vector2::new(10.0, 0.0))
                .with_property("ForceLimitsEnabled", false),
        );
        let mut p = Physics::new(&mut doc);
        run(&mut p, &mut doc, 20);
        let x0 = pos_of(&doc, a).x;
        let n = 120;
        run(&mut p, &mut doc, n);
        let v = vel_of(&p, a);
        assert!((v.x - 10.0).abs() < 0.02 && v.y.abs() < 0.1, "velocity {v:?}");
        let moved = pos_of(&doc, a).x - x0;
        let want = 10.0 * DT * n as f32;
        assert!((moved - want).abs() < 0.05 * want, "moved {moved}, want {want}");
    }

    #[test]
    fn align_position_one_attachment_pulls_to_position() {
        let mut doc = Document::new_place();
        let a = floating(&mut doc, Vec3::new(0.0, 20.0, 0.0));
        let att = doc.dom.insert(a, InstanceBuilder::new("Attachment"));
        let goal = Vec3::new(10.0, 24.0, 5.0);
        doc.dom.insert(
            a,
            InstanceBuilder::new("AlignPosition")
                .with_property("Attachment0", att)
                .with_property("Mode", rbx_dom_weak::types::Enum::from_u32(0))
                .with_property("Position", v3(goal))
                .with_property("MaxForce", 1.0e5f32),
        );
        let mut p = Physics::new(&mut doc);
        run(&mut p, &mut doc, 180);
        let at = pos_of(&doc, a);
        assert!((at - goal).length() < 0.3, "at {at:?}, goal {goal:?}");
    }

    #[test]
    fn vector_force_equal_to_weight_floats() {
        let mut doc = Document::new_place();
        let a = floating(&mut doc, Vec3::new(0.0, 20.0, 0.0));
        let att = doc.dom.insert(a, InstanceBuilder::new("Attachment"));
        let vf = doc.dom.insert(
            a,
            InstanceBuilder::new("VectorForce")
                .with_property("Attachment0", att)
                .with_property("RelativeTo", rbx_dom_weak::types::Enum::from_u32(2))
                .with_property("ApplyAtCenterOfMass", true),
        );
        let mut p = Physics::new(&mut doc);
        let mass = mech::PART_DENSITY * 8.0;
        doc.set_prop_live(
            vf,
            ustr("Force"),
            Variant::Vector3(Vector3::new(0.0, mass * p.gravity, 0.0)),
        );
        run(&mut p, &mut doc, 120);
        let v = vel_of(&p, a);
        assert!(v.length() < 0.05, "floats: {v:?}");
        assert!(
            (pos_of(&doc, a).y - 20.0).abs() < 0.05,
            "{:?}",
            pos_of(&doc, a)
        );
    }

    #[test]
    fn body_gyro_keeps_orientation() {
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let tilt = Xform {
            pos: Vec3::new(0.0, 25.0, 0.0),
            rot: Mat3::from_euler(glam::EulerRot::XYZ, 0.5, 0.3, -0.4),
        };
        let a = doc.dom.insert(
            ws,
            part("Box", Vec3::ZERO, false).with_property("CFrame", tilt.to_cframe()),
        );
        doc.dom.insert(
            a,
            InstanceBuilder::new("BodyPosition")
                .with_property("Position", Vector3::new(0.0, 25.0, 0.0)),
        );
        doc.dom.insert(
            a,
            InstanceBuilder::new("BodyGyro")
                .with_property(
                    "CFrame",
                    CFrame::new(Vector3::new(0.0, 0.0, 0.0), CFrame::identity().orientation),
                )
                .with_property("MaxTorque", Vector3::new(4.0e5, 4.0e5, 4.0e5)),
        );
        let mut p = Physics::new(&mut doc);
        // Kick it spinning too.
        let body = p.body_of(a).unwrap();
        p.world
            .bodies
            .get_mut(body)
            .unwrap()
            .set_angvel(rv(Vec3::new(3.0, -4.0, 2.0)), true);
        run(&mut p, &mut doc, 180);
        let rot = xform_of(&doc, a).unwrap().rot;
        let angle = rot_error(Quat::from_mat3(&rot), Quat::IDENTITY)
            .length()
            .to_degrees();
        assert!(angle < 2.0, "upright within {angle:.2}°");
    }

    #[test]
    fn align_orientation_and_angular_velocity() {
        let mut doc = Document::new_place();
        let a = floating(&mut doc, Vec3::new(0.0, 25.0, 0.0));
        let att = doc.dom.insert(a, InstanceBuilder::new("Attachment"));
        doc.dom.insert(
            a,
            InstanceBuilder::new("BodyPosition")
                .with_property("Position", Vector3::new(0.0, 25.0, 0.0)),
        );
        let goal = Xform {
            pos: Vec3::ZERO,
            rot: Mat3::from_rotation_y(1.0),
        };
        doc.dom.insert(
            a,
            InstanceBuilder::new("AlignOrientation")
                .with_property("Attachment0", att)
                .with_property("Mode", rbx_dom_weak::types::Enum::from_u32(0))
                .with_property("CFrame", goal.to_cframe())
                .with_property("MaxTorque", 1.0e6f32),
        );
        let b = floating(&mut doc, Vec3::new(20.0, 25.0, 0.0));
        let att_b = doc.dom.insert(b, InstanceBuilder::new("Attachment"));
        doc.dom.insert(
            b,
            InstanceBuilder::new("BodyPosition")
                .with_property("Position", Vector3::new(20.0, 25.0, 0.0)),
        );
        doc.dom.insert(
            b,
            InstanceBuilder::new("AngularVelocity")
                .with_property("Attachment0", att_b)
                .with_property("RelativeTo", rbx_dom_weak::types::Enum::from_u32(2))
                .with_property("AngularVelocity", Vector3::new(0.0, 2.0, 0.0))
                .with_property("MaxTorque", 1.0e6f32),
        );
        let mut p = Physics::new(&mut doc);
        assert_eq!(p.mover_count(), 4);
        run(&mut p, &mut doc, 180);
        let rot = xform_of(&doc, a).unwrap().rot;
        let err = rot_error(Quat::from_mat3(&rot), Quat::from_mat3(&goal.rot))
            .length()
            .to_degrees();
        assert!(err < 2.0, "aligned within {err:.2}°");
        let body = p.body_of(b).unwrap();
        let w = gv(p.world.bodies.get(body).unwrap().angvel());
        assert!(
            (w - Vec3::new(0.0, 2.0, 0.0)).length() < 0.05,
            "spins at {w:?}"
        );
    }

    /// A SmoothGrid with one 32³ chunk at the origin whose bottom `layers`
    /// voxel layers are solid Rock (material 8): x, z 0..128 studs.
    fn slab_grid(layers: usize) -> Vec<u8> {
        let mut b = vec![1u8, 5];
        b.extend_from_slice(&[0; 12]);
        // Cells run x fastest, then z, then y; runs of 256.
        let solid = layers * 32 * 32;
        for _ in 0..solid / 256 {
            // Grass: no shift jitter, so the top is flat.
            b.extend_from_slice(&[0x80 | 2, 255]);
        }
        for _ in 0..(32 * 32 * 32 - solid) / 256 {
            b.extend_from_slice(&[0x80, 255]);
        }
        b
    }

    /// Run with: cargo test --release --bin rustudio terrain_collider_perf -- --ignored --nocapture
    #[test]
    #[ignore]
    fn terrain_collider_perf() {
        // 16×16 chunks (2048×2048 studs), two solid layers each.
        let mut b = vec![1u8, 5];
        let one = slab_grid(2);
        let mut prev = [0i32; 3];
        for cx in 0..16 {
            for cz in 0..16 {
                let k = [cx, 0, cz];
                let d = [0, 1, 2].map(|a| (k[a] - prev[a]).to_be_bytes());
                for byte in 0..4 {
                    for x in &d {
                        b.push(x[byte]);
                    }
                }
                prev = k;
                b.extend_from_slice(&one[14..]);
            }
        }
        let mut doc = Document::new_place();
        set_smooth_grid(&mut doc, rbx_dom_weak::types::BinaryString::from(b));
        let t = std::time::Instant::now();
        let p = Physics::new(&mut doc);
        println!(
            "{} terrain triangles in {} colliders: {:?}",
            p.terrain_triangles(),
            p.terrain_colliders.len(),
            t.elapsed()
        );
    }

    /// Fills the place's own Terrain (every place has one) with `grid`.
    fn set_smooth_grid(doc: &mut Document, grid: rbx_dom_weak::types::BinaryString) -> Ref {
        let ws = doc.workspace().unwrap();
        let t = doc.find_named(ws, "Terrain").unwrap();
        doc.dom
            .get_by_ref_mut(t)
            .unwrap()
            .properties
            .insert(rbx_dom_weak::ustr("SmoothGrid"), Variant::BinaryString(grid));
        t
    }

    #[test]
    fn ball_comes_to_rest_on_terrain() {
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let bp = doc.find_named(ws, "Baseplate").unwrap();
        doc.dom.destroy(bp);
        let grid = rbx_dom_weak::types::BinaryString::from(slab_grid(2));
        let t = set_smooth_grid(&mut doc, grid);
        let ball = doc.dom.insert(
            ws,
            part("Ball", Vec3::new(64.0, 30.0, 64.0), false)
                .with_property("Shape", rbx_dom_weak::types::Enum::from_u32(0))
                .with_property("Size", Vector3::new(4.0, 4.0, 4.0)),
        );
        let mut p = Physics::new(&mut doc);
        assert!(p.terrain_triangles() > 0);
        let mut touched_terrain = false;
        for _ in 0..240 {
            p.sync(&mut doc);
            let out = p.step(&mut doc, DT);
            touched_terrain |= out
                .touches
                .iter()
                .any(|(x, y, s)| *s && ((*x == ball && *y == t) || (*x == t && *y == ball)));
        }
        let at = pos_of(&doc, ball);
        // Two solid layers (0..8): the surface half a voxel above them at
        // y = 10 (Studio's slab top, round 21), the ball's radius is 2.
        assert!((at.y - 12.0).abs() < 0.3, "rests at {at:?}");
        assert!(
            vel_of(&p, ball).length() < 0.5,
            "at rest: {:?}",
            vel_of(&p, ball)
        );
        assert!(touched_terrain, "Touched reports the Terrain");
        // Clearing the grid removes the colliders (journal mark on SmoothGrid).
        doc.set_prop_live(
            t,
            ustr("SmoothGrid"),
            Variant::BinaryString(Vec::new().into()),
        );
        p.sync(&mut doc);
        assert_eq!(p.terrain_triangles(), 0);
        run(&mut p, &mut doc, 30);
        assert!(
            pos_of(&doc, ball).y < 5.0,
            "falls through: {:?}",
            pos_of(&doc, ball)
        );
    }

    #[test]
    fn welded_parts_fall_as_one_assembly_and_anchoring_holds_them() {
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let a = doc
            .dom
            .insert(ws, part("A", Vec3::new(0.0, 30.0, 0.0), false));
        let b = doc
            .dom
            .insert(ws, part("B", Vec3::new(2.0, 30.0, 0.0), false));
        let w = doc.dom.insert(
            a,
            InstanceBuilder::new("WeldConstraint")
                .with_property("Part0", a)
                .with_property("Part1", b),
        );
        let mut p = Physics::new(&mut doc);
        assert_eq!(p.counts().0, 1, "one dynamic assembly");
        for _ in 0..30 {
            p.sync(&mut doc);
            p.step(&mut doc, 1.0 / 60.0);
        }
        let ya = xform_of(&doc, a).unwrap().pos;
        let yb = xform_of(&doc, b).unwrap().pos;
        assert!(ya.y < 29.0, "fell: {ya:?}");
        assert!(
            ((yb - ya).length() - 2.0).abs() < 0.01,
            "stayed welded: {ya:?} {yb:?}"
        );
        // Anchoring one part anchors the assembly.
        doc.set_prop_live(b, ustr("Anchored"), Variant::Bool(true));
        p.sync(&mut doc);
        assert_eq!(
            p.counts(),
            (0, 4),
            "A and B fixed, plus baseplate and spawn"
        );
        let before = xform_of(&doc, a).unwrap().pos;
        for _ in 0..10 {
            p.sync(&mut doc);
            p.step(&mut doc, 1.0 / 60.0);
        }
        assert_eq!(xform_of(&doc, a).unwrap().pos, before, "held in place");
        // Removing the weld lets A fall alone.
        doc.dom.transfer_within(w, Ref::none());
        doc.mark_tree(w);
        p.sync(&mut doc);
        assert_eq!(p.counts(), (1, 3));
    }

    /// A live Team Create place (and a file) keeps a WeldConstraint's parts
    /// as Part0Internal / Part1Internal and its Enabled as State: Studio
    /// saves 3 enabled / 0 disabled, the live wire sends 1 (Place E's back rod
    /// fell apart when only Part0 / Enabled were read).
    #[test]
    fn weld_constraint_internal_parts_and_state() {
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let a = doc.dom.insert(ws, part("A", Vec3::new(0.0, 30.0, 0.0), false));
        let b = doc.dom.insert(ws, part("B", Vec3::new(2.0, 30.0, 0.0), false));
        let c = doc.dom.insert(ws, part("C", Vec3::new(4.0, 30.0, 0.0), false));
        let w1 = doc.dom.insert(a, InstanceBuilder::new("WeldConstraint")
            .with_property("Part0Internal", a).with_property("Part1Internal", b)
            .with_property("State", 1i32));
        let w0 = doc.dom.insert(a, InstanceBuilder::new("WeldConstraint")
            .with_property("Part0Internal", b).with_property("Part1Internal", c)
            .with_property("State", 0i32));
        assert!(joint_enabled(&doc, w1));
        assert!(!joint_enabled(&doc, w0));
        assert_eq!(joint_parts(&doc, w1), Some((a, b)));
        let p = Physics::new(&mut doc);
        assert_eq!(p.counts().0, 2, "A+B welded, C alone");
        // A RigidConstraint joins its attachments' parts.
        let mut doc = Document::new_place();
        let ws = doc.workspace().unwrap();
        let a = doc.dom.insert(ws, part("A", Vec3::new(0.0, 30.0, 0.0), false));
        let b = doc.dom.insert(ws, part("B", Vec3::new(2.0, 30.0, 0.0), false));
        let a0 = doc.dom.insert(a, InstanceBuilder::new("Attachment"));
        let a1 = doc.dom.insert(b, InstanceBuilder::new("Attachment"));
        doc.dom.insert(a, InstanceBuilder::new("RigidConstraint")
            .with_property("Attachment0", a0).with_property("Attachment1", a1));
        let p = Physics::new(&mut doc);
        assert_eq!(p.counts().0, 1, "one assembly");
    }
}
