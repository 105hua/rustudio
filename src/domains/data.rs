//! Data options. Backend parity remains incomplete: see checks/parity/data.md.
//! Pages (DataStore*Pages, MemoryStoreHashMapPages) go through the shared
//! Pages implementation (src/domains/pages.rs).
use crate::runtime::World;
use mlua::{Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::Ref;
use std::collections::HashMap;

#[path = "data_budget.rs"]
mod budget;
#[path = "data_config.rs"]
mod config;
#[path = "data_local.rs"]
mod local;
#[path = "data_memory.rs"]
mod memory;
#[path = "data_memory_backend.rs"]
mod memory_backend;
#[path = "data_messaging.rs"]
mod messaging;
#[path = "data_store.rs"]
mod store;
pub use store::bind_context;
pub static LUA_METHODS: std::sync::LazyLock<Vec<(&'static str, &'static str)>> =
    std::sync::LazyLock::new(|| {
        let mut methods = memory::LUA_METHODS.to_vec();
        methods.extend_from_slice(messaging::LUA_METHODS);
        methods.extend_from_slice(config::LUA_METHODS);
        methods.extend_from_slice(store::LUA_METHODS);
        methods
    });

pub const METHODS: &[(&str, &str)] = &[
    ("GetRequestBudgetForRequestType", "DataStoreService"),
    ("SetRateLimitForRequestType", "DataStoreService"),
    ("GetMetadata", "DataStoreSetOptions"),
    ("SetMetadata", "DataStoreSetOptions"),
    ("GetMetadata", "DataStoreIncrementOptions"),
    ("SetMetadata", "DataStoreIncrementOptions"),
    ("SetExperimentalFeatures", "DataStoreOptions"),
];
pub const EVENTS: &[(&str, &str)] = &[("UpdateAvailable", "ConfigSnapshot")];
#[derive(Default)]
struct State {
    metadata: HashMap<Ref, Table>,
    properties: HashMap<Ref, Table>,
}
fn err(s: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(s.into())
}
fn class(lua: &Lua, r: Ref) -> Option<String> {
    lua.app_data_ref::<World>()?
        .d()
        .class_of(r)
        .map(|s| s.to_string())
}
fn copy_table(lua: &Lua, input: Table, depth: usize) -> mlua::Result<Table> {
    if depth > 64 {
        return Err(err("Tables cannot be cyclic"));
    }
    let out = lua.create_table()?;
    for pair in input.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let v = match v {
            Value::Table(t) => Value::Table(copy_table(lua, t, depth + 1)?),
            v => v,
        };
        out.raw_set(k, v)?;
    }
    Ok(out)
}
fn dictionary(v: Option<&Value>) -> mlua::Result<Table> {
    match v {
        Some(Value::Table(t)) => {
            for pair in t.clone().pairs::<Value, Value>() {
                if !matches!(pair?.0, Value::String(_)) {
                    return Err(err("Unable to cast Array to Dictionary"));
                }
            }
            Ok(t.clone())
        }
        v => Err(err(format!(
            "Unable to cast {} to Dictionary",
            match v {
                Some(Value::Integer(_) | Value::Number(_)) => "double",
                Some(Value::Boolean(_)) => "bool",
                Some(Value::String(_)) => "string",
                _ => "nil",
            }
        ))),
    }
}
pub fn call(
    lua: &Lua,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    let c = class(lua, this)?;
    if c == "DataStoreService" {
        if let Some(r) = budget::call(lua, name, args) {
            return Some(r);
        }
    }
    let options = matches!(
        c.as_str(),
        "DataStoreSetOptions" | "DataStoreIncrementOptions"
    );
    if options && matches!(name, "GetMetadata" | "SetMetadata") {
        return Some((|| {
            if name == "SetMetadata" {
                let t = copy_table(lua, dictionary(args.get(1))?, 0)?;
                lua.app_data_mut::<State>()
                    .unwrap()
                    .metadata
                    .insert(this, t);
                Ok(MultiValue::new())
            } else {
                let t = lua
                    .app_data_ref::<State>()
                    .unwrap()
                    .metadata
                    .get(&this)
                    .cloned();
                Ok(MultiValue::from_vec(vec![Value::Table(match t {
                    Some(t) => copy_table(lua, t, 0)?,
                    None => lua.create_table()?,
                })]))
            }
        })());
    }
    if c == "DataStoreOptions" && name == "SetExperimentalFeatures" {
        return Some(dictionary(args.get(1)).map(|_| MultiValue::new()));
    }
    None
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    let props = lua
        .app_data_ref::<State>()
        .and_then(|s| s.properties.get(&r).cloned());
    if let Some(props) = props {
        let value: Value = props.raw_get(key)?;
        if !matches!(value, Value::Nil) {
            return Ok(Some(value));
        }
    }
    if class == "ConfigSnapshot" && matches!(key, "Error" | "Outdated") {
        let props = lua
            .app_data_ref::<State>()
            .and_then(|s| s.properties.get(&r).cloned());
        if let Some(props) = props {
            return Ok(Some(props.get(key)?));
        }
    }
    Ok(None)
}
pub fn newindex(_: &Lua, _: Ref, _: &str, _: &str, _: &Value) -> mlua::Result<bool> {
    Ok(false)
}
pub fn write_acts(_: &str, _: &str) -> bool {
    false
}
/// Universe of the place being played (0 for a local file), set by Play
/// before its runtimes install this domain.
pub static PLAY_UNIVERSE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// A scratch experience of your own where Play may write cloud data:
/// `RUSTUDIO_SCRATCH_UNIVERSE=<universe id>` (unset or 0: none).
pub fn scratch_universe() -> u64 {
    std::env::var("RUSTUDIO_SCRATCH_UNIVERSE")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// Whether this Play keeps DataStores, MemoryStores and Messaging in memory.
/// A published place other than the scratch experience does, unless the
/// user opts in with RUSTUDIO_LIVE_DATA=1: a Play of a live game must never
/// read-modify-write real players' saves (the local player's own save
/// included) or publish to its live servers.
pub fn local_mode_for(universe: u64) -> bool {
    if std::env::var("RUSTUDIO_LOCAL_DATASTORE").as_deref() == Ok("1") {
        return true;
    }
    let scratch = scratch_universe();
    universe != 0
        && (scratch == 0 || universe != scratch)
        && std::env::var("RUSTUDIO_LIVE_DATA").as_deref() != Ok("1")
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let universe = PLAY_UNIVERSE.load(std::sync::atomic::Ordering::Relaxed);
    install_mode(lua, local_mode_for(universe))
}
fn install_mode(lua: &Lua, local_mode: bool) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    lua.set_app_data(budget::Budget::default());
    lua.load(local::SOURCE)
        .set_name("=rustudio.data.local")
        .call::<()>(local_mode)?;
    let create = lua.create_function(|lua, class: String| {
        let r = crate::runtime::ext::create_any(lua, &class)?;
        crate::runtime::inst_value(lua, r)
    })?;
    let register =
        lua.create_function(
            |lua,
             (obj, pages, on_advance): (
                crate::runtime::LInst,
                Vec<Table>,
                Option<mlua::Function>,
            )| {
                super::pages::register_list(lua, obj.0, pages, on_advance)
            },
        )?;
    let now = lua.create_function(|lua, ()| {
        Ok(lua.app_data_ref::<World>().map(|w| w.time).unwrap_or(0.0))
    })?;
    let encode = lua
        .load("return function(v) return game:GetService('HttpService'):JSONEncode(v) end")
        .eval::<mlua::Function>()?;
    lua.load(memory::SOURCE)
        .set_name("=rustudio.data.memory")
        .call::<()>((local_mode, create.clone(), register, now, encode.clone()))?;
    let server = lua.load("return function() local r=game:GetService('RunService'); return r:IsServer() and r:IsRunning() end").eval::<mlua::Function>()?;
    let remote = lua.create_table()?;
    lua.set_named_registry_value("rustudio_messaging_remote", remote.clone())?;
    lua.load(messaging::SOURCE)
        .set_name("=rustudio.data.messaging")
        .call::<()>((local_mode, create.clone(), encode, server.clone(), remote))?;
    let properties = lua.create_function(|lua, (obj, props): (crate::runtime::LInst, Table)| {
        lua.app_data_mut::<State>()
            .unwrap()
            .properties
            .insert(obj.0, props);
        Ok(())
    })?;
    let fire = lua.create_function(
        |lua, (obj, event, args): (crate::runtime::LInst, String, Table)| {
            let args = args
                .sequence_values::<Value>()
                .collect::<mlua::Result<Vec<_>>>()?;
            lua.app_data_mut::<World>()
                .ok_or_else(|| err("the play session has ended"))?
                .fire(obj.0, &event, args);
            Ok(())
        },
    )?;
    let report = lua.create_function(|lua, error: String| {
        if let Some(mut world) = lua.app_data_mut::<World>() {
            world.output.push((crate::app::Level::Error, error));
        }
        Ok(())
    })?;
    let none = lua
        .load(
            "return Enum and Enum.ConfigSnapshotErrorState and Enum.ConfigSnapshotErrorState.None",
        )
        .eval::<Value>()?;
    lua.load(config::SOURCE)
        .set_name("=rustudio.data.config")
        .call::<()>((local_mode, create, properties, fire, report, server, none))?;
    store::install(lua, local_mode)?;
    memory_backend::install(lua, local_mode)?;
    messaging::install_backend(lua, local_mode)?;
    Ok(())
}
pub fn step(_: &Lua, _: f64) -> mlua::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn data_runtime_integration() {
        use crate::doc::Document;
        use crate::runtime::{RunMode, Runtime};
        use rbx_dom_weak::InstanceBuilder;
        let mut d = Document::new_place();
        let parent = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let script = d.dom.insert(parent, InstanceBuilder::new("Script").with_name("DataIntegration").with_property("Source", r#"
local o=Instance.new('DataStoreSetOptions');o:SetMetadata({a={b=1}});local m=o:GetMetadata();m.a.b=2;assert(o:GetMetadata().a.b==1)
local ms=game:GetService('MemoryStoreService');local h=ms:GetHashMap('integration');h:SetAsync('a',1,100);h:SetAsync('b',2,100);assert(h:GetAsync('a')==1)
local p=h:ListItemsAsync(1);assert(typeof(p)=='Instance' and p:GetCurrentPage()[1].key=='a' and not p.IsFinished);p:AdvanceToNextPageAsync();assert(p.IsFinished and p:GetCurrentPage()[1].key=='b')
local q=ms:GetQueue('integration');q:AddAsync('item',100,1);local values,id=q:ReadAsync(1,false,0);assert(values[1]=='item');q:RemoveAsync(id);assert(q:GetSizeAsync()==0)
local cs=game:GetService('ConfigService');cs:SetTestingValue('a',1);local s=cs:GetConfigAsync();assert(s:GetValue('a')==1 and s.Error==Enum.ConfigSnapshotErrorState.None and not s.Outdated)
local updates,changes=0,0;s.UpdateAvailable:Connect(function()updates+=1 end);s:GetValueChangedSignal('a'):Connect(function(v)assert(v==2);changes+=1 end)
cs:SetTestingValue('a',2);task.wait();assert(updates==1 and changes==0 and s.Outdated);s:Refresh();task.wait();assert(changes==1 and s:GetValue('a')==2 and not s.Outdated)
local messaging=game:GetService('MessagingService');local received=0;local connection=messaging:SubscribeAsync('integration',function(m)assert(m.Data.value==3 and type(m.Sent)=='number');received+=1 end)
messaging:PublishAsync('integration',{value=3});task.wait();assert(received==1);connection:Disconnect();messaging:PublishAsync('integration',{value=3});task.wait();assert(received==1)
shared.__data_integration_done=true
"#.to_string()));
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        install_mode(&rt.lua, true).unwrap();
        rt.run_script(script);
        for _ in 0..300 {
            rt.step(0.1);
        }
        let done = rt
            .lua
            .globals()
            .get::<Table>("shared")
            .unwrap()
            .get::<bool>("__data_integration_done")
            .unwrap_or(false);
        let out = rt.world_mut().unwrap().output.clone();
        assert!(done, "integration did not finish: {out:?}");
    }
    #[test]
    fn metadata_is_deep_copied() {
        let lua = Lua::new();
        let t: Table = lua.load("return {x={a=1}}").eval().unwrap();
        let c = copy_table(&lua, t.clone(), 0).unwrap();
        t.get::<Table>("x").unwrap().set("a", 2).unwrap();
        assert_eq!(c.get::<Table>("x").unwrap().get::<i32>("a").unwrap(), 1);
        assert!(dictionary(Some(&Value::Integer(1))).is_err());
        let array = lua.load("return {1,2}").eval::<Table>().unwrap();
        assert!(dictionary(Some(&Value::Table(array))).is_err());
    }
    #[test]
    fn local_writes_and_unpublished_errors() {
        let lua = Lua::new();
        lua.load("__rs_lua_methods={}; game={PlaceId=0}; function warn(...) end")
            .exec()
            .unwrap();
        lua.load(local::SOURCE).call::<()>(false).unwrap();
        let e = lua
            .load("return __rs_lua_methods.GetDataStore(game,'x')")
            .eval::<Value>()
            .unwrap_err();
        assert!(e
            .to_string()
            .contains("You must publish this place to the web to access DataStore."));
        lua.load(local::SOURCE).call::<()>(true).unwrap();
        lua.load("local d=__rs_lua_methods.GetDataStore(game,'x'); d:SetAsync('k',{n=2}); local v=d:GetAsync('k');v.n=3;assert(d:GetAsync('k').n==2); assert(d:RemoveAsync('k').n==2);assert(d:GetAsync('k')==nil)").exec().unwrap();
        lua.load("game.PlaceId=123").exec().unwrap();
        lua.load(local::SOURCE).call::<()>(false).unwrap();
        let e = lua
            .load("return __rs_lua_methods.GetDataStore(game,'x')")
            .eval::<Value>()
            .unwrap_err();
        assert!(e.to_string().contains("backend is not implemented"));
    }
}
