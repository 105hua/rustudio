//! API-gap telemetry: every touch of a Studio API member (from the API dump)
//! that rustudio doesn't implement, with the script that touched it.
//!
//! On with `RUSTUDIO_GAPS=FILE` (or the coverage probe). Kinds:
//! - `missing`: a real member (in the dump) that indexing doesn't know.
//! - `call-unimplemented`: a known method whose call has no implementation.
//! - `default-only`: a property the engine computes at run time in Roblox
//!   (not saved in places) that rustudio only answers with its default.
//! - `write-ignored`: such a property written, with nothing acting on it.
//! - `event-unwired`: an event scripts touched that nothing in rustudio
//!   fires (not in the sources' fire calls, and not fired in the run).
//!
//! The file is JSON, rewritten on every flush (each harness shot and Stop).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use crate::api::Kind;

include!(concat!(env!("OUT_DIR"), "/engine_reads.rs"));

#[derive(Default)]
struct Entry {
    count: u64,
    scripts: HashMap<String, u64>,
}

#[derive(Default)]
struct State {
    on: bool,
    path: Option<std::path::PathBuf>,
    /// (gap kind, declaring class, member) → counts.
    gaps: HashMap<(&'static str, String, String), Entry>,
    /// Events scripts touched: (declaring class, event) → counts.
    events_used: HashMap<(String, String), Entry>,
    /// Event names rustudio fired (by name: the declaring class isn't known there).
    fired: HashSet<String>,
    /// Every touch of an API member (index or assignment) by (declaring
    /// class, member): the weights of the call-weighted coverage.
    touches: HashMap<(&'static str, &'static str), u64>,
}

/// Whether `touch` has read RUSTUDIO_GAPS yet.
static CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Mirror of `State.on` for the hottest path (every member access).
static ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut g = STATE.lock().ok()?;
    let s = g.get_or_insert_with(|| {
        let path = std::env::var_os("RUSTUDIO_GAPS").map(std::path::PathBuf::from);
        ON.store(path.is_some(), std::sync::atomic::Ordering::Relaxed);
        INIT.store(true, std::sync::atomic::Ordering::Relaxed);
        State {
            on: path.is_some(),
            path,
            ..Default::default()
        }
    });
    Some(f(s))
}

/// Telemetry is collecting (cheap check for hot paths: once the state
/// exists, `ON` mirrors it and no lock is taken).
pub fn on() -> bool {
    if INIT.load(std::sync::atomic::Ordering::Relaxed) {
        return ON.load(std::sync::atomic::Ordering::Relaxed);
    }
    with(|s| s.on).unwrap_or(false)
}

/// Whether `STATE` has been made (and `ON` mirrors it).
static INIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Turns collection on without a file (the coverage probe reads it back).
pub fn enable() {
    with(|s| s.on = true);
    ON.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Counts a touch of `class.member` (a script indexing or assigning it), if
/// the API dump has that member.
pub fn touch(class: &str, member: &str) {
    if !ON.load(std::sync::atomic::Ordering::Relaxed) {
        // (The first call reads RUSTUDIO_GAPS; after that, off costs one
        // atomic load per member access.)
        if CHECKED.load(std::sync::atomic::Ordering::Relaxed)
            || CHECKED.swap(true, std::sync::atomic::Ordering::Relaxed)
            || !on()
        {
            return;
        }
    }
    let Some(key) = crate::api::get().and_then(|a| a.lookup_names(class, member)) else {
        return;
    };
    with(|s| *s.touches.entry(key).or_insert(0) += 1);
}

/// Most stack walks per member: enough to name the scripts, cheap in loops.
const SCRIPT_SAMPLES: u64 = 64;

/// The script running now (its chunk name), from the Lua stack.
pub fn current_script(lua: &mlua::Lua) -> String {
    (0..32)
        .map_while(|level| {
            lua.inspect_stack(level, |d| {
                d.source()
                    .short_src
                    .map(|s| s.trim_start_matches(['=', '@']).to_string())
                    .unwrap_or_default()
            })
        })
        .find(|s| !s.is_empty() && !s.starts_with("rustudio.") && s != "[C]")
        .unwrap_or_else(|| "?".into())
}

/// Records a gap of `kind` for `class.member` if the API dump has that
/// member; returns whether it was recorded (false: not a real member).
pub fn gap(lua: &mlua::Lua, kind: &'static str, class: &str, member: &str) -> bool {
    if !on() {
        return false;
    }
    let Some(api) = crate::api::get() else {
        return false;
    };
    let Some((decl, _)) = api.lookup(class, member) else {
        return false;
    };
    let key = (kind, decl.to_string(), member.to_string());
    let need_script = with(|s| s.gaps.get(&key).is_none_or(|e| e.count < SCRIPT_SAMPLES)).unwrap_or(false);
    let script = need_script.then(|| current_script(lua));
    with(|s| {
        let e = s.gaps.entry(key).or_default();
        e.count += 1;
        if let Some(sc) = script {
            *e.scripts.entry(sc).or_insert(0) += 1;
        }
    });
    true
}

/// A property read answered from the reflection default: a gap when Roblox
/// works the value out at run time (not saved in places).
pub fn default_read(lua: &mlua::Lua, class: &str, member: &str) {
    if !on() {
        return;
    }
    let computed = crate::api::get()
        .and_then(|a| a.lookup(class, member))
        .is_some_and(|(_, m)| m.kind == Kind::Property && !m.can_load);
    // (A property the engine itself keeps up to date reads its default
    // honestly until something changes it.)
    if computed && !ENGINE_READS.contains(&member) {
        gap(lua, "default-only", class, member);
    }
}

/// A property written with nothing acting on it beyond storing the value: a
/// gap when Roblox reacts to it at run time (not saved in places).
pub fn plain_write(lua: &mlua::Lua, class: &str, member: &str) {
    if !on() {
        return;
    }
    let computed = crate::api::get()
        .and_then(|a| a.lookup(class, member))
        .is_some_and(|(_, m)| m.kind == Kind::Property && !m.can_load && !m.read_only);
    if computed && !ENGINE_READS.contains(&member) {
        gap(lua, "write-ignored", class, member);
    }
}

/// A script touched event `name` of `class`.
pub fn event_used(lua: &mlua::Lua, class: &str, name: &str) {
    if !on() {
        return;
    }
    let Some((decl, _)) = crate::api::get().and_then(|a| a.lookup(class, name)) else {
        return;
    };
    let key = (decl.to_string(), name.to_string());
    let need = with(|s| s.events_used.get(&key).is_none_or(|e| e.count < SCRIPT_SAMPLES)).unwrap_or(false);
    let script = need.then(|| current_script(lua));
    with(|s| {
        let e = s.events_used.entry(key).or_default();
        e.count += 1;
        if let Some(sc) = script {
            *e.scripts.entry(sc).or_insert(0) += 1;
        }
    });
}

/// rustudio fired event `name` (on any instance).
pub fn fired(name: &str) {
    if !on() {
        return;
    }
    with(|s| {
        if !s.fired.contains(name) {
            s.fired.insert(name.to_string());
        }
    });
}

fn entry_json(kind: &str, class: &str, member: &str, e: &Entry) -> serde_json::Value {
    let mut scripts: Vec<(&String, &u64)> = e.scripts.iter().collect();
    scripts.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let scripts: BTreeMap<String, u64> = scripts
        .into_iter()
        .take(8)
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    serde_json::json!({
        "kind": kind, "class": class, "member": member, "count": e.count, "scripts": scripts,
    })
}

/// The gaps so far, as JSON.
pub fn snapshot() -> serde_json::Value {
    with(|s| {
        let mut rows: Vec<serde_json::Value> = s
            .gaps
            .iter()
            .map(|((k, c, m), e)| entry_json(k, c, m, e))
            .collect();
        // Events scripts use that nothing in rustudio fires (by name: the
        // sources' fire calls, and what fired during the run).
        for ((c, m), e) in &s.events_used {
            if !s.fired.contains(m) && !ENGINE_FIRES.contains(&m.as_str()) {
                rows.push(entry_json("event-unwired", c, m, e));
            }
        }
        rows.sort_by(|a, b| {
            b["count"]
                .as_u64()
                .cmp(&a["count"].as_u64())
                .then(a["class"].as_str().cmp(&b["class"].as_str()))
                .then(a["member"].as_str().cmp(&b["member"].as_str()))
        });
        let mut fired: Vec<&String> = s.fired.iter().collect();
        fired.sort();
        let mut touches: Vec<serde_json::Value> = s
            .touches
            .iter()
            .map(|((c, m), n)| serde_json::json!({ "class": c, "member": m, "count": n }))
            .collect();
        touches.sort_by(|a, b| b["count"].as_u64().cmp(&a["count"].as_u64()));
        serde_json::json!({ "gaps": rows, "events_fired": fired, "touches": touches })
    })
    .unwrap_or_default()
}

/// Rewrites RUSTUDIO_GAPS's file with everything so far.
pub fn flush() {
    let Some(path) = with(|s| s.path.clone()).flatten() else {
        return;
    };
    if let Ok(text) = serde_json::to_string_pretty(&snapshot()) {
        let _ = std::fs::write(path, text);
    }
}
