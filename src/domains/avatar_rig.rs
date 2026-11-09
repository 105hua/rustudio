//! HumanoidRigDescription and DigitsRigDescription methods, matched against a
//! running Studio (checks/parity/avatar.md):
//! - joints are the `<Label>` Instance properties, T-pose adjustments the
//!   `<Label>TposeAdjustment` CFrames; labels that aren't joints of the class
//!   are ignored (nil, no error), as Studio does.
//! - GetJointFromName takes a joint name or an R15 part name (Head → Neck,
//!   LeftHand → LeftWrist …); anything else warns "The provided name is not a
//!   valid joint or part name: X" and gives nil.
//! - Finger controls (DigitsRigDescription) are kept per finger (1 thumb …
//!   5 pinky), clamped to [0, 1]. Studio's FingerControl / FingerTip solve
//!   needs the finger's three joints set; without them it warns "is missing
//!   some data for finger N, skipping" (N the 0-based finger) — so does this.
//!   With the joints set, GetFingerTip is the last joint's world position
//!   and SetFingerTip stores the control as the tip's direction from the
//!   first joint (Studio's own curl solve isn't reproduced).

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};

use super::desc::{cast_err, string_arg};
use crate::runtime::*;

pub const JOINTS: &[&str] = &[
    "Root", "Waist", "Spine", "Chest", "Neck", "HeadBase", "LeftClavicle", "LeftShoulder", "LeftElbow",
    "LeftWrist", "RightClavicle", "RightShoulder", "RightElbow", "RightWrist", "LeftHip", "LeftKnee",
    "LeftAnkle", "LeftToeBase", "RightHip", "RightKnee", "RightAnkle", "RightToeBase",
];
pub const R15: &[&str] = &[
    "Root", "Waist", "Neck", "LeftShoulder", "LeftElbow", "LeftWrist", "RightShoulder", "RightElbow",
    "RightWrist", "LeftHip", "LeftKnee", "LeftAnkle", "RightHip", "RightKnee", "RightAnkle",
];
pub const R6: &[&str] = &["Root", "Neck", "LeftShoulder", "RightShoulder", "LeftHip", "RightHip"];
const FINGERS: &[&str] = &["Thumb", "Index", "Middle", "Ring", "Pinky"];

/// R15 part → the joint that moves it.
fn joint_for_part(name: &str) -> Option<&'static str> {
    Some(match name {
        "HumanoidRootPart" | "LowerTorso" => "Root",
        "UpperTorso" => "Waist",
        "Head" => "Neck",
        "LeftUpperArm" => "LeftShoulder",
        "LeftLowerArm" => "LeftElbow",
        "LeftHand" => "LeftWrist",
        "RightUpperArm" => "RightShoulder",
        "RightLowerArm" => "RightElbow",
        "RightHand" => "RightWrist",
        "LeftUpperLeg" => "LeftHip",
        "LeftLowerLeg" => "LeftKnee",
        "LeftFoot" => "LeftAnkle",
        "RightUpperLeg" => "RightHip",
        "RightLowerLeg" => "RightKnee",
        "RightFoot" => "RightAnkle",
        _ => return None,
    })
}

fn digit_joint(label: &str) -> bool {
    FINGERS.iter().any(|f| {
        label.strip_prefix(f).is_some_and(|n| matches!(n, "1" | "2" | "3"))
    })
}

fn label_arg(args: &MultiValue, i: usize) -> mlua::Result<String> {
    match args.get(i) {
        None | Some(Value::Nil) => Err(rt_err(format!("Argument {i} missing or nil"))),
        Some(v) => match opt_ud::<crate::ltypes::LEnumItem>(v) {
            Some(e) if e.enum_type == "RigLabel" => Ok(e.name),
            _ => match v {
                Value::Number(_) | Value::Integer(_) => crate::ltypes::LEnumItem::from_value("RigLabel", num(v).unwrap_or(0.0) as u32)
                    .map(|e| e.name)
                    .ok_or_else(|| rt_err("Unable to cast value to RigLabel")),
                Value::String(s) => crate::ltypes::LEnumItem::from_name("RigLabel", &s.to_string_lossy())
                    .map(|e| e.name)
                    .ok_or_else(|| rt_err("Unable to cast value to RigLabel")),
                _ => Err(cast_err(v, "token")),
            },
        },
    }
}

fn names(lua: &Lua, list: &[&str]) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([Value::Table(lua.create_sequence_from(list.iter().copied())?)]))
}

fn joint_value(lua: &Lua, r: Ref, prop: &str) -> mlua::Result<Value> {
    let v = world(lua)?.d().prop(r, prop);
    match v {
        Some(Variant::Ref(j)) if j.is_some() && world(lua)?.d().dom.get_by_ref(j).is_some() => inst_value(lua, j),
        _ => Ok(Value::Nil),
    }
}

fn inst_arg(args: &MultiValue, i: usize) -> mlua::Result<Ref> {
    match args.get(i) {
        None | Some(Value::Nil) => Ok(Ref::none()),
        Some(v) => opt_inst(v).ok_or_else(|| cast_err(v, "Object")),
    }
}

fn cframe_arg(args: &MultiValue, i: usize) -> mlua::Result<crate::math::Xform> {
    match args.get(i) {
        None | Some(Value::Nil) => Err(rt_err(format!("Argument {i} missing or nil"))),
        Some(v) => opt_ud::<crate::ltypes::LCFrame>(v).map(|c| c.0).ok_or_else(|| cast_err(v, "CoordinateFrame")),
    }
}

fn vec3_of(args: &MultiValue, i: usize) -> mlua::Result<glam::Vec3> {
    match args.get(i) {
        None | Some(Value::Nil) => Err(rt_err(format!("Argument {i} missing or nil"))),
        Some(v) => opt_ud::<crate::ltypes::LVector3>(v).map(|c| c.0).ok_or_else(|| cast_err(v, "Vector3")),
    }
}

fn v3(lua: &Lua, v: glam::Vec3) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([crate::ltypes::LVector3(v).into_value()]))
}

fn warn(lua: &Lua, msg: String) -> mlua::Result<()> {
    world(lua)?.output.push((crate::app::Level::Warn, msg));
    Ok(())
}

/// Per-finger controls, kept for the session (Studio doesn't save them).
#[derive(Default)]
pub struct Fingers(pub std::collections::HashMap<(Ref, usize), glam::Vec3>);

/// The world positions of a finger's three joints, when all are set.
fn finger_joints(lua: &Lua, r: Ref, f: usize) -> mlua::Result<Option<[glam::Vec3; 3]>> {
    let mut out = [glam::Vec3::ZERO; 3];
    for (k, slot) in out.iter_mut().enumerate() {
        let prop = format!("{}{}", FINGERS[f], k + 1);
        let w = world(lua)?;
        let d = w.d();
        let Some(Variant::Ref(j)) = d.prop(r, &prop) else { return Ok(None) };
        if d.dom.get_by_ref(j).is_none() {
            return Ok(None);
        }
        *slot = match crate::attachment::world(d, j, true) {
            Some(x) => x.pos,
            None => return Ok(None),
        };
    }
    Ok(Some(out))
}

fn finger_index(lua: &Lua, args: &MultiValue) -> mlua::Result<Option<usize>> {
    let i = match args.get(1) {
        None | Some(Value::Nil) => return Err(rt_err("Argument 1 missing or nil")),
        Some(v) => num(v).ok_or_else(|| cast_err(v, "int"))? as i64,
    };
    if !(1..=5).contains(&i) {
        warn(lua, "FingerIndex is expected in the range [1,5]".into())?;
        return Ok(None);
    }
    Ok(Some((i - 1) as usize))
}

pub fn call(lua: &Lua, class: &str, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let r = match (class, name) {
        ("HumanoidRigDescription", "GetJointNames") => names(lua, JOINTS),
        ("HumanoidRigDescription", "GetR15JointNames") => names(lua, R15),
        ("HumanoidRigDescription", "GetR6JointNames") => names(lua, R6),
        ("HumanoidRigDescription", "GetR15JointLabels") => (|| {
            let t = lua.create_table()?;
            for (i, n) in R15.iter().enumerate() {
                if let Some(e) = crate::ltypes::LEnumItem::from_name("RigLabel", n) {
                    t.set(i + 1, crate::ltypes::enum_value(lua, e)?)?;
                }
            }
            Ok(MultiValue::from_iter([Value::Table(t)]))
        })(),
        ("HumanoidRigDescription", "GetJoint") => (|| {
            let l = label_arg(args, 1)?;
            if !JOINTS.contains(&l.as_str()) {
                return Ok(MultiValue::from_iter([Value::Nil]));
            }
            Ok(MultiValue::from_iter([joint_value(lua, this, &l)?]))
        })(),
        ("HumanoidRigDescription", "GetJointFromName") => (|| {
            let n = string_arg(args, 1)?;
            let joint = if JOINTS.contains(&n.as_str()) {
                Some(n.clone())
            } else {
                joint_for_part(&n).map(str::to_string)
            };
            match joint {
                Some(j) => Ok(MultiValue::from_iter([joint_value(lua, this, &j)?])),
                None => {
                    warn(lua, format!("The provided name is not a valid joint or part name: {n}"))?;
                    Ok(MultiValue::from_iter([Value::Nil]))
                }
            }
        })(),
        ("HumanoidRigDescription", "SetJoint") | ("DigitsRigDescription", "SetJoint") => (|| {
            let l = label_arg(args, 1)?;
            let j = inst_arg(args, 2)?;
            let ok = if class == "HumanoidRigDescription" { JOINTS.contains(&l.as_str()) } else { digit_joint(&l) };
            if ok {
                store_prop(lua, this, &l, Variant::Ref(j))?;
            }
            Ok(MultiValue::new())
        })(),
        ("HumanoidRigDescription", "SetTposeAdjustment") => (|| {
            let l = label_arg(args, 1)?;
            let cf = cframe_arg(args, 2)?;
            if JOINTS.contains(&l.as_str()) {
                store_prop(lua, this, &format!("{l}TposeAdjustment"), Variant::CFrame(cf.to_cframe()))?;
            }
            Ok(MultiValue::new())
        })(),
        ("DigitsRigDescription", "GetFingerControl") => (|| {
            let Some(f) = finger_index(lua, args)? else { return v3(lua, glam::Vec3::ZERO) };
            let c = lua.app_data_ref::<Fingers>().and_then(|m| m.0.get(&(this, f)).copied()).unwrap_or(glam::Vec3::ZERO);
            v3(lua, c)
        })(),
        ("DigitsRigDescription", "SetFingerControl") => (|| {
            let Some(f) = finger_index(lua, args)? else { return Ok(MultiValue::new()) };
            let c = vec3_of(args, 2)?.clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
            if finger_joints(lua, this, f)?.is_none() {
                warn(lua, format!("DigitsRigDescription is missing some data for finger {f}, skipping"))?;
            }
            if let Some(mut m) = lua.app_data_mut::<Fingers>() {
                m.0.insert((this, f), c);
            }
            Ok(MultiValue::new())
        })(),
        ("DigitsRigDescription", "GetFingerTip") => (|| {
            let Some(f) = finger_index(lua, args)? else { return v3(lua, glam::Vec3::ZERO) };
            match finger_joints(lua, this, f)? {
                Some(j) => v3(lua, j[2]),
                None => v3(lua, glam::Vec3::ZERO),
            }
        })(),
        ("DigitsRigDescription", "SetFingerTip") => (|| {
            let Some(f) = finger_index(lua, args)? else { return Ok(MultiValue::new()) };
            let tip = vec3_of(args, 2)?;
            match finger_joints(lua, this, f)? {
                Some(j) => {
                    let reach = (j[1] - j[0]).length() + (j[2] - j[1]).length();
                    let d = if reach > 0.0 { (tip - j[0]) / reach } else { glam::Vec3::ZERO };
                    let c = (d * 0.5 + glam::Vec3::splat(0.5)).clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
                    if let Some(mut m) = lua.app_data_mut::<Fingers>() {
                        m.0.insert((this, f), c);
                    }
                }
                None => warn(lua, format!("DigitsRigDescription is missing some data for finger {f}, skipping"))?,
            }
            Ok(MultiValue::new())
        })(),
        _ => return None,
    };
    Some(r)
}
