//! DragDetector drags (the server's half; the editor routes Play's mouse
//! here through play_drag.rs). Measured in Studio 0.705 (offline Studio, Play,
//! mouse drags on the 3D view; checks/parity/world.md "DragDetector"):
//!
//! - DragStart(player, cursorRay, viewFrame, hitFrame, clickedPart, nil,
//!   isModeSwitchKeyDown), DragContinue(player, cursorRay, viewFrame, nil,
//!   isModeSwitchKeyDown) once per cursor move, DragEnd(player). The client
//!   fires them first, the server with the same arguments. The cursor ray
//!   starts 0.1 stud in front of the camera and is 2048 long; hitFrame sits
//!   on the hit with its Y axis along the surface normal.
//! - On the server DragFrame is reset at DragStart and updated before each
//!   DragContinue fires (the client sees it a step later). Without a
//!   ReferenceInstance the reference frame is the world; Axis comes from
//!   Orientation (YXZ degrees, default (0, 180, 90) → +Y).
//! - TranslatePlane: the cursor ray against the plane through the hit,
//!   normal Axis; TranslateLine: the closest point on the Axis line through
//!   the hit; TranslateViewPlane: the plane through the hit facing the
//!   camera; Plane/Line OrLine/OrPlane switch while KeyboardModeSwitchKeyCode
//!   (LeftControl) is held; BestForDevice is TranslatePlaneOrLine for a
//!   mouse. RotateAxis: the signed angle about the Axis line through the
//!   pivot from the hit to the ray's point on the plane through the hit;
//!   RotateTrackball: the shortest arc on the sphere about the pivot
//!   through the hit (where the ray misses the sphere rustudio uses the
//!   hyperbolic sheet — Studio's own rule there isn't known; within ~15°).
//!   Scriptable: SetDragStyleFunction's function (cursor ray → world pivot
//!   or nil); without one Studio's server fires no DragContinue.
//! - Constraint functions (AddConstraintFunction) see the proposed
//!   DragFrame in priority order (higher last, so it wins).
//!   Min/MaxDragTranslation and Min/MaxDragAngle clamp when they differ.
//! - ResponseStyle Geometric moves the pivot (PivotTo); an unanchored part
//!   is anchored for the drag (before DragStart) and restored (before
//!   DragEnd). Physical pulls an unanchored part's grab point toward its
//!   goal (physics::live drag goal: Responsiveness, MaxForce, at the centre
//!   of mass with ApplyAtCenterOfMass; rotation styles also turn it,
//!   MaxTorque); anchored ones move geometrically. Custom moves nothing.
//! - PermissionPolicy Nobody refuses, Scriptable asks the function
//!   (player, part). MaxActivationDistance (32) from the character, as a
//!   ClickDetector.

use std::cell::RefCell;
use std::collections::HashMap;

use glam::{Mat3, Quat, Vec3};
use mlua::{Lua, Value};
use rbx_dom_weak::types::{Ref, Variant};

use super::util::*;
use crate::doc;
use crate::ltypes::*;
use crate::math::Xform;
use crate::runtime::*;

/// One drag in progress (per document: the server's).
#[derive(Clone)]
struct Drag {
    det: Ref,
    part: Ref,
    pivot_inst: Ref,
    player: Ref,
    start_pivot: Xform,
    /// The clicked part's CFrame at the start.
    part_start: Xform,
    hit: Vec3,
    reference: Xform,
    /// Anchored before a Geometric drag of an unanchored part.
    restore_anchor: Option<bool>,
    physical: bool,
    frame: Xform,
}

thread_local! {
    static DRAGS: RefCell<HashMap<u64, Drag>> = RefCell::new(HashMap::new());
}

const IDENT: Xform = Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY };

/// The DragDetector a press on `part` reaches: a child of the part or of a
/// Model / folder above it (below Workspace), enabled.
pub fn detector_for(d: &doc::Document, part: Ref) -> Option<Ref> {
    let ws = d.workspace()?;
    let mut cur = part;
    while cur.is_some() && cur != ws {
        let inst = d.dom.get_by_ref(cur)?;
        if let Some(c) = inst.children().iter().copied().find(|c| d.class_of(*c).is_some_and(|k| k == "DragDetector")) {
            return boolp(d, c, "Enabled", true).then_some(c);
        }
        cur = inst.parent();
    }
    None
}

/// The hit frame Studio passes: the hit point, Y along the surface normal.
pub fn hit_frame(hit: Vec3, normal: Vec3) -> Xform {
    let n = normal.normalize_or(Vec3::Y);
    let q = if n.dot(Vec3::Y) < -0.9999 { Quat::from_rotation_x(std::f32::consts::PI) } else { Quat::from_rotation_arc(Vec3::Y, n) };
    Xform { pos: hit, rot: Mat3::from_quat(q) }
}

/// Studio's cursor ray: from 0.1 stud in front of the eye, 2048 long.
pub fn cursor_ray(origin: Vec3, dir: Vec3) -> LRay {
    let d = dir.normalize_or(Vec3::NEG_Z);
    LRay { origin: origin + d * 0.1, direction: d * 2048.0 }
}

fn ray_value(lua: &Lua, r: &LRay) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(r.clone())?))
}

/// The arguments of one event, as Studio passes them.
pub fn event_args(
    lua: &Lua,
    ev: &str,
    player: Ref,
    ray: &LRay,
    view: Xform,
    hit: Option<(Xform, Ref)>,
    mode: bool,
) -> mlua::Result<Vec<Value>> {
    let p = inst_value(lua, player)?;
    Ok(match ev {
        "DragStart" => {
            let (hf, part) = hit.unwrap_or((IDENT, Ref::none()));
            vec![p, ray_value(lua, ray)?, cframe(lua, view)?, cframe(lua, hf)?, inst_value(lua, part)?, Value::Nil, Value::Boolean(mode)]
        }
        "DragContinue" => vec![p, ray_value(lua, ray)?, cframe(lua, view)?, Value::Nil, Value::Boolean(mode)],
        _ => vec![p],
    })
}

fn enum_or(d: &doc::Document, r: Ref, k: &str, def: u32) -> u32 {
    crate::scene::enum_u32(d.prop(r, k)).unwrap_or(def)
}

/// The reference frame now (GetReferenceFrame's rule).
pub fn reference_frame(w: &World, det: Ref) -> Xform {
    let d = w.d();
    match refp(d, det, "ReferenceInstance") {
        Some(r) if d.class_of(r).is_some_and(|c| c == "Attachment" || c == "Bone") => attachment_world(d, r).unwrap_or(IDENT),
        Some(r) if is_part(d, r) => xf(d, r, "CFrame").unwrap_or(IDENT),
        Some(r) if d.class_of(r).is_some_and(|c| doc::is_a(&c, "PVInstance")) => model_pivot(w, r),
        _ => IDENT,
    }
}

/// Axis and SecondaryAxis in the reference frame, from Orientation (YXZ).
fn axes(d: &doc::Document, det: Ref) -> (Vec3, Vec3) {
    let o = v3p(d, det, "Orientation").unwrap_or(Vec3::new(0.0, 180.0, 90.0)) * (std::f32::consts::PI / 180.0);
    let r = Mat3::from_rotation_y(o.y) * Mat3::from_rotation_x(o.x) * Mat3::from_rotation_z(o.z);
    (r * Vec3::X, r * Vec3::Y)
}

/// Server: a press on `part` (hit at `hit`, surface `normal`) starts a drag
/// if a DragDetector takes it. Returns the detector.
pub fn start(lua: &Lua, part: Ref, player: Ref, ray: &LRay, view: Xform, hit: Vec3, normal: Vec3, mode: bool) -> mlua::Result<Option<Ref>> {
    let (det, policy) = {
        let w = world(lua)?;
        let d = w.d();
        let Some(det) = detector_for(d, part) else { return Ok(None) };
        (det, enum_or(d, det, "PermissionPolicy", 1))
    };
    match policy {
        0 => return Ok(None),
        2 => {
            let id = world(lua)?.d().id;
            let f = super::misc::drag_fn(id, det, "SetPermissionPolicyFunction");
            let ok = match f {
                Some(f) => f.call::<Value>((inst_value(lua, player)?, inst_value(lua, part)?)).map(|v| v.as_boolean() == Some(true)).unwrap_or(false),
                None => false,
            };
            if !ok {
                return Ok(None);
            }
        }
        _ => {}
    }
    let (drag, unanchored_geo) = {
        let w = world(lua)?;
        let d = w.d();
        let parent = d.dom.get_by_ref(det).map(|i| i.parent()).unwrap_or_else(Ref::none);
        let pivot_inst = if d.class_of(parent).is_some_and(|c| doc::is_a(&c, "PVInstance")) { parent } else { part };
        let start_pivot = if is_part(d, pivot_inst) { xf(d, pivot_inst, "CFrame").unwrap_or(IDENT) } else { model_pivot(&w, pivot_inst) };
        let response = enum_or(d, det, "ResponseStyle", 1);
        let anchored = boolp(d, part, "Anchored", false);
        let physical = response == 1 && !anchored;
        let unanchored_geo = response == 0 && !anchored;
        (
            Drag {
                det,
                part,
                pivot_inst,
                player,
                start_pivot,
                part_start: xf(d, part, "CFrame").unwrap_or(IDENT),
                hit,
                reference: reference_frame(&w, det),
                restore_anchor: unanchored_geo.then_some(false),
                physical,
                frame: IDENT,
            },
            unanchored_geo,
        )
    };
    if unanchored_geo {
        store_prop(lua, part, "Anchored", Variant::Bool(true))?;
    }
    store_prop(lua, det, "DragFrame", Variant::CFrame(IDENT.to_cframe()))?;
    let id = world(lua)?.d().id;
    DRAGS.with(|m| m.borrow_mut().insert(id, drag));
    // Studio runs the motion (style function, constraint functions) for the
    // press itself: DragFrame is set when DragStart fires.
    apply_motion(lua, ray, view, mode)?;
    let args = event_args(lua, "DragStart", player, ray, view, Some((hit_frame(hit, normal), part)), mode)?;
    world(lua)?.fire(det, "DragStart", args);
    Ok(Some(det))
}

fn plane_hit(o: Vec3, d: Vec3, p: Vec3, n: Vec3) -> Option<Vec3> {
    let den = n.dot(d);
    if den.abs() < 1e-6 {
        return None;
    }
    let t = n.dot(p - o) / den;
    (t > 0.0).then(|| o + d * t)
}

/// The closest point on the line (p, a) to the ray.
fn line_point(o: Vec3, d: Vec3, p: Vec3, a: Vec3) -> f32 {
    let w0 = p - o;
    let b = a.dot(d);
    let (dd, e) = (a.dot(w0), d.dot(w0));
    let den = 1.0 - b * b;
    if den.abs() < 1e-6 {
        return 0.0;
    }
    (b * e - dd) / den
}

/// Trackball point for a ray: on the sphere about `c` of radius `r`, else
/// on the hyperbolic sheet in the plane facing the eye.
fn trackball(o: Vec3, d: Vec3, c: Vec3, r: f32) -> Vec3 {
    let oc = o - c;
    let b = oc.dot(d);
    let disc = b * b - (oc.dot(oc) - r * r);
    if disc >= 0.0 {
        return o + d * (-b - disc.sqrt()) - c;
    }
    let n = (c - o).normalize_or(Vec3::NEG_Z);
    let q = plane_hit(o, d, c, n).unwrap_or(c) - c;
    let q2 = q.length_squared().max(1e-8);
    q - n * (r * r / (2.0 * q2.sqrt()))
}

/// Server: the cursor moved. Computes the new DragFrame, moves, fires.
pub fn motion(lua: &Lua, ray: &LRay, view: Xform, mode: bool) -> mlua::Result<()> {
    let id = world(lua)?.d().id;
    let Some(dr) = DRAGS.with(|m| m.borrow().get(&id).cloned()) else { return Ok(()) };
    if !apply_motion(lua, ray, view, mode)? {
        return Ok(());
    }
    let args = event_args(lua, "DragContinue", dr.player, ray, view, None, mode)?;
    world(lua)?.fire(dr.det, "DragContinue", args);
    Ok(())
}

/// The new DragFrame for a cursor ray, applied (moves / goals). False when
/// a Scriptable detector has no style function (no motion, no event).
fn apply_motion(lua: &Lua, ray: &LRay, view: Xform, mode: bool) -> mlua::Result<bool> {
    let id = world(lua)?.d().id;
    let Some(dr) = DRAGS.with(|m| m.borrow().get(&id).cloned()) else { return Ok(false) };
    let o = ray.origin;
    let dir = ray.direction.normalize_or(Vec3::NEG_Z);
    let (style, s) = {
        let w = world(lua)?;
        let d = w.d();
        if !d.exists(dr.det) {
            drop(w);
            end(lua)?;
            return Ok(false);
        }
        let (a, _) = axes(d, dr.det);
        let axis = (dr.reference.rot * a).normalize_or(Vec3::Y);
        let mut style = enum_or(d, dr.det, "DragStyle", 1);
        if style == 8 {
            style = 2;
        }
        let style = match (style, mode) {
            (2, false) | (3, true) => 1,
            (2, true) | (3, false) => 0,
            (s, _) => s,
        };
        let s = (axis, enum_or(d, dr.det, "ResponseStyle", 1), xf(d, dr.det, "DragFrame").unwrap_or(IDENT));
        (style, s)
    };
    let (axis, response, _) = s;
    // Proposed DragFrame (in world terms: translation, or rotation about the pivot).
    let proposed: Option<Xform> = match style {
        0 => Some(Xform { pos: axis * line_point(o, dir, dr.hit, axis), rot: Mat3::IDENTITY }),
        1 => plane_hit(o, dir, dr.hit, axis).map(|p| Xform { pos: p - dr.hit, rot: Mat3::IDENTITY }),
        4 => {
            let n = -view.rot.z_axis;
            plane_hit(o, dir, dr.hit, n).map(|p| Xform { pos: p - dr.hit, rot: Mat3::IDENTITY })
        }
        5 => plane_hit(o, dir, dr.hit, axis).map(|p| {
            let c = dr.start_pivot.pos;
            let flat = |v: Vec3| v - axis * axis.dot(v);
            let (r0, r1) = (flat(dr.hit - c), flat(p - c));
            let ang = r0.cross(r1).dot(axis).atan2(r0.dot(r1));
            Xform { pos: Vec3::ZERO, rot: Mat3::from_axis_angle(axis, ang) }
        }),
        6 => {
            let c = dr.start_pivot.pos;
            let r = (dr.hit - c).length().max(1e-3);
            let a = trackball(o, dir, c, r);
            let q = Quat::from_rotation_arc((dr.hit - c).normalize_or(Vec3::Y), a.normalize_or(Vec3::Y));
            Some(Xform { pos: Vec3::ZERO, rot: Mat3::from_quat(q) })
        }
        7 => {
            let f = super::misc::drag_fn(id, dr.det, "SetDragStyleFunction");
            let Some(f) = f else {
                // Studio's server sends no DragContinue without a function.
                return Ok(false);
            };
            let v: Value = f.call(ray_value(lua, ray)?)?;
            ud::<LCFrame>(&v).map(|c| {
                let goal = c.0;
                Xform { pos: goal.pos - dr.start_pivot.pos, rot: goal.rot * dr.start_pivot.rot.transpose() }
            })
        }
        _ => None,
    };
    let mut frame = proposed.unwrap_or(dr.frame);
    // Limits (only when Min and Max differ), then the constraint functions.
    {
        let w = world(lua)?;
        let d = w.d();
        let lo = v3p(d, dr.det, "MinDragTranslation").unwrap_or(Vec3::ZERO);
        let hi = v3p(d, dr.det, "MaxDragTranslation").unwrap_or(Vec3::ZERO);
        let local = dr.reference.rot.transpose() * frame.pos;
        let mut c = local;
        for i in 0..3 {
            if lo[i] != hi[i] {
                c[i] = c[i].clamp(lo[i].min(hi[i]), lo[i].max(hi[i]));
            }
        }
        frame.pos = dr.reference.rot * c;
        let (amin, amax) = (f32p(d, dr.det, "MinDragAngle", 0.0), f32p(d, dr.det, "MaxDragAngle", 0.0));
        if amin != amax {
            let (ax, ang) = Quat::from_mat3(&frame.rot).to_axis_angle();
            let sign = if ax.dot(axis) < 0.0 { -1.0 } else { 1.0 };
            let deg = (ang.to_degrees() * sign).clamp(amin.min(amax), amin.max(amax));
            frame.rot = Mat3::from_axis_angle(axis, deg.to_radians());
        }
    }
    for f in super::misc::drag_constraints(id, dr.det) {
        if let Ok(v) = f.call::<Value>(cframe(lua, frame)?) {
            if let Some(c) = ud::<LCFrame>(&v) {
                frame = c.0;
            }
        }
    }
    DRAGS.with(|m| {
        if let Some(e) = m.borrow_mut().get_mut(&id) {
            e.frame = frame;
        }
    });
    store_prop(lua, dr.det, "DragFrame", Variant::CFrame(frame.to_cframe()))?;
    let goal = Xform { pos: dr.start_pivot.pos + frame.pos, rot: frame.rot * dr.start_pivot.rot };
    match response {
        2 => {}
        1 if dr.physical => {
            let (at_com, mf, mt, resp) = {
                let w = world(lua)?;
                let d = w.d();
                (
                    boolp(d, dr.det, "ApplyAtCenterOfMass", false),
                    f32p(d, dr.det, "MaxForce", 1e7),
                    f32p(d, dr.det, "MaxTorque", 1e4),
                    f32p(d, dr.det, "Responsiveness", 10.0),
                )
            };
            // Where the grab point (fixed in the part) should be, and the
            // part's goal orientation for rotation styles.
            let delta = mul(&goal, &dr.start_pivot.inverse());
            let rotate = matches!(style, 5 | 6 | 7);
            crate::physics::live::set_drag(
                id,
                dr.part,
                Some(crate::physics::live::DragGoal {
                    grab_local: dr.part_start.inverse().apply(dr.hit),
                    goal: delta.apply(dr.hit),
                    goal_rot: rotate.then(|| Quat::from_mat3(&(delta.rot * dr.part_start.rot)).normalize()),
                    max_force: mf,
                    max_torque: mt,
                    responsiveness: resp,
                    at_com,
                }),
            );
        }
        _ => pivot_to(lua, dr.pivot_inst, goal)?,
    }
    Ok(true)
}

/// Server: the button was released (or the detector went away).
pub fn end(lua: &Lua) -> mlua::Result<()> {
    let id = world(lua)?.d().id;
    let Some(dr) = DRAGS.with(|m| m.borrow_mut().remove(&id)) else { return Ok(()) };
    crate::physics::live::set_drag(id, dr.part, None);
    let exists = world(lua)?.d().exists(dr.part);
    if let (Some(was), true) = (dr.restore_anchor, exists) {
        store_prop(lua, dr.part, "Anchored", Variant::Bool(was))?;
    }
    if world(lua)?.d().exists(dr.det) {
        let args = vec![inst_value(lua, dr.player)?];
        world(lua)?.fire(dr.det, "DragEnd", args);
    }
    Ok(())
}

/// The detector being dragged on this document, if any.
pub fn active(lua: &Lua) -> Option<Ref> {
    let id = world(lua).ok()?.d().id;
    DRAGS.with(|m| m.borrow().get(&id).map(|d| d.det))
}

/// Script: DragDetector:RestartDrag() — the drag starts again from where
/// the pivot is now (new start pivot and reference, DragFrame identity).
pub fn restart(lua: &Lua, det: Ref) -> mlua::Result<()> {
    let id = world(lua)?.d().id;
    let Some(mut dr) = DRAGS.with(|m| m.borrow().get(&id).cloned()) else { return Ok(()) };
    if dr.det != det {
        return Ok(());
    }
    {
        let w = world(lua)?;
        let d = w.d();
        let now = if is_part(d, dr.pivot_inst) { xf(d, dr.pivot_inst, "CFrame").unwrap_or(IDENT) } else { model_pivot(&w, dr.pivot_inst) };
        dr.hit = now.apply(dr.start_pivot.inverse().apply(dr.hit));
        dr.start_pivot = now;
        dr.reference = reference_frame(&w, det);
        dr.frame = IDENT;
    }
    store_prop(lua, det, "DragFrame", Variant::CFrame(IDENT.to_cframe()))?;
    DRAGS.with(|m| m.borrow_mut().insert(id, dr));
    Ok(())
}
