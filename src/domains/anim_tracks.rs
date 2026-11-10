//! The Animator's tracks stepped natively (frame hook after PreAnimation,
//! anim.rs `animate`), as Studio's Animator::animatorOnStepped /
//! evaluateAndAccumulateAnimationTracks / AnimationTrack::accumulate do
//! (Studio's profiler names).
//!
//! What stays in Luau (library.luau, run once per load, not per frame):
//! finding and parsing an AnimationId's KeyframeSequence / CurveAnimation
//! (`__rs_anim_load`), and sampling CurveAnimation channels
//! (`__rs_anim_sample`, called per curve joint). KeyframeSequence poses,
//! weights, fades, priorities, events and Motor6D.Transform writes are here.
//!
//! Measured in Studio (probes a_order / a_fade, client and server):
//! - The step runs after the PreAnimation handlers, before Stepped, with the
//!   simulation dt; a track advances by dt on the step after Play.
//! - Marker / KeyframeReached handlers run inside the step, before the joints
//!   are written (they see the previous pose).
//! - A non-looped track reaching its end: Stopped, then DidLoop; it then
//!   fades out linearly over 0.3 s from its current weight, whatever Play's
//!   fadeTime was. Stop() fades over 0.1 s by default. Ended fires when the
//!   weight reaches 0; the Animator lets the track go a step later.
//! - Transform = position (weighted) · slerp(identity, rotation, used weight).

use std::collections::HashMap;
use std::rc::Rc;

use glam::{Mat3, Quat, Vec3};
use mlua::{Function, Lua, Table, Value};
use rbx_dom_weak::types::{Ref, Variant};

use crate::math::Xform;
use crate::runtime::*;

struct Key {
    t: f64,
    cf: Xform,
    style: String,
    dir: String,
}

enum Joint {
    Keys(Vec<Key>),
    /// A CurveAnimation joint: its Luau channel table.
    Curves(Table),
}

struct Frame {
    t: f64,
    name: String,
    markers: Vec<(String, String)>,
}

pub struct Seq {
    joints: Vec<(String, Joint)>,
    frames: Vec<Frame>,
    length: f64,
    looped: bool,
    priority: Value,
}

struct Track {
    me: Value,
    anim: Value,
    owner: Ref,
    rig: Ref,
    seq: Option<Rc<Seq>>,
    pending: Option<String>,
    playing: bool,
    active: bool,
    ended: bool,
    release: bool,
    first_step: bool,
    speed: f64,
    weight: f64,
    target: f64,
    fade: f64,
    fade_start: f64,
    fade_elapsed: f64,
    time: f64,
    looped: bool,
    looped_set: bool,
    priority: Value,
    priority_set: bool,
}

#[derive(Default)]
pub struct Tracks {
    /// Load order (iteration order of the step).
    order: Vec<Ref>,
    all: HashMap<Ref, Track>,
    seqs: HashMap<String, Rc<Seq>>,
    /// rig → (Part1 name, joint): the joints a rig's tracks animate, found
    /// when it starts animating and kept while it does.
    animated: Vec<(Ref, Vec<(String, Ref)>)>,
    steps: u64,
}

fn tracks(lua: &Lua) -> mlua::AppDataRefMut<'_, Tracks> {
    if lua.app_data_ref::<Tracks>().is_none() {
        lua.set_app_data(Tracks::default());
    }
    lua.app_data_mut::<Tracks>().expect("track state")
}

// ------------------------------------------------------------- sequences

fn xform_of(v: &Value) -> Option<Xform> {
    match v {
        Value::UserData(u) => u.borrow::<LCFrame>().ok().map(|c| c.0),
        _ => None,
    }
}

fn convert(t: &Table) -> mlua::Result<Seq> {
    let mut joints = Vec::new();
    let jt: Table = t.get("joints")?;
    for pair in jt.pairs::<String, Table>() {
        let (name, list) = pair?;
        if let Ok(Some(curves)) = list.get::<Option<Table>>("curves") {
            let _ = curves;
            joints.push((name, Joint::Curves(list)));
            continue;
        }
        let mut keys = Vec::new();
        for k in list.sequence_values::<Table>() {
            let k = k?;
            keys.push(Key {
                t: k.get("t")?,
                cf: xform_of(&k.get::<Value>("cf")?).unwrap_or_else(super::anim_ik::ident),
                style: k.get::<Option<String>>("style")?.unwrap_or_else(|| "Linear".into()),
                dir: k.get::<Option<String>>("dir")?.unwrap_or_else(|| "In".into()),
            });
        }
        joints.push((name, Joint::Keys(keys)));
    }
    let mut frames = Vec::new();
    for f in t.get::<Table>("frames")?.sequence_values::<Table>() {
        let f = f?;
        let mut markers = Vec::new();
        if let Ok(Some(ms)) = f.get::<Option<Table>>("markers") {
            for m in ms.sequence_values::<Table>() {
                let m = m?;
                markers.push((m.get::<String>(1)?, m.get::<Option<String>>(2)?.unwrap_or_default()));
            }
        }
        frames.push(Frame { t: f.get("t")?, name: f.get::<Option<String>>("name")?.unwrap_or_default(), markers });
    }
    Ok(Seq {
        joints,
        frames,
        length: t.get::<Option<f64>>("length")?.unwrap_or(0.0),
        looped: t.get::<Option<bool>>("loop")?.unwrap_or(false),
        priority: t.get("priority")?,
    })
}

/// The sequence for an id: (Some, _) loaded, (None, true) still
/// downloading, (None, false) unavailable.
fn load(lua: &Lua, id: &str) -> mlua::Result<(Option<Rc<Seq>>, bool)> {
    if let Some(s) = tracks(lua).seqs.get(id) {
        return Ok((Some(s.clone()), false));
    }
    let f: Function = lua.globals().get("__rs_anim_load")?;
    let (seq, still): (Value, bool) = f.call(id)?;
    let Value::Table(t) = seq else { return Ok((None, still)) };
    let s = Rc::new(convert(&t)?);
    tracks(lua).seqs.insert(id.to_string(), s.clone());
    Ok((Some(s), false))
}

// --------------------------------------------------------------- poses

fn ease_in(style: &str, t: f64) -> f64 {
    use std::f64::consts::PI;
    match style {
        "Sine" => 1.0 - (t * PI / 2.0).cos(),
        "Cubic" => t * t * t,
        "Quart" => t.powi(4),
        "Quint" => t.powi(5),
        "Exponential" => {
            if t == 0.0 {
                0.0
            } else {
                2f64.powf(10.0 * (t - 1.0))
            }
        }
        "Circular" => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
        "Back" => {
            let s = 1.70158;
            t * t * ((s + 1.0) * t - s)
        }
        // Pose easing keeps the textbook 0.3 period (measured, selftest/anim.rs).
        "Elastic" => {
            if t == 0.0 || t == 1.0 {
                return t;
            }
            -(2f64.powf(10.0 * (t - 1.0))) * ((t - 1.075) * (2.0 * PI) / 0.3).sin()
        }
        "Bounce" => {
            let mut u = 1.0 - t;
            let b = if u < 1.0 / 2.75 {
                7.5625 * u * u
            } else if u < 2.0 / 2.75 {
                u -= 1.5 / 2.75;
                7.5625 * u * u + 0.75
            } else if u < 2.5 / 2.75 {
                u -= 2.25 / 2.75;
                7.5625 * u * u + 0.9375
            } else {
                u -= 2.625 / 2.75;
                7.5625 * u * u + 0.984375
            };
            1.0 - b
        }
        "Linear" => t,
        _ => t * t,
    }
}

fn ease(a: f64, style: &str, dir: &str) -> f64 {
    let a = a.clamp(0.0, 1.0);
    if style == "Linear" {
        return a;
    }
    match dir {
        "In" => ease_in(style, a),
        "InOut" => {
            if a < 0.5 {
                ease_in(style, a * 2.0) / 2.0
            } else {
                1.0 - ease_in(style, (1.0 - a) * 2.0) / 2.0
            }
        }
        _ => 1.0 - ease_in(style, 1.0 - a),
    }
}

/// Studio 0.746 measurements: only legacy Cubic reverses In / Out.
fn pose_alpha(a: f64, style: &str, dir: &str) -> f64 {
    use std::f64::consts::PI;
    match style {
        "Constant" => {
            if dir == "In" || (dir == "InOut" && a >= 0.5) {
                1.0
            } else {
                0.0
            }
        }
        "Linear" => a,
        "Cubic" => ease(a, "Cubic", match dir {
            "In" => "Out",
            "Out" => "In",
            d => d,
        }),
        "CubicV2" => ease(a, "Cubic", dir),
        "Elastic" if dir == "InOut" => {
            if a == 0.0 || a == 1.0 {
                return a;
            }
            let t = a * 2.0 - 1.0;
            if t < 0.0 {
                -0.5 * 2f64.powf(10.0 * t) * ((t - 0.1125) * 2.0 * PI / 0.45).sin()
            } else {
                1.0 + 0.5 * 2f64.powf(-10.0 * t) * ((t - 0.1125) * 2.0 * PI / 0.45).sin()
            }
        }
        _ => ease(a, style, dir),
    }
}

/// Keyframe poses: normalized linear quaternion interpolation.
fn pose_lerp(a: &Xform, b: &Xform, t: f64) -> Xform {
    let qa = Quat::from_mat3(&a.rot);
    let mut qb = Quat::from_mat3(&b.rot);
    if qa.dot(qb) < 0.0 {
        qb = -qb;
    }
    let tf = t as f32;
    let q = qa * (1.0 - tf) + qb * tf;
    let rot = if q.length() > 1e-8 { Mat3::from_quat(q.normalize()) } else { Mat3::IDENTITY };
    Xform { pos: a.pos.lerp(b.pos, tf), rot }
}

fn sample_keys(list: &[Key], t: f64) -> Option<Xform> {
    let n = list.len();
    if n == 0 {
        return None;
    }
    if t <= list[0].t {
        return Some(list[0].cf);
    }
    for i in 0..n - 1 {
        let (a, b) = (&list[i], &list[i + 1]);
        if t < b.t {
            if b.t <= a.t {
                return Some(a.cf);
            }
            return Some(pose_lerp(&a.cf, &b.cf, pose_alpha((t - a.t) / (b.t - a.t), &a.style, &a.dir)));
        }
    }
    Some(list[n - 1].cf)
}

fn sample(lua: &Lua, j: &Joint, t: f64) -> mlua::Result<Option<Xform>> {
    match j {
        Joint::Keys(k) => Ok(sample_keys(k, t)),
        Joint::Curves(list) => {
            let f: Function = lua.globals().get("__rs_anim_sample")?;
            Ok(xform_of(&f.call::<Value>((list.clone(), t))?))
        }
    }
}

fn rank(priority: &Value) -> i64 {
    match priority {
        Value::UserData(u) => match u.borrow::<LEnumItem>() {
            Ok(e) if e.name != "Core" => e.value as i64,
            _ => -1,
        },
        _ => -1,
    }
}

// --------------------------------------------------------------- events

fn fire(lua: &Lua, r: Ref, name: &str, args: Vec<Value>) -> mlua::Result<()> {
    world(lua)?.fire(r, name, args);
    signal_timing::flush_immediate(lua);
    Ok(())
}

fn fade_track(st: &mut Track, target: f64, duration: f64) {
    st.target = target;
    st.fade = duration.max(0.0);
    st.fade_start = st.weight;
    st.fade_elapsed = 0.0;
}

/// Studio: a non-looped track that runs out fades over 0.3 s (a_fade).
const END_FADE: f64 = 0.3;

// ----------------------------------------------------------------- step

/// One track's time / weight / events; returns whether it poses its rig.
fn step_track(lua: &Lua, r: Ref, dt: f64) -> mlua::Result<bool> {
    // Let go of a track whose Ended handlers ran without replaying it.
    {
        let mut t = tracks(lua);
        let Some(st) = t.all.get_mut(&r) else { return Ok(false) };
        if st.release {
            st.release = false;
            if st.ended && !st.playing {
                st.active = false;
            }
        }
    }
    let pending = tracks(lua).all.get(&r).and_then(|s| s.pending.clone());
    if let Some(id) = pending {
        let (seq, still) = load(lua, &id)?;
        if let Some(st) = tracks(lua).all.get_mut(&r) {
            if let Some(seq) = &seq {
                st.seq = Some(seq.clone());
                if !st.looped_set {
                    st.looped = seq.looped;
                }
                if !st.priority_set {
                    st.priority = seq.priority.clone();
                }
            }
            if seq.is_some() || !still {
                st.pending = None;
            }
        }
    }
    // (frames crossed, loops, stopped now, end now)
    let (crossed, loops, stopped, ended_now) = {
        let mut t = tracks(lua);
        let Some(st) = t.all.get_mut(&r) else { return Ok(false) };
        if !st.active {
            return Ok(false);
        }
        let length = st.seq.as_ref().map(|s| s.length).unwrap_or(0.0);
        let mut crossed: Vec<usize> = Vec::new();
        let mut loops = 0usize;
        let mut stopped = false;
        let mut ended_now = false;
        if length > 0.0 {
            let seq = st.seq.clone().expect("seq with length");
            let frames = &seq.frames;
            let before = st.time;
            let after = before + dt * st.speed;
            let events = |lo: f64, hi: f64, reverse: bool, include_start: bool, out: &mut Vec<usize>| {
                let n = frames.len();
                for j in 0..n {
                    let i = if reverse { n - 1 - j } else { j };
                    let k = frames[i].t;
                    let hit = if reverse {
                        k >= lo && (k < hi || include_start && k == hi)
                    } else {
                        (k > lo || include_start && k == lo) && k <= hi
                    };
                    if hit {
                        out.push(i);
                    }
                }
            };
            if st.looped {
                st.time = after.rem_euclid(length);
                if st.speed >= 0.0 {
                    let mut lo = before;
                    for _ in 0..(after / length).floor().max(0.0) as i64 {
                        events(lo, length, false, st.first_step, &mut crossed);
                        st.first_step = true;
                        lo = 0.0;
                        loops += 1;
                    }
                    events(lo, st.time, false, st.first_step, &mut crossed);
                } else {
                    let mut hi = before;
                    for _ in 0..(-(after / length).floor()).max(0.0) as i64 {
                        events(0.0, hi, true, st.first_step || hi == length, &mut crossed);
                        hi = length;
                        loops += 1;
                    }
                    events(st.time, hi, true, hi == length || st.first_step, &mut crossed);
                }
            } else {
                st.time = after.clamp(0.0, length);
                events(before.min(st.time), before.max(st.time), st.speed < 0.0, st.first_step, &mut crossed);
                if st.playing && (after >= length && st.speed > 0.0 || after <= 0.0 && st.speed < 0.0) {
                    st.playing = false;
                    fade_track(st, 0.0, END_FADE);
                    stopped = true;
                    // Studio fires DidLoop as a non-looped track ends.
                    loops += 1;
                }
            }
            st.first_step = false;
            // Linear over the requested duration, whatever the size of the
            // change; unloaded tracks don't advance.
            st.fade_elapsed += dt;
            let a = if st.fade > 0.0 { (st.fade_elapsed / st.fade).min(1.0) } else { 1.0 };
            st.weight = st.fade_start + (st.target - st.fade_start) * a;
            if st.weight <= 0.0 && !st.playing && !st.ended {
                ended_now = true;
            }
        } else if !st.playing && st.fade <= 0.0 {
            // A failed / unloaded clip with a positive fade never evaluates
            // it in Studio; Stop(0) still lets it go.
            st.weight = 0.0;
            if !st.ended {
                ended_now = true;
            }
        }
        (crossed, loops, stopped, ended_now)
    };
    if stopped {
        fire(lua, r, "Stopped", vec![])?;
    }
    if !crossed.is_empty() || loops > 0 {
        let seq = tracks(lua).all.get(&r).and_then(|s| s.seq.clone());
        if let Some(seq) = seq {
            // Studio: markers, then named keyframes, then DidLoop; a wrap's
            // terminal and initial keys share the batch.
            for &i in &crossed {
                for (name, value) in &seq.frames[i].markers {
                    fire(lua, r, &format!("MarkerReached:{name}"), vec![Value::String(lua.create_string(value)?)])?;
                }
            }
            for &i in &crossed {
                let f = &seq.frames[i];
                if f.name != "Keyframe" {
                    fire(lua, r, "KeyframeReached", vec![Value::String(lua.create_string(&f.name)?)])?;
                }
            }
            for _ in 0..loops {
                fire(lua, r, "DidLoop", vec![])?;
            }
        }
    }
    if ended_now {
        let mut t = tracks(lua);
        if let Some(st) = t.all.get_mut(&r) {
            st.ended = true;
            st.release = true;
        }
        drop(t);
        fire(lua, r, "Ended", vec![])?;
    }
    let t = tracks(lua);
    let Some(st) = t.all.get(&r) else { return Ok(false) };
    if !st.active || st.seq.is_none() || st.weight <= 0.0 || st.rig.is_none() {
        return Ok(false);
    }
    let rig = st.rig;
    drop(t);
    Ok(world(lua)?.d().dom.get_by_ref(rig).is_some_and(|i| i.parent().is_some()))
}

fn joints_of(lua: &Lua, rig: Ref) -> mlua::Result<Vec<(String, Ref)>> {
    let w = world(lua)?;
    let d = w.d();
    let mut out: Vec<(String, Ref)> = Vec::new();
    for i in d.dom.descendants_of(rig) {
        // AnimationConstraint: Studio's upgraded avatar joint (anim domain).
        if i.class != "Motor6D" && i.class != "AnimationConstraint" {
            continue;
        }
        // An AnimationConstraint's parts come from its attachments (its
        // Part0 / Part1 getters, anim.rs prop_get).
        let stored = match d.prop(i.referent(), "Part1") {
            Some(Variant::Ref(p)) if p.is_some() => Some(p),
            _ => None,
        };
        let derived = || {
            let has_att = matches!(d.prop(i.referent(), "Attachment0"), Some(Variant::Ref(a)) if a.is_some());
            if has_att { crate::rig::joint_frames(d, i.referent()).map(|f| f.1) } else { None }
        };
        let p1 = if i.class == "AnimationConstraint" { derived().or(stored) } else { stored };
        let Some(p1) = p1.filter(|p| p.is_some()) else { continue };
        let Some(name) = d.dom.get_by_ref(p1).map(|p| p.name.clone()) else { continue };
        // The last one found for a name wins (a table keyed by name).
        if let Some(e) = out.iter_mut().find(|e| e.0 == name) {
            e.1 = i.referent();
        } else {
            out.push((name, i.referent()));
        }
    }
    Ok(out)
}

fn write_transform(lua: &Lua, motor: Ref, xf: Xform) -> mlua::Result<()> {
    // Studio's Animator writes Transform without firing Changed (a_chg: no
    // Changed / GetPropertyChangedSignal("Transform") over a whole track),
    // and Transform doesn't replicate: set it and mark it for the engine's
    // readers (rig poser, renderer) only.
    let mut w = world(lua)?;
    let d = w.dm();
    let v = Variant::CFrame(xf.to_cframe());
    let Some(i) = d.dom.get_by_ref_mut(motor) else { return Ok(()) };
    if i.parent().is_none() {
        return Ok(());
    }
    let key = rbx_dom_weak::ustr("Transform");
    if i.properties.get(&key) == Some(&v) {
        return Ok(());
    }
    i.properties.insert(key, v);
    d.mark_prop(motor, key);
    Ok(())
}

/// The Animator step over every track, then each animated rig's joints.
pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    let order = {
        let mut t = tracks(lua);
        t.steps += 1;
        if t.steps % 120 == 0 {
            // Tracks the collector freed (inactive and unreferenced).
            let alive: Vec<Ref> = {
                let w = world(lua)?;
                let d = w.d();
                t.order.iter().copied().filter(|r| t.all.get(r).is_some_and(|s| s.active) || d.dom.get_by_ref(*r).is_some()).collect()
            };
            t.all.retain(|r, _| alive.contains(r));
            t.order = alive;
        }
        if t.order.is_empty() && t.animated.is_empty() {
            return Ok(());
        }
        t.order.clone()
    };
    // rig → tracks posing it, in step order.
    let mut by_rig: Vec<(Ref, Vec<Ref>)> = Vec::new();
    for r in order {
        if step_track(lua, r, dt)? {
            let rig = tracks(lua).all.get(&r).map(|s| s.rig).unwrap_or_else(Ref::none);
            match by_rig.iter_mut().find(|e| e.0 == rig) {
                Some(e) => e.1.push(r),
                None => by_rig.push((rig, vec![r])),
            }
        }
    }
    for (rig, list) in &by_rig {
        let joints = {
            let t = tracks(lua);
            t.animated.iter().find(|e| e.0 == *rig).map(|e| e.1.clone())
        };
        let joints = match joints {
            Some(j) => j,
            None => {
                let j = joints_of(lua, *rig)?;
                tracks(lua).animated.push((*rig, j.clone()));
                j
            }
        };
        // Each track's (seq, weight, rank, time).
        let posing: Vec<(Rc<Seq>, f64, i64, f64)> = {
            let t = tracks(lua);
            list.iter()
                .filter_map(|r| t.all.get(r))
                .filter_map(|s| s.seq.clone().map(|q| (q, s.weight, rank(&s.priority), s.time)))
                .collect()
        };
        for (name, motor) in &joints {
            // This joint's poses by priority level, highest first.
            let mut levels: Vec<(i64, Vec<(Xform, f64)>)> = Vec::new();
            for (seq, w, rk, time) in &posing {
                let Some((_, j)) = seq.joints.iter().find(|(n, _)| n == name) else { continue };
                let Some(cf) = sample(lua, j, *time)? else { continue };
                match levels.iter_mut().find(|l| l.0 == *rk) {
                    Some(l) => l.1.push((cf, *w)),
                    None => levels.push((*rk, vec![(cf, *w)])),
                }
            }
            if levels.is_empty() {
                continue;
            }
            levels.sort_by(|a, b| b.0.cmp(&a.0));
            let (mut remaining, mut used) = (1.0f64, 0.0f64);
            let mut position = Vec3::ZERO;
            let (mut vector, mut scalar) = (glam::DVec3::ZERO, 0.0f64);
            let mut reference: Option<(glam::DVec3, f64)> = None;
            for (_, poses) in &levels {
                let total: f64 = poses.iter().map(|p| p.1).sum();
                let take = total.min(remaining);
                if take > 0.0 {
                    for (cf, pw) in poses {
                        let w = pw * take / total;
                        let q = Quat::from_mat3(&cf.rot);
                        // Canonical (w ≥ 0) like an axis-angle round trip.
                        let q = if q.w < 0.0 { -q } else { q };
                        let mut v = glam::DVec3::new(q.x as f64, q.y as f64, q.z as f64);
                        let mut s = q.w as f64;
                        let refq = *reference.get_or_insert((v, s));
                        if refq.0.dot(v) + refq.1 * s < 0.0 {
                            v = -v;
                            s = -s;
                        }
                        position += cf.pos * w as f32;
                        vector += v * w;
                        scalar += s * w;
                    }
                    used += take;
                    remaining -= take;
                }
            }
            let norm = (vector.dot(vector) + scalar * scalar).sqrt();
            let mut rot = Quat::IDENTITY;
            if norm > 1e-8 {
                let mag = vector.length();
                if mag > 1e-8 {
                    let axis = (vector / mag).as_vec3();
                    let angle = (2.0 * mag.atan2(scalar)) as f32;
                    rot = Quat::from_mat3(&Mat3::from_quat(Quat::from_axis_angle(axis, angle)));
                }
            }
            let rot = Quat::IDENTITY.slerp(rot, used as f32);
            write_transform(lua, *motor, Xform { pos: position, rot: Mat3::from_quat(rot) })?;
        }
    }
    // Rigs whose tracks all faded out go back to their rest pose once.
    let gone: Vec<(Ref, Vec<(String, Ref)>)> = {
        let mut t = tracks(lua);
        let (keep, gone): (Vec<_>, Vec<_>) =
            std::mem::take(&mut t.animated).into_iter().partition(|e| by_rig.iter().any(|b| b.0 == e.0));
        t.animated = keep;
        gone
    };
    for (_, joints) in gone {
        for (_, m) in joints {
            write_transform(lua, m, super::anim_ik::ident())?;
        }
    }
    // The Animator holds playing and fading tracks only: a released one is
    // the collector's once no script holds it.
    let mut t = tracks(lua);
    for st in t.all.values_mut() {
        if !st.active && !matches!(st.me, Value::Nil) {
            st.me = Value::Nil;
        }
    }
    Ok(())
}

// ------------------------------------------------------------ replication

/// One active (playing or fading) track, for Animator replication
/// (crate::anim_replicate). Read-only view of the native state.
pub struct TrackRep {
    pub track: Ref,
    pub owner: Ref,
    pub anim: Option<Ref>,
    pub playing: bool,
    pub speed: f64,
    pub target: f64,
    pub fade: f64,
    pub time: f64,
    pub looped: bool,
    /// AnimationPriority value (Core 0).
    pub priority: u32,
}

/// The active tracks, in load order.
pub fn replicated_state(lua: &Lua) -> Vec<TrackRep> {
    let t = tracks(lua);
    t.order
        .iter()
        .filter_map(|r| t.all.get(r).map(|s| (*r, s)))
        .filter(|(_, s)| s.active)
        .map(|(r, s)| TrackRep {
            track: r,
            owner: s.owner,
            anim: opt_inst(&s.anim),
            playing: s.playing,
            speed: s.speed,
            target: s.target,
            fade: s.fade,
            time: s.time,
            looped: s.looped,
            priority: rank(&s.priority).max(0) as u32,
        })
        .collect()
}

// ------------------------------------------------------------------ API

fn me_ref(v: &Value) -> Option<Ref> {
    opt_inst(v)
}

fn new_track(lua: &Lua, (me, anim, owner, rig, id): (Value, Value, Value, Value, String)) -> mlua::Result<()> {
    let Some(r) = me_ref(&me) else { return Ok(()) };
    let (seq, pending) = load(lua, &id)?;
    let core = LEnumItem::from_name("AnimationPriority", "Core")
        .map(|e| crate::ltypes::enum_value(lua, e))
        .transpose()?
        .unwrap_or(Value::Nil);
    let st = Track {
        me: Value::Nil,
        anim,
        owner: me_ref(&owner).unwrap_or_else(Ref::none),
        rig: me_ref(&rig).unwrap_or_else(Ref::none),
        looped: seq.as_ref().is_some_and(|s| s.looped),
        priority: seq.as_ref().map(|s| s.priority.clone()).unwrap_or(core),
        seq,
        pending: pending.then_some(id),
        playing: false,
        active: false,
        ended: false,
        release: false,
        first_step: false,
        speed: 1.0,
        weight: 0.0,
        target: 0.0,
        fade: 0.1,
        fade_start: 0.0,
        fade_elapsed: 0.0,
        time: 0.0,
        looped_set: false,
        priority_set: false,
    };
    let mut t = tracks(lua);
    t.all.insert(r, st);
    t.order.push(r);
    Ok(())
}

/// Play: false when the Animator's track limit refuses it (the caller warns).
fn play(lua: &Lua, (me, fade, weight, speed): (Value, Option<f64>, Option<f64>, Option<f64>)) -> mlua::Result<(bool, Value, Value)> {
    let Some(r) = me_ref(&me) else { return Ok((true, Value::Nil, Value::Nil)) };
    let mut t = tracks(lua);
    let owner = match t.all.get(&r) {
        Some(st) => st.owner,
        None => return Ok((true, Value::Nil, Value::Nil)),
    };
    let count = t.all.values().filter(|s| s.active && s.owner == owner).count();
    let st = t.all.get_mut(&r).expect("track");
    if !st.playing {
        st.time = 0.0;
    }
    st.playing = true;
    st.ended = false;
    st.release = false;
    st.first_step = true;
    fade_track(st, weight.unwrap_or(1.0), fade.unwrap_or(0.1));
    if !st.active {
        // Studio's insertion checks count > the configured limit (257 slots).
        if count > 256 {
            let (anim, rig) = (st.anim.clone(), st.rig);
            drop(t);
            return Ok((false, anim, inst_value(lua, rig)?));
        }
        st.active = true;
    }
    st.me = me.clone();
    st.speed = speed.unwrap_or(1.0);
    drop(t);
    // AnimationPlayed fires on the Animator and on its Humanoid /
    // AnimationController.
    let parent = world(lua)?.d().dom.get_by_ref(owner).map(|i| i.parent()).unwrap_or_else(Ref::none);
    if parent.is_some() {
        fire(lua, owner, "AnimationPlayed", vec![me.clone()])?;
        fire(lua, parent, "AnimationPlayed", vec![me])?;
    }
    Ok((true, Value::Nil, Value::Nil))
}

fn stop(lua: &Lua, (me, fade): (Value, Option<f64>)) -> mlua::Result<()> {
    let Some(r) = me_ref(&me) else { return Ok(()) };
    let fire_it = {
        let mut t = tracks(lua);
        match t.all.get_mut(&r) {
            Some(st) if st.playing => {
                st.playing = false;
                fade_track(st, 0.0, fade.unwrap_or(0.1));
                true
            }
            _ => false,
        }
    };
    if fire_it {
        fire(lua, r, "Stopped", vec![])?;
    }
    Ok(())
}

fn adjust_speed(lua: &Lua, (me, speed): (Value, Option<f64>)) -> mlua::Result<()> {
    let Some(r) = me_ref(&me) else { return Ok(()) };
    if let Some(st) = tracks(lua).all.get_mut(&r) {
        st.speed = speed.unwrap_or(1.0);
    }
    Ok(())
}

fn adjust_weight(lua: &Lua, (me, weight, fade): (Value, Option<f64>, Option<f64>)) -> mlua::Result<()> {
    let Some(r) = me_ref(&me) else { return Ok(()) };
    if let Some(st) = tracks(lua).all.get_mut(&r) {
        fade_track(st, weight.unwrap_or(1.0), fade.unwrap_or(0.1));
    }
    Ok(())
}

/// AnimationTrack properties (runtime_ext reads them through `__rs_track_get`).
fn get(lua: &Lua, (me, key): (Value, String)) -> mlua::Result<Value> {
    let Some(r) = me_ref(&me) else { return Ok(Value::Nil) };
    let t = tracks(lua);
    let Some(st) = t.all.get(&r) else { return Ok(Value::Nil) };
    Ok(match key.as_str() {
        "Length" => Value::Number(st.seq.as_ref().map(|s| s.length).unwrap_or(0.0)),
        "TimePosition" => Value::Number(st.time),
        "IsPlaying" => Value::Boolean(st.playing),
        "Speed" => Value::Number(st.speed),
        "WeightCurrent" => Value::Number(st.weight),
        "WeightTarget" => Value::Number(st.target),
        "Looped" => Value::Boolean(st.looped),
        "Priority" => st.priority.clone(),
        "Animation" => st.anim.clone(),
        _ => Value::Nil,
    })
}

fn set(lua: &Lua, (me, key, value): (Value, String, Value)) -> mlua::Result<bool> {
    let Some(r) = me_ref(&me) else { return Ok(false) };
    let mut t = tracks(lua);
    let Some(st) = t.all.get_mut(&r) else { return Ok(false) };
    match key.as_str() {
        "Looped" => {
            st.looped = matches!(value, Value::Boolean(true));
            st.looped_set = true;
        }
        "Priority" => {
            st.priority = value;
            st.priority_set = true;
        }
        "TimePosition" => {
            let length = st.seq.as_ref().map(|s| s.length).unwrap_or(0.0);
            let v = match value {
                Value::Number(n) => n,
                Value::Integer(i) => i as f64,
                Value::String(s) => s.to_str().ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0.0),
                _ => 0.0,
            };
            st.time = if length <= 0.0 {
                0.0
            } else if st.looped {
                v.rem_euclid(length)
            } else {
                v.clamp(0.0, length)
            };
            st.first_step = false;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// The tracks an Animator holds (playing or fading), in load order.
fn playing(lua: &Lua, owner: Value) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let Some(o) = me_ref(&owner) else { return Ok(out) };
    let list: Vec<Ref> = {
        let t = tracks(lua);
        t.order.iter().copied().filter(|r| t.all.get(r).is_some_and(|s| s.active && s.owner == o)).collect()
    };
    for (i, r) in list.into_iter().enumerate() {
        out.raw_set(i + 1, inst_value(lua, r)?)?;
    }
    Ok(out)
}

fn keyframe_time(lua: &Lua, (me, name): (Value, String)) -> mlua::Result<Option<f64>> {
    let Some(r) = me_ref(&me) else { return Ok(None) };
    let t = tracks(lua);
    Ok(t.all.get(&r).and_then(|s| s.seq.as_ref()).and_then(|q| q.frames.iter().find(|f| f.name == name).map(|f| f.t)))
}

/// Animator:GetTrackByAnimationId: an active track of this Animator.
fn by_id(lua: &Lua, (owner, id): (Value, String)) -> mlua::Result<Value> {
    let Some(o) = me_ref(&owner) else { return Ok(Value::Nil) };
    let cands: Vec<(Ref, Value)> = {
        let t = tracks(lua);
        t.order
            .iter()
            .filter_map(|r| t.all.get(r).map(|s| (*r, s)))
            .filter(|(_, s)| s.active && s.owner == o)
            .map(|(r, s)| (r, s.anim.clone()))
            .collect()
    };
    for (r, anim) in cands {
        if let Some(a) = opt_inst(&anim) {
            if let Ok(Value::String(s)) = index(lua, a, "AnimationId") {
                if s.to_str()? == id.as_str() {
                    return inst_value(lua, r);
                }
            }
        }
    }
    Ok(Value::Nil)
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set("__rs_track_new", lua.create_function(new_track)?)?;
    g.set("__rs_track_play", lua.create_function(play)?)?;
    g.set("__rs_track_stop", lua.create_function(stop)?)?;
    g.set("__rs_track_speed", lua.create_function(adjust_speed)?)?;
    g.set("__rs_track_weight", lua.create_function(adjust_weight)?)?;
    g.set("__rs_track_get", lua.create_function(get)?)?;
    g.set("__rs_track_set", lua.create_function(set)?)?;
    g.set("__rs_track_playing", lua.create_function(playing)?)?;
    g.set("__rs_track_keyframe_time", lua.create_function(keyframe_time)?)?;
    g.set("__rs_track_by_id", lua.create_function(by_id)?)?;
    g.set(
        "__rs_anim_forget",
        lua.create_function(|lua, id: String| {
            tracks(lua).seqs.remove(&id);
            Ok(())
        })?,
    )?;
    Ok(())
}
