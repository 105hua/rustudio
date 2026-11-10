//! The Luau ↔ engine bridge's C functions, as in Roblox.
//!
//! an admin script's client anti-tamper (Client.Core.Anti: indexInstance,
//! newindexInstance, namecallInstance, indexEnum, namecallEnum) takes the
//! function that raised `game.____________` (`debug.info(2, "f")` in an
//! xpcall handler) and wants a C function with no name and arity 0
//! (`debug.info` "s" `[C]`, "l" -1, "n" "", "a" 0) that fails when called
//! with bad arguments, and always with the same string. Studio's bridge goes
//! further, and scripts can see all of it:
//!
//! - metamethods (`wrap_meta`): unnamed C closures that check their
//!   arguments first with Luau's own argument errors (`missing argument #1
//!   (Instance expected)`, `invalid argument #2 (string expected, got
//!   table)`, with the caller's position), in Studio's order per type
//!   (`Spec`), then call the implementation protected and raise any failure
//!   as a string;
//! - the Instance `__namecall` (`namecall`): reads the method name
//!   (`lua_namecallatom`) on entry and hands it to a Rust dispatcher, which
//!   answers with the results, a failure, or a function to call (a method
//!   written in Lua, which may yield);
//! - method functions (`method`): C closures named after the method
//!   (`debug.info` "n"), vararg, that check their self and then either
//!   answer from Rust or call a Lua implementation through a continuation,
//!   so it can still yield (WaitForChild, InvokeServer, Signal:Wait).
//!
//! A failure message that starts with `LOCATED` gets the calling script's
//! position (`luaL_where(L, 1)`), as Luau's argument checks do; every other
//! failure is location-free, like Studio's engine errors.

use mlua::{ffi, Function, Lua, Value};
use std::ffi::{c_char, c_int, CStr, CString};

/// What `getmetatable` answers for every bridge object.
pub const LOCKED: &str = "The metatable is locked";

/// Locks the metatable of `ud`'s type (mlua refuses a `__metatable` meta
/// field, so it is set raw, once per type per VM).
pub fn lock(lua: &Lua, ud: &mlua::AnyUserData) -> mlua::Result<()> {
    unsafe {
        lua.exec_raw::<()>(ud, |l| {
            if ffi::lua_getmetatable(l, -1) != 0 {
                ffi::lua_pushstring(l, c"__metatable".as_ptr());
                ffi::lua_pushstring(l, c"The metatable is locked".as_ptr());
                ffi::lua_rawset(l, -3);
            }
            ffi::lua_settop(l, 0);
        })
    }
}

/// Prefix of a failure that takes the caller's position.
pub const LOCATED: char = '\u{1}';

/// A failure raised with the calling script's position.
pub fn located(msg: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::RuntimeError(format!("{LOCATED}{msg}"))
}

/// A metamethod's argument checks, in the order Studio makes them.
pub struct Spec {
    /// `__type`s accepted as self (#1); the first names it in errors.
    pub self_types: &'static [&'static CStr],
    /// Check the key (#2, a string or number) before self.
    pub key_first: bool,
}

pub const INSTANCE: Spec = Spec { self_types: &[c"Instance", c"Object"], key_first: false };

/// Luau's type name for a value, as its argument errors give it (`number`
/// for every number, a userdata's `__type`).
pub fn type_name(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Boolean(_) => "boolean".into(),
        Value::Integer(_) | Value::Number(_) => "number".into(),
        // Vector3 is Luau's vector (typeof "Vector3", src/dtmeta.rs).
        Value::Vector(_) => "Vector3".into(),
        Value::String(_) => "string".into(),
        Value::Table(_) => "table".into(),
        Value::Function(_) => "function".into(),
        Value::Thread(_) => "thread".into(),
        Value::UserData(u) => u
            .metatable()
            .ok()
            .and_then(|m| m.get::<String>("__type").ok())
            .unwrap_or_else(|| "userdata".into()),
        _ => "userdata".into(),
    }
}

unsafe fn check_self(l: *mut ffi::lua_State, types: &[&CStr]) {
    unsafe {
        if ffi::lua_type(l, 1) == ffi::LUA_TUSERDATA {
            let t = CStr::from_ptr(ffi::luaL_typename(l, 1));
            if types.contains(&t) {
                return;
            }
        }
        ffi::luaL_typeerror(l, 1, types[0].as_ptr());
    }
}

unsafe fn check_key(l: *mut ffi::lua_State) {
    unsafe {
        match ffi::lua_type(l, 2) {
            ffi::LUA_TSTRING => {}
            // `o[1]`: "1 is not a valid member of …"
            ffi::LUA_TNUMBER => {
                ffi::lua_tolstring(l, 2, std::ptr::null_mut());
            }
            _ => ffi::luaL_typeerror(l, 2, c"string".as_ptr()),
        }
    }
}

/// Raises the failure on top of the stack as a string. `errtext` is the
/// stack index of the error-to-string function.
unsafe fn fail(l: *mut ffi::lua_State, errtext: c_int) -> ! {
    unsafe {
        if ffi::lua_type(l, -1) != ffi::LUA_TSTRING {
            ffi::lua_pushvalue(l, errtext);
            ffi::lua_insert(l, -2);
            if ffi::lua_pcall(l, 1, 1, 0) != 0 || ffi::lua_type(l, -1) != ffi::LUA_TSTRING {
                ffi::lua_settop(l, 0);
                ffi::lua_pushstring(l, c"invalid argument".as_ptr());
            }
        }
        let mut len = 0usize;
        let s = ffi::lua_tolstring(l, -1, &mut len);
        if len > 0 && *s == LOCATED as u8 as c_char {
            ffi::lua_pushlstring(l, s.add(1), len - 1);
            ffi::luaL_where(l, 1);
            ffi::lua_insert(l, -2);
            ffi::lua_concat(l, 2);
        }
        ffi::lua_error(l)
    }
}

/// Upvalues: 1 the implementation, 2 the error-to-string function,
/// 3 whether the implementation answers `(ok, ...)` (rustudio's
/// `checked`) instead of raising, 4 the `Spec` (light userdata) or nil,
/// 5 whether to pass the key's atom after the arguments, 6 a `Fast`
/// (light userdata) or nil.
unsafe extern "C-unwind" fn meta_trampoline(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        // Upvalue 6: a fast path that answers by the key's atom without
        // the protected call (`Fast`), or declines with -1. It checks self
        // itself (exactly the bridge's own userdata, which the checks below
        // would pass), so it goes first.
        if ffi::lua_type(l, ffi::lua_upvalueindex(6)) == ffi::LUA_TLIGHTUSERDATA && ffi::lua_type(l, 2) == ffi::LUA_TSTRING {
            let mut atom: c_int = -1;
            ffi::lua_tostringatom(l, 2, &mut atom);
            // (A key with no atom, a child's name, goes too: the fast path
            // reads the key itself or declines.)
            {
                let fast: Fast = std::mem::transmute(ffi::lua_touserdata(l, ffi::lua_upvalueindex(6)));
                let r = fast(l, atom);
                if r >= 0 {
                    return r;
                }
                if r == -2 {
                    fail(l, ffi::lua_upvalueindex(2));
                }
            }
        }
        let spec = ffi::lua_touserdata(l, ffi::lua_upvalueindex(4)) as *const Spec;
        if let Some(spec) = spec.as_ref() {
            if spec.key_first {
                check_key(l);
                check_self(l, spec.self_types);
            } else {
                check_self(l, spec.self_types);
                check_key(l);
            }
        }
        let mut n = ffi::lua_gettop(l);
        // Upvalue 5 set: the implementation also takes the key's Luau atom
        // (src/runtime_routes.rs), read as Studio's bridge does.
        if ffi::lua_toboolean(l, ffi::lua_upvalueindex(5)) != 0 {
            let mut atom: c_int = -1;
            if ffi::lua_tostringatom(l, 2, &mut atom).is_null() {
                atom = -1;
            }
            ffi::lua_pushinteger(l, atom as _);
            n += 1;
        }
        ffi::lua_pushvalue(l, ffi::lua_upvalueindex(1));
        ffi::lua_insert(l, 1);
        if ffi::lua_pcall(l, n, ffi::LUA_MULTRET, 0) != 0 {
            fail(l, ffi::lua_upvalueindex(2));
        }
        if ffi::lua_toboolean(l, ffi::lua_upvalueindex(3)) != 0 {
            if ffi::lua_toboolean(l, 1) == 0 {
                ffi::lua_settop(l, 2);
                fail(l, ffi::lua_upvalueindex(2));
            }
            ffi::lua_remove(l, 1);
        }
        ffi::lua_gettop(l)
    }
}

/// The message of a failure that isn't a string: an mlua error's own text
/// (argument conversion, `… is not a valid member of "Enum.X"`). Other
/// values are never touched (a userdata's __tostring could be a script's,
/// e.g. an admin script's proxy detector).
fn error_text(lua: &Lua) -> mlua::Result<Function> {
    if let Ok(f) = lua.named_registry_value::<Function>("rs_cmeta_errtext") {
        return Ok(f);
    }
    let f = lua.create_function(|_, v: Value| {
        Ok(match v {
            Value::Error(e) => crate::runtime::describe_error(&e),
            Value::String(s) => s.to_string_lossy(),
            other => format!("invalid argument ({})", other.type_name()),
        })
    })?;
    lua.set_named_registry_value("rs_cmeta_errtext", f.clone())?;
    Ok(f)
}

/// `inner` as an unnamed C function that raises its failures as strings.
/// `checked`: `inner` answers `(true, ...)` or `(false, message)`.
pub fn wrap(lua: &Lua, inner: Function, checked: bool) -> mlua::Result<Function> {
    let errtext = error_text(lua)?;
    unsafe {
        lua.exec_raw::<Function>(
            (inner, errtext, checked, Value::Nil, false, Value::Nil, Value::Nil, Value::Nil, Value::Nil, Value::Nil),
            |l| {
                ffi::lua_pushcclosurek(l, meta_trampoline, std::ptr::null(), 10, None);
            },
        )
    }
}

/// A metamethod: `wrap`, after `spec`'s argument checks.
pub fn wrap_meta(lua: &Lua, inner: Function, checked: bool, spec: &'static Spec) -> mlua::Result<Function> {
    wrap_meta_with(lua, inner, checked, spec, false, None)
}

/// A fast path for a metamethod, run after the argument checks with the
/// key's atom (>= 0): pushes its results and returns how many; or pushes
/// a failure message and returns -2 (raised as the implementation's are);
/// or returns -1 having left the stack as it was, and the implementation
/// runs as usual. It must not raise itself. Its data (`Fast`'s second
/// half) is in upvalues 7 to 10.
pub type Fast = unsafe fn(*mut ffi::lua_State, c_int) -> c_int;

/// `wrap_meta` whose implementation also takes the key's atom (an
/// integer after the other arguments; negative: the key has none), tried
/// first by `fast` if given.
pub fn wrap_meta_atom(
    lua: &Lua,
    inner: Function,
    checked: bool,
    spec: &'static Spec,
    fast: Option<(Fast, [Value; 4])>,
) -> mlua::Result<Function> {
    wrap_meta_with(lua, inner, checked, spec, true, fast)
}

/// Gives `f` (made by `wrap_meta_atom`) its fast path afterwards: the
/// fast path's probe needs the bridge objects, which need `f` first.
pub fn set_fast(lua: &Lua, f: &Function, fast: Fast, data: [Value; 4]) -> mlua::Result<()> {
    let [d1, d2, d3, d4] = data;
    let fast = Value::LightUserData(mlua::LightUserData(fast as *mut std::ffi::c_void));
    unsafe {
        lua.exec_raw::<()>((f, fast, d1, d2, d3, d4), |l| {
            for i in (6..=10).rev() {
                ffi::lua_setupvalue(l, 1, i);
            }
            ffi::lua_settop(l, 0);
        })
    }
}

fn wrap_meta_with(
    lua: &Lua,
    inner: Function,
    checked: bool,
    spec: &'static Spec,
    atom: bool,
    fast: Option<(Fast, [Value; 4])>,
) -> mlua::Result<Function> {
    let errtext = error_text(lua)?;
    let spec = mlua::LightUserData(spec as *const Spec as *mut std::ffi::c_void);
    let (fast, [d1, d2, d3, d4]) = match fast {
        Some((f, data)) => (Value::LightUserData(mlua::LightUserData(f as *mut std::ffi::c_void)), data),
        None => (Value::Nil, [Value::Nil, Value::Nil, Value::Nil, Value::Nil]),
    };
    unsafe {
        lua.exec_raw::<Function>((inner, errtext, checked, spec, atom, fast, d1, d2, d3, d4), |l| {
            ffi::lua_pushcclosurek(l, meta_trampoline, std::ptr::null(), 10, None);
        })
    }
}

/// A registry-cached wrapped function, made on first use by `make`.
pub fn cached(
    lua: &Lua,
    key: &str,
    make: impl FnOnce(&Lua) -> mlua::Result<Function>,
) -> mlua::Result<Function> {
    if let Ok(f) = lua.named_registry_value::<Function>(key) {
        return Ok(f);
    }
    let f = make(lua)?;
    lua.set_named_registry_value(key, f.clone())?;
    Ok(f)
}

/// Results of a call that went into Lua: everything on the stack (the
/// called function sat at 1); a failure is raised on.
unsafe extern "C-unwind" fn call_cont(l: *mut ffi::lua_State, status: c_int) -> c_int {
    unsafe {
        if status != ffi::LUA_OK {
            ffi::lua_error(l);
        }
        ffi::lua_gettop(l)
    }
}

/// Calls the function at 1 with the `n` values after it; it may yield.
/// Luau's `lua_pcallyieldable`: in a yieldable thread the call runs in the
/// VM loop instead of nested in this C frame (a nested `lua_call` broke
/// Luau's fast-pcall error handling: a script `pcall` that caught an error
/// inside, e.g. in an OnInvoke, crashed the resume), and `call_cont`
/// finishes it, raising a failure on as if the call were unprotected.
unsafe fn call_lua(l: *mut ffi::lua_State, n: c_int) -> c_int {
    unsafe { ffi::lua_pcallyieldable(l, n, ffi::LUA_MULTRET, 0) }
}

/// Instance `__namecall`. Upvalues (3 to 6: see `set_fast_call`): 1 the dispatcher, called as
/// `(name, atom, self, ...)` with the name as a light userdata (a C string
/// that lives while the call's bytecode does; read it before anything else
/// runs) and its Luau atom (runtime_routes.rs; negative: none) and answering `(true, ...)`, `(false, message)` or a function to
/// call with the original arguments; 2 the error-to-string function.
unsafe extern "C-unwind" fn namecall_fn(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        let mut atom: c_int = -1;
        let name = ffi::lua_namecallatom(l, &mut atom);
        let n = ffi::lua_gettop(l);
        // Upvalue 3: a `FastCall` answering as the dispatcher would (it
        // checks self itself), pushing its results after the arguments, or
        // declining (-1).
        let mut handled = false;
        if !name.is_null() && atom >= 0 && ffi::lua_type(l, ffi::lua_upvalueindex(3)) == ffi::LUA_TLIGHTUSERDATA {
            ffi::luaL_checkstack(l, n + 4, std::ptr::null());
            let fast: FastCall = std::mem::transmute(ffi::lua_touserdata(l, ffi::lua_upvalueindex(3)));
            // Not yieldable while the fast path runs: this closure has a
            // continuation, so Luau would count a `lua_pcall` the fast
            // path makes (a Lua-written member, a signal handler) as a
            // yieldable call, and a script `pcall` inside it would be
            // scheduled for reentry instead of running nested, breaking
            // the stack (the anim curve tests). The dispatcher path runs
            // in an mlua callback's frame, which never counts.
            lua_hostcall(l, 1);
            handled = fast(l, name, atom, n) >= 0;
            lua_hostcall(l, -1);
        }
        if !handled {
            check_self(l, INSTANCE.self_types);
        }
        if name.is_null() {
            ffi::lua_pushstring(l, c"attempt to call a nil value".as_ptr());
            fail(l, ffi::lua_upvalueindex(2));
        }
        ffi::luaL_checkstack(l, n + 4, std::ptr::null());
        if !handled {
            ffi::lua_pushvalue(l, ffi::lua_upvalueindex(1));
            ffi::lua_pushlightuserdata(l, name as *mut std::ffi::c_void);
            ffi::lua_pushinteger(l, atom as _);
            for i in 1..=n {
                ffi::lua_pushvalue(l, i);
            }
            if ffi::lua_pcall(l, n + 2, ffi::LUA_MULTRET, 0) != 0 {
                fail(l, ffi::lua_upvalueindex(2));
            }
        }
        let first = n + 1;
        match ffi::lua_type(l, first) {
            ffi::LUA_TFUNCTION => {
                ffi::lua_settop(l, first);
                ffi::lua_insert(l, 1);
                call_lua(l, n)
            }
            _ if ffi::lua_toboolean(l, first) == 0 => {
                ffi::lua_settop(l, first + 1);
                fail(l, ffi::lua_upvalueindex(2));
            }
            _ => ffi::lua_gettop(l) - first,
        }
    }
}

unsafe extern "C-unwind" {
    /// vendor/luau0-src (rustudio): enter (1) / leave (-1) a stretch of
    /// host code in which Lua calls are never yieldable.
    fn lua_hostcall(l: *mut ffi::lua_State, delta: c_int);
}

/// The dispatcher run directly (no protected call) for a method name with
/// an atom: `(l, name, atom, n)` with the `n` arguments at 1..=n; pushes
/// the dispatcher's answer after them and returns >= 0, or returns -1
/// having pushed nothing. It must not raise.
pub type FastCall = unsafe fn(*mut ffi::lua_State, *const c_char, c_int, c_int) -> c_int;

/// The Instance `__namecall` over `dispatch` (see `namecall_fn`).
pub fn namecall(lua: &Lua, dispatch: Function) -> mlua::Result<Function> {
    let errtext = error_text(lua)?;
    unsafe {
        lua.exec_raw::<Function>((dispatch, errtext, Value::Nil, Value::Nil, Value::Nil, Value::Nil, Value::Nil), |l| {
            ffi::lua_pushcclosurek(l, namecall_fn, std::ptr::null(), 7, Some(call_cont));
        })
    }
}

/// Gives the `namecall` function `f` its `FastCall` afterwards, with its
/// data in upvalues 4 to 7 (as a `Fast`'s in 7 to 10).
pub fn set_fast_call(lua: &Lua, f: &Function, fast: FastCall, data: [Value; 4]) -> mlua::Result<()> {
    let [d1, d2, d3, d4] = data;
    let fast = Value::LightUserData(mlua::LightUserData(fast as *mut std::ffi::c_void));
    unsafe {
        lua.exec_raw::<()>((f, fast, d1, d2, d3, d4), |l| {
            for i in (3..=7).rev() {
                ffi::lua_setupvalue(l, 1, i);
            }
            ffi::lua_settop(l, 0);
        })
    }
}

/// A method function. Upvalues: 1 a Rust check-or-answer function (nil:
/// none) answering `(true, ...)` or `(false, message)`; 2 the
/// error-to-string function; 3 a Lua implementation (nil: none) called
/// with the original arguments once 1 passes; 4 the `__type` self must
/// have (nil: unchecked), refused with Luau's argument error naming the
/// method.
unsafe extern "C-unwind" fn method_fn(l: *mut ffi::lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(l, ffi::lua_upvalueindex(4)) == ffi::LUA_TSTRING {
            let want = CStr::from_ptr(ffi::lua_tolstring(l, ffi::lua_upvalueindex(4), std::ptr::null_mut()));
            check_self(l, &[want]);
        }
        let n = ffi::lua_gettop(l);
        let has_impl = ffi::lua_type(l, ffi::lua_upvalueindex(3)) == ffi::LUA_TFUNCTION;
        if ffi::lua_type(l, ffi::lua_upvalueindex(1)) == ffi::LUA_TFUNCTION {
            ffi::luaL_checkstack(l, n + 2, std::ptr::null());
            ffi::lua_pushvalue(l, ffi::lua_upvalueindex(1));
            if has_impl {
                for i in 1..=n {
                    ffi::lua_pushvalue(l, i);
                }
            } else {
                ffi::lua_insert(l, 1);
            }
            let first = if has_impl { n + 1 } else { 1 };
            if ffi::lua_pcall(l, n, ffi::LUA_MULTRET, 0) != 0 {
                fail(l, ffi::lua_upvalueindex(2));
            }
            if ffi::lua_toboolean(l, first) == 0 {
                ffi::lua_settop(l, first + 1);
                fail(l, ffi::lua_upvalueindex(2));
            }
            if !has_impl {
                return ffi::lua_gettop(l) - first;
            }
            ffi::lua_settop(l, n);
        }
        if !has_impl {
            return 0;
        }
        ffi::luaL_checkstack(l, 1, std::ptr::null());
        ffi::lua_pushvalue(l, ffi::lua_upvalueindex(3));
        ffi::lua_insert(l, 1);
        call_lua(l, n)
    }
}

/// A named method function (`debug.info(f, "n")` is `name`): `check`
/// (a `checked`-style Rust function) and/or `imp` (a Lua implementation,
/// which may yield), self's `__type` checked first when `self_type` is
/// given.
pub fn method(
    lua: &Lua,
    name: &str,
    check: Option<Function>,
    imp: Option<Function>,
    self_type: Option<&str>,
) -> mlua::Result<Function> {
    let errtext = error_text(lua)?;
    let cname = CString::new(name).unwrap_or_default();
    let self_type = match self_type {
        Some(t) => Value::String(lua.create_string(t)?),
        None => Value::Nil,
    };
    unsafe {
        lua.exec_raw::<Function>((check, errtext, imp, self_type), |l| {
            ffi::lua_pushcclosurek(l, method_fn, cname.as_ptr(), 4, Some(call_cont));
        })
    }
}
