//! Engine API domain: services (core engine services). See src/domains/mod.rs
//! for the hook contract and checks/parity/services.md for what was measured.
//!
//! Every behaviour here was checked against Studio 0.744 in an offline Studio, in
//! a Run test from a server Script unless noted. Where Studio has no real
//! backend in a Play test, this does what Studio does there:
//! - capability-gated members raise Studio's own capability error
//!   ("The current thread cannot call 'X' (lacking capability InternalTest)"):
//!   RunService.FrameNumber/RunState, Stats' Harmony and per-category memory
//!   calls, ScriptContext coverage, AnalyticsService.GetDurationLoggerTimestamp,
//!   UniqueIdLookupService, CustomLog.
//! - switched-off features give Studio's message: CollectionService
//!   collections, ScriptService:ResolveModulePath, HttpRbxApiService,
//!   SavePlace on an unpublished place.
//! - AnalyticsService sends nothing (a Play test has no analytics backend)
//!   but checks its player argument as Studio does.
//! Logger objects are parentless instances of class Logger (Studio's are
//! plain Objects, so `typeof` says "Instance" here, not "Object").

#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use mlua::{Function, Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::{Ref, Variant};

use crate::runtime::*;

#[path = "services_encoding.rs"]
mod encoding;
#[path = "services_http.rs"]
mod http;
#[path = "services_reflect.rs"]
mod reflect;

pub const METHODS: &[(&str, &str)] = &[
    ("IsPropertyModified", "Instance"),
    ("ResetPropertyToDefault", "Instance"),
    ("GetStyled", "Instance"),
    ("GetStyledPropertyChangedSignal", "Instance"),
    ("QueryDescendants", "Instance"),
    ("GetMessage", "DataModel"),
    ("GetRemoteBuildMode", "DataModel"),
    ("IsGearTypeAllowed", "DataModel"),
    ("BindToSimulation", "RunService"),
    ("BindToAnimation", "RunService"),
    ("GetPredictionStatus", "RunService"),
    ("IsResimulating", "RunService"),
    ("SetPredictionMode", "RunService"),
    ("ClearOutput", "LogService"),
    ("GetLogHistory", "LogService"),
    ("GetLogger", "LogService"),
    ("Info", "LogService"),
    ("Log", "LogService"),
    ("Output", "LogService"),
    ("Warn", "LogService"),
    ("GetLogger", "Logger"),
    ("Info", "Logger"),
    ("Log", "Logger"),
    ("Output", "Logger"),
    ("Warn", "Logger"),
    ("EnableCoverage", "ScriptContext"),
    ("GetCoverageStats", "ScriptContext"),
    ("GetHarmonyQualityLevel", "Stats"),
    ("GetMemoryCategoryNames", "Stats"),
    ("GetMemoryUsageMbAllCategories", "Stats"),
    ("GetMemoryUsageMbAllCategoriesFiltered", "Stats"),
    ("GetMemoryUsageMbFiltered", "Stats"),
    ("GetMemoryUsageMbForTag", "Stats"),
    ("GetTotalMemoryUsageMb", "Stats"),
    ("ResetHarmonyMemoryTarget", "Stats"),
    ("SetHarmonyMemoryTarget", "Stats"),
    ("CreateCollection", "CollectionService"),
    ("GetCollection", "CollectionService"),
    ("GetCollections", "CollectionService"),
    ("SmoothDamp", "TweenService"),
    ("CreateWebStreamClient", "HttpService"),
    ("GetSecret", "HttpService"),
    ("Base64Decode", "EncodingService"),
    ("Base64Encode", "EncodingService"),
    ("CompressBuffer", "EncodingService"),
    ("ComputeBufferHash", "EncodingService"),
    ("ComputeStringHash", "EncodingService"),
    ("DecompressBuffer", "EncodingService"),
    ("GetDecompressedBufferSize", "EncodingService"),
    ("GetClass", "ReflectionService"),
    ("GetClasses", "ReflectionService"),
    ("GetEventsOfClass", "ReflectionService"),
    ("GetMethodsOfClass", "ReflectionService"),
    ("GetPropertiesOfClass", "ReflectionService"),
    ("FireCustomEvent", "AnalyticsService"),
    ("FireEvent", "AnalyticsService"),
    ("FireInGameEconomyEvent", "AnalyticsService"),
    ("FireLogEvent", "AnalyticsService"),
    ("FirePlayerProgressionEvent", "AnalyticsService"),
    ("GetDurationLoggerTimestamp", "AnalyticsService"),
    ("LogCustomEvent", "AnalyticsService"),
    ("LogEconomyEvent", "AnalyticsService"),
    ("LogFunnelStepEvent", "AnalyticsService"),
    ("LogJourneyEvent", "AnalyticsService"),
    ("LogOnboardingFunnelStepEvent", "AnalyticsService"),
    ("LogProgressionCompleteEvent", "AnalyticsService"),
    ("LogProgressionEvent", "AnalyticsService"),
    ("LogProgressionFailEvent", "AnalyticsService"),
    ("LogProgressionStartEvent", "AnalyticsService"),
    ("Close", "CustomLog"),
    ("GetLogPath", "CustomLog"),
    ("Open", "CustomLog"),
    ("WriteAppend", "CustomLog"),
    ("GetAttachedReceivers", "CustomEvent"),
    ("SetValue", "CustomEvent"),
    ("GetCurrentValue", "CustomEventReceiver"),
    ("IsUserFeatureEnabled", "UserSettings"),
    ("Reset", "UserSettings"),
    ("GetCameraYInvertValue", "UserGameSettings"),
    ("GetOnboardingCompleted", "UserGameSettings"),
    ("InFullScreen", "UserGameSettings"),
    ("InStudioMode", "UserGameSettings"),
    ("SetCameraYInvertVisible", "UserGameSettings"),
    ("SetGamepadCameraSensitivityVisible", "UserGameSettings"),
    ("SetOnboardingCompleted", "UserGameSettings"),
    ("GetFFlag", "GlobalSettings"),
    ("GetFVariable", "GlobalSettings"),
    ("ResolveModulePath", "ScriptService"),
    ("GetOrCreateUniqueIdRemoteCommand", "UniqueIdLookupService"),
    ("GetDataInRange", "MicroProfilerService"),
    ("GetDataSize", "MicroProfilerService"),
    ("ProcessCommand", "MicroProfilerService"),
    ("EndRemoteRomarkTest", "RomarkService"),
    ("SetPrimaryPart", "GeneratedFolder"),
    ("GetServiceVisibility", "ExplorerServiceVisibilityService"),
    ("GetPlayer", "NetworkReplicator"),
    ("EncryptStringForPlayerId", "NetworkServer"),
    // Studio declares IsA on Object: the non-Instance objects (Logger,
    // EditableImage…) have it too (Instances take the core one).
    ("IsA", "Object"),
];

/// Written in Luau (services.luau) because they yield or throw a table.
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("Error", "LogService"),
    ("Error", "Logger"),
    ("GetLogHistoryAsync", "LogService"),
    ("SavePlace", "DataModel"),
    ("RequestLimitedAsync", "HttpRbxApiService"),
    ("GetPlayerSegmentsAsync", "AnalyticsService"),
    ("SerializeInstancesAsync", "SerializationService"),
    ("DeserializeInstancesAsync", "SerializationService"),
    ("GetAsync", "HttpService"),
    ("PostAsync", "HttpService"),
    ("RequestAsync", "HttpService"),
];

pub const EVENTS: &[(&str, &str)] = &[
    // Fired from the session's output (services `on_leave`), deferred as in Studio.
    ("MessageOut", "LogService"),
    // Fired at once by the logger's own calls, on it and the loggers above it.
    ("MessageOut", "Logger"),
    // Source links and SetValue (services `newindex`, services.luau).
    ("ReceiverConnected", "CustomEvent"),
    ("ReceiverDisconnected", "CustomEvent"),
    ("EventConnected", "CustomEventReceiver"),
    ("EventDisconnected", "CustomEventReceiver"),
    ("SourceValueChanged", "CustomEventReceiver"),
    // A tag coming into use (services `step`). TagRemoved exists but Studio
    // didn't fire it when the last instance lost the tag (Run test), and
    // neither does this.
    ("TagAdded", "CollectionService"),
    ("TagRemoved", "CollectionService"),
    // GetService making a service / a service leaving (services `step`).
    ("ServiceAdded", "ServiceProvider"),
    ("ServiceRemoving", "ServiceProvider"),
    // Every property change of an instance in the DataModel (`item_changed`).
    ("ItemChanged", "DataModel"),
    // Never fired in a Play test, in Studio or here: there is nothing to
    // drive them (no fullscreen switch or Studio-mode change in a test
    // session, no live server lifecycle, restarts or memory warnings, the
    // legacy gear-type settings never change, no MicroProfiler capture).
    ("FullscreenChanged", "UserGameSettings"),
    ("StudioModeChanged", "UserGameSettings"),
    ("ServerLifecycleChanged", "DataModel"),
    ("ServerRestartScheduled", "DataModel"),
    ("ServerLowMemoryWarning", "DataModel"),
    ("AllowedGearTypeChanged", "DataModel"),
    ("DataChanged", "MicroProfilerService"),
    // Prediction: nothing is resimulated in rustudio (or in a Studio test),
    // so neither fires. Game scripts may index both, connect to Rollback,
    // but not to Misprediction (PluginOrOpenCloud: src/security.rs).
    ("Misprediction", "RunService"),
    ("Rollback", "RunService"),
];

// ------------------------------------------------------------------ state

#[derive(Default)]
struct State {
    /// Session output already turned into history / MessageOut.
    seen: usize,
    /// GetLogHistory: (message, MessageType value, unix time).
    history: Vec<(String, u32, f64)>,
    logger_paths: HashMap<Ref, String>,
    /// CustomEvent → its receivers (Source links), parented or not.
    receivers: HashMap<Ref, Vec<Ref>>,
    http_hint_shown: bool,
    sim_hooked: bool,
    /// The DataModel's services, for ServiceAdded / ServiceRemoving.
    services: Option<HashSet<Ref>>,
    last_step: Option<Instant>,
    frame_time: f64,
    step_started: Option<Instant>,
    heartbeat_time: f64,
    physics_prev: (f64, u64),
    physics_step: f64,
}

fn state(lua: &Lua) -> mlua::Result<mlua::AppDataRefMut<'_, State>> {
    lua.app_data_mut::<State>().ok_or_else(|| rt_err("services domain not installed"))
}

fn impl_fn(lua: &Lua, name: &str) -> mlua::Result<Function> {
    let t: Table = lua.named_registry_value("svc_impl")?;
    t.get(name)
}

fn lcall(lua: &Lua, name: &str, args: impl mlua::IntoLuaMulti) -> mlua::Result<MultiValue> {
    impl_fn(lua, name)?.call(args)
}

fn ok(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([v]))
}
fn nothing() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}
fn s(lua: &Lua, v: impl AsRef<[u8]>) -> mlua::Result<Value> {
    Ok(Value::String(lua.create_string(v)?))
}

fn capability(what: &str, name: &str, cap: &str) -> mlua::Error {
    rt_err(format!("The current thread cannot {what} '{name}' (lacking capability {cap})"))
}

fn class_of(lua: &Lua, r: Ref) -> mlua::Result<String> {
    Ok(inst_meta(lua, r)?.0)
}

fn rest(args: &MultiValue) -> MultiValue {
    args.iter().skip(1).cloned().collect()
}

/// An argument's buffer bytes, or Studio's "Argument N missing or nil".
fn buffer_arg(args: &MultiValue, i: usize) -> mlua::Result<Vec<u8>> {
    match arg(args, i) {
        Value::Buffer(b) => Ok(b.to_vec()),
        _ => Err(rt_err(format!("Argument {i} missing or nil"))),
    }
}

/// An EnumItem argument of `enum_name` (or its number / name), as its value.
fn enum_arg(args: &MultiValue, i: usize, enum_name: &str) -> mlua::Result<u32> {
    let v = arg(args, i);
    if let Some(e) = opt_ud::<LEnumItem>(&v) {
        if e.enum_type == enum_name {
            return Ok(e.value);
        }
        return Err(rt_err(format!("Unable to cast EnumItem to {enum_name}")));
    }
    let valid = |n: u32| LEnumItem::from_value(enum_name, n).is_some();
    match v {
        Value::Integer(n) if n >= 0 && valid(n as u32) => Ok(n as u32),
        Value::Number(n) if n >= 0.0 && n.fract() == 0.0 && valid(n as u32) => Ok(n as u32),
        Value::Integer(_) | Value::Number(_) => Err(rt_err(format!("Unable to cast double to {enum_name}"))),
        Value::String(st) => {
            let name = st.to_str()?.to_string();
            crate::doc::db()
                .enums
                .get(enum_name)
                .and_then(|e| e.items.get(name.as_str()).copied())
                .ok_or_else(|| rt_err(format!("Unable to cast string \"{name}\" to {enum_name}")))
        }
        _ => Err(rt_err(format!("Argument {i} missing or nil"))),
    }
}

fn enum_val(lua: &Lua, enum_name: &str, value: u32) -> mlua::Result<Value> {
    match LEnumItem::from_value(enum_name, value) {
        Some(e) => enum_value(lua, e),
        None => Ok(Value::Nil),
    }
}

// ------------------------------------------------------------------ calls

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if !METHODS.iter().any(|(m, _)| *m == name) {
        return None;
    }
    let class = match class_of(lua, this) {
        Ok(c) => c,
        Err(e) => return Some(Err(e)),
    };
    let owner = METHODS
        .iter()
        .find(|(m, b)| *m == name && crate::doc::is_a(&class, b))
        .map(|(_, b)| *b)?;
    Some(call_inner(lua, name, owner, &class, this, args))
}

fn call_inner(lua: &Lua, name: &str, owner: &str, class: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let me = arg(args, 0);
    match (owner, name) {
        // ---------------------------------------------------- Instance
        ("Instance", _) => lcall(lua, name, args.clone()),

        // ---------------------------------------------------- DataModel
        ("DataModel", "GetMessage") => ok(s(lua, "")?),
        ("DataModel", "GetRemoteBuildMode") => ok(Value::Boolean(false)),
        ("DataModel", "IsGearTypeAllowed") => {
            enum_arg(args, 1, "GearType")?;
            ok(Value::Boolean(false))
        }

        // ---------------------------------------------------- RunService
        ("RunService", "BindToSimulation" | "BindToAnimation") => lcall(lua, name, args.clone()),
        ("RunService", "GetPredictionStatus") => {
            // Nothing is predicted: the server is authoritative over all of
            // its instances; a client predicts nothing either (Studio's edit
            // DataModel answers None).
            let v = if world(lua)?.side == Side::Server { 0 } else { 2 };
            ok(enum_val(lua, "PredictionStatus", v)?)
        }
        ("RunService", "IsResimulating") => ok(Value::Boolean(false)),
        ("RunService", "SetPredictionMode") => {
            if opt_inst(&arg(args, 1)).is_none() {
                return Err(rt_err("Argument 1 missing or nil"));
            }
            enum_arg(args, 2, "PredictionMode")?;
            nothing()
        }

        // ---------------------------------------------------- LogService / Logger
        ("LogService", "GetLogHistory") => {
            capture_output(lua)?;
            let entries = state(lua)?.history.clone();
            let t = lua.create_table_with_capacity(entries.len(), 0)?;
            for (i, (msg, ty, at)) in entries.into_iter().enumerate() {
                let e = lua.create_table()?;
                e.set("message", msg)?;
                e.set("messageType", enum_val(lua, "MessageType", ty)?)?;
                e.set("timestamp", at)?;
                t.set(i + 1, e)?;
            }
            ok(Value::Table(t))
        }
        ("LogService", "ClearOutput") => {
            // Studio clears its Output window; the session's history stays.
            nothing()
        }
        ("LogService" | "Logger", "GetLogger") => lcall(lua, "GetLogger", args.clone()),
        ("LogService" | "Logger", "Info" | "Warn" | "Output" | "Log") => {
            let mut a = MultiValue::new();
            a.push_back(me);
            a.push_back(s(lua, name)?);
            a.extend(rest(args));
            lcall(lua, "log", a)
        }

        // ---------------------------------------------------- ScriptContext
        ("ScriptContext", _) => Err(capability("call", name, "PluginOrOpenCloud")),

        // ---------------------------------------------------- Stats
        ("Stats", "GetMemoryUsageMbForTag") => {
            let tag = enum_arg(args, 1, "DeveloperMemoryTag")?;
            let lua_mb = lua.used_memory() as f64 / (1024.0 * 1024.0);
            ok(Value::Number(match tag {
                0 => (rss_mb() - lua_mb).max(0.0),
                4 => lua_mb,
                _ => 0.0,
            }))
        }
        ("Stats", "GetTotalMemoryUsageMb") => ok(Value::Number(rss_mb())),
        ("Stats", _) => Err(capability("call", name, "InternalTest")),

        // ---------------------------------------------------- CollectionService
        ("CollectionService", "CreateCollection") => Err(rt_err("Collections are not enabled")),
        ("CollectionService", "GetCollection") => {
            arg_str(args, 1, "string")?;
            ok(Value::Table(lua.create_table()?))
        }
        ("CollectionService", "GetCollections") => {
            if opt_inst(&arg(args, 1)).is_none() {
                return Err(rt_err("Argument 1 missing or nil"));
            }
            ok(Value::Table(lua.create_table()?))
        }

        // ---------------------------------------------------- TweenService
        ("TweenService", "SmoothDamp") => lcall(lua, "SmoothDamp", args.clone()),

        // ---------------------------------------------------- HttpService
        ("HttpService", "GetSecret") => {
            arg_str(args, 1, "string")?;
            Err(rt_err("Can't find secret with given key"))
        }
        ("HttpService", "CreateWebStreamClient") => {
            enum_arg(args, 1, "WebStreamClientType")?;
            http_enabled_check(lua, this)?;
            Err(rt_err("WebStreamClient is not supported in this session"))
        }

        // ---------------------------------------------------- EncodingService
        ("EncodingService", _) => encoding_call(lua, name, args),

        // ---------------------------------------------------- ReflectionService
        ("ReflectionService", "GetClass") => ok(reflect::get_class(lua, &arg_str(args, 1, "string")?, &arg(args, 2))?),
        ("ReflectionService", "GetClasses") => ok(reflect::get_classes(lua, &arg(args, 1))?),
        ("ReflectionService", _) => {
            let kind = match name {
                "GetPropertiesOfClass" => "Property",
                "GetMethodsOfClass" => "Function",
                _ => "Event",
            };
            ok(reflect::members_of(lua, &arg_str(args, 1, "string")?, kind, &arg(args, 2))?)
        }

        // ---------------------------------------------------- AnalyticsService
        ("AnalyticsService", "GetDurationLoggerTimestamp") => Err(capability("call", name, "InternalTest")),
        ("AnalyticsService", "FireEvent") => {
            arg_str(args, 1, "string")?;
            nothing()
        }
        ("AnalyticsService", _) => {
            // Every other call takes the player first: Studio logs (doesn't
            // throw) when it isn't one, and sends nothing in a Play test.
            impl_fn(lua, "analytics_player")?.call::<bool>(arg(args, 1))?;
            nothing()
        }

        // ---------------------------------------------------- CustomLog
        ("CustomLog", _) => Err(capability("call", name, "InternalTest")),

        // ---------------------------------------------------- CustomEvent
        ("CustomEvent", "GetAttachedReceivers") => {
            let list = receivers_of(lua, this)?;
            let t = lua.create_table()?;
            for (i, r) in list.into_iter().enumerate() {
                t.set(i + 1, inst_value(lua, r)?)?;
            }
            ok(Value::Table(t))
        }
        ("CustomEvent", "SetValue") => lcall(lua, "custom_event_set", args.clone()),
        ("CustomEventReceiver", "GetCurrentValue") => {
            let src = match world(lua)?.d().prop(this, "Source") {
                Some(Variant::Ref(r)) if !r.is_none() => Some(r),
                _ => None,
            };
            let src = match src {
                Some(r) => inst_value(lua, r)?,
                None => Value::Nil,
            };
            lcall(lua, "custom_event_value", src)
        }

        // ---------------------------------------------------- settings
        ("UserSettings", "IsUserFeatureEnabled") => {
            let f = arg_str(args, 1, "string")?;
            Err(rt_err(format!("Flag {f} does not exist")))
        }
        ("UserSettings", "Reset") => {
            emit(lua, crate::app::Level::Error, "Insufficient permissions to set ControlMode".into())?;
            Err(capability("set", "camera control mode", "RobloxScript"))
        }
        ("UserGameSettings", "GetCameraYInvertValue") => {
            let inv = matches!(ugs(lua)?.raw_get::<Value>("CameraYInverted")?, Value::Boolean(true));
            ok(Value::Integer(if inv { -1 } else { 1 }))
        }
        ("UserGameSettings", "GetOnboardingCompleted") => {
            arg_str(args, 1, "string")?;
            ok(Value::Boolean(false))
        }
        ("UserGameSettings", "SetOnboardingCompleted") => {
            arg_str(args, 1, "string")?;
            Err(rt_err("Invalid onboarding ID"))
        }
        ("UserGameSettings", "InFullScreen") => ok(Value::Boolean(false)),
        ("UserGameSettings", "InStudioMode") => ok(Value::Boolean(true)),
        ("UserGameSettings", _) => nothing(),
        ("GlobalSettings", _) => {
            let f = arg_str(args, 1, "string")?;
            Err(rt_err(format!("Flag {f} does not exist")))
        }

        ("Object", "IsA") => {
            let c = arg_str(args, 1, "string")?;
            ok(Value::Boolean(crate::doc::is_a(class, &c)))
        }

        // ---------------------------------------------------- the rest
        ("ScriptService", "ResolveModulePath") => Err(rt_err("ScriptService:ResolveModulePath is not enabled")),
        ("UniqueIdLookupService", _) => Err(capability("call", name, "RemoteCommand")),
        // rustudio has no MicroProfiler: no captured data to hand out.
        ("MicroProfilerService", "GetDataSize") => ok(Value::Integer(0)),
        ("MicroProfilerService", _) => ok(Value::Integer(0)),
        ("RomarkService", _) => nothing(),
        ("GeneratedFolder", "SetPrimaryPart") => {
            let p = arg(args, 1);
            match opt_inst(&p) {
                Some(r) if crate::doc::is_a(&class_of(lua, r)?, "BasePart") => {
                    world(lua)?.dm().dom.get_by_ref_mut(this).map(|i| {
                        i.properties.insert("PrimaryPart".into(), Variant::Ref(r));
                    });
                    nothing()
                }
                _ => Err(rt_err("Argument 1 missing or nil")),
            }
        }
        ("ExplorerServiceVisibilityService", _) => {
            let r = opt_inst(&arg(args, 1)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            ok(Value::Boolean(true))
        }
        ("NetworkReplicator", "GetPlayer") => ok(Value::Nil),
        ("NetworkServer", _) => Err(capability("call", name, "RobloxScript")),
        _ => Err(rt_err(format!("{name} is not a valid member of {class}"))),
    }
}

fn encoding_call(lua: &Lua, name: &str, args: &MultiValue) -> mlua::Result<MultiValue> {
    let buf = |v: Vec<u8>| -> mlua::Result<MultiValue> { ok(Value::Buffer(lua.create_buffer(v)?)) };
    match name {
        "Base64Encode" => buf(encoding::base64_encode(&buffer_arg(args, 1)?)),
        "Base64Decode" => {
            let b = buffer_arg(args, 1)?;
            buf(encoding::base64_decode(&b).ok_or_else(|| rt_err("Base64Decode: input is not a base64 string"))?)
        }
        "ComputeStringHash" => {
            let data = match arg(args, 1) {
                Value::String(st) => st.as_bytes().to_vec(),
                Value::Integer(n) => n.to_string().into_bytes(),
                Value::Number(n) => lua.create_string(n.to_string())?.as_bytes().to_vec(),
                _ => return Err(rt_err("Argument 1 missing or nil")),
            };
            let alg = enum_arg(args, 2, "HashAlgorithm")?;
            ok(s(lua, encoding::hash(alg, &data).unwrap_or_default())?)
        }
        "ComputeBufferHash" => {
            let data = buffer_arg(args, 1)?;
            let alg = enum_arg(args, 2, "HashAlgorithm")?;
            buf(encoding::hash(alg, &data).unwrap_or_default())
        }
        "CompressBuffer" => {
            let data = buffer_arg(args, 1)?;
            enum_arg(args, 2, "CompressionAlgorithm")?;
            let level = match arg(args, 3) {
                Value::Nil => 1,
                Value::Integer(n) => n as i32,
                Value::Number(n) => n.trunc() as i32,
                _ => return Err(rt_err("Unable to cast to int")),
            };
            buf(encoding::compress(&data, level).map_err(rt_err)?)
        }
        "DecompressBuffer" => {
            let data = buffer_arg(args, 1)?;
            enum_arg(args, 2, "CompressionAlgorithm")?;
            buf(encoding::decompress(&data).map_err(rt_err)?)
        }
        "GetDecompressedBufferSize" => {
            let data = buffer_arg(args, 1)?;
            enum_arg(args, 2, "CompressionAlgorithm")?;
            ok(match encoding::decompressed_size(&data) {
                Some(n) => Value::Number(n as f64),
                None => Value::Nil,
            })
        }
        _ => Err(rt_err(format!("{name} is not a valid member of EncodingService"))),
    }
}

fn receivers_of(lua: &Lua, ev: Ref) -> mlua::Result<Vec<Ref>> {
    let list = state(lua)?.receivers.get(&ev).cloned().unwrap_or_default();
    let w = world(lua)?;
    let d = w.d();
    // Receivers saved in the place link up without a script setting Source.
    let mut out: Vec<Ref> = list.into_iter().filter(|r| d.dom.get_by_ref(*r).is_some()).collect();
    for i in d.dom.descendants_of(d.dom.root_ref()) {
        if i.class == "CustomEventReceiver"
            && matches!(i.properties.get(&"Source".into()), Some(Variant::Ref(r)) if *r == ev)
            && !out.contains(&i.referent())
        {
            out.push(i.referent());
        }
    }
    Ok(out)
}

#[cfg(target_os = "linux")]
fn rss_mb() -> f64 {
    let pages = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|n| n.parse::<f64>().ok()))
        .unwrap_or(0.0);
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as f64;
    pages * page / (1024.0 * 1024.0)
}

#[cfg(not(target_os = "linux"))]
fn rss_mb() -> f64 {
    0.0
}

// ------------------------------------------------------------------ output

/// A line in the session output, as rustudio's own engine lines are pushed.
fn emit(lua: &Lua, level: crate::app::Level, text: String) -> mlua::Result<()> {
    world(lua)?.output.push((level, text));
    Ok(())
}

/// Turns output lines the session hasn't seen into log history and
/// LogService.MessageOut (fired deferred, as Studio does).
fn capture_output(lua: &Lua) -> mlua::Result<()> {
    let Some(st) = lua.app_data_ref::<State>() else { return Ok(()) };
    let seen = st.seen;
    drop(st);
    let (fresh, log_service) = {
        let Some(w) = lua.app_data_ref::<World>() else { return Ok(()) };
        if w.output.len() <= seen {
            return Ok(());
        }
        let fresh: Vec<(crate::app::Level, String)> = w.output[seen..].to_vec();
        let ls = w
            .doc
            .as_ref()
            .and_then(|d| d.find_child(d.dom.root_ref(), "LogService"))
            .filter(|ls| w.listening.contains(ls));
        (fresh, ls)
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() as f64) / 1000.0)
        .unwrap_or(0.0);
    let mut lines = Vec::with_capacity(fresh.len());
    for (level, text) in &fresh {
        let mark = crate::output_panel::STACK_MARK;
        let (t, info) = match text.strip_prefix(mark) {
            Some(r) => (r, true),
            None => (text.as_str(), false),
        };
        let msg = t.rsplit_once("  —  ").map(|(m, _)| m).unwrap_or(t).to_string();
        let ty = match level {
            crate::app::Level::Error => 3,
            crate::app::Level::Warn => 2,
            crate::app::Level::Info if info => 1,
            crate::app::Level::Info => 0,
        };
        lines.push((msg, ty, now));
    }
    {
        let mut st = state(lua)?;
        st.seen += fresh.len();
        st.history.extend(lines.iter().cloned());
        // A long session keeps its most recent lines.
        if st.history.len() > 40_000 {
            st.history.drain(..10_000);
        }
    }
    if let Some(ls) = log_service {
        for (msg, ty, _) in lines {
            let args = vec![
                Value::String(lua.create_string(&msg)?),
                enum_val(lua, "MessageType", ty)?,
                Value::Table(lua.create_table()?),
            ];
            world(lua)?.fire(ls, "MessageOut", args);
        }
    }
    Ok(())
}

/// The runtime entered a World: output already in it was seen (or drained).
pub fn on_enter(lua: &Lua) {
    let len = lua.app_data_ref::<World>().map(|w| w.output.len()).unwrap_or(0);
    if let Some(mut st) = lua.app_data_mut::<State>() {
        st.seen = len;
    }
}

/// The runtime is about to hand its World back (and its output drained).
pub fn on_leave(lua: &Lua) {
    let _ = capture_output(lua);
    if let Some(mut st) = lua.app_data_mut::<State>() {
        if let Some(t) = st.step_started.take() {
            st.heartbeat_time = t.elapsed().as_secs_f64();
        }
    }
}

// ------------------------------------------------------------------ HTTP

fn http_enabled_check(lua: &Lua, hs: Ref) -> mlua::Result<()> {
    let on = matches!(world(lua)?.d().prop(hs, "HttpEnabled"), Some(Variant::Bool(true)));
    if on {
        return Ok(());
    }
    let first = {
        let mut st = state(lua)?;
        !std::mem::replace(&mut st.http_hint_shown, true)
    };
    if first {
        let mark = crate::output_panel::STACK_MARK;
        emit(lua, crate::app::Level::Info, format!("{mark}Go to Experience Settings and turn on Allow HTTP requests."))?;
    }
    Err(rt_err("HTTP requests are not enabled. Enable via Experience Settings"))
}

/// `__svc_http_start(hs, method, url, headers, body)` → request id.
fn http_start(lua: &Lua, a: (LInst, String, Value, Option<Table>, Option<mlua::LuaString>)) -> mlua::Result<(Option<u64>, Option<String>)> {
    match http_start_inner(lua, a) {
        Ok(id) => Ok((Some(id), None)),
        Err(e) => Ok((None, Some(error_message(&e)))),
    }
}

fn http_start_inner(lua: &Lua, (hs, method, url, headers, body): (LInst, String, Value, Option<Table>, Option<mlua::LuaString>)) -> mlua::Result<u64> {
    // Checked in this order, before HttpEnabled (the real signed-in Studio
    // 0.746 with HttpEnabled off still gives these).
    let url = match url {
        Value::String(s) => s.to_str()?.to_string(),
        _ => return Err(rt_err("Invalid URL type, URL should be a string or secret.")),
    };
    let mut hv = Vec::new();
    if let Some(h) = headers {
        for p in h.pairs::<String, Value>() {
            let (k, v) = p?;
            if http::restricted_header(&k) {
                return Err(rt_err(format!("Header \"{k}\" is not allowed!")));
            }
            let v = match v {
                Value::String(s) => s.to_str()?.to_string(),
                _ => return Err(rt_err(format!("Header {k} must have its value be a string or secret!"))),
            };
            hv.push((k, v));
        }
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(rt_err("URL must be http"));
    }
    http_enabled_check(lua, hs.0)?;
    if http::is_roblox_domain(&url) {
        return Err(rt_err("HttpService is not allowed to access that Roblox resource"));
    }
    // Methods are case-sensitive ("get" fails the same way).
    if !matches!(method.as_str(), "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" | "TRACE") {
        return Err(rt_err("HttpError: Aborted"));
    }
    let place_id = match {
        let w = world(lua)?;
        let d = w.d();
        d.prop(d.dom.root_ref(), "PlaceId")
    } {
        Some(Variant::Int64(n)) => n,
        Some(Variant::Int32(n)) => n as i64,
        _ => 0,
    };
    Ok(http::start(http::Request {
        method,
        url,
        headers: http::studio_headers(hv, place_id),
        body: body.map(|b| b.as_bytes().to_vec()),
    }))
}

/// `__svc_http_poll(id)` → nil (pending), or (ok, status, message, headers, body | error).
fn http_poll(lua: &Lua, id: u64) -> mlua::Result<MultiValue> {
    let Some(r) = http::poll(id) else { return Ok(MultiValue::new()) };
    let mut out = MultiValue::new();
    match r {
        Ok(resp) => {
            out.push_back(Value::Boolean(true));
            out.push_back(Value::Integer(resp.status as i64));
            out.push_back(s(lua, resp.status_message)?);
            let h = lua.create_table()?;
            for (k, v) in resp.headers {
                h.set(k, v)?;
            }
            out.push_back(Value::Table(h));
            out.push_back(s(lua, resp.body)?);
        }
        Err(e) => {
            out.push_back(Value::Boolean(false));
            out.push_back(s(lua, e)?);
        }
    }
    Ok(out)
}

const HTTP_LUA: &str = r#"
local start, poll = ...
local M = __rs_lua_methods
local function fail(m) error(m, 0) end
local function wait_for(id)
	while true do
		local r = table.pack(poll(id))
		if r.n > 0 then return table.unpack(r, 1, r.n) end
		task.wait()
	end
end
local function content_type(ct)
	local names = { [0] = "application/json", "application/xml", "application/x-www-form-urlencoded", "text/plain", "text/xml" }
	if ct == nil then return "application/json" end
	local v = if typeof(ct) == "EnumItem" then ct.Value else tonumber(ct)
	return names[v] or "application/json"
end
local function cast_name(v) return if type(v) == "number" then "double" elseif type(v) == "boolean" then "bool" else typeof(v) end
-- "HTTP 404 (Not Found)" over HTTP/1.1; HTTP/2 has no reason phrase: "HTTP 404".
local function status_error(status, msg)
	if msg == "" then return "HTTP " .. status end
	return "HTTP " .. status .. " (" .. msg .. ")"
end
local function simple(self, method, url, body, ct, compress, headers)
	if headers ~= nil and type(headers) ~= "table" then fail("Unable to cast " .. cast_name(headers) .. " to Dictionary") end
	local h = {}
	if headers then for k, v in headers do h[k] = v end end
	if method == "POST" and h["Content-Type"] == nil then h["Content-Type"] = content_type(ct) end
	local id, err = start(self, method, url, h, body)
	if not id then fail(err) end
	local okk, status, msg, _, resp = wait_for(id)
	if not okk then fail(status) end
	if status < 200 or status >= 300 then fail(status_error(status, msg)) end
	return resp
end
local prev_get, prev_post, prev_req = M.GetAsync, M.PostAsync, M.RequestAsync
M.GetAsync = function(self, url, nocache, headers)
	if typeof(self) ~= "Instance" or self.ClassName ~= "HttpService" then return prev_get(self, url, nocache, headers) end
	return simple(self, "GET", url, nil, nil, nil, headers)
end
M.PostAsync = function(self, url, data, ct, compress, headers)
	if typeof(self) ~= "Instance" or self.ClassName ~= "HttpService" then return prev_post(self, url, data, ct, compress, headers) end
	if data == nil then fail("Argument 2 missing or nil") end
	-- (compress: Studio sends the body as it is, verified with httpbin.)
	return simple(self, "POST", url, tostring(data), ct, compress, headers)
end
M.RequestAsync = function(self, opts)
	if typeof(self) ~= "Instance" or self.ClassName ~= "HttpService" then return prev_req(self, opts) end
	if type(opts) ~= "table" then fail("Unable to cast " .. cast_name(opts) .. " to Dictionary") end
	if opts.Url == nil then fail("Invalid request options") end
	local method = if opts.Method == nil then "GET" else tostring(opts.Method)
	if opts.Body ~= nil and type(opts.Body) ~= "string" then fail("Invalid request options, Body should be a string") end
	if opts.Body ~= nil and (method == "GET" or method == "HEAD") then
		fail("Invalid request options, Body should be empty for 'GET' or 'HEAD' method")
	end
	local h = {}
	if opts.Headers then for k, v in opts.Headers do h[k] = v end end
	local id, err = start(self, method, opts.Url, h, opts.Body)
	if not id then fail(err) end
	local okk, status, msg, headers, body = wait_for(id)
	if not okk then fail(status) end
	return { Success = status >= 200 and status < 300, StatusCode = status, StatusMessage = msg, Headers = headers, Body = body }
end
"#;

// ------------------------------------------------------------------ properties

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    let is = |b: &str| crate::doc::is_a(class, b);
    Ok(Some(match key {
        "Sandboxed" => Value::Boolean(matches!(world(lua)?.d().prop(r, "Sandboxed"), Some(Variant::Bool(true)))),
        // Not scriptable: Studio has no such member for scripts.
        "IsInSandbox" | "PredictionMode" if is("Instance") => {
            let full = world(lua)?.full_name(r);
            return Err(rt_err(format!("{key} is not a valid member of {class} \"{full}\"")));
        }
        "GearGenreSetting" if class == "DataModel" => enum_val(lua, "GearGenreSetting", 0)?,
        "FrameNumber" if class == "RunService" => return Err(capability("read", key, "InternalTest")),
        "RunState" if class == "RunService" => return Err(capability("read", key, "PluginOrOpenCloud")),
        "SelectionThickness" if class == "Selection" => Value::Number(0.02f32 as f64),
        "FullPath" if class == "Logger" => {
            let p = state(lua)?.logger_paths.get(&r).cloned();
            s(lua, p.unwrap_or_else(|| world(lua).map(|w| w.d().dom.get_by_ref(r).map(|i| i.name.clone()).unwrap_or_default()).unwrap_or_default()))?
        }
        // UserSettings().GameSettings: its UserGameSettings.
        "GameSettings" if class == "UserSettings" => {
            let c = {
                let w = world(lua)?;
                let d = w.d();
                d.dom.get_by_ref(r).and_then(|i| {
                    i.children().iter().copied().find(|c| d.class_of(*c).is_some_and(|k| k == "UserGameSettings"))
                })
            };
            match c {
                Some(c) => crate::runtime::inst_value(lua, c)?,
                None => Value::Nil,
            }
        }
        // UserSettings():GetService("UserGameSettings"): the values live in
        // library.luau's table, which the engine reads (RotationType, the
        // game menu's sliders).
        _ if class == "UserGameSettings" => match ugs(lua)?.raw_get::<Value>(key)? {
            Value::Nil => return Ok(None),
            v => v,
        },
        "ConnectionState" if class == "WebSocketClient" => enum_val(lua, "WebSocketState", 3)?,
        "ConnectionState" if class == "WebStreamClient" => enum_val(lua, "WebStreamClientState", 3)?,
        "SchedulerDutyCycle" | "SchedulerRate" if class == "TaskScheduler" => Value::Number(0.0),
        "ThreadPoolSize" if class == "TaskScheduler" => Value::Integer(
            std::thread::available_parallelism().map(|n| n.get() as i64).unwrap_or(1),
        ),
        _ if class == "Stats" => match stats_prop(lua, key)? {
            Some(v) => v,
            None => return Ok(None),
        },
        _ => return Ok(None),
    }))
}

/// library.luau's UserGameSettings values (`__rs_user_game_settings`).
fn ugs(lua: &Lua) -> mlua::Result<mlua::Table> {
    lua.named_registry_value("rs_user_game_settings")
}

fn stats_prop(lua: &Lua, key: &str) -> mlua::Result<Option<Value>> {
    let st = |f: fn(&State) -> f64| -> mlua::Result<f64> { Ok(state(lua).map(|s| f(&s)).unwrap_or(0.0)) };
    Ok(Some(match key {
        "InstanceCount" => {
            let w = world(lua)?;
            let d = w.d();
            Value::Integer(d.dom.descendants_of(d.dom.root_ref()).count() as i64)
        }
        "PrimitivesCount" | "MovingPrimitivesCount" => {
            let w = world(lua)?;
            let d = w.d();
            let Some(ws) = d.workspace() else { return Ok(Some(Value::Integer(0))) };
            let parts = d.dom.descendants_of(ws).filter(|i| crate::doc::is_a(&i.class, "BasePart"));
            let n = if key == "PrimitivesCount" {
                parts.count()
            } else {
                parts
                    .filter(|i| w.velocities.get(&i.referent()).is_some_and(|v| v.length_squared() > 1e-6))
                    .count()
            };
            Value::Integer(n as i64)
        }
        "ContactsCount" => Value::Integer(perf::contacts(world(lua)?.d().id) as i64),
        "MemoryTrackingEnabled" => Value::Boolean(true),
        "FrameTime" => Value::Number(st(|s| s.frame_time)?),
        "HeartbeatTime" => Value::Number(st(|s| s.heartbeat_time)?),
        "HeartbeatTimeMs" => Value::Number(st(|s| s.heartbeat_time)? * 1000.0),
        "PhysicsStepTime" => Value::Number(st(|s| s.physics_step)?),
        "PhysicsStepTimeMs" => Value::Number(st(|s| s.physics_step)? * 1000.0),
        // What crosses between the session's server and client DataModels
        // (replication and remotes), in Roblox wire-size terms (perf).
        "DataReceiveKbps" | "DataSendKbps" | "PhysicsReceiveKbps" | "PhysicsSendKbps" => {
            let server = world(lua)?.side == Side::Server;
            let to_client = matches!((key, server), ("DataSendKbps" | "PhysicsSendKbps", true) | ("DataReceiveKbps" | "PhysicsReceiveKbps", false));
            let physics = key.starts_with("Physics");
            Value::Number(perf::kbps(to_client, physics))
        }
        // The renderer draws the client's view: a server (Run, or Play's
        // server side) has nothing drawn, as in Studio's Run test.
        _ if key.starts_with("Render") || key.ends_with("Count") => {
            if world(lua)?.side != Side::Client {
                return Ok(Some(if key.ends_with("Count") { Value::Integer(0) } else { Value::Number(0.0) }));
            }
            let f = perf::frame();
            match key {
                "RenderCPUFrameTime" => Value::Number(f.cpu_s),
                "RenderGPUFrameTime" => Value::Number(f.gpu_s),
                "SceneDrawcallCount" => Value::Integer(f.n[0] as i64),
                "SceneTriangleCount" => Value::Integer(f.n[1] as i64),
                "ShadowsDrawcallCount" => Value::Integer(f.n[2] as i64),
                "ShadowsTriangleCount" => Value::Integer(f.n[3] as i64),
                "UI2DDrawcallCount" => Value::Integer(f.n[4] as i64),
                "UI2DTriangleCount" => Value::Integer(f.n[5] as i64),
                "UI3DDrawcallCount" => Value::Integer(f.n[6] as i64),
                "UI3DTriangleCount" => Value::Integer(f.n[7] as i64),
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    }))
}

/// Engine counters Stats reads, fed by the renderer, the GUI passes,
/// physics and replication (one call each; see services.md).
pub mod perf {
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    use std::sync::Mutex;
    use std::time::Instant;

    use rbx_dom_weak::types::Variant;

    pub const SCENE: usize = 0;
    pub const SHADOW: usize = 2;
    pub const UI2D: usize = 4;
    pub const UI3D: usize = 6;

    /// This frame's running counts: (calls, triangles) per kind.
    static ACC: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];

    #[derive(Clone, Copy, Default)]
    pub struct Frame {
        pub n: [u64; 8],
        pub cpu_s: f64,
        pub gpu_s: f64,
    }
    static LAST: Mutex<Frame> = Mutex::new(Frame { n: [0; 8], cpu_s: 0.0, gpu_s: 0.0 });

    /// One draw of `kind`: `calls` draw calls covering `tris` triangles.
    pub fn draw(kind: usize, calls: u64, tris: u64) {
        ACC[kind].fetch_add(calls, Relaxed);
        ACC[kind + 1].fetch_add(tris, Relaxed);
    }

    /// The renderer finished a frame: its counts become the last frame's.
    /// `cpu_s`: the frame's render encoding on the CPU; `gpu_s`: GPU time
    /// when known (0 otherwise).
    pub fn end_frame(cpu_s: f64, gpu_s: f64) {
        let mut f = Frame { n: [0; 8], cpu_s, gpu_s };
        for (i, a) in ACC.iter().enumerate() {
            f.n[i] = a.swap(0, Relaxed);
        }
        if let Ok(mut l) = LAST.lock() {
            *l = f;
        }
    }

    pub fn frame() -> Frame {
        LAST.lock().map(|f| *f).unwrap_or_default()
    }

    // --------------------------------------------------------- contacts

    static CONTACTS: Mutex<Option<HashMap<u64, usize>>> = Mutex::new(None);

    /// Physics stepped DataModel `doc`: its touching collider pairs.
    pub fn set_contacts(doc: u64, n: usize) {
        if let Ok(mut m) = CONTACTS.lock() {
            m.get_or_insert_with(HashMap::new).insert(doc, n);
        }
    }

    pub fn contacts(doc: u64) -> usize {
        CONTACTS.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(&doc).copied())).unwrap_or(0)
    }

    // --------------------------------------------------------- network

    /// (when, bytes) sent in each direction: [to client data, to client
    /// physics, to server data, to server physics].
    static NET: Mutex<[VecDeque<(Instant, u64)>; 4]> =
        Mutex::new([VecDeque::new(), VecDeque::new(), VecDeque::new(), VecDeque::new()]);
    static NET_TOTAL: AtomicU64 = AtomicU64::new(0);

    pub fn net(to_client: bool, physics: bool, bytes: u64) {
        if bytes == 0 {
            return;
        }
        NET_TOTAL.fetch_add(bytes, Relaxed);
        let i = (!to_client as usize) * 2 + physics as usize;
        if let Ok(mut q) = NET.lock() {
            let now = Instant::now();
            q[i].push_back((now, bytes));
            while q[i].front().is_some_and(|(t, _)| now.duration_since(*t).as_secs_f64() > 1.0) {
                q[i].pop_front();
            }
        }
    }

    /// Kilobits per second over the last second.
    pub fn kbps(to_client: bool, physics: bool) -> f64 {
        let i = (!to_client as usize) * 2 + physics as usize;
        let Ok(q) = NET.lock() else { return 0.0 };
        let now = Instant::now();
        let bytes: u64 = q[i].iter().filter(|(t, _)| now.duration_since(*t).as_secs_f64() <= 1.0).map(|e| e.1).sum();
        bytes as f64 * 8.0 / 1000.0
    }

    /// A property value's size on Roblox's wire (estimated: the value's own
    /// encoding, no compression).
    pub fn value_bytes(v: &Variant) -> u64 {
        match v {
            Variant::Bool(_) => 1,
            Variant::Int32(_) | Variant::Float32(_) | Variant::Ref(_) => 4,
            Variant::Int64(_) | Variant::Float64(_) | Variant::UDim(_) | Variant::Vector2(_) => 8,
            Variant::String(s) => s.len() as u64 + 2,
            Variant::BinaryString(b) => AsRef::<[u8]>::as_ref(b).len() as u64 + 4,
            Variant::Content(c) => c.as_uri().map_or(0, |u| u.len() as u64) + 2,
            Variant::ContentId(c) => c.as_str().len() as u64 + 2,
            Variant::Vector3(_) | Variant::Color3(_) => 12,
            Variant::Color3uint8(_) => 3,
            Variant::BrickColor(_) | Variant::Enum(_) => 2,
            Variant::UDim2(_) | Variant::Rect(_) => 16,
            Variant::CFrame(_) => 13,
            Variant::NumberSequence(s) => s.keypoints.len() as u64 * 12 + 4,
            Variant::ColorSequence(s) => s.keypoints.len() as u64 * 16 + 4,
            Variant::Tags(t) => t.iter().map(|x| x.len() as u64 + 1).sum::<u64>() + 2,
            Variant::Attributes(a) => a.iter().map(|(k, v)| k.len() as u64 + 2 + value_bytes(v)).sum::<u64>() + 4,
            _ => 8,
        }
    }

    fn xfer_bytes(x: &crate::runtime::Xfer) -> u64 {
        use crate::runtime::Xfer as X;
        match x {
            X::Nil => 1,
            X::Bool(_) => 2,
            X::Num(_) => 9,
            X::Str(s) => s.len() as u64 + 3,
            X::Buffer(b) => b.len() as u64 + 5,
            X::Table(t) => 3 + t.iter().map(|(k, v)| xfer_bytes(k) + xfer_bytes(v)).sum::<u64>(),
            X::Inst(_) => 5,
            X::V3(_) => 13,
            X::V2(_) => 9,
            X::CF(_) => 14,
            X::C3(_) => 13,
            X::Brick(_) | X::EnumItem(_) => 5,
            X::UDim(_) => 9,
            X::UDim2(_) => 17,
        }
    }

    /// Remote calls crossing between the session's DataModels: the remote's
    /// id, the call kind and each argument.
    pub fn remotes(to_client: bool, msgs: &[crate::runtime::RemoteMsg]) {
        let bytes: u64 = msgs.iter().map(|m| 6 + m.args.iter().map(xfer_bytes).sum::<u64>()).sum();
        net(to_client, false, bytes);
    }

    /// One replicated change: instance id + property id + value.
    pub fn prop_bytes(v: &Variant) -> u64 {
        6 + value_bytes(v)
    }
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    match key {
        "Sandboxed" if crate::doc::is_a(class, "Instance") => {
            let b = match value {
                Value::Boolean(b) => *b,
                Value::Nil => false,
                v => return Err(rt_err(format!("Unable to assign property Sandboxed. bool expected, got {}", v.type_name()))),
            };
            world(lua)?.dm().dom.get_by_ref_mut(r).map(|i| {
                i.properties.insert("Sandboxed".into(), Variant::Bool(b));
            });
            Ok(true)
        }
        // Kept in library.luau's table (see prop_get); the normal write
        // follows (type check, Changed).
        _ if class == "UserGameSettings" => {
            let t = ugs(lua)?;
            if !t.raw_get::<Value>(key)?.is_nil() {
                let mut a = MultiValue::new();
                a.push_back(Value::Nil);
                a.push_back(value.clone());
                let v = match t.raw_get::<Value>(key)? {
                    Value::UserData(u) => match u.borrow::<crate::ltypes::LEnumItem>().map(|e| e.enum_type.clone()) {
                        Ok(en) => enum_val(lua, &en, enum_arg(&a, 1, &en)?)?,
                        Err(_) => value.clone(),
                    },
                    _ => value.clone(),
                };
                // RobloxScript members (MasterVolume, CameraYInverted…): the
                // reflection database lists them as unscriptable, so the
                // normal write would refuse them as read-only. The identity
                // check already passed; finish the write here.
                if crate::runtime::security::member_write_secured(class, key) {
                    let changed = t.raw_get::<Value>(key)? != v;
                    t.raw_set(key, v)?;
                    if changed {
                        let mut w = world(lua)?;
                        w.fire(r, "Changed", vec![Value::String(lua.create_string(key)?)]);
                        w.fire(r, &format!("Changed:{key}"), vec![]);
                    }
                    return Ok(true);
                }
                t.raw_set(key, v)?;
            }
            Ok(false)
        }
        "Source" if class == "CustomEventReceiver" => {
            let new = opt_inst(value);
            if new.is_none() && !matches!(value, Value::Nil) {
                return Err(rt_err("Unable to assign property Source. Object expected"));
            }
            let old = match world(lua)?.d().prop(r, "Source") {
                Some(Variant::Ref(o)) if !o.is_none() => Some(o),
                _ => None,
            };
            if old == new {
                return Ok(true);
            }
            world(lua)?.dm().dom.get_by_ref_mut(r).map(|i| {
                i.properties.insert("Source".into(), Variant::Ref(new.unwrap_or_else(Ref::none)));
            });
            {
                let mut st = state(lua)?;
                if let Some(o) = old {
                    if let Some(v) = st.receivers.get_mut(&o) {
                        v.retain(|x| *x != r);
                    }
                }
                if let Some(n) = new {
                    st.receivers.entry(n).or_default().push(r);
                }
            }
            let me = inst_value(lua, r)?;
            if let Some(o) = old {
                let ov = inst_value(lua, o)?;
                let mut w = world(lua)?;
                w.fire(o, "ReceiverDisconnected", vec![me.clone()]);
                w.fire(r, "EventDisconnected", vec![ov]);
            }
            if let Some(n) = new {
                let nv = inst_value(lua, n)?;
                let mut w = world(lua)?;
                w.fire(n, "ReceiverConnected", vec![me.clone()]);
                w.fire(r, "EventConnected", vec![nv]);
            }
            world(lua)?.fire(r, "Changed", vec![Value::String(lua.create_string("Source")?)]);
            Ok(true)
        }
        // Any Lighting property change: LightingChanged(skyboxChanged = false)
        // (Studio: Ambient = … fires it with false), then the normal write.
        _ if class == "Lighting" && !matches!(key, "Name" | "Parent" | "Archivable") => {
            world(lua)?.fire(r, "LightingChanged", vec![Value::Boolean(false)]);
            Ok(false)
        }
        _ => Ok(false),
    }
}

pub fn write_acts(class: &str, key: &str) -> bool {
    matches!((class, key), ("CustomEventReceiver", "Source")) || class == "UserGameSettings"
        || key == "Sandboxed"
}

// ------------------------------------------------------------------ install / step

pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let rs = lua.create_table()?;
    rs.set(
        "fire_sync",
        lua.create_function(|lua, args: MultiValue| {
            let mut it = args.into_iter();
            let r = it.next().and_then(|v| opt_inst(&v)).ok_or_else(|| rt_err("fire_sync: Instance expected"))?;
            let name: String = match it.next() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(rt_err("fire_sync: event name expected")),
            };
            fire_sync(lua, r, &name, it.collect());
            Ok(())
        })?,
    )?;
    rs.set(
        "fire",
        lua.create_function(|lua, args: MultiValue| {
            let mut it = args.into_iter();
            let r = it.next().and_then(|v| opt_inst(&v)).ok_or_else(|| rt_err("fire: Instance expected"))?;
            let name: String = match it.next() {
                Some(Value::String(s)) => s.to_str()?.to_string(),
                _ => return Err(rt_err("fire: event name expected")),
            };
            world(lua)?.fire(r, &name, it.collect());
            Ok(())
        })?,
    )?;
    rs.set(
        "output",
        lua.create_function(|lua, (level, text): (String, String)| {
            use crate::app::Level;
            let mark = crate::output_panel::STACK_MARK;
            let (lv, t) = match level.as_str() {
                "warn" => (Level::Warn, text),
                "error" => (Level::Error, text),
                "info" => (Level::Info, format!("{mark}{text}")),
                _ => (Level::Info, format!("{text}  —  ")),
            };
            emit(lua, lv, t)
        })?,
    )?;
    rs.set(
        "create_logger",
        lua.create_function(|lua, (name, path): (String, String)| {
            let r = ext::create_any(lua, "Logger")?;
            if let Some(i) = world(lua)?.dm().dom.get_by_ref_mut(r) {
                i.name = name;
            }
            state(lua)?.logger_paths.insert(r, path);
            inst_value(lua, r)
        })?,
    )?;
    rs.set(
        "member",
        lua.create_function(|_, (class, name): (String, String)| {
            let Some(api) = crate::api::get() else { return Ok((None, false)) };
            Ok(match api.lookup(&class, &name) {
                Some((_, m)) => (Some(match m.kind {
                    crate::api::Kind::Property => "Property",
                    crate::api::Kind::Function => "Function",
                    crate::api::Kind::Event => "Event",
                    crate::api::Kind::Callback => "Callback",
                }), m.read_only),
                None => (None, false),
            })
        })?,
    )?;
    rs.set(
        "hook_sim",
        lua.create_function(|lua, ()| {
            state(lua)?.sim_hooked = true;
            Ok(())
        })?,
    )?;
    rs.set(
        "frame_dt",
        lua.create_function(|lua, ()| {
            let ft = state(lua)?.frame_time;
            Ok(if ft > 0.0 { ft.min(1.0 / 15.0) } else { 1.0 / 60.0 })
        })?,
    )?;
    let m: Table = lua
        .load(include_str!("services.luau"))
        .set_name("=rustudio.services")
        .call(rs)?;
    lua.set_named_registry_value("svc_impl", m)?;
    let start = lua.create_function(http_start)?;
    let poll = lua.create_function(http_poll)?;
    lua.load(HTTP_LUA).set_name("=rustudio.services.http").call::<()>((start, poll))?;
    let ser = lua.create_function(serialize)?;
    let deser = lua.create_function(deserialize)?;
    lua.load(SER_LUA).set_name("=rustudio.services.serialization").call::<()>((ser, deser))?;
    Ok(())
}

/// Fires `name` on `r` now, each handler in its own thread, as Roblox does
/// for events that fire synchronously (Logger.MessageOut).
fn fire_sync(lua: &Lua, r: Ref, name: &str, args: Vec<Value>) {
    let key = (r, name.to_string());
    let (funcs, waiters) = {
        let Ok(mut s) = signals(lua) else { return };
        let funcs: Vec<(Function, u8)> = s.conns.get(&key).map(|v| v.iter().map(|c| (c.func.clone(), c.ident)).collect()).unwrap_or_default();
        if let Some(v) = s.conns.get_mut(&key) {
            v.retain(|c| !c.once);
        }
        let waiters: Vec<mlua::Thread> = s.waiters.remove(&key).unwrap_or_default().into_iter().map(|(_, t)| t).collect();
        (funcs, waiters)
    };
    for (f, ident) in funcs {
        match lua.create_thread(f) {
            Ok(th) => {
                crate::runtime::security::set(&th, ident);
                resume(lua, &th, args.clone())
            }
            Err(e) => report(lua, &e),
        }
    }
    for th in waiters {
        resume(lua, &th, args.clone());
    }
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    let now = Instant::now();
    let hooked = {
        let mut st = state(lua)?;
        if let Some(prev) = st.last_step.replace(now) {
            st.frame_time = now.duration_since(prev).as_secs_f64();
        }
        st.step_started = Some(now);
        // The physics step's own time (play.rs's p3 section), per step.
        let (ms, calls) = crate::bench::section_total("p3_physics_step");
        let (pms, pcalls) = std::mem::replace(&mut st.physics_prev, (ms, calls));
        if calls > pcalls && ms >= pms {
            st.physics_step = (ms - pms) / (calls - pcalls) as f64 / 1000.0;
        }
        st.sim_hooked
    };
    // BindToSimulation / BindToAnimation run inside the physics step: just
    // before this frame's PostSimulation.
    if hooked {
        let mut w = world(lua)?;
        let rs = w.doc.as_ref().and_then(|d| d.find_child(d.dom.root_ref(), "RunService"));
        if let Some(rs) = rs {
            // The fire sits in `now` (Immediate places) or `events` (Deferred).
            if let Some(i) = w.now.iter().position(|e| e.0 == rs && e.1 == "PostSimulation") {
                let seq = w.now[i].3;
                let args = w.now[i].2.clone();
                w.now.insert(i, (rs, "__svc_sim".to_string(), args, seq));
            } else if let Some(i) = w.events.iter().position(|e| e.0 == rs && e.1 == "PostSimulation") {
                let (seq, depth) = (w.events[i].3, w.events[i].4);
                let args = w.events[i].2.clone();
                w.events.insert(i, (rs, "__svc_sim".to_string(), args, seq, depth));
            }
        }
    }
    tag_and_service_events(lua)?;
    flush_item_changed(lua)?;
    Ok(())
}

/// CollectionService.TagAdded / TagRemoved and ServiceProvider.ServiceAdded
/// / ServiceRemoving, worked out once a frame from what changed.
fn tag_and_service_events(lua: &Lua) -> mlua::Result<()> {
    let (added, removed, svc_new, svc_gone, root) = {
        let w = world(lua)?;
        let Some(d) = w.doc.as_ref() else { return Ok(()) };
        let root = d.dom.root_ref();
        let cs = d.find_child(root, "CollectionService");
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let _ = cs;
        TAG_FIRES.with(|q| {
            let mut q = q.borrow_mut();
            let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q).into_iter().partition(|e| e.0 == root);
            *q = rest;
            for (_, add, t) in mine {
                if add { added.push(t) } else { removed.push(t) }
            }
        });
        let (mut svc_new, mut svc_gone) = (Vec::new(), Vec::new());
        if w.listening.contains(&root) {
            let api = crate::api::get();
            let now: HashSet<Ref> = d
                .dom
                .get_by_ref(root)
                .map(|i| i.children().to_vec())
                .unwrap_or_default()
                .into_iter()
                .filter(|c| {
                    d.dom.get_by_ref(*c).is_some_and(|i| api.and_then(|a| a.classes.get(i.class.as_str())).is_some_and(|c| c.service))
                })
                .collect();
            drop(w);
            let mut st = state(lua)?;
            if let Some(prev) = &st.services {
                svc_new = now.difference(prev).copied().collect();
                svc_gone = prev.difference(&now).copied().collect();
            }
            st.services = Some(now);
        }
        (added, removed, svc_new, svc_gone, root)
    };
    if !added.is_empty() || !removed.is_empty() {
        let f = impl_fn(lua, "tag_check")?;
        f.call::<()>((lua.create_sequence_from(added)?, lua.create_sequence_from(removed)?))?;
    }
    for s in svc_new {
        let v = inst_value(lua, s)?;
        world(lua)?.fire(root, "ServiceAdded", vec![v]);
    }
    for s in svc_gone {
        let v = inst_value(lua, s)?;
        world(lua)?.fire(root, "ServiceRemoving", vec![v]);
    }
    Ok(())
}

// ------------------------------------------------------------------ ItemChanged

thread_local! {
    /// Property changes waiting to become DataModel.ItemChanged: (instance,
    /// property). Queued by `World::fire` for each Changed while the
    /// DataModel has listeners.
    static ITEM_CHANGED: std::cell::RefCell<Vec<(Ref, Ref, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// CollectionService tag fires ("TagAdded:x" / "TagRemoved:x") this
    /// frame: (DataModel root, added?, tag).
    static TAG_FIRES: std::cell::RefCell<Vec<(Ref, bool, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Called by `World::fire` for every Changed and Tag… fire (even on
/// instances nobody listens to).
pub fn on_fire(w: &World, r: Ref, name: &str, args: &[Value]) {
    let Some(d) = w.doc.as_ref() else { return };
    let root = d.dom.root_ref();
    if name != "Changed" {
        let (added, tag) = match (name.strip_prefix("TagAdded:"), name.strip_prefix("TagRemoved:")) {
            (Some(t), _) => (true, t),
            (_, Some(t)) => (false, t),
            _ => return,
        };
        if !w.listening.contains(&r) {
            return;
        }
        TAG_FIRES.with(|q| {
            let mut q = q.borrow_mut();
            if q.len() > 50_000 {
                q.clear();
            }
            q.push((root, added, tag.to_string()));
        });
        return;
    }
    if !w.listening.contains(&root) {
        return;
    }
    // A ValueBase's Changed carries the new value: its property is Value.
    let prop = match args.first() {
        Some(Value::String(s)) => s.to_string_lossy(),
        _ => "Value".to_string(),
    };
    ITEM_CHANGED.with(|q| {
        let mut q = q.borrow_mut();
        // A DataModel that stopped stepping (its session ended) never takes its own.
        if q.len() > 50_000 {
            q.clear();
        }
        q.push((root, r, prop));
    });
}

fn flush_item_changed(lua: &Lua) -> mlua::Result<()> {
    let root = world(lua)?.d().dom.root_ref();
    // Server and client share this thread: take only this DataModel's.
    let list: Vec<(Ref, String)> = ITEM_CHANGED.with(|q| {
        let mut q = q.borrow_mut();
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *q).into_iter().partition(|e| e.0 == root);
        *q = rest;
        mine.into_iter().map(|(_, r, p)| (r, p)).collect()
    });
    for (r, prop) in list {
        // Only instances still in the DataModel.
        let inside = {
            let w = world(lua)?;
            let d = w.d();
            let mut cur = Some(r);
            let mut found = false;
            while let Some(c) = cur {
                if c == root {
                    found = true;
                    break;
                }
                cur = d.dom.get_by_ref(c).map(|i| i.parent()).filter(|p| !p.is_none());
            }
            found && r != root
        };
        if !inside {
            continue;
        }
        let v = inst_value(lua, r)?;
        let p = Value::String(lua.create_string(&prop)?);
        world(lua)?.fire(root, "ItemChanged", vec![v, p]);
    }
    Ok(())
}

// ------------------------------------------------------------------ SerializationService

/// `__svc_serialize({instances})` → buffer: an rbxm (binary model), as
/// Studio's SerializeInstancesAsync writes ("<roblox!" header).
fn serialize(lua: &Lua, list: Table) -> mlua::Result<Value> {
    let mut refs = Vec::new();
    for v in list.sequence_values::<Value>() {
        match opt_inst(&v?) {
            Some(r) => refs.push(r),
            None => return Err(rt_err("Unable to cast value to Objects")),
        }
    }
    if refs.is_empty() {
        return Err(rt_err("No instances provided for serialization"));
    }
    let mut out = Vec::new();
    {
        let w = world(lua)?;
        rbx_binary::to_writer(&mut out, &w.d().dom, &refs).map_err(|e| rt_err(e.to_string()))?;
    }
    Ok(Value::Buffer(lua.create_buffer(out)?))
}

/// `__svc_deserialize(buffer)` → {instances}, parentless copies.
fn deserialize(lua: &Lua, v: Value) -> mlua::Result<Table> {
    let Value::Buffer(b) = v else { return Err(rt_err("Argument 1 missing or nil")) };
    let bytes = b.to_vec();
    if !bytes.starts_with(b"<roblox!") {
        return Err(rt_err("Serializer::checkHeaderForXML can't read header"));
    }
    let dom = rbx_binary::from_reader(std::io::Cursor::new(bytes)).map_err(|e| rt_err(e.to_string()))?;
    let roots = dom.root().children().to_vec();
    let made: Vec<Ref> = {
        let mut w = world(lua)?;
        let dest = &mut w.dm().dom;
        roots.iter().map(|r| dom.clone_into_external(*r, dest)).collect()
    };
    let t = lua.create_table()?;
    for (i, r) in made.into_iter().enumerate() {
        t.set(i + 1, inst_value(lua, r)?)?;
    }
    Ok(t)
}

const SER_LUA: &str = r#"
local ser, deser = ...
local M = __rs_lua_methods
local function clean(e) return (string.gsub(string.gsub(tostring(e), "^runtime error: ", ""), "\n.*$", "")) end
local function call(f, ...)
	local ok, r = pcall(f, ...)
	if not ok then error(clean(r), 0) end
	return r
end
M.SerializeInstancesAsync = function(self, list)
	if type(list) ~= "table" then error("Argument 1 missing or nil", 0) end
	local b = call(ser, list)
	task.wait()
	return b
end
M.DeserializeInstancesAsync = function(self, b)
	if typeof(b) ~= "buffer" then error("Argument 1 missing or nil", 0) end
	local r = call(deser, b)
	task.wait()
	return r
end
"#;
