//! Member routes: what answers `inst.Member` for a class, worked out once
//! per (class, member), as Studio's member descriptors are.
//!
//! Studio: Luau's `useratom` callback gives every string that is a known member name a 16-bit atom (its index
//! in the global name table); the Instance `__index` / `__namecall`
//! take the key's atom, find the member's
//! descriptor in the class's table by atom (cached per call site), check
//! security, and call the descriptor's getter. Nothing is looked up by
//! string and nothing is allocated on that path.
//!
//! rustudio's members are answered by a chain of handlers (the engine
//! domains in src/domains, runtime_ext, then the generic reflected
//! property read) that each test the class and key. A `Route` records, per
//! (class, member), which of them can answer at all (their code names the
//! member, build.rs `member_claims`), the member's kind (`Disp`) and its
//! reflected property, so a read asks only those, with no string
//! interning, reflection walk or allocation.

use super::*;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::sync::LazyLock;

// ------------------------------------------------------------- atoms

/// The global member-name table: every name of a reflected or API-dump
/// member, and every method / event the runtime itself implements.
struct Atoms {
    by_name: FxHashMap<Box<[u8]>, i16>,
    names: Vec<&'static str>,
}

static ATOMS: LazyLock<Atoms> = LazyLock::new(|| {
    let mut names: Vec<String> = Vec::new();
    let db = doc::db();
    for c in db.classes.values() {
        for p in c.properties.keys() {
            names.push(p.to_string());
        }
    }
    if let Some(api) = crate::api::get() {
        for c in api.classes.values() {
            names.extend(c.members.keys().cloned());
        }
    }
    let lists = METHODS
        .iter()
        .chain(LUA_METHODS)
        .chain(ext::METHODS)
        .chain(ext::LUA_METHODS)
        .chain(editable::METHODS)
        .chain(domains::METHODS.iter())
        .chain(domains::LUA_METHODS.iter())
        .chain(domains::EVENTS.iter());
    names.extend(lists.map(|(m, _)| m.to_string()));
    names.sort();
    names.dedup();
    let mut by_name = FxHashMap::default();
    let mut list = Vec::new();
    for (i, n) in names.into_iter().enumerate() {
        // Luau's atoms are int16; past that a name just has none (looked
        // up by its text instead).
        let Ok(a) = i16::try_from(i) else { break };
        let n: &'static str = ustr(&n).as_str();
        by_name.insert(n.as_bytes().into(), a);
        list.push(n);
    }
    Atoms { by_name, names: list }
});

/// Luau's `useratom`: called once per string (lazily, the first time its
/// atom is asked for) with the string's bytes.
unsafe extern "C-unwind" fn useratom(_l: *mut mlua::ffi::lua_State, s: *const std::ffi::c_char, len: usize) -> i16 {
    if s.is_null() || len >= 256 {
        return -1;
    }
    let b = unsafe { std::slice::from_raw_parts(s as *const u8, len) };
    ATOMS.by_name.get(b).copied().unwrap_or(-1)
}

/// Installs the atom callback on a new VM (before scripts make strings;
/// strings made earlier get their atom when it's first asked for).
pub(super) fn install_atoms(lua: &Lua) {
    LazyLock::force(&ATOMS);
    unsafe {
        let _ = lua.exec_raw::<()>((), |l| {
            (*mlua::ffi::lua_callbacks(l)).useratom = Some(useratom);
        });
    }
}

// ------------------------------------------------------------ routes

/// Members read before anything else (the class can't change them).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Special {
    None,
    Name,
    ClassName,
    Parent,
    /// A script's Enabled is the saved Disabled turned round (round 26).
    ScriptEnabled,
}

/// A reflected property: its canonical name (aliases resolved), interned
/// once, and its enum type.
#[derive(Clone, Copy)]
pub(super) struct PropInfo {
    pub canon: rbx_dom_weak::Ustr,
    pub enum_name: Option<&'static str>,
}

/// Who answers a method call (`call_method`): the core runtime, the
/// extensions (runtime_ext / editable / domains' METHODS), and which
/// domains' `call` can (domains::call_claims bits).
#[derive(Clone, Copy)]
pub(super) struct MethodRoute {
    pub core: bool,
    pub extended: bool,
    pub domains: u16,
    /// Domains whose declaration a more derived core one shadows.
    pub shadowed: u16,
}

/// A reflected property as a write sees it.
#[derive(Clone, Copy)]
pub(super) struct SetInfo {
    pub canon: rbx_dom_weak::Ustr,
    /// The value type to convert to (and the enum's name); None: a type
    /// scripts can't assign.
    pub ty: Option<(VariantType, Option<&'static str>)>,
    pub read_only: bool,
    /// The runtime or a domain acts on a plain write of it (write_acts).
    pub acts: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Route {
    pub special: Special,
    pub disp: Disp,
    /// A capability rule covers this member on this class (security.rs
    /// `rule`): the gate runs, as Studio's check before the lookup.
    pub restricted: bool,
    /// A *Async method another domain may replace per VM (Cache.overrides).
    pub overridable: bool,
    pub method: MethodRoute,
    /// Engine domains whose prop_get can answer (domains::claims bits).
    pub domains: u16,
    /// runtime_ext's prop_get can answer.
    pub ext: bool,
    /// One of core prop_get's own computed members (Bone / Attachment /
    /// BasePart frames, BrickColor, velocities).
    pub core_special: bool,
    pub prop: Option<PropInfo>,
    /// Engine domains whose newindex can take a write (newindex_claims).
    pub ni_domains: u16,
    /// runtime_ext's newindex can take it.
    pub ni_ext: bool,
    /// newindex's own Attachment / BasePart frame members.
    pub set_attachment: bool,
    pub set_part: bool,
    pub set: Option<SetInfo>,
    /// A read no handler takes part in: the name, class name or parent, or
    /// a reflected property no domain / runtime_ext / core getter computes
    /// (answered from the stored value when there is one), no gate.
    pub plain: bool,
    /// A read that runs no handler (plain, or a method / event / callback
    /// member): nothing can have been fired, no signal flush after it.
    pub quiet: bool,
    /// Not a member at all for the class (no property, method, event, stub,
    /// handler or capitalised spelling): `inst.key` is the child named key.
    pub child_only: bool,
    /// The member writes Parent, runtime_ext doesn't take it and no gate
    /// applies (claiming domains are still asked first).
    pub plain_parent_write: bool,
}

fn compute(class: rbx_dom_weak::Ustr, key: &str) -> Route {
    let c = class.as_str();
    let special = match key {
        "Name" | "name" => Special::Name,
        "ClassName" | "className" => Special::ClassName,
        "Parent" | "parent" => Special::Parent,
        "Enabled" if matches!(c, "Script" | "LocalScript") => Special::ScriptEnabled,
        _ => Special::None,
    };
    let core_special = (c == "Bone" && matches!(key, "TransformedCFrame" | "TransformedWorldCFrame"))
        || (doc::is_a(c, "Attachment")
            && matches!(
                key,
                "WorldCFrame" | "WorldPosition" | "WorldOrientation" | "Position" | "Orientation"
                    | "Axis" | "SecondaryAxis" | "WorldAxis" | "WorldSecondaryAxis"
            ))
        || (doc::is_a(c, "BasePart")
            && matches!(
                key,
                "Position" | "Orientation" | "Rotation" | "BrickColor" | "AssemblyLinearVelocity" | "Velocity"
            ));
    let prop = prop_desc(c, key).map(|(canon, dt, _)| PropInfo {
        canon: ustr(canon),
        enum_name: match &dt {
            DataType::Enum(e) => Some(ustr(e).as_str()),
            _ => None,
        },
    });
    let set = prop_desc(c, key).map(|(canon, dt, read_only)| SetInfo {
        canon: ustr(canon),
        ty: match &dt {
            DataType::Value(t) => Some((*t, None)),
            DataType::Enum(e) => Some((VariantType::Enum, Some(ustr(e).as_str()))),
            _ => None,
        },
        read_only,
        acts: ext::write_acts(c, canon) || domains::write_acts(c, canon),
    });
    let domains_bits = domains::claims(c, key);
    let restricted = security::restricted_names().is_some_and(|n| n.contains(key)) && security::rule(c, key).is_some();
    let ext_claims = domains::ext_claims(c, key);
    let d = disp(class, key);
    let plain = match special {
        Special::Name | Special::ClassName | Special::Parent => true,
        Special::ScriptEnabled => false,
        Special::None => {
            matches!(d, Disp::Other)
                && domains_bits == 0
                && !ext_claims
                && !core_special
                && prop.is_some_and(|p| !matches!(p.canon.as_str(), "OnInvoke" | "OnServerInvoke" | "OnClientInvoke"))
        }
    };
    let quiet = plain
        || (special == Special::None && !matches!(d, Disp::Other))
        // (a child lookup, or the member error)
        || (special == Special::None && domains_bits == 0 && !ext_claims && !core_special && prop.is_none());
    let ni_domains = domains::newindex_claims(c, key);
    let ni_ext = domains::ext_newindex_claims(c, key);
    let mname = match d {
        Disp::Method(m, _) => m,
        _ => key,
    };
    Route {
        special,
        disp: d,
        restricted,
        overridable: mname.ends_with("Async"),
        method: method_route(class, mname),
        domains: domains_bits,
        ext: ext_claims,
        core_special,
        prop,
        ni_domains,
        ni_ext,
        set_attachment: doc::is_a(c, "Attachment")
            && matches!(key, "WorldCFrame" | "WorldPosition" | "WorldOrientation" | "Position" | "Orientation"),
        set_part: doc::is_a(c, "BasePart")
            && matches!(key, "Position" | "Orientation" | "Rotation" | "BrickColor" | "AssemblyLinearVelocity" | "Velocity"),
        set,
        plain: plain && !restricted,
        child_only: special == Special::None
            && matches!(d, Disp::Other)
            && domains_bits == 0
            && !ext_claims
            && !core_special
            && prop.is_none()
            && !restricted
            && crate::runtime::stubs::lookup(c, key).is_none()
            && !capitalized(key).is_some_and(|k| real_member(c, &k)),
        quiet: quiet && !restricted,
        plain_parent_write: special == Special::Parent && !ni_ext && !restricted,
    }
}

/// The route of the method `name` of `class`.
pub(super) fn method_route(class: rbx_dom_weak::Ustr, name: &str) -> MethodRoute {
    let (core, extended) = method_owner(class, name);
    let core_base = core
        .then(|| METHODS.iter().find(|(m, b)| *m == name && doc::is_a(class.as_str(), b)).map(|(_, b)| *b))
        .flatten();
    let (domains, shadowed) = domains::call_claims_shadow(class.as_str(), name, core, core_base);
    MethodRoute { core, extended, domains, shadowed }
}

thread_local! {
    static BY_ATOM: RefCell<FxHashMap<(usize, i32), Route>> = RefCell::new(FxHashMap::default());
    static BY_NAME: RefCell<FxHashMap<usize, FxHashMap<Box<str>, Route>>> = RefCell::new(FxHashMap::default());
}

/// The route for `class.key`; `atom` is the key's Luau atom (or negative:
/// none, looked up by its text).
pub(super) fn route(class: rbx_dom_weak::Ustr, key: &str, atom: i32) -> Route {
    let cid = class.as_str().as_ptr() as usize;
    if atom >= 0 {
        if let Some(r) = BY_ATOM.with(|m| m.borrow().get(&(cid, atom)).copied()) {
            return r;
        }
        let r = compute(class, key);
        BY_ATOM.with(|m| m.borrow_mut().insert((cid, atom), r));
        return r;
    }
    if let Some(r) = BY_NAME.with(|m| m.borrow().get(&cid).and_then(|t| t.get(key).copied())) {
        return r;
    }
    let r = compute(class, key);
    BY_NAME.with(|m| m.borrow_mut().entry(cid).or_default().insert(key.into(), r));
    r
}

// ------------------------------------------------- the fast read path

/// mlua's userdata block for an `LInst` / `LObj`: its length, its first
/// word (the storage variant), and where the `Ref` sits (behind the
/// pointer at +8, at +16 in the box). Read off a probe userdata at start
/// (`probe_layout`); the fast path is off unless the probe matched.
const UD_LEN: usize = 32;
const UD_REF_AT: usize = 16;
static UD_WORD0: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The `Ref` in the instance userdata at `p` (UD_LEN bytes).
unsafe fn ud_ref(p: *const u8) -> Option<Ref> {
    unsafe {
        let w = p as *const u64;
        if w.read_unaligned() != UD_WORD0.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let rc = (w.add(1).read_unaligned()) as *const u8;
        if rc.is_null() {
            return None;
        }
        Some((rc.add(UD_REF_AT) as *const Ref).read_unaligned())
    }
}

/// Checks the layout on probe userdata: returns the LInst and LObj
/// metatables when it holds.
fn probe_layout(lua: &Lua) -> Option<[Value; 2]> {
    use mlua::ffi;
    let refs = [Ref::new(), Ref::new()];
    let a = lua.create_userdata(LInst(refs[0])).ok()?;
    let b = lua.create_userdata(LObj(refs[1])).ok()?;
    let mut ok = true;
    let mut mts = Vec::new();
    for (u, want) in [(&a, refs[0]), (&b, refs[1])] {
        PROBE.with(|c| c.set(None));
        let mt = unsafe {
            lua.exec_raw::<Value>(u, |l| {
                if ffi::lua_objlen(l, -1) as usize == UD_LEN {
                    let p = ffi::lua_touserdata(l, -1) as *const u64;
                    let w0 = p.read_unaligned();
                    let rc = p.add(1).read_unaligned() as *const u8;
                    if !rc.is_null() && (rc as usize) % 8 == 0 {
                        let r = (rc.add(UD_REF_AT) as *const Ref).read_unaligned();
                        PROBE.with(|c| c.set(Some((w0, r))));
                    }
                }
                if ffi::lua_getmetatable(l, -1) == 0 {
                    ffi::lua_pushnil(l);
                }
                ffi::lua_replace(l, 1);
                ffi::lua_settop(l, 1);
            })
            .ok()?
        };
        match PROBE.with(|c| c.take()) {
            Some((w0, r)) if r == want && matches!(mt, Value::Table(_)) => {
                let prev = UD_WORD0.swap(w0, std::sync::atomic::Ordering::Relaxed);
                ok &= prev == 0 || prev == w0;
            }
            _ => ok = false,
        }
        mts.push(mt);
    }
    if !ok {
        return None;
    }
    let b = mts.pop()?;
    let a = mts.pop()?;
    Some([a, b])
}

thread_local! {
    static PROBE: std::cell::Cell<Option<(u64, Ref)>> = const { std::cell::Cell::new(None) };
}

/// The Instance `__index` fast path (cmeta `Fast`) and its data: the
/// LInst / LObj metatables and a registry slot for handing results back.
/// None (the full path only) when the userdata layout didn't match.
pub(super) fn fast_index_fn(lua: &Lua) -> Option<(crate::cmeta::Fast, [Value; 4])> {
    if std::env::var_os("RUSTUDIO_NO_FAST_INDEX").is_some() {
        return None;
    }
    let [a, b] = probe_layout(lua)?;
    // A registry slot mlua owns (its registry accessors take no protected
    // call), for moving values between mlua and the calling thread's
    // stack. Kept for the VM's life.
    let key: &'static mut mlua::RegistryKey = Box::leak(Box::new(lua.create_registry_value(true).ok()?));
    let slot = Value::LightUserData(mlua::LightUserData(key as *mut mlua::RegistryKey as *mut std::ffi::c_void));
    // The instance cache's weak table: cached Instance userdata pushed
    // straight from it.
    let weak = lua.app_data_ref::<Cache>().and_then(|c| c.weak.clone()).map(Value::Table).unwrap_or(Value::Nil);
    Some((fast_index as crate::cmeta::Fast, [a, b, slot, weak]))
}

/// The registry slot in upvalue `up` (see `fast_index_fn`). (A pointer:
/// a nested call uses the same slot, each use complete before Lua runs.)
unsafe fn slot_key(l: *mut mlua::ffi::lua_State, up: std::ffi::c_int) -> *mut mlua::RegistryKey {
    unsafe { mlua::ffi::lua_touserdata(l, mlua::ffi::lua_upvalueindex(up)) as *mut mlua::RegistryKey }
}

/// Pushes `v` onto `l`: plain values directly, the rest through the
/// registry slot (mlua puts values on its own current stack).
unsafe fn push_value(lua: &Lua, l: *mut mlua::ffi::lua_State, key: *mut mlua::RegistryKey, v: Value) -> bool {
    use mlua::ffi;
    unsafe {
        let key = &mut *key;
        match v {
            Value::Nil => ffi::lua_pushnil(l),
            Value::Boolean(b) => ffi::lua_pushboolean(l, b as _),
            Value::Integer(i) => ffi::lua_pushnumber(l, i as f64),
            Value::Number(n) => ffi::lua_pushnumber(l, n),
            Value::Vector(v) => ffi::lua_pushvector(l, v.x(), v.y(), v.z()),
            other => {
                if lua.replace_registry_value(key, other).is_err() {
                    return false;
                }
                let id = key.id() as _;
                ffi::lua_rawgeti(l, ffi::LUA_REGISTRYINDEX, id);
                ffi::lua_pushboolean(l, 1);
                ffi::lua_rawseti(l, ffi::LUA_REGISTRYINDEX, id);
            }
        }
        true
    }
}

/// Pushes a fast read's answer: an instance from the cache's weak table
/// (upvalue `weak`) when it is there, else through `inst_value`.
unsafe fn push_fv(
    lua: &Lua,
    l: *mut mlua::ffi::lua_State,
    key: *mut mlua::RegistryKey,
    weak: std::ffi::c_int,
    v: Fv,
) -> mlua::Result<bool> {
    use mlua::ffi;
    unsafe {
        match v {
            Fv::V(v) => Ok(push_value(lua, l, key, v)),
            Fv::Str(t) => {
                ffi::lua_pushlstring(l, t.as_ptr() as *const _, t.len());
                Ok(true)
            }
            Fv::Static(t) => {
                ffi::lua_pushlstring(l, t.as_ptr() as *const _, t.len());
                Ok(true)
            }
            Fv::Insts(refs) => {
                ffi::lua_createtable(l, refs.len() as _, 0);
                ffi::luaL_checkstack(l, 2, std::ptr::null());
                for (i, r) in refs.into_iter().enumerate() {
                    if !push_fv(lua, l, key, weak, Fv::Inst(r))? {
                        return Ok(false);
                    }
                    ffi::lua_rawseti(l, -2, (i + 1) as _);
                }
                Ok(true)
            }
            Fv::Inst(r) if r.is_none() => {
                ffi::lua_pushnil(l);
                Ok(true)
            }
            Fv::Inst(r) => {
                if ffi::lua_type(l, ffi::lua_upvalueindex(weak)) == ffi::LUA_TTABLE {
                    if let Some(slot) = cached_inst_slot(lua, r) {
                        if ffi::lua_rawgeti(l, ffi::lua_upvalueindex(weak), slot as _) == ffi::LUA_TUSERDATA {
                            return Ok(true);
                        }
                        ffi::lua_settop(l, -2);
                    }
                }
                let v = inst_value(lua, r)?;
                Ok(push_value(lua, l, key, v))
            }
        }
    }
}

/// Studio's Instance `__index`:
/// the C function goes straight to the member by the key's atom. Here:
/// the routed `index_entry` called directly from the C `__index`, without
/// the protected call into an mlua function, its argument conversion and
/// result tables; the value comes back through the registry slot (upvalue
/// 9). Same gates, handlers, signal flush and failures as the full path,
/// which still answers keys with no atom, scripts' own userdata
/// (`newproxy`), and runs with per-call diagnostics on.
unsafe fn fast_index(l: *mut mlua::ffi::lua_State, atom: std::ffi::c_int) -> std::ffi::c_int {
    use mlua::ffi;
    unsafe {
        // Self: exactly an Instance / Object of this VM (upvalues 7, 8).
        if ffi::lua_getmetatable(l, 1) == 0 {
            return -1;
        }
        let ours = ffi::lua_rawequal(l, -1, ffi::lua_upvalueindex(7)) != 0
            || ffi::lua_rawequal(l, -1, ffi::lua_upvalueindex(8)) != 0;
        ffi::lua_settop(l, -2);
        if !ours || ffi::lua_objlen(l, 1) as usize != UD_LEN {
            return -1;
        }
        let Some(r) = ud_ref(ffi::lua_touserdata(l, 1) as *const u8) else { return -1 };
        if crate::bench::call_probes_on() {
            return -1;
        }
        // (No atom: a child's name, read off the stack.)
        let key: &str = if atom >= 0 {
            let Some(&key) = ATOMS.names.get(atom as usize) else { return -1 };
            key
        } else {
            let mut len = 0usize;
            let p = ffi::lua_tolstring(l, 2, &mut len);
            if p.is_null() {
                return -1;
            }
            match std::str::from_utf8(std::slice::from_raw_parts(p as *const u8, len)) {
                Ok(k) => k,
                Err(_) => return -1,
            }
        };
        let slot = slot_key(l, 9);
        let lua = Lua::get_or_init_from_ptr(l);
        // (mlua's current thread: the caller, as in its own callbacks)
        lua.with_current_state(l, || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let (v, quiet) = crate::bench::api_timed("index", key, || index_entry_fast(lua, r, key, atom));
                if !quiet {
                    signal_timing::flush_immediate(lua);
                }
                v
            }));
            match r {
                Ok(Ok(v)) => match push_fv(lua, l, slot, 10, v) {
                    Ok(true) => 1,
                    Ok(false) => {
                        ffi::lua_pushstring(l, c"internal error (stack)".as_ptr());
                        -2
                    }
                    Err(e) => {
                        let m = raw_error_message(&e);
                        ffi::lua_pushlstring(l, m.as_ptr() as *const _, m.len());
                        -2
                    }
                },
                Ok(Err(e)) => {
                    let m = raw_error_message(&e);
                    ffi::lua_pushlstring(l, m.as_ptr() as *const _, m.len());
                    -2
                }
                // A panic: the full path runs it again under mlua's handler.
                Err(_) => -1,
            }
        })
    }
}

/// The value at `i` on `l` as an mlua value: plain values directly, the
/// rest through the registry slot.
unsafe fn stack_value(
    lua: &Lua,
    l: *mut mlua::ffi::lua_State,
    i: std::ffi::c_int,
    key: *mut mlua::RegistryKey,
) -> mlua::Result<Value> {
    use mlua::ffi;
    unsafe {
        let key = &*key;
        Ok(match ffi::lua_type(l, i) {
            ffi::LUA_TNIL => Value::Nil,
            ffi::LUA_TBOOLEAN => Value::Boolean(ffi::lua_toboolean(l, i) != 0),
            ffi::LUA_TNUMBER => Value::Number(ffi::lua_tonumber(l, i)),
            ffi::LUA_TVECTOR => {
                let v = ffi::lua_tovector(l, i);
                Value::Vector(mlua::Vector::new(*v, *v.add(1), *v.add(2)))
            }
            _ => {
                let id = key.id() as _;
                ffi::lua_pushvalue(l, i);
                ffi::lua_rawseti(l, ffi::LUA_REGISTRYINDEX, id);
                let v = lua.registry_value::<Value>(key);
                ffi::lua_pushboolean(l, 1);
                ffi::lua_rawseti(l, ffi::LUA_REGISTRYINDEX, id);
                v?
            }
        })
    }
}

/// The Instance / Object of this VM at `i` (metatables in upvalues `mt`,
/// `mt + 1`): its Ref.
unsafe fn arg_ref(l: *mut mlua::ffi::lua_State, i: std::ffi::c_int, mt: std::ffi::c_int) -> Option<Ref> {
    use mlua::ffi;
    unsafe {
        if ffi::lua_getmetatable(l, i) == 0 {
            return None;
        }
        let ours = ffi::lua_rawequal(l, -1, ffi::lua_upvalueindex(mt)) != 0
            || ffi::lua_rawequal(l, -1, ffi::lua_upvalueindex(mt + 1)) != 0;
        ffi::lua_settop(l, -2);
        if !ours || ffi::lua_objlen(l, i) as usize != UD_LEN {
            return None;
        }
        ud_ref(ffi::lua_touserdata(l, i) as *const u8)
    }
}

/// Self at 1 is exactly an Instance / Object of this VM (metatables in
/// upvalues `mt` and `mt + 1`): its Ref.
unsafe fn self_ref(l: *mut mlua::ffi::lua_State, mt: std::ffi::c_int) -> Option<Ref> {
    use mlua::ffi;
    unsafe {
        if ffi::lua_getmetatable(l, 1) == 0 {
            return None;
        }
        let ours = ffi::lua_rawequal(l, -1, ffi::lua_upvalueindex(mt)) != 0
            || ffi::lua_rawequal(l, -1, ffi::lua_upvalueindex(mt + 1)) != 0;
        ffi::lua_settop(l, -2);
        if !ours || ffi::lua_objlen(l, 1) as usize != UD_LEN {
            return None;
        }
        ud_ref(ffi::lua_touserdata(l, 1) as *const u8)
    }
}

/// The Instance `__newindex` as `fast_index`: the routed
/// `newindex_entry` straight from the C function.
unsafe fn fast_newindex(l: *mut mlua::ffi::lua_State, atom: std::ffi::c_int) -> std::ffi::c_int {
    use mlua::ffi;
    unsafe {
        let Some(r) = self_ref(l, 7) else { return -1 };
        if crate::bench::call_probes_on() {
            return -1;
        }
        let Some(&key) = ATOMS.names.get(atom as usize) else { return -1 };
        let slot = slot_key(l, 9);
        let lua = Lua::get_or_init_from_ptr(l);
        // (mlua's current thread: the caller, as in its own callbacks)
        lua.with_current_state(l, || {
            // `x.Parent = p` (Place E's Emitter2D: 372 a frame, nearly all to the
            // parent it has): the Ref straight off the stack, a no-op answered
            // in one borrow.
            if key == "Parent" && !domains::route_check_on() {
                let p = match ffi::lua_type(l, 3) {
                    ffi::LUA_TNIL => Some(Ref::none()),
                    ffi::LUA_TUSERDATA => arg_ref(l, 3, 7),
                    _ => None,
                };
                if let Some(p) = p {
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::bench::api_timed("newindex", key, || parent_write(lua, r, key, atom, p))
                    }));
                    match res {
                        Ok(None) => {}
                        Ok(Some(Ok(quiet))) => {
                            if !quiet {
                                signal_timing::flush_immediate(lua);
                            }
                            return 0;
                        }
                        Ok(Some(Err(e))) => {
                            signal_timing::flush_immediate(lua);
                            let m = raw_error_message(&e);
                            ffi::lua_pushlstring(l, m.as_ptr() as *const _, m.len());
                            return -2;
                        }
                        Err(_) => {
                            ffi::lua_pushstring(l, c"internal error".as_ptr());
                            return -2;
                        }
                    }
                }
            }
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let value = stack_value(lua, l, 3, slot)?;
                let v = crate::bench::api_timed("newindex", key, || newindex_entry(lua, r, key, value, atom));
                signal_timing::flush_immediate(lua);
                v
            }));
            match res {
                Ok(Ok(())) => 0,
                Ok(Err(e)) => {
                    let m = raw_error_message(&e);
                    ffi::lua_pushlstring(l, m.as_ptr() as *const _, m.len());
                    -2
                }
                Err(_) => {
                    ffi::lua_pushstring(l, c"internal error".as_ptr());
                    -2
                }
            }
        })
    }
}

/// The Instance `__newindex` fast path and its data (see `fast_index_fn`).
pub(super) fn fast_newindex_fn(lua: &Lua) -> Option<(crate::cmeta::Fast, [Value; 4])> {
    let (_, data) = fast_index_fn(lua)?;
    Some((fast_newindex as crate::cmeta::Fast, data))
}

/// The Instance `__namecall`: the method by the name's atom,
/// dispatched (`namecall`) straight from the C function, its arguments
/// converted without an mlua call; the answer is pushed after them in the
/// dispatcher's form ((true, results…), (false, message) or a Lua
/// method's function). Data in upvalues 4 to 6.
unsafe fn fast_namecall(
    l: *mut mlua::ffi::lua_State,
    _name: *const std::ffi::c_char,
    atom: std::ffi::c_int,
    n: std::ffi::c_int,
) -> std::ffi::c_int {
    use mlua::ffi;
    unsafe {
        if n < 1 || self_ref(l, 4).is_none() {
            return -1;
        }
        if crate::bench::call_probes_on() {
            return -1;
        }
        let Some(&name) = ATOMS.names.get(atom as usize) else { return -1 };
        let slot = slot_key(l, 6);
        let lua = Lua::get_or_init_from_ptr(l);
        // (mlua's current thread: the caller, as in its own callbacks)
        lua.with_current_state(l, || {
            // A native method (`native_call`): plain arguments off the stack.
            if n <= 4 && !domains::route_check_on() {
                let mut nargs: [NArg; 3] = [NArg::Other, NArg::Other, NArg::Other];
                for i in 2..=n {
                    nargs[(i - 2) as usize] = match ffi::lua_type(l, i) {
                        ffi::LUA_TNIL => NArg::Nil,
                        ffi::LUA_TBOOLEAN => NArg::Bool(ffi::lua_toboolean(l, i) != 0),
                        ffi::LUA_TSTRING => {
                            let mut len = 0usize;
                            let p = ffi::lua_tolstring(l, i, &mut len);
                            match std::str::from_utf8(std::slice::from_raw_parts(p as *const u8, len)) {
                                Ok(s) => NArg::Str(s),
                                Err(_) => NArg::Other,
                            }
                        }
                        ffi::LUA_TUSERDATA => match arg_ref(l, i, 4) {
                            Some(o) => NArg::Inst(o),
                            None => NArg::Other,
                        },
                        _ => NArg::Other,
                    };
                }
                let r = self_ref(l, 4).unwrap_or_else(Ref::none);
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::bench::api_timed("call", name, || native_call(lua, r, name, atom, &nargs[..(n - 1) as usize]))
                }));
                match res {
                    Ok(None) => {}
                    Ok(Some(Ok((v, quiet)))) => {
                        if !quiet {
                            signal_timing::flush_immediate(lua);
                        }
                        ffi::lua_pushboolean(l, 1);
                        match push_fv(lua, l, slot, 7, v) {
                            Ok(true) => return 0,
                            Ok(false) => {}
                            Err(e) => {
                                let m = raw_error_message(&e);
                                ffi::lua_settop(l, n);
                                ffi::lua_pushboolean(l, 0);
                                ffi::lua_pushlstring(l, m.as_ptr() as *const _, m.len());
                                return 0;
                            }
                        }
                        ffi::lua_settop(l, n);
                        ffi::lua_pushboolean(l, 0);
                        ffi::lua_pushstring(l, c"internal error (stack)".as_ptr());
                        return 0;
                    }
                    Ok(Some(Err(e))) => {
                        signal_timing::flush_immediate(lua);
                        // (as `checked` words a method's failure)
                        let m = raw_error_message(&e);
                        ffi::lua_pushboolean(l, 0);
                        ffi::lua_pushlstring(l, m.as_ptr() as *const _, m.len());
                        return 0;
                    }
                    Err(_) => return -1,
                }
            }
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut args = MultiValue::with_capacity(n as usize);
                for i in 1..=n {
                    args.push_back(stack_value(lua, l, i, slot)?);
                }
                namecall(lua, name, atom, args)
            }));
            let out = match res {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => {
                    let m = describe_error(&e);
                    ffi::lua_pushboolean(l, 0);
                    ffi::lua_pushlstring(l, m.as_ptr() as *const _, m.len());
                    return 0;
                }
                Err(_) => {
                    ffi::lua_pushboolean(l, 0);
                    ffi::lua_pushstring(l, c"internal error".as_ptr());
                    return 0;
                }
            };
            ffi::luaL_checkstack(l, out.len() as std::ffi::c_int + 2, std::ptr::null());
            for v in out {
                if !push_value(lua, l, slot, v) {
                    ffi::lua_settop(l, n);
                    ffi::lua_pushboolean(l, 0);
                    ffi::lua_pushstring(l, c"internal error (stack)".as_ptr());
                    return 0;
                }
            }
            0
        })
    }
}

/// The Instance `__namecall` fast path and its data.
pub(super) fn fast_namecall_fn(lua: &Lua) -> Option<(crate::cmeta::FastCall, [Value; 4])> {
    let (_, data) = fast_index_fn(lua)?;
    Some((fast_namecall as crate::cmeta::FastCall, data))
}
