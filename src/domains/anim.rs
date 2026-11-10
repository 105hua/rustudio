//! Engine API domain: anim (Humanoid state API, Animator, AnimationTrack,
//! animation clips, curves, keyframes, IKControl, Motor / Motor6D /
//! JointInstance). See src/domains/mod.rs for the hook contract and
//! checks/parity/anim.md for the Studio measurements behind each member.
//!
//! - anim_lib.luau: curves (FloatCurve / RotationCurve / ValueCurve /
//!   MarkerCurve / EulerRotationCurve / Vector3Curve / CompositeValueCurve),
//!   Keyframe / Pose / KeyframeSequence, the clip providers, the Animator /
//!   AnimationTrack extras, AnimationNodeDefinition, AnimationRigData and the
//!   Humanoid members written in Luau (statuses, GetAppliedDescription,
//!   ReplaceBodyPartR15, BuildRigFromAttachments, GetRelativeVelocityAtFloor).
//! - anim_ik.rs: IKControl solving and its getters.
//! - anim_types.rs: RotationCurveKey / ValueCurveKey.
//! - here: dispatch, Humanoid Set/GetStateEnabled (+ StateEnabledChanged),
//!   Motor stepping, JointInstance.Active, the Animator's read-only values.
//!
//! What Studio can't do here either (behaviour copied from Studio 0.746):
//! the Humanoid status calls answer false and fire nothing;
//! CompositeValueCurve is flag-disabled (empty); ValueCurve is inert for
//! scripts; GetClipEvaluatorAsync / RegisterEvaluationParallelCallback /
//! AnimationTrack target calls raise Studio's "not enabled / not available"
//! errors; RTAnimationTracker's TrackerError / TrackerPrompt never fire
//! (no camera / face tracker); Motor6D.ChildName / ParentName are
//! NotScriptable ("not a valid member", as Studio says).
#![allow(unused_imports, unused_variables)]

use std::collections::HashMap;

use glam::{Mat3, Vec3};
use mlua::{Function, Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};

use crate::runtime::*;

#[path = "anim_ik.rs"]
mod anim_ik;
#[path = "anim_types.rs"]
pub mod anim_types;
#[path = "anim_tween.rs"]
pub mod anim_tween;
#[path = "anim_tracks.rs"]
pub mod anim_tracks;

/// Native steps queued among the frame's RunService events
/// (`Runtime::fire`), run at their place in the signal queue: Studio steps
/// TweenService after the client's PreRender handlers (the server's
/// Heartbeat ones) and the Animator after the PreAnimation handlers.
pub const HOOK_TWEEN: &str = "\0tween";
pub const HOOK_ANIMATE: &str = "\0animate";

/// The frame hook that follows a RunService event on this side, if any.
pub fn hook_after(side: Side, event: &str) -> Option<&'static str> {
    match event {
        "PreAnimation" => Some(HOOK_ANIMATE),
        "PreRender" if side == Side::Client => Some(HOOK_TWEEN),
        "Heartbeat" if side != Side::Client => Some(HOOK_TWEEN),
        _ => None,
    }
}

/// Runs a queued frame hook (from the signal flush / dispatch).
pub fn run_hook(lua: &Lua, name: &str, args: &[Value]) {
    let dt = match args.first() {
        Some(Value::Number(n)) => *n,
        _ => 0.0,
    };
    let r = match name {
        HOOK_TWEEN => {
            let _s = crate::bench::scope("tween_step");
            anim_tween::step(lua, dt)
        }
        HOOK_ANIMATE => {
            let _s = crate::bench::scope("animate_step");
            animate(lua, dt)
        }
        _ => Ok(()),
    };
    if let Err(e) = r {
        if let Ok(mut w) = world(lua) {
            w.output.push((crate::app::Level::Error, format!("{name}: {e}")));
        }
    }
}

/// The Animator's step: tracks (anim_tracks.rs), then IKControls.
fn animate(lua: &Lua, dt: f64) -> mlua::Result<()> {
    anim_tracks::step(lua, dt)?;
    after(lua, dt as f32)
}

pub const METHODS: &[(&str, &str)] = &[
    // Humanoid
    ("AddStatus", "Humanoid"),
    ("RemoveStatus", "Humanoid"),
    ("HasStatus", "Humanoid"),
    ("GetStatuses", "Humanoid"),
    ("AddCustomStatus", "Humanoid"),
    ("RemoveCustomStatus", "Humanoid"),
    ("HasCustomStatus", "Humanoid"),
    ("GetAppliedDescription", "Humanoid"),
    ("GetRelativeVelocityAtFloor", "Humanoid"),
    ("ReplaceBodyPartR15", "Humanoid"),
    ("BuildRigFromAttachments", "Humanoid"),
    ("SetStateEnabled", "Humanoid"),
    ("GetStateEnabled", "Humanoid"),
    // Animator / AnimationController / AnimationTrack
    ("GetTrackByAnimationId", "Animator"),
    ("StepAnimations", "Animator"),
    ("ApplyJointVelocities", "Animator"),
    ("RegisterEvaluationParallelCallback", "Animator"),
    ("GetPlayingAnimationTracks", "AnimationController"),
    ("GetParameter", "AnimationTrack"),
    ("SetParameter", "AnimationTrack"),
    ("GetParameterDefaults", "AnimationTrack"),
    ("GetTargetNames", "AnimationTrack"),
    ("GetTargetInstance", "AnimationTrack"),
    ("SetTargetInstance", "AnimationTrack"),
    // Providers
    ("RegisterAnimationClip", "AnimationClipProvider"),
    ("RegisterActiveAnimationClip", "AnimationClipProvider"),
    ("GetClipEvaluatorAsync", "AnimationClipProvider"),
    // AnimationNodeDefinition / AnimationRigData
    ("AddInputPin", "AnimationNodeDefinition"),
    ("RemoveInputPin", "AnimationNodeDefinition"),
    ("GetOrderedInputPinNames", "AnimationNodeDefinition"),
    ("SetOrderedInputPinNames", "AnimationNodeDefinition"),
    ("LoadFromModel", "AnimationRigData"),
    // Keyframes
    ("AddPose", "Keyframe"),
    ("RemovePose", "Keyframe"),
    ("GetPoses", "Keyframe"),
    ("AddMarker", "Keyframe"),
    ("RemoveMarker", "Keyframe"),
    ("GetMarkers", "Keyframe"),
    ("AddKeyframe", "KeyframeSequence"),
    ("RemoveKeyframe", "KeyframeSequence"),
    ("GetKeyframes", "KeyframeSequence"),
    ("AddSubPose", "Pose"),
    ("RemoveSubPose", "Pose"),
    ("GetSubPoses", "Pose"),
    // Curves
    ("GetKeys", "FloatCurve"),
    ("GetKeyAtIndex", "FloatCurve"),
    ("GetKeyIndicesAtTime", "FloatCurve"),
    ("GetValueAtTime", "FloatCurve"),
    ("InsertKey", "FloatCurve"),
    ("RemoveKeyAtIndex", "FloatCurve"),
    ("SetKeys", "FloatCurve"),
    ("GetKeys", "RotationCurve"),
    ("GetKeyAtIndex", "RotationCurve"),
    ("GetKeyIndicesAtTime", "RotationCurve"),
    ("GetValueAtTime", "RotationCurve"),
    ("InsertKey", "RotationCurve"),
    ("RemoveKeyAtIndex", "RotationCurve"),
    ("SetKeys", "RotationCurve"),
    ("GetKeys", "ValueCurve"),
    ("GetKeyAtIndex", "ValueCurve"),
    ("GetKeyIndicesAtTime", "ValueCurve"),
    ("GetValueAtTime", "ValueCurve"),
    ("InsertKey", "ValueCurve"),
    ("InsertKeyValue", "ValueCurve"),
    ("RemoveKeyAtIndex", "ValueCurve"),
    ("SetKeys", "ValueCurve"),
    ("GetMarkers", "MarkerCurve"),
    ("GetMarkerAtIndex", "MarkerCurve"),
    ("InsertMarkerAtTime", "MarkerCurve"),
    ("RemoveMarkerAtIndex", "MarkerCurve"),
    ("X", "EulerRotationCurve"),
    ("Y", "EulerRotationCurve"),
    ("Z", "EulerRotationCurve"),
    ("GetAnglesAtTime", "EulerRotationCurve"),
    ("GetRotationAtTime", "EulerRotationCurve"),
    ("X", "Vector3Curve"),
    ("Y", "Vector3Curve"),
    ("Z", "Vector3Curve"),
    ("GetValueAtTime", "Vector3Curve"),
    ("GetComponentCurves", "CompositeValueCurve"),
    ("GetValueAtTime", "CompositeValueCurve"),
    // IKControl / Motor
    ("GetChainCount", "IKControl"),
    ("GetChainLength", "IKControl"),
    ("GetNodeLocalCFrame", "IKControl"),
    ("GetNodeWorldCFrame", "IKControl"),
    ("GetRawFinalTarget", "IKControl"),
    ("GetSmoothedFinalTarget", "IKControl"),
    ("SetDesiredAngle", "Motor"),
];

/// Methods that yield, written in Luau and set in `__rs_lua_methods`
/// (anim_lib.luau's YIELDING table) so they run on the calling thread.
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("PlayEmote", "Humanoid"),
    ("PlayEmoteAsync", "Humanoid"),
    ("ApplyAvatarRulesAsync", "Humanoid"),
    ("GetAnimationClipAsync", "AnimationClipProvider"),
    ("GetAnimations", "AnimationClipProvider"),
    ("GetAnimationsAsync", "AnimationClipProvider"),
    ("GetAnimations", "KeyframeSequenceProvider"),
    ("GetAnimationsAsync", "KeyframeSequenceProvider"),
];

/// Events this domain adds. The status events and the tracker events
/// exist in Studio and never fire there (dead status API; no tracker
/// device); InputPinsChanged fires on every pin call.
pub const EVENTS: &[(&str, &str)] = &[
    ("StatusAdded", "Humanoid"),
    ("StatusRemoved", "Humanoid"),
    ("CustomStatusAdded", "Humanoid"),
    ("CustomStatusRemoved", "Humanoid"),
    ("InputPinsChanged", "AnimationNodeDefinition"),
    ("TrackerError", "RTAnimationTracker"),
    ("TrackerPrompt", "RTAnimationTracker"),
];

/// Per-Lua-state animation state.
#[derive(Default)]
struct AnimState {
    frame: u64,
    dirty: bool,
    scanned: bool,
    motors: Vec<Ref>,
    iks: Vec<Ref>,
    ik: HashMap<Ref, anim_ik::IkState>,
    /// Motors and IKControls, kept from the document's change journal.
    index: Option<crate::doc::ClassIndex>,
}

fn state(lua: &Lua) -> mlua::AppDataRefMut<'_, AnimState> {
    if lua.app_data_ref::<AnimState>().is_none() {
        lua.set_app_data(AnimState::default());
    }
    lua.app_data_mut::<AnimState>().expect("anim state")
}

fn mine(class: &str, name: &str) -> bool {
    METHODS
        .iter()
        .any(|(m, b)| *m == name && crate::doc::is_a(class, b))
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let class = world(lua).ok()?.d().class_of(this)?;
    if !mine(&class, name) {
        return None;
    }
    Some(call_inner(lua, name, this, &class, args))
}

fn arg_at(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

fn call_inner(lua: &Lua, name: &str, this: Ref, class: &str, args: &MultiValue) -> mlua::Result<MultiValue> {
    match name {
        "SetStateEnabled" => {
            let e = opt_ud::<crate::ltypes::LEnumItem>(&arg_at(args, 1))
                .filter(|e| e.enum_type == "HumanoidStateType")
                .ok_or_else(|| rt_err("SetStateEnabled: HumanoidStateType expected"))?;
            let on = !matches!(arg_at(args, 2), Value::Boolean(false) | Value::Nil);
            {
                let mut w = world(lua)?;
                let set = w.states_disabled.entry(this).or_default();
                if on {
                    set.remove(&*e.name);
                } else {
                    set.insert(e.name.to_string());
                }
                w.requests.state_enabled.push((this, e.name.to_string(), on));
            }
            // Studio fires StateEnabledChanged on every call, changed or not.
            let item = crate::ltypes::enum_value(lua, e)?;
            world(lua)?.fire(this, "StateEnabledChanged", vec![item, Value::Boolean(on)]);
            Ok(MultiValue::new())
        }
        "GetStateEnabled" => {
            let off = opt_ud::<crate::ltypes::LEnumItem>(&arg_at(args, 1)).is_some_and(|e| {
                world(lua)
                    .ok()
                    .and_then(|w| w.states_disabled.get(&this).map(|s| s.contains(&*e.name)))
                    .unwrap_or(false)
            });
            Ok(MultiValue::from_iter([Value::Boolean(!off)]))
        }
        "SetDesiredAngle" => {
            let v = match arg_at(args, 1) {
                Value::Number(n) => n as f32,
                Value::Integer(n) => n as f32,
                ref v => {
                    return Err(rt_err(format!(
                        "Unable to cast {} to float",
                        anim_types::cast_name(v)
                    )))
                }
            };
            store_prop(lua, this, "DesiredAngle", Variant::Float32(v))?;
            state(lua).dirty = true;
            Ok(MultiValue::new())
        }
        "GetChainCount" | "GetChainLength" | "GetNodeLocalCFrame" | "GetNodeWorldCFrame"
        | "GetRawFinalTarget" | "GetSmoothedFinalTarget" => {
            let index = match arg_at(args, 1) {
                Value::Number(n) => n as i64,
                Value::Integer(n) => n as i64,
                Value::Nil if !name.starts_with("GetNode") => 0,
                ref v => {
                    return Err(rt_err(format!(
                        "Unable to cast {} to int",
                        anim_types::cast_name(v)
                    )))
                }
            };
            let st = state(lua).ik.get(&this).cloned();
            let w = world(lua)?;
            let v = anim_ik::getter(lua, w.d(), this, st.as_ref(), name, index)?;
            Ok(MultiValue::from_iter([v]))
        }
        _ => {
            let f: Function = lua.named_registry_value("rs_anim_call")?;
            let mut a = MultiValue::new();
            a.push_back(Value::String(lua.create_string(name)?));
            a.push_back(inst_value(lua, this)?);
            a.extend(args.iter().skip(1).cloned());
            f.call(a)
        }
    }
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    match (class, key) {
        ("Animator", "EvaluationThrottled") => return Ok(Some(Value::Boolean(false))),
        ("Animator", "RootMotionWeight") => return Ok(Some(Value::Number(0.0))),
        ("Animator", "RootMotion") => {
            return Ok(Some(mlua::IntoLua::into_lua(
                crate::ltypes::LCFrame(anim_ik::ident()),
                lua,
            )?))
        }
        // Studio derives an AnimationConstraint's parts and frames from its
        // attachments.
        ("AnimationConstraint", "Part0" | "Part1" | "C0" | "C1") => {
            let derived = {
                let w = world(lua)?;
                let d = w.d();
                let has_att = matches!(d.prop(r, "Attachment0"), Some(Variant::Ref(a)) if a.is_some());
                if has_att { crate::rig::joint_frames(d, r) } else { None }
            };
            let Some((p0, p1, c0, c1)) = derived else { return Ok(None) };
            return Ok(Some(match key {
                "Part0" => inst_value(lua, p0)?,
                "Part1" => inst_value(lua, p1)?,
                "C0" => mlua::IntoLua::into_lua(crate::ltypes::LCFrame(c0), lua)?,
                _ => mlua::IntoLua::into_lua(crate::ltypes::LCFrame(c1), lua)?,
            }));
        }
        ("Motor6D", "ChildName" | "ParentName") => {
            let full = world(lua)?.full_name(r);
            return Err(rt_err(format!("{key} is not a valid member of Motor6D \"{full}\"")));
        }
        (_, "Active") if crate::doc::is_a(class, "JointInstance") => {
            let w = world(lua)?;
            let d = w.d();
            let part = |k: &str| match d.prop(r, k) {
                Some(Variant::Ref(p)) if p.is_some() && d.exists(p) => Some(p),
                _ => None,
            };
            let on = !matches!(d.prop(r, "Enabled"), Some(Variant::Bool(false)))
                && part("Part0").is_some()
                && part("Part1").is_some()
                && d.workspace().is_some_and(|ws| d.is_ancestor(ws, r));
            return Ok(Some(Value::Boolean(on)));
        }
        ("FloatCurve" | "MarkerCurve" | "RotationCurve", "Length")
        | ("ValueCurve", "Length" | "ValueType")
        | ("KeyframeSequence" | "CurveAnimation" | "AnimationClip" | "AnimationGraphDefinition", "Length")
        | ("RTAnimationTracker", _)
        | ("AnimationStreamTrack", _) => {}
        _ => return Ok(None),
    }
    let f: Function = lua.named_registry_value("rs_anim_props")?;
    let (handled, v): (bool, Value) = f.call((inst_value(lua, r)?, class, key))?;
    Ok(handled.then_some(v))
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    match class {
        "IKControl" | "Motor" => {
            state(lua).dirty = true;
            Ok(false)
        }
        "RTAnimationTracker" | "AnimationStreamTrack" => {
            let f: Function = lua.named_registry_value("rs_anim_writes")?;
            f.call((inst_value(lua, r)?, class, key, value.clone()))
        }
        _ => Ok(false),
    }
}

pub fn write_acts(class: &str, key: &str) -> bool {
    matches!(
        (class, key),
        ("Motor", "DesiredAngle" | "MaxVelocity")
            | ("IKControl", _)
            | ("Motor6D", "Transform")
    )
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    anim_types::install(lua)?;
    anim_tween::install(lua)?;
    anim_tracks::install(lua)?;
    let create = lua.create_function(|lua, class: String| {
        let r = super::super::ext::create_any(lua, &class)?;
        inst_value(lua, r)
    })?;
    let fire = lua.create_function(|lua, args: MultiValue| {
        let mut it = args.into_iter();
        let target = it
            .next()
            .and_then(|v| opt_inst(&v))
            .ok_or_else(|| rt_err("fire: Instance expected"))?;
        let name = match it.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(rt_err("fire: event name expected")),
        };
        world(lua)?.fire(target, &name, it.collect());
        Ok(())
    })?;
    let setbin = lua.create_function(|lua, (inst, key, data): (Value, String, mlua::LuaString)| {
        let r = opt_inst(&inst).ok_or_else(|| rt_err("setbin: Instance expected"))?;
        let bytes: Vec<u8> = data.as_bytes().to_vec();
        store_prop(lua, r, &key, Variant::BinaryString(bytes.into()))
    })?;
    // An animation asset already downloaded (__rs_asset_ready said true),
    // decoded into a Model: the Animator loads clips itself, on either side,
    // not through InsertService (server-only, and it yields).
    lua.globals().set(
        "__rs_anim_asset",
        lua.create_function(|lua, id: f64| {
            let crate::assets::Fetch::Ready(bytes) = crate::assets::public_prefetch(id as u64) else {
                return Ok(Value::Nil);
            };
            let Ok(mut tmp) = crate::doc::Document::decode(&bytes) else {
                return Ok(Value::Nil);
            };
            let r = super::super::ext::create_any(lua, "Model")?;
            {
                let mut w = world(lua)?;
                for child in tmp.root().children().to_vec() {
                    tmp.transfer(child, &mut w.dm().dom, r);
                }
            }
            inst_value(lua, r)
        })?,
    )?;
    // StarterPlayer.AvatarJointUpgrade (rig.rs) and the upgraded joint
    // builder, for BuildRigFromAttachments / ReplaceBodyPartR15.
    lua.globals().set(
        "__rs_anim_joint_upgrade",
        lua.create_function(|lua, ()| Ok(crate::rig::joint_upgrade(world(lua)?.d())))?,
    )?;
    lua.globals().set(
        "__rs_anim_joint_spec",
        lua.create_function(|lua, name: String| {
            let Some(sp) = crate::rig::upgrade_spec(&name) else { return Ok(Value::Nil) };
            let t = lua.create_table()?;
            t.set("UpperAngle", sp.upper)?;
            t.set("TwistLowerAngle", sp.twist.0)?;
            t.set("TwistUpperAngle", sp.twist.1)?;
            t.set("MaxFrictionTorque", sp.friction)?;
            if let Some((r0, r1)) = sp.rotation {
                t.set("Rotation0", lua.create_sequence_from(r0)?)?;
                t.set("Rotation1", lua.create_sequence_from(r1)?)?;
            }
            Ok(Value::Table(t))
        })?,
    )?;
    let (call, props, writes): (Function, Function, Function) = lua
        .load(include_str!("anim_lib.luau"))
        .set_name("=rustudio.anim")
        .call((create, fire, setbin))?;
    lua.set_named_registry_value("rs_anim_call", call)?;
    lua.set_named_registry_value("rs_anim_props", props)?;
    lua.set_named_registry_value("rs_anim_writes", writes)?;
    Ok(())
}

/// Finds the Motors and IKControls (every 30 frames, or after a script
/// touched one). Only what changed in the DataModel since the last look is
/// visited (doc::ClassIndex), so this is cheap even when a script writes a
/// Motor every frame.
fn rescan(lua: &Lua) -> mlua::Result<()> {
    let mut index = state(lua)
        .index
        .take()
        .unwrap_or_else(|| crate::doc::ClassIndex::new(crate::doc::Reader::Anim));
    let (motors, iks) = {
        let mut w = world(lua)?;
        let d = w.dm();
        index.update(d, |c| c == "Motor" || c == "IKControl");
        let mut motors = Vec::new();
        let mut iks = Vec::new();
        for r in index.list() {
            match d.dom.get_by_ref(*r).map(|i| i.class.as_str()) {
                Some("Motor") => motors.push(*r),
                Some("IKControl") => iks.push(*r),
                _ => {}
            }
        }
        let iks = anim_ik::by_priority(d, &iks);
        (motors, iks)
    };
    let mut s = state(lua);
    s.index = Some(index);
    s.motors = motors;
    s.ik.retain(|k, _| iks.contains(k));
    s.iks = iks;
    s.dirty = false;
    s.scanned = true;
    Ok(())
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    let need = {
        let mut s = state(lua);
        s.frame += 1;
        !s.scanned || s.dirty || s.frame % 30 == 0
    };
    if need {
        rescan(lua)?;
    }
    step_motors(lua, dt as f32)
}

/// Motor: CurrentAngle moves to DesiredAngle at MaxVelocity per 1/60 s
/// (Studio 0.746: 0.025 per 1/240 s step at MaxVelocity 0.1), and Part1
/// follows Part0 · C0 · Rz(CurrentAngle) · C1⁻¹. Motor6D doesn't step.
fn step_motors(lua: &Lua, dt: f32) -> mlua::Result<()> {
    let motors = state(lua).motors.clone();
    if motors.is_empty() {
        return Ok(());
    }
    let mut writes: Vec<(Ref, f32, Option<(Ref, crate::math::Xform)>)> = Vec::new();
    {
        let w = world(lua)?;
        if w.side != Side::Server {
            return Ok(());
        }
        let d = w.d();
        let f = |r: Ref, k: &str| crate::scene::f32_of(d.prop(r, k)).unwrap_or(0.0);
        for m in motors {
            if !d.exists(m) {
                continue;
            }
            let (cur, want, vel) = (f(m, "CurrentAngle"), f(m, "DesiredAngle"), f(m, "MaxVelocity").abs());
            if (want - cur).abs() < 1e-7 {
                continue;
            }
            let stepv = vel * 60.0 * dt;
            let new = cur + (want - cur).clamp(-stepv, stepv);
            let part = |k: &str| match d.prop(m, k) {
                Some(Variant::Ref(p)) if p.is_some() && d.exists(p) => Some(p),
                _ => None,
            };
            let place = match (part("Part0"), part("Part1")) {
                (Some(p0), Some(p1)) if !matches!(d.prop(p1, "Anchored"), Some(Variant::Bool(true))) => {
                    let cf = |r: Ref, k: &str| match d.prop(r, k) {
                        Some(Variant::CFrame(c)) => crate::math::Xform::from_cframe(&c),
                        _ => anim_ik::ident(),
                    };
                    let rz = crate::math::Xform { pos: Vec3::ZERO, rot: Mat3::from_rotation_z(new) };
                    let xf = anim_ik::mul(
                        &anim_ik::mul(&anim_ik::mul(&cf(p0, "CFrame"), &cf(m, "C0")), &rz),
                        &cf(m, "C1").inverse(),
                    );
                    Some((p1, xf))
                }
                _ => None,
            };
            writes.push((m, new, place));
        }
    }
    for (m, a, place) in writes {
        store_prop(lua, m, "CurrentAngle", Variant::Float32(a))?;
        if let Some((p, xf)) = place {
            store_prop(lua, p, "CFrame", anim_ik::to_variant(&xf))?;
        }
    }
    Ok(())
}

/// After the Animator's tracks: every IKControl, lowest Priority first.
fn after(lua: &Lua, dt: f32) -> mlua::Result<()> {
    let iks = state(lua).iks.clone();
    for ik in iks {
        let mut st = state(lua).ik.get(&ik).cloned().unwrap_or_default();
        let out = {
            let w = world(lua)?;
            if !w.d().exists(ik) {
                continue;
            }
            anim_ik::solve(w.d(), ik, &mut st, dt)
        };
        state(lua).ik.insert(ik, st);
        for (m, t) in out.into_iter().flatten() {
            store_prop(lua, m, "Transform", anim_ik::to_variant(&t))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::doc::Document;
    use crate::runtime::{RunMode, Runtime, World};
    use rbx_dom_weak::InstanceBuilder;

    /// Runs a server Script for up to `frames` frames (PreAnimation fired
    /// each one, as Play does) and checks it printed `anim-pass`.
    fn run(source: &str, frames: usize) {
        run_with(source, frames, |_| {});
    }

    fn run_with(source: &str, frames: usize, setup: impl FnOnce(&mut Document)) {
        let mut d = Document::new_place();
        setup(&mut d);
        let parent = d.find_child(d.dom.root_ref(), "ServerScriptService").unwrap();
        let s = d.dom.insert(
            parent,
            InstanceBuilder::new("Script")
                .with_name("AnimDomain")
                .with_property("Source", format!("game:GetService('RunService')\n{source}")),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        let rs = {
            let w = rt.world_mut().unwrap();
            w.d().find_child(w.d().dom.root_ref(), "RunService").unwrap()
        };
        for _ in 0..frames {
            rt.fire(rs, "PreAnimation", vec![mlua::Value::Number(1.0 / 60.0)]);
            rt.step(1.0 / 60.0);
        }
        let output = rt.leave().output;
        assert!(output.iter().any(|(_, s)| s.contains("anim-pass")), "{output:?}");
    }

    #[test]
    fn float_curve_keys_as_studio() {
        run(r#"
local function eq(a, b) assert(a == b, tostring(a) .. " ~= " .. tostring(b)) end
local function near(a, b) assert(math.abs(a - b) < 1e-4, tostring(a) .. " ~= " .. tostring(b)) end
local c = Instance.new("FloatCurve")
eq(c.Length, 0); eq(c:GetValueAtTime(1), nil)
local r = c:InsertKey(FloatCurveKey.new(1, 10, Enum.KeyInterpolationMode.Linear)); eq(r[1], true); eq(r[2], 0)
r = c:InsertKey(FloatCurveKey.new(0, 0, Enum.KeyInterpolationMode.Linear)); eq(r[1], true); eq(r[2], 0)
r = c:InsertKey(FloatCurveKey.new(1, 20, Enum.KeyInterpolationMode.Linear)); eq(r[1], false); eq(r[2], 1)
eq(c.Length, 2); near(c:GetValueAtTime(0.5), 10); near(c:GetValueAtTime(-1), 0); near(c:GetValueAtTime(5), 20)
local i = c:GetKeyIndicesAtTime(9); eq(i[1], 1); eq(i[2], 2)
eq(select(2, pcall(c.GetKeyAtIndex, c, 0)), "FloatCurve index is out of range")
eq(c:RemoveKeyAtIndex(1, 10), 2); eq(c:RemoveKeyAtIndex(1), 0)
eq(select(2, pcall(c.RemoveKeyAtIndex, c, 0)), "Invalid argument provided: startingIndex.")
eq(c:SetKeys({FloatCurveKey.new(2, 1), FloatCurveKey.new(1, 0), FloatCurveKey.new(2, 5)}), 2)
near(c:GetValueAtTime(1.5), 0.5)
local k = FloatCurveKey.new(0, 0, Enum.KeyInterpolationMode.Cubic); k.RightTangent = 1
local k2 = FloatCurveKey.new(1, 0, Enum.KeyInterpolationMode.Cubic); k2.LeftTangent = -1
c:SetKeys({k, k2}); near(c:GetValueAtTime(0.5), 0.25); near(c:GetValueAtTime(0.25), 0.1875)
eq(c:GetKeys()[1].RightTangent, 1); eq(c:GetKeys()[1].LeftTangent, nil)
c:SetKeys({FloatCurveKey.new(1,0), FloatCurveKey.new(2,0), FloatCurveKey.new(3,0)})
for _, case in {{0,1}, {1.5,1}, {2,2}, {3,2}, {4,2}} do eq(c:GetKeyIndicesAtTime(case[1])[1], case[2]) end
eq(select(2, pcall(c.SetKeys, c, {1, 2})), "Variant cast failed")
local m = Instance.new("MarkerCurve")
m:InsertMarkerAtTime(1, "A"); m:InsertMarkerAtTime(0.5, "B"); r = m:InsertMarkerAtTime(0.5, "C")
eq(r[1], false); eq(m.Length, 2); eq(m:GetMarkers()[1].Value, "C"); eq(m:GetMarkerAtIndex(2).Time, 1)
local rc = Instance.new("RotationCurve")
rc:InsertKey(RotationCurveKey.new(0, CFrame.new(), Enum.KeyInterpolationMode.Linear))
rc:InsertKey(RotationCurveKey.new(1, CFrame.Angles(0, math.pi/2, 0), Enum.KeyInterpolationMode.Linear))
near(rc:GetValueAtTime(0.5).LookVector.X, -math.sin(math.pi/4))
eq(select(2, pcall(rc.SetKeys, rc, {RotationCurveKey.new(0, CFrame.new())})), "Variant cast failed")
eq(typeof(rc:GetKeys()[1]), "RotationCurveKey"); eq(rc:GetKeys()[1].LeftTangent, 0)
local e = Instance.new("EulerRotationCurve")
local x = e:X(); eq(x.Name, "X"); eq(x.Parent, e)
x:InsertKey(FloatCurveKey.new(0, 0, Enum.KeyInterpolationMode.Linear)); x:InsertKey(FloatCurveKey.new(1, 1, Enum.KeyInterpolationMode.Linear))
local a = e:GetAnglesAtTime(0.5); near(a[1], 0.5); eq(a[2], nil)
local v = Instance.new("ValueCurve"); eq(v.ValueType, "nil"); eq(#v:InsertKeyValue(0, 1), 0)
local cv = Instance.new("CompositeValueCurve"); eq(#cv:GetComponentCurves(), 0)
print("anim-pass")
"#, 1);
    }

    #[test]
    fn curve_animation_from_script_keys_plays() {
        run(r#"
local model = Instance.new("Model", workspace)
local animator = Instance.new("Animator", Instance.new("AnimationController", model))
local root = Instance.new("Part", model); root.Name = "Root"
local limb = Instance.new("Part", model); limb.Name = "Limb"
local motor = Instance.new("Motor6D", root); motor.Part0 = root; motor.Part1 = limb
local clip = Instance.new("CurveAnimation"); clip.Loop = true
local f = Instance.new("Folder", clip); f.Name = "Root"
local g = Instance.new("Folder", f); g.Name = "Limb"
local pos = Instance.new("Vector3Curve", g); pos.Name = "Position"
local x = pos:X()
x:InsertKey(FloatCurveKey.new(0, 0, Enum.KeyInterpolationMode.Linear))
x:InsertKey(FloatCurveKey.new(1, 4, Enum.KeyInterpolationMode.Linear))
local rot = Instance.new("RotationCurve", g); rot.Name = "Rotation"
rot:InsertKey(RotationCurveKey.new(0, CFrame.new(), Enum.KeyInterpolationMode.Linear))
rot:InsertKey(RotationCurveKey.new(1, CFrame.Angles(0, 0, 1), Enum.KeyInterpolationMode.Linear))
assert(math.abs(clip.Length - 1) < 1e-4, clip.Length)
local a = Instance.new("Animation")
a.AnimationId = game:GetService("AnimationClipProvider"):RegisterAnimationClip(clip)
local t = animator:LoadAnimation(a); t:Play(0, 1, 0); t.TimePosition = 0.25
task.wait(0.05)
assert(math.abs(motor.Transform.X - 1) < 0.02, motor.Transform.X)
assert(math.abs(motor.Transform.RightVector.Y - math.sin(0.25)) < 0.02, motor.Transform.RightVector.Y)
assert(animator:GetTrackByAnimationId(a.AnimationId) == t)
local kfs = Instance.new("KeyframeSequence"); local kf = Instance.new("Keyframe"); kfs:AddKeyframe(kf); kf.Time = 2
assert(kfs.Length == 2 and #kfs:GetKeyframes() == 1)
local p = Instance.new("Pose"); kf:AddPose(p); assert(#kf:GetPoses() == 1)
kf:RemovePose(p); assert(p.Parent == nil)
print("anim-pass")
"#, 30);
    }

    #[test]
    fn humanoid_state_api_and_statuses() {
        run(r#"
local h = Instance.new("Humanoid")
local ev = {}
h.StateEnabledChanged:Connect(function(s, on) table.insert(ev, s.Name .. tostring(on)) end)
h.StatusAdded:Connect(function() table.insert(ev, "status") end)
assert(h:AddStatus(Enum.Status.Poison) == false and h:HasStatus(Enum.Status.Poison) == false)
assert(h:AddCustomStatus("x") == false and #h:GetStatuses() == 0)
h:SetStateEnabled(Enum.HumanoidStateType.Climbing, false)
h:SetStateEnabled(Enum.HumanoidStateType.Climbing, false)
assert(h:GetStateEnabled(Enum.HumanoidStateType.Climbing) == false)
task.wait()
assert(table.concat(ev, ",") == "Climbingfalse,Climbingfalse", table.concat(ev, ","))
local d = h:GetAppliedDescription()
assert(d:IsA("HumanoidDescription") and d.Parent == nil and h:FindFirstChildOfClass("HumanoidDescription"))
assert(d ~= h:GetAppliedDescription())
local nd = Instance.new("AnimationNodeDefinition"); local pins = 0
nd.InputPinsChanged:Connect(function() pins += 1 end)
nd:AddInputPin("A"); nd:AddInputPin("B"); nd:AddInputPin("A"); nd:RemoveInputPin("Z")
task.wait()
assert(pins == 4 and table.concat(nd:GetOrderedInputPinNames(), "") == "ABA")
assert(select(2, pcall(h.PlayEmote, h, "wave")) == "No emote exists with name wave")
assert(select(2, pcall(h.ApplyAvatarRulesAsync, h, nil)) == "Argument 1 missing or nil")
assert(select(2, pcall(h.ApplyAvatarRulesAsync, h, Instance.new("AvatarRules"))) == "AvatarSettings service not found")
local motor = Instance.new("Motor"); motor:SetDesiredAngle(1); assert(motor.DesiredAngle == 1)
assert(motor.Active == false)
print("anim-pass")
"#, 5);
    }

    /// BuildRigFromAttachments with the joint upgrade on (Studio's default)
    /// makes AnimationConstraints + ball sockets that an Animator poses and
    /// whose parts / frames read from the attachments; Disabled keeps Motor6Ds.
    #[test]
    fn upgraded_joints_build_and_animate() {
        run(r#"
local function rig()
    local m = Instance.new("Model", workspace)
    local function part(name, y)
        local p = Instance.new("Part", m); p.Name = name; p.CFrame = CFrame.new(0, y, 0); return p
    end
    local hrp, torso, arm = part("HumanoidRootPart", 3), part("UpperTorso", 3), part("RightUpperArm", 3)
    local function att(p, name, pos) local a = Instance.new("Attachment", p); a.Name = name; a.Position = pos; return a end
    att(hrp, "RootRigAttachment", Vector3.new()); att(torso, "RootRigAttachment", Vector3.new())
    att(torso, "RightShoulderRigAttachment", Vector3.new(1, 0.5, 0)); att(arm, "RightShoulderRigAttachment", Vector3.new(-0.5, 0.5, 0))
    local h = Instance.new("Humanoid", m)
    Instance.new("Animator", h)
    return m, h
end
local m, h = rig()
h:BuildRigFromAttachments()
local sh = m.RightUpperArm:FindFirstChild("RightShoulder")
assert(sh and sh:IsA("AnimationConstraint"), sh and sh.ClassName)
assert(sh.IsKinematic and sh.MaxTorque == 3000)
assert(sh.Part0 == m.UpperTorso and sh.Part1 == m.RightUpperArm, tostring(sh.Part0))
assert(sh.C0.Position == Vector3.new(1, 0.5, 0))
local bs = m.RightUpperArm:FindFirstChild("RightShoulderBallSocket")
assert(bs and bs.UpperAngle == 110 and bs.TwistLowerAngle == -85 and bs.Attachment0.Name == "JointRotation")
assert(m.UpperTorso:FindFirstChild("Root") and not m.UpperTorso:FindFirstChild("RootBallSocket"))
-- an Animator poses it
local seq = Instance.new("KeyframeSequence"); seq.Loop = true
for i = 0, 1 do
    local k = Instance.new("Keyframe", seq); k.Time = i
    local p = Instance.new("Pose", k); p.Name = "UpperTorso"
    local q = Instance.new("Pose", p); q.Name = "RightUpperArm"; q.CFrame = CFrame.new(0, 2, 0)
end
local a = Instance.new("Animation")
a.AnimationId = game:GetService("KeyframeSequenceProvider"):RegisterKeyframeSequence(seq)
local t = h:LoadAnimation(a); t:Play(0)
task.wait(0.1)
assert(math.abs(sh.Transform.Y - 2) < 1e-3, tostring(sh.Transform))
-- ReplaceBodyPartR15 rebuilds the upgraded joint on the new part.
h.RigType = Enum.HumanoidRigType.R15
local new = Instance.new("Part"); new.Name = "NewArm"
local att = Instance.new("Attachment", new); att.Name = "RightShoulderRigAttachment"; att.Position = Vector3.new(-0.5, 0.5, 0)
assert(h:ReplaceBodyPartR15(Enum.BodyPartR15.RightUpperArm, new) == true)
assert(new.Parent == m and not m:FindFirstChild("RightUpperArm"))
local j = new:FindFirstChild("RightShoulder")
assert(j and j:IsA("AnimationConstraint") and j.Part0 == m.UpperTorso and new:FindFirstChild("RightShoulderBallSocket"))
assert(h:ReplaceBodyPartR15(Enum.BodyPartR15.Head, Instance.new("Part")) == false)
print("anim-pass")
"#, 20);
        // StarterPlayer.AvatarJointUpgrade = Disabled keeps Motor6D rigs.
        let mut d = crate::doc::Document::new_place();
        let sp = d.find_child(d.dom.root_ref(), "StarterPlayer").unwrap();
        d.dom.get_by_ref_mut(sp).unwrap().properties.insert(
            rbx_dom_weak::ustr("AvatarJointUpgrade"),
            rbx_dom_weak::types::Variant::Enum(rbx_dom_weak::types::Enum::from_u32(1)),
        );
        assert!(!crate::rig::joint_upgrade(&d));
        assert!(crate::rig::joint_upgrade(&crate::doc::Document::new_place()));
    }

    /// A RotationCurve as Studio 0.746 saved it (3 keys: Y 0, 1, 2 rad at
    /// t 0, 1, 2, Linear) reads back and plays in a CurveAnimation.
    #[test]
    fn studio_saved_rotation_curve_plays() {
        let hex = "01000000030000000d0000000000000000000000000000803f00000000000000000d000000004377f53e0000000040a9603f00000000000000000d00000000a56a573f0000000040510a3f000000000000000001000000030000000000000060090000c0120000";
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        run_with(r#"
local clip = workspace.Clip
local rc = clip.Root.Limb.Rotation
local keys = rc:GetKeys()
assert(#keys == 3 and rc.Length == 3, #keys)
assert(math.abs(keys[2].Time - 1) < 1e-4 and keys[2].Interpolation == Enum.KeyInterpolationMode.Linear)
local _, y = rc:GetValueAtTime(1.5):ToOrientation()
assert(math.abs(y - 1.5) < 1e-3, y)
assert(math.abs(clip.Length - 2) < 1e-4, clip.Length)
local model = Instance.new("Model", workspace)
local animator = Instance.new("Animator", Instance.new("AnimationController", model))
local root = Instance.new("Part", model); root.Name = "Root"
local limb = Instance.new("Part", model); limb.Name = "Limb"
local motor = Instance.new("Motor6D", root); motor.Part0 = root; motor.Part1 = limb
local a = Instance.new("Animation")
a.AnimationId = game:GetService("AnimationClipProvider"):RegisterAnimationClip(clip)
local t = animator:LoadAnimation(a); t:Play(0, 1, 0); t.TimePosition = 0.5
task.wait(0.05)
local _, ty = motor.Transform:ToOrientation()
assert(math.abs(ty - 0.5) < 0.01, ty)
-- a script edit writes Studio's bytes back
rc:InsertKey(RotationCurveKey.new(3, CFrame.Angles(0, 3, 0), Enum.KeyInterpolationMode.Linear))
print("anim-pass")
"#, 10, |d| {
            let ws = d.workspace().unwrap();
            let clip = d.dom.insert(ws, InstanceBuilder::new("CurveAnimation").with_name("Clip"));
            let root = d.dom.insert(clip, InstanceBuilder::new("Folder").with_name("Root"));
            let limb = d.dom.insert(root, InstanceBuilder::new("Folder").with_name("Limb"));
            d.dom.insert(
                limb,
                InstanceBuilder::new("RotationCurve")
                    .with_name("Rotation")
                    .with_property("ValuesAndTimes", rbx_dom_weak::types::BinaryString::from(bytes)),
            );
        });
    }

    #[test]
    fn motor_steps_and_ik_reaches() {
        run(r#"
local function near(a, b, e) assert(math.abs(a - b) < (e or 0.02), tostring(a) .. " ~= " .. tostring(b)) end
local a = Instance.new("Part", workspace); a.Anchored = true; a.CFrame = CFrame.new(10, 10, 0)
local b = Instance.new("Part", workspace); b.CFrame = CFrame.new(10, 10, 3)
local m = Instance.new("Motor"); m.Part0 = a; m.Part1 = b; m.C1 = CFrame.new(0, 0, -3); m.Parent = a
m.MaxVelocity = 0.1; m.DesiredAngle = 1
assert(m.Active)
task.wait(0.1)
near(m.CurrentAngle, 0.6, 0.11)
task.wait(0.3)
near(m.CurrentAngle, 1, 1e-4)
-- A two-bone arm: shoulder at the origin, 1 + 1 long, pointing down.
local model = Instance.new("Model", workspace)
local base = Instance.new("Part", model); base.Name = "Base"; base.Anchored = true; base.CFrame = CFrame.new(0, 0, 0)
local upper = Instance.new("Part", model); upper.Name = "Upper"; upper.CFrame = CFrame.new(0, -0.5, 0)
local lower = Instance.new("Part", model); lower.Name = "Lower"; lower.CFrame = CFrame.new(0, -1.5, 0)
local hand = Instance.new("Part", model); hand.Name = "Hand"; hand.CFrame = CFrame.new(0, -2, 0)
local function joint(name, p0, p1, c0, c1)
    local j = Instance.new("Motor6D"); j.Name = name; j.Part0 = p0; j.Part1 = p1; j.C0 = c0; j.C1 = c1; j.Parent = p1; return j
end
local sh = joint("Shoulder", base, upper, CFrame.new(), CFrame.new(0, 0.5, 0))
local el = joint("Elbow", upper, lower, CFrame.new(0, -0.5, 0), CFrame.new(0, 0.5, 0))
local wr = joint("Wrist", lower, hand, CFrame.new(0, -0.5, 0), CFrame.new())
local tgt = Instance.new("Part", workspace); tgt.Anchored = true; tgt.CFrame = CFrame.new(1, -1, 0.5)
local ik = Instance.new("IKControl", model)
ik.ChainRoot = upper; ik.EndEffector = hand; ik.Target = tgt; ik.Type = Enum.IKControlType.Position; ik.SmoothTime = 0
assert(ik:GetChainCount() == 2, ik:GetChainCount())
near(ik:GetChainLength(), 2, 1e-4)
near(ik:GetNodeLocalCFrame(2).Y, -1, 1e-4)
assert(ik:GetNodeWorldCFrame(0) == CFrame.new())
task.wait(0.2)
-- Forward kinematics of the solved joints: the wrist node on the target.
local w = ik:GetNodeWorldCFrame(3).Position
near((w - tgt.Position).Magnitude, 0, 0.01)
near((ik:GetRawFinalTarget().Position - tgt.Position).Magnitude, 0, 0.01)
-- Out of reach: 0.9913 of full stretch.
tgt.CFrame = CFrame.new(0, 5, 0)
task.wait(0.3)
near(ik:GetNodeWorldCFrame(3).Position.Magnitude, 2 * 0.9913, 0.01)
print("anim-pass")
"#, 60);
    }
}
