//! TestService, FunctionalTest, the registration result objects,
//! StudioTestService, TestCoordinatorService and RenderingTest (see
//! testing.rs for what Studio does and where it was measured).

use std::sync::Mutex;

use mlua::{Function, Lua, MultiValue, Table, Thread, Value};
use mlua::thread::ThreadStatus;
use rbx_dom_weak::types::{Ref, Variant};

use super::*;

// ----------------------------------------------------------------- state

#[derive(Default)]
struct SvcState {
    errors: i64,
    tests: i64,
    warns: i64,
    /// Whether this session has looked at starting a TestService run.
    run_checked: bool,
    run: Option<Run>,
}

struct Run {
    svc: Ref,
    start: f64,
    timeout: f64,
    threads: Vec<Thread>,
    /// The summary (or the timeout) has been printed.
    complete: bool,
    /// Frames since completion; the rendering phase runs on the next one.
    rendering_done: bool,
}

fn st(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, SvcState>> {
    if lua.app_data_ref::<SvcState>().is_none() {
        lua.set_app_data(SvcState::default());
    }
    lua.app_data_mut::<SvcState>()
        .ok_or_else(|| rt_err("testing state missing"))
}

/// A client's TestService result on its way to the server (Studio's
/// server + clients TestService run).
struct Collected {
    condition: Option<bool>,
    text: String,
    line: i64,
}

static COLLECTED: Mutex<Vec<Collected>> = Mutex::new(Vec::new());

/// StudioTestService:EndTest's value (JSON) once a script ended the test.
static END_REQUEST: Mutex<Option<String>> = Mutex::new(None);

/// The play session polls this each frame: Some(value JSON) once a server
/// script called StudioTestService:EndTest (the session then stops, as
/// Studio's does).
pub fn take_end_request() -> Option<String> {
    END_REQUEST.lock().ok().and_then(|mut e| e.take())
}

/// After an EndTest stop: with RUSTUDIO_TEST_RESULT=FILE (a headless test
/// run driven by an agent), writes {"value": EndTest's value, "output":
/// [[level, text]…]} there and exits.
pub fn report_test_run(value: &str, log: &[crate::app::LogLine]) {
    let Some(path) = std::env::var_os("RUSTUDIO_TEST_RESULT") else {
        return;
    };
    let value: serde_json::Value =
        serde_json::from_str(value).unwrap_or(serde_json::Value::Null);
    let output: Vec<serde_json::Value> = log
        .iter()
        .map(|l| {
            let lv = match l.level {
                Level::Info => "info",
                Level::Warn => "warning",
                Level::Error => "error",
            };
            serde_json::json!([lv, l.text])
        })
        .collect();
    let j = serde_json::json!({ "ended": "EndTest", "value": value, "output": output });
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&j).unwrap_or_default());
    std::process::exit(0);
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(SvcState::default());
    if world(lua).is_ok_and(|w| w.side == Side::Server) {
        if let Ok(mut c) = COLLECTED.lock() {
            c.clear();
        }
        if let Ok(mut e) = END_REQUEST.lock() {
            *e = None;
        }
    }
    Ok(())
}

// --------------------------------------------------------------- reading

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    match (class, key) {
        // Enabled is true only in a UTP launcher / RCC team-test session,
        // never in Studio's Play or Run.
        ("TestService", "Enabled") => Ok(Some(Value::Boolean(false))),
        ("TestService", "ErrorCount" | "TestCount" | "WarnCount") => {
            let s = st(lua)?;
            let n = match key {
                "ErrorCount" => s.errors,
                "TestCount" => s.tests,
                _ => s.warns,
            };
            Ok(Some(Value::Number(n as f64)))
        }
        // What a registration result object carries (set by the runner;
        // Studio's defaults otherwise).
        ("TestCase", "SuiteName" | "TestId" | "TestName")
        | ("TestResult", "SkipReason" | "SuiteName" | "TestId" | "TestName" | "UncaughtError")
        | ("TestSuite", "SuiteName")
        | ("TestFailure", "Matcher" | "Message" | "StackTrace")
        | ("TestRunResult", "Summary") => Ok(Some(match stored(lua, r, key)? {
            Value::Nil => lstr(lua, "")?,
            v => v,
        })),
        ("TestCase" | "TestSuite", "TimeoutSec")
        | ("TestResult", "Duration" | "FailedChecks" | "PassedChecks")
        | ("TestFailure", "Line")
        | ("TestRunResult", "Duration" | "Failed" | "Passed" | "Skipped" | "Total") => {
            Ok(Some(match stored(lua, r, key)? {
                Value::Nil => Value::Number(0.0),
                v => v,
            }))
        }
        ("TestResult", "Status") => {
            let v = match stored(lua, r, key)? {
                Value::Number(n) => n as u32,
                Value::Integer(n) => n as u32,
                _ => 0,
            };
            Ok(Some(enum_item(lua, "TestCaseStatus", v)?))
        }
        ("TestRunResult", "Status") => {
            // Nothing ran: NoTests.
            let v = match stored(lua, r, key)? {
                Value::Number(n) => n as u32,
                Value::Integer(n) => n as u32,
                _ => 2,
            };
            Ok(Some(enum_item(lua, "TestRunStatus", v)?))
        }
        _ => Ok(None),
    }
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    Ok(false)
}

// ----------------------------------------------------------------- calls

pub fn call(
    lua: &Lua,
    class: &str,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    let r = match class {
        "TestService" => test_service(lua, name, this, args)?,
        "FunctionalTest" => functional_test(lua, name, this, args)?,
        "TestCase" | "TestResult" | "TestSuite" | "TestFailure" | "TestRunResult"
        | "TestExecutionContext" => result_object(lua, class, name, this, args)?,
        "StudioTestService" => studio_test_service(lua, name, this, args)?,
        "TestCoordinatorService" => match name {
            "Participant" | "Signal" | "AwaitAllAsync" | "AwaitAnyAsync" | "AwaitSignalAsync"
            | "BarrierAsync" => Err(capability_err(name, "InternalTest")),
            _ => return None,
        },
        "RenderingTest" if name == "RenderdocTriggerCapture" => {
            // Studio (no RenderDoc in the build) prints this and returns.
            out(lua, Level::Error, "RenderDoc support is disabled.").and_then(|_| ret0())
        }
        _ => return None,
    };
    Some(r)
}

/// TestService's (condition, description, source, line) arguments.
struct Report {
    condition: bool,
    text: String,
    source: Option<Ref>,
    line: i64,
}

/// A string argument as Studio converts it: numbers print as numbers, a
/// table warns ("argument #N expects a string…") and reads "{}".
fn text_arg(lua: &Lua, args: &MultiValue, i: usize) -> mlua::Result<String> {
    match arg(args, i) {
        Value::Nil => Err(rt_err(format!("Argument {i} missing or nil"))),
        Value::String(s) => Ok(s.to_string_lossy()),
        Value::Number(n) => Ok(num_str(n)),
        Value::Integer(n) => Ok(n.to_string()),
        Value::Boolean(b) => Ok(b.to_string()),
        v => {
            out(
                lua,
                Level::Warn,
                format!("argument #{i} expects a string, but {} was passed", v.type_name()),
            )?;
            Ok("{}".into())
        }
    }
}

fn source_line(args: &MultiValue, src: usize) -> (Option<Ref>, i64) {
    let source = opt_inst(&arg(args, src));
    let line = match arg(args, src + 1) {
        Value::Number(n) => n as i64,
        Value::Integer(n) => n as i64,
        _ => 0,
    };
    (source, line)
}

fn conditional(lua: &Lua, args: &MultiValue) -> mlua::Result<Report> {
    let condition = match arg(args, 1) {
        Value::Nil => return Err(rt_err("Argument 1 missing or nil")),
        Value::Boolean(b) => b,
        _ => true,
    };
    let text = text_arg(lua, args, 2)?;
    let (source, line) = source_line(args, 3);
    Ok(Report { condition, text, source, line })
}

fn plain(lua: &Lua, args: &MultiValue) -> mlua::Result<Report> {
    let text = text_arg(lua, args, 1)?;
    let (source, line) = source_line(args, 2);
    Ok(Report { condition: true, text, source, line })
}

/// "TestService: text", or "TestService.Source(line): text" with a source.
fn line_text(lua: &Lua, rep: &Report, prefix: &str) -> mlua::Result<String> {
    Ok(match rep.source {
        Some(s) => format!("TestService.{}({}): {prefix}{}", name_of(lua, s)?, rep.line, rep.text),
        None => format!("TestService: {prefix}{}", rep.text),
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Message,
    Checkpoint,
    Check,
    Warn,
    Require,
    Error,
    Fail,
}

/// One TestService report: prints as Studio does, counts, and forwards a
/// client's result to the server in a server + clients run.
fn report(lua: &Lua, kind: Kind, rep: Report) -> mlua::Result<()> {
    let (level, prefix, counted, failed) = match kind {
        Kind::Message => (Some(Level::Info), "", false, false),
        Kind::Checkpoint => (Some(Level::Info), "checkpoint ", false, false),
        Kind::Check if rep.condition => (None, "", true, false),
        Kind::Check => (Some(Level::Error), "", true, true),
        Kind::Warn if rep.condition => (None, "", true, false),
        Kind::Warn => (Some(Level::Warn), "", true, false),
        Kind::Require if rep.condition => (None, "", true, false),
        Kind::Require | Kind::Fail => (Some(Level::Error), "fatal ", true, true),
        Kind::Error => (Some(Level::Error), "", true, true),
    };
    if counted {
        let mut s = st(lua)?;
        s.tests += 1;
        if failed {
            s.errors += 1;
        }
        if kind == Kind::Warn && !rep.condition {
            s.warns += 1;
        }
    }
    let text = line_text(lua, &rep, prefix)?;
    if let Some(level) = level {
        out(lua, level, text)?;
    }
    forward_to_server(lua, kind, &rep, prefix)
}

/// The TestService's settings that make a run: (ExecuteWithStudioRun,
/// NumberOfPlayers, Timeout).
fn run_settings(lua: &Lua, svc: Ref) -> mlua::Result<(bool, i64, f64)> {
    Ok((
        prop_bool(lua, svc, "ExecuteWithStudioRun", false)?,
        prop_f64(lua, svc, "NumberOfPlayers", 0.0)? as i64,
        prop_f64(lua, svc, "Timeout", 10.0)?,
    ))
}

fn forward_to_server(lua: &Lua, kind: Kind, rep: &Report, prefix: &str) -> mlua::Result<()> {
    let (client, peer) = {
        let w = world(lua)?;
        (w.side == Side::Client, w.has_peer)
    };
    if !client || !peer {
        return Ok(());
    }
    let Some(svc) = service_ref(lua, "TestService")? else {
        return Ok(());
    };
    let (studio_run, players, _) = run_settings(lua, svc)?;
    if !studio_run || players <= 0 {
        return Ok(());
    }
    let condition = match kind {
        Kind::Message | Kind::Checkpoint => None,
        Kind::Error | Kind::Fail => Some(false),
        _ => Some(rep.condition),
    };
    if let Ok(mut c) = COLLECTED.lock() {
        c.push(Collected {
            condition,
            text: format!("{prefix}{}", rep.text),
            line: rep.line,
        });
    }
    Ok(())
}

fn flag_disabled(what: &str) -> mlua::Error {
    rt_err(format!("{what} is not available. Feature flag is disabled."))
}

/// Luau type names as Studio's argument casts say them.
fn cast_name(v: &Value) -> &'static str {
    match v {
        Value::Table(t) => {
            if t.raw_len() > 0 || t.clone().pairs::<Value, Value>().next().is_none() {
                "Array"
            } else {
                "Dictionary"
            }
        }
        Value::Function(_) => "Function",
        Value::String(_) => "string",
        Value::Number(_) | Value::Integer(_) => "double",
        Value::Boolean(_) => "bool",
        Value::UserData(_) => "Instance",
        _ => "nil",
    }
}

fn test_service(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let r = match name {
        "Message" => plain(lua, args).and_then(|rep| report(lua, Kind::Message, rep)).and_then(|_| ret0()),
        "Checkpoint" => plain(lua, args).and_then(|rep| report(lua, Kind::Checkpoint, rep)).and_then(|_| ret0()),
        "Check" | "Warn" | "Require" => conditional(lua, args)
            .and_then(|rep| {
                let k = match name {
                    "Check" => Kind::Check,
                    "Warn" => Kind::Warn,
                    _ => Kind::Require,
                };
                report(lua, k, rep)
            })
            .and_then(|_| ret0()),
        "Error" | "Fail" => plain(lua, args)
            .and_then(|rep| report(lua, if name == "Error" { Kind::Error } else { Kind::Fail }, rep))
            .and_then(|_| ret0()),
        "Done" => done(lua).and_then(|_| ret0()),
        // FFlag EnableTestServiceRegistrationAPI is off in Studio. The
        // argument casts come first (measured): RegisterTest(name,
        // options, callback) in this build.
        "RegisterTest" => (|| {
            let opts = arg(args, 2);
            if !matches!(opts, Value::Nil | Value::Table(_)) {
                return Err(rt_err(format!("Unable to cast {} to Dictionary", cast_name(&opts))));
            }
            let cb = arg(args, 3);
            if !matches!(cb, Value::Nil | Value::Function(_)) {
                return Err(rt_err(format!("Unable to cast {} to Function", cast_name(&cb))));
            }
            Err(flag_disabled("TestService test registration"))
        })(),
        "RegisterSuite" | "ListTests" | "ResetTestRegistry" | "RunTestsAsync" => {
            Err(flag_disabled(&format!("TestService:{name}")))
        }
        // An explicit error in Studio's implementation (the member exists).
        "ScopeTime" => Err(rt_err("ScopeTime is not a valid member of TestService")),
        "isFeatureEnabled" => {
            let flag = match arg(args, 1) {
                Value::String(s) => s.to_string_lossy(),
                Value::Nil => return Some(Err(rt_err("Argument 1 missing or nil"))),
                v => num_str(match v {
                    Value::Number(n) => n,
                    Value::Integer(n) => n as f64,
                    _ => 0.0,
                }),
            };
            Err(rt_err(format!("Flag {flag} referenced from Lua isn't defined in C++ code")))
        }
        "GetTestControlSchema" | "GetTestControls" | "RegisterTestLegacy" | "ResetTestControl"
        | "SetTestControl" | "SignalProfilingCapture" | "SignalProfilingStart"
        | "SignalProfilingStop" | "StartTestSession" | "StopTestSession" | "TakeSnapshot"
        | "getTestSessionProviderStats" | "CaptureScreenshotAsync" | "FetchTestControlsAsync"
        | "RequestValidationAsync" | "StartAudioCaptureAsync" | "StartVideoCaptureAsync"
        | "StopAudioCaptureAsync" | "StopVideoCaptureAsync" => {
            Err(capability_err(name, "InternalTest"))
        }
        _ => return None,
    };
    Some(r)
}

fn functional_test(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let kind = match name {
        "Pass" | "Passed" => (Kind::Check, true),
        "Warn" => (Kind::Warn, false),
        "Error" | "Failed" => (Kind::Check, false),
        _ => return None,
    };
    Some((|| {
        // It reports through the place's TestService (none outside the
        // DataModel).
        if !in_game(lua, this)? || service_ref(lua, "TestService")?.is_none() {
            return Err(rt_err("Can't find TestService"));
        }
        let text = match arg(args, 1) {
            Value::Nil => String::new(),
            _ => text_arg(lua, args, 1)?,
        };
        report(lua, kind.0, Report { condition: kind.1, text, source: None, line: 0 })?;
        ret0()
    })())
}

fn result_object(
    lua: &Lua,
    class: &str,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    let list = |key: &str| -> mlua::Result<MultiValue> {
        // A copy of the array the object carries.
        let t = lua.create_table()?;
        if let Value::Table(src) = stored(lua, this, key)? {
            for (i, v) in src.sequence_values::<Value>().enumerate() {
                t.raw_set(i + 1, v?)?;
            }
        }
        ret1(Value::Table(t))
    };
    let r = match (class, name) {
        ("TestCase", "Assert" | "AssertLegacy" | "EndTest" | "Message" | "RequireLegacy") => {
            Err(capability_err(name, "InternalTest"))
        }
        ("TestCase", "Check" | "Require") => {
            // Studio's order: the arguments, then the flag.
            let n = args.len().saturating_sub(1);
            if n < 2 {
                Err(rt_err(format!("TestCase:{name} requires an actualValue and an expectedValue.")))
            } else if !matches!(arg(args, 3), Value::Nil | Value::String(_)) {
                Err(rt_err(format!(
                    "TestCase:{name}: message must be a string or nil, got {}.",
                    arg(args, 3).type_name()
                )))
            } else {
                Err(flag_disabled(&format!("TestCase:{name}")))
            }
        }
        (_, "GetTestTags") => list("Tags"),
        ("TestResult", "GetFailures") => list("Failures"),
        ("TestRunResult", "GetTests") => list("Tests"),
        ("TestSuite", "RegisterTest") => Err(flag_disabled("TestService test registration")),
        ("TestFailure", "GetActual" | "GetExpected" | "GetContext" | "GetSource") => {
            let key = &name[3..];
            match stored(lua, this, key) {
                Ok(Value::Nil) if key == "Context" => lua.create_table().map(Value::Table).and_then(ret1),
                Ok(v) => ret1(v),
                Err(e) => Err(e),
            }
        }
        ("TestExecutionContext", "Skip") => {
            // A context exists only inside a registered test's callback
            // (registration is off in Studio): Skip ends the test as skipped.
            let reason = match arg(args, 1) {
                Value::Nil => String::new(),
                _ => match text_arg(lua, args, 1) {
                    Ok(s) => s,
                    Err(e) => return Some(Err(e)),
                },
            };
            Err(rt_err(if reason.is_empty() {
                "Test skipped.".to_string()
            } else {
                format!("Test skipped: {reason}")
            }))
        }
        _ => return None,
    };
    Some(r)
}

// ------------------------------------------------------- StudioTestService

fn json_fn(lua: &Lua, which: &str) -> mlua::Result<Function> {
    let key = format!("rs_testing_json_{which}");
    if let Ok(f) = lua.named_registry_value::<Function>(&key) {
        return Ok(f);
    }
    let src = if which == "encode" {
        "return function(v) local hs = game:GetService('HttpService') \
         local ok, s = pcall(hs.JSONEncode, hs, v) \
         if ok then return s end return hs:JSONEncode(tostring(v)) end"
    } else {
        "return function(s) local hs = game:GetService('HttpService') return hs:JSONDecode(s) end"
    };
    let f: Function = lua.load(src).set_name("=StudioTestService").eval()?;
    lua.set_named_registry_value(&key, f.clone())?;
    Ok(f)
}

fn studio_test_service(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    let side = world(lua).map(|w| w.side).unwrap_or(Side::Server);
    let r = match name {
        // A test run's arguments (ExecutePlayModeAsync / ExecuteRunModeAsync
        // in Studio; RUSTUDIO_TEST_ARGS, JSON, for a headless rustudio run);
        // nil in a plain Play, as Studio's.
        "GetTestArgs" => match std::env::var("RUSTUDIO_TEST_ARGS") {
            Ok(s) if !s.trim().is_empty() => json_fn(lua, "decode")
                .and_then(|f| f.call::<Value>(s))
                .and_then(ret1),
            _ => ret1(Value::Nil),
        },
        "EndTest" => {
            if side != Side::Server {
                Err(rt_err(
                    "EndTest: can only be called from the server DataModel of a running Studio play session",
                ))
            } else {
                (|| {
                    let s: String = json_fn(lua, "encode")?.call(arg(args, 1))?;
                    if let Ok(mut e) = END_REQUEST.lock() {
                        *e = Some(s);
                    }
                    ret0()
                })()
            }
        }
        // Only in a server + clients test session, which rustudio doesn't run.
        "AddPlayers" => Err(rt_err(
            "AddPlayers: can only be called from the server DataModel of a running Studio test session",
        )),
        "CanLeaveTest" | "LeaveTest" => Err(rt_err(
            "CanLeaveTest/LeaveTest: can only be called from the client DataModel of a running Studio test session",
        )),
        _ => return None,
    };
    Some(r)
}

// ------------------------------------------------------------- the run

fn done(lua: &Lua) -> mlua::Result<()> {
    let active = st(lua)?.run.as_ref().is_some_and(|r| !r.complete);
    if active {
        complete(lua)?;
    }
    Ok(())
}

fn complete(lua: &Lua) -> mlua::Result<()> {
    let (t, w, e) = {
        let mut s = st(lua)?;
        if let Some(run) = s.run.as_mut() {
            run.complete = true;
        }
        (s.tests, s.warns, s.errors)
    };
    out(
        lua,
        Level::Info,
        format!("TestService: Run completed, tests: {t}, warns: {w}, errors: {e}"),
    )
}

/// Starts Studio's TestService run when a Run (F8) begins with
/// ExecuteWithStudioRun: each enabled Script under TestService runs (after
/// Studio's warning that the old macro parser is gone); the run completes
/// at Done, when every test script has finished, or fails at Timeout.
fn begin_run(lua: &Lua) -> mlua::Result<()> {
    let (mode, side) = {
        let w = world(lua)?;
        (w.mode, w.side)
    };
    if mode != RunMode::Run || side != Side::Server {
        return Ok(());
    }
    let Some(svc) = service_ref(lua, "TestService")? else {
        return Ok(());
    };
    let (studio_run, _, timeout) = run_settings(lua, svc)?;
    if !studio_run {
        return Ok(());
    }
    let scripts: Vec<Ref> = {
        let w = world(lua)?;
        let d = w.d();
        d.dom
            .get_by_ref(svc)
            .map(|i| i.children().to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|r| {
                d.class_of(*r).is_some_and(|c| c == "Script")
                    && !matches!(d.prop(*r, "Disabled"), Some(Variant::Bool(true)))
                    && !matches!(d.prop(*r, "Enabled"), Some(Variant::Bool(false)))
            })
            .collect()
    };
    if scripts.is_empty() {
        return out(lua, Level::Info, "TestService: Doesn't include any assertions");
    }
    let start = world(lua)?.time;
    {
        let mut s = st(lua)?;
        s.errors = 0;
        s.tests = 0;
        s.warns = 0;
        s.run = Some(Run {
            svc,
            start,
            timeout,
            threads: Vec::new(),
            complete: false,
            rendering_done: false,
        });
    }
    let shared: Table = lua.named_registry_value("rustudio_shared")?;
    for r in scripts {
        out(lua, Level::Warn, "TestService macro parser is not supported anymore.")?;
        if !world(lua)?.started.insert(r) {
            continue;
        }
        match compile_script(lua, r, &shared).and_then(|f| lua.create_thread(f)) {
            Ok(th) => {
                resume(lua, &th, vec![]);
                if let Some(run) = st(lua)?.run.as_mut() {
                    run.threads.push(th);
                }
            }
            Err(e) => report_error(lua, &e),
        }
        check_finished(lua)?;
    }
    Ok(())
}

fn report_error(lua: &Lua, e: &mlua::Error) {
    crate::runtime::report(lua, e);
}

fn finished(th: &Thread) -> bool {
    matches!(th.status(), ThreadStatus::Finished | ThreadStatus::Error)
}

/// Completes the run once no test script is running any more.
fn check_finished(lua: &Lua) -> mlua::Result<()> {
    let all = st(lua)?
        .run
        .as_ref()
        .is_some_and(|r| !r.complete && r.threads.iter().all(finished));
    if all {
        complete(lua)?;
    }
    Ok(())
}

/// After the run: Studio runs the RenderingTests under TestService against
/// its image service (127.0.0.1:8001, the internal rendering harness), which
/// isn't there, so each fails as Studio's do without it.
fn rendering_phase(lua: &Lua, svc: Ref) -> mlua::Result<()> {
    let tests: Vec<(Ref, String)> = {
        let w = world(lua)?;
        let d = w.d();
        d.dom
            .descendants_of(svc)
            .filter(|i| i.class == "RenderingTest")
            .filter(|i| !matches!(d.prop(i.referent(), "ShouldSkip"), Some(Variant::Bool(true))))
            .map(|i| (i.referent(), i.name.clone()))
            .collect()
    };
    for (_, name) in &tests {
        out(lua, Level::Info, format!("Run rendering test : '{name}'"))?;
        out(
            lua,
            Level::Error,
            format!("Rendering test '{name}' failed. Exception: HttpError: ConnectionClosed"),
        )?;
        report(lua, Kind::Fail, Report { condition: false, text: name.clone(), source: None, line: 0 })?;
    }
    let n = tests.len();
    out(
        lua,
        if n == 0 { Level::Info } else { Level::Error },
        format!("Rendering tests finished. 0 of {n} has passed."),
    )?;
    if n > 0 {
        report(
            lua,
            Kind::Fail,
            Report {
                condition: false,
                text: "Timeout waiting for alltestfinished reply from server".into(),
                source: None,
                line: 0,
            },
        )?;
    }
    Ok(())
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    let first = {
        let mut s = st(lua)?;
        !std::mem::replace(&mut s.run_checked, true)
    };
    if first {
        begin_run(lua)?;
    }
    // The run: finished scripts, the timeout, then the rendering phase.
    let state = st(lua)?.run.as_ref().map(|r| (r.complete, r.rendering_done, r.start, r.timeout, r.svc));
    if let Some((complete, rendering_done, start, timeout, svc)) = state {
        if !complete {
            check_finished(lua)?;
            let still = st(lua)?.run.as_ref().is_some_and(|r| !r.complete);
            if still && world(lua)?.time - start >= timeout {
                if let Some(run) = st(lua)?.run.as_mut() {
                    run.complete = true;
                }
                out(
                    lua,
                    Level::Error,
                    format!("TestService: Tests failed to complete in {} seconds", fmt_g(timeout)),
                )?;
            }
        } else if !rendering_done {
            if let Some(run) = st(lua)?.run.as_mut() {
                run.rendering_done = true;
                run.threads.clear();
            }
            rendering_phase(lua, svc)?;
        }
    }
    // Clients' results reach the server's TestService.
    let server = world(lua).is_ok_and(|w| w.side == Side::Server);
    if server {
        let got: Vec<Collected> = COLLECTED.lock().map(|mut c| std::mem::take(&mut *c)).unwrap_or_default();
        if !got.is_empty() {
            if let Some(svc) = service_ref(lua, "TestService")? {
                for c in got {
                    let text = lstr(lua, &c.text)?;
                    let line = Value::Number(c.line as f64);
                    let mut w = world(lua)?;
                    match c.condition {
                        None => w.fire(svc, "ServerCollectResult", vec![text, Value::Nil, line]),
                        Some(b) => w.fire(
                            svc,
                            "ServerCollectConditionalResult",
                            vec![Value::Boolean(b), text, Value::Nil, line],
                        ),
                    }
                }
            }
        }
    }
    Ok(())
}
