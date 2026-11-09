//! Engine API domains (2026-11-07 parity push). Each domain module owns a
//! slice of Studio's API (see its header) and plugs into the runtime through
//! the hooks below; the runtime asks domains before its own extension code
//! (runtime_ext.rs) and long before the generated stubs (stubs.rs).
//!
//! A domain module provides (all optional except METHODS / EVENTS):
//! - `METHODS: &[(&str, &str)]`  (method, base class it lives on)
//! - `LUA_METHODS` (optional; add it to `LUA_METHODS` below): methods
//!   written in Luau because they yield, set in `__rs_lua_methods` by install
//! - `EVENTS: &[(&str, &str)]`   (event, base class) that the domain fires
//! - `call(lua, name, this, args) -> Option<Result<MultiValue>>`
//! - `prop_get(lua, r, class, key) -> Result<Option<Value>>`
//! - `newindex(lua, r, class, key, &value) -> Result<bool>` (true = handled)
//! - `write_acts(class, key) -> bool` (a plain property write has behaviour)
//! - `install(lua)` once per Lua state, `step(lua, dt)` once per frame.
//!
//! Fire events with `world(lua)?.fire(r, "Name", args)`.

use std::sync::LazyLock;

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::Ref;

#[path = "pages.rs"] pub mod pages;
#[path = "web_social.rs"] pub mod web_social;
#[path = "avatar.rs"] pub mod avatar;
#[path = "data.rs"] pub mod data;
#[path = "anim.rs"] pub mod anim;
#[path = "world.rs"] pub mod world;
#[path = "geometry.rs"] pub mod geometry;
#[path = "gui_input.rs"] pub mod gui_input;
#[path = "media.rs"] pub mod media;
#[path = "services.rs"] pub mod services;
#[path = "players_chat.rs"] pub mod players_chat;
#[path = "assets_cloud.rs"] pub mod assets_cloud;
#[path = "testing.rs"] pub mod testing;


/// Every domain's (method, base class).
pub static METHODS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    let mut v = Vec::new();
    v.extend_from_slice(pages::METHODS);
    v.extend_from_slice(web_social::METHODS);
    v.extend_from_slice(avatar::METHODS);
    v.extend_from_slice(data::METHODS);
    v.extend_from_slice(anim::METHODS);
    v.extend_from_slice(world::METHODS);
    v.extend_from_slice(geometry::METHODS);
    v.extend_from_slice(gui_input::METHODS);
    v.extend_from_slice(media::METHODS);
    v.extend_from_slice(services::METHODS);
    v.extend_from_slice(players_chat::METHODS);
    v.extend_from_slice(assets_cloud::METHODS);
    v.extend_from_slice(testing::METHODS);
    v
});

/// Methods a domain writes in Luau because they yield (each domain's
/// optional `LUA_METHODS`; the functions go in `__rs_lua_methods` from the
/// domain's `install`).
pub static LUA_METHODS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    let mut v = Vec::new();
    v.extend_from_slice(pages::LUA_METHODS);
    v.extend_from_slice(avatar::LUA_METHODS);
    v.extend_from_slice(anim::LUA_METHODS);
    v.extend_from_slice(services::LUA_METHODS);
    v.extend_from_slice(assets_cloud::LUA_METHODS);
    v.extend_from_slice(&data::LUA_METHODS);
    v.extend_from_slice(web_social::LUA_METHODS);
    v.extend_from_slice(geometry::LUA_METHODS);
    v.extend_from_slice(players_chat::LUA_METHODS);
    v.extend_from_slice(gui_input::LUA_METHODS);
    v
});

pub static EVENTS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    let mut v = Vec::new();
    v.extend_from_slice(pages::EVENTS);
    v.extend_from_slice(web_social::EVENTS);
    v.extend_from_slice(avatar::EVENTS);
    v.extend_from_slice(data::EVENTS);
    v.extend_from_slice(anim::EVENTS);
    v.extend_from_slice(world::EVENTS);
    v.extend_from_slice(geometry::EVENTS);
    v.extend_from_slice(gui_input::EVENTS);
    v.extend_from_slice(media::EVENTS);
    v.extend_from_slice(services::EVENTS);
    v.extend_from_slice(players_chat::EVENTS);
    v.extend_from_slice(assets_cloud::EVENTS);
    v.extend_from_slice(testing::EVENTS);
    v
});

pub fn event_exists(class: &str, name: &str) -> bool {
    EVENTS.iter().any(|(e, b)| *e == name && crate::doc::is_a(class, b))
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if let Some(r) = pages::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = web_social::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = avatar::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = data::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = anim::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = world::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = geometry::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = gui_input::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = media::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = services::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = players_chat::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = assets_cloud::call(lua, name, this, args) { return Some(r); }
    if let Some(r) = testing::call(lua, name, this, args) { return Some(r); }
    None
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(v) = pages::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = web_social::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = avatar::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = data::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = anim::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = world::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = geometry::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = gui_input::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = media::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = services::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = players_chat::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = assets_cloud::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    if let Some(v) = testing::prop_get(lua, r, class, key)? { return Ok(Some(v)); }
    Ok(None)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if pages::newindex(lua, r, class, key, value)? { return Ok(true); }
    if web_social::newindex(lua, r, class, key, value)? { return Ok(true); }
    if avatar::newindex(lua, r, class, key, value)? { return Ok(true); }
    if data::newindex(lua, r, class, key, value)? { return Ok(true); }
    if anim::newindex(lua, r, class, key, value)? { return Ok(true); }
    if world::newindex(lua, r, class, key, value)? { return Ok(true); }
    if geometry::newindex(lua, r, class, key, value)? { return Ok(true); }
    if gui_input::newindex(lua, r, class, key, value)? { return Ok(true); }
    if media::newindex(lua, r, class, key, value)? { return Ok(true); }
    if services::newindex(lua, r, class, key, value)? { return Ok(true); }
    if players_chat::newindex(lua, r, class, key, value)? { return Ok(true); }
    if assets_cloud::newindex(lua, r, class, key, value)? { return Ok(true); }
    if testing::newindex(lua, r, class, key, value)? { return Ok(true); }
    Ok(false)
}

pub fn write_acts(class: &str, key: &str) -> bool {
    false
        || pages::write_acts(class, key)
        || web_social::write_acts(class, key)
        || avatar::write_acts(class, key)
        || data::write_acts(class, key)
        || anim::write_acts(class, key)
        || world::write_acts(class, key)
        || geometry::write_acts(class, key)
        || gui_input::write_acts(class, key)
        || media::write_acts(class, key)
        || services::write_acts(class, key)
        || players_chat::write_acts(class, key)
        || assets_cloud::write_acts(class, key)
        || testing::write_acts(class, key)
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    pages::install(lua)?;
    web_social::install(lua)?;
    avatar::install(lua)?;
    data::install(lua)?;
    anim::install(lua)?;
    world::install(lua)?;
    geometry::install(lua)?;
    gui_input::install(lua)?;
    media::install(lua)?;
    services::install(lua)?;
    players_chat::install(lua)?;
    assets_cloud::install(lua)?;
    testing::install(lua)?;
    Ok(())
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    pages::step(lua, dt)?;
    web_social::step(lua, dt)?;
    avatar::step(lua, dt)?;
    data::step(lua, dt)?;
    anim::step(lua, dt)?;
    world::step(lua, dt)?;
    geometry::step(lua, dt)?;
    gui_input::step(lua, dt)?;
    media::step(lua, dt)?;
    services::step(lua, dt)?;
    players_chat::step(lua, dt)?;
    assets_cloud::step(lua, dt)?;
    testing::step(lua, dt)?;
    Ok(())
}
