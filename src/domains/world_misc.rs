//! The rest of the world domain (Studio 0.705, measured in an offline Studio):
//! - Tool GripPos / GripForward / GripRight / GripUp read Grip's position,
//!   −Z, X and Y; writing them rebuilds Grip the way Studio does (G3D
//!   lookAt): Forward = lookAt(v, old up); Right = the same turned so X is
//!   v; Up = lookAt(v, old right) turned so Y is v.
//! - Decal.ResampleMode is kept (Default / Pixelated).
//! - Trail:Clear drops the trail's laid points (effects.rs).
//! - Beam:SetTextureOffset(offset = 0) sets the texture cycle's phase
//!   (effects.rs): Studio's 0.25 moves the texture a quarter of its length
//!   toward Attachment0, 0.5 splits it in halves; TextureSpeed scrolls on.
//! - VehicleSeat.AreHingesDetected: the motorised HingeConstraints joined
//!   to the seat's assembly.
//! - StarterGear.Loaded: false (Studio loads no player gear).
//! - ProceduralModel: GenerationError is ""; ForceGeneration returns true;
//!   WaitForGenerationAsync errors outside the DataModel (Studio's message)
//!   and returns true inside (nothing left to generate).
//! - JointsService (deprecated): SetJoinAfterMoveInstance / Target name a
//!   moved instance and what to join it to; CreateJoinAfterMoveJoints welds
//!   their touching faces, ClearJoinAfterMoveJoints forgets them;
//!   ShowPermissibleJoints only highlights in Studio's editor.
//! - Dragger (legacy): MouseDown(part, point, parts) picks the parts up
//!   (all three arguments required); MouseMove(ray) casts the ray past them
//!   and moves them together so the grabbed part rests on the hit surface
//!   centred on the hit (Studio: P at the floor hit x/z, bottom on the
//!   floor); a miss leaves them. AxisRotate turns them 90° about the box
//!   around them: Y +90°, Z −90°, X does nothing (as Studio). MouseMove
//!   errors before MouseDown / after MouseUp with Studio's messages.
//!   (Studio's very first MouseMove after MouseDown jumps the parts near
//!   the origin — a Studio bug rustudio doesn't copy.)
//! - WireframeHandleAdornment: AddLine / AddLines (pairs; an odd last point
//!   is dropped) / AddPath (loop closes it) / AddText (size 12) / Clear keep
//!   the geometry the viewport draws (viewport.rs): only with an Adornee
//!   (none: Studio draws nothing), at Adornee.CFrame × CFrame × (Scale ⊙ p),
//!   1-pixel unlit lines in Color3 (Transparency), depth-tested unless
//!   AlwaysOnTop; text at CFrame × p (Studio ignores the Adornee for text),
//!   monospace, baseline-left on the point. Thickness has no visible effect
//!   in Studio's lines.
//! - DragDetector, outside a drag (rustudio's Play has no 3D drag input
//!   yet, so DragStart / DragContinue / DragEnd don't fire):
//!   GetReferenceFrame is ReferenceInstance's frame (an Attachment's world
//!   CFrame, a part's or model's pivot), else identity; SetDragStyleFunction
//!   / SetPermissionPolicyFunction keep the function (required);
//!   AddConstraintFunction(priority, f) keeps it and returns a
//!   connection; RestartDrag does nothing without a
//!   drag.

use std::cell::RefCell;
use std::collections::HashMap;

use glam::{Mat3, Vec3};
use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};

use super::util::*;
use crate::doc;
use crate::ltypes::*;
use crate::math::Xform;
use crate::runtime::*;

thread_local! {
    /// JointsService's join-after-move (instance, target) per document.
    static JOIN: RefCell<HashMap<u64, (Option<Ref>, Option<Ref>)>> = RefCell::new(HashMap::new());
    /// WireframeHandleAdornment geometry (per document and adornment).
    static WIREFRAMES: RefCell<HashMap<(u64, Ref), Wire>> = RefCell::new(HashMap::new());
    /// DragDetector → its style / permission functions and constraints.
    static DRAG_FNS: RefCell<HashMap<(u64, Ref), Vec<(String, mlua::Function)>>> = RefCell::new(HashMap::new());
    /// DragDetector → its constraint functions (priority, function, connection).
    static DRAG_CONSTRAINTS: RefCell<HashMap<(u64, Ref), Vec<(f64, mlua::Function, Value)>>> = RefCell::new(HashMap::new());
    /// Dragger → (grabbed part, parts, still down).
    static DRAG: RefCell<HashMap<(u64, Ref), (Ref, Vec<Ref>, bool)>> = RefCell::new(HashMap::new());
}

fn dragger(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let id = world(lua)?.d().id;
    match name {
        "MouseDown" => {
            for i in (1..=3).rev() {
                if arg(args, i).is_nil() {
                    return Err(missing(i));
                }
            }
            let part = opt_inst(&arg(args, 1)).ok_or_else(|| rt_err("Unable to cast value to Object"))?;
            let parts: Vec<Ref> = match arg(args, 3) {
                Value::Table(t) => t.sequence_values::<Value>().flatten().filter_map(|v| opt_inst(&v)).collect(),
                _ => return Err(rt_err("Unable to cast value to Objects")),
            };
            DRAG.with(|d| d.borrow_mut().insert((id, this), (part, parts, true)));
            none()
        }
        "MouseUp" => {
            DRAG.with(|d| {
                if let Some(e) = d.borrow_mut().get_mut(&(id, this)) {
                    e.2 = false;
                }
            });
            none()
        }
        "MouseMove" => {
            let ray = match arg(args, 1) {
                Value::Nil => return Err(missing(1)),
                v => ud::<LRay>(&v).ok_or_else(|| rt_err("Unable to cast value to Ray"))?,
            };
            let Some((part, parts, down)) = DRAG.with(|d| d.borrow().get(&(id, this)).cloned()) else {
                return Err(rt_err("Call to LuaDragger::mouseMove without mouseDown"));
            };
            if !down {
                return Err(rt_err("Call to LuaDragger::mouseMove after mouseUp"));
            }
            let moves: Vec<(Ref, Xform)> = {
                let w = world(lua)?;
                let d = w.d();
                let r = crate::math::Ray { origin: ray.origin, dir: ray.direction };
                let Some(ws) = d.workspace() else { return none() };
                let mut best: Option<(f32, Ref)> = None;
                for i in d.dom.descendants_of(ws) {
                    let q = i.referent();
                    if !doc::is_a(&i.class, "BasePart") || i.class == "Terrain" || parts.contains(&q) {
                        continue;
                    }
                    let (Some(x), Some(sz)) = (xf(d, q, "CFrame"), v3p(d, q, "Size")) else { continue };
                    if let Some(t) = r.hit_obb(&x, sz).filter(|t| *t <= 1.0) {
                        if best.is_none_or(|b| t < b.0) {
                            best = Some((t, q));
                        }
                    }
                }
                let Some((t, hit_part)) = best else { return none() };
                let hit = r.at(t);
                // The hit face's normal.
                let hx = xf(d, hit_part, "CFrame").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY });
                let hs = v3p(d, hit_part, "Size").unwrap_or(Vec3::ONE) * 0.5;
                let l = hx.rot.transpose() * (hit - hx.pos) / hs.max(Vec3::splat(1e-4));
                let a = l.abs();
                let n = if a.x >= a.y && a.x >= a.z {
                    hx.rot.x_axis * l.x.signum()
                } else if a.y >= a.z {
                    hx.rot.y_axis * l.y.signum()
                } else {
                    hx.rot.z_axis * l.z.signum()
                };
                let (Some(px), Some(ps)) = (xf(d, part, "CFrame"), v3p(d, part, "Size")) else { return none() };
                let h = ps * 0.5;
                let ext = px.rot.x_axis.dot(n).abs() * h.x + px.rot.y_axis.dot(n).abs() * h.y + px.rot.z_axis.dot(n).abs() * h.z;
                let delta = hit + n * ext - px.pos;
                parts
                    .iter()
                    .filter_map(|q| xf(d, *q, "CFrame").map(|x| (*q, Xform { pos: x.pos + delta, rot: x.rot })))
                    .collect()
            };
            for (q, x) in moves {
                store_prop(lua, q, "CFrame", Variant::CFrame(x.to_cframe()))?;
            }
            none()
        }
        _ => {
            // AxisRotate(axis = X)
            let axis = enum_of(&arg(args, 1), "Axis").unwrap_or(0);
            let Some((_, parts, _)) = DRAG.with(|d| d.borrow().get(&(id, this)).cloned()) else {
                return none();
            };
            let turn = match axis {
                1 => Mat3::from_rotation_y(std::f32::consts::FRAC_PI_2),
                2 => Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_2),
                _ => return none(),
            };
            let moves: Vec<(Ref, Xform)> = {
                let w = world(lua)?;
                let d = w.d();
                let mut lo = Vec3::splat(f32::INFINITY);
                let mut hi = Vec3::splat(f32::NEG_INFINITY);
                let boxes: Vec<(Ref, Xform, Vec3)> = parts
                    .iter()
                    .filter_map(|q| Some((*q, xf(d, *q, "CFrame")?, v3p(d, *q, "Size")?)))
                    .collect();
                for (_, x, s) in &boxes {
                    let h = *s * 0.5;
                    let e = x.rot.x_axis.abs() * h.x + x.rot.y_axis.abs() * h.y + x.rot.z_axis.abs() * h.z;
                    lo = lo.min(x.pos - e);
                    hi = hi.max(x.pos + e);
                }
                let c = (lo + hi) * 0.5;
                boxes
                    .into_iter()
                    .map(|(q, x, _)| (q, Xform { pos: c + turn * (x.pos - c), rot: turn * x.rot }))
                    .collect()
            };
            for (q, x) in moves {
                store_prop(lua, q, "CFrame", Variant::CFrame(x.to_cframe()))?;
            }
            none()
        }
    }
}

/// A WireframeHandleAdornment's lines (its own space, before Scale) and
/// texts (point, text, size in pixels).
#[derive(Clone, Default)]
pub struct Wire {
    pub lines: Vec<(Vec3, Vec3)>,
    pub texts: Vec<(Vec3, String, f32)>,
}

/// Every wireframe with geometry on a document (the renderer's view).
pub fn wireframes(doc: u64) -> Vec<(Ref, Wire)> {
    WIREFRAMES.with(|m| m.borrow().iter().filter(|((d, _), w)| *d == doc && (!w.lines.is_empty() || !w.texts.is_empty())).map(|((_, r), w)| (*r, w.clone())).collect())
}

/// What the viewport draws for the wireframes of a document: line
/// endpoints with their colour (alpha + 2 = no distance fade, + 4 = always
/// on top: the line shader reads those flags) and texts (world point,
/// text, pixel size, colour).
#[allow(clippy::type_complexity)]
pub fn wire_geometry(d: &crate::doc::Document) -> (Vec<([f32; 3], [f32; 4])>, Vec<(Vec3, String, f32, [f32; 4])>) {
    let mut lines = Vec::new();
    let mut texts = Vec::new();
    let root = d.dom.root_ref();
    for (r, w) in wireframes(d.id) {
        if !d.exists(r) || !d.is_ancestor(root, r) || !boolp(d, r, "Visible", true) {
            continue;
        }
        // No Adornee: Studio draws nothing.
        let Some(adornee) = refp(d, r, "Adornee") else { continue };
        let base = if is_part(d, adornee) {
            xf(d, adornee, "CFrame")
        } else {
            match d.dom.get_by_ref(adornee).and_then(|i| i.properties.get(&rbx_dom_weak::ustr("WorldPivotData")).cloned()) {
                Some(Variant::OptionalCFrame(Some(cf))) => Some(Xform::from_cframe(&cf)),
                _ => None,
            }
        };
        let Some(base) = base else { continue };
        let own = xf(d, r, "CFrame").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY });
        let frame = mul(&base, &own);
        let scale = v3p(d, r, "Scale").unwrap_or(Vec3::ONE);
        let rgb = match d.prop(r, "Color3") {
            Some(Variant::Color3(c)) => [c.r, c.g, c.b],
            Some(Variant::Color3uint8(c)) => [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0],
            _ => [0.05, 0.41, 0.67],
        };
        let alpha = (1.0 - f32p(d, r, "Transparency", 0.0)).clamp(0.0, 1.0);
        let top = boolp(d, r, "AlwaysOnTop", false);
        let flagged = [rgb[0], rgb[1], rgb[2], alpha + 2.0 + if top { 2.0 } else { 0.0 }];
        for (a, b) in &w.lines {
            lines.push((frame.apply(*a * scale).to_array(), flagged));
            lines.push((frame.apply(*b * scale).to_array(), flagged));
        }
        for (p, t, size) in &w.texts {
            texts.push((own.apply(*p), t.clone(), *size, [rgb[0], rgb[1], rgb[2], alpha]));
        }
    }
    (lines, texts)
}

fn points_arg(args: &MultiValue, i: usize) -> mlua::Result<Vec<Vec3>> {
    match arg(args, i) {
        Value::Table(t) => t
            .sequence_values::<Value>()
            .map(|v| v.and_then(|v| ud::<LVector3>(&v).map(|v| v.0).ok_or_else(|| rt_err("Unable to cast value to Vector3"))))
            .collect(),
        Value::Nil => Err(missing(i)),
        v => Err(rt_err(format!("Unable to cast {} to Array", v.type_name()))),
    }
}

fn wireframe(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let id = world(lua)?.d().id;
    let mut add_lines: Vec<(Vec3, Vec3)> = Vec::new();
    let mut text = None;
    match name {
        "AddLine" => add_lines.push((vec3_arg(args, 1)?, vec3_arg(args, 2)?)),
        // Pairs; an odd last point is dropped (Studio: no error).
        "AddLines" => {
            let p = points_arg(args, 1)?;
            add_lines.extend(p.chunks_exact(2).map(|c| (c[0], c[1])));
        }
        "AddPath" => {
            let p = points_arg(args, 1)?;
            let close = matches!(arg(args, 2), Value::Boolean(true));
            add_lines.extend(p.windows(2).map(|w| (w[0], w[1])));
            if close && p.len() > 2 {
                add_lines.push((p[p.len() - 1], p[0]));
            }
        }
        "AddText" => {
            let at = vec3_arg(args, 1)?;
            let s = match arg(args, 2) {
                Value::String(s) => s.to_str()?.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Integer(n) => n.to_string(),
                Value::Nil => return Err(missing(2)),
                v => return Err(rt_err(format!("Unable to cast {} to string", v.type_name()))),
            };
            text = Some((at, s, num_arg(args, 3).unwrap_or(12.0) as f32));
        }
        _ => {
            WIREFRAMES.with(|m| m.borrow_mut().remove(&(id, this)));
            return none();
        }
    }
    WIREFRAMES.with(|m| {
        let mut m = m.borrow_mut();
        let w = m.entry((id, this)).or_default();
        w.lines.extend(add_lines);
        if let Some(t) = text {
            w.texts.push(t);
        }
    });
    none()
}

/// A DragDetector's SetDragStyleFunction / SetPermissionPolicyFunction function.
pub fn drag_fn(doc: u64, det: Ref, which: &str) -> Option<mlua::Function> {
    DRAG_FNS.with(|m| m.borrow().get(&(doc, det)).and_then(|v| v.iter().find(|(k, _)| k == which).map(|(_, f)| f.clone())))
}

/// A DragDetector's constraint functions still connected, lowest priority
/// first (the highest has the last word).
pub fn drag_constraints(doc: u64, det: Ref) -> Vec<mlua::Function> {
    DRAG_CONSTRAINTS.with(|m| {
        let mut m = m.borrow_mut();
        let Some(v) = m.get_mut(&(doc, det)) else { return Vec::new() };
        v.retain(|(_, _, c)| match c {
            Value::UserData(u) => mlua::ObjectLike::get::<bool>(u, "Connected").unwrap_or(true),
            _ => true,
        });
        let mut v: Vec<(f64, mlua::Function)> = v.iter().map(|(p, f, _)| (*p, f.clone())).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, f)| f).collect()
    })
}

/// G3D CoordinateFrame::lookAt's rotation: −Z toward `look`, `up` kept as
/// far as it can be.
fn g3d_look_at(look: Vec3, up: Vec3) -> Option<Mat3> {
    let l = look.try_normalize()?;
    let mut up = up;
    if l.dot(up.normalize_or_zero()).abs() > 0.99 {
        up = Vec3::X;
        if l.dot(up).abs() > 0.99 {
            up = Vec3::Y;
        }
    }
    let z = -l;
    let x = (-z.cross(up)).try_normalize()?;
    let y = z.cross(x);
    Some(Mat3::from_cols(x, y, z))
}

fn grip(lua: &Lua, r: Ref) -> mlua::Result<Xform> {
    Ok(xf(world(lua)?.d(), r, "Grip").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }))
}

pub fn call(lua: &Lua, name: &str, this: Ref, class: &str, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    Some(match (class, name) {
        ("Beam", "SetTextureOffset") => {
            let id = world(lua).ok()?.d().id;
            crate::effects::set_texture_offset(id, this, num_arg(args, 1).unwrap_or(0.0) as f32);
            none()
        }
        ("Trail", "Clear") => {
            let id = world(lua).ok()?.d().id;
            crate::effects::clear(id, this);
            none()
        }
        ("ProceduralModel", "ForceGeneration") => one(Value::Boolean(true)),
        ("ProceduralModel", "WaitForGenerationAsync") => {
            let inside = world(lua).ok()?.d().is_ancestor(world(lua).ok()?.d().dom.root_ref(), this);
            if inside {
                one(Value::Boolean(true))
            } else {
                Err(rt_err("ProceduralModel outside the DataModel will not generate, can't wait for generation."))
            }
        }
        ("JointsService", "SetJoinAfterMoveInstance" | "SetJoinAfterMoveTarget") => {
            let r = opt_inst(&arg(args, 1));
            let id = world(lua).ok()?.d().id;
            JOIN.with(|j| {
                let mut j = j.borrow_mut();
                let e = j.entry(id).or_default();
                if name == "SetJoinAfterMoveInstance" {
                    e.0 = r;
                } else {
                    e.1 = r;
                }
            });
            none()
        }
        ("JointsService", "ClearJoinAfterMoveJoints") => {
            let id = world(lua).ok()?.d().id;
            JOIN.with(|j| j.borrow_mut().remove(&id));
            none()
        }
        ("JointsService", "CreateJoinAfterMoveJoints") => (|| {
            let id = world(lua)?.d().id;
            let Some((Some(inst), target)) = JOIN.with(|j| j.borrow().get(&id).copied()) else {
                return none();
            };
            super::part::join_between(lua, inst, target)?;
            none()
        })(),
        ("JointsService", "ShowPermissibleJoints") => none(),
        ("Dragger", "MouseDown" | "MouseMove" | "MouseUp" | "AxisRotate") => dragger(lua, name, this, args),
        ("WireframeHandleAdornment", "AddLine" | "AddLines" | "AddPath" | "AddText" | "Clear") => wireframe(lua, name, this, args),
        ("DragDetector", "GetReferenceFrame") => (|| {
            let x = {
                let w = world(lua)?;
                let d = w.d();
                let id = Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY };
                match refp(d, this, "ReferenceInstance") {
                    Some(r) if d.class_of(r).is_some_and(|c| c == "Attachment" || c == "Bone") => {
                        attachment_world(d, r).unwrap_or(id)
                    }
                    Some(r) if d.class_of(r).is_some_and(|c| doc::is_a(&c, "PVInstance")) => {
                        if is_part(d, r) { xf(d, r, "CFrame").unwrap_or(id) } else { model_pivot(&w, r) }
                    }
                    _ => id,
                }
            };
            one(cframe(lua, x)?)
        })(),
        ("DragDetector", "SetDragStyleFunction" | "SetPermissionPolicyFunction") => (|| {
            let f = match arg(args, 1) {
                Value::Function(f) => f,
                _ => return Err(missing(1)),
            };
            let id = world(lua)?.d().id;
            DRAG_FNS.with(|m| {
                let mut m = m.borrow_mut();
                let v = m.entry((id, this)).or_default();
                v.retain(|(k, _)| k != name);
                v.push((name.to_string(), f));
            });
            none()
        })(),
        ("DragDetector", "AddConstraintFunction") => (|| {
            let f = match arg(args, 2) {
                Value::Function(f) => f,
                _ => return Err(missing(2)),
            };
            let prio = num_arg(args, 1).ok_or_else(|| missing(1))?;
            // The connection is a signal connection the detector keeps.
            let sig: mlua::Function = lua.globals().get("__rs_new_signal")?;
            let s: mlua::AnyUserData = sig.call((inst_value(lua, this)?, format!("__DragConstraint{prio}")))?;
            let connect: mlua::Function = lua.load("return function(s, f) return s:Connect(f) end").eval()?;
            let conn: Value = connect.call((s, f.clone()))?;
            let id = world(lua)?.d().id;
            DRAG_CONSTRAINTS.with(|m| m.borrow_mut().entry((id, this)).or_default().push((prio, f, conn.clone())));
            one(conn)
        })(),
        ("DragDetector", "RestartDrag") => super::drag::restart(lua, this).and_then(|_| none()),
        _ => return None,
    })
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    Ok(Some(match (class, key) {
        ("Tool", "GripPos") => vec3(lua, grip(lua, r)?.pos)?,
        ("Tool", "GripForward") => vec3(lua, -grip(lua, r)?.rot.z_axis)?,
        ("Tool", "GripRight") => vec3(lua, grip(lua, r)?.rot.x_axis)?,
        ("Tool", "GripUp") => vec3(lua, grip(lua, r)?.rot.y_axis)?,
        ("Decal", "ResampleMode") => {
            match crate::scene::enum_u32(world(lua)?.d().prop(r, key)).and_then(|v| LEnumItem::from_value("ResamplerMode", v)) {
                Some(e) => enum_value(lua, e)?,
                None => enum_item(lua, "ResamplerMode", "Default")?,
            }
        }
        ("VehicleSeat", "AreHingesDetected") => {
            let n = {
                let w = world(lua)?;
                let d = w.d();
                super::part::with_graph(d, |g| {
                    let parts = g.assembly(r);
                    let mut seen = std::collections::HashSet::new();
                    for p in &parts {
                        for j in g.joints_of(*p) {
                            if d.class_of(j).is_some_and(|c| c == "HingeConstraint")
                                && crate::scene::enum_u32(d.prop(j, "ActuatorType")) == Some(1)
                            {
                                seen.insert(j);
                            }
                        }
                    }
                    seen.len()
                })
            };
            Value::Integer(n as i64)
        }
        ("StarterGear", "Loaded") => Value::Boolean(false),
        ("ProceduralModel", "GenerationError") => Value::String(lua.create_string("")?),
        _ => return Ok(None),
    }))
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    match (class, key) {
        ("Tool", "GripPos" | "GripForward" | "GripRight" | "GripUp") => {
            let v = ud::<LVector3>(value).map(|v| v.0).ok_or_else(|| {
                rt_err(format!("Unable to assign property {key}. Vector3 expected, got {}", value.type_name()))
            })?;
            let g = grip(lua, r)?;
            let new = match key {
                "GripPos" => Some(Xform { pos: v, rot: g.rot }),
                "GripForward" => g3d_look_at(v, g.rot.y_axis).map(|m| Xform { pos: g.pos, rot: m }),
                "GripRight" => g3d_look_at(v, g.rot.y_axis)
                    .map(|m| Xform { pos: g.pos, rot: Mat3::from_cols(-m.z_axis, m.y_axis, m.x_axis) }),
                _ => g3d_look_at(v, g.rot.x_axis)
                    .map(|m| Xform { pos: g.pos, rot: Mat3::from_cols(m.y_axis, -m.z_axis, -m.x_axis) }),
            };
            if let Some(x) = new {
                store_prop(lua, r, "Grip", Variant::CFrame(x.to_cframe()))?;
            }
            Ok(true)
        }
        ("Decal", "ResampleMode") => {
            let v = enum_of(value, "ResamplerMode").ok_or_else(|| {
                rt_err(format!("Unable to assign property {key}. EnumItem expected, got {}", value.type_name()))
            })?;
            store_prop(lua, r, key, Variant::Enum(rbx_dom_weak::types::Enum::from_u32(v)))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub fn write_acts(class: &str, key: &str) -> bool {
    class == "Tool" && matches!(key, "GripPos" | "GripForward" | "GripRight" | "GripUp")
}
