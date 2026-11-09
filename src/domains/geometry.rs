//! Engine API domain: geometry. See src/domains/mod.rs for the hook contract.
//!
//! - PathfindingService / Path / PathWaypoint: geometry_nav.rs.
//! - EditableMesh / EditableImage: src/editable.rs + src/editable_mesh.rs
//!   (hooked from runtime_ext.rs).
#![allow(unused_imports, unused_variables)]

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::Ref;

use crate::runtime::*;

#[path = "geometry_nav.rs"]
pub mod nav;
#[path = "geometry_csg.rs"]
pub mod csg;
#[path = "geometry_ops.rs"]
pub mod ops;

pub static METHODS: &[(&str, &str)] = &[
    ("CreatePath", "PathfindingService"),
    ("GetWaypoints", "Path"),
    ("GetPointCoordinates", "Path"),
    ("CalculateConstraintsToPreserve", "GeometryService"),
    ("CreateBasicMeshPart", "GeometryService"),
    ("GenerateFragmentSites", "GeometryService"),
    ("SubstituteGeometry", "PartOperation"),
    ("ApplyMesh", "MeshPart"),
];
pub static EVENTS: &[(&str, &str)] = nav::EVENTS;
pub static LUA_METHODS: &[(&str, &str)] = &[
    ("FindPathAsync", "PathfindingService"),
    ("ComputeRawPathAsync", "PathfindingService"),
    ("ComputeSmoothPathAsync", "PathfindingService"),
    ("ComputeAsync", "Path"),
    ("CheckOcclusionAsync", "Path"),
    ("UnionAsync", "BasePart"),
    ("SubtractAsync", "BasePart"),
    ("IntersectAsync", "BasePart"),
    ("UnionAsync", "GeometryService"),
    ("SubtractAsync", "GeometryService"),
    ("IntersectAsync", "GeometryService"),
    ("FragmentAsync", "GeometryService"),
    ("SweepPartAsync", "GeometryService"),
    ("BakeToMeshAsync", "GeometryService"),
    ("GetSurfaceSlotsAsync", "MeshPart"),
];

/// An error's message alone (Studio's text, without mlua's prefix and
/// traceback).
pub fn plain_error(e: &mlua::Error) -> String {
    match e {
        mlua::Error::RuntimeError(s) => s.clone(),
        mlua::Error::CallbackError { cause, .. } => plain_error(cause),
        mlua::Error::WithContext { cause, .. } => plain_error(cause),
        other => other.to_string(),
    }
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if let Some(r) = nav::call(lua, name, this, args) {
        return Some(r);
    }
    if let Some(r) = ops::call(lua, name, this, args) {
        return Some(r);
    }
    None
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(v) = nav::prop_get(lua, r, class, key)? {
        return Ok(Some(v));
    }
    Ok(None)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    Ok(false)
}

pub fn write_acts(class: &str, key: &str) -> bool {
    false
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    nav::install(lua)?;
    ops::install(lua)?;
    Ok(())
}

pub fn step(lua: &Lua, dt: f64) -> mlua::Result<()> {
    nav::step(lua, dt)?;
    Ok(())
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
