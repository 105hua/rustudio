//! Thread identity and capabilities, as measured in Studio (Studio probes,
//! 2026-11-08).
//!
//! Every Luau thread carries an identity in its thread data
//! (`lua_setthreaddata`): 2 for game scripts (server Script, client
//! LocalScript, the modules they require, everything they spawn) and 1 for
//! the command bar (rustudio's `=harness`). A new thread copies its
//! parent's (Luau's `userthread` callback runs inside `lua_newthread`, so
//! coroutine.create/wrap, task.spawn/defer/delay and spawn/delay all
//! inherit); a script's first thread is set to 2 when it starts, and a
//! signal handler runs at the identity of the thread that connected it.
//! Unset (the main thread outside the harness) reads as 2.
//!
//! Checks follow the API dump: a member's Security (read / write / call /
//! connect / wait) names one legacy capability, its Capabilities list the
//! sandbox ones. Measured order: sandbox capabilities first, and a write
//! checks the read requirements before the write ones (Instance.UniqueId
//! write reports RobloxScript, its read tag; RunService.RunState write
//! reports PluginOrOpenCloud). NotScriptable members stay "not a valid
//! member". The message is Studio's exe format string:
//! `The current thread cannot {verb} '{name}' (lacking capability {cap})`.
//!
//! rustudio's own Lua (chunks named `rustudio.*` and a few other engine
//! chunks) acts on behalf of scripts: when the nearest Lua frame belongs to
//! it, the check passes. The prelude is transparent (its Signal:Connect /
//! Wait are the script's own call), C frames are skipped, and a thread with
//! no Lua frame at all is Rust driving the engine only on the main thread.

use super::*;
use mlua::ffi;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::OnceLock;

/// Game scripts (Script, LocalScript, their modules and threads).
pub const GAME: u8 = 2;
/// The command bar (rustudio's harness).
pub const COMMAND: u8 = 1;

// Legacy capability bits.
const PLUGIN: u8 = 0x1;
const LOCAL_USER: u8 = 0x2;
const WRITE_PLAYER: u8 = 0x4;
const ROBLOX_SCRIPT: u8 = 0x8;
const ROBLOX_ENGINE: u8 = 0x10;
const NOT_ACCESSIBLE: u8 = 0x20;

// Sandbox capabilities some identity lacks (every other one in the dump,
// Basic, UI, Physics…, is granted to game scripts).
const SB_INTERNAL_TEST: u8 = 0x1;
const SB_PLUGIN_OR_OPEN_CLOUD: u8 = 0x2;
const SB_REMOTE_COMMAND: u8 = 0x4;
const SB_ASSISTANT: u8 = 0x8;
const SANDBOX: [(u8, &str); 4] = [
    (SB_INTERNAL_TEST, "InternalTest"),
    (SB_PLUGIN_OR_OPEN_CLOUD, "PluginOrOpenCloud"),
    (SB_REMOTE_COMMAND, "RemoteCommand"),
    (SB_ASSISTANT, "Assistant"),
];

/// Legacy capabilities an identity holds. Measured: identity 2 lacks
/// Plugin, LocalUser, RobloxScript, RobloxEngine, NotAccessible and
/// WritePlayer; identity 1 can call settings/PluginManager and read
/// DataCost but not SourceAssetId (RobloxScript) or Player.InputLatency
/// (RobloxEngine).
fn legacy_granted(id: u8) -> u8 {
    match id {
        // WritePlayer for the command bar is unmeasured (harness tests make
        // Players); game scripts measured without it.
        COMMAND => PLUGIN | LOCAL_USER | WRITE_PLAYER,
        _ => 0,
    }
}

/// Sandbox capabilities enforced from the dump: only what was measured.
/// RunService.RunState / Misprediction (PluginOrOpenCloud) are refused to
/// game scripts. InternalTest is not refused uniformly in Studio
/// (GeometryService:CreateBasicMeshPart answers "This API is not yet
/// released!"), so InternalTest / RemoteCommand members keep their
/// per-member, measured gates in the domains.
fn sandbox_denied(id: u8) -> u8 {
    let _ = (SB_INTERNAL_TEST, SB_REMOTE_COMMAND, SB_ASSISTANT);
    match id {
        COMMAND => 0,
        _ => SB_PLUGIN_OR_OPEN_CLOUD,
    }
}

fn legacy_name(bit: u8) -> &'static str {
    match bit {
        PLUGIN => "Plugin",
        LOCAL_USER => "LocalUser",
        WRITE_PLAYER => "WritePlayer",
        ROBLOX_SCRIPT => "RobloxScript",
        ROBLOX_ENGINE => "RobloxEngine",
        _ => "NotAccessible",
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Req {
    legacy: u8,
    sandbox: u8,
}

impl Req {
    fn is_free(self) -> bool {
        self.legacy == 0 && self.sandbox == 0
    }
    /// The capability `id` lacks for this, if any (sandbox first).
    fn missing(self, id: u8) -> Option<&'static str> {
        let denied = self.sandbox & sandbox_denied(id);
        if denied != 0 {
            return SANDBOX.iter().find(|(b, _)| denied & b != 0).map(|(_, n)| *n);
        }
        if self.legacy != 0 && legacy_granted(id) & self.legacy == 0 {
            return Some(legacy_name(self.legacy));
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MKind {
    Property,
    Function,
    Event,
    Callback,
}

#[derive(Debug)]
pub struct Rule {
    pub class: &'static str,
    pub kind: MKind,
    pub read: Req,
    pub write: Req,
    /// Tagged NotScriptable: scripts get "not a valid member" (measured on
    /// the Workspace *Alias properties) before any capability diagnostic.
    pub not_scriptable: bool,
}

struct Rules {
    by_name: FxHashMap<&'static str, Vec<Rule>>,
    supers: FxHashMap<&'static str, &'static str>,
}

fn legacy_of(tag: &str) -> u8 {
    match tag {
        "PluginSecurity" => PLUGIN,
        "LocalUserSecurity" => LOCAL_USER,
        "RobloxScriptSecurity" => ROBLOX_SCRIPT,
        "RobloxSecurity" => ROBLOX_ENGINE,
        "NotAccessibleSecurity" => NOT_ACCESSIBLE,
        _ => 0,
    }
}

fn sandbox_of(list: Option<&serde_json::Value>) -> u8 {
    let mut out = 0;
    for c in list.and_then(|l| l.as_array()).into_iter().flatten() {
        if let Some((b, _)) = SANDBOX.iter().find(|(_, n)| Some(*n) == c.as_str()) {
            out |= b;
        }
    }
    out
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn dump_path() -> std::path::PathBuf {
    std::env::var_os("RUSTUDIO_API_DUMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/api-dump.json").into())
}

fn parse(text: &str) -> Option<Rules> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut rules = Rules { by_name: FxHashMap::default(), supers: FxHashMap::default() };
    let mut names: FxHashMap<String, &'static str> = FxHashMap::default();
    let mut intern = |s: &str| *names.entry(s.to_string()).or_insert_with(|| leak(s));
    for c in v.get("Classes")?.as_array()? {
        let class = intern(c.get("Name")?.as_str()?);
        if let Some(sup) = c.get("Superclass").and_then(|s| s.as_str()).filter(|s| *s != "<<<ROOT>>>") {
            let sup = intern(sup);
            rules.supers.insert(class, sup);
        }
        for m in c.get("Members").and_then(|m| m.as_array()).into_iter().flatten() {
            let Some(name) = m.get("Name").and_then(|n| n.as_str()) else { continue };
            let kind = match m.get("MemberType").and_then(|t| t.as_str()) {
                Some("Property") => MKind::Property,
                Some("Function") => MKind::Function,
                Some("Event") => MKind::Event,
                Some("Callback") => MKind::Callback,
                _ => continue,
            };
            let not_scriptable = m
                .get("Tags")
                .and_then(|t| t.as_array())
                .is_some_and(|a| a.iter().any(|x| x.as_str() == Some("NotScriptable")));
            let (rs, ws) = match m.get("Security") {
                Some(serde_json::Value::String(s)) => (legacy_of(s), legacy_of(s)),
                Some(o) => (
                    legacy_of(o.get("Read").and_then(|r| r.as_str()).unwrap_or("None")),
                    legacy_of(o.get("Write").and_then(|r| r.as_str()).unwrap_or("None")),
                ),
                None => (0, 0),
            };
            let (rc, wc) = match m.get("Capabilities") {
                // Write-side Capabilities alone are unmeasured (RunState's
                // write fails on its Read list): not enforced.
                Some(serde_json::Value::Object(o)) => (sandbox_of(o.get("Read")), {
                    let _ = o.get("Write");
                    0
                }),
                Some(l) => (sandbox_of(Some(l)), 0),
                None => (0, 0),
            };
            let rule = Rule {
                class,
                kind,
                read: Req { legacy: rs, sandbox: rc },
                write: Req { legacy: ws, sandbox: wc },
                not_scriptable,
            };
            if rule.read.is_free() && rule.write.is_free() && !not_scriptable {
                continue;
            }
            let name = intern(name);
            rules.by_name.entry(name).or_default().push(rule);
        }
    }
    Some(rules)
}

fn rules() -> Option<&'static Rules> {
    static RULES: OnceLock<Option<Rules>> = OnceLock::new();
    RULES
        .get_or_init(|| parse(&std::fs::read_to_string(dump_path()).ok()?))
        .as_ref()
}

/// Every restricted member name (any class): the hot path's one lookup.
pub fn restricted_names() -> Option<&'static FxHashSet<&'static str>> {
    static NAMES: OnceLock<FxHashSet<&'static str>> = OnceLock::new();
    let r = rules()?;
    Some(NAMES.get_or_init(|| r.by_name.keys().copied().collect()))
}

/// The restricted rule for `class.name` (the most derived declaration).
pub fn rule(class: &str, name: &str) -> Option<&'static Rule> {
    let r = rules()?;
    let list = r.by_name.get(name)?;
    let mut cur: &str = class;
    for _ in 0..64 {
        if let Some(x) = list.iter().find(|x| x.class == cur) {
            return Some(x);
        }
        cur = r.supers.get(cur)?;
    }
    None
}

// ------------------------------------------------------- thread identity

fn state_of(th: &Thread) -> *mut ffi::lua_State {
    th.to_pointer() as *mut ffi::lua_State
}

unsafe fn ident_of(l: *mut ffi::lua_State) -> u8 {
    match unsafe { ffi::lua_getthreaddata(l) } as usize {
        0 => GAME,
        n => (n - 1) as u8,
    }
}

unsafe fn set_ident(l: *mut ffi::lua_State, id: u8) {
    unsafe { ffi::lua_setthreaddata(l, (id as usize + 1) as *mut std::ffi::c_void) }
}

/// Luau's thread-creation callback: a new thread takes its parent's identity.
unsafe extern "C-unwind" fn userthread(parent: *mut ffi::lua_State, child: *mut ffi::lua_State) {
    if !parent.is_null() {
        unsafe { ffi::lua_setthreaddata(child, ffi::lua_getthreaddata(parent)) }
    }
}

/// Installs the inheritance callback (once per VM, before any script runs).
pub fn install_thread_hook(lua: &Lua) -> mlua::Result<()> {
    unsafe {
        lua.exec_raw::<()>((), |state| {
            (*ffi::lua_callbacks(state)).userthread = Some(userthread);
        })
    }
}

/// The running thread's identity.
pub fn current(lua: &Lua) -> u8 {
    unsafe { ident_of(state_of(&lua.current_thread())) }
}

pub fn set(th: &Thread, id: u8) {
    unsafe { set_ident(state_of(th), id) }
}

pub fn of(th: &Thread) -> u8 {
    unsafe { ident_of(state_of(th)) }
}

/// Runs `f` with the main thread at `id` (the harness), restoring it after
/// (also on unwind).
pub fn with_main_identity<R>(lua: &Lua, id: u8, f: impl FnOnce() -> R) -> R {
    struct Restore(*mut ffi::lua_State, *mut std::ffi::c_void);
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe { ffi::lua_setthreaddata(self.0, self.1) }
        }
    }
    let mut main: *mut ffi::lua_State = std::ptr::null_mut();
    let _ = unsafe {
        lua.exec_raw::<()>((), |state| {
            main = ffi::lua_mainthread(state);
        })
    };
    if main.is_null() {
        return f();
    }
    let _restore = Restore(main, unsafe { ffi::lua_getthreaddata(main) });
    unsafe { set_ident(main, id) };
    f()
}

// --------------------------------------------------------------- callers

/// Chunks of rustudio's own that aren't named `rustudio.*`.
fn internal_chunk(src: &[u8]) -> bool {
    let s = src.strip_prefix(b"=").unwrap_or(src);
    s.starts_with(b"rustudio.")
        || s.starts_with(b"gui_input.")
        || s.starts_with(b"CoreGui.RobloxGui.CoreScripts/")
        || s == b"StudioTestService"
        || s == b"geometry_ops"
        || s == b"geometry_nav"
        || s == b"web_social_http"
        || s == b"stub-default"
}

/// Who is asking: Some(true) rustudio's own Lua, Some(false) a script (or
/// the harness), None no Lua frame on this thread.
fn caller_is_internal(l: *mut ffi::lua_State) -> Option<bool> {
    unsafe {
        for level in 0..256 {
            let mut ar: ffi::lua_Debug = std::mem::zeroed();
            if ffi::lua_getinfo(l, level, c"s".as_ptr(), &mut ar) == 0 {
                return None;
            }
            if ar.source.is_null() {
                continue;
            }
            let what = if ar.what.is_null() { &b""[..] } else { std::ffi::CStr::from_ptr(ar.what).to_bytes() };
            if what == b"C" {
                continue;
            }
            let src = std::ffi::CStr::from_ptr(ar.source).to_bytes();
            if src == b"=rustudio.prelude" {
                continue;
            }
            return Some(internal_chunk(src));
        }
    }
    None
}

/// Whether the running code may do something `id` lacks a capability for:
/// rustudio's own Lua, or Rust driving the engine on the main thread.
fn internal(lua: &Lua) -> bool {
    let th = lua.current_thread();
    let l = state_of(&th);
    match caller_is_internal(l) {
        Some(b) => b,
        None => unsafe { ffi::lua_mainthread(l) == l },
    }
}

pub fn denial(verb: &str, name: &str, cap: &str) -> mlua::Error {
    rt_err(format!("The current thread cannot {verb} '{name}' (lacking capability {cap})"))
}

/// Checks `req` (and `first`, a write's read requirements) for the running
/// thread.
fn check(lua: &Lua, verb: &str, name: &str, first: Option<Req>, req: Req) -> mlua::Result<()> {
    let id = current(lua);
    let missing = first.and_then(|f| f.missing(id)).or_else(|| req.missing(id));
    match missing {
        Some(cap) if !internal(lua) => Err(denial(verb, name, cap)),
        _ => Ok(()),
    }
}

/// Checks `req` for the running thread's identity alone, with no bypass for
/// rustudio's Lua: for C functions a script holds (settings, a gated
/// method), which internal code may be handed as a callback (a
/// BindableFunction's OnInvoke…) and must not then lend its trust to.
fn check_identity(lua: &Lua, verb: &str, name: &str, req: Req) -> mlua::Result<()> {
    match req.missing(current(lua)) {
        Some(cap) => Err(denial(verb, name, cap)),
        None => Ok(()),
    }
}

/// A global that wants a legacy capability (settings, PluginManager).
/// rustudio's own Lua never calls either.
pub fn check_global(lua: &Lua, name: &str) -> mlua::Result<()> {
    check_identity(lua, "call", name, Req { legacy: PLUGIN, sandbox: 0 })
}

// ------------------------------------------------------------------ hooks

/// Instance.new: a Player wants WritePlayer (`The current thread cannot
/// create 'Player' (lacking capability WritePlayer)`).
pub fn check_create(lua: &Lua, class: &str) -> mlua::Result<()> {
    // Identity only: rustudio's Lua wraps Instance.new (assets_cloud
    // gates), so the nearest frame says nothing; no engine Lua makes Players.
    if class == "Player" {
        if let Some(cap) = (Req { legacy: WRITE_PLAYER, sandbox: 0 }).missing(current(lua)) {
            let th = lua.current_thread();
            let l = state_of(&th);
            let rust = caller_is_internal(l).is_none() && unsafe { ffi::lua_mainthread(l) == l };
            if !rust {
                return Err(denial("create", class, cap));
            }
        }
    }
    Ok(())
}

/// GetService / FindService("CoreGui") answer nil to game scripts (measured
/// in both Play contexts).
pub fn service_hidden(lua: &Lua, name: &str) -> bool {
    name == "CoreGui" && current(lua) == GAME && !internal(lua)
}

#[inline]
fn maybe_restricted(key: &str) -> bool {
    restricted_names().is_some_and(|n| n.contains(key))
}

/// `inst.key` (before the member is looked up): a property read is
/// checked here; a restricted method comes back gated (checked again at
/// the call, by whoever calls it); a restricted event's Connect / Wait are
/// checked by the prelude's Signal.
pub fn index_hook(lua: &Lua, r: Ref, key: &str) -> mlua::Result<Option<Value>> {
    if !maybe_restricted(key) {
        return Ok(None);
    }
    let (class, _) = inst_class(lua, r)?;
    let Some(rule) = rule(class.as_str(), key) else { return Ok(None) };
    if rule.not_scriptable {
        return if internal(lua) { Ok(None) } else { Err(not_member(lua, r, class.as_str(), key)) };
    }
    match rule.kind {
        MKind::Property => {
            check(lua, "read", key, None, rule.read)?;
            Ok(None)
        }
        MKind::Function => {
            if check(lua, "call", key, None, rule.read).is_ok() {
                return Ok(None);
            }
            gate(lua, rule, key).map(|f| Some(Value::Function(f)))
        }
        MKind::Event => {
            if check(lua, "connect", key, None, rule.read).is_ok() {
                return Ok(None);
            }
            // Studio's event exists even where rustudio has no behaviour:
            // a signal whose Connect / Wait refuse.
            match index(lua, r, key) {
                Ok(v) => Ok(Some(v)),
                Err(_) => signal(lua, r, key).map(Some),
            }
        }
        MKind::Callback => Ok(None),
    }
}

/// `inst.key = v`: the read requirements, then the write ones.
pub fn newindex_hook(lua: &Lua, r: Ref, key: &str) -> mlua::Result<()> {
    if !maybe_restricted(key) {
        return Ok(());
    }
    let (class, _) = inst_class(lua, r)?;
    match rule(class.as_str(), key) {
        Some(rule) if rule.not_scriptable && !internal(lua) => Err(not_member(lua, r, class.as_str(), key)),
        Some(rule) if rule.not_scriptable => Ok(()),
        Some(rule) if rule.kind == MKind::Property => check(lua, "write", key, Some(rule.read), rule.write),
        _ => Ok(()),
    }
}

fn not_member(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Error {
    let full = world(lua).map(|w| w.full_name(r)).unwrap_or_default();
    rt_err(format!("{key} is not a valid member of {class} \"{full}\""))
}

/// Connect / Wait on a restricted event (prelude Signal): (true) or
/// (false, message).
fn event_check(lua: &Lua, (inst, name, verb): (LInst, String, String)) -> mlua::Result<MultiValue> {
    let (class, _) = inst_class(lua, inst.0)?;
    let r = match rule(class.as_str(), &name) {
        Some(rule) if rule.kind == MKind::Event && !rule.not_scriptable => check(lua, &verb, &name, None, rule.read),
        _ => Ok(()),
    };
    checked(lua, r.map(|_| MultiValue::new()))
}

/// A restricted method as a script sees it: an unnamed C function that
/// checks whoever calls it, then calls the real member.
fn gate(lua: &Lua, rule: &'static Rule, key: &str) -> mlua::Result<Function> {
    let cache_key = format!("{}.{}", rule.class, key);
    let cache: Table = match lua.named_registry_value::<Table>("rs_sec_gates") {
        Ok(t) => t,
        Err(_) => {
            let t = lua.create_table()?;
            lua.set_named_registry_value("rs_sec_gates", t.clone())?;
            t
        }
    };
    if let Ok(Value::Function(f)) = cache.raw_get::<Value>(cache_key.as_str()) {
        return Ok(f);
    }
    let name = key.to_string();
    let inner = lua.create_function(move |lua, args: MultiValue| {
        let r = (|| {
            // Internal code indexing the member gets the real function, so
            // only the identity counts here.
            check_identity(lua, "call", &name, rule.read)?;
            let this = args.front().and_then(opt_inst).ok_or_else(|| {
                rt_err(format!("Expected ':' not '.' calling member function {name}"))
            })?;
            match index(lua, this, &name)? {
                Value::Function(f) => f.call::<MultiValue>(args),
                _ => Err(rt_err(format!("attempt to call a {} value", "non-function"))),
            }
        })();
        checked(lua, r)
    })?;
    let f = crate::cmeta::wrap(lua, inner, true)?;
    cache.raw_set(cache_key, f.clone())?;
    Ok(f)
}

/// Globals for the prelude and library (captured into locals there, then
/// removed by `seal`): the running identity, the event check, and the set
/// of restricted event names.
pub fn install_globals(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    g.set("__rs_identity", lua.create_function(|lua, ()| Ok(current(lua)))?)?;
    g.set("__rs_sec_event", lua.create_function(event_check)?)?;
    let names = lua.create_table()?;
    if let Some(r) = rules() {
        for (name, list) in &r.by_name {
            if list.iter().any(|x| x.kind == MKind::Event) {
                names.raw_set(*name, true)?;
            }
        }
    }
    g.set("__rs_sec_event_names", names)?;
    Ok(())
}

/// After the library: settings / PluginManager as C functions that check
/// the caller (Studio's results carry no source position), and the
/// install-time globals gone.
pub fn seal(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let the_settings: Value = match g.get::<Value>("settings")? {
        Value::Function(f) => f.call(())?,
        _ => Value::Nil,
    };
    let settings = lua.create_function(move |lua, ()| {
        let r = check_global(lua, "settings").map(|_| MultiValue::from_iter([the_settings.clone()]));
        checked(lua, r)
    })?;
    g.set("settings", crate::cmeta::wrap(lua, settings, true)?)?;
    let pm: Table = lua
        .load(
            r#"
            local pm = setmetatable({}, { __tostring = function() return "PluginManager" end, __type = "Instance" })
            function pm:CreatePlugin()
                return setmetatable({}, { __tostring = function() return "Plugin" end })
            end
            return pm
            "#,
        )
        .set_name("=rustudio.security")
        .eval()?;
    let plugin_manager = lua.create_function(move |lua, ()| {
        let r = check_global(lua, "PluginManager").map(|_| MultiValue::from_iter([Value::Table(pm.clone())]));
        checked(lua, r)
    })?;
    g.set("PluginManager", crate::cmeta::wrap(lua, plugin_manager, true)?)?;
    // DebuggerManager(): Plugin, as settings.
    let debugger_manager = lua.create_function(|lua, ()| {
        let r = check_global(lua, "DebuggerManager").and_then(|_| {
            // (GetService hides it from game scripts; domains/testing_debug.)
            let found = {
                let w = world(lua)?;
                let d = w.d();
                d.find_child(d.dom.root_ref(), "DebuggerManager")
            };
            let dm = match found {
                Some(r) => inst_value(lua, r)?,
                None => Value::Nil,
            };
            Ok(MultiValue::from_iter([dm]))
        });
        checked(lua, r)
    })?;
    g.set("DebuggerManager", crate::cmeta::wrap(lua, debugger_manager, true)?)?;
    // Instance.new as Roblox's: a C function whose failures (WritePlayer,
    // "Unable to create…") reach pcall as plain strings.
    if let Ok(inst) = g.get::<Table>("Instance") {
        if let Ok(Value::Function(f)) = inst.raw_get::<Value>("new") {
            inst.raw_set("new", crate::cmeta::wrap(lua, f, false)?)?;
        }
    }
    for k in ["__rs_identity", "__rs_sec_event", "__rs_sec_event_names"] {
        g.raw_set(k, Value::Nil)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client runtime with a LocalScript `src` in StarterPlayerScripts
    /// (not started); returns (runtime, script).
    fn client(src: &str) -> (Runtime, Ref) {
        let mut d = Document::new_place();
        let sp = d.find_child(d.dom.root_ref(), "StarterPlayer").unwrap();
        let sps = d.find_child(sp, "StarterPlayerScripts").unwrap();
        let s = d.dom.insert(
            sps,
            InstanceBuilder::new("LocalScript").with_name("T").with_property("Source", src.to_string()),
        );
        let rt = Runtime::new().unwrap();
        let mut w = World::new(d, RunMode::Run);
        w.side = Side::Client;
        rt.enter(w);
        rt.bind_globals().unwrap();
        (rt, s)
    }

    fn harness(rt: &Runtime, code: &str) {
        let r = with_main_identity(&rt.lua, COMMAND, || rt.lua.load(code).set_name("=harness").exec());
        r.unwrap();
    }

    fn lines(rt: Runtime) -> Vec<String> {
        rt.leave()
            .output
            .iter()
            .map(|(_, t)| t.split("  —").next().unwrap_or("").to_string())
            .collect()
    }

    fn has(out: &[String], want: &str) -> bool {
        out.iter().any(|t| t == want)
    }

    const DENY_SETTINGS: &str = "The current thread cannot call 'settings' (lacking capability Plugin)";

    /// Game script identity 2, Studio's exact messages (no source prefix),
    /// inherited by every thread the script makes.
    #[test]
    fn game_script_identity_and_inheritance() {
        let (rt, s) = client(
            r#"
            printidentity() printidentity("pre")
            print("settings", pcall(settings))
            print("pm", pcall(PluginManager))
            local function ident(n) print(n, select(2, pcall(settings))) end
            task.spawn(ident, "spawn") task.defer(ident, "defer") task.delay(0, ident, "delay")
            coroutine.resume(coroutine.create(function() ident("create") end))
            coroutine.wrap(function() ident("wrap") end)()
            xpcall(function() error("x") end, function() ident("xpcall") end)
            local z = setmetatable({}, {__index = function() ident("meta") end}); local _ = z.x
            local b = Instance.new("BindableEvent"); b.Event:Connect(function() ident("bindable") end); b:Fire()
            print("spawnc", pcall(function() return coroutine.wrap(settings)() end))
            local f = Instance.new("Folder")
            print("read", pcall(function() return f.DataCost end))
            print("write", pcall(function() f.RobloxLocked = nil end))
            print("uid", pcall(function() f.UniqueId = nil end))
            print("call", pcall(function() return f:GetDebugId() end))
            print("dot", pcall(function() return f.GetDebugId(f) end))
            print("state", pcall(function() return game:GetService("RunService").RunState end))
            print("http", pcall(function() return game:GetService("HttpService").HttpEnabled end))
            print("hw", pcall(function() game:GetService("HttpService").HttpEnabled = true end))
            print("ev", pcall(function() return game:GetService("StarterGui").CoreGuiChangedSignal:Connect(print) end))
            print("wait", pcall(function() return game:GetService("StarterGui").CoreGuiChangedSignal:Wait() end))
            print("mis", pcall(function() return game:GetService("RunService").Misprediction:Connect(print) end))
            print("player", pcall(Instance.new, "Player"))
            print("coregui", game:GetService("CoreGui"))
            print("alias", pcall(function() return workspace.SignalBehaviorAlias end))
            print("stats", tostring(stats()), tostring(UserSettings()))
        "#,
        );
        rt.run_script(s);
        for _ in 0..3 {
            rt.step(1.0 / 60.0);
        }
        let out = lines(rt);
        for want in [
            "Current identity is 2".to_string(),
            "pre 2".to_string(),
            format!("settings false {DENY_SETTINGS}"),
            "pm false The current thread cannot call 'PluginManager' (lacking capability Plugin)".to_string(),
            format!("spawn {DENY_SETTINGS}"),
            format!("defer {DENY_SETTINGS}"),
            format!("delay {DENY_SETTINGS}"),
            format!("create {DENY_SETTINGS}"),
            format!("wrap {DENY_SETTINGS}"),
            format!("xpcall {DENY_SETTINGS}"),
            format!("meta {DENY_SETTINGS}"),
            format!("bindable {DENY_SETTINGS}"),
            "read false The current thread cannot read 'DataCost' (lacking capability LocalUser)".to_string(),
            "write false The current thread cannot write 'RobloxLocked' (lacking capability Plugin)".to_string(),
            "uid false The current thread cannot write 'UniqueId' (lacking capability RobloxScript)".to_string(),
            "call false The current thread cannot call 'GetDebugId' (lacking capability Plugin)".to_string(),
            "dot false The current thread cannot call 'GetDebugId' (lacking capability Plugin)".to_string(),
            "state false The current thread cannot read 'RunState' (lacking capability PluginOrOpenCloud)".to_string(),
            "http true false".to_string(),
            "hw false The current thread cannot write 'HttpEnabled' (lacking capability LocalUser)".to_string(),
            "ev false The current thread cannot connect 'CoreGuiChangedSignal' (lacking capability RobloxScript)".to_string(),
            "wait false The current thread cannot wait 'CoreGuiChangedSignal' (lacking capability RobloxScript)".to_string(),
            "mis false The current thread cannot connect 'Misprediction' (lacking capability PluginOrOpenCloud)".to_string(),
            "player false The current thread cannot create 'Player' (lacking capability WritePlayer)".to_string(),
            "coregui nil".to_string(),
            "alias false SignalBehaviorAlias is not a valid member of Workspace \"Workspace\"".to_string(),
            "stats Stats GlobalBasicSettings".to_string(),
        ] {
            assert!(has(&out, &want), "missing {want:?} in {out:#?}");
        }
        // coroutine.wrap adds the caller's position to a string error.
        assert!(out.iter().any(|t| t.starts_with("spawnc false ") && t.ends_with(DENY_SETTINGS)), "{out:#?}");
    }

    /// The command bar (harness) is identity 1: settings and Plugin /
    /// LocalUser members work, RobloxScript / RobloxEngine ones don't, and
    /// what it spawns keeps 1.
    #[test]
    fn harness_is_command_identity() {
        let (rt, _) = client("");
        harness(
            &rt,
            r#"
            printidentity()
            print("settings", (pcall(settings)))
            print("pm", tostring(PluginManager()))
            local f = Instance.new("Folder")
            print("datacost", (pcall(function() return f.DataCost end)))
            print("debugid", (pcall(function() return f:GetDebugId() end)))
            print("src", pcall(function() return f.SourceAssetId end))
            task.spawn(function() printidentity("spawned") end)
            coroutine.wrap(function() printidentity("wrapped") end)()
            print("coregui", game:GetService("CoreGui") ~= nil)
        "#,
        );
        harness(&rt, "printidentity('again')");
        let out = lines(rt);
        for want in [
            "Current identity is 1",
            "settings true",
            "pm PluginManager",
            "datacost true",
            "src false The current thread cannot read 'SourceAssetId' (lacking capability RobloxScript)",
            "spawned 1",
            "wrapped 1",
            "again 1",
            "coregui true",
        ] {
            assert!(has(&out, want), "missing {want:?} in {out:#?}");
        }
        // GetDebugId exists in Studio; rustudio may not implement it, but
        // the capability itself must not be what fails.
        assert!(!out.iter().any(|t| t.contains("cannot call 'GetDebugId'")), "{out:#?}");
    }

    /// A handler runs at its connector's identity, whoever fires (measured
    /// both ways with a BindableEvent).
    #[test]
    fn handlers_run_at_connector_identity() {
        let (rt, s) = client(
            r#"
            local b = Instance.new("BindableEvent"); b.Name = "B1"; b.Parent = workspace
            b.Event:Connect(function() printidentity("script-connected") end)
            task.wait()
            workspace:WaitForChild("B2"):Fire()
        "#,
        );
        harness(
            &rt,
            r#"local b = Instance.new("BindableEvent"); b.Name = "B2"
               b.Event:Connect(function() printidentity("cmd-connected") end); b.Parent = workspace"#,
        );
        rt.run_script(s);
        harness(&rt, "workspace.B1:Fire()");
        for _ in 0..4 {
            rt.step(1.0 / 60.0);
        }
        let out = lines(rt);
        assert!(has(&out, "script-connected 2"), "{out:#?}");
        assert!(has(&out, "cmd-connected 1"), "{out:#?}");
    }

    /// A content id gives a game script Roblox content, not the person's
    /// files: `file://` and plain paths don't load, and nothing of the image
    /// reaches the script.
    #[test]
    fn content_ids_do_not_read_local_files() {
        let dir = std::env::temp_dir().join(format!("rustudio-content-ids-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("private.png");
        image::RgbaImage::from_pixel(1, 1, image::Rgba([12, 34, 56, 255]))
            .save(&png)
            .unwrap();
        let path = png.display().to_string();
        let (rt, s) = client(&format!(
            r#"
            local assets = game:GetService("AssetService")
            for _, id in {{ {file:?}, {path:?} }} do
                local ok, image = pcall(function() return assets:CreateEditableImageAsync(Content.fromUri(id)) end)
                print("loaded", ok, if ok then buffer.readu8(image:ReadPixelsBuffer(Vector2.zero, Vector2.one), 0) else nil)
            end
        "#,
            file = format!("file://{path}"),
        ));
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = lines(rt);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            out.iter().filter(|t| *t == "loaded false nil").count(),
            2,
            "{out:#?}"
        );
    }

    /// A module runs at its requirer's identity, once per identity: a
    /// command-bar require doesn't hand game scripts its result.
    #[test]
    fn module_cache_per_identity() {
        let (rt, s) = client(r#"print("game got", require(workspace:WaitForChild("M")).id)"#);
        harness(
            &rt,
            r#"local m = Instance.new("ModuleScript"); m.Name = "M"
               m.Source = "printidentity('module') return { id = pcall(settings) and 'cmd' or 'game' }"
               m.Parent = workspace
               print("cmd got", require(m).id)"#,
        );
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = lines(rt);
        assert!(has(&out, "module 1") && has(&out, "module 2"), "{out:#?}");
        assert!(has(&out, "cmd got cmd"), "{out:#?}");
        assert!(has(&out, "game got game"), "{out:#?}");
    }

    /// rustudio's own Lua reaches restricted members on a script's behalf;
    /// the same access from the script, or through a restricted C function
    /// it hands internal code to call, is refused.
    #[test]
    fn internal_code_is_not_blocked() {
        let (rt, s) = client(
            r#"
            local f = Instance.new("Folder")
            print("via library", secTestPeek(f))
            print("direct", (pcall(function() return f.DataCost end)))
            print("call through", (pcall(secTestCall, settings)))
            print("gate through", (pcall(secTestCall, f.GetDebugId, f)))
        "#,
        );
        rt.lua
            .load(
                r#"function secTestPeek(f) return (pcall(function() return f.DataCost end)) end
                   function secTestCall(fn, ...) return fn(...) end"#,
            )
            .set_name("=rustudio.test")
            .exec()
            .unwrap();
        rt.run_script(s);
        rt.step(1.0 / 60.0);
        let out = lines(rt);
        assert!(has(&out, "via library true"), "{out:#?}");
        assert!(has(&out, "direct false"), "{out:#?}");
        // A script's restricted function stays refused when internal Lua
        // is the one calling it (no observable bypass).
        assert!(has(&out, "call through false"), "{out:#?}");
        assert!(has(&out, "gate through false"), "{out:#?}");
    }

    #[test]
    fn dump_rules_match_measured_tags() {
        if rules().is_none() {
            return;
        }
        let r = rule("Folder", "DataCost").unwrap();
        assert_eq!(r.read.missing(GAME), Some("LocalUser"));
        assert_eq!(r.read.missing(COMMAND), None);
        let u = rule("Folder", "UniqueId").unwrap();
        assert_eq!(u.read.missing(GAME), Some("RobloxScript"));
        assert_eq!(u.write.missing(GAME), Some("RobloxEngine"));
        let rs = rule("RunService", "RunState").unwrap();
        assert_eq!(rs.read.missing(GAME), Some("PluginOrOpenCloud"));
        assert_eq!(rs.read.missing(COMMAND), None);
        assert!(rule("Workspace", "SignalBehaviorAlias").unwrap().not_scriptable);
        assert!(rule("Part", "Anchored").is_none());
        assert_eq!(rule("Player", "InputLatency").unwrap().read.missing(COMMAND), Some("RobloxEngine"));
        assert_eq!(rule("Instance", "SourceAssetId").unwrap().read.missing(COMMAND), Some("RobloxScript"));
    }
}
