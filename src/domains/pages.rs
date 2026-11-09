//! Pages: the one shared implementation of Pages:GetCurrentPage,
//! Pages:AdvanceToNextPageAsync and Pages.IsFinished for every Pages
//! subclass (DataStore*Pages, MemoryStoreHashMapPages, FriendPages,
//! InventoryPages / OutfitPages / CatalogPages, StandardPages,
//! RecommendationPages, AudioPages, …). Producers in other domains only
//! supply a source; nothing else wraps these methods.
//!
//! Source interface (one for everyone):
//! - Luau `__rs_pages_new(className, next)`: `next()` returns
//!   `(page, finished)` — the next page's 1-based array of records and
//!   whether it is the last page. It owns its cursor, may yield (web fetches),
//!   and is called once for the first page before the Pages is returned
//!   (call from a yieldable thread) and once per AdvanceToNextPageAsync.
//! - Luau `__rs_pages_list(className, pages, onAdvance?)` / Rust
//!   [`register_list`]: already-fetched pages; `onAdvance(i)` runs before page
//!   `i` (1-based) becomes current and may error (budget charges).
//!
//! Studio behaviour (Pages base class, Studio):
//! - GetCurrentPage returns a fresh deep copy of the current page each call.
//! - IsFinished is true while the current page is the last one.
//! - AdvanceToNextPageAsync on a finished Pages errors "No pages to advance
//!   to" without yielding; otherwise it always yields (a list source yields
//!   one resumption, a fetching source yields on its request). A failed fetch
//!   leaves the current page and IsFinished unchanged.
use mlua::{Function, Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::Ref;
use std::cell::Cell;
use std::collections::HashMap;

pub const METHODS: &[(&str, &str)] = &[];
pub const LUA_METHODS: &[(&str, &str)] = &[
    ("GetCurrentPage", "Pages"),
    ("AdvanceToNextPageAsync", "Pages"),
];
pub const EVENTS: &[(&str, &str)] = &[];

struct Entry {
    page: Table,
    finished: bool,
    next: Function,
    /// A list source doesn't yield by itself, so the advance yields for it.
    yields: bool,
}

#[derive(Default)]
struct State {
    pages: HashMap<Ref, Entry>,
}

fn err(s: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(s.into())
}

/// A deep copy (records are plain tables, cycles refused).
pub fn copy_table(lua: &Lua, input: &Table, depth: usize) -> mlua::Result<Table> {
    if depth > 64 {
        return Err(err("Tables cannot be cyclic"));
    }
    let out = lua.create_table()?;
    for pair in input.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let v = match v {
            Value::Table(t) => Value::Table(copy_table(lua, &t, depth + 1)?),
            v => v,
        };
        out.raw_set(k, v)?;
    }
    Ok(out)
}

fn page_of(lua: &Lua, v: Value) -> mlua::Result<Table> {
    match v {
        Value::Table(t) => copy_table(lua, &t, 0),
        Value::Nil => lua.create_table(),
        _ => Err(err("Pages source returned a non-table page")),
    }
}

fn insert(lua: &Lua, r: Ref, page: Value, finished: bool, next: Function, yields: bool) -> mlua::Result<()> {
    let page = page_of(lua, page)?;
    lua.app_data_mut::<State>()
        .ok_or_else(|| err("pages not installed"))?
        .pages
        .insert(r, Entry { page, finished, next, yields });
    Ok(())
}

/// Already-fetched pages for the Pages instance `r` (any subclass). An empty
/// list is one empty page. `on_advance(i)` runs before page `i` (1-based)
/// becomes current.
pub fn register_list(lua: &Lua, r: Ref, mut pages: Vec<Table>, on_advance: Option<Function>) -> mlua::Result<()> {
    if pages.is_empty() {
        pages.push(lua.create_table()?);
    }
    let pages = pages
        .iter()
        .map(|t| copy_table(lua, t, 0))
        .collect::<mlua::Result<Vec<_>>>()?;
    let first = Value::Table(pages[0].clone());
    let finished = pages.len() <= 1;
    let index = Cell::new(0usize);
    let next = lua.create_function(move |_, ()| {
        let i = index.get() + 1;
        let page = pages.get(i).cloned().ok_or_else(|| err("No pages to advance to"))?;
        if let Some(f) = &on_advance {
            f.call::<()>(i + 1)?;
        }
        index.set(i);
        Ok((page, i + 1 >= pages.len()))
    })?;
    insert(lua, r, first, finished, next, true)
}

/// Pages.IsFinished for a registered Pages (None for one nobody registered).
pub fn is_finished(lua: &Lua, r: Ref) -> Option<bool> {
    lua.app_data_ref::<State>()?.pages.get(&r).map(|e| e.finished)
}

pub fn call(_: &Lua, _: &str, _: Ref, _: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    None
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if key == "IsFinished" && crate::doc::is_a(class, "Pages") {
        return Ok(is_finished(lua, r).map(Value::Boolean));
    }
    Ok(None)
}

pub fn newindex(_: &Lua, _: Ref, _: &str, _: &str, _: &Value) -> mlua::Result<bool> {
    Ok(false)
}

pub fn write_acts(_: &str, _: &str) -> bool {
    false
}

const SOURCE: &str = r#"
local register, current, begin, commit = ...
local M = __rs_lua_methods
function __rs_pages_new(class, next)
    local page, finished = next()
    return register(class, next, page, finished == true, false)
end
function __rs_pages_list(class, list, onAdvance)
    return register(class, list, onAdvance)
end
M.GetCurrentPage = function(self)
    return current(self)
end
M.AdvanceToNextPageAsync = function(self)
    local next, yields = begin(self)
    if next == nil then error('No pages to advance to', 0) end
    if yields then task.wait() end
    local page, finished = next()
    commit(self, page, finished == true)
end
"#;

pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.set_app_data(State::default());
    let register = lua.create_function(|lua, args: MultiValue| {
        let mut it = args.into_iter();
        let class = match it.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(err("Pages class expected")),
        };
        if !crate::doc::is_a(&class, "Pages") {
            return Err(err(format!("{class} is not a Pages class")));
        }
        let r = crate::runtime::ext::create_any(lua, &class)?;
        match it.next() {
            // (class, next, page, finished, yields)
            Some(Value::Function(next)) => {
                let page = it.next().unwrap_or(Value::Nil);
                let finished = matches!(it.next(), Some(Value::Boolean(true)));
                insert(lua, r, page, finished, next, false)?;
            }
            // (class, list, onAdvance)
            Some(Value::Table(list)) => {
                let pages = list.sequence_values::<Table>().collect::<mlua::Result<Vec<_>>>()?;
                let on_advance = match it.next() {
                    Some(Value::Function(f)) => Some(f),
                    _ => None,
                };
                register_list(lua, r, pages, on_advance)?;
            }
            _ => return Err(err("Pages source expected")),
        }
        crate::runtime::inst_value(lua, r)
    })?;
    let current = lua.create_function(|lua, this: crate::runtime::LInst| {
        let page = lua
            .app_data_ref::<State>()
            .and_then(|s| s.pages.get(&this.0).map(|e| e.page.clone()))
            .ok_or_else(|| err("Pages has no result"))?;
        copy_table(lua, &page, 0)
    })?;
    let begin = lua.create_function(|lua, this: crate::runtime::LInst| {
        let s = lua.app_data_ref::<State>().ok_or_else(|| err("pages not installed"))?;
        let e = s.pages.get(&this.0).ok_or_else(|| err("Pages has no result"))?;
        if e.finished {
            return Ok((None, false));
        }
        Ok((Some(e.next.clone()), e.yields))
    })?;
    let commit = lua.create_function(|lua, (this, page, finished): (crate::runtime::LInst, Value, bool)| {
        let page = page_of(lua, page)?;
        let mut s = lua.app_data_mut::<State>().ok_or_else(|| err("pages not installed"))?;
        let e = s.pages.get_mut(&this.0).ok_or_else(|| err("Pages has no result"))?;
        e.page = page;
        e.finished = finished;
        Ok(())
    })?;
    lua.load(SOURCE)
        .set_name("=rustudio.pages")
        .call::<()>((register, current, begin, commit))
}

/// A pure-Luau stand-in with the same interface and semantics, for unit
/// tests of producers that run without a World (no instances).
#[cfg(test)]
pub const MOCK: &str = r#"
local M=__rs_lua_methods
local function copy(v)if type(v)~='table'then return v end;local t={};for k,x in pairs(v)do t[k]=copy(x)end;return t end
function __rs_pages_new(class,next)local page,finished=next();return {ClassName=class,page=copy(page or {}),IsFinished=finished==true,next=next}end
function __rs_pages_list(class,list,onAdvance)
 local i=1;local p=__rs_pages_new(class,function()return list[1] or {},#list<=1 end)
 p.next=function()i+=1;if onAdvance then onAdvance(i)end;return list[i],i>=#list end;return p
end
M.GetCurrentPage=function(self)return copy(self.page)end
M.AdvanceToNextPageAsync=function(self)if self.IsFinished then error('No pages to advance to',0)end;local page,finished=self.next();self.page=copy(page or {});self.IsFinished=finished==true end
"#;

pub fn step(_: &Lua, _: f64) -> mlua::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;
    use crate::runtime::{RunMode, Runtime, World};
    use rbx_dom_weak::InstanceBuilder;

    fn run(src: &str) -> (bool, Vec<(crate::app::Level, String)>) {
        let mut d = Document::new_place();
        let parent = d.find_child(d.dom.root_ref(), "ServerScriptService").unwrap();
        let script = d.dom.insert(
            parent,
            InstanceBuilder::new("Script")
                .with_name("PagesTest")
                .with_property("Source", format!("{src}\nshared.__pages_done=true")),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(script);
        for _ in 0..50 {
            rt.step(0.05);
        }
        let done = rt
            .lua
            .globals()
            .get::<Table>("shared")
            .unwrap()
            .get::<bool>("__pages_done")
            .unwrap_or(false);
        let out = rt.world_mut().unwrap().output.clone();
        (done, out)
    }

    #[test]
    fn list_source_matches_studio() {
        let (done, out) = run(r#"
local charged={}
local p=__rs_pages_list('StandardPages',{{{a=1}},{{a=2}}},function(i)table.insert(charged,i)end)
assert(p.ClassName=='StandardPages' and p:IsA('Pages'))
assert(not p.IsFinished and p:GetCurrentPage()[1].a==1)
local c=p:GetCurrentPage();c[1].a=9;assert(p:GetCurrentPage()[1].a==1,'copy')
local yielded=false;task.defer(function()yielded=true end)
p:AdvanceToNextPageAsync();assert(yielded,'advance yields')
assert(p.IsFinished and p:GetCurrentPage()[1].a==2 and charged[1]==2)
local ok,e=pcall(function()p:AdvanceToNextPageAsync()end);assert(not ok and e=='No pages to advance to',e)
local q=__rs_pages_list('StandardPages',{});assert(q.IsFinished and #q:GetCurrentPage()==0)
"#);
        assert!(done, "{out:?}");
    }

    #[test]
    fn fetch_source_matches_studio() {
        let (done, out) = run(r#"
local n,fail=0,false
local p=__rs_pages_new('FriendPages',function()
    task.wait();if fail then error('Request Failed',0)end
    n+=1;return {{Id=n}},n==2
end)
assert(n==1 and not p.IsFinished and p:GetCurrentPage()[1].Id==1)
fail=true;local ok,e=pcall(function()p:AdvanceToNextPageAsync()end)
assert(not ok and e=='Request Failed' and not p.IsFinished and p:GetCurrentPage()[1].Id==1)
fail=false;p:AdvanceToNextPageAsync();assert(n==2 and p.IsFinished and p:GetCurrentPage()[1].Id==2)
local ok2,e2=pcall(function()p:AdvanceToNextPageAsync()end);assert(not ok2 and e2=='No pages to advance to' and n==2)
"#);
        assert!(done, "{out:?}");
    }
}
