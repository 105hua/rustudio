//! SharedTable as in Studio 0.744 (docs/RE-DATATYPES.md SharedTable rows):
//! userdata (`typeof` "SharedTable", tostring `SharedTable: 0x…`, locked
//! metatable), indexable and assignable like a table, generalized iteration
//! (`for k, v in st`), no `__namecall` (`st:f()` is the VM's "attempt to
//! call missing method 'f' of SharedTable"), no operators (the VM's
//! messages), compared by identity. The `SharedTable` global is a frozen
//! table of named C functions (new, clone, cloneAndFreeze, isFrozen, size,
//! clear, increment, update).
//!
//! rustudio runs every script (and every Actor) of a side in one Luau
//! state, so a SharedTable is shared across Actors by reference; the
//! SharedTableRegistry (library.luau) hands out the same object by name.
//!
//! Estimated (no Studio row): keys must be strings or numbers; functions,
//! threads and coroutines are refused as values; a plain table stored in a
//! SharedTable is copied into a new SharedTable (as `new` does); writes to
//! a frozen SharedTable fail with "attempt to modify a readonly table";
//! `increment` treats a missing value as 0 and returns the old value.

use crate::cmeta::LOCATED;
use mlua::{AnyUserData, Lua, MetaMethod, Table, UserData, UserDataMethods, Value};

/// The elements live in a plain Lua table only Rust can reach; frozen =
/// that table is read-only.
pub struct LSharedTable {
    t: Table,
}

fn err(msg: String) -> mlua::Error {
    mlua::Error::runtime(msg)
}

fn type_name(v: &Value) -> String {
    match v {
        Value::UserData(u) => u
            .metatable()
            .ok()
            .and_then(|m| m.get::<String>("__type").ok())
            .unwrap_or_else(|| "userdata".into()),
        Value::Integer(_) => "number".into(),
        Value::Vector(_) => "Vector3".into(),
        other => other.type_name().into(),
    }
}

/// Argument `n` as a SharedTable's element table.
fn arg(v: &Value, n: usize) -> mlua::Result<Table> {
    if let Value::UserData(u) = v {
        if let Ok(s) = u.borrow::<LSharedTable>() {
            return Ok(s.t.clone());
        }
    }
    let what = if v.is_nil() { "missing" } else { "invalid" };
    let got = if v.is_nil() { String::new() } else { format!(", got {}", type_name(v)) };
    Err(err(format!("{LOCATED}{what} argument #{n} (SharedTable expected{got})")))
}

fn check_key(k: &Value) -> mlua::Result<()> {
    match k {
        Value::String(_) | Value::Integer(_) | Value::Number(_) => Ok(()),
        other => Err(err(format!("{LOCATED}invalid argument #2 (string or number expected, got {})", type_name(other)))),
    }
}

/// A value as stored: tables become SharedTables, functions and threads
/// are refused.
fn store_value(lua: &Lua, v: Value) -> mlua::Result<Value> {
    match v {
        Value::Table(t) => Ok(Value::UserData(from_table(lua, &t)?)),
        Value::Function(_) | Value::Thread(_) => {
            Err(err(format!("{LOCATED}SharedTable cannot hold a {} value", v.type_name())))
        }
        other => Ok(other),
    }
}

fn set(lua: &Lua, t: &Table, k: Value, v: Value) -> mlua::Result<()> {
    check_key(&k)?;
    if t.is_readonly() {
        return Err(err(format!("{LOCATED}attempt to modify a readonly table")));
    }
    let v = if v.is_nil() { v } else { store_value(lua, v)? };
    t.raw_set(k, v)
}

fn make(lua: &Lua, t: Table) -> mlua::Result<AnyUserData> {
    lua.create_userdata(LSharedTable { t })
}

fn from_table(lua: &Lua, src: &Table) -> mlua::Result<AnyUserData> {
    let t = lua.create_table()?;
    for pair in src.pairs::<Value, Value>() {
        let (k, v) = pair?;
        set(lua, &t, k, v)?;
    }
    make(lua, t)
}

/// A copy; `deep` copies nested SharedTables too, `freeze` freezes the
/// copy (and, deep, its nested copies).
fn copy(lua: &Lua, src: &Table, deep: bool, freeze: bool) -> mlua::Result<AnyUserData> {
    let t = lua.create_table()?;
    for pair in src.pairs::<Value, Value>() {
        let (k, mut v) = pair?;
        if deep {
            if let Value::UserData(u) = &v {
                if let Ok(inner) = u.borrow::<LSharedTable>().map(|s| s.t.clone()) {
                    v = Value::UserData(copy(lua, &inner, true, freeze)?);
                }
            }
        }
        t.raw_set(k, v)?;
    }
    if freeze {
        t.set_readonly(true);
    }
    make(lua, t)
}

impl UserData for LSharedTable {
    fn add_fields<F: mlua::UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "SharedTable");
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // A snapshot, as Studio iterates a consistent view.
        m.add_meta_method(MetaMethod::Iter, |lua, s, ()| {
            let mut items = Vec::new();
            for pair in s.t.pairs::<Value, Value>() {
                items.push(pair?);
            }
            let mut i = 0usize;
            let next = lua.create_function_mut(move |_, _: mlua::MultiValue| {
                let r = items.get(i).cloned();
                i += 1;
                Ok(match r {
                    Some((k, v)) => (k, v),
                    None => (Value::Nil, Value::Nil),
                })
            })?;
            Ok((next, Value::Nil, Value::Nil))
        });
    }
}

/// The `SharedTable` global (dtmeta's constructor pass names, wraps and
/// freezes it).
pub fn install(lua: &Lua, g: &Table) -> mlua::Result<()> {
    let st = lua.create_table()?;
    st.set(
        "new",
        lua.create_function(|lua, a: mlua::Variadic<Value>| match a.first() {
            None | Some(Value::Nil) => make(lua, lua.create_table()?),
            Some(Value::Table(t)) => from_table(lua, t),
            Some(other) => Err(err(format!("{LOCATED}invalid argument #1 (table expected, got {})", type_name(other)))),
        })?,
    )?;
    st.set(
        "clone",
        lua.create_function(|lua, (s, deep): (Value, Option<bool>)| copy(lua, &arg(&s, 1)?, deep.unwrap_or(false), false))?,
    )?;
    st.set(
        "cloneAndFreeze",
        lua.create_function(|lua, (s, deep): (Value, Option<bool>)| copy(lua, &arg(&s, 1)?, deep.unwrap_or(false), true))?,
    )?;
    st.set("isFrozen", lua.create_function(|_, s: Value| Ok(arg(&s, 1)?.is_readonly()))?)?;
    st.set(
        "size",
        lua.create_function(|_, s: Value| {
            let t = arg(&s, 1)?;
            let mut n = 0i64;
            for pair in t.pairs::<Value, Value>() {
                pair?;
                n += 1;
            }
            Ok(n)
        })?,
    )?;
    st.set(
        "clear",
        lua.create_function(|_, s: Value| {
            let t = arg(&s, 1)?;
            if t.is_readonly() {
                return Err(err(format!("{LOCATED}attempt to modify a readonly table")));
            }
            t.clear()
        })?,
    )?;
    st.set(
        "increment",
        lua.create_function(|lua, (s, k, d): (Value, Value, Value)| {
            let t = arg(&s, 1)?;
            check_key(&k)?;
            let delta = match d {
                Value::Integer(i) => i as f64,
                Value::Number(n) => n,
                other => {
                    return Err(err(format!("{LOCATED}invalid argument #3 (number expected, got {})", type_name(&other))))
                }
            };
            let old = match t.raw_get::<Value>(k.clone())? {
                Value::Nil => 0.0,
                Value::Integer(i) => i as f64,
                Value::Number(n) => n,
                other => return Err(err(format!("{LOCATED}SharedTable.increment: value is a {}, not a number", type_name(&other)))),
            };
            set(lua, &t, k, Value::Number(old + delta))?;
            Ok(old)
        })?,
    )?;
    st.set(
        "update",
        lua.create_function(|lua, (s, k, f): (Value, Value, mlua::Function)| {
            let t = arg(&s, 1)?;
            check_key(&k)?;
            let old: Value = t.raw_get(k.clone())?;
            let new: Value = f.call(old)?;
            set(lua, &t, k, new)
        })?,
    )?;
    g.set("SharedTable", st)?;
    Ok(())
}

/// The type's metatable as Studio's: no `__namecall`, no `__tostring`
/// (Luau's default prints `SharedTable: 0x…`), locked.
pub fn seal(lua: &Lua) -> mlua::Result<()> {
    // `(true, value)` / `(false, message)` for cmeta's unnamed C wrapper
    // (a leading LOCATED takes the caller's position).
    fn answer(lua: &Lua, r: mlua::Result<Value>) -> mlua::Result<mlua::MultiValue> {
        Ok(match r {
            Ok(v) => mlua::MultiValue::from_vec(vec![Value::Boolean(true), v]),
            Err(e) => {
                let msg = crate::dtmeta::studio_text(&e);
                mlua::MultiValue::from_vec(vec![Value::Boolean(false), Value::String(lua.create_string(msg)?)])
            }
        })
    }
    let index = lua.create_function(|lua, (s, k): (Value, Value)| {
        let r = arg(&s, 1).and_then(|t| {
            check_key(&k)?;
            t.raw_get::<Value>(k)
        });
        answer(lua, r)
    })?;
    let newindex = lua.create_function(|lua, (s, k, v): (Value, Value, Value)| {
        let r = arg(&s, 1).and_then(|t| set(lua, &t, k, v)).map(|_| Value::Nil);
        answer(lua, r)
    })?;
    let index = crate::cmeta::wrap(lua, index, true)?;
    let newindex = crate::cmeta::wrap(lua, newindex, true)?;
    let ud = make(lua, lua.create_table()?)?;
    unsafe {
        lua.exec_raw::<()>((ud, index, newindex), |l| {
            use mlua::ffi;
            if ffi::lua_getmetatable(l, 1) != 0 {
                ffi::lua_pushstring(l, c"__index".as_ptr());
                ffi::lua_pushvalue(l, 2);
                ffi::lua_rawset(l, -3);
                ffi::lua_pushstring(l, c"__newindex".as_ptr());
                ffi::lua_pushvalue(l, 3);
                ffi::lua_rawset(l, -3);
                for k in [c"__namecall", c"__tostring", c"__eq", c"__len"] {
                    ffi::lua_pushstring(l, k.as_ptr());
                    ffi::lua_pushnil(l);
                    ffi::lua_rawset(l, -3);
                }
                ffi::lua_pushstring(l, c"__metatable".as_ptr());
                ffi::lua_pushstring(l, c"The metatable is locked".as_ptr());
                ffi::lua_rawset(l, -3);
            }
            ffi::lua_settop(l, 0);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> Lua {
        let lua = Lua::new();
        crate::ltypes::install(&lua, &lua.globals()).unwrap();
        crate::dtmeta::install_all(&lua).unwrap();
        lua
    }

    fn run(lua: &Lua, src: &str) -> String {
        lua.load(src).set_name("=T").eval::<String>().unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    /// RE-DATATYPES SharedTable create / values / read / write / namecall /
    /// operator / constructor rows.
    #[test]
    fn shape_like_studio() {
        let lua = lua();
        let got = run(&lua, r#"
            local o, other = SharedTable.new(), SharedTable.new()
            local function e(f) local ok, m = pcall(f) return tostring(ok) .. "|" .. tostring(m) end
            o.x = 1
            return table.concat({
                type(o), typeof(o), (tostring(o):gsub("%x+$", "P")), tostring(getmetatable(o)),
                tostring(o == other), o.x, tostring(o.missing),
                e(function() return o:foo() end), e(function() return o + 1 end), e(function() return #o end),
                e(function() return o < o end),
                tostring(table.isfrozen(SharedTable)), e(function() SharedTable.foo = 1 end),
                e(function() return SharedTable.new('a') end),
                debug.info(SharedTable.cloneAndFreeze, "s"), debug.info(SharedTable.cloneAndFreeze, "n"),
            }, " # ")
        "#);
        assert_eq!(
            got,
            "userdata # SharedTable # SharedTable: 0xP # The metatable is locked # false # 1 # nil # \
             false|T:8: attempt to call missing method 'foo' of SharedTable # \
             false|T:8: attempt to perform arithmetic (add) on SharedTable and number # \
             false|T:8: attempt to get length of a SharedTable value # \
             false|T:9: attempt to compare SharedTable < SharedTable # true # \
             false|T:10: attempt to modify a readonly table # \
             false|T:11: invalid argument #1 to 'new' (table expected, got string) # [C] # cloneAndFreeze"
        );
    }

    /// The library functions and iteration.
    #[test]
    fn semantics() {
        let lua = lua();
        let got = run(&lua, r#"
            local s = SharedTable.new({ a = 1, b = { c = 2 } })
            local out = {}
            out[#out + 1] = typeof(s.b) .. s.b.c
            out[#out + 1] = SharedTable.size(s)
            out[#out + 1] = SharedTable.increment(s, "a", 5) .. "," .. s.a
            SharedTable.update(s, "n", function(v) return (v or 10) + 1 end)
            out[#out + 1] = s.n
            local keys = {}
            for k, v in s do keys[#keys + 1] = k end
            table.sort(keys)
            out[#out + 1] = table.concat(keys, "")
            local f = SharedTable.cloneAndFreeze(s, true)
            out[#out + 1] = tostring(SharedTable.isFrozen(f)) .. tostring(SharedTable.isFrozen(f.b)) .. tostring(SharedTable.isFrozen(s))
            out[#out + 1] = select(2, pcall(function() f.a = 2 end))
            local c = SharedTable.clone(s)
            out[#out + 1] = tostring(rawequal(c.b, s.b)) .. tostring(rawequal(SharedTable.clone(s, true).b, s.b))
            SharedTable.clear(s)
            out[#out + 1] = SharedTable.size(s) .. "," .. SharedTable.size(c)
            out[#out + 1] = select(2, pcall(SharedTable.size, {}))
            return table.concat(out, " # ")
        "#);
        assert_eq!(
            got,
            "SharedTable2 # 2 # 1,6 # 11 # abn # truetruefalse # T:15: attempt to modify a readonly table # \
             truefalse # 0,3 # invalid argument #1 to 'size' (SharedTable expected, got table)"
        );
    }
}
