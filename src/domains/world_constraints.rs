//! Constraints, attachments, legacy movers and sensors (Studio 0.705,
//! measured in an offline Studio; frames and numbers in checks/parity/world.md).
//!
//! - Constraint.Active: enabled, its attachments (or parts) set and under
//!   Workspace, not both ends anchored, and simulated at least one step
//!   (Studio reads false right after parenting, true after a frame).
//! - Current* readouts come from the attachments' world frames, as Studio
//!   computes them: Hinge / Cylindrical CurrentAngle = signed angle about
//!   the rotation axis from Attachment0's secondary axis to Attachment1's
//!   (Cylindrical: Attachment0's frame turned by InclinationAngle about its
//!   Z, Attachment1's not); TorsionSpring = the angle between the two
//!   secondary axes; Prismatic / Cylindrical CurrentPosition = the offset
//!   along Attachment0's axis; Rod / Rope CurrentDistance and Spring
//!   CurrentLength = the distance between the attachments.
//! - GetDebugAppliedForce / Torque: Studio has no applied-force queries
//!   ("Querying applied forces is not currently supported.").
//! - BodyPosition / BodyVelocity GetLastForce: Studio always answers zero.
//! - RocketPropulsion Fire / Abort drive physics.rs's Rocket law; it and
//!   BodyPosition fire ReachedTarget from the physics (physics::live).
//! - AtmosphereSensor: AirDensity = Workspace.AirDensity·e^(−y/33381)
//!   (y ≥ 0), RelativeWindVelocity = GlobalWind·min((y/5)^0.20735, 3) − the
//!   part's velocity (world space; zero below y = 0), fitted to Studio's
//!   readings at 0…2000 studs.
//! - FluidForceSensor: Force / Torque / CenterOfPressure are zero while
//!   Workspace.FluidForces isn't Experimental (Studio's default);
//!   EvaluateAsync applies the pressure model fitted to Studio (each face
//!   moving into the air pushed back by 1.2·ρ·(v·n)²·area).
//!   BuoyancyForce / BuoyancyTorque aren't members in this Studio build.
//! - PhysicsActivitySensor can't be made by scripts (InternalTest); its
//!   Active is whether an awake assembly's part is inside Position / Size.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use glam::{Mat3, Vec3};
use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};

use super::util::*;
use crate::doc::{self, Document};
use crate::ltypes::*;
use crate::math::Xform;
use crate::runtime::*;

thread_local! {
    /// (document id) → (tree revision seen, constraints simulated last step).
    static SEEN: RefCell<HashMap<u64, (u64, HashSet<Ref>)>> = RefCell::new(HashMap::new());
    /// Manual sensors' last Sense() readings.
    static SENSED: RefCell<HashMap<Ref, Vec<(String, Vec3)>>> = RefCell::new(HashMap::new());
}

fn one_attachment(d: &Document, r: Ref, class: &str) -> bool {
    match class {
        "VectorForce" | "Torque" | "LinearVelocity" | "AngularVelocity" | "LineForce" | "Plane" => {
            class != "LineForce" && class != "Plane"
        }
        "AlignPosition" | "AlignOrientation" => crate::scene::enum_u32(d.prop(r, "Mode")) == Some(0),
        _ => false,
    }
}

/// Whether the constraint could act now (everything but "simulated yet").
fn could_act(d: &Document, r: Ref, class: &str) -> bool {
    let on = if class == "WeldConstraint" { crate::physics::joint_enabled(d, r) } else { boolp(d, r, "Enabled", true) };
    if !on || !in_workspace(d, r) {
        return false;
    }
    let ends: Vec<Option<Ref>> = if class == "WeldConstraint" || class == "NoCollisionConstraint" {
        vec![refp(d, r, "Part0"), refp(d, r, "Part1")]
    } else {
        let p = |k: &str| refp(d, r, k).and_then(|a| d.dom.get_by_ref(a)).map(|a| a.parent()).filter(|p| is_part(d, *p));
        if one_attachment(d, r, class) {
            vec![p("Attachment0")]
        } else {
            vec![p("Attachment0"), p("Attachment1")]
        }
    };
    if ends.iter().any(|e| e.is_none_or(|p| !in_workspace(d, p))) {
        return false;
    }
    // Both ends anchored themselves (a part welded to an anchored one still
    // counts as free here: Studio's WeldConstraint to an anchored part is Active).
    !ends.iter().flatten().all(|p| boolp(d, *p, "Anchored", false))
}

fn active(lua: &Lua, r: Ref, class: &str) -> mlua::Result<bool> {
    let w = world(lua)?;
    let d = w.d();
    if !could_act(d, r, class) {
        return Ok(false);
    }
    Ok(SEEN.with(|s| {
        let mut s = s.borrow_mut();
        let e = seen_fresh(&mut s, d);
        e.contains(&r)
    }))
}

/// The constraints under Workspace (simulated: Active), re-read when the
/// tree changed since they were last asked for. Only on a read of Active:
/// `step` did this walk of the whole Workspace on every tree change, which
/// in a live Place A match is every frame (1 ms a frame, 110 ms at map spawn).
fn seen_fresh<'a>(s: &'a mut HashMap<u64, (u64, HashSet<Ref>)>, d: &Document) -> &'a HashSet<Ref> {
    let e = s.entry(d.id).or_insert((u64::MAX, HashSet::new()));
    if e.0 != d.tree_revision {
        e.0 = d.tree_revision;
        e.1 = match d.workspace() {
            Some(ws) => d
                .dom
                .descendants_of(ws)
                .filter(|i| constraint_class(&i.class))
                .map(|i| i.referent())
                .collect(),
            None => HashSet::new(),
        };
    }
    &e.1
}

/// The two attachments' world frames.
fn frames(d: &Document, r: Ref) -> Option<(Xform, Xform)> {
    let a0 = attachment_world(d, refp(d, r, "Attachment0")?)?;
    let a1 = attachment_world(d, refp(d, r, "Attachment1")?)?;
    Some((a0, a1))
}

/// Signed angle (degrees) about `n` from `r0` to `v` projected on n's plane.
fn signed_angle(n: Vec3, r0: Vec3, v: Vec3) -> f32 {
    let v = v - n * n.dot(v);
    r0.cross(v).dot(n).atan2(r0.dot(v)).to_degrees()
}

fn cyl_axes(d: &Document, r: Ref, a0: &Xform) -> (Vec3, Vec3) {
    let inc = f32p(d, r, "InclinationAngle", 0.0).to_radians();
    let (s, c) = inc.sin_cos();
    (a0.rot.x_axis * c + a0.rot.y_axis * s, a0.rot.y_axis * c - a0.rot.x_axis * s)
}

/// Attachment / AlignOrientation axis setters (Studio's, measured): the new
/// primary axis keeps the old secondary where it can (else the old Z).
pub fn with_axis(old: Mat3, axis: Vec3) -> Option<Mat3> {
    let x = axis.try_normalize()?;
    let mut z = x.cross(old.y_axis);
    if z.length_squared() < 1e-10 {
        z = old.z_axis - x * x.dot(old.z_axis);
    }
    let z = z.try_normalize()?;
    Some(Mat3::from_cols(x, z.cross(x), z))
}

pub fn with_secondary(old: Mat3, axis: Vec3) -> Option<Mat3> {
    let y = axis.try_normalize()?;
    let x = old.x_axis;
    let z = x.cross(y).try_normalize()?;
    Some(Mat3::from_cols(x, z.cross(x), z))
}

fn set_rot(lua: &Lua, r: Ref, rot: Mat3) -> mlua::Result<()> {
    let pos = xf(world(lua)?.d(), r, "CFrame").map(|x| x.pos).unwrap_or(Vec3::ZERO);
    store_prop(lua, r, "CFrame", Variant::CFrame(Xform { pos, rot }.to_cframe()))
}

// ---------------------------------------------------------------- sensors

fn wind_scale(y: f32) -> f32 {
    if y < 0.0 {
        return 0.0;
    }
    (y.max(7.68e-5) / 5.0).powf(0.20735).min(3.0)
}

fn air_density(d: &Document, y: f32) -> f32 {
    let base = d.workspace().map(|ws| f32p(d, ws, "AirDensity", 0.0012)).unwrap_or(0.0012);
    base * (-y.max(0.0) / 33381.0).exp()
}

fn sensor_part(d: &Document, r: Ref) -> Option<Ref> {
    let p = d.dom.get_by_ref(r)?.parent();
    is_part(d, p).then_some(p)
}

/// The sensor's readings now: (name, value) — floats as Vec3.x.
fn read_sensor(lua: &Lua, r: Ref, class: &str) -> mlua::Result<Vec<(String, Vec3)>> {
    let w = world(lua)?;
    let d = w.d();
    let part = sensor_part(d, r);
    let pos = part.and_then(|p| xf(d, p, "CFrame")).map(|x| x.pos).unwrap_or(Vec3::ZERO);
    let vel = part.and_then(|p| w.velocities.get(&p).copied()).unwrap_or(Vec3::ZERO);
    Ok(match class {
        "AtmosphereSensor" => {
            let wind = d.workspace().and_then(|ws| v3p(d, ws, "GlobalWind")).unwrap_or(Vec3::ZERO);
            let rel = if part.is_some() { wind * wind_scale(pos.y) - vel } else { Vec3::ZERO };
            vec![
                ("AirDensity".into(), Vec3::new(if part.is_some() { air_density(d, pos.y) } else { 0.0 }, 0.0, 0.0)),
                ("RelativeWindVelocity".into(), rel),
            ]
        }
        "FluidForceSensor" => {
            let on = d.workspace().and_then(|ws| crate::scene::enum_u32(d.prop(ws, "FluidForces"))) == Some(1);
            let (f, t, c) = match (on, part) {
                (true, Some(p)) => {
                    let x = xf(d, p, "CFrame").unwrap_or(Xform { pos, rot: Mat3::IDENTITY });
                    let wv = crate::physics::live::with_stats(d.id, |s| s.spin.get(&p).map(|s| s.0))
                        .flatten()
                        .unwrap_or(Vec3::ZERO);
                    let wind = d.workspace().and_then(|ws| v3p(d, ws, "GlobalWind")).unwrap_or(Vec3::ZERO);
                    fluid(d, p, &x, vel - wind * wind_scale(x.pos.y), wv)
                }
                _ => (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO),
            };
            vec![("Force".into(), f), ("Torque".into(), t), ("CenterOfPressure".into(), c)]
        }
        "PhysicsActivitySensor" => {
            let p = v3p(d, r, "Position").unwrap_or(Vec3::ZERO);
            let h = v3p(d, r, "Size").unwrap_or(Vec3::ZERO) * 0.5;
            let busy = w.velocities.iter().any(|(q, v)| {
                v.length_squared() > 1e-6
                    && xf(d, *q, "CFrame").is_some_and(|x| (x.pos - p).abs().cmple(h).all())
            });
            vec![("Active".into(), Vec3::new(f32::from(u8::from(busy)), 0.0, 0.0))]
        }
        _ => Vec::new(),
    })
}

/// The pressure model fitted to Studio's FluidForceSensor:EvaluateAsync.
fn fluid(d: &Document, part: Ref, x: &Xform, v: Vec3, w: Vec3) -> (Vec3, Vec3, Vec3) {
    let s = v3p(d, part, "Size").unwrap_or(Vec3::ONE);
    let rho = air_density(d, x.pos.y);
    let h = s * 0.5;
    let faces = [
        (x.rot.x_axis, h.x, s.y * s.z),
        (x.rot.y_axis, h.y, s.x * s.z),
        (x.rot.z_axis, h.z, s.x * s.y),
    ];
    let mut f = Vec3::ZERO;
    let mut t = Vec3::ZERO;
    for (axis, half, area) in faces {
        for n in [axis, -axis] {
            let c = n * half;
            let vn = (v + w.cross(c)).dot(n);
            if vn > 0.0 {
                let ff = -n * (1.2 * rho * vn * vn * area);
                f += ff;
                t += c.cross(ff);
            }
        }
    }
    let cop = if f.length_squared() > 1e-12 { x.pos + f.cross(t) / f.length_squared() } else { x.pos };
    (f, t, cop)
}

fn sensor_value(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    let manual = crate::scene::enum_u32(world(lua)?.d().prop(r, "UpdateType")) == Some(1);
    let readings = if manual {
        SENSED.with(|s| s.borrow().get(&r).cloned()).unwrap_or_default()
    } else {
        read_sensor(lua, r, class)?
    };
    let v = readings.iter().find(|(k, _)| k == key).map(|(_, v)| *v).unwrap_or(Vec3::ZERO);
    Ok(Some(match key {
        "AirDensity" => Value::Number(v.x as f64),
        "Active" => Value::Boolean(v.x > 0.5),
        _ => vec3(lua, v)?,
    }))
}

// ------------------------------------------------------------------ hooks

pub fn call(lua: &Lua, name: &str, this: Ref, class: &str, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    Some(match name {
        "GetDebugAppliedForce" | "GetDebugAppliedTorque" if doc::is_a(class, "Constraint") => {
            if arg(args, 1).is_nil() {
                Err(missing(1))
            } else {
                Err(rt_err("Querying applied forces is not currently supported."))
            }
        }
        "GetConstraints" if class == "Attachment" => {
            let found = {
                let w = world(lua).ok()?;
                let d = w.d();
                d.dom
                    .descendants()
                    .filter(|i| doc::is_a(&i.class, "Constraint"))
                    .filter(|i| {
                        ["Attachment0", "Attachment1"]
                            .iter()
                            .any(|k| matches!(i.properties.get(&rbx_dom_weak::ustr(k)), Some(Variant::Ref(x)) if *x == this))
                    })
                    .map(|i| i.referent())
                    .collect::<Vec<_>>()
            };
            list(lua, found).map(Value::Table).and_then(one)
        }
        "SetAxis" | "SetSecondaryAxis" if class == "Attachment" => (|| {
            let v = vec3_arg(args, 1)?;
            let old = xf(world(lua)?.d(), this, "CFrame").map(|x| x.rot).unwrap_or(Mat3::IDENTITY);
            let new = if name == "SetAxis" { with_axis(old, v) } else { with_secondary(old, v) };
            if let Some(rot) = new {
                set_rot(lua, this, rot)?;
            }
            none()
        })(),
        "GetLastForce" | "lastForce" if matches!(class, "BodyPosition" | "BodyVelocity") => {
            vec3(lua, Vec3::ZERO).and_then(one)
        }
        "Fire" | "fire" if class == "RocketPropulsion" => {
            crate::physics::live::rocket_fire(this);
            none()
        }
        "Abort" if class == "RocketPropulsion" => {
            crate::physics::live::rocket_abort(this);
            none()
        }
        "Sense" if doc::is_a(class, "SensorBase") => (|| {
            let now = read_sensor(lua, this, class)?;
            let old = SENSED.with(|s| s.borrow_mut().insert(this, now.clone()));
            if old.as_ref() != Some(&now) {
                world(lua)?.fire(this, "OnSensorOutputChanged", vec![]);
            }
            none()
        })(),
        "EvaluateAsync" if class == "FluidForceSensor" => (|| {
            let v = vec3_arg(args, 1)?;
            let wv = vec3_arg(args, 2)?;
            let cf = ud::<LCFrame>(&arg(args, 3)).ok_or_else(|| missing(3))?.0;
            let (f, t, c) = {
                let w = world(lua)?;
                let d = w.d();
                match sensor_part(d, this) {
                    Some(p) => fluid(d, p, &cf, v, wv),
                    None => (Vec3::ZERO, Vec3::ZERO, cf.pos),
                }
            };
            Ok(MultiValue::from_iter([vec3(lua, f)?, vec3(lua, t)?, vec3(lua, c)?]))
        })(),
        _ => return None,
    })
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    // WeldConstraint isn't a Constraint, but its Active works the same way
    // (Studio: false until simulated, false with both parts anchored).
    if class == "WeldConstraint" && key == "Active" {
        return Ok(Some(Value::Boolean(active(lua, r, class)?)));
    }
    // Enabled isn't saved or replicated: it comes from State (bit 0).
    if class == "WeldConstraint" && key == "Enabled" {
        let w = world(lua)?;
        return Ok(Some(Value::Boolean(crate::physics::joint_enabled(w.d(), r))));
    }
    if doc::is_a(class, "Constraint") {
        if key == "Active" {
            return Ok(Some(Value::Boolean(active(lua, r, class)?)));
        }
        let v = {
            let w = world(lua)?;
            let d = w.d();
            let fr = frames(d, r);
            match (class, key) {
                ("HingeConstraint", "CurrentAngle") => {
                    fr.map(|(a0, a1)| signed_angle(a0.rot.x_axis, a0.rot.y_axis, a1.rot.y_axis))
                }
                ("CylindricalConstraint", "CurrentAngle") => fr.map(|(a0, a1)| {
                    let (n, r0) = cyl_axes(d, r, &a0);
                    signed_angle(n, r0, a1.rot.y_axis)
                }),
                ("TorsionSpringConstraint", "CurrentAngle") => {
                    fr.map(|(a0, a1)| a0.rot.y_axis.dot(a1.rot.y_axis).clamp(-1.0, 1.0).acos().to_degrees())
                }
                ("PrismaticConstraint" | "CylindricalConstraint" | "SlidingBallConstraint", "CurrentPosition") => {
                    fr.map(|(a0, a1)| (a1.pos - a0.pos).dot(a0.rot.x_axis))
                }
                ("RodConstraint" | "RopeConstraint", "CurrentDistance") | ("SpringConstraint", "CurrentLength") => {
                    fr.map(|(a0, a1)| (a1.pos - a0.pos).length())
                }
                ("CylindricalConstraint", "WorldRotationAxis") => {
                    let axis = refp(d, r, "Attachment0")
                        .and_then(|a| attachment_world(d, a))
                        .map(|a0| cyl_axes(d, r, &a0).0)
                        .unwrap_or(Vec3::X);
                    drop(w);
                    return Ok(Some(vec3(lua, axis)?));
                }
                ("AlignOrientation", "PrimaryAxis" | "SecondaryAxis" | "LookAtPosition") => {
                    let x = xf(d, r, "CFrame").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY });
                    let v = match key {
                        "PrimaryAxis" => x.rot.x_axis,
                        "SecondaryAxis" => x.rot.y_axis,
                        _ => x.pos,
                    };
                    drop(w);
                    return Ok(Some(vec3(lua, v)?));
                }
                ("BallSocketConstraint", "MaxFrictionTorque") => Some(f32p(d, r, key, 0.0)),
                (_, "CurrentAngle" | "CurrentPosition" | "CurrentDistance" | "CurrentLength") => Some(0.0),
                _ => return Ok(None),
            }
        };
        return Ok(Some(Value::Number(v.unwrap_or(0.0) as f64)));
    }
    if doc::is_a(class, "SensorBase") {
        let ours = matches!(
            (class, key),
            ("AtmosphereSensor", "AirDensity" | "RelativeWindVelocity")
                | ("FluidForceSensor", "Force" | "Torque" | "CenterOfPressure")
                | ("PhysicsActivitySensor", "Active")
        );
        if ours {
            return sensor_value(lua, r, class, key);
        }
        if class == "FluidForceSensor" && matches!(key, "BuoyancyForce" | "BuoyancyTorque") {
            let full = world(lua)?.full_name(r);
            return Err(rt_err(format!("{key} is not a valid member of {class} \"{full}\"")));
        }
        if class == "PhysicsActivitySensor" && matches!(key, "Position" | "Size") {
            return Ok(Some(vec3(lua, v3p(world(lua)?.d(), r, key).unwrap_or(Vec3::ZERO))?));
        }
    }
    Ok(None)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    let vec_in = || {
        ud::<LVector3>(value).map(|v| v.0).ok_or_else(|| {
            rt_err(format!("Unable to assign property {key}. Vector3 expected, got {}", value.type_name()))
        })
    };
    match (class, key) {
        ("AlignOrientation", "PrimaryAxis" | "SecondaryAxis") => {
            let v = vec_in()?;
            let old = xf(world(lua)?.d(), r, "CFrame").map(|x| x.rot).unwrap_or(Mat3::IDENTITY);
            let new = if key == "PrimaryAxis" { with_axis(old, v) } else { with_secondary(old, v) };
            if let Some(rot) = new {
                set_rot(lua, r, rot)?;
            }
            Ok(true)
        }
        ("AlignOrientation", "LookAtPosition") => {
            let v = vec_in()?;
            let rot = xf(world(lua)?.d(), r, "CFrame").map(|x| x.rot).unwrap_or(Mat3::IDENTITY);
            store_prop(lua, r, "CFrame", Variant::CFrame(Xform { pos: v, rot }.to_cframe()))?;
            Ok(true)
        }
        ("BallSocketConstraint", "MaxFrictionTorque") => {
            let n = crate::ltypes::num(value).ok_or_else(|| {
                rt_err(format!("Unable to assign property {key}. number expected, got {}", value.type_name()))
            })?;
            store_prop(lua, r, key, Variant::Float32(n as f32))?;
            Ok(true)
        }
        ("PhysicsActivitySensor", "Position" | "Size") => {
            let v = vec_in()?;
            store_prop(lua, r, key, Variant::Vector3(rbx_dom_weak::types::Vector3::new(v.x, v.y, v.z)))?;
            Ok(true)
        }
        // A new goal may be reached again (the store itself is the runtime's).
        ("BodyPosition", "Position") => {
            crate::physics::live::bp_rearm(r);
            Ok(false)
        }
        _ => Ok(false),
    }
}

pub fn write_acts(class: &str, key: &str) -> bool {
    matches!(
        (class, key),
        ("AlignOrientation", "PrimaryAxis" | "SecondaryAxis" | "LookAtPosition") | ("BodyPosition", "Position")
    )
}

/// Constraint class names (fast check without the reflection walk).
fn constraint_class(c: &str) -> bool {
    c.ends_with("Constraint") || matches!(
        c,
        "AlignPosition" | "AlignOrientation" | "AngularVelocity" | "LinearVelocity" | "VectorForce" | "Torque" | "LineForce" | "Plane" | "AnimationConstraint"
    )
}

pub fn step(lua: &Lua, _dt: f64) -> mlua::Result<()> {
    let (doc_id, reached) = {
        let w = match world(lua) {
            Ok(w) => w,
            Err(_) => return Ok(()),
        };
        let d = w.d();
        (d.id, crate::physics::live::take_reached(d.id))
    };
    if !reached.is_empty() {
        let mut w = world(lua)?;
        for r in reached {
            if w.d().exists(r) {
                w.fire(r, "ReachedTarget", vec![]);
            }
        }
    }
    let _ = doc_id;
    Ok(())
}
