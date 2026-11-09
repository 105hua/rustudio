//! Engine API domain: testing — the test and debugger APIs game scripts can
//! reach (Security None), measured against Roblox Studio (offline Studio, build
//! 1708278) and its program; notes in checks/parity/testing.md.
//!
//! - TestService (testing_service.rs): Check / Checkpoint / Done / Error /
//!   Fail / Message / Require / Warn print exactly what Studio prints and
//!   count into ErrorCount / TestCount / WarnCount; a Run (F8) with
//!   ExecuteWithStudioRun runs the Scripts under TestService as Studio's
//!   test run does (macro-parser warning, Done / all-finished / Timeout,
//!   "Run completed" summary, rendering tests). FunctionalTest reports
//!   through it. ServerCollectResult / ServerCollectConditionalResult fire on
//!   the server for a client's results in a Play session with
//!   ExecuteWithStudioRun and NumberOfPlayers > 0 (Studio's server + clients
//!   TestService run).
//! - The registration API (RegisterTest / RegisterSuite / ListTests /
//!   ResetTestRegistry / RunTestsAsync, TestCase:Check / Require) is behind
//!   FFlag EnableTestServiceRegistrationAPI, off in Studio: rustudio gives
//!   Studio's "Feature flag is disabled" errors. Members tagged with the
//!   InternalTest capability (StartTestSession, TakeSnapshot,
//!   TestCoordinatorService…) give Studio's capability error. The result
//!   objects (TestCase, TestResult, TestSuite, TestFailure, TestRunResult)
//!   answer from what they carry.
//! - StudioTestService: GetTestArgs / EndTest drive a headless test run
//!   (RUSTUDIO_TEST_ARGS in, EndTest stops Play and writes
//!   RUSTUDIO_TEST_RESULT); AddPlayers / CanLeaveTest / LeaveTest give
//!   Studio's errors outside a server + clients session (rustudio has none).
//! - DebuggerManager / ScriptDebugger / DebuggerBreakpoint / DebuggerWatch
//!   (testing_debug.rs): Studio's legacy script debugger, which no longer
//!   stops at its breakpoints (Studio's Luau debugger replaced it): the
//!   objects, events and errors behave as in Studio, nothing ever pauses.
//! - Studio-side services and plugin objects (testing_studio.rs):
//!   StudioService / DraggerService / Plugin read rustudio's editor settings
//!   (snapping, local space), as Studio's read its toolbar; Collaborator,
//!   LiveSyncService, PluginAction, PluginDragEvent, PluginGui,
//!   DockWidgetPluginGui, Studio.
//! - RenderingTest: RenderdocTriggerCapture prints Studio's "RenderDoc
//!   support is disabled."; the test run's rendering phase fails each test
//!   as Studio does with no image service (TestFramesCountdownAboutToStart
//!   isn't reached in Studio either).
#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};

use mlua::{Lua, MultiValue, Table, Thread, Value};
use rbx_dom_weak::types::{Ref, Variant};

use crate::runtime::*;

#[path = "testing_service.rs"]
mod service;
#[path = "testing_debug.rs"]
mod debug;
#[path = "testing_studio.rs"]
pub mod studio;

pub use service::{report_test_run, take_end_request};
pub use studio::{set_editor_state, EditorState};

pub const METHODS: &[(&str, &str)] = &[
    // TestService
    ("Check", "TestService"),
    ("Checkpoint", "TestService"),
    ("Done", "TestService"),
    ("Error", "TestService"),
    ("Fail", "TestService"),
    ("Message", "TestService"),
    ("Require", "TestService"),
    ("Warn", "TestService"),
    ("GetTestControlSchema", "TestService"),
    ("GetTestControls", "TestService"),
    ("ListTests", "TestService"),
    ("RegisterSuite", "TestService"),
    ("RegisterTest", "TestService"),
    ("RegisterTestLegacy", "TestService"),
    ("ResetTestControl", "TestService"),
    ("ResetTestRegistry", "TestService"),
    ("ScopeTime", "TestService"),
    ("SetTestControl", "TestService"),
    ("SignalProfilingCapture", "TestService"),
    ("SignalProfilingStart", "TestService"),
    ("SignalProfilingStop", "TestService"),
    ("StartTestSession", "TestService"),
    ("StopTestSession", "TestService"),
    ("TakeSnapshot", "TestService"),
    ("getTestSessionProviderStats", "TestService"),
    ("isFeatureEnabled", "TestService"),
    ("CaptureScreenshotAsync", "TestService"),
    ("FetchTestControlsAsync", "TestService"),
    ("RequestValidationAsync", "TestService"),
    ("RunTestsAsync", "TestService"),
    ("StartAudioCaptureAsync", "TestService"),
    ("StartVideoCaptureAsync", "TestService"),
    ("StopAudioCaptureAsync", "TestService"),
    ("StopVideoCaptureAsync", "TestService"),
    // registration objects
    ("Assert", "TestCase"),
    ("AssertLegacy", "TestCase"),
    ("Check", "TestCase"),
    ("EndTest", "TestCase"),
    ("GetTestTags", "TestCase"),
    ("Message", "TestCase"),
    ("Require", "TestCase"),
    ("RequireLegacy", "TestCase"),
    ("GetFailures", "TestResult"),
    ("GetTestTags", "TestResult"),
    ("GetTestTags", "TestSuite"),
    ("RegisterTest", "TestSuite"),
    ("GetActual", "TestFailure"),
    ("GetContext", "TestFailure"),
    ("GetExpected", "TestFailure"),
    ("GetSource", "TestFailure"),
    ("GetTests", "TestRunResult"),
    ("Skip", "TestExecutionContext"),
    // FunctionalTest
    ("Error", "FunctionalTest"),
    ("Failed", "FunctionalTest"),
    ("Pass", "FunctionalTest"),
    ("Passed", "FunctionalTest"),
    ("Warn", "FunctionalTest"),
    // StudioTestService
    ("AddPlayers", "StudioTestService"),
    ("CanLeaveTest", "StudioTestService"),
    ("EndTest", "StudioTestService"),
    ("GetTestArgs", "StudioTestService"),
    ("LeaveTest", "StudioTestService"),
    // TestCoordinatorService
    ("Participant", "TestCoordinatorService"),
    ("Signal", "TestCoordinatorService"),
    ("AwaitAllAsync", "TestCoordinatorService"),
    ("AwaitAnyAsync", "TestCoordinatorService"),
    ("AwaitSignalAsync", "TestCoordinatorService"),
    ("BarrierAsync", "TestCoordinatorService"),
    // RenderingTest
    ("RenderdocTriggerCapture", "RenderingTest"),
    // debugger
    ("AddWatch", "ScriptDebugger"),
    ("GetBreakpoints", "ScriptDebugger"),
    ("GetGlobals", "ScriptDebugger"),
    ("GetLocals", "ScriptDebugger"),
    ("GetStack", "ScriptDebugger"),
    ("GetUpvalues", "ScriptDebugger"),
    ("GetWatchValue", "ScriptDebugger"),
    ("GetWatches", "ScriptDebugger"),
    ("SetBreakpoint", "ScriptDebugger"),
    ("SetGlobal", "ScriptDebugger"),
    ("SetLocal", "ScriptDebugger"),
    ("SetUpvalue", "ScriptDebugger"),
    ("AddDebugger", "DebuggerManager"),
    ("GetDebuggers", "DebuggerManager"),
    ("Resume", "DebuggerManager"),
    ("StepIn", "DebuggerManager"),
    ("StepOut", "DebuggerManager"),
    ("StepOver", "DebuggerManager"),
    // Studio side
    ("GetSyncState", "LiveSyncService"),
    ("BindToClose", "PluginGui"),
];

/// Events this domain fires. TestRegistryChanged (registration is off),
/// ScriptDebugger.EncounteredBreak / Resuming (nothing pauses),
/// RenderingTest.TestFramesCountdownAboutToStart (Studio's rendering run
/// fails before it) and LiveSyncService.SyncStatusChanged (no live sync)
/// never fire in Studio either, so they stay plain signals.
pub const EVENTS: &[(&str, &str)] = &[
    ("ServerCollectConditionalResult", "TestService"),
    ("ServerCollectResult", "TestService"),
    ("BreakpointAdded", "ScriptDebugger"),
    ("BreakpointRemoved", "ScriptDebugger"),
    ("WatchAdded", "ScriptDebugger"),
    ("WatchRemoved", "ScriptDebugger"),
    ("DebuggerAdded", "DebuggerManager"),
    ("DebuggerRemoved", "DebuggerManager"),
];

/// Classes this domain answers for (calls, reads, writes).
const CLASSES: &[&str] = &[
    "TestService", "TestCase", "TestResult", "TestSuite", "TestFailure", "TestRunResult",
    "TestExecutionContext", "FunctionalTest", "StudioTestService", "TestCoordinatorService",
    "RenderingTest", "ScriptDebugger", "DebuggerManager", "DebuggerBreakpoint", "DebuggerWatch",
    "StudioService", "DraggerService", "Collaborator", "LiveSyncService", "Studio", "Plugin",
    "PluginAction", "PluginDragEvent", "PluginGui", "DockWidgetPluginGui", "QWidgetPluginGui",
];

fn mine(class: &str) -> bool {
    CLASSES.contains(&class)
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let class = match inst_class(lua, this) {
        Ok((c, _)) => c.to_string(),
        Err(_) => return None,
    };
    if !mine(&class) && !crate::doc::is_a(&class, "PluginGui") {
        return None;
    }
    if let Some(r) = service::call(lua, &class, name, this, args) {
        return Some(r);
    }
    if let Some(r) = debug::call(lua, &class, name, this, args) {
        return Some(r);
    }
    studio::call(lua, &class, name, this, args)
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if !mine(class) && !crate::doc::is_a(class, "PluginGui") {
        return Ok(None);
    }
    if let Some(v) = service::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    if let Some(v) = debug::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    studio::prop_get(lua, r, class, key)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if !mine(class) && !crate::doc::is_a(class, "PluginGui") {
        return Ok(false);
    }
    if service::newindex(lua, r, class, key, value)? {
        return Ok(true);
    }
    if debug::newindex(lua, r, class, key, value)? {
        return Ok(true);
    }
    studio::newindex(lua, r, class, key, value)
}

pub fn write_acts(class: &str, key: &str) -> bool {
    // TestService.Timeout / ExecuteWithStudioRun / NumberOfPlayers steer the
    // test run.
    class == "TestService" && matches!(key, "Timeout" | "ExecuteWithStudioRun" | "NumberOfPlayers")
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    service::install(lua)?;
    debug::install(lua)?;
    Ok(())
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    service::step(lua, dt)?;
    debug::step(lua, dt)?;
    Ok(())
}

// ------------------------------------------------------------- helpers

fn ret1(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([v]))
}

fn ret0() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn lstr(lua: &Lua, s: &str) -> mlua::Result<Value> {
    Ok(Value::String(lua.create_string(s)?))
}

fn out(lua: &Lua, level: Level, text: impl Into<String>) -> mlua::Result<()> {
    world(lua)?.output.push((level, text.into()));
    Ok(())
}

/// Studio's error for a member tagged with a capability scripts don't have.
fn capability_err(name: &str, cap: &str) -> mlua::Error {
    rt_err(format!("The current thread cannot call '{name}' (lacking capability {cap})"))
}

fn enum_item(lua: &Lua, en: &str, value: u32) -> mlua::Result<Value> {
    match crate::ltypes::LEnumItem::from_value(en, value) {
        Some(item) => crate::ltypes::enum_value(lua, item),
        None => Ok(Value::Number(value as f64)),
    }
}

/// A saved property of the instance, if the place has one.
fn doc_prop(lua: &Lua, r: Ref, key: &str) -> mlua::Result<Option<Variant>> {
    Ok(world(lua)?.d().prop(r, key))
}

fn prop_f64(lua: &Lua, r: Ref, key: &str, dflt: f64) -> mlua::Result<f64> {
    Ok(match doc_prop(lua, r, key)? {
        Some(Variant::Float64(f)) => f,
        Some(Variant::Float32(f)) => f as f64,
        Some(Variant::Int32(i)) => i as f64,
        Some(Variant::Int64(i)) => i as f64,
        _ => dflt,
    })
}

fn prop_bool(lua: &Lua, r: Ref, key: &str, dflt: bool) -> mlua::Result<bool> {
    Ok(match doc_prop(lua, r, key)? {
        Some(Variant::Bool(b)) => b,
        _ => dflt,
    })
}

fn prop_string(lua: &Lua, r: Ref, key: &str) -> mlua::Result<Option<String>> {
    Ok(match doc_prop(lua, r, key)? {
        Some(Variant::String(s)) => Some(s),
        Some(Variant::BinaryString(b)) => Some(String::from_utf8_lossy(b.as_ref()).into_owned()),
        _ => None,
    })
}

/// Per-instance values this domain keeps for an object (written by the
/// engine side or by scripts), in a registry table keyed by the instance.
fn store(lua: &Lua, r: Ref) -> mlua::Result<Table> {
    let all: Table = match lua.named_registry_value::<Table>("rs_testing_store") {
        Ok(t) => t,
        Err(_) => {
            let t = lua.create_table()?;
            lua.set_named_registry_value("rs_testing_store", t.clone())?;
            t
        }
    };
    let key = format!("{r:?}");
    if let Ok(t) = all.get::<Table>(key.as_str()) {
        return Ok(t);
    }
    let t = lua.create_table()?;
    all.set(key, t.clone())?;
    Ok(t)
}

fn stored(lua: &Lua, r: Ref, key: &str) -> mlua::Result<Value> {
    store(lua, r)?.get(key)
}

/// Studio's "%g" for a number of seconds (3 → "3", 2.5 → "2.5").
fn fmt_g(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// A Luau number as tostring prints it.
fn num_str(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Whether `r` is in the DataModel (game is an ancestor).
fn in_game(lua: &Lua, r: Ref) -> mlua::Result<bool> {
    let w = world(lua)?;
    let d = w.d();
    let root = d.dom.root_ref();
    Ok(r == root || d.is_ancestor(root, r))
}

fn service_ref(lua: &Lua, class: &str) -> mlua::Result<Option<Ref>> {
    let w = world(lua)?;
    let d = w.d();
    Ok(d.find_child(d.dom.root_ref(), class))
}

fn new_object(lua: &Lua, class: &str, name: &str, parent: Ref) -> mlua::Result<Ref> {
    let mut w = world(lua)?;
    let r = w
        .dm()
        .dom
        .insert(parent, rbx_dom_weak::InstanceBuilder::new(class).with_name(name));
    if parent.is_some() {
        w.dm().mark_tree(r);
        w.structure_changed = true;
    }
    Ok(r)
}

fn name_of(lua: &Lua, r: Ref) -> mlua::Result<String> {
    Ok(world(lua)?
        .d()
        .dom
        .get_by_ref(r)
        .map(|i| i.name.clone())
        .unwrap_or_default())
}
