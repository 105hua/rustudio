//! Stubs: every member a game script may reach (src/api_stubs.rs, generated
//! by tools/gen_stubs.py from api-dump.json) exists even before rustudio
//! implements it, so no game dies on "is not a valid member" or indexing nil.
//! The runtime consults this only after its own lookups fail, so real
//! implementations always win:
//!
//! - a property reads its typed default (or what a script last wrote to it);
//!   writes are kept per instance and fire Changed, read-only ones refuse;
//! - a function returns its typed empty value (false, 0, "", {}, nil…);
//! - an event is a connectable signal nothing fires;
//! - a callback can be assigned (and is never called).
//!
//! Each use is recorded in the gap telemetry (`stub`, `stub-call`,
//! `stub-write`), which is how the coverage table tells "exists" from "has
//! real behaviour" and how docs/ENGINE-GAPS.md ranks what to implement next.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::*;
use crate::api::Kind;
use crate::api_stubs::{Stub, STUBS};

/// Callbacks with real behaviour (BindableFunction / RemoteFunction invokes).
pub const REAL_CALLBACKS: &[&str] = &["OnInvoke", "OnServerInvoke", "OnClientInvoke"];

fn table() -> &'static HashMap<String, &'static Stub> {
    static T: OnceLock<HashMap<String, &'static Stub>> = OnceLock::new();
    T.get_or_init(|| {
        STUBS
            .iter()
            .map(|s| (format!("{}.{}", s.class, s.member), s))
            .collect()
    })
}

/// The stub for `member` on an instance of `class` (declared there or on a
/// superclass).
pub fn lookup(class: &str, member: &str) -> Option<&'static Stub> {
    let api = crate::api::get();
    let mut cur = Some(class.to_string());
    while let Some(c) = cur {
        if let Some(s) = table().get(&format!("{c}.{member}")) {
            return Some(s);
        }
        cur = api.and_then(|a| a.classes.get(&c)).and_then(|i| i.superclass.clone());
    }
    None
}

/// Whether the stub of this property has a non-nil typed default (false, 0,
/// "", an enum item, a Vector3…), i.e. something better than nil to give.
pub fn typed_default(class: &str, member: &str) -> bool {
    lookup(class, member).is_some_and(|s| !matches!(s.default, "" | "nil"))
}

/// The typed default of a property rustudio does store (known to the
/// reflection database, which has no default for it): not a stub use.
pub fn default_value(lua: &Lua, r: Ref, class: &str, member: &str) -> mlua::Result<Option<Value>> {
    match lookup(class, member).filter(|s| s.kind == Kind::Property) {
        Some(s) => Ok(Some(value(lua, r, s)?)),
        None => Ok(None),
    }
}

fn store(lua: &Lua) -> mlua::Result<Table> {
    if let Ok(t) = lua.named_registry_value::<Table>("rs_stub_store") {
        return Ok(t);
    }
    let t = lua.create_table()?;
    lua.set_named_registry_value("rs_stub_store", t.clone())?;
    Ok(t)
}

fn slot(r: Ref) -> String {
    format!("{r:?}")
}

/// The typed empty value of a stub (its generated Luau expression).
fn value(lua: &Lua, r: Ref, s: &Stub) -> mlua::Result<Value> {
    match s.default {
        "" | "nil" => Ok(Value::Nil),
        "@signal" => signal(lua, r, s.member),
        "{}" => Ok(Value::Table(lua.create_table()?)),
        expr => {
            let cache: Table = match lua.named_registry_value::<Table>("rs_stub_defaults") {
                Ok(t) => t,
                Err(_) => {
                    let t = lua.create_table()?;
                    lua.set_named_registry_value("rs_stub_defaults", t.clone())?;
                    t
                }
            };
            let hit: Value = cache.get(expr)?;
            if !hit.is_nil() {
                return Ok(match hit {
                    Value::Boolean(false) if expr != "false" => Value::Nil,
                    v => v,
                });
            }
            // A constructor rustudio lacks gives nil (remembered as such).
            let v = lua
                .load(format!("return {expr}"))
                .set_name("stub-default")
                .eval::<Value>()
                .unwrap_or(Value::Boolean(false));
            cache.set(expr, v.clone())?;
            Ok(match v {
                Value::Boolean(false) if expr != "false" => Value::Nil,
                v => v,
            })
        }
    }
}

/// The first call of each stub function says so in Output (once per member
/// per thread), so a test that leans on one can't pass silently.
fn warn_stub_once(lua: &Lua, s: &'static Stub) {
    thread_local! {
        static SEEN: std::cell::RefCell<std::collections::HashSet<usize>> = Default::default();
    }
    let key = s as *const Stub as usize;
    if SEEN.with(|v| v.borrow_mut().insert(key)) {
        if let Ok(mut w) = world(lua) {
            w.output.push((
                Level::Warn,
                format!("rustudio: {}.{} is a stub (it does nothing here)", s.class, s.member),
            ));
        }
    }
}

/// A stub function's callable (one per declaring class and name).
fn function(lua: &Lua, s: &'static Stub) -> mlua::Result<Value> {
    let key = format!("{}.{}", s.class, s.member);
    let fns: Table = match lua.named_registry_value::<Table>("rs_stub_fns") {
        Ok(t) => t,
        Err(_) => {
            let t = lua.create_table()?;
            lua.set_named_registry_value("rs_stub_fns", t.clone())?;
            t
        }
    };
    if let Ok(f) = fns.get::<Function>(key.as_str()) {
        return Ok(Value::Function(f));
    }
    let raw = lua.create_function(move |lua, args: MultiValue| {
        let this = opt_inst(&arg(&args, 0)).unwrap_or_else(Ref::none);
        crate::telemetry::gap(lua, "stub-call", s.class, s.member);
        warn_stub_once(lua, s);
        let r = value(lua, this, s).map(|v| {
            if s.default.is_empty() {
                MultiValue::new()
            } else {
                MultiValue::from_iter([v])
            }
        });
        checked(lua, r)
    })?;
    let f = crate::cmeta::method(lua, s.member, Some(raw), None, None)?;
    fns.set(key.as_str(), f.clone())?;
    Ok(Value::Function(f))
}

/// Indexing a member the runtime doesn't know: the stub's value, or None.
pub fn index(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    let Some(s) = lookup(class, key) else {
        return Ok(None);
    };
    // Callbacks the runtime does call (runtime.rs set_callback and the
    // invoke paths): unset, they read nil, which is no stub.
    if s.kind == Kind::Callback && REAL_CALLBACKS.contains(&s.member) {
        return Ok(Some(Value::Nil));
    }
    crate::telemetry::gap(lua, "stub", s.class, s.member);
    Ok(Some(match s.kind {
        Kind::Property | Kind::Callback => {
            let kept: Option<Table> = store(lua)?.get(slot(r))?;
            match kept.map(|t| t.get::<Value>(key)).transpose()? {
                Some(v) if !v.is_nil() => v,
                _ if s.kind == Kind::Callback => Value::Nil,
                _ => value(lua, r, s)?,
            }
        }
        Kind::Function => function(lua, s)?,
        Kind::Event => {
            crate::telemetry::event_used(lua, class, key);
            signal(lua, r, key)?
        }
    }))
}

/// Assigning a member the runtime doesn't know: kept when it's a writable
/// stub property or a callback (true), refused when read-only, else false.
pub fn assign(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    let Some(s) = lookup(class, key).filter(|s| matches!(s.kind, Kind::Property | Kind::Callback))
    else {
        return Ok(false);
    };
    if s.read_only {
        return Err(rt_err(format!(
            "Unable to assign property {key}. Property is read only"
        )));
    }
    crate::telemetry::gap(lua, "stub-write", s.class, s.member);
    let st = store(lua)?;
    let t: Table = match st.get::<Option<Table>>(slot(r))? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            st.set(slot(r), t.clone())?;
            t
        }
    };
    let old: Value = t.get(key)?;
    t.set(key, value.clone())?;
    if s.kind == Kind::Property && !old.equals(value).unwrap_or(false) {
        let mut w = world(lua)?;
        w.fire(r, "Changed", vec![Value::String(lua.create_string(key)?)]);
        w.fire(r, &format!("Changed:{key}"), vec![]);
    }
    Ok(true)
}

/// A declared method whose call has no implementation: the stub's empty
/// value when there is one, else the "not implemented" error as before.
pub fn unimplemented_call(lua: &Lua, this: Ref, class: &str, name: &str) -> mlua::Result<MultiValue> {
    if let Some(s) = lookup(class, name).filter(|s| s.kind == Kind::Function) {
        crate::telemetry::gap(lua, "stub-call", s.class, s.member);
        warn_stub_once(lua, s);
        let v = value(lua, this, s)?;
        return Ok(if s.default.is_empty() {
            MultiValue::new()
        } else {
            MultiValue::from_iter([v])
        });
    }
    crate::telemetry::gap(lua, "call-unimplemented", class, name);
    Err(rt_err(format!("{name} is not implemented in rustudio yet")))
}
