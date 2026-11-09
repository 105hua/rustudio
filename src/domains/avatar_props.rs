//! Avatar property classes rustudio's reflection database doesn't know yet,
//! or that Roblox works out at run time:
//! - AvatarAccessoryRules / AnimationRules / BodyRules / ClothingRules /
//!   CollisionRules (the place's Avatar Settings): saved properties, stored
//!   on the instance (so they save, clone and replicate), with Studio's
//!   defaults (read from a running Studio 2026-11-07, checks/parity/avatar.md).
//!   ClassicPants/Shirt/TShirtOverride and BodyPartsCollidableOnDeath are in
//!   the API dump but flag-gated off in the Studio build we compared with
//!   ("not a valid member"); they behave like their siblings here.
//! - WrapLayer TargetShaping / deformation maps (saved), BaseWrap
//!   CageOriginWorld / ImportOriginWorld and WrapLayer ReferenceOriginWorld
//!   (the parent part's CFrame times the local origin; identity without a
//!   part, as Studio answers), DebugMode (stored; no debug drawing).
//! - Accoutrement AttachmentPos / Forward / Right / Up: views of
//!   AttachmentPoint (position, -look, right and up vectors), writable.

use mlua::{Lua, Value};
use rbx_dom_weak::types::{Content, Enum, NumberRange, Ref, Variant, Vector3};

use crate::runtime::*;

/// How a stored property converts.
#[derive(Clone, Copy)]
pub enum Kind {
    I64,
    Range(f32, f32),
    En(&'static str, u32),
    Bool(bool),
    F32(f32),
    V3(f32, f32, f32),
    Content,
}

/// (class, property, kind with default) for the stored properties.
pub fn stored(class: &str, key: &str) -> Option<Kind> {
    use Kind::*;
    let k = match (class, key) {
        ("AvatarAccessoryRules", k) if k.ends_with("AccessoryOverride") => I64,
        ("AvatarAccessoryRules", "AccessoryMode") => En("AvatarSettingsAccessoryMode", 0),
        ("AvatarAccessoryRules", "EnableEmissives" | "EnableSound" | "EnableVFX") => Bool(true),
        ("AvatarAccessoryRules", "LimitBounds") => V3(0.0, 0.0, 0.0),
        ("AvatarAccessoryRules", "LimitMethod") => En("AvatarSettingsAccessoryLimitMethod", 0),
        ("AvatarAnimationRules", k) if k.ends_with("AnimationOverride") => I64,
        ("AvatarAnimationRules", "AnimationPacksMode") => En("AvatarSettingsAnimationPacksMode", 0),
        ("AvatarBodyRules", "AvatarType") => En("GameAvatarType", 1),
        ("AvatarBodyRules", "BuildMode") => En("AvatarSettingsBuildMode", 0),
        ("AvatarBodyRules", "ScaleMode") => En("AvatarSettingsScaleMode", 0),
        ("AvatarBodyRules", "CustomBuildBodyType") => Range(0.0, 1.0),
        ("AvatarBodyRules", "CustomBuildHead") => Range(0.95, 1.0),
        ("AvatarBodyRules", "CustomBuildHeight") => Range(0.9, 1.05),
        ("AvatarBodyRules", "CustomBuildProportions") => Range(0.0, 1.0),
        ("AvatarBodyRules", "CustomBuildWidth") => Range(0.7, 1.0),
        ("AvatarBodyRules", "CustomWorldHeight") => Range(5.5, 5.5),
        ("AvatarBodyRules", k) if k.ends_with("Override") => I64,
        ("AvatarClothingRules", k) if k.ends_with("Override") => I64,
        ("AvatarClothingRules", "ClothingMode") => En("AvatarSettingsClothingMode", 0),
        ("AvatarClothingRules", "LimitBounds") => V3(0.0, 0.0, 0.0),
        ("AvatarCollisionRules", "BodyPartsCollidableOnDeath") => Bool(false),
        ("AvatarCollisionRules", "CollisionMode") => En("AvatarSettingsCollisionMode", 0),
        ("AvatarCollisionRules", "HitAndTouchDetectionMode") => En("AvatarSettingsHitAndTouchDetectionMode", 0),
        ("AvatarCollisionRules", "SingleColliderSize") => V3(2.0, 3.0, 1.0),
        ("WrapLayer", "TargetShaping") => F32(0.0),
        ("WrapLayer", "RotateScaleDeformationMap" | "ScaleOnlyDeformationMap") => Content,
        ("WrapLayer", "DebugMode") => En("WrapLayerDebugMode", 0),
        ("WrapTarget", "DebugMode") => En("WrapTargetDebugMode", 0),
        _ => return None,
    };
    // Only names the API dump has (so a typo'd key stays "not a valid member").
    crate::api::get()
        .and_then(|a| a.lookup(class, key))
        .map(|_| k)
}

fn default_variant(k: Kind) -> Variant {
    match k {
        Kind::I64 => Variant::Int64(0),
        Kind::Range(a, b) => Variant::NumberRange(NumberRange::new(a, b)),
        Kind::En(_, v) => Variant::Enum(Enum::from_u32(v)),
        Kind::Bool(b) => Variant::Bool(b),
        Kind::F32(f) => Variant::Float32(f),
        Kind::V3(x, y, z) => Variant::Vector3(Vector3::new(x, y, z)),
        Kind::Content => Variant::Content(Content::none()),
    }
}

fn enum_name(k: Kind) -> Option<&'static str> {
    match k {
        Kind::En(n, _) => Some(n),
        _ => None,
    }
}

fn type_name(k: Kind) -> &'static str {
    match k {
        Kind::I64 => "int64",
        Kind::Range(..) => "NumberRange",
        Kind::En(n, _) => n,
        Kind::Bool(_) => "bool",
        Kind::F32(_) => "float",
        Kind::V3(..) => "Vector3",
        Kind::Content => "Content",
    }
}

fn bad(class: &str, key: &str, want: &str, v: &Value) -> mlua::Error {
    let got = match v {
        Value::UserData(u) => u
            .metatable()
            .ok()
            .and_then(|m| m.get::<String>("__type").ok())
            .unwrap_or_else(|| "userdata".into()),
        _ => v.type_name().to_string(),
    };
    let _ = class;
    rt_err(format!("Unable to assign property {key}. {want} expected, got {got}"))
}

fn to_variant(class: &str, key: &str, k: Kind, v: &Value) -> mlua::Result<Variant> {
    let want = type_name(k);
    Ok(match k {
        Kind::I64 => Variant::Int64(num(v).ok_or_else(|| bad(class, key, "int64", v))? as i64),
        Kind::F32(_) => Variant::Float32(num(v).ok_or_else(|| bad(class, key, "float", v))? as f32),
        Kind::Bool(_) => match v {
            Value::Boolean(b) => Variant::Bool(*b),
            Value::Nil => Variant::Bool(false),
            _ => return Err(bad(class, key, "bool", v)),
        },
        Kind::Range(..) => match opt_ud::<crate::ltypes::LNumberRange>(v) {
            Some(r) => Variant::NumberRange(NumberRange::new(r.min, r.max)),
            None => return Err(bad(class, key, "NumberRange", v)),
        },
        Kind::V3(..) => match opt_ud::<crate::ltypes::LVector3>(v) {
            Some(p) => Variant::Vector3(Vector3::new(p.0.x, p.0.y, p.0.z)),
            None => return Err(bad(class, key, "Vector3", v)),
        },
        Kind::En(e, _) => {
            let item = match v {
                Value::UserData(_) => opt_ud::<crate::ltypes::LEnumItem>(v).filter(|i| i.enum_type == e),
                Value::Number(_) | Value::Integer(_) => crate::ltypes::LEnumItem::from_value(e, num(v).unwrap_or(-1.0) as u32),
                Value::String(s) => crate::ltypes::LEnumItem::from_name(e, &s.to_string_lossy()),
                _ => None,
            };
            match item {
                Some(i) => Variant::Enum(Enum::from_u32(i.value)),
                None => return Err(bad(class, key, &format!("EnumItem of type {e}"), v)),
            }
        }
        Kind::Content => match crate::runtime::editable::content_of(v) {
            Some(c) => Variant::Content(c),
            None => return Err(bad(class, key, want, v)),
        },
    })
}

/// The part a wrap sits in (its parent), if any.
fn parent_part(lua: &Lua, r: Ref) -> mlua::Result<Option<Ref>> {
    let w = world(lua)?;
    let d = w.d();
    let Some(p) = d.dom.get_by_ref(r).map(|i| i.parent()) else {
        return Ok(None);
    };
    Ok(d.class_of(p)
        .filter(|c| crate::doc::is_a(c, "BasePart"))
        .map(|_| p))
}

fn cframe_prop(lua: &Lua, r: Ref, name: &str) -> mlua::Result<crate::math::Xform> {
    Ok(match world(lua)?.d().prop(r, name) {
        Some(Variant::CFrame(cf)) => crate::math::Xform::from_cframe(&cf),
        _ => crate::math::Xform { pos: glam::Vec3::ZERO, rot: glam::Mat3::IDENTITY },
    })
}

fn cf_value(lua: &Lua, xf: crate::math::Xform) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(crate::ltypes::LCFrame(xf))?))
}

fn v3_value(lua: &Lua, v: glam::Vec3) -> mlua::Result<Value> {
    Ok(crate::ltypes::LVector3(v).into_value())
}

pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    if let Some(k) = stored(class, key) {
        let v = world(lua)?.d().prop(r, key).unwrap_or_else(|| default_variant(k));
        return Ok(Some(variant_to_lua(lua, &v, enum_name(k))?));
    }
    if crate::doc::is_a(class, "BaseWrap") {
        let local = match key {
            "CageOriginWorld" => "CageOrigin",
            "ImportOriginWorld" => "ImportOrigin",
            "ReferenceOriginWorld" if class == "WrapLayer" => "ReferenceOrigin",
            _ => return Ok(None),
        };
        let lo = cframe_prop(lua, r, local)?;
        let xf = match parent_part(lua, r)? {
            Some(p) => crate::ltypes::cf_mul(&cframe_prop(lua, p, "CFrame")?, &lo),
            None => lo,
        };
        return Ok(Some(cf_value(lua, xf)?));
    }
    if crate::doc::is_a(class, "Accoutrement") {
        let ap = cframe_prop(lua, r, "AttachmentPoint")?;
        let v = match key {
            "AttachmentPos" => ap.pos,
            "AttachmentRight" => ap.rot.x_axis,
            "AttachmentUp" => ap.rot.y_axis,
            "AttachmentForward" => -ap.rot.z_axis,
            _ => return Ok(None),
        };
        return Ok(Some(v3_value(lua, v)?));
    }
    Ok(None)
}

fn store(lua: &Lua, r: Ref, key: &str, v: Variant) -> mlua::Result<()> {
    store_prop(lua, r, key, v)
}

pub fn newindex(lua: &Lua, r: Ref, class: &str, key: &str, value: &Value) -> mlua::Result<bool> {
    if let Some(k) = stored(class, key) {
        let v = to_variant(class, key, k, value)?;
        store(lua, r, key, v)?;
        return Ok(true);
    }
    if crate::doc::is_a(class, "BaseWrap")
        && matches!(key, "CageOriginWorld" | "ImportOriginWorld" | "ReferenceOriginWorld")
    {
        return Err(rt_err(format!("Unable to assign property {key}. Property is read only")));
    }
    if crate::doc::is_a(class, "Accoutrement")
        && matches!(key, "AttachmentPos" | "AttachmentRight" | "AttachmentUp" | "AttachmentForward")
    {
        let Some(v) = opt_ud::<crate::ltypes::LVector3>(value).map(|v| v.0) else {
            return Err(bad(class, key, "Vector3", value));
        };
        let mut ap = cframe_prop(lua, r, "AttachmentPoint")?;
        // Roblox rebuilds AttachmentPoint from the vector set and the others
        // (Accoutrement::setAttachmentForward/Right/Up): a right-handed frame.
        let (mut right, mut up, mut back) = (ap.rot.x_axis, ap.rot.y_axis, ap.rot.z_axis);
        match key {
            "AttachmentPos" => ap.pos = v,
            "AttachmentRight" => {
                right = v.normalize_or_zero();
                back = right.cross(up).normalize_or_zero();
                up = back.cross(right).normalize_or_zero();
            }
            "AttachmentUp" => {
                up = v.normalize_or_zero();
                right = up.cross(back).normalize_or_zero();
                back = right.cross(up).normalize_or_zero();
            }
            _ => {
                back = -v.normalize_or_zero();
                right = up.cross(back).normalize_or_zero();
                up = back.cross(right).normalize_or_zero();
            }
        }
        if key != "AttachmentPos" && right.length_squared() > 0.0 && up.length_squared() > 0.0 {
            ap.rot = glam::Mat3::from_cols(right, up, back);
        }
        let cf = ap.to_cframe();
        store(lua, r, "AttachmentPoint", Variant::CFrame(cf))?;
        return Ok(true);
    }
    Ok(false)
}

pub fn write_acts(class: &str, key: &str) -> bool {
    stored(class, key).is_some()
        || (crate::doc::is_a(class, "Accoutrement")
            && matches!(key, "AttachmentPos" | "AttachmentRight" | "AttachmentUp" | "AttachmentForward"))
}
