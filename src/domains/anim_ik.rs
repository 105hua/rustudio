//! IKControl: inverse kinematics on Motor6D chains, run every frame after
//! the Animator's tracks have written their Transforms (the Animator frame
//! hook runs them after the tracks, anim.rs `animate`). Measured in Studio 0.746 through
//! an offline Studio (R15 right arm, ChainRoot RightUpperArm, EndEffector RightHand):
//!
//! - The chain is the joints from the ChainRoot's joint down to the
//!   EndEffector's; its nodes are those joints' frames (Part0 · C0 ·
//!   Transform). GetChainCount = joints − 1 (2 for the arm), GetChainLength
//!   = the summed distances between the nodes (1.643), GetNodeWorldCFrame /
//!   GetNodeLocalCFrame take node indices 1 … count + 1 (anything else gives
//!   identity), the local frame of node 1 being in the chain's parent part.
//! - GetRawFinalTarget is where the last node has to go: the target
//!   position (plus Offset) with the effector's current rotation (Position)
//!   or the target's rotation (Transform / Rotation), carried back to the
//!   last joint. Identity until the first solve.
//! - GetSmoothedFinalTarget follows it with a critically damped spring of
//!   ω = 2 / SmoothTime (Unity-style SmoothDamp: 0.428 of the way after one
//!   0.0375 s frame at the default 0.05 s, as measured).
//! - An unreachable target leaves the arm 0.9913 of fully stretched (wrist
//!   1.6288 of 1.6431 from the shoulder); a reachable one puts the effector
//!   exactly on it. Position keeps the end joint's Transform; Transform and
//!   Rotation also turn it to the target's rotation; LookAt points the
//!   effector's LookVector at the target.
//! - The solve starts from the joints' current Transforms: with no track
//!   playing nothing resets them, so a disabled or zero-weight control
//!   leaves the last solved pose, as Studio does.
//!
//! The solver itself is FABRIK on the node positions (with the Pole, when
//! set, choosing the bend plane), turned back into joint rotations; Studio's
//! own iteration details are not reproduced bit for bit.

use std::collections::HashMap;

use glam::{Mat3, Quat, Vec3};
use mlua::{Lua, Value};
use rbx_dom_weak::types::{CFrame, Ref, Variant};

use crate::doc::{self, Document};
use crate::math::Xform;

pub const REACH: f32 = 0.9913;

pub fn ident() -> Xform {
    Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }
}
pub fn mul(a: &Xform, b: &Xform) -> Xform {
    Xform { pos: a.pos + a.rot * b.pos, rot: a.rot * b.rot }
}
fn cf(d: &Document, r: Ref, k: &str) -> Option<Xform> {
    match d.prop(r, k) {
        Some(Variant::CFrame(c)) => Some(Xform::from_cframe(&c)),
        _ => None,
    }
}
fn cf_or(d: &Document, r: Ref, k: &str) -> Xform {
    cf(d, r, k).unwrap_or_else(ident)
}
fn refp(d: &Document, r: Ref, k: &str) -> Option<Ref> {
    match d.prop(r, k) {
        Some(Variant::Ref(x)) if x.is_some() && d.exists(x) => Some(x),
        _ => None,
    }
}
fn num(d: &Document, r: Ref, k: &str, def: f32) -> f32 {
    crate::scene::f32_of(d.prop(r, k)).unwrap_or(def)
}
fn is_a(d: &Document, r: Ref, base: &str) -> bool {
    d.class_of(r).is_some_and(|c| doc::is_a(&c, base))
}
fn rot_lerp(a: Mat3, b: Mat3, t: f32) -> Mat3 {
    Mat3::from_quat(Quat::from_mat3(&a).normalize().slerp(Quat::from_mat3(&b).normalize(), t))
}
fn xf_lerp(a: &Xform, b: &Xform, t: f32) -> Xform {
    Xform { pos: a.pos.lerp(b.pos, t), rot: rot_lerp(a.rot, b.rot, t) }
}
fn ortho(m: Mat3) -> Mat3 {
    Mat3::from_quat(Quat::from_mat3(&m).normalize())
}

/// One joint of a chain.
#[derive(Clone, Copy)]
pub struct Link {
    pub motor: Ref,
    pub part0: Ref,
    #[allow(dead_code)]
    pub part1: Ref,
    pub c0: Xform,
    pub c1: Xform,
    pub t: Xform,
}

/// The Motor6D whose Part1 is `part`, within `scope`'s descendants.
fn joint_above(d: &Document, scope: Ref, part: Ref) -> Option<Link> {
    d.dom.descendants_of(scope).find_map(|i| {
        if !crate::rig::is_animated_joint(&i.class) {
            return None;
        }
        let r = i.referent();
        if matches!(d.prop(r, "Enabled"), Some(Variant::Bool(false))) {
            return None;
        }
        let (p0, p1, c0, c1) = crate::rig::joint_frames(d, r)?;
        if p1 != part {
            return None;
        }
        Some(Link { motor: r, part0: p0, part1: p1, c0, c1, t: cf_or(d, r, "Transform") })
    })
}

/// The model a rig part belongs to (its topmost Model ancestor below
/// Workspace / the root), for finding joints.
fn rig_scope(d: &Document, part: Ref) -> Ref {
    let mut best = d.dom.get_by_ref(part).map(|i| i.parent()).unwrap_or(part);
    let mut cur = best;
    while let Some(i) = d.dom.get_by_ref(cur) {
        if i.class == "Model" {
            best = cur;
        }
        let p = i.parent();
        if p.is_none() || d.class_of(p).is_some_and(|c| c == "Workspace" || c == "DataModel") {
            break;
        }
        cur = p;
    }
    best
}

/// World frame of a part from its joints' Transforms (forward kinematics
/// up to a part no Motor6D holds), so a server-side rig whose parts sit in
/// the rest pose still solves from the animated one.
fn world_of(d: &Document, scope: Ref, part: Ref, depth: usize) -> Xform {
    if depth < 32 {
        if let Some(j) = joint_above(d, scope, part) {
            let p0 = world_of(d, scope, j.part0, depth + 1);
            return mul(&mul(&mul(&p0, &j.c0), &j.t), &j.c1.inverse());
        }
    }
    cf_or(d, part, "CFrame")
}

/// The effector: its part and its frame in that part.
fn effector(d: &Document, ik: Ref) -> Option<(Ref, Xform)> {
    let e = refp(d, ik, "EndEffector")?;
    let extra = cf_or(d, ik, "EndEffectorOffset");
    if is_a(d, e, "BasePart") {
        return Some((e, extra));
    }
    if is_a(d, e, "Attachment") {
        let p = d.dom.get_by_ref(e)?.parent();
        if is_a(d, p, "BasePart") {
            return Some((p, mul(&cf_or(d, e, "CFrame"), &extra)));
        }
    }
    None
}

/// The chain from the ChainRoot's joint down to the effector part's, root first.
pub fn chain(d: &Document, ik: Ref) -> Option<(Ref, Vec<Link>, Ref, Xform)> {
    let root = refp(d, ik, "ChainRoot")?;
    let (eff, eoff) = effector(d, ik)?;
    let scope = rig_scope(d, eff);
    let mut links = Vec::new();
    let mut part = eff;
    // A ChainRoot that is the effector itself (or a Bone / Attachment
    // root) still needs one joint: the effector's own.
    loop {
        let j = joint_above(d, scope, part)?;
        links.push(j);
        if part == root || links.len() > 64 {
            break;
        }
        part = j.part0;
        if part == root {
            if let Some(j) = joint_above(d, scope, part) {
                links.push(j);
            }
            break;
        }
    }
    links.reverse();
    Some((scope, links, eff, eoff))
}

/// Node world frames (joint frames Part0 · C0 · Transform) for a chain.
pub fn nodes(d: &Document, scope: Ref, links: &[Link]) -> (Xform, Vec<Xform>) {
    let base = links.first().map(|l| world_of(d, scope, l.part0, 0)).unwrap_or_else(ident);
    let mut out = Vec::with_capacity(links.len());
    let mut p0 = base;
    for l in links {
        let n = mul(&mul(&p0, &l.c0), &l.t);
        out.push(n);
        p0 = mul(&n, &l.c1.inverse());
    }
    (base, out)
}

fn target_frame(d: &Document, ik: Ref) -> Option<Xform> {
    let t = refp(d, ik, "Target")?;
    let base = if is_a(d, t, "BasePart") {
        cf_or(d, t, "CFrame")
    } else if is_a(d, t, "Attachment") {
        crate::attachment::world(d, t, true)?
    } else if is_a(d, t, "Model") {
        let pp = refp(d, t, "PrimaryPart")?;
        cf_or(d, pp, "CFrame")
    } else {
        return None;
    };
    Some(mul(&base, &cf_or(d, ik, "Offset")))
}

fn pole_point(d: &Document, ik: Ref) -> Option<Vec3> {
    let p = refp(d, ik, "Pole")?;
    if is_a(d, p, "BasePart") {
        Some(cf_or(d, p, "CFrame").pos)
    } else if is_a(d, p, "Attachment") {
        crate::attachment::world(d, p, true).map(|x| x.pos)
    } else {
        None
    }
}

/// FABRIK on node positions: `pts[0]` stays, the last goes to `goal`.
fn fabrik(pts: &mut [Vec3], goal: Vec3, pole: Option<Vec3>) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    let lens: Vec<f32> = (0..n - 1).map(|i| (pts[i + 1] - pts[i]).length()).collect();
    let total: f32 = lens.iter().sum();
    let base = pts[0];
    let mut goal = goal;
    let to = goal - base;
    if to.length() > total * REACH {
        goal = base + to.normalize_or_zero() * total * REACH;
    }
    // Bend plane from the pole: each middle node turned about the
    // root-goal line to face it.
    if let Some(pole) = pole {
        let axis = (goal - base).normalize_or_zero();
        if axis != Vec3::ZERO {
            for i in 1..n - 1 {
                let proj = |p: Vec3| {
                    let v = p - base;
                    v - axis * v.dot(axis)
                };
                let a = proj(pts[i]);
                let b = proj(pole);
                if a.length() > 1e-5 && b.length() > 1e-5 {
                    let q = Quat::from_rotation_arc(a.normalize(), b.normalize());
                    pts[i] = base + q * (pts[i] - base);
                }
            }
        }
    }
    for _ in 0..24 {
        if (pts[n - 1] - goal).length() < 1e-4 {
            break;
        }
        pts[n - 1] = goal;
        for i in (0..n - 1).rev() {
            let dir = (pts[i] - pts[i + 1]).normalize_or_zero();
            pts[i] = pts[i + 1] + dir * lens[i];
        }
        pts[0] = base;
        for i in 0..n - 1 {
            let dir = (pts[i + 1] - pts[i]).normalize_or_zero();
            pts[i + 1] = pts[i] + dir * lens[i];
        }
    }
}

/// Per-control state kept between frames.
#[derive(Default, Clone)]
pub struct IkState {
    pub raw: Option<Xform>,
    pub smoothed: Option<Xform>,
    pub vel: Vec3,
}

/// Turns joint `i` so the world rotation `q` (about its node) applies to
/// everything below it.
fn turn(links: &mut [Link], i: usize, q: Quat, p0w: &Xform) {
    let l = &mut links[i];
    let n = mul(&mul(p0w, &l.c0), &l.t);
    let rot = ortho(Mat3::from_quat(q) * n.rot);
    let new_n = Xform { pos: n.pos, rot };
    l.t = mul(&mul(p0w, &l.c0).inverse(), &new_n);
    l.t.rot = ortho(l.t.rot);
}

fn p0_world(base: &Xform, links: &[Link], i: usize) -> Xform {
    let mut p0 = *base;
    for l in &links[..i] {
        p0 = mul(&mul(&mul(&p0, &l.c0), &l.t), &l.c1.inverse());
    }
    p0
}

/// Solves one control; returns the joints' new Transforms.
pub fn solve(d: &Document, ik: Ref, st: &mut IkState, dt: f32) -> Option<Vec<(Ref, Xform)>> {
    if matches!(d.prop(ik, "Enabled"), Some(Variant::Bool(false))) {
        return None;
    }
    let target = target_frame(d, ik)?;
    let (scope, links0, _eff, eoff) = chain(d, ik)?;
    if links0.is_empty() {
        return None;
    }
    let weight = num(d, ik, "Weight", 1.0).clamp(0.0, 1.0);
    let smooth_time = num(d, ik, "SmoothTime", 0.05).max(0.0);
    let kind = match d.prop(ik, "Type") {
        Some(Variant::Enum(e)) => e.to_u32(),
        _ => 0,
    };
    // Enum.IKControlType: Transform 0, Position 1, Rotation 2, LookAt 3.
    let mut links = links0.clone();
    let last = links.len() - 1;
    let (base, ns) = nodes(d, scope, &links);
    // Effector frame relative to the last node (rigid: below it).
    let eff_rel = mul(&links[last].c1.inverse(), &eoff);
    let eff_world = mul(&ns[last], &eff_rel);
    // Raw final target: where the last node goes.
    let want_rot = match kind {
        1 | 3 => eff_world.rot,
        _ => target.rot,
    };
    // node = effector placed at the target · eff_rel⁻¹
    let raw = mul(&Xform { pos: target.pos, rot: want_rot }, &eff_rel.inverse());
    st.raw = Some(raw);
    // Smoothing (SmoothDamp, ω = 2 / SmoothTime).
    let goal = match (st.smoothed, smooth_time > 1e-6 && dt > 0.0) {
        (Some(prev), true) => {
            let omega = 2.0 / smooth_time;
            let x = omega * dt;
            let e = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
            let change = prev.pos - raw.pos;
            let temp = (st.vel + change * omega) * dt;
            st.vel = (st.vel - temp * omega) * e;
            let pos = raw.pos + (change + temp) * e;
            let rot = rot_lerp(prev.rot, raw.rot, 1.0 - e * (1.0 + x));
            Xform { pos, rot }
        }
        _ => {
            st.vel = Vec3::ZERO;
            raw
        }
    };
    st.smoothed = Some(goal);
    let pole = pole_point(d, ik);
    match kind {
        // LookAt: the effector's LookVector at the target, the turn shared
        // out along the chain.
        3 => {
            let n = links.len();
            for i in 0..n {
                let p0w = p0_world(&base, &links, i);
                let mut node_l = links.clone();
                let (_, ns) = nodes_from(&p0w, &mut node_l[i..]);
                let ew = mul(&ns[ns.len() - 1], &eff_rel);
                let look = -(ew.rot * Vec3::Z);
                let want = (target.pos - ew.pos).normalize_or_zero();
                if want == Vec3::ZERO {
                    break;
                }
                let full = Quat::from_rotation_arc(look.normalize_or_zero(), want);
                let share = 1.0 / (n - i) as f32;
                turn(&mut links, i, Quat::IDENTITY.slerp(full, share), &p0w);
            }
        }
        _ => {
            if kind != 2 && links.len() >= 2 {
                // Bones between the nodes, then the outer loop again so a
                // Position solve's goal follows the effector's new rotation.
                for _ in 0..4 {
                    let (_, cur) = nodes_from(&base, &mut links.clone());
                    let goal_pos = if kind == 1 {
                        let ew = mul(&cur[last], &eff_rel);
                        let at = Xform { pos: goal.pos, rot: ew.rot };
                        mul(&at, &eff_rel.inverse()).pos
                    } else {
                        goal.pos
                    };
                    let mut pts: Vec<Vec3> = cur.iter().map(|n| n.pos).collect();
                    fabrik(&mut pts, goal_pos, pole);
                    for i in 0..last {
                        let (_, cur) = nodes_from(&base, &mut links.clone());
                        let a = (cur[i + 1].pos - cur[i].pos).normalize_or_zero();
                        let b = (pts[i + 1] - cur[i].pos).normalize_or_zero();
                        if a == Vec3::ZERO || b == Vec3::ZERO {
                            continue;
                        }
                        let p0w = p0_world(&base, &links, i);
                        turn(&mut links, i, Quat::from_rotation_arc(a, b), &p0w);
                    }
                }
            }
            if kind == 0 || kind == 2 {
                // The end joint takes the target's rotation.
                let p0w = p0_world(&base, &links, last);
                let l = &mut links[last];
                let node = mul(&mul(&p0w, &l.c0), &l.t);
                let want = Xform { pos: node.pos, rot: goal.rot };
                l.t = mul(&mul(&p0w, &l.c0).inverse(), &want);
                l.t.rot = ortho(l.t.rot);
            }
        }
    }
    Some(
        links
            .iter()
            .zip(&links0)
            .map(|(l, o)| (l.motor, xf_lerp(&o.t, &l.t, weight)))
            .collect(),
    )
}

/// Nodes of a chain from its base part's world frame.
fn nodes_from(base: &Xform, links: &mut [Link]) -> (Xform, Vec<Xform>) {
    let mut out = Vec::with_capacity(links.len());
    let mut p0 = *base;
    for l in links.iter() {
        let n = mul(&mul(&p0, &l.c0), &l.t);
        out.push(n);
        p0 = mul(&n, &l.c1.inverse());
    }
    (*base, out)
}

/// IKControl getters (Studio's numbers in edit mode too).
pub fn getter(lua: &Lua, d: &Document, ik: Ref, st: Option<&IkState>, name: &str, index: i64) -> mlua::Result<Value> {
    let cfv = |x: Xform| -> mlua::Result<Value> {
        mlua::IntoLua::into_lua(crate::ltypes::LCFrame(x), lua)
    };
    match name {
        "GetRawFinalTarget" => return cfv(st.and_then(|s| s.raw).unwrap_or_else(ident)),
        "GetSmoothedFinalTarget" => return cfv(st.and_then(|s| s.smoothed).unwrap_or_else(ident)),
        _ => {}
    }
    let Some((scope, links, _, _)) = chain(d, ik) else {
        return match name {
            "GetChainCount" => Ok(Value::Integer(0)),
            "GetChainLength" => Ok(Value::Number(0.0)),
            _ => cfv(ident()),
        };
    };
    let (base, ns) = nodes(d, scope, &links);
    let count = ns.len().saturating_sub(1);
    match name {
        "GetChainCount" => Ok(Value::Integer(count as i64)),
        "GetChainLength" => Ok(Value::Number(
            ns.windows(2).map(|w| (w[1].pos - w[0].pos).length() as f64).sum(),
        )),
        "GetNodeWorldCFrame" | "GetNodeLocalCFrame" => {
            if index < 1 || index as usize > ns.len() {
                return cfv(ident());
            }
            let i = index as usize - 1;
            if name == "GetNodeWorldCFrame" {
                cfv(ns[i])
            } else {
                let parent = if i == 0 { base } else { ns[i - 1] };
                cfv(mul(&parent.inverse(), &ns[i]))
            }
        }
        _ => Ok(Value::Nil),
    }
}

/// Every IKControl in a document, by Priority (lowest first).
pub fn controls(d: &Document) -> Vec<Ref> {
    let all: Vec<Ref> = d
        .dom
        .descendants_of(d.dom.root_ref())
        .filter(|i| i.class == "IKControl")
        .map(|i| i.referent())
        .collect();
    by_priority(d, &all)
}

/// IKControls in solving order: by Priority, ties in the given order.
pub fn by_priority(d: &Document, iks: &[Ref]) -> Vec<Ref> {
    let mut v: Vec<(i32, Ref)> = iks
        .iter()
        .map(|r| (num(d, *r, "Priority", 0.0) as i32, *r))
        .collect();
    v.sort_by_key(|(p, _)| *p);
    v.into_iter().map(|(_, r)| r).collect()
}

pub fn to_variant(x: &Xform) -> Variant {
    Variant::CFrame(CFrame::from(x.to_cframe()))
}

#[allow(dead_code)]
pub fn hashmap() -> HashMap<Ref, IkState> {
    HashMap::new()
}
