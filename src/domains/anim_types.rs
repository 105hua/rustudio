//! Animation curve key datatypes Studio has and rustudio's ltypes.rs
//! doesn't: RotationCurveKey and ValueCurveKey (FloatCurveKey is in
//! ltypes.rs). Measured in Studio (offline Studio, 2026-11-07):
//! - `RotationCurveKey.new(time, cframe, interp = Cubic)` keeps only the
//!   rotation of the CFrame; LeftTangent / RightTangent start at 0 (not nil,
//!   unlike FloatCurveKey) and tostring gives
//!   `{Linear, 1, x, y, z, R00 … R22, left, right}` with %.9g numbers.
//! - `ValueCurveKey.new(time, value, interp = Cubic)`: Time, Value,
//!   Interpolation; tostring gives "" (Studio prints nothing for it).

use glam::{Mat3, Vec3};
use mlua::{Lua, MetaMethod, UserData, UserDataFields, UserDataMethods, Value};

use crate::ltypes::{enum_arg, enum_value, rt_err, LCFrame, LEnumItem};
use crate::math::Xform;

/// Studio's `%.9g` number text ("0.540302336", "5.96046448e-08").
pub fn g9(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    if !x.is_finite() {
        return if x.is_nan() { "nan".into() } else if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let exp = x.abs().log10().floor() as i32;
    let trim = |s: String| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    if !(-5..9).contains(&exp) {
        let s = format!("{:.8e}", x);
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let e: i32 = e.parse().unwrap_or(0);
        format!("{}e{}{:02}", trim(m.to_string()), if e < 0 { '-' } else { '+' }, e.abs())
    } else {
        let decimals = (8 - exp).max(0) as usize;
        trim(format!("{:.*}", decimals, x))
    }
}

#[derive(Debug, Clone)]
pub struct LRotationCurveKey {
    pub time: f32,
    pub rot: Mat3,
    pub interp: LEnumItem,
    pub left: f32,
    pub right: f32,
}

impl LRotationCurveKey {
    pub fn cframe(&self) -> LCFrame {
        LCFrame(Xform { pos: Vec3::ZERO, rot: self.rot })
    }
}

impl UserData for LRotationCurveKey {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "RotationCurveKey");
        f.add_field_method_get("Time", |_, this| Ok(this.time as f64));
        f.add_field_method_get("Value", |_, this| Ok(this.cframe()));
        f.add_field_method_get("Interpolation", |lua, this| enum_value(lua, this.interp.clone()));
        f.add_field_method_get("LeftTangent", |_, this| Ok(this.left as f64));
        f.add_field_method_get("RightTangent", |_, this| Ok(this.right as f64));
        f.add_field_method_set("LeftTangent", |_, this, v: f32| {
            this.left = v;
            Ok(())
        });
        f.add_field_method_set("RightTangent", |_, this, v: f32| {
            this.right = v;
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let r = this.rot;
            let mut parts = vec![this.interp.name.clone(), g9(this.time as f64)];
            parts.extend(["0", "0", "0"].map(String::from));
            for row in 0..3 {
                for col in 0..3 {
                    parts.push(g9(r.col(col)[row] as f64));
                }
            }
            parts.push(g9(this.left as f64));
            parts.push(g9(this.right as f64));
            Ok(format!("{{{}}}", parts.join(", ")))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: mlua::AnyUserData| {
            let Ok(o) = o.borrow::<LRotationCurveKey>() else { return Ok(false) };
            Ok(this.time == o.time
                && this.rot == o.rot
                && this.interp == o.interp
                && this.left == o.left
                && this.right == o.right)
        });
    }
}

#[derive(Clone)]
pub struct LValueCurveKey {
    pub time: f32,
    pub value: Value,
    pub interp: LEnumItem,
}

impl UserData for LValueCurveKey {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "ValueCurveKey");
        f.add_field_method_get("Time", |_, this| Ok(this.time as f64));
        f.add_field_method_get("Value", |_, this| Ok(this.value.clone()));
        f.add_field_method_get("Interpolation", |lua, this| enum_value(lua, this.interp.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok(""));
    }
}

/// Studio's cast-failure name for a Lua value ("double", "string", …).
pub fn cast_name(v: &Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Boolean(_) => "bool".into(),
        Value::Integer(_) | Value::Number(_) => "double".into(),
        Value::String(_) => "string".into(),
        Value::Table(_) => "table".into(),
        Value::Function(_) => "function".into(),
        Value::UserData(u) => u
            .metatable()
            .ok()
            .and_then(|m| m.get::<String>("__type").ok())
            .unwrap_or_else(|| "userdata".into()),
        other => other.type_name().into(),
    }
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    let rck = lua.create_table()?;
    rck.set(
        "new",
        lua.create_function(|_, (t, cf, mode): (Value, Value, Option<Value>)| {
            let time = match t {
                Value::Number(n) => n as f32,
                Value::Integer(n) => n as f32,
                ref v => return Err(rt_err(format!("Unable to cast {} to float", cast_name(v)))),
            };
            let rot = match &cf {
                Value::UserData(u) => u.borrow::<LCFrame>().map(|c| c.0.rot).ok(),
                _ => None,
            }
            .ok_or_else(|| rt_err(format!("Unable to cast {} to CoordinateFrame", cast_name(&cf))))?;
            Ok(LRotationCurveKey {
                time,
                rot,
                interp: enum_arg(mode.as_ref(), "KeyInterpolationMode", "Cubic")?,
                left: 0.0,
                right: 0.0,
            })
        })?,
    )?;
    g.set("RotationCurveKey", rck)?;
    let vck = lua.create_table()?;
    vck.set(
        "new",
        lua.create_function(|_, (t, value, mode): (f32, Value, Option<Value>)| {
            Ok(LValueCurveKey {
                time: t,
                value,
                interp: enum_arg(mode.as_ref(), "KeyInterpolationMode", "Cubic")?,
            })
        })?,
    )?;
    g.set("ValueCurveKey", vck)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::g9;

    #[test]
    fn studio_number_text() {
        assert_eq!(g9(0.540302336), "0.540302336");
        assert_eq!(g9(5.96046448e-08), "5.96046448e-08");
        assert_eq!(g9(1.0), "1");
        assert_eq!(g9(-0.841470957), "-0.841470957");
        assert_eq!(g9(0.99999994), "0.99999994");
    }
}
