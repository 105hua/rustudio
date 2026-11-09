//! HumanoidDescription accessories and emotes, as Studio keeps them
//! (behaviour read from a running Studio, checks/parity/avatar.md):
//! - One ordered list of accessories. The rigid string properties
//!   (HatAccessory "1,2" …) are views of its rigid entries; writing one moves
//!   that type's entries to the end. The full list (with layered clothing,
//!   Order, Puffiness, Position / Rotation / Scale) is kept in AccessoryBlob,
//!   so it survives Clone and replication.
//! - SetAccessories checks every entry first and, on a bad one, warns as
//!   Studio does and changes nothing.
//! - Emotes (name → asset ids) and equipped emotes ({Slot, Name}) are kept
//!   in EmotesDataInternal / EquippedEmotesDataInternal (JSON); every change
//!   fires EmotesChanged / EquippedEmotesChanged with the new table.

use mlua::{Lua, MultiValue, Table, Value};
use rbx_dom_weak::types::{Ref, Variant};
use serde_json::{json, Value as J};

use crate::runtime::*;

/// AccessoryType item name ↔ the rigid string property, in Studio's order.
pub const RIGID: &[(&str, &str)] = &[
    ("Hat", "HatAccessory"),
    ("Hair", "HairAccessory"),
    ("Face", "FaceAccessory"),
    ("Neck", "NeckAccessory"),
    ("Shoulder", "ShouldersAccessory"),
    ("Front", "FrontAccessory"),
    ("Back", "BackAccessory"),
    ("Waist", "WaistAccessory"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Acc {
    pub asset_id: i64,
    /// Enum.AccessoryType item name.
    pub ty: String,
    pub layered: bool,
    pub order: Option<i64>,
    pub puffiness: Option<f32>,
    pub pos: [f32; 3],
    pub rot: [f32; 3],
    pub scale: [f32; 3],
}

impl Acc {
    fn rigid(asset_id: i64, ty: &str) -> Acc {
        Acc {
            asset_id,
            ty: ty.to_string(),
            layered: false,
            order: None,
            puffiness: None,
            pos: [0.0; 3],
            rot: [0.0; 3],
            scale: [1.0; 3],
        }
    }

    fn to_json(&self) -> J {
        json!({
            "AssetId": self.asset_id, "AccessoryType": self.ty, "IsLayered": self.layered,
            "Order": self.order, "Puffiness": self.puffiness,
            "Position": self.pos, "Rotation": self.rot, "Scale": self.scale,
        })
    }

    fn from_json(v: &J) -> Option<Acc> {
        let v3 = |k: &str, d: f32| -> [f32; 3] {
            let a = v.get(k).and_then(|a| a.as_array());
            let at = |i: usize| a.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(d);
            [at(0), at(1), at(2)]
        };
        Some(Acc {
            asset_id: v.get("AssetId")?.as_i64()?,
            ty: v.get("AccessoryType")?.as_str()?.to_string(),
            layered: v.get("IsLayered").and_then(|b| b.as_bool()).unwrap_or(false),
            order: v.get("Order").and_then(|o| o.as_i64()),
            puffiness: v.get("Puffiness").and_then(|p| p.as_f64()).map(|p| p as f32),
            pos: v3("Position", 0.0),
            rot: v3("Rotation", 0.0),
            scale: v3("Scale", 1.0),
        })
    }
}

/// The ids a rigid property names ("13, 14,,abc" → [13, 14]).
pub fn parse_ids(s: &str) -> Vec<i64> {
    s.split(',')
        .filter_map(|p| p.trim().parse::<i64>().ok())
        .filter(|n| *n > 0)
        .collect()
}

fn str_prop(lua: &Lua, r: Ref, name: &str) -> mlua::Result<String> {
    Ok(match world(lua)?.d().prop(r, name) {
        Some(Variant::String(s)) => s,
        Some(Variant::BinaryString(b)) => String::from_utf8_lossy(b.as_ref()).into_owned(),
        _ => String::new(),
    })
}

/// The description's accessory list (the blob reconciled with the rigid
/// string properties, which may have been written since).
pub fn accessories(lua: &Lua, r: Ref) -> mlua::Result<Vec<Acc>> {
    let blob = str_prop(lua, r, "AccessoryBlob")?;
    let mut list: Vec<Acc> = serde_json::from_str::<J>(&blob)
        .ok()
        .and_then(|v| v.as_array().map(|a| a.iter().filter_map(Acc::from_json).collect()))
        .unwrap_or_default();
    for (ty, prop) in RIGID {
        let want = parse_ids(&str_prop(lua, r, prop)?);
        let have: Vec<i64> = list
            .iter()
            .filter(|a| !a.layered && a.ty == *ty)
            .map(|a| a.asset_id)
            .collect();
        if want != have {
            list.retain(|a| a.layered || a.ty != *ty);
            list.extend(want.into_iter().map(|id| Acc::rigid(id, ty)));
        }
    }
    Ok(list)
}

/// Stores a list: the blob and each rigid property.
pub fn set_accessories_list(lua: &Lua, r: Ref, list: &[Acc]) -> mlua::Result<()> {
    let blob = J::Array(list.iter().map(Acc::to_json).collect()).to_string();
    store_prop(lua, r, "AccessoryBlob", Variant::String(blob))?;
    for (ty, prop) in RIGID {
        let ids: Vec<String> = list
            .iter()
            .filter(|a| !a.layered && a.ty == *ty)
            .map(|a| a.asset_id.to_string())
            .collect();
        let s = ids.join(",");
        if str_prop(lua, r, prop)? != s {
            store_prop(lua, r, prop, Variant::String(s))?;
        }
    }
    Ok(())
}

fn v3(lua: &Lua, v: [f32; 3]) -> mlua::Result<Value> {
    Ok(crate::ltypes::LVector3(glam::Vec3::from(v)).into_value())
}

fn enum_item(lua: &Lua, ty: &str, name: &str) -> mlua::Result<Value> {
    match crate::ltypes::LEnumItem::from_name(ty, name) {
        Some(i) => crate::ltypes::enum_value(lua, i),
        None => Ok(Value::Nil),
    }
}

fn warn(lua: &Lua, msg: &str) -> mlua::Result<()> {
    world(lua)?.output.push((crate::app::Level::Warn, msg.to_string()));
    Ok(())
}

/// "Unable to cast X to T" for a value of the wrong type, as Studio's
/// argument conversion says.
pub fn cast_err(v: &Value, to: &str) -> mlua::Error {
    let from = match v {
        Value::Number(_) | Value::Integer(_) => "double",
        Value::Boolean(_) => "bool",
        Value::String(_) => "string",
        Value::Table(_) => "table",
        Value::Function(_) => "function",
        Value::Nil => "nil",
        _ => "userdata",
    };
    rt_err(format!("Unable to cast {from} to {to}"))
}

pub fn table_arg(args: &MultiValue, i: usize, to: &str) -> mlua::Result<Table> {
    match args.get(i) {
        Some(Value::Table(t)) => Ok(t.clone()),
        None | Some(Value::Nil) => Err(rt_err(format!("Argument {i} missing or nil"))),
        Some(v) => Err(cast_err(v, to)),
    }
}

pub fn int64_arg(args: &MultiValue, i: usize) -> mlua::Result<i64> {
    match args.get(i) {
        None | Some(Value::Nil) => Err(rt_err(format!("Argument {i} missing or nil"))),
        Some(Value::String(s)) => s
            .to_str()
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|n| n as i64)
            .ok_or_else(|| rt_err("Unable to cast string to int64")),
        Some(v) => num(v).map(|n| n as i64).ok_or_else(|| cast_err(v, "int64")),
    }
}

pub fn string_arg(args: &MultiValue, i: usize) -> mlua::Result<String> {
    match args.get(i) {
        None | Some(Value::Nil) => Err(rt_err(format!("Argument {i} missing or nil"))),
        Some(Value::String(s)) => Ok(s.to_string_lossy()),
        Some(v @ (Value::Number(_) | Value::Integer(_))) => Ok(num(v).map(|n| {
            if n.fract() == 0.0 { format!("{}", n as i64) } else { n.to_string() }
        }).unwrap_or_default()),
        Some(v) => Err(cast_err(v, "string")),
    }
}

pub fn bool_arg(args: &MultiValue, i: usize) -> bool {
    !matches!(args.get(i), None | Some(Value::Nil) | Some(Value::Boolean(false)))
}

fn acc_table(lua: &Lua, a: &Acc) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("AssetId", a.asset_id)?;
    t.set("AccessoryType", enum_item(lua, "AccessoryType", &a.ty)?)?;
    t.set("IsLayered", a.layered)?;
    t.set("Position", v3(lua, a.pos)?)?;
    t.set("Rotation", v3(lua, a.rot)?)?;
    t.set("Scale", v3(lua, a.scale)?)?;
    if a.layered {
        t.set("Order", a.order.unwrap_or(0))?;
        t.set("Puffiness", a.puffiness.unwrap_or(1.0))?;
    }
    Ok(t)
}

fn read_v3(t: &Table, k: &str, d: f32) -> mlua::Result<[f32; 3]> {
    Ok(match t.get::<Value>(k)? {
        v @ (Value::UserData(_) | Value::Vector(_)) => opt_ud::<crate::ltypes::LVector3>(&v).map(|v| v.0.to_array()).unwrap_or([d; 3]),
        _ => [d; 3],
    })
}

fn set_accessories(lua: &Lua, r: Ref, args: &MultiValue) -> mlua::Result<()> {
    let t = table_arg(args, 1, "Array")?;
    let rigid_too = bool_arg(args, 2);
    let mut given = Vec::new();
    for e in t.sequence_values::<Value>() {
        let Value::Table(e) = e? else {
            warn(lua, "Input table contained an invalid entry!")?;
            return Ok(());
        };
        let id = match e.get::<Value>("AssetId")? {
            v @ (Value::Number(_) | Value::Integer(_)) => num(&v).unwrap_or(0.0) as i64,
            _ => 0,
        };
        if id <= 0 {
            warn(lua, "Input table contained an invalid asset id!")?;
            return Ok(());
        }
        let ty = match opt_ud::<crate::ltypes::LEnumItem>(&e.get::<Value>("AccessoryType")?) {
            Some(i) if i.enum_type == "AccessoryType" => i.name,
            _ => {
                warn(lua, "Input table contained an invalid accessory type!")?;
                return Ok(());
            }
        };
        let layered = matches!(e.get::<Value>("IsLayered")?, Value::Boolean(true));
        let order = num(&e.get::<Value>("Order")?).map(|n| n as i64);
        if layered && order.is_none() {
            warn(lua, "HumanoidDescription:SetAccessories Input table missing Order!")?;
            return Ok(());
        }
        given.push(Acc {
            asset_id: id,
            ty,
            layered,
            order: if layered { order } else { None },
            puffiness: if layered { Some(num(&e.get::<Value>("Puffiness")?).unwrap_or(1.0) as f32) } else { None },
            pos: read_v3(&e, "Position", 0.0)?,
            rot: read_v3(&e, "Rotation", 0.0)?,
            scale: read_v3(&e, "Scale", 1.0)?,
        });
    }
    let list = if rigid_too {
        given
    } else {
        let mut keep: Vec<Acc> = accessories(lua, r)?.into_iter().filter(|a| !a.layered).collect();
        keep.extend(given.into_iter().filter(|a| a.layered));
        keep
    };
    set_accessories_list(lua, r, &list)
}

// ------------------------------------------------------------------ emotes

/// Emotes in insertion order (name, ids).
pub fn emotes(lua: &Lua, r: Ref) -> mlua::Result<Vec<(String, Vec<i64>)>> {
    let s = str_prop(lua, r, "EmotesDataInternal")?;
    Ok(serde_json::from_str::<J>(&s)
        .ok()
        .and_then(|v| {
            v.as_array().map(|a| {
                a.iter()
                    .filter_map(|e| {
                        let name = e.get(0)?.as_str()?.to_string();
                        let ids = e.get(1)?.as_array()?.iter().filter_map(|x| x.as_i64()).collect();
                        Some((name, ids))
                    })
                    .collect()
            })
        })
        .unwrap_or_default())
}

pub fn equipped(lua: &Lua, r: Ref) -> mlua::Result<Vec<(i64, String)>> {
    let s = str_prop(lua, r, "EquippedEmotesDataInternal")?;
    Ok(serde_json::from_str::<J>(&s)
        .ok()
        .and_then(|v| {
            v.as_array().map(|a| {
                a.iter()
                    .filter_map(|e| Some((e.get(0)?.as_i64()?, e.get(1)?.as_str()?.to_string())))
                    .collect()
            })
        })
        .unwrap_or_default())
}

fn emotes_table(lua: &Lua, list: &[(String, Vec<i64>)]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (name, ids) in list {
        t.set(name.as_str(), lua.create_sequence_from(ids.iter().copied())?)?;
    }
    Ok(t)
}

fn equipped_table(lua: &Lua, list: &[(i64, String)]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (i, (slot, name)) in list.iter().enumerate() {
        let e = lua.create_table()?;
        e.set("Slot", *slot)?;
        e.set("Name", name.as_str())?;
        t.set(i + 1, e)?;
    }
    Ok(t)
}

fn store_emotes(lua: &Lua, r: Ref, list: &[(String, Vec<i64>)]) -> mlua::Result<()> {
    let j = J::Array(list.iter().map(|(n, ids)| json!([n, ids])).collect()).to_string();
    store_prop(lua, r, "EmotesDataInternal", Variant::String(j))?;
    let t = emotes_table(lua, list)?;
    world(lua)?.fire(r, "EmotesChanged", vec![Value::Table(t)]);
    Ok(())
}

fn store_equipped(lua: &Lua, r: Ref, list: &[(i64, String)]) -> mlua::Result<()> {
    let j = J::Array(list.iter().map(|(s, n)| json!([s, n])).collect()).to_string();
    store_prop(lua, r, "EquippedEmotesDataInternal", Variant::String(j))?;
    let t = equipped_table(lua, list)?;
    world(lua)?.fire(r, "EquippedEmotesChanged", vec![Value::Table(t)]);
    Ok(())
}

fn ids_of(v: &Value) -> Option<Vec<i64>> {
    match v {
        Value::Table(t) => Some(
            t.clone()
                .sequence_values::<Value>()
                .filter_map(|x| x.ok().and_then(|x| num(&x)).map(|n| n as i64))
                .collect(),
        ),
        _ => None,
    }
}

pub const METHODS: &[(&str, &str)] = &[
    ("GetAccessories", "HumanoidDescription"),
    ("SetAccessories", "HumanoidDescription"),
    ("AddEmote", "HumanoidDescription"),
    ("RemoveEmote", "HumanoidDescription"),
    ("GetEmotes", "HumanoidDescription"),
    ("SetEmotes", "HumanoidDescription"),
    ("GetEquippedEmotes", "HumanoidDescription"),
    ("SetEquippedEmotes", "HumanoidDescription"),
];


pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let one = |v: Value| Ok(MultiValue::from_iter([v]));
    match name {
        "GetAccessories" => {
            let rigid_too = match args.get(1) {
                None | Some(Value::Nil) => return Err(rt_err("Argument 1 missing or nil")),
                Some(v) => !matches!(v, Value::Boolean(false)),
            };
            let t = lua.create_table()?;
            let mut n = 0;
            for a in accessories(lua, this)?.iter().filter(|a| rigid_too || a.layered) {
                n += 1;
                t.set(n, acc_table(lua, a)?)?;
            }
            one(Value::Table(t))
        }
        "SetAccessories" => {
            set_accessories(lua, this, args)?;
            Ok(MultiValue::new())
        }
        "GetEmotes" => one(Value::Table(emotes_table(lua, &emotes(lua, this)?)?)),
        "GetEquippedEmotes" => one(Value::Table(equipped_table(lua, &equipped(lua, this)?)?)),
        "AddEmote" => {
            let name = string_arg(args, 1)?;
            let id = int64_arg(args, 2)?;
            let mut list = emotes(lua, this)?;
            match list.iter_mut().find(|(n, _)| *n == name) {
                Some((_, ids)) => {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                None => list.push((name, vec![id])),
            }
            store_emotes(lua, this, &list)?;
            Ok(MultiValue::new())
        }
        "RemoveEmote" => {
            let name = string_arg(args, 1)?;
            let mut list = emotes(lua, this)?;
            let before = list.len();
            list.retain(|(n, _)| *n != name);
            if list.len() != before {
                store_emotes(lua, this, &list)?;
            }
            Ok(MultiValue::new())
        }
        "SetEmotes" => {
            let t = table_arg(args, 1, "Dictionary")?;
            let mut list = Vec::new();
            for pair in t.pairs::<Value, Value>() {
                let (k, v) = pair?;
                let Value::String(k) = k else { continue };
                if let Some(ids) = ids_of(&v) {
                    list.push((k.to_string_lossy(), ids));
                }
            }
            list.sort_by(|a, b| a.0.cmp(&b.0));
            store_emotes(lua, this, &list)?;
            Ok(MultiValue::new())
        }
        "SetEquippedEmotes" => {
            let t = table_arg(args, 1, "Array")?;
            let mut list = Vec::new();
            for (i, v) in t.sequence_values::<Value>().enumerate() {
                match v? {
                    Value::String(s) => list.push(((i + 1) as i64, s.to_string_lossy())),
                    Value::Table(e) => {
                        let slot = num(&e.get::<Value>("Slot")?).map(|n| n as i64).unwrap_or((i + 1) as i64);
                        if let Value::String(n) = e.get::<Value>("Name")? {
                            list.push((slot, n.to_string_lossy()));
                        }
                    }
                    _ => {}
                }
            }
            store_equipped(lua, this, &list)?;
            Ok(MultiValue::new())
        }
        _ => Err(rt_err(format!("{name} is not a valid member of HumanoidDescription"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_ids_parse_like_studio() {
        assert_eq!(parse_ids("13, 14,,abc"), vec![13, 14]);
        assert_eq!(parse_ids(""), Vec::<i64>::new());
    }

    #[test]
    fn accessory_json_round_trips() {
        let a = Acc { order: Some(3), puffiness: Some(0.5), layered: true, ..Acc::rigid(11, "Shirt") };
        assert_eq!(Acc::from_json(&a.to_json()), Some(a));
    }
}
