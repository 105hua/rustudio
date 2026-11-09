//! The global environment as Roblox game scripts observe it (docs/RE-GLOBALS.md,
//! measured against Studio 0.744; anti-cheats such as an admin script probe these).
//!
//! * A script's environment holds exactly `_G`, `shared` and `script`; its
//!   metatable is locked ("The metatable is locked") and reads through to a
//!   public table: the host globals minus rustudio's own helpers
//!   (`__rs_*`, `__ac_*`, …), which stay in the real globals where the
//!   prelude, library.luau and the domains find them.
//! * `_G` is its own per-VM table (not the environment), `shared` another.
//! * Engine Lua (chunks named `=rustudio.*`) reads as C functions to
//!   scripts: the vendored Luau's `luaG_isengine` (debug.info, tracebacks,
//!   getfenv/setfenv, error positions). Threads keep the host globals (mlua's
//!   `Lua::globals` reads the running thread's), so getfenv never hands those
//!   out: it answers the environment of the nearest script frame instead
//!   (registry `rs_public_env` when there is none).
//! * tostring of address-bearing values prints Luau's encoded pointer
//!   (`lua_setpointerencodekey`, as Roblox does) with a random key per VM.
//! * collectgarbage/loadstring/version/require/task.cancel… as measured.

use mlua::{ffi, Function, Lua, Table, Thread, Value};
use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

/// The compatibility version Studio reports (docs/RE-GLOBALS.md misc.version).
pub const VERSION: &str = "0.744.0.7441077";
pub const LOCKED: &str = "The metatable is locked";

unsafe extern "C-unwind" {
    // vendor/luau0-src lapi.cpp (rustudio patch)
    fn lua_setcfunctionname(l: *mut ffi::lua_State, idx: c_int, name: *const c_char);
}

/// Per-VM pointer-encoding key. Studio's addresses are 16 lowercase hex
/// digits, stable for a live object, different between its server and
/// client VMs and between runs; a freed object's address can come back
/// (two fresh buffers printed the same). Luau's own encoder (a 4-round
/// Feistel over SipHash) keyed at random gives all of that.
pub fn init_vm(lua: &Lua) {
    use std::hash::{BuildHasher, Hasher};
    let mut words = [0u64; 4];
    for (i, w) in words.iter_mut().enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_usize(i);
        h.write_usize(lua as *const Lua as usize);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        *w = h.finish() | 2; // never one of the identity keys (1,0,0,0)/(0,1,0,0)
    }
    unsafe {
        let _ = lua.exec_raw::<()>((), |l| {
            ffi::lua_setpointerencodekey(l, words[0], words[1], words[2], words[3]);
        });
    }
}

/// Globals that are rustudio's and absent in Studio (docs/RE-GLOBALS.md
/// "Internal globals visible to game scripts" and the inventory rows Studio
/// reads as nil). `_G` and `shared` live in each script environment itself.
pub fn hidden(name: &str) -> bool {
    #[cfg(test)]
    if TEST_INTERNALS.with(|t| t.get()) && name.starts_with("__") {
        return false;
    }
    name.starts_with("__")
        || matches!(
            name,
            "_G" | "shared" | "LoadLibrary" | "integer" | "dialog_ui" | "textgen_start" | "web_get"
        )
}

// Unit tests of engine domains drive internals (`__rs_pages_list`,
// `__ac_test_collect`…) from game scripts; this module's own tests turn
// that off to see what Studio's scripts see.
#[cfg(test)]
thread_local! {
    pub static TEST_INTERNALS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

fn reg<T: mlua::FromLua>(lua: &Lua, key: &str) -> mlua::Result<T> {
    lua.named_registry_value(key)
}

/// A C function named `name` (debug.info "n", argument errors).
fn named_c(lua: &Lua, name: &str, f: ffi::lua_CFunction) -> mlua::Result<Function> {
    let name = std::ffi::CString::new(name).unwrap();
    unsafe {
        lua.exec_raw::<Function>((), |l| {
            ffi::lua_pushcclosurek(l, f, name.as_ptr(), 0, None);
        })
    }
}

/// Gives a C function (an mlua callback) its debug name if it has none.
fn name_function(lua: &Lua, f: &Function, name: &str) {
    let Ok(cname) = std::ffi::CString::new(name) else { return };
    unsafe {
        let _ = lua.exec_raw::<()>(f.clone(), |l| {
            let mut ar: ffi::lua_Debug = std::mem::zeroed();
            if ffi::lua_getinfo(l, -1, c"sn".as_ptr(), &mut ar) != 0
                && !ar.what.is_null()
                && CStr::from_ptr(ar.what).to_bytes() == b"C"
                && (ar.name.is_null() || *ar.name == 0)
            {
                lua_setcfunctionname(l, -1, cname.as_ptr());
            }
        });
    }
}

/// A distinct C function named `name` that forwards to `inner` (Studio's
/// `Spawn`/`Delay`/`ElapsedTime` are their own functions, not aliases).
unsafe extern "C-unwind" fn forward(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        let n = ffi::lua_gettop(l);
        ffi::lua_pushvalue(l, ffi::lua_upvalueindex(1));
        ffi::lua_insert(l, 1);
        ffi::lua_call(l, n, ffi::LUA_MULTRET);
        ffi::lua_gettop(l)
    }
}

fn forwarder(lua: &Lua, name: &str, inner: Function) -> mlua::Result<Function> {
    let name = std::ffi::CString::new(name).unwrap();
    unsafe {
        lua.exec_raw::<Function>(inner, |l| {
            ffi::lua_pushcclosurek(l, forward, name.as_ptr(), 1, None);
        })
    }
}

/// Raises `msg` at the caller's position, as luaL_error from a C function.
unsafe fn raise_at_caller(l: *mut ffi::lua_State, msg: &CStr) -> ! {
    unsafe {
        ffi::lua_settop(l, 0);
        ffi::luaL_where(l, 1);
        ffi::lua_pushstring(l, msg.as_ptr());
        ffi::lua_concat(l, 2);
        ffi::lua_error(l)
    }
}

/// collectgarbage: only "count" (KiB, a whole number equal to gcinfo()).
unsafe extern "C-unwind" fn collectgarbage(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        let mut len = 0usize;
        let s = ffi::luaL_checklstring(l, 1, &mut len);
        if std::slice::from_raw_parts(s as *const u8, len) == b"count" {
            ffi::lua_pushnumber(l, ffi::lua_gc(l, ffi::LUA_GCCOUNT, 0) as f64);
            return 1;
        }
        raise_at_caller(l, c"collectgarbage must be called with 'count'; use gcinfo() instead")
    }
}

/// loadstring in game scripts (ServerScriptService.LoadStringEnabled isn't a
/// property in this Studio build; both sides refuse, without a position).
unsafe extern "C-unwind" fn loadstring(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        ffi::lua_settop(l, 0);
        ffi::lua_pushstring(l, c"loadstring() is not available".as_ptr());
        ffi::lua_error(l)
    }
}

unsafe extern "C-unwind" fn version(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        ffi::lua_pushlstring(l, VERSION.as_ptr() as *const c_char, VERSION.len());
        1
    }
}

/// task.cancel(running thread) fails in Studio ("cannot cancel thread").
unsafe extern "C-unwind" fn task_cancel(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(l, 1) == ffi::LUA_TTHREAD {
            let co = ffi::lua_tothread(l, 1);
            let st = ffi::lua_costatus(l, co);
            if co == l || st == ffi::LUA_CORUN || st == ffi::LUA_CONOR {
                raise_at_caller(l, c"cannot cancel thread");
            }
        }
        forward(l)
    }
}

/// `__rs_caller_script()` for require-by-string: the `script` of the
/// function that called require (level 2: this, require, its caller), read
/// from that function's environment without getfenv (which would clear the
/// caller's safeenv and with it Luau's fast paths).
unsafe extern "C-unwind" fn caller_script(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        ffi::lua_settop(l, 0);
        let mut ar: ffi::lua_Debug = std::mem::zeroed();
        let mut level = 2;
        while ffi::lua_getinfo(l, level, c"sf".as_ptr(), &mut ar) != 0 {
            let lua_fn = !ar.what.is_null()
                && CStr::from_ptr(ar.what).to_bytes() != b"C"
                && !ar.source.is_null()
                && !CStr::from_ptr(ar.source).to_bytes().starts_with(b"=rustudio.");
            if lua_fn {
                ffi::lua_getfenv(l, -1);
                if ffi::lua_istable(l, -1) != 0 {
                    ffi::lua_pushstring(l, c"script".as_ptr());
                    ffi::lua_rawget(l, -2);
                    return 1;
                }
                ffi::lua_pop(l, 1);
            }
            ffi::lua_pop(l, 1);
            level += 1;
        }
        // Only C frames below (pcall(require, "./x")): the thread's script.
        ffi::lua_pushstring(l, c"script".as_ptr());
        ffi::lua_rawget(l, ffi::LUA_GLOBALSINDEX);
        1
    }
}

/// debug.* entries Studio has and Luau lacks (presence and metadata are
/// measured; their behaviour isn't: memory categories are kept, dumps are
/// no-ops).
unsafe extern "C-unwind" fn noop(_l: *mut ffi::lua_State) -> c_int {
    0
}

/// Seconds east of UTC for the local zone now (Studio's tick() is local
/// time: it differs from os.time() by the zone offset).
fn local_offset() -> f64 {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            0.0
        } else {
            tm.tm_gmtoff as f64
        }
    }
}

/// Combining marks / joiners that extend the previous grapheme (an
/// approximation of UAX #29 sufficient for Latin/Cyrillic/emoji text).
fn extends(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x0610..=0x061A | 0x064B..=0x065F
        | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF
        | 0x200C | 0x200D | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F
        | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F | 0xE0100..=0xE01EF)
}

/// Installs the script-visible globals. `public` is the table script
/// environments read through (the runtime's `rustudio_shared`).
pub fn install(lua: &Lua, public: &Table) -> mlua::Result<()> {
    let g = lua.globals();

    // _G: its own table per VM, shared by every script on this side.
    let script_g = lua.create_table()?;
    lua.set_named_registry_value("rs_script_G", script_g)?;

    // The environments' shared, locked metatable.
    let env_mt = lua.create_table()?;
    env_mt.raw_set("__index", public.clone())?;
    env_mt.raw_set("__metatable", LOCKED)?;
    lua.set_named_registry_value("rs_env_mt", env_mt)?;

    // Names set on the host globals after a sync still resolve (unless
    // they're rustudio's).
    let host = g.clone();
    let fallback = lua.create_function(move |_, (_t, k): (Value, Value)| {
        if let Value::String(s) = &k {
            if hidden(&s.to_string_lossy()) {
                return Ok(Value::Nil);
            }
        }
        host.raw_get::<Value>(k)
    })?;
    let pmt = lua.create_table()?;
    pmt.raw_set("__index", fallback)?;
    pmt.raw_set("__metatable", LOCKED)?;
    public.set_metatable(Some(pmt))?;

    // What getfenv answers in place of the host globals (vendor lbaselib).
    let default_env = new_env(lua, None)?;
    lua.set_named_registry_value("rs_public_env", default_env)?;

    // The string metatable is locked as well.
    unsafe {
        lua.exec_raw::<()>((), |l| {
            ffi::lua_pushstring(l, c"".as_ptr());
            if ffi::lua_getmetatable(l, -1) != 0 {
                ffi::lua_pushstring(l, c"__metatable".as_ptr());
                ffi::lua_pushstring(l, c"The metatable is locked".as_ptr());
                ffi::lua_rawset(l, -3);
            }
        })?;
    }

    // require-by-string's context.
    g.raw_set("__rs_caller_script", named_c(lua, "", caller_script)?)?;

    // Studio's debug and utf8 extras.
    let debug: Table = g.raw_get("debug")?;
    for n in ["dumpcodesize", "dumpheap", "dumprefs", "loadmodule", "tracerefs"] {
        if debug.raw_get::<Value>(n)?.is_nil() {
            debug.raw_set(n, named_c(lua, n, noop)?)?;
        }
    }
    let category = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let c1 = category.clone();
    debug.raw_set(
        "setmemorycategory",
        lua.create_function(move |_, tag: String| Ok(std::mem::replace(&mut *c1.borrow_mut(), tag)))?,
    )?;
    let c2 = category.clone();
    debug.raw_set(
        "resetmemorycategory",
        lua.create_function(move |_, ()| {
            c2.borrow_mut().clear();
            Ok(())
        })?,
    )?;
    debug.raw_set(
        "getmemorycategory",
        lua.create_function(move |_, ()| Ok(category.borrow().clone()))?,
    )?;
    let utf8: Table = g.raw_get("utf8")?;
    utf8.raw_set(
        "graphemes",
        lua.create_function(|lua, (s, i, j): (mlua::LuaString, Option<i64>, Option<i64>)| {
            let bytes = s.as_bytes().to_vec();
            let len = bytes.len() as i64;
            let norm = |v: i64| if v < 0 { (len + v + 1).max(1) } else { v };
            let start = norm(i.unwrap_or(1)).max(1) as usize;
            let end = (norm(j.unwrap_or(len)).min(len)) as usize;
            let text = String::from_utf8_lossy(&bytes[(start - 1).min(bytes.len())..end.max(start - 1)]).into_owned();
            let mut spans: Vec<(usize, usize)> = Vec::new();
            for (off, c) in text.char_indices() {
                let a = start + off;
                let b = a + c.len_utf8() - 1;
                match spans.last_mut() {
                    Some(last) if extends(c) || text[..off].ends_with('\u{200D}') => last.1 = b,
                    _ => spans.push((a, b)),
                }
            }
            let mut it = spans.into_iter();
            lua.create_function_mut(move |_, ()| {
                Ok(match it.next() {
                    Some((a, b)) => (Some(a as i64), Some(b as i64)),
                    None => (None, None),
                })
            })
        })?,
    )?;
    // (No Unicode tables in rustudio: text passes through unchanged.)
    utf8.raw_set("nfcnormalize", lua.create_function(|_, s: mlua::LuaString| Ok(s))?)?;
    utf8.raw_set("nfdnormalize", lua.create_function(|_, s: mlua::LuaString| Ok(s))?)?;

    // task.cancel refuses the running thread.
    let task: Table = g.raw_get("task")?;
    if let Ok(inner) = task.raw_get::<Function>("cancel") {
        let f = unsafe {
            lua.exec_raw::<Function>(inner, |l| {
                ffi::lua_pushcclosurek(l, task_cancel, c"cancel".as_ptr(), 1, None);
            })
        }?;
        task.raw_set("cancel", f)?;
    }

    sync(lua)
}

/// Copies the host globals scripts may see into the public table and puts
/// Studio's own versions of the measured globals over them. Call again
/// after the host globals change (bind_globals).
pub fn sync(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let public: Table = reg(lua, "rustudio_shared")?;
    // Aliases (ypcall = pcall, Spawn = spawn…) must not name the original.
    for n in ["spawn", "delay", "elapsedTime", "pcall"] {
        if let Ok(f) = g.raw_get::<Function>(n) {
            name_function(lua, &f, n);
        }
    }
    for pair in g.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let Value::String(name) = &k else { continue };
        let name = name.to_string_lossy();
        if hidden(&name) {
            continue;
        }
        if let Value::Function(f) = &v {
            name_function(lua, f, &name);
        }
        public.raw_set(k, v)?;
    }
    // Library functions keep their own names (task.spawn → "spawn", os.clock).
    for lib in ["task", "os", "debug", "coroutine", "string", "math", "table", "utf8", "buffer", "bit32", "vector"] {
        if let Ok(t) = g.raw_get::<Table>(lib) {
            for pair in t.pairs::<Value, Value>() {
                let (k, v) = pair?;
                if let (Value::String(n), Value::Function(f)) = (&k, &v) {
                    name_function(lua, f, &n.to_string_lossy());
                }
            }
        }
    }
    public.raw_set("_VERSION", "Luau")?;
    public.raw_set("collectgarbage", named_c(lua, "collectgarbage", collectgarbage)?)?;
    public.raw_set("loadstring", named_c(lua, "loadstring", loadstring)?)?;
    public.raw_set("version", named_c(lua, "version", version)?)?;
    public.raw_set("Version", named_c(lua, "Version", version)?)?;
    // elapsedTime: seconds since the process started (Studio's includes its
    // whole session), not game time; tick(): local wall time.
    let since = crate::runtime::process_start();
    let elapsed = lua.create_function(move |_, ()| Ok(since.elapsed().as_secs_f64()))?;
    name_function(lua, &elapsed, "elapsedTime");
    public.raw_set("ElapsedTime", forwarder(lua, "ElapsedTime", elapsed.clone())?)?;
    public.raw_set("elapsedTime", elapsed)?;
    let tick = lua.create_function(|_, ()| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        Ok(now + local_offset())
    })?;
    name_function(lua, &tick, "tick");
    public.raw_set("tick", tick)?;
    for (alias, of) in [("Spawn", "spawn"), ("Delay", "delay")] {
        if let Ok(f) = g.raw_get::<Function>(of) {
            public.raw_set(alias, forwarder(lua, alias, f)?)?;
        }
    }
    Ok(())
}

/// A script environment: `_G`, `shared`, `script` and nothing else, locked,
/// reading through to the public globals. Flagged safe for Luau's fast
/// paths (Luau clears it if the script calls getfenv/setfenv).
pub fn new_env(lua: &Lua, script: Option<Value>) -> mlua::Result<Table> {
    let env = lua.create_table_with_capacity(0, 4)?;
    env.raw_set("_G", reg::<Table>(lua, "rs_script_G")?)?;
    env.raw_set("shared", lua.globals().raw_get::<Value>("shared")?)?;
    if let Some(s) = script {
        env.raw_set("script", s)?;
    }
    env.set_metatable(Some(reg::<Table>(lua, "rs_env_mt")?))?;
    env.set_safeenv(true);
    Ok(env)
}

/// The test harness's environment: scripts' `_G` and `shared`, and every
/// host global (internals included).
pub fn harness_env(lua: &Lua) -> mlua::Result<Table> {
    let env = lua.create_table()?;
    env.raw_set("_G", reg::<Table>(lua, "rs_script_G")?)?;
    env.raw_set("shared", lua.globals().raw_get::<Value>("shared")?)?;
    let mt = lua.create_table()?;
    mt.raw_set("__index", lua.globals())?;
    env.set_metatable(Some(mt))?;
    Ok(env)
}

#[cfg(test)]
mod tests {
    use crate::doc::Document;
    use crate::runtime::{RunMode, Runtime, Side, World};
    use rbx_dom_weak::InstanceBuilder;

    /// Runs Scripts (server) or LocalScripts (client) for a second of game
    /// time; Output lines without their " — source" suffix.
    fn run(side: Side, scripts: &[(&str, &str)], modules: &[(&str, &str)]) -> Vec<String> {
        super::TEST_INTERNALS.with(|t| t.set(false));
        let mut d = Document::new_place();
        let root = d.dom.root_ref();
        let parent = match side {
            Side::Server => d.find_child(root, "ServerScriptService").unwrap(),
            _ => {
                let sp = d.find_child(root, "StarterPlayer").unwrap();
                d.find_child(sp, "StarterPlayerScripts").unwrap()
            }
        };
        let class = if matches!(side, Side::Server) { "Script" } else { "LocalScript" };
        let refs: Vec<_> = scripts
            .iter()
            .map(|(name, src)| {
                d.dom.insert(parent, InstanceBuilder::new(class).with_name(*name).with_property("Source", src.to_string()))
            })
            .collect();
        for (name, src) in modules {
            d.dom.insert(
                refs[0],
                InstanceBuilder::new("ModuleScript").with_name(*name).with_property("Source", src.to_string()),
            );
        }
        let rt = Runtime::new().unwrap();
        let mut w = World::new(d, RunMode::Run);
        w.side = side;
        rt.enter(w);
        rt.bind_globals().unwrap();
        for r in refs {
            rt.run_script(r);
        }
        for _ in 0..60 {
            rt.step(1.0 / 60.0);
        }
        rt.leave().output.iter().map(|(_, t)| t.split("  —").next().unwrap_or("").to_string()).collect()
    }

    fn expect(out: &[String], wants: &[&str]) {
        for want in wants {
            assert!(out.iter().any(|t| t == want), "missing {want:?} in {out:#?}");
        }
    }

    #[test]
    fn environment_is_studios() {
        let a = r#"
            local env = getfenv()
            local keys = {}
            for k in pairs(env) do table.insert(keys, tostring(k)) end
            table.sort(keys)
            print("keys", table.concat(keys, " "))
            print("mt", getmetatable(env))
            print("internals", __rs_new_signal, __ac_is_server, __rs_lua_methods, __rs_caller_script, LoadLibrary, integer)
            print("fenv", getfenv(0) == env, getfenv(print) == env, getfenv(require) == env, select(2, pcall(getfenv, 2)) == env)
            print("setfenv", pcall(setfenv, print, {}))
            _G.marker = 42
            print("G", marker, _G == env, shared == _G, rawget(env, "script") == script)
            print("version", _VERSION, version(), Version(), version ~= Version)
            shared.req = require
            task.wait(0.1)
            print("same require", rawequal(shared.req2, require))
            local ok, e = pcall(loadstring, "return 1")
            print("loadstring", ok, e)
            print("strmt", getmetatable(""))
            print("strmt set", pcall(setmetatable, "", {}))
            local cb
            workspace.ChildAdded:Connect(function() cb = getfenv(0) == env end)
            Instance.new("Part", workspace)
            task.wait(0.1)
            print("callback env", cb)
        "#;
        let b = "shared.req2 = require";
        let out = run(Side::Server, &[("A", a), ("B", b)], &[]);
        expect(
            &out,
            &[
                "keys _G script shared",
                "mt The metatable is locked",
                "internals nil nil nil nil nil nil",
                "fenv true true true true",
                "setfenv false 'setfenv' cannot change environment of given object",
                "G nil false false true",
                &format!("version Luau {v} {v} true", v = super::VERSION),
                "same require true",
                "loadstring false loadstring() is not available",
                "strmt The metatable is locked",
                "callback env true",
            ],
        );
        expect(&out, &["strmt set false invalid argument #1 to 'setmetatable' (table expected, got string)"]);
    }

    #[test]
    fn engine_functions_read_as_native() {
        let src = r#"
            local function meta(f)
                local s, l, n, a, v = debug.info(f, "slna")
                return string.format("%s %d %s %d %s", s, l, n, a, tostring(v))
            end
            for _, name in {"print", "warn", "require", "wait", "spawn", "delay", "Spawn", "Delay", "tick", "time",
                    "elapsedTime", "ElapsedTime", "collectgarbage", "loadstring", "version", "Version", "settings",
                    "stats", "UserSettings", "printidentity"} do
                print("meta", name, meta(getfenv()[name]))
            end
            for _, name in {"wait", "spawn", "defer", "delay", "cancel", "synchronize", "desynchronize"} do
                print("task", name, meta(task[name]))
            end
            print("debug", meta(debug.profilebegin), meta(os.clock))
            for _, name in {"dumpheap", "loadmodule", "setmemorycategory", "resetmemorycategory"} do
                print("debugx", name, meta(debug[name]))
            end
            print("utf8", meta(utf8.graphemes), meta(utf8.nfcnormalize))
            local tb = debug.traceback()
            print("tb clean", not tb:find("rustudio"), tb:find("ServerScriptService.T:") ~= nil)
        "#;
        let out = run(Side::Server, &[("T", src)], &[]);
        for name in [
            "print", "warn", "require", "wait", "spawn", "delay", "Spawn", "Delay", "tick", "time", "elapsedTime",
            "ElapsedTime", "collectgarbage", "loadstring", "version", "Version", "settings", "stats",
            "UserSettings", "printidentity",
        ] {
            expect(&out, &[&format!("meta {name} [C] -1 {name} 0 true")]);
        }
        for name in ["wait", "spawn", "defer", "delay", "cancel", "synchronize", "desynchronize"] {
            expect(&out, &[&format!("task {name} [C] -1 {name} 0 true")]);
        }
        expect(
            &out,
            &[
                "debug [C] -1 profilebegin 0 true [C] -1 clock 0 true",
                "debugx dumpheap [C] -1 dumpheap 0 true",
                "debugx loadmodule [C] -1 loadmodule 0 true",
                "debugx setmemorycategory [C] -1 setmemorycategory 0 true",
                "debugx resetmemorycategory [C] -1 resetmemorycategory 0 true",
                "utf8 [C] -1 graphemes 0 true [C] -1 nfcnormalize 0 true",
                "tb clean true true",
            ],
        );
    }

    #[test]
    fn tostring_scrambles_addresses() {
        let src = r#"
            local t, f, co, u, b = {}, function() end, coroutine.create(function() end), newproxy(), buffer.create(1)
            for _, v in {t, f, co, u, b} do
                local s = tostring(v)
                local hex = s:match("^%a+: 0x(%x+)$")
                print("fmt", typeof(v), hex ~= nil and #hex == 16 and hex == hex:lower(), s == tostring(v), hex:sub(1, 5) ~= "00005")
            end
            print("native", tostring(print) == tostring(print), tostring(game))
        "#;
        let out = run(Side::Server, &[("T", src)], &[]);
        for ty in ["table", "function", "thread", "userdata", "buffer"] {
            expect(&out, &[&format!("fmt {ty} true true true")]);
        }
        expect(&out, &["native true Game"]);
    }

    #[test]
    fn gc_options_are_studios() {
        let src = r#"
            print("count", collectgarbage("count") == gcinfo(), math.floor(collectgarbage("count")) == collectgarbage("count"))
            print("types", type(gcinfo()), type(collectgarbage("count")))
            for _, opt in {"collect", "stop", "step", "isrunning", "setpause", "bad"} do
                print("opt", opt, pcall(collectgarbage, opt))
            end
            print("none", pcall(collectgarbage))
            print("bool", pcall(function() return collectgarbage(true) end))
            print("num", pcall(function() return collectgarbage(23) end))
        "#;
        let out = run(Side::Server, &[("T", src)], &[]);
        let refuse = "collectgarbage must be called with 'count'; use gcinfo() instead";
        expect(&out, &["count true true", "types number number"]);
        for opt in ["collect", "stop", "step", "isrunning", "setpause", "bad"] {
            assert!(
                out.iter().any(|t| t.starts_with(&format!("opt {opt} false ")) && t.ends_with(refuse)),
                "{opt}: {out:#?}"
            );
        }
        expect(
            &out,
            &[
                "none false missing argument #1 to 'collectgarbage' (string expected)",
                "bool false ServerScriptService.T:8: invalid argument #1 to 'collectgarbage' (string expected, got boolean)",
                &format!("num false ServerScriptService.T:9: {refuse}"),
            ],
        );
    }

    #[test]
    fn require_and_scheduler_messages() {
        let src = r##"
            local function p(k, f) print(k, pcall(f)) end
            p("nil", function() return require(nil) end)
            p("table", function() return require({}) end)
            p("script", function() return require(script) end)
            p("bad path", function() return require("bad") end)
            p("cycle", function() return require(script.Cycle) end)
            p("cycle again", function() return require(script.Cycle) end)
            p("none", function() return require(script.NoResult) end)
            p("two", function() return require(script.TwoResults) end)
            p("path", function() return require("./T/Ok") end)
            p("self", function() return require("@self/Ok") end)
            p("cancel", function() return task.cancel(coroutine.running()) end)
            print("spawn", select("#", spawn(function() end)), select("#", delay(0, function() end)))
            print("task.spawn", typeof(task.spawn(function() end)), typeof(task.defer(function() end)), typeof(task.delay(0, function() end)))
            print("tick", math.abs(tick() - os.time() - (tick() - os.time())) < 1)
        "##;
        let out = run(
            Side::Server,
            &[("T", src)],
            &[
                ("Cycle", "return require(script)"),
                ("NoResult", "local x = 1"),
                ("TwoResults", "return 1, 2"),
                ("Ok", "return 'ok'"),
            ],
        );
        expect(
            &out,
            &[
                "nil false Attempted to call require with invalid argument(s).",
                "table false Attempted to call require with invalid argument(s).",
                "script false Attempted to call require with invalid argument(s).",
                "bad path false error requiring \"bad\": require path must start with a valid prefix: ./, ../, or @",
                "cycle false Requested module experienced an error while loading",
                "cycle again false Requested module experienced an error while loading",
                "none false Module code did not return exactly one value",
                "two false Module code did not return exactly one value",
                "path true ok",
                "self true ok",
                "cancel false ServerScriptService.T:13: cannot cancel thread",
                "spawn 0 0",
                "task.spawn thread thread thread",
                "Requested module was required recursively",
            ],
        );
        let client = r#"
            print("client", pcall(require, 123))
            print("client false", pcall(require, false))
        "#;
        let out = run(Side::Client, &[("C", client)], &[]);
        expect(
            &out,
            &[
                "client false require(assetId) cannot be called from a client.  assetId = 123",
                "client false false require(assetId) cannot be called from a client.  assetId = 0",
            ],
        );
    }
}
