//! Camera members (Studio 0.705, measured in an offline Studio):
//! - DiagonalFieldOfView: Studio's own formula, tan(d/2) = tan²(h/2) +
//!   tan²(v/2) (no square root: 70° on 862×675 reads 104.43), and its
//!   inverse when set; MaxAxisFieldOfView is the wider of the vertical and
//!   horizontal angles. A camera that isn't Workspace.CurrentCamera has a
//!   1×1 viewport. Both write FieldOfView.
//! - PanUnits(n): orbits Focus by n × 45° (the yaw snapped to 45° steps),
//!   aiming at Focus with the pitch kept; TiltUnits(n): pitch + n × 10°
//!   snapped to 10° steps, within ±81°, distance to Focus kept; returns
//!   true. GetPanSpeed / GetTiltSpeed are 0; SetCameraPanMode changes
//!   nothing a script can see.
//! - Interpolate(cframe, focus, duration): needs CameraType Scriptable and
//!   a positive duration (Studio's errors); moves CFrame to cframe's
//!   position looking at focus and Focus to focus over `duration`, then
//!   fires InterpolationFinished; a new Interpolate replaces a running one
//!   without firing. (Studio's path curves about the focus and its final
//!   Focus differs; rustudio's is straight.)
//! - ProjectionType / OrthographicMode / OrthographicSize / ViewStretch:
//!   kept and read back (defaults Perspective, Vertical, 300, 1;
//!   OrthographicSize clamped to [0.001, 100000]). Studio gates them on the
//!   feature flag SupportOrthoCameraApi (setter  asks
//!   , which refuses anything but Perspective while the flag at
//!    is off): an offline Studio has it off and ignores
//!   the writes; with it on (as live Roblox) the camera renders
//!   orthographic — rustudio behaves as the flag-on Studio: camera.rs
//!   (projection, rays), render.rs (no sky), library.luau (rays / points).

use std::cell::RefCell;
use std::collections::HashMap;

use glam::{Mat3, Quat, Vec3};
use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};

use super::util::*;
use crate::doc;
use crate::ltypes::*;
use crate::math::Xform;
use crate::runtime::*;

struct Interp {
    from: Xform,
    to: Xform,
    focus_from: Xform,
    focus_to: Xform,
    t: f64,
    dur: f64,
}

thread_local! {
    static INTERP: RefCell<HashMap<(u64, Ref), Interp>> = RefCell::new(HashMap::new());
}

fn aspect(lua: &Lua, cam: Ref) -> mlua::Result<f32> {
    let w = world(lua)?;
    let d = w.d();
    let current = d.workspace().and_then(|ws| refp(d, ws, "CurrentCamera")) == Some(cam);
    if !current {
        return Ok(1.0);
    }
    let v = w.input.view_size;
    let v = if v[0] > 0.0 { v } else { [1265.0, 836.0] };
    Ok(v[0] / v[1].max(1.0))
}

fn fov(lua: &Lua, cam: Ref) -> mlua::Result<f32> {
    Ok(f32p(world(lua)?.d(), cam, "FieldOfView", 70.0))
}

fn set_fov(lua: &Lua, cam: Ref, deg: f32) -> mlua::Result<()> {
    store_prop(lua, cam, "FieldOfView", Variant::Float32(deg.clamp(1.0, 120.0)))
}

fn half_tan(deg: f32) -> f32 {
    (deg.to_radians() * 0.5).tan()
}

fn from_half_tan(t: f32) -> f32 {
    (t.atan() * 2.0).to_degrees()
}

pub fn call(lua: &Lua, name: &str, this: Ref, class: &str, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if class != "Camera" {
        return None;
    }
    Some(match name {
        "GetPanSpeed" | "GetTiltSpeed" => one(Value::Number(0.0)),
        "SetCameraPanMode" => none(),
        "PanUnits" | "TiltUnits" => (|| {
            // `units` is an int: Studio rounds (PanUnits(0.5) pans one step).
            let n = (num_arg(args, 1).ok_or_else(|| missing(1))? as f32).round();
            let (cf, focus) = {
                let w = world(lua)?;
                let d = w.d();
                (
                    xf(d, this, "CFrame").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }),
                    xf(d, this, "Focus").map(|f| f.pos).unwrap_or(Vec3::ZERO),
                )
            };
            let off = cf.pos - focus;
            let dist = off.length();
            let look = -cf.rot.z_axis;
            let pitch = look.y.clamp(-1.0, 1.0).asin().to_degrees();
            // Yaw of the camera's position around the focus (0 = +Z side).
            let yaw = off.x.atan2(off.z).to_degrees();
            let (yaw, pitch) = if name == "PanUnits" {
                (((yaw - 45.0 * n) / 45.0).round() * 45.0, pitch)
            } else {
                (yaw, (((pitch + 10.0 * n) / 10.0).round() * 10.0).clamp(-81.0, 81.0))
            };
            let (sy, cy) = yaw.to_radians().sin_cos();
            let (sp, cp) = pitch.to_radians().sin_cos();
            // Looking toward the focus: the position sits opposite the look.
            let dir = Vec3::new(-sy * cp, sp, -cy * cp);
            let pos = focus - dir * dist;
            let x = look_at(pos, pos + dir, Vec3::Y);
            store_prop(lua, this, "CFrame", Variant::CFrame(x.to_cframe()))?;
            if name == "TiltUnits" {
                one(Value::Boolean(true))
            } else {
                none()
            }
        })(),
        "Interpolate" => (|| {
            let to = ud::<LCFrame>(&arg(args, 1)).ok_or_else(|| missing(1))?.0;
            let focus = ud::<LCFrame>(&arg(args, 2)).ok_or_else(|| missing(2))?.0;
            let dur = num_arg(args, 3).ok_or_else(|| missing(3))?;
            let (from, focus_from, scriptable, doc_id) = {
                let w = world(lua)?;
                let d = w.d();
                (
                    xf(d, this, "CFrame").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }),
                    xf(d, this, "Focus").unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }),
                    crate::scene::enum_u32(d.prop(this, "CameraType")) == Some(6),
                    d.id,
                )
            };
            if !scriptable {
                return Err(rt_err("Attempted to use interpolation with a camera mode other than scriptable."));
            }
            if !(dur > 0.0) {
                return Err(rt_err("Interpolation time must be positive."));
            }
            let target = if (focus.pos - to.pos).length_squared() > 1e-8 {
                look_at(to.pos, focus.pos, Vec3::Y)
            } else {
                to
            };
            INTERP.with(|m| {
                m.borrow_mut().insert(
                    (doc_id, this),
                    Interp { from, to: target, focus_from, focus_to: focus, t: 0.0, dur },
                )
            });
            none()
        })(),
        _ => return None,
    })
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if class != "Camera" {
        return Ok(None);
    }
    Ok(Some(match key {
        "DiagonalFieldOfView" => {
            let tv = half_tan(fov(lua, r)?);
            let a = aspect(lua, r)?;
            Value::Number(from_half_tan(tv * tv * (1.0 + a * a)) as f64)
        }
        "MaxAxisFieldOfView" => {
            let v = fov(lua, r)?;
            let a = aspect(lua, r)?;
            Value::Number(if a >= 1.0 { from_half_tan(half_tan(v) * a) } else { v } as f64)
        }
        "ProjectionType" | "OrthographicMode" => {
            let (ty, def) = if key == "ProjectionType" {
                ("ProjectionType", "Perspective")
            } else {
                ("OrthographicMode", "Vertical")
            };
            match crate::scene::enum_u32(world(lua)?.d().prop(r, key)).and_then(|v| LEnumItem::from_value(ty, v)) {
                Some(e) => enum_value(lua, e)?,
                None => enum_item(lua, ty, def)?,
            }
        }
        "OrthographicSize" => Value::Number(f32p(world(lua)?.d(), r, key, 300.0) as f64),
        "ViewStretch" => Value::Number(f32p(world(lua)?.d(), r, key, 1.0) as f64),
        _ => return Ok(None),
    }))
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if class != "Camera" {
        return Ok(false);
    }
    let number = || {
        crate::ltypes::num(value).map(|n| n as f32).ok_or_else(|| {
            rt_err(format!("Unable to assign property {key}. number expected, got {}", value.type_name()))
        })
    };
    match key {
        "DiagonalFieldOfView" => {
            let d = number()?;
            let a = aspect(lua, r)?;
            let tv = (half_tan(d) / (1.0 + a * a)).max(0.0).sqrt();
            set_fov(lua, r, from_half_tan(tv))?;
        }
        "MaxAxisFieldOfView" => {
            let m = number()?;
            let a = aspect(lua, r)?;
            set_fov(lua, r, if a >= 1.0 { from_half_tan(half_tan(m) / a) } else { m })?;
        }
        // Studio clamps OrthographicSize to [0.001, 100000].
        "OrthographicSize" => {
            store_prop(lua, r, key, Variant::Float32(number()?.clamp(0.001, 100000.0)))?;
        }
        "ViewStretch" => {
            store_prop(lua, r, key, Variant::Float32(number()?))?;
        }
        "ProjectionType" | "OrthographicMode" => {
            let v = enum_of(value, key).ok_or_else(|| {
                rt_err(format!("Unable to assign property {key}. EnumItem expected, got {}", value.type_name()))
            })?;
            store_prop(lua, r, key, Variant::Enum(rbx_dom_weak::types::Enum::from_u32(v)))?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub fn write_acts(class: &str, key: &str) -> bool {
    class == "Camera" && matches!(key, "DiagonalFieldOfView" | "MaxAxisFieldOfView")
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    let doc_id = match world(lua) {
        Ok(w) => w.d().id,
        Err(_) => return Ok(()),
    };
    let mut moves: Vec<(Ref, Xform, Xform, bool)> = Vec::new();
    INTERP.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|(id, cam), it| {
            if *id != doc_id {
                return true;
            }
            it.t += dt;
            let k = (it.t / it.dur).min(1.0) as f32;
            let qa = Quat::from_mat3(&it.from.rot).normalize();
            let qb = Quat::from_mat3(&it.to.rot).normalize();
            let cf = Xform { pos: it.from.pos.lerp(it.to.pos, k), rot: Mat3::from_quat(qa.slerp(qb, k)) };
            let focus = Xform { pos: it.focus_from.pos.lerp(it.focus_to.pos, k), rot: it.focus_to.rot };
            let done = k >= 1.0;
            moves.push((*cam, cf, focus, done));
            !done
        });
    });
    for (cam, cf, focus, done) in moves {
        if !world(lua)?.d().exists(cam) {
            continue;
        }
        store_prop(lua, cam, "CFrame", Variant::CFrame(cf.to_cframe()))?;
        store_prop(lua, cam, "Focus", Variant::CFrame(focus.to_cframe()))?;
        if done {
            world(lua)?.fire(cam, "InterpolationFinished", vec![]);
        }
    }
    Ok(())
}
