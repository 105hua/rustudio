//! Terrain's legacy cell API on smooth voxels, as Studio 0.705 maps it
//! (measured in an offline Studio):
//! - SetCell / SetCells write a full voxel of the CellMaterial's smooth
//!   material (Granite → Slate, Iron / Aluminum / Gold / CinderBlock /
//!   Cement → Concrete, WoodPlank / WoodLog → WoodPlanks, Gravel → Asphalt,
//!   MossyStone → Pavement, RedPlastic → Sandstone, BluePlastic → Ice,
//!   Empty → Air); the CellBlock sets the occupancy (Solid 1, the wedges ½,
//!   CornerWedge ⅓, InverseCornerWedge ⅔; reads 1 / 0.5 / 0.336 / 0.668);
//!   the orientation is ignored. SetCells' Region3int16 is inclusive.
//! - GetCell maps back (Slate → Granite, Concrete → Cement, WoodPlanks →
//!   WoodPlank, Pavement → MossyStone, Sandstone → RedPlastic, Ice →
//!   BluePlastic, Water → Water, Air → Empty, anything else → Grass) and
//!   always answers block Solid, orientation NegZ.
//! - SetWaterCell writes a full Water voxel; GetWaterCell answers (is
//!   water, WaterForce.None, WaterDirection.NegX) — force and direction
//!   aren't kept by smooth terrain.
//! - CountCells = non-Air voxels. AutowedgeCell is false and AutowedgeCells
//!   does nothing (smooth terrain has no wedges to add).
//! - Material slots 0…63: 0…22 are the smooth materials (variant "", the
//!   terrain's MaterialColors), 23…63 empty; slots 0 / 1 (Air, Water) can't
//!   be set or reset; others take a terrain material, a variant name and a
//!   colour. Messages are Studio's.
//! - Read / Write / Modify / Iterate / ClearVoxelsAsync_beta: Studio
//!   refuses them all ("Beta is not enabled", before any argument check),
//!   so their operation objects (TerrainIterate/Modify/WriteOperation) are
//!   never reached by a script.

use std::cell::RefCell;
use std::collections::HashMap;

use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::Ref;

use super::util::*;
use crate::ltypes::*;
use crate::runtime::*;
use crate::terrain::{self, Voxels};

const CELL_TO_SMOOTH: &[(&str, &str)] = &[
    ("Empty", "Air"),
    ("Grass", "Grass"),
    ("Sand", "Sand"),
    ("Brick", "Brick"),
    ("Granite", "Slate"),
    ("Asphalt", "Asphalt"),
    ("Iron", "Concrete"),
    ("Aluminum", "Concrete"),
    ("Gold", "Concrete"),
    ("WoodPlank", "WoodPlanks"),
    ("WoodLog", "WoodPlanks"),
    ("Gravel", "Asphalt"),
    ("CinderBlock", "Concrete"),
    ("MossyStone", "Pavement"),
    ("Cement", "Concrete"),
    ("RedPlastic", "Sandstone"),
    ("BluePlastic", "Ice"),
    ("Water", "Water"),
];

const SMOOTH_TO_CELL: &[(&str, &str)] = &[
    ("Air", "Empty"),
    ("Grass", "Grass"),
    ("Sand", "Sand"),
    ("Brick", "Brick"),
    ("Slate", "Granite"),
    ("Asphalt", "Asphalt"),
    ("Concrete", "Cement"),
    ("WoodPlanks", "WoodPlank"),
    ("Pavement", "MossyStone"),
    ("Sandstone", "RedPlastic"),
    ("Ice", "BluePlastic"),
    ("Water", "Water"),
];

/// CellBlock → stored share (bytes 255 / 127 / 85 / 170, as Studio's).
fn block_share(b: u32) -> f32 {
    match b {
        1 | 4 => 127.0 / 255.0,
        2 => 85.0 / 255.0,
        3 => 170.0 / 255.0,
        _ => 1.0,
    }
}

thread_local! {
    /// (document id, terrain) → slot → (material, variant, colour).
    static SLOTS: RefCell<HashMap<(u64, Ref), HashMap<i64, (u8, String, [f32; 3])>>> = RefCell::new(HashMap::new());
}

/// The voxels being edited this frame, else the saved grid (as runtime_ext).
fn voxels(lua: &Lua, this: Ref) -> mlua::Result<Voxels> {
    let mut w = world(lua)?;
    Ok(match w.terrain_edit.take() {
        Some((r, v)) if r == this => v,
        Some(other) => {
            w.terrain_edit = Some(other);
            match Voxels::from_dom(&w.d().dom) {
                Some(Ok(v)) => v,
                _ => Voxels::empty(),
            }
        }
        None => match Voxels::from_dom(&w.d().dom) {
            Some(Ok(v)) => v,
            _ => Voxels::empty(),
        },
    })
}

fn put_back(lua: &Lua, this: Ref, v: Voxels) -> mlua::Result<()> {
    world(lua)?.terrain_edit = Some((this, v));
    Ok(())
}

/// Integer arguments 1..=n; Studio names the last missing one.
fn ints(args: &MultiValue, n: usize) -> mlua::Result<Vec<i32>> {
    if let Some(i) = (1..=n).rev().find(|i| num_arg(args, *i).is_none()) {
        return Err(missing(i));
    }
    Ok((1..=n).map(|i| num_arg(args, i).unwrap_or(0.0) as i32).collect())
}

fn cell_material(lua: &Lua, m: u8) -> mlua::Result<Value> {
    let smooth = terrain::material_name(m);
    let cell = SMOOTH_TO_CELL.iter().find(|(s, _)| *s == smooth).map(|(_, c)| *c).unwrap_or("Grass");
    enum_item(lua, "CellMaterial", cell)
}

pub fn call(lua: &Lua, name: &str, this: Ref, class: &str, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if class != "Terrain" {
        return None;
    }
    Some(match name {
        "GetCell" => (|| {
            let c = ints(args, 3)?;
            let v = voxels(lua, this)?;
            let (m, _) = v.get(c[0], c[1], c[2]);
            put_back(lua, this, v)?;
            Ok(MultiValue::from_iter([
                cell_material(lua, m)?,
                enum_item(lua, "CellBlock", "Solid")?,
                enum_item(lua, "CellOrientation", "NegZ")?,
            ]))
        })(),
        "GetWaterCell" => (|| {
            let c = ints(args, 3)?;
            let v = voxels(lua, this)?;
            let (m, _) = v.get(c[0], c[1], c[2]);
            put_back(lua, this, v)?;
            Ok(MultiValue::from_iter([
                Value::Boolean(terrain::is_water(m)),
                enum_item(lua, "WaterForce", "None")?,
                enum_item(lua, "WaterDirection", "NegX")?,
            ]))
        })(),
        "SetWaterCell" => (|| {
            if arg(args, 5).is_nil() {
                return Err(missing(5));
            }
            if arg(args, 4).is_nil() {
                return Err(missing(4));
            }
            let c = ints(args, 3)?;
            let mut v = voxels(lua, this)?;
            v.set([c[0], c[1], c[2]], terrain::WATER, 1.0);
            put_back(lua, this, v)?;
            none()
        })(),
        "SetCell" => (|| {
            for i in (4..=6).rev() {
                if arg(args, i).is_nil() {
                    return Err(missing(i));
                }
            }
            let c = ints(args, 3)?;
            let cm = enum_of(&arg(args, 4), "CellMaterial").ok_or_else(|| rt_err("Unable to cast value to CellMaterial"))?;
            let block = enum_of(&arg(args, 5), "CellBlock").unwrap_or(0);
            let mut v = voxels(lua, this)?;
            v.set([c[0], c[1], c[2]], smooth_of(cm), block_share(block));
            put_back(lua, this, v)?;
            none()
        })(),
        "SetCells" => (|| {
            for i in (1..=4).rev() {
                if arg(args, i).is_nil() {
                    return Err(missing(i));
                }
            }
            let reg = ud::<LRegion3int16>(&arg(args, 1)).ok_or_else(|| rt_err("Unable to cast value to Region3int16"))?;
            let cm = enum_of(&arg(args, 2), "CellMaterial").ok_or_else(|| rt_err("Unable to cast value to CellMaterial"))?;
            let block = enum_of(&arg(args, 3), "CellBlock").unwrap_or(0);
            let mat = smooth_of(cm);
            let (lo, hi) = (reg.min.0, reg.max.0);
            let mut cells = Vec::new();
            for x in lo[0]..=hi[0] {
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        cells.push(([x as i32, y as i32, z as i32], mat, block_share(block)));
                    }
                }
            }
            let mut v = voxels(lua, this)?;
            v.set_many(cells);
            put_back(lua, this, v)?;
            none()
        })(),
        "CountCells" => (|| {
            let v = voxels(lua, this)?;
            let n = v.count();
            put_back(lua, this, v)?;
            one(Value::Number(n as f64))
        })(),
        "AutowedgeCell" => (|| {
            ints(args, 3)?;
            one(Value::Boolean(false))
        })(),
        "AutowedgeCells" => {
            if arg(args, 1).is_nil() { Err(missing(1)) } else { none() }
        }
        "GetMaterialSlot" | "SetMaterialSlot" | "ResetMaterialSlot" => slot_call(lua, name, this, args),
        "ReadVoxelsAsync_beta" | "WriteVoxelsAsync_beta" | "ModifyVoxelsAsync_beta" | "IterateVoxelsAsync_beta"
        | "ClearVoxelsAsync_beta" => Err(rt_err("Beta is not enabled")),
        _ => return None,
    })
}

/// The smooth material index a CellMaterial value writes.
fn smooth_of(cm: u32) -> u8 {
    LEnumItem::from_value("CellMaterial", cm)
        .and_then(|e| CELL_TO_SMOOTH.iter().find(|(c, _)| *c == e.name).map(|(_, s)| *s))
        .and_then(terrain::material_index)
        .unwrap_or(0)
}

fn slot_call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let i = match arg(args, 1) {
        Value::Nil => return Err(missing(1)),
        v => crate::ltypes::num(&v).ok_or_else(|| {
            let what = ud::<LEnumItem>(&v).map(|e| e.enum_type).unwrap_or_else(|| v.type_name().to_string());
            rt_err(format!("Unable to cast {what} to int"))
        })? as i64,
    };
    if !(0..64).contains(&i) {
        return Err(rt_err("slotIndex is out of range"));
    }
    let doc_id = world(lua)?.d().id;
    match name {
        "GetMaterialSlot" => {
            let set = SLOTS.with(|s| s.borrow().get(&(doc_id, this)).and_then(|m| m.get(&i)).cloned());
            let (m, variant, color) = match set {
                Some(x) => x,
                None if i < 23 => {
                    let m = i as u8;
                    // Terrain.MaterialColors (as GetMaterialColor reads it).
                    let color = if m < 2 {
                        [0.0; 3]
                    } else {
                        let w = world(lua)?;
                        let rgb = match w.d().prop(this, "MaterialColors") {
                            Some(rbx_dom_weak::types::Variant::BinaryString(b)) if AsRef::<[u8]>::as_ref(&b).len() >= 69 => {
                                let b: &[u8] = b.as_ref();
                                [b[m as usize * 3], b[m as usize * 3 + 1], b[m as usize * 3 + 2]]
                            }
                            _ => terrain::material_color(m),
                        };
                        rgb.map(|x| x as f32 / 255.0)
                    };
                    (m, String::new(), color)
                }
                None => return none(),
            };
            Ok(MultiValue::from_iter([
                enum_item(lua, "Material", terrain::material_name(m))?,
                Value::String(lua.create_string(&variant)?),
                Value::UserData(lua.create_userdata(LColor3(color))?),
            ]))
        }
        "SetMaterialSlot" => {
            for k in (2..=4).rev() {
                if arg(args, k).is_nil() {
                    return Err(missing(k));
                }
            }
            if i < 2 {
                return Err(rt_err("Air and water slots cannot be set"));
            }
            let mat = ud::<LEnumItem>(&arg(args, 2))
                .filter(|e| e.enum_type == "Material")
                .ok_or_else(|| rt_err("Unable to cast value to Material"))?;
            let m = terrain::material_index(&mat.name)
                .filter(|m| *m >= 2)
                .ok_or_else(|| rt_err("baseMaterial is not a terrain material"))?;
            let variant = match arg(args, 3) {
                Value::String(s) => s.to_str()?.to_string(),
                v => return Err(rt_err(format!("Unable to cast {} to string", v.type_name()))),
            };
            let color = ud::<LColor3>(&arg(args, 4)).ok_or_else(|| rt_err("Unable to cast value to Color3"))?.0;
            SLOTS.with(|s| s.borrow_mut().entry((doc_id, this)).or_default().insert(i, (m, variant, color)));
            none()
        }
        _ => {
            if i < 2 {
                return Err(rt_err("Air and water slots cannot be reset"));
            }
            SLOTS.with(|s| {
                if let Some(m) = s.borrow_mut().get_mut(&(doc_id, this)) {
                    m.remove(&i);
                }
            });
            none()
        }
    }
}
