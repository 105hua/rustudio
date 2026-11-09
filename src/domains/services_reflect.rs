//! ReflectionService, from Studio's API dump (app/api-dump.json), in the
//! shapes Studio 0.744 returns (measured in an offline Studio):
//! - classes: {Name, Superclass, Subclasses (direct, when any), Serialized,
//!   Display = {Category = "General"}, Permits = {New | GetService = caps}}
//! - properties: {Name, Owner, Type, Display = {Category, DeprecationMessage?},
//!   Serialized, Permits = {Read?, Write?}} (non-scriptable: Permits = {})
//! - methods: {Name, Owner, ReturnType, Parameters = {{Name, Type, DefaultValue?}},
//!   Display = {}, Permits = {Call, CallParallel?}}
//! - events: {Name, Owner, Parameters, Display = {}, Permits = {Listen}}
//! Members are listed class by class from the class itself up to Object,
//! each class's in byte order of name. Types: {EngineType, ScriptType?,
//! EnumType? | InstanceType?}.
//! Known differences: Studio's own reflection database has members the
//! dump leaves out (Part.shap, formFactorRaw…) and lists 656 classes where
//! the dump has 944; class capabilities (New = Basic…) aren't in the dump,
//! so classes report empty ones.

use std::sync::OnceLock;

use mlua::{Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, Value};
use serde_json::Value as J;

/// SecurityCapabilities: the set of capabilities a member needs.
#[derive(Clone, Debug, PartialEq)]
pub struct LCaps(pub Vec<String>);

impl UserData for LCaps {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "SecurityCapabilities");
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.0.join(", ")));
        m.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::AnyUserData| {
            Ok(other.borrow::<LCaps>().map(|o| o.0 == this.0).unwrap_or(false))
        });
        m.add_method("Contains", |_, this, other: mlua::AnyUserData| {
            let o = other.borrow::<LCaps>()?;
            Ok(o.0.iter().all(|c| this.0.contains(c)))
        });
    }
}

struct Dump {
    classes: Vec<J>,
}

fn dump() -> Option<&'static Dump> {
    static D: OnceLock<Option<Dump>> = OnceLock::new();
    D.get_or_init(|| {
        let path = std::env::var_os("RUSTUDIO_API_DUMP")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/api-dump.json").into());
        let v: J = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let mut classes = v.get("Classes")?.as_array()?.clone();
        classes.sort_by(|a, b| a["Name"].as_str().cmp(&b["Name"].as_str()));
        Some(Dump { classes })
    })
    .as_ref()
}

fn class<'a>(d: &'a Dump, name: &str) -> Option<&'a J> {
    d.classes
        .binary_search_by(|c| c["Name"].as_str().unwrap_or("").cmp(name))
        .ok()
        .map(|i| &d.classes[i])
}

fn tags(v: &J) -> Vec<&str> {
    v.get("Tags").and_then(|t| t.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default()
}

fn caps(lua: &Lua, list: Option<&J>) -> mlua::Result<Value> {
    let v = list
        .and_then(|l| l.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(Value::UserData(lua.create_userdata(LCaps(v))?))
}

/// A dump type ({Category, Name}) as Studio's {EngineType, ScriptType…}.
/// `prop`: a property's object type is a RefType, a parameter's an Instance.
fn type_table(lua: &Lua, t: &J, prop: bool) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let cat = t["Category"].as_str().unwrap_or("");
    let name = t["Name"].as_str().unwrap_or("");
    let script = |e: &str| -> Option<&'static str> {
        Some(match e {
            "bool" => "boolean",
            "float" | "double" | "int" | "int64" => "number",
            "string" => "string",
            "ContentId" => "string",
            "Buffer" => "buffer",
            "ValueTable" | "ValueArray" | "Dictionary" | "Array" | "Map" => "table",
            "Function" | "WeakFunctionRef" => "Function",
            "Void" | "Variant" | "Tuple" | "Color3uint8" => return None,
            _ => return Some(""),
        })
    };
    match cat {
        "Enum" => {
            out.set("EngineType", "Enum")?;
            out.set("EnumType", name)?;
            out.set("ScriptType", "EnumItem")?;
        }
        "Class" => {
            out.set("EngineType", if prop { "RefType" } else { "Instance" })?;
            out.set("InstanceType", name)?;
            out.set("ScriptType", "Instance")?;
        }
        "Group" if name == "Objects" => {
            out.set("EngineType", "Instances")?;
            out.set("InstanceType", "Instance")?;
            out.set("ScriptType", "Instances")?;
        }
        _ => {
            let engine = match name {
                "CFrame" => "CoordinateFrame",
                "null" => "Void",
                "buffer" => "Buffer",
                "Dictionary" => "ValueTable",
                "Array" => "ValueArray",
                "Function" => "WeakFunctionRef",
                "Content" if cat == "DataType" => "Content",
                other => other,
            };
            out.set("EngineType", engine)?;
            match script(engine) {
                Some("") => out.set("ScriptType", name)?,
                Some(s) => out.set("ScriptType", s)?,
                None => {}
            }
        }
    }
    Ok(out)
}

fn default_value(lua: &Lua, s: &str) -> mlua::Result<Value> {
    Ok(match s {
        "true" => Value::Boolean(true),
        "false" => Value::Boolean(false),
        "nil" => Value::Nil,
        _ => match s.parse::<f64>() {
            Ok(n) => Value::Number(n),
            Err(_) => Value::String(lua.create_string(s)?),
        },
    })
}

fn display(lua: &Lua, m: &J, with_category: bool) -> mlua::Result<Table> {
    let d = lua.create_table()?;
    if with_category {
        if let Some(c) = m["Category"].as_str() {
            d.set("Category", c)?;
        }
    }
    if tags(m).contains(&"Deprecated") {
        if let Some(p) = m.get("PreferredDescriptor").and_then(|p| p["Name"].as_str()) {
            d.set("DeprecationMessage", format!("Use `{p}` instead."))?;
        }
    }
    Ok(d)
}

fn params(lua: &Lua, m: &J) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (i, p) in m["Parameters"].as_array().into_iter().flatten().enumerate() {
        let e = lua.create_table()?;
        e.set("Name", p["Name"].as_str().unwrap_or(""))?;
        e.set("Type", type_table(lua, &p["Type"], false)?)?;
        if let Some(d) = p.get("Default").and_then(|d| d.as_str()) {
            e.set("DefaultValue", default_value(lua, d)?)?;
        }
        t.set(i + 1, e)?;
    }
    Ok(t)
}

fn member(lua: &Lua, owner: &str, m: &J) -> mlua::Result<Option<Table>> {
    let kind = m["MemberType"].as_str().unwrap_or("");
    let t = lua.create_table()?;
    t.set("Name", m["Name"].as_str().unwrap_or(""))?;
    t.set("Owner", owner)?;
    let permits = lua.create_table()?;
    let cap = m.get("Capabilities");
    let sec_ok = |key: &str| -> bool {
        match &m["Security"] {
            J::String(s) => s == "None",
            J::Object(o) => o.get(key).and_then(|s| s.as_str()) == Some("None"),
            _ => true,
        }
    };
    let safe = matches!(m["ThreadSafety"].as_str(), Some("Safe" | "ReadSafe"));
    match kind {
        "Property" => {
            let tg = tags(m);
            t.set("Type", type_table(lua, &m["ValueType"], true)?)?;
            t.set("Display", display(lua, m, true)?)?;
            t.set(
                "Serialized",
                m.get("Serialization").and_then(|s| s["CanSave"].as_bool()).unwrap_or(false),
            )?;
            if !tg.contains(&"NotScriptable") {
                if sec_ok("Read") {
                    permits.set("Read", caps(lua, cap.and_then(|c| c.get("Read")))?)?;
                }
                if sec_ok("Write") && !tg.contains(&"ReadOnly") {
                    permits.set("Write", caps(lua, cap.and_then(|c| c.get("Write")))?)?;
                }
            }
        }
        "Function" => {
            let ret = match &m["ReturnType"] {
                J::Array(a) => a.first().cloned().unwrap_or(J::Null),
                v => v.clone(),
            };
            t.set("ReturnType", type_table(lua, &ret, false)?)?;
            t.set("Parameters", params(lua, m)?)?;
            t.set("Display", display(lua, m, false)?)?;
            if sec_ok("Call") {
                let c = cap.and_then(|c| if c.is_array() { Some(c) } else { c.get("Call") });
                permits.set("Call", caps(lua, c)?)?;
                if safe {
                    permits.set("CallParallel", caps(lua, c)?)?;
                }
            }
        }
        "Event" => {
            t.set("Parameters", params(lua, m)?)?;
            t.set("Display", display(lua, m, false)?)?;
            if sec_ok("Listen") {
                let c = cap.and_then(|c| if c.is_array() { Some(c) } else { c.get("Listen") });
                permits.set("Listen", caps(lua, c)?)?;
            }
        }
        _ => return Ok(None),
    }
    t.set("Permits", permits)?;
    Ok(Some(t))
}

/// The filter's fields: ExcludeInherited, and Security (must be a
/// SecurityCapabilities, Studio: "ReflectionClassFilter's Security field is
/// expected to be type SecurityCapabilities").
fn filter(v: &Value, what: &str) -> mlua::Result<bool> {
    let Value::Table(t) = v else { return Ok(false) };
    match t.get::<Value>("Security")? {
        Value::Nil => {}
        Value::UserData(u) if u.is::<LCaps>() => {}
        _ => {
            return Err(mlua::Error::RuntimeError(format!(
                "{what}'s Security field is expected to be type SecurityCapabilities"
            )))
        }
    }
    Ok(matches!(t.get::<Value>("ExcludeInherited")?, Value::Boolean(true)))
}

fn class_table(lua: &Lua, d: &Dump, c: &J) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let name = c["Name"].as_str().unwrap_or("");
    t.set("Name", name)?;
    if let Some(s) = c["Superclass"].as_str().filter(|s| *s != "<<<ROOT>>>") {
        t.set("Superclass", s)?;
    }
    let subs: Vec<&str> = d
        .classes
        .iter()
        .filter(|x| x["Superclass"].as_str() == Some(name))
        .filter_map(|x| x["Name"].as_str())
        .collect();
    if !subs.is_empty() {
        t.set("Subclasses", lua.create_sequence_from(subs)?)?;
    }
    t.set("Serialized", true)?;
    let disp = lua.create_table()?;
    disp.set("Category", "General")?;
    t.set("Display", disp)?;
    let permits = lua.create_table()?;
    let tg = tags(c);
    if tg.contains(&"Service") {
        permits.set("GetService", caps(lua, None)?)?;
    } else if !tg.contains(&"NotCreatable") {
        permits.set("New", caps(lua, None)?)?;
    }
    t.set("Permits", permits)?;
    Ok(t)
}

pub fn get_class(lua: &Lua, name: &str, f: &Value) -> mlua::Result<Value> {
    filter(f, "ReflectionClassFilter")?;
    let Some(d) = dump() else { return Ok(Value::Nil) };
    match class(d, name) {
        Some(c) => Ok(Value::Table(class_table(lua, d, c)?)),
        None => Ok(Value::Nil),
    }
}

pub fn get_classes(lua: &Lua, f: &Value) -> mlua::Result<Value> {
    filter(f, "ReflectionClassFilter")?;
    let out = lua.create_table()?;
    if let Some(d) = dump() {
        for (i, c) in d.classes.iter().enumerate() {
            out.set(i + 1, class_table(lua, d, c)?)?;
        }
    }
    Ok(Value::Table(out))
}

/// GetPropertiesOfClass / GetMethodsOfClass / GetEventsOfClass.
pub fn members_of(lua: &Lua, class_name: &str, kind: &str, f: &Value) -> mlua::Result<Value> {
    let own_only = filter(f, "ReflectionMemberFilter")?;
    let Some(d) = dump() else { return Ok(Value::Nil) };
    let Some(mut c) = class(d, class_name) else { return Ok(Value::Nil) };
    let out = lua.create_table()?;
    let mut n = 0;
    loop {
        let owner = c["Name"].as_str().unwrap_or("");
        let mut ms: Vec<&J> = c["Members"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["MemberType"].as_str() == Some(kind))
            .collect();
        ms.sort_by(|a, b| a["Name"].as_str().cmp(&b["Name"].as_str()));
        for m in ms {
            if let Some(t) = member(lua, owner, m)? {
                n += 1;
                out.set(n, t)?;
            }
        }
        if own_only {
            break;
        }
        match c["Superclass"].as_str().and_then(|s| class(d, s)) {
            Some(s) => c = s,
            None => break,
        }
    }
    Ok(Value::Table(out))
}
