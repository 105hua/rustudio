//! TweenService stepped natively, as Studio does.
//!
//! Studio: TweenService steps (profiler "TweenService") inside the UI
//! layout update (before "UpdateUILayouts"), so a
//! client steps its tweens right after the PreRender handlers with the render
//! frame's dt; a server (no render) steps them after Heartbeat with
//! Heartbeat's dt (measured with probes). The generic
//! stepper: drops dead entries, moves
//! the tweens Played since the last step into the active list (for each: a newer tween takes over the properties it shares with older
//! ones; an older one left with none is Cancelled), then steps the list
//! from the back (float dt) and swap-removes the ones that
//! finished. So tweens finishing in the same step fire Completed in reverse
//! Play order.
//!
//! Per tween (measured with probes):
//! - Play from Begin / Completed / Cancelled starts over: Delayed while
//!   DelayTime runs (the step that ends the delay writes nothing), then the
//!   start values are read on the first playing step, which already
//!   advances by that step's dt. Play while Playing or Delayed does nothing;
//!   Play while Paused resumes with the elapsed time and start values kept.
//! - A repetition lasts Time (twice with Reverses; the turn carries the
//!   overshoot). Its last step writes the end value; the overshoot is
//!   dropped and the next repetition (after DelayTime again) starts from
//!   the same start values. RepeatCount < 0 repeats forever.
//! - The last step writes the goal and fires Completed(Completed).
//! - Pause only acts while Playing; Cancel while Playing / Delayed / Paused
//!   (state Cancelled, Completed(Cancelled) at the call; values stay).
//! - Destroying the tween or its instance doesn't stop it.
//! - Numbers / vectors / CFrames (position lerp, rotation slerp) / colours /
//!   UDims / Rects interpolate; integer properties round; booleans and
//!   anything else take the goal on the first step.
//! - Easing = TweenService:GetValue (all 33 style × direction rows equal
//!   Studio's to 1e-6, t_ease); Elastic's period is 4/13 (pose easing keeps
//!   its own 0.3, library.luau).

use std::collections::HashMap;

use glam::{Mat3, Quat};
use mlua::{Lua, Value};
use rbx_dom_weak::types::{Ref, Variant};

use crate::runtime::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Begin,
    Delayed,
    Playing,
    Paused,
    Completed,
    Cancelled,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Begin => "Begin",
            State::Delayed => "Delayed",
            State::Playing => "Playing",
            State::Paused => "Paused",
            State::Completed => "Completed",
            State::Cancelled => "Cancelled",
        }
    }
}

pub struct Tween {
    /// The Tween's own Lua value while it is in the active list (keeps it
    /// from being collected, as Studio's list holds it).
    me: Option<Value>,
    inst: Ref,
    inst_val: Value,
    info: Value,
    time: f64,
    style: String,
    dir: String,
    repeats: i32,
    reverses: bool,
    delay: f64,
    goals: Vec<(String, Value)>,
    starts: Option<Vec<Option<(Value, bool)>>>,
    state: State,
    elapsed: f64,
    delay_elapsed: f64,
    rep: i32,
}

#[derive(Default)]
pub struct Tweens {
    pub all: HashMap<Ref, Tween>,
    active: Vec<Ref>,
    to_add: Vec<Ref>,
    steps: u64,
}

fn tweens(lua: &Lua) -> mlua::AppDataRefMut<'_, Tweens> {
    if lua.app_data_ref::<Tweens>().is_none() {
        lua.set_app_data(Tweens::default());
    }
    lua.app_data_mut::<Tweens>().expect("tween state")
}

// ------------------------------------------------------------------ easing

fn bounce_out(mut t: f64) -> f64 {
    if t < 1.0 / 2.75 {
        7.5625 * t * t
    } else if t < 2.0 / 2.75 {
        t -= 1.5 / 2.75;
        7.5625 * t * t + 0.75
    } else if t < 2.5 / 2.75 {
        t -= 2.25 / 2.75;
        7.5625 * t * t + 0.9375
    } else {
        t -= 2.625 / 2.75;
        7.5625 * t * t + 0.984375
    }
}

fn ease_in(style: &str, t: f64) -> f64 {
    use std::f64::consts::PI;
    match style {
        "Linear" => t,
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
        // Studio's TweenService Elastic: period 4/13 (t_ease: every row
        // matches; the textbook 0.3 is off by up to 0.02).
        "Elastic" => {
            if t == 0.0 || t == 1.0 {
                return t;
            }
            let u = 1.0 - t;
            -(2f64.powf(-10.0 * u)) * ((u - 1.0 / 13.0) * 6.5 * PI).sin()
        }
        "Bounce" => 1.0 - bounce_out(1.0 - t),
        _ => t * t, // Quad
    }
}

/// TweenService:GetValue(alpha, style, direction).
pub fn ease(alpha: f64, style: &str, dir: &str) -> f64 {
    let a = if alpha.is_nan() { 0.0 } else { alpha.clamp(0.0, 1.0) };
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

// ---------------------------------------------------------- interpolation

fn ud<T: 'static + Clone>(v: &Value) -> Option<T> {
    match v {
        Value::UserData(u) => u.borrow::<T>().ok().map(|r| (*r).clone()),
        _ => None,
    }
}

/// start → goal at k (in Lua values). `int`: the property holds an integer.
fn lerp(lua: &Lua, a: &Value, b: &Value, k: f64, int: bool) -> mlua::Result<Value> {
    use mlua::IntoLua;
    let kf = k as f32;
    if let (Some(x), Some(y)) = (num(a), num(b)) {
        let v = x + (y - x) * k;
        return Ok(Value::Number(if int { v.round() } else { v }));
    }
    if let (Some(x), Some(y)) = (crate::ltypes::v3_opt(a), crate::ltypes::v3_opt(b)) {
        return LVector3(x.0.lerp(y.0, kf)).into_lua(lua);
    }
    if let (Some(x), Some(y)) = (ud::<LCFrame>(a), ud::<LCFrame>(b)) {
        let q = Quat::from_mat3(&x.0.rot).slerp(Quat::from_mat3(&y.0.rot), kf);
        return LCFrame(crate::math::Xform { pos: x.0.pos.lerp(y.0.pos, kf), rot: Mat3::from_quat(q) }).into_lua(lua);
    }
    if let (Some(x), Some(y)) = (ud::<LColor3>(a), ud::<LColor3>(b)) {
        return LColor3([0, 1, 2].map(|i| x.0[i] + (y.0[i] - x.0[i]) * kf)).into_lua(lua);
    }
    if let (Some(x), Some(y)) = (ud::<LVector2>(a), ud::<LVector2>(b)) {
        return LVector2(x.0.lerp(y.0, kf)).into_lua(lua);
    }
    let udim = |p: LUDim, q: LUDim| LUDim {
        scale: p.scale + (q.scale - p.scale) * kf,
        offset: (p.offset as f32 + (q.offset - p.offset) as f32 * kf).round() as i32,
    };
    if let (Some(x), Some(y)) = (ud::<LUDim2>(a), ud::<LUDim2>(b)) {
        return LUDim2 { x: udim(x.x, y.x), y: udim(x.y, y.y) }.into_lua(lua);
    }
    if let (Some(x), Some(y)) = (ud::<LUDim>(a), ud::<LUDim>(b)) {
        return udim(x, y).into_lua(lua);
    }
    if let (Some(x), Some(y)) = (ud::<LRect>(a), ud::<LRect>(b)) {
        return LRect { min: x.min.lerp(y.min, kf), max: x.max.lerp(y.max, kf) }.into_lua(lua);
    }
    // Booleans and the rest: the goal from the first step on.
    Ok(if k > 0.0 { b.clone() } else { a.clone() })
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

// ------------------------------------------------------------------ API

fn enum_name(v: &Value, dflt: &str) -> String {
    ud::<LEnumItem>(v).map(|e| e.name).unwrap_or_else(|| dflt.to_string())
}

fn state_value(lua: &Lua, s: State) -> mlua::Result<Value> {
    match LEnumItem::from_name("PlaybackState", s.name()) {
        Some(e) => crate::ltypes::enum_value(lua, e),
        None => Ok(Value::Nil),
    }
}

fn completed(lua: &Lua, tw: Ref, s: State) -> mlua::Result<()> {
    let v = state_value(lua, s)?;
    world(lua)?.fire(tw, "Completed", vec![v]);
    signal_timing::flush_immediate(lua);
    Ok(())
}

/// TweenService:Create: a Tween instance, registered here.
fn create(lua: &Lua, (inst, info, goals): (Value, Value, Value)) -> mlua::Result<Value> {
    let Some(target) = opt_inst(&inst) else {
        return Err(rt_err("TweenService:Create: Unable to cast value to Object"));
    };
    let Value::Table(goals) = goals else {
        return Err(rt_err("TweenService:Create: property table expected"));
    };
    let ti = ud::<LTweenInfo>(&info);
    let mut list = Vec::new();
    for pair in goals.pairs::<Value, Value>() {
        let (k, v) = pair?;
        if let Value::String(s) = k {
            list.push((s.to_str()?.to_string(), v));
        }
    }
    let r = super::super::super::ext::create_any(lua, "Tween")?;
    let (time, style, dir, repeats, reverses, delay) = match &ti {
        Some(t) => (t.time, t.easing_style.name.clone(), t.easing_direction.name.clone(), t.repeat_count, t.reverses, t.delay_time),
        None => (1.0, "Quad".into(), "Out".into(), 0, false, 0.0),
    };
    tweens(lua).all.insert(
        r,
        Tween {
            me: None,
            inst: target,
            inst_val: inst,
            info,
            time,
            style,
            dir,
            repeats,
            reverses,
            delay,
            goals: list,
            starts: None,
            state: State::Begin,
            elapsed: 0.0,
            delay_elapsed: 0.0,
            rep: 0,
        },
    );
    inst_value(lua, r)
}

fn play(lua: &Lua, me: Value) -> mlua::Result<bool> {
    let Some(r) = opt_inst(&me) else { return Ok(false) };
    let mut t = tweens(lua);
    let Some(tw) = t.all.get_mut(&r) else { return Ok(false) };
    match tw.state {
        State::Playing | State::Delayed => return Ok(true),
        State::Paused => tw.state = State::Playing,
        _ => {
            tw.elapsed = 0.0;
            tw.delay_elapsed = 0.0;
            tw.rep = 0;
            tw.starts = None;
            tw.state = if tw.delay > 0.0 { State::Delayed } else { State::Playing };
        }
    }
    tw.me = Some(me);
    if !t.active.contains(&r) && !t.to_add.contains(&r) {
        t.to_add.push(r);
    }
    Ok(true)
}

fn pause(lua: &Lua, me: Value) -> mlua::Result<bool> {
    let Some(r) = opt_inst(&me) else { return Ok(false) };
    let mut t = tweens(lua);
    let Some(tw) = t.all.get_mut(&r) else { return Ok(false) };
    if tw.state == State::Playing {
        tw.state = State::Paused;
    }
    Ok(true)
}

fn cancel(lua: &Lua, me: Value) -> mlua::Result<bool> {
    let Some(r) = opt_inst(&me) else { return Ok(false) };
    let fire = {
        let mut t = tweens(lua);
        let Some(tw) = t.all.get_mut(&r) else { return Ok(false) };
        if matches!(tw.state, State::Playing | State::Delayed | State::Paused) {
            tw.state = State::Cancelled;
            tw.elapsed = 0.0;
            true
        } else {
            false
        }
    };
    if fire {
        completed(lua, r, State::Cancelled)?;
    }
    Ok(true)
}

/// Tween.PlaybackState / Instance / TweenInfo (runtime_ext asks through
/// the `__rs_tween_get` global).
fn get(lua: &Lua, (me, key): (Value, String)) -> mlua::Result<Value> {
    let Some(r) = opt_inst(&me) else { return Ok(Value::Nil) };
    let (state, inst, info) = {
        let t = tweens(lua);
        match t.all.get(&r) {
            Some(tw) => (tw.state, tw.inst_val.clone(), tw.info.clone()),
            None => (State::Begin, Value::Nil, Value::Nil),
        }
    };
    match key.as_str() {
        "PlaybackState" => state_value(lua, state),
        "Instance" => Ok(inst),
        "TweenInfo" => {
            if matches!(info, Value::Nil) {
                let f: mlua::Function = lua.globals().get::<mlua::Table>("TweenInfo")?.get("new")?;
                f.call(())
            } else {
                Ok(info)
            }
        }
        _ => Ok(Value::Nil),
    }
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set("__rs_tween_create", lua.create_function(create)?)?;
    g.set("__rs_tween_play", lua.create_function(play)?)?;
    g.set("__rs_tween_pause", lua.create_function(pause)?)?;
    g.set("__rs_tween_cancel", lua.create_function(cancel)?)?;
    g.set("__rs_tween_get", lua.create_function(get)?)?;
    g.set(
        "__rs_tween_ease",
        lua.create_function(|_, (a, s, d): (f64, Value, Value)| {
            Ok(ease(a, &enum_name(&s, "Quad"), &enum_name(&d, "Out")))
        })?,
    )?;
    Ok(())
}

// ------------------------------------------------------------------ step

/// The property's current value, and whether it holds an integer.
fn read(lua: &Lua, inst: Ref, key: &str) -> Option<(Value, bool)> {
    let v = index(lua, inst, key).ok()?;
    let int = world(lua).ok().is_some_and(|w| {
        matches!(w.d().prop(inst, key), Some(Variant::Int32(_) | Variant::Int64(_)))
    });
    Some((v, int))
}

fn write(lua: &Lua, inst: Ref, key: &str, v: Value) {
    if newindex(lua, inst, key, v).is_ok() {
        signal_timing::flush_immediate(lua);
    }
}

/// One tween's step; false once it leaves the active list.
fn step_one(lua: &Lua, r: Ref, dt: f64) -> mlua::Result<bool> {
    // State bookkeeping first (no Lua runs while the state is borrowed).
    let need_starts = {
        let mut t = tweens(lua);
        let Some(tw) = t.all.get_mut(&r) else { return Ok(false) };
        match tw.state {
            State::Delayed => {
                tw.delay_elapsed += dt;
                if tw.delay_elapsed >= tw.delay {
                    tw.state = State::Playing;
                    tw.elapsed = 0.0;
                }
                return Ok(true);
            }
            State::Playing => {}
            _ => return Ok(false),
        }
        tw.starts.is_none().then(|| (tw.inst, tw.goals.iter().map(|g| g.0.clone()).collect::<Vec<_>>()))
    };
    if let Some((inst, keys)) = need_starts {
        let starts: Vec<Option<(Value, bool)>> = keys.iter().map(|k| read(lua, inst, k)).collect();
        if let Some(tw) = tweens(lua).all.get_mut(&r) {
            tw.starts = Some(starts);
        }
    }
    // (inst, writes, k, end of a repetition)
    let (inst, list, k, rep_end) = {
        let mut t = tweens(lua);
        let Some(tw) = t.all.get_mut(&r) else { return Ok(false) };
        if tw.state != State::Playing {
            return Ok(false);
        }
        tw.elapsed += dt;
        let period = tw.time * if tw.reverses { 2.0 } else { 1.0 };
        let rep_end = tw.elapsed >= period || tw.time <= 0.0;
        let k = if rep_end {
            if tw.reverses { 0.0 } else { 1.0 }
        } else {
            let a = if tw.elapsed < tw.time { tw.elapsed / tw.time } else { (period - tw.elapsed) / tw.time };
            ease(a, &tw.style, &tw.dir)
        };
        let starts = tw.starts.clone().unwrap_or_default();
        let list: Vec<(String, Value, Value, bool)> = tw
            .goals
            .iter()
            .zip(starts)
            .filter_map(|((key, goal), s)| s.map(|(sv, int)| (key.clone(), sv, goal.clone(), int)))
            .collect();
        (tw.inst, list, k, rep_end)
    };
    for (key, a, b, int) in list {
        // A repetition's end writes the end value itself.
        let v = if rep_end && !int {
            if k == 1.0 { b } else { a }
        } else {
            lerp(lua, &a, &b, k, int)?
        };
        write(lua, inst, &key, v);
    }
    if !rep_end {
        return Ok(true);
    }
    // The state moves after the writes (their Changed handlers still see
    // Playing, as in Studio), unless a handler paused / cancelled it.
    let done = {
        let mut t = tweens(lua);
        let Some(tw) = t.all.get_mut(&r) else { return Ok(false) };
        if tw.state != State::Playing {
            return Ok(tw.state == State::Delayed);
        }
        tw.rep += 1;
        if tw.repeats >= 0 && tw.rep > tw.repeats {
            tw.state = State::Completed;
            tw.me = None;
            true
        } else {
            tw.elapsed = 0.0;
            if tw.delay > 0.0 {
                tw.state = State::Delayed;
                tw.delay_elapsed = 0.0;
            }
            false
        }
    };
    if done {
        completed(lua, r, State::Completed)?;
        return Ok(false);
    }
    Ok(true)
}

/// The frame's tween step (frame hook `\0tween`): Studio's
/// updateGenericTweens.
pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    let adding = {
        let mut t = tweens(lua);
        t.steps += 1;
        // Tween instances the collector freed (not playing, unreferenced).
        if t.steps % 120 == 0 && !t.all.is_empty() {
            let w = world(lua)?;
            let d = w.d();
            t.all.retain(|r, tw| tw.me.is_some() || d.dom.get_by_ref(*r).is_some());
        }
        if t.active.is_empty() && t.to_add.is_empty() {
            return Ok(());
        }
        std::mem::take(&mut t.to_add)
    };
    // New tweens take over the properties they share with older ones.
    for r in adding {
        let cancelled: Vec<Ref> = {
            let mut t = tweens(lua);
            let Some(new) = t.all.get(&r) else { continue };
            let (inst, keys): (Ref, Vec<String>) = (new.inst, new.goals.iter().map(|g| g.0.clone()).collect());
            let active = t.active.clone();
            let mut out = Vec::new();
            for o in active {
                if o == r {
                    continue;
                }
                let Some(old) = t.all.get_mut(&o) else { continue };
                if old.inst != inst || !matches!(old.state, State::Playing | State::Delayed) {
                    continue;
                }
                let before = old.goals.len();
                let mut i = 0;
                old.goals.retain(|g| {
                    let keep = !keys.contains(&g.0);
                    if let Some(s) = old.starts.as_mut() {
                        if !keep && i < s.len() {
                            s.remove(i);
                        } else {
                            i += 1;
                        }
                    }
                    keep
                });
                if old.goals.is_empty() && before > 0 {
                    old.state = State::Cancelled;
                    old.me = None;
                    out.push(o);
                }
            }
            t.active.push(r);
            out
        };
        for o in cancelled {
            completed(lua, o, State::Cancelled)?;
        }
    }
    // Back to front; a finished tween's slot takes the last one.
    let mut i = tweens(lua).active.len();
    while i > 0 {
        i -= 1;
        let r = match tweens(lua).active.get(i) {
            Some(r) => *r,
            None => continue,
        };
        if !step_one(lua, r, dt)? {
            let mut t = tweens(lua);
            if i < t.active.len() && t.active[i] == r {
                t.active.swap_remove(i);
            } else if let Some(j) = t.active.iter().position(|x| *x == r) {
                t.active.swap_remove(j);
            }
            if let Some(tw) = t.all.get_mut(&r) {
                if tw.state != State::Paused {
                    tw.me = None;
                }
                // A paused tween keeps its Lua value until resumed or freed.
            }
        }
    }
    Ok(())
}
