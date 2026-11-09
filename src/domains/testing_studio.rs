//! Studio-side services and plugin objects a script may hold: StudioService,
//! DraggerService, Plugin (GridSize, CollisionEnabled) read rustudio's
//! editor settings the way Studio's read its toolbar (measured in Studio
//! with move snapping on at 1 stud and rotate snapping off: GridSize 1,
//! RotateIncrement 0, LinearSnapEnabled true / 1, AngleSnapEnabled false /
//! 45); a script's writes to DraggerService stay on the service and don't
//! move the editor's settings (as in Studio, where they don't change
//! StudioService.GridSize either). Collaborator answers what Team Create
//! replicated. LiveSyncService: nothing is ever file-synced in rustudio.
//! Studio.LocalAssetsFolder can't be read (QDir), as in Studio.

use std::sync::RwLock;

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::ustr;

use super::*;

/// The editor's settings the Studio services report; the editor updates it
/// (play.rs, each Play frame).
#[derive(Clone, Copy, Debug)]
pub struct EditorState {
    pub snap_move: bool,
    pub move_inc: f32,
    pub snap_rotate: bool,
    pub rotate_inc: f32,
    pub local_space: bool,
    pub collisions: bool,
    pub join_surfaces: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        EditorState {
            snap_move: crate::mech::SNAP_ON_BY_DEFAULT,
            move_inc: crate::mech::MOVE_SNAP,
            snap_rotate: crate::mech::SNAP_ON_BY_DEFAULT,
            rotate_inc: crate::mech::ROTATE_SNAP_DEG,
            local_space: false,
            collisions: false,
            join_surfaces: false,
        }
    }
}

static EDITOR: RwLock<Option<EditorState>> = RwLock::new(None);

pub fn set_editor_state(s: EditorState) {
    if let Ok(mut e) = EDITOR.write() {
        *e = Some(s);
    }
}

fn editor() -> EditorState {
    EDITOR.read().ok().and_then(|e| *e).unwrap_or_default()
}

fn num(x: f64) -> Value {
    Value::Number(x)
}

/// The value a script wrote (kept on the instance), else `dflt`.
fn written_or(lua: &Lua, r: Ref, key: &str, dflt: Value) -> mlua::Result<Value> {
    Ok(match stored(lua, r, key)? {
        Value::Nil => dflt,
        v => v,
    })
}

/// A saved / replicated property of the instance, else `dflt`.
fn doc_or(lua: &Lua, r: Ref, key: &str, enum_name: Option<&str>, dflt: Value) -> mlua::Result<Value> {
    match doc_prop(lua, r, key)? {
        Some(v) => variant_to_lua(lua, &v, enum_name),
        None => Ok(dflt),
    }
}

fn color3(lua: &Lua, c: [f32; 3]) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(crate::ltypes::LColor3(c))?))
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    let e = editor();
    let v = match (class, key) {
        ("StudioService", "ActiveScript") => Value::Nil,
        ("StudioService", "DraggerSolveConstraints") => Value::Boolean(false),
        ("StudioService", "DrawConstraintsOnTop") => Value::Boolean(true),
        ("StudioService", "ShowConstraintDetails") => Value::Boolean(false),
        ("StudioService", "GridSize") | ("Plugin", "GridSize") => {
            num(if e.snap_move { e.move_inc as f64 } else { 0.0 })
        }
        ("StudioService", "RotateIncrement") => {
            num(if e.snap_rotate { e.rotate_inc as f64 } else { 0.0 })
        }
        ("StudioService", "StudioLocaleId") => lstr(lua, "en_US")?,
        ("StudioService", "UseLocalSpace") => written_or(lua, r, key, Value::Boolean(e.local_space))?,
        ("Plugin", "CollisionEnabled") => Value::Boolean(e.collisions),
        ("DraggerService", _) => {
            let dflt = match key {
                "AlignDraggedObjects" => Value::Boolean(false),
                "AngleSnapEnabled" => Value::Boolean(e.snap_rotate),
                "AngleSnapIncrement" => num(e.rotate_inc as f64),
                "AnimateHover" => Value::Boolean(false),
                "CollisionsEnabled" => Value::Boolean(e.collisions),
                "DraggerCoordinateSpace" => {
                    enum_item(lua, "DraggerCoordinateSpace", if e.local_space { 0 } else { 1 })?
                }
                "DraggerMovementMode" => enum_item(lua, "DraggerMovementMode", 0)?,
                "GeometrySnapColor" => color3(lua, [1.0, 0.0, 1.0])?,
                "HoverAnimateFrequency" => num(1.0),
                "HoverThickness" => num(4.0),
                "JointsEnabled" => Value::Boolean(e.join_surfaces),
                "LinearSnapEnabled" => Value::Boolean(e.snap_move),
                "LinearSnapIncrement" => num(e.move_inc as f64),
                "ShowHover" => Value::Boolean(true),
                "ShowPivotIndicator" => Value::Boolean(false),
                _ => return Ok(None),
            };
            written_or(lua, r, key, dflt)?
        }
        // Team Create replicates these onto each collaborator.
        ("Collaborator", _) => match key {
            "CollaboratorColor" => doc_or(lua, r, key, None, num(0.0))?,
            "CollaboratorColor3" => match doc_prop(lua, r, key)? {
                Some(v) => variant_to_lua(lua, &v, None)?,
                None => color3(lua, [0.0, 0.0, 0.0])?,
            },
            "CurDocGUID" | "Username" => doc_or(lua, r, key, None, lstr(lua, "")?)?,
            "CurScriptLineNumber" => doc_or(lua, r, key, None, num(0.0))?,
            "IsIdle" => doc_or(lua, r, key, None, Value::Boolean(false))?,
            "Status" => doc_or(lua, r, key, Some("CollaboratorStatus"), enum_item(lua, "CollaboratorStatus", 0)?)?,
            _ => return Ok(None),
        },
        ("LiveSyncService", "HasSyncedInstances") => Value::Boolean(false),
        ("Studio", "LocalAssetsFolder") => {
            return Err(rt_err("Unable to get property LocalAssetsFolder, type QDir"))
        }
        // What CreatePluginAction / the drag gave the object (Studio's
        // defaults for a bare one: "", true, "").
        ("PluginAction", "ActionId" | "StatusTip") | ("PluginDragEvent", "Data" | "MimeType" | "Sender") => {
            match prop_string(lua, r, key)? {
                Some(s) => lstr(lua, &s)?,
                None => written_or(lua, r, key, lstr(lua, "")?)?,
            }
        }
        ("PluginAction", "AllowBinding") => match doc_prop(lua, r, key)? {
            Some(Variant::Bool(b)) => Value::Boolean(b),
            _ => written_or(lua, r, key, Value::Boolean(true))?,
        },
        ("DockWidgetPluginGui", "HostWidgetWasRestored") => {
            written_or(lua, r, key, Value::Boolean(false))?
        }
        _ => return Ok(None),
    };
    Ok(Some(v))
}

fn type_err(key: &str, want: &str, v: &Value) -> mlua::Error {
    rt_err(format!(
        "Unable to assign property {key}. {want} expected, got {}",
        v.type_name()
    ))
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    let want = match (class, key) {
        ("DraggerService", "AlignDraggedObjects" | "AngleSnapEnabled" | "AnimateHover"
            | "CollisionsEnabled" | "JointsEnabled" | "LinearSnapEnabled" | "ShowHover"
            | "ShowPivotIndicator")
        | ("StudioService", "UseLocalSpace") => "bool",
        ("DraggerService", "AngleSnapIncrement" | "HoverAnimateFrequency" | "HoverThickness"
            | "LinearSnapIncrement") => "number",
        ("DraggerService", "GeometrySnapColor") => "Color3",
        ("DraggerService", "DraggerCoordinateSpace" | "DraggerMovementMode") => "EnumItem",
        _ => return Ok(false),
    };
    let ok = match want {
        "bool" => matches!(value, Value::Boolean(_)),
        "number" => matches!(value, Value::Number(_) | Value::Integer(_)),
        _ => matches!(value, Value::UserData(_)),
    };
    if !ok {
        return Err(type_err(key, want, value));
    }
    let v = match (want, value) {
        ("number", Value::Integer(i)) => Value::Number(*i as f64),
        // Enum items by the right enum only.
        ("EnumItem", v) => match opt_ud::<crate::ltypes::LEnumItem>(v) {
            Some(item) if item.enum_type == key => v.clone(),
            _ => return Err(type_err(key, "EnumItem", value)),
        },
        _ => value.clone(),
    };
    let old = stored(lua, r, key)?;
    store(lua, r)?.set(key, v.clone())?;
    if !old.equals(&v).unwrap_or(false) {
        let mut w = world(lua)?;
        w.fire(r, "Changed", vec![Value::String(lua.create_string(key)?)]);
        w.fire(r, &format!("Changed:{key}"), vec![]);
    }
    Ok(true)
}

pub fn call(
    lua: &Lua,
    class: &str,
    name: &str,
    this: Ref,
    args: &MultiValue,
) -> Option<mlua::Result<MultiValue>> {
    let r = match name {
        "GetSyncState" if class == "LiveSyncService" => (|| {
            if opt_inst(&arg(args, 1)).is_none() {
                return Err(rt_err("Argument 1 missing or nil"));
            }
            ret1(enum_item(lua, "InstanceFileSyncStatus", 0)?)
        })(),
        // The widget's close callback (called when the widget closes; no
        // plugin widget ever closes in rustudio's play session).
        "BindToClose" if crate::doc::is_a(class, "PluginGui") => (|| {
            let f = arg(args, 1);
            if !matches!(f, Value::Nil | Value::Function(_)) {
                return Err(rt_err(format!("Unable to cast {} to Function", f.type_name())));
            }
            store(lua, this)?.set("__close", f)?;
            ret0()
        })(),
        _ => return None,
    };
    Some(r)
}
