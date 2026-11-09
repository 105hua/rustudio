//! Property values in project files (`$properties`, `.meta.json` /
//! `.data.json` `properties`, `.model.json` `Properties`), as Rojo 7 and Argon
//! read them:
//!
//! - **implicit**: the JSON shape is read as the property's type from the
//!   reflection database (`"Anchored": true`, `"Size": [4, 1, 2]`, `"CFrame":
//!   [x, y, z, r00…r22]`, an enum item's name, `"Color": [r, g, b]`);
//! - **explicit**: an object with one key naming the type (`{"Vector3": [1,
//!   2, 3]}`, `{"Enum": 512}`, `{"String": "…"}`), for properties the
//!   database doesn't know.
//!
//! Argon's one extension: a BasePart `Color` array with a component above 1
//! is 0–255 (its syncback writes parts' colours that way). The writer side
//! (`to_json`) produces the implicit form, numbers trimmed like Argon's.

use rbx_dom_weak::types::{
    Attributes, Axes, BrickColor, CFrame, Color3, Color3uint8, ColorSequence,
    ColorSequenceKeypoint, Content, ContentId, CustomPhysicalProperties, Enum, Faces, Font,
    FontStyle, FontWeight, Matrix3, NumberRange, NumberSequence, NumberSequenceKeypoint,
    PhysicalProperties, Rect, Tags, UDim, UDim2, Variant, VariantType, Vector2, Vector2int16,
    Vector3, Vector3int16,
};
use rbx_reflection::{DataType, PropertyKind};
use serde_json::{json, Value as J};

/// The property's descriptor (aliases followed): (canonical name, type).
pub fn descriptor(class: &str, prop: &str) -> Option<(String, DataType<'static>)> {
    let db = crate::doc::db();
    let mut cur = db.classes.get(class)?;
    loop {
        if let Some(p) = cur.properties.get(prop) {
            if let PropertyKind::Alias { alias_for } = &p.kind {
                return descriptor(class, alias_for);
            }
            return Some((p.name.to_string(), p.data_type.clone()));
        }
        cur = db.classes.get(cur.superclass.as_deref()?)?;
    }
}

fn is_base_part(class: &str) -> bool {
    crate::doc::is_a(class, "BasePart")
}

fn nums<const N: usize>(v: &J) -> Option<[f32; N]> {
    let a = v.as_array()?;
    if a.len() != N {
        return None;
    }
    let mut out = [0f32; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64()? as f32;
    }
    Some(out)
}

fn cframe(v: &J) -> Option<CFrame> {
    if let Some(c) = nums::<12>(v) {
        return Some(CFrame::new(
            Vector3::new(c[0], c[1], c[2]),
            Matrix3::new(
                Vector3::new(c[3], c[4], c[5]),
                Vector3::new(c[6], c[7], c[8]),
                Vector3::new(c[9], c[10], c[11]),
            ),
        ));
    }
    // Rojo's object form: {"position": [..], "orientation": [[..], [..], [..]]}.
    let p = nums::<3>(v.get("position")?)?;
    let o = v.get("orientation")?.as_array()?;
    let r: Vec<[f32; 3]> = o.iter().filter_map(nums::<3>).collect();
    (r.len() == 3).then(|| {
        CFrame::new(
            Vector3::new(p[0], p[1], p[2]),
            Matrix3::new(
                Vector3::new(r[0][0], r[0][1], r[0][2]),
                Vector3::new(r[1][0], r[1][1], r[1][2]),
                Vector3::new(r[2][0], r[2][1], r[2][2]),
            ),
        )
    })
}

fn font(v: &J) -> Option<Font> {
    let family = v.get("family")?.as_str()?;
    let weight = match v.get("weight").and_then(|w| w.as_str()).unwrap_or("Regular") {
        "Thin" => FontWeight::Thin,
        "ExtraLight" => FontWeight::ExtraLight,
        "Light" => FontWeight::Light,
        "Medium" => FontWeight::Medium,
        "SemiBold" => FontWeight::SemiBold,
        "Bold" => FontWeight::Bold,
        "ExtraBold" => FontWeight::ExtraBold,
        "Heavy" => FontWeight::Heavy,
        _ => FontWeight::Regular,
    };
    let style = match v.get("style").and_then(|s| s.as_str()) {
        Some("Italic") => FontStyle::Italic,
        _ => FontStyle::Normal,
    };
    Some(Font::new(family, weight, style))
}

fn attributes(v: &J) -> Option<Attributes> {
    let mut out = Attributes::new();
    for (k, x) in v.as_object()? {
        let val = match x {
            J::Bool(b) => Variant::Bool(*b),
            J::Number(n) => Variant::Float64(n.as_f64()?),
            J::String(s) => Variant::String(s.clone()),
            J::Object(o) if o.len() == 1 => {
                let (ty, inner) = o.iter().next()?;
                explicit(ty, inner, "", "")?
            }
            _ => continue,
        };
        out.insert(k.clone(), val);
    }
    Some(out)
}

/// A value of a known type from its JSON (the shapes Rojo and Argon write).
fn typed(ty: VariantType, v: &J, class: &str, prop: &str) -> Option<Variant> {
    Some(match ty {
        VariantType::Bool => Variant::Bool(v.as_bool()?),
        VariantType::String => Variant::String(v.as_str()?.to_string()),
        VariantType::Float32 => Variant::Float32(v.as_f64()? as f32),
        VariantType::Float64 => Variant::Float64(v.as_f64()?),
        VariantType::Int32 => Variant::Int32(v.as_f64()? as i32),
        VariantType::Int64 => Variant::Int64(v.as_f64()? as i64),
        VariantType::Vector3 => {
            let a = nums::<3>(v)?;
            Variant::Vector3(Vector3::new(a[0], a[1], a[2]))
        }
        VariantType::Vector2 => {
            let a = nums::<2>(v)?;
            Variant::Vector2(Vector2::new(a[0], a[1]))
        }
        VariantType::Vector3int16 => {
            let a = nums::<3>(v)?;
            Variant::Vector3int16(Vector3int16::new(a[0] as i16, a[1] as i16, a[2] as i16))
        }
        VariantType::Vector2int16 => {
            let a = nums::<2>(v)?;
            Variant::Vector2int16(Vector2int16::new(a[0] as i16, a[1] as i16))
        }
        VariantType::CFrame => Variant::CFrame(cframe(v)?),
        VariantType::OptionalCFrame => Variant::OptionalCFrame(Some(cframe(v)?)),
        VariantType::Color3 => {
            let a = nums::<3>(v)?;
            // Argon: a part's Color written 0–255.
            if is_base_part(class) && prop == "Color" && a.iter().any(|c| *c > 1.0) {
                return Some(Variant::Color3uint8(Color3uint8::new(
                    a[0] as u8, a[1] as u8, a[2] as u8,
                )));
            }
            if is_base_part(class) && prop == "Color" {
                return Some(Variant::Color3uint8(Color3uint8::new(
                    (a[0] * 255.0).round() as u8,
                    (a[1] * 255.0).round() as u8,
                    (a[2] * 255.0).round() as u8,
                )));
            }
            Variant::Color3(Color3::new(a[0], a[1], a[2]))
        }
        VariantType::Color3uint8 => {
            let a = nums::<3>(v)?;
            Variant::Color3uint8(Color3uint8::new(a[0] as u8, a[1] as u8, a[2] as u8))
        }
        VariantType::UDim => {
            let a = nums::<2>(v)?;
            Variant::UDim(UDim::new(a[0], a[1] as i32))
        }
        VariantType::UDim2 => {
            let a = v.as_array()?;
            let (x, y) = (nums::<2>(a.first()?)?, nums::<2>(a.get(1)?)?);
            Variant::UDim2(UDim2::new(UDim::new(x[0], x[1] as i32), UDim::new(y[0], y[1] as i32)))
        }
        VariantType::NumberRange => {
            let a = nums::<2>(v)?;
            Variant::NumberRange(NumberRange::new(a[0], a[1]))
        }
        VariantType::Rect => {
            let a = nums::<4>(v)?;
            Variant::Rect(Rect::new(Vector2::new(a[0], a[1]), Vector2::new(a[2], a[3])))
        }
        VariantType::Content => Variant::Content(Content::from(v.as_str()?)),
        VariantType::ContentId => Variant::ContentId(ContentId::from(v.as_str()?)),
        VariantType::BrickColor => Variant::BrickColor(match v {
            J::Number(n) => BrickColor::from_number(n.as_f64()? as u16)?,
            J::String(s) => BrickColor::from_name(s)?,
            _ => return None,
        }),
        VariantType::Tags => Variant::Tags(Tags::from(
            v.as_array()?
                .iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect::<Vec<_>>(),
        )),
        VariantType::Attributes => Variant::Attributes(attributes(v)?),
        VariantType::Font => Variant::Font(font(v)?),
        VariantType::Faces | VariantType::Axes => {
            let names: Vec<&str> = v.as_array()?.iter().filter_map(|x| x.as_str()).collect();
            let has = |n: &str| names.contains(&n);
            if ty == VariantType::Faces {
                let mut bits = 0u8;
                for (n, bit) in [
                    ("Right", Faces::RIGHT),
                    ("Top", Faces::TOP),
                    ("Back", Faces::BACK),
                    ("Left", Faces::LEFT),
                    ("Bottom", Faces::BOTTOM),
                    ("Front", Faces::FRONT),
                ] {
                    if has(n) {
                        bits |= bit.bits();
                    }
                }
                Variant::Faces(Faces::from_bits(bits)?)
            } else {
                let mut bits = 0u8;
                for (n, bit) in [("X", Axes::X), ("Y", Axes::Y), ("Z", Axes::Z)] {
                    if has(n) {
                        bits |= bit.bits();
                    }
                }
                Variant::Axes(Axes::from_bits(bits)?)
            }
        }
        VariantType::NumberSequence => {
            let kps = v.get("keypoints")?.as_array()?;
            Variant::NumberSequence(NumberSequence {
                keypoints: kps
                    .iter()
                    .filter_map(|k| {
                        Some(NumberSequenceKeypoint::new(
                            k.get("time")?.as_f64()? as f32,
                            k.get("value")?.as_f64()? as f32,
                            k.get("envelope").and_then(|e| e.as_f64()).unwrap_or(0.0) as f32,
                        ))
                    })
                    .collect(),
            })
        }
        VariantType::ColorSequence => {
            let kps = v.get("keypoints")?.as_array()?;
            Variant::ColorSequence(ColorSequence {
                keypoints: kps
                    .iter()
                    .filter_map(|k| {
                        let c = nums::<3>(k.get("color")?)?;
                        Some(ColorSequenceKeypoint::new(
                            k.get("time")?.as_f64()? as f32,
                            Color3::new(c[0], c[1], c[2]),
                        ))
                    })
                    .collect(),
            })
        }
        VariantType::PhysicalProperties => Variant::PhysicalProperties(match v {
            J::String(s) if s == "Default" => PhysicalProperties::Default,
            J::Object(o) => {
                let f = |k: &str| o.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
                PhysicalProperties::Custom(CustomPhysicalProperties::new(
                    f("density")?,
                    f("friction")?,
                    f("elasticity")?,
                    f("frictionWeight").unwrap_or(1.0),
                    f("elasticityWeight").unwrap_or(1.0),
                    f("acousticAbsorption").unwrap_or(1.0),
                ))
            }
            _ => return None,
        }),
        _ => return None,
    })
}

fn variant_type(name: &str) -> Option<VariantType> {
    Some(match name {
        "Bool" => VariantType::Bool,
        "String" => VariantType::String,
        "Float32" => VariantType::Float32,
        "Float64" => VariantType::Float64,
        "Int32" => VariantType::Int32,
        "Int64" => VariantType::Int64,
        "Vector3" => VariantType::Vector3,
        "Vector2" => VariantType::Vector2,
        "Vector3int16" => VariantType::Vector3int16,
        "Vector2int16" => VariantType::Vector2int16,
        "CFrame" => VariantType::CFrame,
        "OptionalCFrame" => VariantType::OptionalCFrame,
        "Color3" => VariantType::Color3,
        "Color3uint8" => VariantType::Color3uint8,
        "UDim" => VariantType::UDim,
        "UDim2" => VariantType::UDim2,
        "NumberRange" => VariantType::NumberRange,
        "NumberSequence" => VariantType::NumberSequence,
        "ColorSequence" => VariantType::ColorSequence,
        "Rect" => VariantType::Rect,
        "Content" => VariantType::Content,
        "ContentId" => VariantType::ContentId,
        "BrickColor" => VariantType::BrickColor,
        "Tags" => VariantType::Tags,
        "Attributes" => VariantType::Attributes,
        "Font" => VariantType::Font,
        "Faces" => VariantType::Faces,
        "Axes" => VariantType::Axes,
        "PhysicalProperties" => VariantType::PhysicalProperties,
        _ => return None,
    })
}

fn explicit(ty: &str, v: &J, class: &str, prop: &str) -> Option<Variant> {
    if ty == "Enum" {
        return Some(Variant::Enum(Enum::from_u32(v.as_f64()? as u32)));
    }
    typed(variant_type(ty)?, v, class, prop)
}

/// A property value from project JSON: (canonical name, value), or an
/// error naming what was wrong (Argon reports these as warnings and skips
/// the property).
pub fn resolve(class: &str, prop: &str, v: &J) -> Result<(String, Variant), String> {
    // Explicit: one key naming the type.
    if let Some(o) = v.as_object() {
        if o.len() == 1 {
            let (ty, inner) = o.iter().next().expect("one");
            if ty == "Enum" || variant_type(ty).is_some() {
                let name = descriptor(class, prop).map(|d| d.0).unwrap_or_else(|| prop.to_string());
                return explicit(ty, inner, class, prop)
                    .map(|x| (name, x))
                    .ok_or_else(|| format!("bad {ty} value for {class}.{prop}"));
            }
        }
    }
    let (name, dt) =
        descriptor(class, prop).ok_or_else(|| format!("Unknown property {class}.{prop}"))?;
    let val = match dt {
        DataType::Enum(e) => {
            let en = crate::doc::db()
                .enums
                .get(AsRef::<str>::as_ref(&e))
                .ok_or_else(|| format!("unknown enum {e}"))?;
            let n = match v {
                J::String(s) => *en
                    .items
                    .get(s.as_str())
                    .ok_or_else(|| format!("{s} is not an item of Enum.{e} ({class}.{prop})"))?,
                J::Number(n) => n.as_f64().unwrap_or(0.0) as u32,
                _ => return Err(format!("Enum.{e} item name expected for {class}.{prop}")),
            };
            Variant::Enum(Enum::from_u32(n))
        }
        DataType::Value(ty) => typed(ty, v, class, prop)
            .ok_or_else(|| format!("wrong type of value for {class}.{prop} (expected {ty:?})"))?,
        _ => return Err(format!("unsupported property type for {class}.{prop}")),
    };
    Ok((name, val))
}

fn trim(x: f32) -> J {
    let t = ((x as f64) * 1_000_000.0).trunc() / 1_000_000.0;
    if t.fract() == 0.0 && t.abs() < 1e15 {
        json!(t as i64)
    } else {
        // Shortest decimal for the f32 (avoids 0.30000001192…).
        let s = format!("{x}");
        s.parse::<f64>().map(|f| json!(f)).unwrap_or(json!(t))
    }
}

/// The implicit JSON of a value (what Argon's syncback writes), or None for
/// types project files can't hold implicitly.
pub fn to_json(class: &str, prop: &str, v: &Variant) -> Option<J> {
    Some(match v {
        Variant::Bool(b) => json!(b),
        Variant::String(s) => json!(s),
        Variant::Float32(f) => trim(*f),
        Variant::Float64(f) => trim(*f as f32),
        Variant::Int32(i) => json!(i),
        Variant::Int64(i) => json!(i),
        Variant::Vector3(a) => json!([trim(a.x), trim(a.y), trim(a.z)]),
        Variant::Vector2(a) => json!([trim(a.x), trim(a.y)]),
        Variant::CFrame(c) | Variant::OptionalCFrame(Some(c)) => {
            let (p, o) = (c.position, c.orientation);
            json!([
                trim(p.x), trim(p.y), trim(p.z),
                trim(o.x.x), trim(o.x.y), trim(o.x.z),
                trim(o.y.x), trim(o.y.y), trim(o.y.z),
                trim(o.z.x), trim(o.z.y), trim(o.z.z)
            ])
        }
        Variant::Color3uint8(c) if is_base_part(class) && prop == "Color" => json!([c.r, c.g, c.b]),
        Variant::Color3(c) => json!([trim(c.r), trim(c.g), trim(c.b)]),
        Variant::Color3uint8(c) => json!([c.r, c.g, c.b]),
        Variant::UDim(u) => json!([trim(u.scale), u.offset]),
        Variant::UDim2(u) => json!([[trim(u.x.scale), u.x.offset], [trim(u.y.scale), u.y.offset]]),
        Variant::NumberRange(n) => json!([trim(n.min), trim(n.max)]),
        Variant::Content(c) => json!(c.as_uri().unwrap_or("")),
        Variant::ContentId(c) => json!(c.as_str()),
        Variant::BrickColor(b) => json!(*b as u16),
        Variant::Tags(t) => json!(t.iter().collect::<Vec<_>>()),
        Variant::Vector3int16(a) => json!([a.x, a.y, a.z]),
        Variant::Vector2int16(a) => json!([a.x, a.y]),
        Variant::Rect(r) => json!([trim(r.min.x), trim(r.min.y), trim(r.max.x), trim(r.max.y)]),
        Variant::Font(f) => {
            let mut o = serde_json::Map::new();
            o.insert("family".into(), json!(f.family));
            o.insert("weight".into(), json!(format!("{:?}", f.weight)));
            o.insert("style".into(), json!(format!("{:?}", f.style)));
            J::Object(o)
        }
        Variant::Faces(f) => {
            let names: Vec<&str> = [
                ("Right", Faces::RIGHT), ("Top", Faces::TOP), ("Back", Faces::BACK),
                ("Left", Faces::LEFT), ("Bottom", Faces::BOTTOM), ("Front", Faces::FRONT),
            ]
            .into_iter()
            .filter(|(_, b)| f.contains(*b))
            .map(|(n, _)| n)
            .collect();
            json!(names)
        }
        Variant::Axes(a) => {
            let names: Vec<&str> = [("X", Axes::X), ("Y", Axes::Y), ("Z", Axes::Z)]
                .into_iter()
                .filter(|(_, b)| a.contains(*b))
                .map(|(n, _)| n)
                .collect();
            json!(names)
        }
        Variant::NumberSequence(n) => json!({ "keypoints": n.keypoints.iter().map(|k| json!({
            "time": trim(k.time), "value": trim(k.value), "envelope": trim(k.envelope)
        })).collect::<Vec<_>>() }),
        Variant::ColorSequence(c) => json!({ "keypoints": c.keypoints.iter().map(|k| json!({
            "time": trim(k.time), "color": [trim(k.color.r), trim(k.color.g), trim(k.color.b)]
        })).collect::<Vec<_>>() }),
        Variant::PhysicalProperties(PhysicalProperties::Default) => json!("Default"),
        Variant::PhysicalProperties(PhysicalProperties::Custom(c)) => json!({
            "density": c.density(), "friction": c.friction(), "elasticity": c.elasticity(),
            "frictionWeight": c.friction_weight(), "elasticityWeight": c.elasticity_weight(),
            "acousticAbsorption": c.acoustic_absorption()
        }),
        Variant::Enum(e) => {
            // The item's name when the database knows the enum.
            match descriptor(class, prop).map(|d| d.1) {
                Some(DataType::Enum(name)) => crate::doc::db()
                    .enums
                    .get(AsRef::<str>::as_ref(&name))
                    .and_then(|en| en.items.iter().find(|(_, v)| **v == e.to_u32()))
                    .map(|(n, _)| json!(n))
                    .unwrap_or(json!(e.to_u32())),
                _ => json!({ "Enum": e.to_u32() }),
            }
        }
        _ => return None,
    })
}
