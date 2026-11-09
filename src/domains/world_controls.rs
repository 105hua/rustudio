//! Controller-side members (Studio 0.705, measured in an offline Studio):
//! - ControlState: Add*Field declares a field (defaults as the API dump;
//!   a second Add of the same name keeps the first); SetField /
//!   UpdateFields stage values, clamped as Studio does (Number to
//!   [min, max], Int rounded then clamped, Vector3 / Vector2 to their max
//!   magnitude, UnitVector3 normalised) and type-checked with Studio's
//!   messages. Staged values become the state once per frame while the
//!   ControlState is under Workspace, and OnStateChanged fires when that
//!   changed something. GetState(ref) answers the current (or previous)
//!   state as a dictionary (Instance fields aren't in it, as in Studio);
//!   GetChangedState(prev, next) the fields that differ. HasField is
//!   switched off in Studio ("ControlState.HasField is not enabled");
//!   GetReplicationWeight is 64.
//! - Controller (Humanoid / Vehicle / SkateboardController): BindButton
//!   (button, caption) / UnbindButton; GetButton errors until bound, then
//!   answers whether the button is held (Jump = Space; Dismount =
//!   Backspace) on the client; ButtonChanged fires when a bound button's
//!   state changes.
//! - SkateboardPlatform mounting (server; Studio 0.744 measured, mount at
//!   ): a Humanoid standing on a platform takes it —
//!   ControllingHumanoid is set and the Humanoid goes PlatformStand — and,
//!   when the Humanoid and the platform are both direct children of one
//!   Model, a SkateboardController is made (Controller) and Equipped
//!   (humanoid, controller) fires (also `equipped`). Otherwise Studio logs
//!   "Error mounting skateboard platform. Both Humanoid and
//!   SkateboardPlatform must be direct children of a Model" and leaves the
//!   humanoid platform-standing on it with no controller and no way off.
//!   (Studio itself crashes on the successful mount — twice, in its own
//!   Play — so the riding itself, Steer / Throttle / AxisChanged /
//!   MoveStateChanged / Unequipped, can't be measured: Steer and Throttle
//!   read 0 and those events don't fire.)
//! - SkateboardPlatform:ApplySpecificImpulse(v): adds v to the velocity
//!   (an impulse per unit mass: Studio's 100 up read 97.7 a frame later).
//! - ControllerBase.Active: the ControllerManager's ActiveController with a
//!   RootPart under Workspace. AirController.LinearImpulse reads back what
//!   was written.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{Vec2, Vec3};
use mlua::{Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::{Ref, Variant};

use super::util::*;
use crate::doc;
use crate::ltypes::*;
use crate::math::Xform;
use crate::runtime::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Bool,
    Number(f64, f64),
    Int(f64, f64),
    Vector3(f64),
    Unit3,
    Vector2(f64),
    CFrame,
    Instance,
}

#[derive(Clone, PartialEq, Debug)]
enum Val {
    Bool(bool),
    Num(f64),
    V3(Vec3),
    V2(Vec2),
    CF([f32; 12]),
    Inst(Ref),
}

#[derive(Default)]
struct State {
    kinds: BTreeMap<String, Kind>,
    current: BTreeMap<String, Val>,
    previous: BTreeMap<String, Val>,
    staged: BTreeMap<String, Val>,
}

thread_local! {
    static STATES: RefCell<HashMap<(u64, Ref), State>> = RefCell::new(HashMap::new());
    /// (document, platform) → (humanoid, its SkateboardController if mounted).
    static BOARDS: RefCell<HashMap<(u64, Ref), (Ref, Option<Ref>)>> = RefCell::new(HashMap::new());
    /// (document) → (tree revision, the SkateboardPlatforms under Workspace).
    static BOARD_LIST: RefCell<HashMap<u64, (u64, Vec<Ref>)>> = RefCell::new(HashMap::new());
    /// (document, controller) → bound buttons → last held state.
    static BUTTONS: RefCell<HashMap<(u64, Ref), HashMap<u32, bool>>> = RefCell::new(HashMap::new());
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Bool => "a boolean",
        Kind::Number(..) | Kind::Int(..) => "a number",
        Kind::Vector3(_) | Kind::Unit3 => "a Vector3",
        Kind::Vector2(_) => "a Vector2",
        Kind::CFrame => "a CFrame",
        Kind::Instance => "an Instance",
    }
}

fn convert(name: &str, k: Kind, v: &Value) -> mlua::Result<Val> {
    let bad = || rt_err(format!("ControlState: field '{name}' expects {}", kind_name(k)));
    Ok(match k {
        Kind::Bool => match v {
            Value::Boolean(b) => Val::Bool(*b),
            _ => return Err(bad()),
        },
        Kind::Number(lo, hi) => Val::Num(crate::ltypes::num(v).ok_or_else(bad)?.clamp(lo, hi.max(lo))),
        Kind::Int(lo, hi) => Val::Num(crate::ltypes::num(v).ok_or_else(bad)?.round().clamp(lo, hi.max(lo))),
        Kind::Vector3(max) => Val::V3(ud::<LVector3>(v).ok_or_else(bad)?.0.clamp_length_max(max as f32)),
        Kind::Unit3 => Val::V3(ud::<LVector3>(v).ok_or_else(bad)?.0.normalize_or_zero()),
        Kind::Vector2(max) => Val::V2(ud::<LVector2>(v).ok_or_else(bad)?.0.clamp_length_max(max as f32)),
        Kind::CFrame => {
            let x = ud::<LCFrame>(v).ok_or_else(bad)?.0;
            let r = x.rot;
            Val::CF([
                x.pos.x, x.pos.y, x.pos.z, r.x_axis.x, r.y_axis.x, r.z_axis.x, r.x_axis.y, r.y_axis.y, r.z_axis.y,
                r.x_axis.z, r.y_axis.z, r.z_axis.z,
            ])
        }
        Kind::Instance => match v {
            Value::Nil => Val::Inst(Ref::none()),
            v => Val::Inst(opt_inst(v).ok_or_else(bad)?),
        },
    })
}

fn to_lua(lua: &Lua, v: &Val) -> mlua::Result<Value> {
    Ok(match v {
        Val::Bool(b) => Value::Boolean(*b),
        Val::Num(n) => Value::Number(*n),
        Val::V3(x) => vec3(lua, *x)?,
        Val::V2(x) => Value::UserData(lua.create_userdata(LVector2(*x))?),
        Val::CF(c) => {
            let rot = glam::Mat3::from_cols(
                Vec3::new(c[3], c[6], c[9]),
                Vec3::new(c[4], c[7], c[10]),
                Vec3::new(c[5], c[8], c[11]),
            );
            Value::UserData(lua.create_userdata(LCFrame(Xform { pos: Vec3::new(c[0], c[1], c[2]), rot }))?)
        }
        Val::Inst(r) => inst_value(lua, *r)?,
    })
}

fn dict(lua: &Lua, m: &BTreeMap<String, Val>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (k, v) in m {
        if !matches!(v, Val::Inst(_)) {
            t.set(k.as_str(), to_lua(lua, v)?)?;
        }
    }
    Ok(t)
}

fn name_arg(args: &MultiValue, i: usize) -> mlua::Result<String> {
    match arg(args, i) {
        Value::String(s) => Ok(s.to_str()?.to_string()),
        Value::Nil => Err(missing(i)),
        v => Err(rt_err(format!("Unable to cast {} to string", v.type_name()))),
    }
}

/// Which state a StateReferenceFrame names (CurrentState 0, PreviousState
/// 1, LastObservedState 2 = current).
fn pick(st: &State, v: &Value, default_prev: bool) -> BTreeMap<String, Val> {
    let r = enum_of(v, "StateReferenceFrame").unwrap_or(if default_prev { 1 } else { 0 });
    if r == 1 { st.previous.clone() } else { st.current.clone() }
}

fn button_key(b: u32) -> &'static str {
    if b == 32 { "Space" } else { "Backspace" }
}

pub fn call(lua: &Lua, name: &str, this: Ref, class: &str, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if class == "ControlState" {
        return control_state(lua, name, this, args);
    }
    if doc::is_a(class, "Controller") {
        return Some(match name {
            "BindButton" | "bindButton" => (|| {
                if arg(args, 2).is_nil() {
                    return Err(missing(2));
                }
                let b = enum_of(&arg(args, 1), "Button").ok_or_else(|| missing(1))?;
                let id = world(lua)?.d().id;
                BUTTONS.with(|m| m.borrow_mut().entry((id, this)).or_default().insert(b, false));
                none()
            })(),
            "UnbindButton" => (|| {
                let b = enum_of(&arg(args, 1), "Button").ok_or_else(|| missing(1))?;
                let id = world(lua)?.d().id;
                BUTTONS.with(|m| {
                    if let Some(x) = m.borrow_mut().get_mut(&(id, this)) {
                        x.remove(&b);
                    }
                });
                none()
            })(),
            "GetButton" | "getButton" => (|| {
                let b = enum_of(&arg(args, 1), "Button").ok_or_else(|| missing(1))?;
                let (id, held) = {
                    let w = world(lua)?;
                    (w.d().id, w.side == Side::Client && w.keys_down.contains(button_key(b)))
                };
                let bound = BUTTONS.with(|m| m.borrow().get(&(id, this)).is_some_and(|x| x.contains_key(&b)));
                if !bound {
                    return Err(rt_err("Button has not been bound. Call BindButton first."));
                }
                one(Value::Boolean(held))
            })(),
            _ => return None,
        });
    }
    if class == "SkateboardPlatform" && name == "ApplySpecificImpulse" {
        return Some((|| {
            let j = vec3_arg(args, 1)?;
            let mut w = world(lua)?;
            let v = w
                .velocity_requests
                .iter()
                .rev()
                .find(|(r, _)| *r == this)
                .map(|(_, v)| *v)
                .or_else(|| w.velocities.get(&this).copied())
                .unwrap_or(Vec3::ZERO);
            w.velocity_requests.push((this, v + j));
            none()
        })());
    }
    None
}

fn control_state(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let id = world(lua).ok()?.d().id;
    let with = |f: &mut dyn FnMut(&mut State) -> mlua::Result<MultiValue>| {
        STATES.with(|s| f(s.borrow_mut().entry((id, this)).or_default()))
    };
    let nums = |i: usize, d: f64| num_arg(args, i).unwrap_or(d);
    Some(match name {
        "AddBoolField" | "AddCFrameField" | "AddInstanceField" | "AddIntField" | "AddNumberField"
        | "AddUnitVector3Field" | "AddVector2Field" | "AddVector3Field" => (|| {
            let field = name_arg(args, 1)?;
            let (k, default) = match name {
                "AddBoolField" => (Kind::Bool, Value::Boolean(matches!(arg(args, 2), Value::Boolean(true)))),
                "AddNumberField" => (Kind::Number(nums(3, 0.0), nums(4, 1.0)), Value::Number(nums(2, 0.0))),
                "AddIntField" => (Kind::Int(nums(3, 0.0), nums(4, 255.0)), Value::Number(nums(2, 0.0))),
                "AddVector3Field" => (Kind::Vector3(nums(3, 100.0)), arg(args, 2)),
                "AddUnitVector3Field" => (Kind::Unit3, arg(args, 2)),
                "AddVector2Field" => (Kind::Vector2(nums(3, 1.0)), arg(args, 2)),
                "AddCFrameField" => (Kind::CFrame, arg(args, 2)),
                _ => (Kind::Instance, Value::Nil),
            };
            let default = match (&default, k) {
                (Value::Nil, Kind::Vector3(_) | Kind::Unit3) => vec3(lua, Vec3::ZERO)?,
                (Value::Nil, Kind::Vector2(_)) => Value::UserData(lua.create_userdata(LVector2(Vec2::ZERO))?),
                (Value::Nil, Kind::CFrame) => vec3(lua, Vec3::ZERO).and_then(|_| {
                    Ok(Value::UserData(lua.create_userdata(LCFrame(Xform { pos: Vec3::ZERO, rot: glam::Mat3::IDENTITY }))?))
                })?,
                _ => default,
            };
            let v = convert(&field, k, &default)?;
            with(&mut |st| {
                if !st.kinds.contains_key(&field) {
                    st.kinds.insert(field.clone(), k);
                    st.current.insert(field.clone(), v.clone());
                    st.previous.insert(field.clone(), v.clone());
                }
                none()
            })
        })(),
        "SetField" => (|| {
            let field = name_arg(args, 1)?;
            let value = arg(args, 2);
            with(&mut |st| {
                let k = *st.kinds.get(&field).ok_or_else(|| rt_err(format!("ControlState: unknown field '{field}'")))?;
                let v = convert(&field, k, &value)?;
                st.staged.insert(field.clone(), v);
                none()
            })
        })(),
        "UpdateFields" => (|| {
            let t = match arg(args, 1) {
                Value::Table(t) => t,
                Value::Nil => return Err(missing(1)),
                v => return Err(rt_err(format!("Unable to cast {} to Dictionary", v.type_name()))),
            };
            let pairs: Vec<(String, Value)> = t.pairs::<String, Value>().collect::<mlua::Result<_>>()?;
            with(&mut |st| {
                for (field, value) in &pairs {
                    let k = *st.kinds.get(field).ok_or_else(|| rt_err(format!("ControlState: unknown field '{field}'")))?;
                    let v = convert(field, k, value)?;
                    st.staged.insert(field.clone(), v);
                }
                none()
            })
        })(),
        "GetState" => {
            let m = STATES.with(|s| s.borrow().get(&(id, this)).map(|st| pick(st, &arg(args, 1), false))).unwrap_or_default();
            dict(lua, &m).map(Value::Table).and_then(one)
        }
        "GetChangedState" => {
            let (a, b) = STATES
                .with(|s| s.borrow().get(&(id, this)).map(|st| (pick(st, &arg(args, 1), true), pick(st, &arg(args, 2), false))))
                .unwrap_or_default();
            let diff: BTreeMap<String, Val> =
                b.into_iter().filter(|(k, v)| a.get(k) != Some(v)).collect();
            dict(lua, &diff).map(Value::Table).and_then(one)
        }
        "HasField" => Err(rt_err("ControlState.HasField is not enabled")),
        "GetReplicationWeight" => one(Value::Number(64.0)),
        _ => return None,
    })
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    Ok(Some(match (class, key) {
        ("SkateboardController", "Steer" | "Throttle") => Value::Number(0.0),
        (_, "Active") if doc::is_a(class, "ControllerBase") => {
            let w = world(lua)?;
            let d = w.d();
            let parent = d.dom.get_by_ref(r).map(|i| i.parent()).unwrap_or_else(Ref::none);
            let on = d.class_of(parent).is_some_and(|c| c == "ControllerManager")
                && refp(d, parent, "ActiveController") == Some(r)
                && refp(d, parent, "RootPart").is_some_and(|p| in_workspace(d, p));
            Value::Boolean(on)
        }
        ("AirController", "LinearImpulse") => vec3(lua, v3p(world(lua)?.d(), r, key).unwrap_or(Vec3::ZERO))?,
        ("SkateboardPlatform", "Controller" | "ControllingHumanoid") => {
            let id = world(lua)?.d().id;
            let b = BOARDS.with(|m| m.borrow().get(&(id, r)).copied());
            let v = match (key, b) {
                ("Controller", Some((_, Some(c)))) => c,
                ("ControllingHumanoid", Some((h, _))) => h,
                _ => Ref::none(),
            };
            inst_value(lua, v)?
        }
        _ => return Ok(None),
    }))
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if (class, key) == ("AirController", "LinearImpulse") {
        let v = ud::<LVector3>(value).map(|v| v.0).ok_or_else(|| {
            rt_err(format!("Unable to assign property {key}. Vector3 expected, got {}", value.type_name()))
        })?;
        store_prop(lua, r, key, Variant::Vector3(rbx_dom_weak::types::Vector3::new(v.x, v.y, v.z)))?;
        return Ok(true);
    }
    Ok(false)
}

/// Server: humanoids standing on free SkateboardPlatforms take them.
fn skateboards(lua: &Lua) -> mlua::Result<()> {
    let mounts: Vec<(Ref, Ref, bool)> = {
        let w = world(lua)?;
        if w.side != Side::Server {
            return Ok(());
        }
        let d = w.d();
        let boards = BOARD_LIST.with(|m| {
            let mut m = m.borrow_mut();
            let e = m.entry(d.id).or_insert((u64::MAX, Vec::new()));
            if e.0 != d.tree_revision {
                e.0 = d.tree_revision;
                e.1 = d
                    .workspace()
                    .map(|ws| d.dom.descendants_of(ws).filter(|i| i.class == "SkateboardPlatform").map(|i| i.referent()).collect())
                    .unwrap_or_default();
            }
            e.1.clone()
        });
        if boards.is_empty() {
            return Ok(());
        }
        let taken: Vec<Ref> = BOARDS.with(|m| m.borrow().keys().filter(|(i, _)| *i == d.id).map(|(_, r)| *r).collect());
        let mut v = Vec::new();
        for b in boards.into_iter().filter(|b| !taken.contains(b) && d.exists(*b)) {
            let (Some(bx), Some(bs)) = (xf(d, b, "CFrame"), v3p(d, b, "Size")) else { continue };
            let top = bx.pos.y + bs.y * 0.5;
            // A humanoid's root over the board, its feet on it.
            for h in d.dom.descendants().filter(|i| i.class == "Humanoid").map(|i| i.referent()) {
                let model = d.dom.get_by_ref(h).map(|i| i.parent()).unwrap_or_else(Ref::none);
                let Some(root) = d.dom.get_by_ref(model).and_then(|m| m.children().iter().copied().find(|c| d.dom.get_by_ref(*c).is_some_and(|i| i.name == "HumanoidRootPart"))) else { continue };
                let Some(rx) = xf(d, root, "CFrame") else { continue };
                let local = bx.rot.transpose() * (rx.pos - bx.pos);
                let hip = f32p(d, h, "HipHeight", 2.0);
                let rh = v3p(d, root, "Size").map(|s| s.y * 0.5).unwrap_or(1.0);
                let gap = rx.pos.y - rh - hip - top;
                if local.x.abs() <= bs.x * 0.5 && local.z.abs() <= bs.z * 0.5 && gap.abs() < 1.0 {
                    let parent = d.dom.get_by_ref(b).map(|i| i.parent()).unwrap_or_else(Ref::none);
                    let same_model = parent == model && d.class_of(model).is_some_and(|c| doc::is_a(&c, "Model"));
                    v.push((b, h, same_model));
                    break;
                }
            }
        }
        v
    };
    for (b, h, same_model) in mounts {
        let id = world(lua)?.d().id;
        let controller = if same_model { Some(crate::runtime::ext::create_any(lua, "SkateboardController")?) } else { None };
        BOARDS.with(|m| m.borrow_mut().insert((id, b), (h, controller)));
        let was = matches!(world(lua)?.d().prop(h, "PlatformStand"), Some(Variant::Bool(true)));
        store_prop(lua, h, "PlatformStand", Variant::Bool(true))?;
        if !was {
            world(lua)?.fire(h, "PlatformStanding", vec![Value::Boolean(true)]);
        }
        match controller {
            Some(c) => {
                let args = vec![inst_value(lua, h)?, inst_value(lua, c)?];
                let mut w = world(lua)?;
                w.fire(b, "Equipped", args.clone());
                w.fire(b, "equipped", args);
            }
            None => world(lua)?.output.push((
                crate::app::Level::Error,
                "Error mounting skateboard platform. Both Humanoid and SkateboardPlatform must be direct children of a Model".into(),
            )),
        }
    }
    Ok(())
}

/// Commits staged ControlState values (under Workspace) and watches bound
/// controller buttons.
pub fn step(lua: &Lua) -> mlua::Result<()> {
    skateboards(lua)?;
    let (id, live, keys, client) = {
        let w = world(lua)?;
        let d = w.d();
        let live: HashSet<Ref> = STATES.with(|s| {
            s.borrow().keys().filter(|(i, r)| *i == d.id && in_workspace(d, *r)).map(|(_, r)| *r).collect()
        });
        (d.id, live, w.keys_down.clone(), w.side == Side::Client)
    };
    let mut changed = Vec::new();
    STATES.with(|s| {
        let mut s = s.borrow_mut();
        s.retain(|(i, r), _| *i != id || world(lua).is_ok_and(|w| w.d().exists(*r)));
        for r in &live {
            if let Some(st) = s.get_mut(&(id, *r)) {
                st.previous = st.current.clone();
                for (k, v) in std::mem::take(&mut st.staged) {
                    st.current.insert(k, v);
                }
                if st.current != st.previous {
                    changed.push(*r);
                }
            }
        }
    });
    let mut presses = Vec::new();
    BUTTONS.with(|b| {
        for ((i, r), m) in b.borrow_mut().iter_mut() {
            if *i != id {
                continue;
            }
            for (btn, held) in m.iter_mut() {
                let now = client && keys.contains(button_key(*btn));
                if now != *held {
                    *held = now;
                    presses.push((*r, *btn));
                }
            }
        }
    });
    let mut w = world(lua)?;
    for r in changed {
        w.fire(r, "OnStateChanged", vec![]);
    }
    drop(w);
    for (r, b) in presses {
        let item = match LEnumItem::from_value("Button", b) {
            Some(e) => enum_value(lua, e)?,
            None => Value::Nil,
        };
        world(lua)?.fire(r, "ButtonChanged", vec![item]);
    }
    Ok(())
}
