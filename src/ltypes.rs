//! Luau datatypes that mirror Roblox's public API reference
//! (create.roblox.com/docs/reference/engine/datatypes): Vector3, Vector2,
//! CFrame, Color3, BrickColor, UDim, UDim2 and Enum / EnumItem, plus Ray,
//! Region3(int16), Vector3int16/Vector2int16, NumberRange, NumberSequence,
//! ColorSequence, Rect, PhysicalProperties, TweenInfo, Random, DateTime,
//! Faces, Axes and Raycast/Overlap/CatalogSearch params. Each userdata sets
//! `__type` so Luau's `typeof()` returns the Roblox type name.
//! [`datatype_to_variant`] / [`variant_to_datatype`] convert the property
//! datatypes to and from rbx_types `Variant`s.

use glam::{EulerRot, Mat3, Quat, Vec2, Vec3};
use mlua::{
    FromLua, IntoLua, Lua, MetaMethod, Result as LResult, Table, UserData, UserDataFields, UserDataMethods,
    Value,
};
use rbx_dom_weak::types::BrickColor;

use crate::doc;
use crate::math::Xform;

pub fn rt_err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(msg.into())
}

pub fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

/// C's `%.{prec}g` (Studio prints datatypes with `%g` / `%.9g` of the
/// float32 widened to double: Vector2 `0.333333343`, Color3 `0.333333`;
/// `-0` stays `-0`).
pub fn fmt_g(x: f64, prec: usize) -> String {
    if x.is_nan() {
        return if x.is_sign_negative() { "-nan" } else { "nan" }.into();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf" } else { "inf" }.into();
    }
    let prec = prec.max(1);
    if x == 0.0 {
        return if x.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let e = format!("{:.*e}", prec - 1, x);
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let strip = |t: &str| -> String {
        if t.contains('.') {
            t.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            t.to_string()
        }
    };
    if exp < -4 || exp >= prec as i32 {
        format!("{}e{}{:02}", strip(mant), if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        strip(&format!("{:.*}", (prec as i32 - 1 - exp).max(0) as usize, x))
    }
}

/// `%g` and `%.9g` of a float32.
pub fn g6(x: f32) -> String {
    fmt_g(x as f64, 6)
}
pub fn g9(x: f32) -> String {
    fmt_g(x as f64, 9)
}

fn fmt_num(f: f32) -> String {
    let s = format!("{f}");
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

fn ud<T: 'static + Clone>(v: &Value) -> Option<T> {
    if let Some(x) = vec_any::<T>(v) {
        return Some(x);
    }
    match v {
        Value::UserData(u) => u.borrow::<T>().ok().map(|r| (*r).clone()),
        _ => None,
    }
}

// ----------------------------------------------------------------- Vector3

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LVector3(pub Vec3);

impl FromLua for LVector3 {
    fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
        v3_opt(&v).ok_or_else(|| rt_err(format!("Vector3 expected, got {}", crate::cmeta::type_name(&v))))
    }
}

/// Vector3 is Luau's native vector (Studio probes item 1: type "vector",
/// typeof "Vector3", equal values rawequal and the same table key,
/// float32 components, -0 kept).
impl IntoLua for LVector3 {
    fn into_lua(self, _: &Lua) -> LResult<Value> {
        Ok(self.into_value())
    }
}

impl LVector3 {
    pub fn into_value(self) -> Value {
        Value::Vector(mlua::Vector::new(self.0.x, self.0.y, self.0.z))
    }
}

/// The Vector3 in `v`, if it is one.
pub fn v3_opt(v: &Value) -> Option<LVector3> {
    match v {
        Value::Vector(v) => Some(LVector3(Vec3::new(v.x(), v.y(), v.z()))),
        _ => None,
    }
}

/// `v3_opt` for the generic `ud::<T>`-style helpers (T = LVector3), which
/// otherwise only see userdata.
pub fn vec_any<T: 'static>(v: &Value) -> Option<T> {
    if std::any::TypeId::of::<T>() != std::any::TypeId::of::<LVector3>() {
        return None;
    }
    // (T is LVector3: no boxing per call.)
    let x = v3_opt(v)?;
    (&mut Some(x) as &mut dyn std::any::Any).downcast_mut::<Option<T>>()?.take()
}

/// The vector type's metatable: Vector3's members (dtmeta builds the C
/// index / newindex / namecall over these; arithmetic is the VM's).
pub fn install_vector3(lua: &Lua) -> LResult<()> {
    let getters = lua.create_table()?;
    let get = |f: fn(Vec3) -> Value| lua.create_function(move |_, v: LVector3| Ok(f(v.0)));
    for (name, i) in [("X", 0usize), ("Y", 1), ("Z", 2), ("x", 0), ("y", 1), ("z", 2)] {
        getters.raw_set(name, lua.create_function(move |_, v: LVector3| Ok(v.0[i] as f64))?)?;
    }
    for n in ["Magnitude", "magnitude"] {
        getters.raw_set(n, get(|v| Value::Number(v.length() as f64))?)?;
    }
    for n in ["Unit", "unit"] {
        getters.raw_set(n, get(|v| LVector3(v.normalize_or_zero()).into_value())?)?;
    }
    let methods = lua.create_table()?;
    macro_rules! method {
        ($name:literal, $args:ty, $f:expr) => {
            methods.raw_set($name, lua.create_function(move |_, (this, a): (LVector3, $args)| {
                let f: fn(Vec3, $args) -> LResult<Value> = $f;
                f(this.0, a)
            })?)?;
        };
    }
    method!("Dot", LVector3, |a, o| Ok(Value::Number(a.dot(o.0) as f64)));
    method!("Cross", LVector3, |a, o| Ok(LVector3(a.cross(o.0)).into_value()));
    method!("Lerp", (LVector3, f32), |a, (o, t)| Ok(LVector3(a.lerp(o.0, t)).into_value()));
    method!("Abs", (), |a, ()| Ok(LVector3(a.abs()).into_value()));
    method!("Floor", (), |a, ()| Ok(LVector3(a.floor()).into_value()));
    method!("Ceil", (), |a, ()| Ok(LVector3(a.ceil()).into_value()));
    method!("Sign", (), |a, ()| {
        Ok(LVector3(a.map(|c| if c == 0.0 { 0.0 } else { c.signum() })).into_value())
    });
    method!("Min", LVector3, |a, o| Ok(LVector3(a.min(o.0)).into_value()));
    method!("Max", LVector3, |a, o| Ok(LVector3(a.max(o.0)).into_value()));
    method!("Angle", LVector3, |a, o| Ok(Value::Number(a.angle_between(o.0) as f64)));
    method!("FuzzyEq", (LVector3, Option<f32>), |a, (o, eps)| {
        Ok(Value::Boolean((a - o.0).abs().max_element() <= eps.unwrap_or(1e-5)))
    });
    crate::dtmeta::install_vector(lua, getters, methods)
}

// ----------------------------------------------------------------- Vector2

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LVector2(pub Vec2);

impl FromLua for LVector2 {
    fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
        ud::<LVector2>(&v).ok_or_else(|| rt_err(format!("Vector2 expected, got {}", v.type_name())))
    }
}

impl UserData for LVector2 {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Vector2");
        for (name, i) in [("X", 0), ("Y", 1), ("x", 0), ("y", 1)] {
            f.add_field_method_get(name, move |_, this| Ok(this.0[i] as f64));
        }
        f.add_field_method_get("Magnitude", |_, this| Ok(this.0.length() as f64));
        f.add_field_method_get("Unit", |_, this| Ok(LVector2(this.0.normalize_or_zero())));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("Dot", |_, this, o: LVector2| Ok(this.0.dot(o.0) as f64));
        m.add_method("Lerp", |_, this, (o, a): (LVector2, f32)| {
            Ok(LVector2(this.0.lerp(o.0, a)))
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("{}, {}", g9(this.0.x), g9(this.0.y)))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LVector2| Ok(this.0 == o.0));
        m.add_meta_method(MetaMethod::Unm, |_, this, ()| Ok(LVector2(-this.0)));
        m.add_meta_function(MetaMethod::Add, |_, (a, b): (LVector2, LVector2)| {
            Ok(LVector2(a.0 + b.0))
        });
        m.add_meta_function(MetaMethod::Sub, |_, (a, b): (LVector2, LVector2)| {
            Ok(LVector2(a.0 - b.0))
        });
        m.add_meta_function(MetaMethod::Mul, |_, (a, b): (Value, Value)| {
            match (ud::<LVector2>(&a), ud::<LVector2>(&b), num(&a), num(&b)) {
                (Some(x), Some(y), _, _) => Ok(LVector2(x.0 * y.0)),
                (Some(x), None, _, Some(k)) => Ok(LVector2(x.0 * k as f32)),
                (None, Some(y), Some(k), _) => Ok(LVector2(y.0 * k as f32)),
                // Studio's own text, location-free.
                _ => Err(rt_err("attempt to multiply a Vector2 with an incompatible value type or nil")),
            }
        });
        m.add_meta_function(MetaMethod::Div, |_, (a, b): (LVector2, Value)| {
            match (ud::<LVector2>(&b), num(&b)) {
                (Some(y), _) => Ok(LVector2(a.0 / y.0)),
                (None, Some(k)) => Ok(LVector2(a.0 / k as f32)),
                _ => Err(rt_err("attempt to divide a Vector2 with an incompatible value type or nil")),
            }
        });
        m.add_meta_function(MetaMethod::IDiv, |_, (a, b): (LVector2, Value)| {
            match (ud::<LVector2>(&b), num(&b)) {
                (Some(y), _) => Ok(LVector2((a.0 / y.0).floor())),
                (None, Some(k)) => Ok(LVector2((a.0 / k as f32).floor())),
                _ => Err(rt_err("attempt to divide a Vector2 with an incompatible value type or nil")),
            }
        });
    }
}

// ------------------------------------------------------------------ CFrame

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LCFrame(pub Xform);

impl FromLua for LCFrame {
    fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
        ud::<LCFrame>(&v).ok_or_else(|| rt_err(format!("CFrame expected, got {}", v.type_name())))
    }
}

pub fn cf_mul(a: &Xform, b: &Xform) -> Xform {
    Xform {
        pos: a.pos + a.rot * b.pos,
        rot: a.rot * b.rot,
    }
}

pub fn cf_inv(a: &Xform) -> Xform {
    let r = a.rot.transpose();
    Xform {
        pos: -(r * a.pos),
        rot: r,
    }
}

/// CFrame.lookAt / lookAlong: right = look × up, however short (Studio,
/// round 19: a view 1e-5 off straight down keeps right = +X); only an
/// exactly parallel up falls back to look × X (straight down: right +Z,
/// up +X; straight up: right −Z, up +X; the same with up = X given).
pub fn look_at(pos: Vec3, target: Vec3, up: Vec3) -> Xform {
    look_at_eps(pos, target, up, 0.0)
}

/// The two-Vector3 CFrame.new(pos, lookAt): the older constructor already
/// takes the fallback 1e-5 off vertical (Studio: right (0, −1e-5, 1) there).
pub fn look_at_legacy(pos: Vec3, target: Vec3) -> Xform {
    look_at_eps(pos, target, Vec3::Y, 1e-8)
}

fn look_at_eps(pos: Vec3, target: Vec3, up: Vec3, eps: f32) -> Xform {
    let look = (target - pos).normalize_or(Vec3::NEG_Z);
    let mut right = look.cross(up);
    if right.length_squared() <= eps {
        right = look.cross(Vec3::X);
    }
    let right = right.normalize_or(Vec3::X);
    let up2 = right.cross(look);
    Xform {
        pos,
        rot: Mat3::from_cols(right, up2, -look),
    }
}

impl UserData for LCFrame {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "CFrame");
        f.add_field_method_get("Position", |_, this| Ok(LVector3(this.0.pos)));
        f.add_field_method_get("p", |_, this| Ok(LVector3(this.0.pos)));
        for (name, i) in [("X", 0), ("Y", 1), ("Z", 2), ("x", 0), ("y", 1), ("z", 2)] {
            f.add_field_method_get(name, move |_, this| Ok(this.0.pos[i] as f64));
        }
        f.add_field_method_get("LookVector", |_, this| Ok(LVector3(-this.0.rot.z_axis)));
        f.add_field_method_get("lookVector", |_, this| Ok(LVector3(-this.0.rot.z_axis)));
        f.add_field_method_get("RightVector", |_, this| Ok(LVector3(this.0.rot.x_axis)));
        f.add_field_method_get("UpVector", |_, this| Ok(LVector3(this.0.rot.y_axis)));
        f.add_field_method_get("rightVector", |_, this| Ok(LVector3(this.0.rot.x_axis)));
        f.add_field_method_get("upVector", |_, this| Ok(LVector3(this.0.rot.y_axis)));
        f.add_field_method_get("XVector", |_, this| Ok(LVector3(this.0.rot.x_axis)));
        f.add_field_method_get("YVector", |_, this| Ok(LVector3(this.0.rot.y_axis)));
        f.add_field_method_get("ZVector", |_, this| Ok(LVector3(this.0.rot.z_axis)));
        f.add_field_method_get("Rotation", |_, this| {
            Ok(LCFrame(Xform {
                pos: Vec3::ZERO,
                rot: this.0.rot,
            }))
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("Inverse", |_, this, ()| Ok(LCFrame(cf_inv(&this.0))));
        m.add_method("inverse", |_, this, ()| Ok(LCFrame(cf_inv(&this.0))));
        m.add_method("Lerp", |_, this, (o, a): (LCFrame, f32)| {
            let q = Quat::from_mat3(&this.0.rot).slerp(Quat::from_mat3(&o.0.rot), a);
            Ok(LCFrame(Xform {
                pos: this.0.pos.lerp(o.0.pos, a),
                rot: Mat3::from_quat(q),
            }))
        });
        m.add_method("lerp", |_, this, (o, a): (LCFrame, f32)| {
            let q = Quat::from_mat3(&this.0.rot).slerp(Quat::from_mat3(&o.0.rot), a);
            Ok(LCFrame(Xform {
                pos: this.0.pos.lerp(o.0.pos, a),
                rot: Mat3::from_quat(q),
            }))
        });
        m.add_method("ToWorldSpace", |_, this, o: LCFrame| {
            Ok(LCFrame(cf_mul(&this.0, &o.0)))
        });
        m.add_method("ToObjectSpace", |_, this, o: LCFrame| {
            Ok(LCFrame(cf_mul(&cf_inv(&this.0), &o.0)))
        });
        m.add_method("PointToWorldSpace", |_, this, v: LVector3| {
            Ok(LVector3(this.0.pos + this.0.rot * v.0))
        });
        m.add_method("PointToObjectSpace", |_, this, v: LVector3| {
            Ok(LVector3(this.0.rot.transpose() * (v.0 - this.0.pos)))
        });
        m.add_method("VectorToWorldSpace", |_, this, v: LVector3| {
            Ok(LVector3(this.0.rot * v.0))
        });
        m.add_method("VectorToObjectSpace", |_, this, v: LVector3| {
            Ok(LVector3(this.0.rot.transpose() * v.0))
        });
        // Deprecated spellings still used by Roblox's own scripts (round 19:
        // Studio also answers lerp, toAxisAngle, toEulerAngles(XYZ / YXZ)
        // and toOrientation; Place B's Poppercam calls toAxisAngle).
        m.add_method("toWorldSpace", |_, this, o: LCFrame| {
            Ok(LCFrame(cf_mul(&this.0, &o.0)))
        });
        m.add_method("toObjectSpace", |_, this, o: LCFrame| {
            Ok(LCFrame(cf_mul(&cf_inv(&this.0), &o.0)))
        });
        m.add_method("pointToWorldSpace", |_, this, v: LVector3| {
            Ok(LVector3(this.0.pos + this.0.rot * v.0))
        });
        m.add_method("pointToObjectSpace", |_, this, v: LVector3| {
            Ok(LVector3(this.0.rot.transpose() * (v.0 - this.0.pos)))
        });
        m.add_method("vectorToWorldSpace", |_, this, v: LVector3| {
            Ok(LVector3(this.0.rot * v.0))
        });
        m.add_method("vectorToObjectSpace", |_, this, v: LVector3| {
            Ok(LVector3(this.0.rot.transpose() * v.0))
        });
        m.add_method("GetComponents", |_, this, ()| {
            Ok(mlua::Variadic::from_iter(components(&this.0)))
        });
        m.add_method("components", |_, this, ()| {
            Ok(mlua::Variadic::from_iter(components(&this.0)))
        });
        m.add_method("ToEulerAnglesXYZ", |_, this, ()| {
            let (x, y, z) = this.0.rot.to_euler(EulerRot::XYZ);
            Ok((x as f64, y as f64, z as f64))
        });
        m.add_method("toEulerAnglesXYZ", |_, this, ()| {
            let (x, y, z) = this.0.rot.to_euler(EulerRot::XYZ);
            Ok((x as f64, y as f64, z as f64))
        });
        m.add_method("ToOrientation", |_, this, ()| {
            let (y, x, z) = this.0.rot.to_euler(EulerRot::YXZ);
            Ok((x as f64, y as f64, z as f64))
        });
        m.add_method("toOrientation", |_, this, ()| {
            let (y, x, z) = this.0.rot.to_euler(EulerRot::YXZ);
            Ok((x as f64, y as f64, z as f64))
        });
        m.add_method("ToEulerAnglesYXZ", |_, this, ()| {
            let (y, x, z) = this.0.rot.to_euler(EulerRot::YXZ);
            Ok((x as f64, y as f64, z as f64))
        });
        m.add_method("toEulerAnglesYXZ", |_, this, ()| {
            let (y, x, z) = this.0.rot.to_euler(EulerRot::YXZ);
            Ok((x as f64, y as f64, z as f64))
        });
        m.add_method("ToEulerAngles", |_, this, order: Option<Value>| {
            let order = rotation_order(order.as_ref())?;
            let a = this.0.rot.to_euler(euler_rot(&order));
            let mut out = [0.0f64; 3];
            for (i, c) in order.chars().enumerate() {
                out[(c as u8 - b'X') as usize] = [a.0, a.1, a.2][i] as f64;
            }
            Ok((out[0], out[1], out[2]))
        });
        m.add_method("toEulerAngles", |_, this, order: Option<Value>| {
            let order = rotation_order(order.as_ref())?;
            let a = this.0.rot.to_euler(euler_rot(&order));
            let mut out = [0.0f64; 3];
            for (i, c) in order.chars().enumerate() {
                out[(c as u8 - b'X') as usize] = [a.0, a.1, a.2][i] as f64;
            }
            Ok((out[0], out[1], out[2]))
        });
        m.add_method("ToAxisAngle", |_, this, ()| {
            let (axis, angle) = Quat::from_mat3(&this.0.rot).to_axis_angle();
            Ok((LVector3(axis), angle as f64))
        });
        m.add_method("toAxisAngle", |_, this, ()| {
            let (axis, angle) = Quat::from_mat3(&this.0.rot).to_axis_angle();
            Ok((LVector3(axis), angle as f64))
        });
        m.add_method("Orthonormalize", |_, this, ()| {
            let x = this.0.rot.x_axis.normalize_or(Vec3::X);
            let y = (this.0.rot.y_axis - x * x.dot(this.0.rot.y_axis))
                .normalize_or(x.any_orthonormal_vector());
            Ok(LCFrame(Xform {
                pos: this.0.pos,
                rot: Mat3::from_cols(x, y, x.cross(y)),
            }))
        });
        m.add_method("FuzzyEq", |_, this, (o, eps): (LCFrame, Option<f32>)| {
            let eps = eps.unwrap_or(1e-5);
            let close = |a: Vec3, b: Vec3| (a - b).abs().max_element() <= eps;
            Ok(close(this.0.pos, o.0.pos)
                && close(this.0.rot.x_axis, o.0.rot.x_axis)
                && close(this.0.rot.y_axis, o.0.rot.y_axis)
                && close(this.0.rot.z_axis, o.0.rot.z_axis))
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let c = components(&this.0);
            Ok(c.iter()
                .map(|v| g9(*v as f32))
                .collect::<Vec<_>>()
                .join(", "))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LCFrame| Ok(this.0 == o.0));
        m.add_meta_function(MetaMethod::Mul, |lua, args: mlua::MultiValue| {
            let a = match args.front() {
                Some(v) => ud::<LCFrame>(v).ok_or_else(|| {
                    crate::cmeta::located(format!(
                        "invalid argument #1 (CFrame expected, got {})",
                        crate::cmeta::type_name(v)
                    ))
                })?,
                None => return Err(crate::cmeta::located("missing argument #1 (CFrame expected)")),
            };
            let Some(b) = args.get(1) else {
                return Err(crate::cmeta::located("missing argument #2 (Vector3 expected)"));
            };
            if let Some(o) = ud::<LCFrame>(b) {
                return LCFrame(cf_mul(&a.0, &o.0)).into_lua_value(lua);
            }
            if let Some(v) = ud::<LVector3>(b) {
                return LVector3(a.0.pos + a.0.rot * v.0).into_lua_value(lua);
            }
            Err(crate::cmeta::located(format!(
                "invalid argument #2 (Vector3 expected, got {})",
                crate::cmeta::type_name(b)
            )))
        });
        m.add_meta_function(MetaMethod::Add, |_, (a, v): (LCFrame, LVector3)| {
            Ok(LCFrame(Xform {
                pos: a.0.pos + v.0,
                rot: a.0.rot,
            }))
        });
        m.add_meta_function(MetaMethod::Sub, |_, (a, v): (LCFrame, LVector3)| {
            Ok(LCFrame(Xform {
                pos: a.0.pos - v.0,
                rot: a.0.rot,
            }))
        });
    }
}

trait IntoLuaValue {
    fn into_lua_value(self, lua: &Lua) -> LResult<Value>;
}

impl<T: UserData + 'static> IntoLuaValue for T {
    fn into_lua_value(self, lua: &Lua) -> LResult<Value> {
        Ok(Value::UserData(lua.create_userdata(self)?))
    }
}

impl IntoLuaValue for LVector3 {
    fn into_lua_value(self, _: &Lua) -> LResult<Value> {
        Ok(self.into_value())
    }
}

fn components(x: &Xform) -> Vec<f64> {
    let r = &x.rot;
    vec![
        x.pos.x, x.pos.y, x.pos.z, r.x_axis.x, r.y_axis.x, r.z_axis.x, r.x_axis.y, r.y_axis.y,
        r.z_axis.y, r.x_axis.z, r.y_axis.z, r.z_axis.z,
    ]
    .into_iter()
    .map(|f| f as f64)
    .collect()
}

// ------------------------------------------------------------------ Color3

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LColor3(pub [f32; 3]);

impl FromLua for LColor3 {
    fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
        ud::<LColor3>(&v).ok_or_else(|| rt_err(format!("Color3 expected, got {}", v.type_name())))
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = (h.rem_euclid(1.0)) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

fn rgb_to_hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    } / 6.0;
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

impl UserData for LColor3 {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Color3");
        for (name, i) in [("R", 0), ("G", 1), ("B", 2), ("r", 0), ("g", 1), ("b", 2)] {
            f.add_field_method_get(name, move |_, this| Ok(this.0[i] as f64));
        }
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("Lerp", |_, this, (o, a): (LColor3, f32)| {
            Ok(LColor3(
                [0, 1, 2].map(|i| this.0[i] + (o.0[i] - this.0[i]) * a),
            ))
        });
        m.add_method("lerp", |_, this, (o, a): (LColor3, f32)| {
            Ok(LColor3(
                [0, 1, 2].map(|i| this.0[i] + (o.0[i] - this.0[i]) * a),
            ))
        });
        m.add_method("ToHSV", |_, this, ()| {
            let (h, s, v) = rgb_to_hsv(this.0);
            Ok((h as f64, s as f64, v as f64))
        });
        m.add_method("ToHex", |_, this, ()| {
            let c = this.0.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
            Ok(format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "{}, {}, {}",
                g6(this.0[0]),
                g6(this.0[1]),
                g6(this.0[2])
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LColor3| Ok(this.0 == o.0));
    }
}

// -------------------------------------------------------------- BrickColor

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LBrickColor(pub BrickColor);

impl FromLua for LBrickColor {
    fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
        ud::<LBrickColor>(&v)
            .ok_or_else(|| rt_err(format!("BrickColor expected, got {}", v.type_name())))
    }
}

/// The BrickColor closest to an RGB colour (how Roblox derives BrickColor from Color).
pub fn nearest_brick(rgb: [u8; 3]) -> BrickColor {
    let mut best = (
        u32::MAX,
        BrickColor::from_number(194).expect("medium stone grey"),
    );
    for n in 1..=1032u16 {
        if let Some(b) = BrickColor::from_number(n) {
            let c = b.to_color3uint8();
            let d = [
                c.r as i32 - rgb[0] as i32,
                c.g as i32 - rgb[1] as i32,
                c.b as i32 - rgb[2] as i32,
            ];
            let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as u32;
            if dist < best.0 {
                best = (dist, b);
            }
        }
    }
    best.1
}

impl UserData for LBrickColor {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "BrickColor");
        f.add_field_method_get("Name", |_, this| Ok(this.0.to_string()));
        f.add_field_method_get("Number", |_, this| Ok(this.0 as u16 as f64));
        f.add_field_method_get("Color", |_, this| {
            let c = this.0.to_color3uint8();
            Ok(LColor3([
                c.r as f32 / 255.0,
                c.g as f32 / 255.0,
                c.b as f32 / 255.0,
            ]))
        });
        for (name, i) in [("r", 0), ("g", 1), ("b", 2)] {
            f.add_field_method_get(name, move |_, this| {
                let c = this.0.to_color3uint8();
                Ok([c.r, c.g, c.b][i] as f64 / 255.0)
            });
        }
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.0.to_string()));
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LBrickColor| Ok(this.0 == o.0));
    }
}

// ------------------------------------------------------------- UDim, UDim2

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LUDim {
    pub scale: f32,
    pub offset: i32,
}

impl FromLua for LUDim {
    fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
        ud::<LUDim>(&v).ok_or_else(|| rt_err(format!("UDim expected, got {}", v.type_name())))
    }
}

impl UserData for LUDim {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "UDim");
        f.add_field_method_get("Scale", |_, this| Ok(this.scale as f64));
        f.add_field_method_get("Offset", |_, this| Ok(this.offset as f64));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("{}, {}", g9(this.scale), this.offset))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LUDim| Ok(*this == o));
        m.add_meta_function(MetaMethod::Add, |_, (a, b): (LUDim, LUDim)| {
            Ok(LUDim {
                scale: a.scale + b.scale,
                offset: a.offset + b.offset,
            })
        });
        m.add_meta_function(MetaMethod::Sub, |_, (a, b): (LUDim, LUDim)| {
            Ok(LUDim {
                scale: a.scale - b.scale,
                offset: a.offset - b.offset,
            })
        });
        m.add_meta_method(MetaMethod::Unm, |_, this, ()| {
            Ok(LUDim { scale: -this.scale, offset: this.offset.wrapping_neg() })
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LUDim2 {
    pub x: LUDim,
    pub y: LUDim,
}

impl FromLua for LUDim2 {
    fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
        ud::<LUDim2>(&v).ok_or_else(|| rt_err(format!("UDim2 expected, got {}", v.type_name())))
    }
}

impl UserData for LUDim2 {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "UDim2");
        f.add_field_method_get("X", |_, this| Ok(this.x));
        f.add_field_method_get("Y", |_, this| Ok(this.y));
        f.add_field_method_get("Width", |_, this| Ok(this.x));
        f.add_field_method_get("Height", |_, this| Ok(this.y));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("Lerp", |_, this, (o, a): (LUDim2, f32)| {
            let l = |p: LUDim, q: LUDim| LUDim {
                scale: p.scale + (q.scale - p.scale) * a,
                offset: (p.offset as f32 + (q.offset - p.offset) as f32 * a).round() as i32,
            };
            Ok(LUDim2 {
                x: l(this.x, o.x),
                y: l(this.y, o.y),
            })
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "{{{}, {}}}, {{{}, {}}}",
                g9(this.x.scale),
                this.x.offset,
                g9(this.y.scale),
                this.y.offset
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LUDim2| Ok(*this == o));
        m.add_meta_function(MetaMethod::Add, |_, (a, b): (LUDim2, LUDim2)| {
            Ok(LUDim2 {
                x: LUDim {
                    scale: a.x.scale + b.x.scale,
                    offset: a.x.offset + b.x.offset,
                },
                y: LUDim {
                    scale: a.y.scale + b.y.scale,
                    offset: a.y.offset + b.y.offset,
                },
            })
        });
        m.add_meta_function(MetaMethod::Sub, |_, (a, b): (LUDim2, LUDim2)| {
            Ok(LUDim2 {
                x: LUDim {
                    scale: a.x.scale - b.x.scale,
                    offset: a.x.offset - b.x.offset,
                },
                y: LUDim {
                    scale: a.y.scale - b.y.scale,
                    offset: a.y.offset - b.y.offset,
                },
            })
        });
        m.add_meta_method(MetaMethod::Unm, |_, this, ()| {
            let neg = |u: LUDim| LUDim { scale: -u.scale, offset: u.offset.wrapping_neg() };
            Ok(LUDim2 { x: neg(this.x), y: neg(this.y) })
        });
    }
}

// -------------------------------------------------------------------- Enum

#[derive(Debug, Clone, PartialEq)]
pub struct LEnumItem {
    pub enum_type: String,
    pub name: String,
    pub value: u32,
}

impl FromLua for LEnumItem {
    fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
        ud::<LEnumItem>(&v)
            .ok_or_else(|| rt_err(format!("EnumItem expected, got {}", v.type_name())))
    }
}

impl LEnumItem {
    /// An enum only api-dump.json knows (missing from the reflection
    /// database): its items.
    fn dump_items(enum_type: &str) -> Option<&'static Vec<(String, u32)>> {
        crate::api::get()?.enums.get(enum_type)
    }

    /// Whether `Enum.<name>` exists (reflection database or API dump).
    pub fn enum_exists(name: &str) -> bool {
        doc::db().enums.contains_key(name) || Self::dump_items(name).is_some()
    }

    /// Whether a script sees `Enum.<name>`: Studio's API dump when loaded
    /// (the reflection database is another version).
    pub fn script_enum_exists(name: &str) -> bool {
        match crate::api::get() {
            Some(api) => api.enums.contains_key(name),
            None => Self::enum_exists(name),
        }
    }

    /// All items of an enum, sorted by value.
    pub fn items(enum_type: &str) -> Option<Vec<(String, u32)>> {
        let mut v: Vec<(String, u32)> = match doc::db().enums.get(enum_type) {
            Some(e) => e.items.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            None => Self::dump_items(enum_type)?.clone(),
        };
        v.sort_by_key(|(_, v)| *v);
        Some(v)
    }

    pub fn from_value(enum_type: &str, value: u32) -> Option<Self> {
        let Some(e) = doc::db().enums.get(enum_type) else {
            let (name, value) = Self::dump_items(enum_type)?.iter().find(|(_, v)| *v == value)?.clone();
            return Some(LEnumItem { enum_type: enum_type.to_string(), name, value });
        };
        let name = e
            .items
            .iter()
            .find(|(_, v)| **v == value)
            .map(|(k, _)| k.to_string())?;
        Some(LEnumItem {
            enum_type: enum_type.to_string(),
            name,
            value,
        })
    }

    pub fn from_name(enum_type: &str, name: &str) -> Option<Self> {
        let Some(e) = doc::db().enums.get(enum_type) else {
            let (name, value) = Self::dump_items(enum_type)?.iter().find(|(n, _)| n == name)?.clone();
            return Some(LEnumItem { enum_type: enum_type.to_string(), name, value });
        };
        if let Some(value) = e.items.get(name) {
            return Some(LEnumItem {
                enum_type: enum_type.to_string(),
                name: name.to_string(),
                value: *value,
            });
        }
        // A legacy name (Studio's API dump, LegacyNames) is the item it
        // became: Enum.ContextActionPriority.Default is Medium.
        let now = crate::api::get()?
            .legacy_enum_items
            .get(&(enum_type.to_string(), name.to_string()))?;
        let value = *e.items.get(now.as_str())?;
        Some(LEnumItem {
            enum_type: enum_type.to_string(),
            name: now.clone(),
            value,
        })
    }
}

// Enum, Enums and EnumItem metamethods are C functions as in Studio
//: reads check self then the key, writes the
// key then self, and every write fails. Methods (GetEnums, GetEnumItems,
// FromName, FromValue, IsA) are new named C functions on every read.
const ITEM_INDEX: crate::cmeta::Spec = crate::cmeta::Spec { self_types: &[c"EnumItem"], key_first: false };
const ITEM_NEWINDEX: crate::cmeta::Spec = crate::cmeta::Spec { self_types: &[c"EnumItem"], key_first: true };
const ENUM_INDEX: crate::cmeta::Spec = crate::cmeta::Spec { self_types: &[c"Enum"], key_first: false };
const ENUM_NEWINDEX: crate::cmeta::Spec = crate::cmeta::Spec { self_types: &[c"Enum"], key_first: true };
const ENUMS: crate::cmeta::Spec = crate::cmeta::Spec { self_types: &[c"Enums"], key_first: true };

/// A metamethod (index or newindex) of an Enum type, cached per VM.
fn enum_meta(
    lua: &Lua,
    key: &str,
    spec: &'static crate::cmeta::Spec,
    f: impl Fn(&Lua, mlua::AnyUserData, String) -> LResult<Value> + mlua::MaybeSend + 'static,
) -> LResult<mlua::Function> {
    crate::cmeta::cached(lua, key, |lua| {
        let inner = lua.create_function(move |lua, (this, key, _): (mlua::AnyUserData, String, Value)| {
            f(lua, this, key)
        })?;
        crate::cmeta::wrap_meta(lua, inner, false, spec)
    })
}

/// A method of an Enum type: a named C function whose self must be
/// `self_type`; `f` gets the call's arguments after self.
fn enum_method<T: 'static + Clone>(
    lua: &Lua,
    name: &str,
    self_type: &str,
    f: impl Fn(&Lua, T, mlua::MultiValue) -> LResult<Value> + mlua::MaybeSend + 'static,
) -> LResult<Value> {
    let check = lua.create_function(move |lua, mut args: mlua::MultiValue| {
        let this = match args.pop_front() {
            Some(Value::UserData(u)) => u.borrow::<T>()?.clone(),
            _ => return Err(rt_err("invalid self")),
        };
        Ok((true, f(lua, this, args)?))
    })?;
    Ok(Value::Function(crate::cmeta::method(lua, name, Some(check), None, Some(self_type))?))
}

impl UserData for LEnumItem {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "EnumItem");
        f.add_meta_field_with("__index", |lua| {
            enum_meta(lua, "rs_enumitem_index", &ITEM_INDEX, |lua, this, key| {
                let this = this.borrow::<LEnumItem>()?.clone();
                Ok(match key.as_str() {
                    "Name" => Value::String(lua.create_string(&this.name)?),
                    "Value" => Value::Number(this.value as f64),
                    "EnumType" => enum_type_value(lua, &this.enum_type)?,
                    "IsA" => enum_method(lua, "IsA", "EnumItem", |_, this: LEnumItem, args| {
                        Ok(Value::Boolean(matches!(
                            args.front(),
                            Some(Value::String(s)) if s.to_str().is_ok_and(|s| *s == this.enum_type)
                        )))
                    })?,
                    _ => {
                        return Err(rt_err(format!(
                            "{key} is not a valid member of \"Enum.{}.{}\"",
                            this.enum_type, this.name
                        )))
                    }
                })
            })
        });
        f.add_meta_field_with("__newindex", |lua| {
            enum_meta(lua, "rs_enumitem_newindex", &ITEM_NEWINDEX, |_, _, key| {
                Err(rt_err(format!("{key} is not a valid member of EnumItem")))
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("Enum.{}.{}", this.enum_type, this.name))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LEnumItem| {
            Ok(this.enum_type == o.enum_type && this.value == o.value)
        });
    }
}

/// `Enum.Material` and friends.
#[derive(Debug, Clone, PartialEq)]
pub struct LEnum(pub String);

impl UserData for LEnum {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Enum");
        f.add_meta_field_with("__index", |lua| {
            enum_meta(lua, "rs_enum_index", &ENUM_INDEX, |lua, this, key| {
                let this = this.borrow::<LEnum>()?.clone();
                LEnum::index(lua, &this, key)
            })
        });
        f.add_meta_field_with("__newindex", |lua| {
            enum_meta(lua, "rs_enum_newindex", &ENUM_NEWINDEX, |_, this, key| {
                let this = this.borrow::<LEnum>()?.clone();
                Err(rt_err(format!("{key} is not a valid member of \"Enum.{}\"", this.0)))
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.0.clone()));
    }
}

impl LEnum {
    fn index(lua: &Lua, this: &LEnum, key: String) -> mlua::Result<Value> {
        if let Some(item) = LEnumItem::from_name(&this.0, &key) {
            return enum_value(lua, item);
        }
        match key.as_str() {
            "GetEnumItems" => enum_method(lua, "GetEnumItems", "Enum", |lua, this: LEnum, _| {
                let items = LEnumItem::items(&this.0).ok_or_else(|| rt_err("unknown enum"))?;
                let t = lua.create_table_with_capacity(items.len(), 0)?;
                for (i, (k, v)) in items.into_iter().enumerate() {
                    // The same objects as Enum.X.Y (an admin script keys a table by
                    // GetEnumItems' items, then looks up
                    // Enum.DataStoreRequestType.SetIncrementAsync).
                    t.raw_set(i + 1, enum_value(lua, LEnumItem { enum_type: this.0.clone(), name: k, value: v })?)?;
                }
                Ok(Value::Table(t))
            }),
            // A missing name or value is nil.
            "FromName" => enum_method(lua, "FromName", "Enum", |lua, this: LEnum, args| {
                match args.front() {
                    Some(Value::String(s)) => match LEnumItem::from_name(&this.0, &s.to_str()?) {
                        Some(item) => enum_value(lua, item),
                        None => Ok(Value::Nil),
                    },
                    _ => Ok(Value::Nil),
                }
            }),
            "FromValue" => enum_method(lua, "FromValue", "Enum", |lua, this: LEnum, args| {
                let v = match args.front() {
                    Some(Value::Integer(n)) => *n as f64,
                    Some(Value::Number(n)) => *n,
                    _ => return Ok(Value::Nil),
                };
                if v < 0.0 || v.fract() != 0.0 || v > u32::MAX as f64 {
                    return Ok(Value::Nil);
                }
                match LEnumItem::from_value(&this.0, v as u32) {
                    Some(item) => enum_value(lua, item),
                    None => Ok(Value::Nil),
                }
            }),
            _ => Err(rt_err(format!("{key} is not a valid member of \"Enum.{}\"", this.0))),
        }
    }
}

/// The one Lua object for `Enum.<name>` (`Enum.Material == Enum.Material`).
pub fn enum_type_value(lua: &Lua, name: &str) -> LResult<Value> {
    let cache: Table = match lua.named_registry_value::<Option<Table>>("rs_enum_types")? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            lua.set_named_registry_value("rs_enum_types", t.clone())?;
            t
        }
    };
    if let Value::UserData(u) = cache.raw_get::<Value>(name)? {
        return Ok(Value::UserData(u));
    }
    let u = lua.create_userdata(LEnum(name.to_string()))?;
    cache.raw_set(name, u.clone())?;
    Ok(Value::UserData(u))
}

/// Every enum a script sees (`Enum:GetEnums()`), sorted: Studio's own API
/// dump (app/api-dump.json, 0.744: 652), else the reflection database.
pub fn enum_names() -> Vec<String> {
    let mut v: Vec<String> = match crate::api::get() {
        Some(api) => api.enums.keys().cloned().collect(),
        None => doc::db().enums.keys().map(|k| k.to_string()).collect(),
    };
    v.sort();
    v.dedup();
    v
}

/// The global `Enum` (typeof "Enums", tostring "Enums").
#[derive(Debug, Clone)]
pub struct LEnums;

impl UserData for LEnums {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Enums");
        f.add_meta_field_with("__index", |lua| {
            enum_meta(lua, "rs_enums_index", &ENUMS, |lua, _, key| {
                if LEnumItem::script_enum_exists(&key) {
                    return enum_type_value(lua, &key);
                }
                if key == "GetEnums" {
                    return enum_method(lua, "GetEnums", "Enums", |lua, _: LEnums, _| {
                        let names = enum_names();
                        let t = lua.create_table_with_capacity(names.len(), 0)?;
                        for (i, n) in names.iter().enumerate() {
                            t.raw_set(i + 1, enum_type_value(lua, n)?)?;
                        }
                        Ok(Value::Table(t))
                    });
                }
                Err(rt_err(format!("{key} is not a valid member of \"Enum\"")))
            })
        });
        f.add_meta_field_with("__newindex", |lua| {
            enum_meta(lua, "rs_enums_newindex", &ENUMS, |_, _, key| {
                Err(rt_err(format!("{key} is not a valid member of \"Enum\"")))
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("Enums"));
    }
}

// ----------------------------------------------------------- constructors

fn f(v: &Value) -> f32 {
    num(v).unwrap_or(0.0) as f32
}

/// Installs Vector3, Vector2, CFrame, Color3, BrickColor, UDim, UDim2 and
/// Enum into `globals`.
pub fn install(lua: &Lua, g: &Table) -> LResult<()> {
    install_font(lua, g)?;
    crate::sharedtable::install(lua, g)?;
    install_vector3(lua)?;
    // Vector3
    let v3 = lua.create_table()?;
    v3.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            Ok(LVector3(Vec3::new(
                a.first().map(f).unwrap_or(0.0),
                a.get(1).map(f).unwrap_or(0.0),
                a.get(2).map(f).unwrap_or(0.0),
            )))
        })?,
    )?;
    v3.set("zero", LVector3(Vec3::ZERO))?;
    v3.set("one", LVector3(Vec3::ONE))?;
    v3.set("xAxis", LVector3(Vec3::X))?;
    v3.set("yAxis", LVector3(Vec3::Y))?;
    v3.set("zAxis", LVector3(Vec3::Z))?;
    v3.set(
        "FromNormalId",
        lua.create_function(|_, n: LEnumItem| {
            Ok(LVector3(match n.name.as_str() {
                "Right" => Vec3::X,
                "Top" => Vec3::Y,
                "Back" => Vec3::Z,
                "Left" => Vec3::NEG_X,
                "Bottom" => Vec3::NEG_Y,
                _ => Vec3::NEG_Z,
            }))
        })?,
    )?;
    v3.set(
        "FromAxis",
        lua.create_function(|_, a: LEnumItem| {
            Ok(LVector3(match a.name.as_str() {
                "Y" => Vec3::Y,
                "Z" => Vec3::Z,
                _ => Vec3::X,
            }))
        })?,
    )?;
    g.set("Vector3", v3)?;

    let v2 = lua.create_table()?;
    v2.set(
        "new",
        lua.create_function(|_, (x, y): (Option<f32>, Option<f32>)| {
            Ok(LVector2(Vec2::new(x.unwrap_or(0.0), y.unwrap_or(0.0))))
        })?,
    )?;
    v2.set("zero", LVector2(Vec2::ZERO))?;
    v2.set("one", LVector2(Vec2::ONE))?;
    g.set("Vector2", v2)?;

    // CFrame
    let cf = lua.create_table()?;
    cf.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            let id = Xform {
                pos: Vec3::ZERO,
                rot: Mat3::IDENTITY,
            };
            Ok(LCFrame(match a.len() {
                0 => id,
                1 => Xform {
                    pos: ud::<LVector3>(&a[0]).ok_or_else(|| ctor_arg(&a, 0, "Vector3"))?.0,
                    rot: Mat3::IDENTITY,
                },
                2 => {
                    let p = ud::<LVector3>(&a[0]).ok_or_else(|| ctor_arg(&a, 0, "Vector3"))?.0;
                    let t = ud::<LVector3>(&a[1]).ok_or_else(|| ctor_arg(&a, 1, "Vector3"))?.0;
                    look_at_legacy(p, t)
                }
                3 => Xform {
                    pos: Vec3::new(f(&a[0]), f(&a[1]), f(&a[2])),
                    rot: Mat3::IDENTITY,
                },
                7 => {
                    let q = Quat::from_xyzw(f(&a[3]), f(&a[4]), f(&a[5]), f(&a[6])).normalize();
                    Xform {
                        pos: Vec3::new(f(&a[0]), f(&a[1]), f(&a[2])),
                        rot: Mat3::from_quat(q),
                    }
                }
                12 => {
                    let c: Vec<f32> = a.iter().map(f).collect();
                    Xform {
                        pos: Vec3::new(c[0], c[1], c[2]),
                        rot: Mat3::from_cols(
                            Vec3::new(c[3], c[6], c[9]),
                            Vec3::new(c[4], c[7], c[10]),
                            Vec3::new(c[5], c[8], c[11]),
                        ),
                    }
                }
                n => {
                    return Err(rt_err(format!(
                        "invalid number of arguments to CFrame.new: {n}"
                    )))
                }
            }))
        })?,
    )?;
    let angles = lua.create_function(|_, (x, y, z): (f32, f32, f32)| {
        Ok(LCFrame(Xform {
            pos: Vec3::ZERO,
            rot: Mat3::from_euler(EulerRot::XYZ, x, y, z),
        }))
    })?;
    cf.set("Angles", angles.clone())?;
    cf.set("fromEulerAnglesXYZ", angles)?;
    let orient = lua.create_function(|_, (x, y, z): (f32, f32, f32)| {
        Ok(LCFrame(Xform {
            pos: Vec3::ZERO,
            rot: Mat3::from_euler(EulerRot::YXZ, y, x, z),
        }))
    })?;
    cf.set("fromOrientation", orient.clone())?;
    cf.set("fromEulerAnglesYXZ", orient)?;
    cf.set(
        "fromEulerAngles",
        lua.create_function(|_, (x, y, z, order): (f32, f32, f32, Option<Value>)| {
            let order = rotation_order(order.as_ref())?;
            Ok(LCFrame(Xform {
                pos: Vec3::ZERO,
                rot: euler_mat(&order, [x, y, z]),
            }))
        })?,
    )?;
    cf.set(
        "fromMatrix",
        lua.create_function(
            |_, (p, vx, vy, vz): (LVector3, LVector3, LVector3, Option<LVector3>)| {
                let vz = vz
                    .map(|v| v.0)
                    .unwrap_or_else(|| vx.0.cross(vy.0).normalize_or_zero());
                Ok(LCFrame(Xform {
                    pos: p.0,
                    rot: Mat3::from_cols(vx.0, vy.0, vz),
                }))
            },
        )?,
    )?;
    cf.set(
        "lookAlong",
        lua.create_function(|_, (p, dir, up): (LVector3, LVector3, Option<LVector3>)| {
            Ok(LCFrame(look_at(
                p.0,
                p.0 + dir.0,
                up.map(|u| u.0).unwrap_or(Vec3::Y),
            )))
        })?,
    )?;
    cf.set(
        "fromAxisAngle",
        lua.create_function(|_, (axis, a): (LVector3, f32)| {
            Ok(LCFrame(Xform {
                pos: Vec3::ZERO,
                rot: Mat3::from_axis_angle(axis.0.normalize_or(Vec3::Y), a),
            }))
        })?,
    )?;
    cf.set(
        "lookAt",
        lua.create_function(|_, (p, t, up): (LVector3, LVector3, Option<LVector3>)| {
            Ok(LCFrame(look_at(
                p.0,
                t.0,
                up.map(|u| u.0).unwrap_or(Vec3::Y),
            )))
        })?,
    )?;
    cf.set(
        "identity",
        LCFrame(Xform {
            pos: Vec3::ZERO,
            rot: Mat3::IDENTITY,
        }),
    )?;
    g.set("CFrame", cf)?;

    // Color3
    let c3 = lua.create_table()?;
    c3.set(
        "new",
        lua.create_function(|_, (r, gg, b): (Option<f32>, Option<f32>, Option<f32>)| {
            Ok(LColor3([
                r.unwrap_or(0.0),
                gg.unwrap_or(0.0),
                b.unwrap_or(0.0),
            ]))
        })?,
    )?;
    c3.set(
        "fromRGB",
        lua.create_function(|_, (r, gg, b): (Option<f32>, Option<f32>, Option<f32>)| {
            Ok(LColor3([
                r.unwrap_or(0.0) / 255.0,
                gg.unwrap_or(0.0) / 255.0,
                b.unwrap_or(0.0) / 255.0,
            ]))
        })?,
    )?;
    c3.set(
        "fromHSV",
        lua.create_function(|_, (h, s, v): (f32, f32, f32)| Ok(LColor3(hsv_to_rgb(h, s, v))))?,
    )?;
    c3.set(
        "fromHex",
        lua.create_function(|_, hex: String| {
            let h = hex.trim_start_matches('#');
            let h = if h.len() == 3 {
                h.chars().flat_map(|c| [c, c]).collect::<String>()
            } else {
                h.to_string()
            };
            let n = u32::from_str_radix(&h, 16)
                .map_err(|_| rt_err("Unable to convert characters to hex value"))?;
            Ok(LColor3([
                ((n >> 16) & 255) as f32 / 255.0,
                ((n >> 8) & 255) as f32 / 255.0,
                (n & 255) as f32 / 255.0,
            ]))
        })?,
    )?;
    c3.set(
        "toHSV",
        lua.create_function(|_, c: LColor3| {
            let (h, s, v) = rgb_to_hsv(c.0);
            Ok((h as f64, s as f64, v as f64))
        })?,
    )?;
    g.set("Color3", c3)?;

    // BrickColor
    let bc = lua.create_table()?;
    bc.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            let b = match (a.first(), a.len()) {
                (Some(Value::String(s)), _) => BrickColor::from_name(&s.to_str()?),
                (Some(v), 1) if num(v).is_some() => {
                    BrickColor::from_number(num(v).unwrap_or(0.0) as u16)
                }
                (Some(v), 1) => ud::<LColor3>(v)
                    .map(|c| nearest_brick(c.0.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8))),
                (Some(_), 3) => Some(nearest_brick(
                    [f(&a[0]), f(&a[1]), f(&a[2])]
                        .map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8),
                )),
                _ => None,
            };
            Ok(LBrickColor(b.unwrap_or_else(|| {
                BrickColor::from_number(194).expect("default brick")
            })))
        })?,
    )?;
    bc.set(
        "palette",
        lua.create_function(|_, i: u16| {
            Ok(LBrickColor(
                BrickColor::from_number(i.max(1))
                    .unwrap_or_else(|| BrickColor::from_number(194).expect("default")),
            ))
        })?,
    )?;
    let random_brick = lua.create_function(|_, ()| {
        let n = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
            % 200) as u16
            + 1;
        Ok(LBrickColor(BrickColor::from_number(n).unwrap_or_else(
            || BrickColor::from_number(194).expect("default"),
        )))
    })?;
    bc.set("Random", random_brick.clone())?;
    bc.set("random", random_brick)?;
    for (name, n) in [
        ("White", 1),
        ("Gray", 194),
        ("DarkGray", 199),
        ("Black", 26),
        ("Red", 21),
        ("Yellow", 24),
        ("Green", 28),
        ("Blue", 23),
    ] {
        bc.set(
            name,
            lua.create_function(move |_, ()| {
                Ok(LBrickColor(
                    BrickColor::from_number(n).expect("named brick"),
                ))
            })?,
        )?;
    }
    g.set("BrickColor", bc)?;

    // UDim / UDim2
    let u = lua.create_table()?;
    u.set(
        "new",
        lua.create_function(|_, (s, o): (Option<f32>, Option<f64>)| {
            Ok(LUDim {
                scale: s.unwrap_or(0.0),
                offset: o.unwrap_or(0.0) as i32,
            })
        })?,
    )?;
    g.set("UDim", u)?;
    let u2 = lua.create_table()?;
    u2.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            if a.len() == 2 {
                return Ok(LUDim2 {
                    x: ud::<LUDim>(&a[0]).unwrap_or(LUDim {
                        scale: 0.0,
                        offset: 0,
                    }),
                    y: ud::<LUDim>(&a[1]).unwrap_or(LUDim {
                        scale: 0.0,
                        offset: 0,
                    }),
                });
            }
            let g = |i: usize| a.get(i).map(f).unwrap_or(0.0);
            Ok(LUDim2 {
                x: LUDim {
                    scale: g(0),
                    offset: g(1) as i32,
                },
                y: LUDim {
                    scale: g(2),
                    offset: g(3) as i32,
                },
            })
        })?,
    )?;
    u2.set(
        "fromScale",
        lua.create_function(|_, (x, y): (Option<f32>, Option<f32>)| {
            Ok(LUDim2 {
                x: LUDim {
                    scale: x.unwrap_or(0.0),
                    offset: 0,
                },
                y: LUDim {
                    scale: y.unwrap_or(0.0),
                    offset: 0,
                },
            })
        })?,
    )?;
    u2.set(
        "fromOffset",
        lua.create_function(|_, (x, y): (Option<f64>, Option<f64>)| {
            Ok(LUDim2 {
                x: LUDim {
                    scale: 0.0,
                    offset: x.unwrap_or(0.0) as i32,
                },
                y: LUDim {
                    scale: 0.0,
                    offset: y.unwrap_or(0.0) as i32,
                },
            })
        })?,
    )?;
    g.set("UDim2", u2)?;

    g.set("Enum", LEnums)?;
    install_datatypes(lua, g)?;
    Ok(())
}

// ============================================================================
// Further datatypes from the public reference: Ray, Region3, Region3int16,
// Vector3int16, Vector2int16, NumberRange, NumberSequence(+Keypoint),
// ColorSequence(+Keypoint), Rect, PhysicalProperties, TweenInfo, Random,
// DateTime, Faces, Axes, RaycastParams, OverlapParams, CatalogSearchParams.
// ============================================================================

macro_rules! from_ud {
    ($t:ty, $name:literal) => {
        impl FromLua for $t {
            fn from_lua(v: Value, _: &Lua) -> LResult<Self> {
                ud::<$t>(&v).ok_or_else(|| {
                    rt_err(format!(concat!($name, " expected, got {}"), v.type_name()))
                })
            }
        }
    };
}

fn fnum(x: f32) -> String {
    fmt_num(x)
}

/// `Enum.RotationOrder` argument (EnumItem, name string or nil → "XYZ").
fn rotation_order(v: Option<&Value>) -> LResult<String> {
    let name = match v {
        None | Some(Value::Nil) => return Ok("XYZ".into()),
        Some(Value::String(s)) => s.to_str()?.to_string(),
        Some(v) => ud::<LEnumItem>(v)
            .map(|e| e.name)
            .ok_or_else(|| rt_err("Enum.RotationOrder expected"))?,
    };
    match name.as_str() {
        "XYZ" | "XZY" | "YXZ" | "YZX" | "ZXY" | "ZYX" => Ok(name),
        _ => Err(rt_err(format!("invalid rotation order '{name}'"))),
    }
}

fn euler_rot(order: &str) -> EulerRot {
    match order {
        "XZY" => EulerRot::XZY,
        "YXZ" => EulerRot::YXZ,
        "YZX" => EulerRot::YZX,
        "ZXY" => EulerRot::ZXY,
        "ZYX" => EulerRot::ZYX,
        _ => EulerRot::XYZ,
    }
}

/// R = R_a * R_b * R_c for order "abc" (so "XYZ" matches CFrame.Angles).
fn euler_mat(order: &str, xyz: [f32; 3]) -> Mat3 {
    order.chars().fold(Mat3::IDENTITY, |m, c| {
        m * match c {
            'X' => Mat3::from_rotation_x(xyz[0]),
            'Y' => Mat3::from_rotation_y(xyz[1]),
            _ => Mat3::from_rotation_z(xyz[2]),
        }
    })
}

/// An EnumItem argument of `enum_type`, accepting the item, its name or its
/// value; nil gives `default`.
pub fn enum_arg(v: Option<&Value>, enum_type: &str, default: &str) -> LResult<LEnumItem> {
    let fallback = || {
        LEnumItem::from_name(enum_type, default).unwrap_or(LEnumItem {
            enum_type: enum_type.into(),
            name: default.into(),
            value: 0,
        })
    };
    match v {
        None | Some(Value::Nil) => Ok(fallback()),
        Some(Value::String(s)) => {
            let s = s.to_str()?;
            LEnumItem::from_name(enum_type, &s)
                .ok_or_else(|| rt_err(format!("'{s}' is not a valid Enum.{enum_type}")))
        }
        Some(v) if num(v).is_some() => {
            let n = num(v).unwrap_or(0.0) as u32;
            LEnumItem::from_value(enum_type, n)
                .ok_or_else(|| rt_err(format!("{n} is not a valid Enum.{enum_type}")))
        }
        Some(v) => match ud::<LEnumItem>(v) {
            Some(e) if e.enum_type == enum_type => Ok(e),
            Some(e) => Err(rt_err(format!(
                "Enum.{enum_type} expected, got Enum.{}",
                e.enum_type
            ))),
            None => Err(rt_err(format!(
                "Enum.{enum_type} expected, got {}",
                v.type_name()
            ))),
        },
    }
}

/// Luau's argument error for a constructor (`new`) argument `i` (0-based).
fn ctor_arg(a: &[Value], i: usize, want: &str) -> mlua::Error {
    match a.get(i) {
        Some(v) => crate::cmeta::located(format!(
            "invalid argument #{} to 'new' ({want} expected, got {})",
            i + 1,
            crate::cmeta::type_name(v)
        )),
        None => crate::cmeta::located(format!("missing argument #{} to 'new' ({want} expected)", i + 1)),
    }
}

fn arg_f32(a: &[Value], i: usize) -> f32 {
    a.get(i).and_then(num).unwrap_or(0.0) as f32
}

/// Integer conversion with int16 wrap-around, as Roblox's int16 types do.
fn to_i16(x: f64) -> i16 {
    if x.is_finite() {
        (x.trunc() as i64) as i16
    } else {
        0
    }
}

// --------------------------------------------------------------------- Ray

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LRay {
    pub origin: Vec3,
    pub direction: Vec3,
}
from_ud!(LRay, "Ray");

impl LRay {
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        let d = self.direction.normalize_or_zero();
        self.origin + d * (p - self.origin).dot(d).max(0.0)
    }
}

impl UserData for LRay {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Ray");
        f.add_field_method_get("Origin", |_, this| Ok(LVector3(this.origin)));
        f.add_field_method_get("Direction", |_, this| Ok(LVector3(this.direction)));
        f.add_field_method_get("Unit", |_, this| {
            Ok(LRay {
                origin: this.origin,
                direction: this.direction.normalize_or_zero(),
            })
        });
        f.add_field_method_get("unit", |_, this| {
            Ok(LRay {
                origin: this.origin,
                direction: this.direction.normalize_or_zero(),
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("ClosestPoint", |_, this, p: LVector3| {
            Ok(LVector3(this.closest_point(p.0)))
        });
        m.add_method("Distance", |_, this, p: LVector3| {
            Ok(this.closest_point(p.0).distance(p.0) as f64)
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let (o, d) = (this.origin, this.direction);
            Ok(format!(
                "{{{}, {}, {}}}, {{{}, {}, {}}}",
                fnum(o.x),
                fnum(o.y),
                fnum(o.z),
                fnum(d.x),
                fnum(d.y),
                fnum(d.z)
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LRay| Ok(*this == o));
    }
}

// ----------------------------------------------------------------- Region3

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LRegion3 {
    pub min: Vec3,
    pub max: Vec3,
}
from_ud!(LRegion3, "Region3");

impl UserData for LRegion3 {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Region3");
        f.add_field_method_get("CFrame", |_, this| {
            Ok(LCFrame(Xform {
                pos: (this.min + this.max) * 0.5,
                rot: Mat3::IDENTITY,
            }))
        });
        f.add_field_method_get("Size", |_, this| Ok(LVector3(this.max - this.min)));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("ExpandToGrid", |_, this, res: f32| {
            if !(res > 0.0) {
                return Err(rt_err("Region3:ExpandToGrid: resolution must be positive"));
            }
            Ok(LRegion3 {
                min: (this.min / res).floor() * res,
                max: (this.max / res).ceil() * res,
            })
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let c = components(&Xform {
                pos: (this.min + this.max) * 0.5,
                rot: Mat3::IDENTITY,
            });
            let s = this.max - this.min;
            Ok(format!(
                "{}; {}, {}, {}",
                c.iter()
                    .map(|v| fnum(*v as f32))
                    .collect::<Vec<_>>()
                    .join(", "),
                fnum(s.x),
                fnum(s.y),
                fnum(s.z)
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LRegion3| Ok(*this == o));
    }
}

// ------------------------------------------------- Vector3int16 / Vector2int16

macro_rules! int16_vector {
    ($t:ident, $name:literal, $n:literal, [$($field:literal => $i:literal),+]) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $t(pub [i16; $n]);
        from_ud!($t, $name);

        impl $t {
            fn zip(a: Self, b: Self, op: impl Fn(i16, i16) -> i16) -> Self {
                let mut o = [0i16; $n];
                for i in 0..$n {
                    o[i] = op(a.0[i], b.0[i]);
                }
                $t(o)
            }
            fn scale(a: Self, op: impl Fn(f64) -> f64) -> Self {
                $t(a.0.map(|c| to_i16(op(c as f64))))
            }
        }

        impl UserData for $t {
            fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
                f.add_meta_field("__type", $name);
                $( f.add_field_method_get($field, |_, this| Ok(this.0[$i] as f64)); )+
            }

            fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
                m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
                    Ok(this.0.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", "))
                });
                m.add_meta_method(MetaMethod::Eq, |_, this, o: $t| Ok(*this == o));
                m.add_meta_method(MetaMethod::Unm, |_, this, ()| Ok($t(this.0.map(|c| c.wrapping_neg()))));
                m.add_meta_function(MetaMethod::Add, |_, (a, b): ($t, $t)| Ok($t::zip(a, b, i16::wrapping_add)));
                m.add_meta_function(MetaMethod::Sub, |_, (a, b): ($t, $t)| Ok($t::zip(a, b, i16::wrapping_sub)));
                m.add_meta_function(MetaMethod::Mul, |_, (a, b): (Value, Value)| {
                    match (ud::<$t>(&a), ud::<$t>(&b), num(&a), num(&b)) {
                        (Some(x), Some(y), _, _) => Ok($t::zip(x, y, i16::wrapping_mul)),
                        (Some(x), None, _, Some(k)) => Ok($t::scale(x, |c| c * k)),
                        (None, Some(y), Some(k), _) => Ok($t::scale(y, |c| c * k)),
                        // Studio: Vector3int16 * Color3 is 0, 0, 0.
                        (Some(x), None, _, None) => Ok($t::scale(x, |_| 0.0)),
                        (None, Some(y), None, _) => Ok($t::scale(y, |_| 0.0)),
                        _ => Err(crate::cmeta::located(concat!("missing argument #1 (", $name, " expected)"))),
                    }
                });
                m.add_meta_function(MetaMethod::Div, |_, (a, b): (Value, Value)| {
                    match (ud::<$t>(&a), ud::<$t>(&b), num(&a), num(&b)) {
                        (Some(x), Some(y), _, _) => Ok($t::zip(x, y, |p, q| to_i16(p as f64 / q as f64))),
                        (Some(x), None, _, Some(k)) => Ok($t::scale(x, |c| c / k)),
                        (None, Some(y), Some(k), _) => Ok($t::scale(y, |c| k / c)),
                        _ => Err(rt_err(concat!("invalid ", $name, " division"))),
                    }
                });
            }
        }
    };
}

int16_vector!(LVector3int16, "Vector3int16", 3, ["X" => 0, "Y" => 1, "Z" => 2]);
int16_vector!(LVector2int16, "Vector2int16", 2, ["X" => 0, "Y" => 1]);

// ------------------------------------------------------------ Region3int16

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LRegion3int16 {
    pub min: LVector3int16,
    pub max: LVector3int16,
}
from_ud!(LRegion3int16, "Region3int16");

impl UserData for LRegion3int16 {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Region3int16");
        f.add_field_method_get("Min", |_, this| Ok(this.min));
        f.add_field_method_get("Max", |_, this| Ok(this.max));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let s = |v: LVector3int16| {
                v.0.iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            Ok(format!("{}; {}", s(this.min), s(this.max)))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LRegion3int16| Ok(*this == o));
    }
}

// ------------------------------------------------------------- NumberRange

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LNumberRange {
    pub min: f32,
    pub max: f32,
}
from_ud!(LNumberRange, "NumberRange");

impl UserData for LNumberRange {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "NumberRange");
        f.add_field_method_get("Min", |_, this| Ok(this.min as f64));
        f.add_field_method_get("Max", |_, this| Ok(this.max as f64));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("{} {} ", g6(this.min), g6(this.max)))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LNumberRange| Ok(*this == o));
    }
}

// ---------------------------------------------- NumberSequence / ColorSequence

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LNumberSequenceKeypoint {
    pub time: f32,
    pub value: f32,
    pub envelope: f32,
}
from_ud!(LNumberSequenceKeypoint, "NumberSequenceKeypoint");

impl LNumberSequenceKeypoint {
    fn text(&self) -> String {
        format!(
            "{} {} {} ",
            g6(self.time),
            g6(self.value),
            g6(self.envelope)
        )
    }
}

impl UserData for LNumberSequenceKeypoint {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "NumberSequenceKeypoint");
        f.add_field_method_get("Time", |_, this| Ok(this.time as f64));
        f.add_field_method_get("Value", |_, this| Ok(this.value as f64));
        f.add_field_method_get("Envelope", |_, this| Ok(this.envelope as f64));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.text()));
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LNumberSequenceKeypoint| {
            Ok(*this == o)
        });
    }
}

/// FloatCurveKey (FloatCurve keys, HapticEffect waveforms): Time, Value and
/// Interpolation fixed at creation; the tangents (nil until set) writable.
/// Place D's second admin script builds its haptics from them as its client loads.
#[derive(Debug, Clone, PartialEq)]
pub struct LFloatCurveKey {
    pub time: f32,
    pub value: f32,
    pub interpolation: LEnumItem,
    pub left_tangent: Option<f32>,
    pub right_tangent: Option<f32>,
}
from_ud!(LFloatCurveKey, "FloatCurveKey");

impl UserData for LFloatCurveKey {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "FloatCurveKey");
        f.add_field_method_get("Time", |_, this| Ok(this.time as f64));
        f.add_field_method_get("Value", |_, this| Ok(this.value as f64));
        f.add_field_method_get("Interpolation", |lua, this| {
            enum_value(lua, this.interpolation.clone())
        });
        f.add_field_method_get("LeftTangent", |_, this| Ok(this.left_tangent.map(|v| v as f64)));
        f.add_field_method_get("RightTangent", |_, this| Ok(this.right_tangent.map(|v| v as f64)));
        f.add_field_method_set("LeftTangent", |_, this, v: Option<f32>| {
            this.left_tangent = v;
            Ok(())
        });
        f.add_field_method_set("RightTangent", |_, this, v: Option<f32>| {
            this.right_tangent = v;
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("FloatCurveKey"));
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LFloatCurveKey| Ok(*this == o));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LNumberSequence {
    pub keypoints: Vec<LNumberSequenceKeypoint>,
}
from_ud!(LNumberSequence, "NumberSequence");

impl UserData for LNumberSequence {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "NumberSequence");
        f.add_field_method_get("Keypoints", |lua, this| {
            lua.create_sequence_from(this.keypoints.iter().copied())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(this.keypoints.iter().map(|k| k.text()).collect::<String>())
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LNumberSequence| Ok(*this == o));
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LColorSequenceKeypoint {
    pub time: f32,
    pub value: [f32; 3],
}
from_ud!(LColorSequenceKeypoint, "ColorSequenceKeypoint");

impl LColorSequenceKeypoint {
    fn text(&self) -> String {
        format!(
            "{} {} {} {} 0 ",
            g6(self.time),
            g6(self.value[0]),
            g6(self.value[1]),
            g6(self.value[2])
        )
    }
}

impl UserData for LColorSequenceKeypoint {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "ColorSequenceKeypoint");
        f.add_field_method_get("Time", |_, this| Ok(this.time as f64));
        f.add_field_method_get("Value", |_, this| Ok(LColor3(this.value)));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.text()));
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LColorSequenceKeypoint| {
            Ok(*this == o)
        });
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LColorSequence {
    pub keypoints: Vec<LColorSequenceKeypoint>,
}
from_ud!(LColorSequence, "ColorSequence");

impl UserData for LColorSequence {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "ColorSequence");
        f.add_field_method_get("Keypoints", |lua, this| {
            lua.create_sequence_from(this.keypoints.iter().copied())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(this.keypoints.iter().map(|k| k.text()).collect::<String>())
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LColorSequence| Ok(*this == o));
    }
}

/// Documented constraints: 2..=20 keypoints, non-descending time, first at 0
/// and last at 1. (Error wording is ours.)
fn check_keypoints(name: &str, times: &[f32]) -> LResult<()> {
    if times.len() < 2 {
        return Err(rt_err(format!("{name}: requires at least 2 keypoints")));
    }
    if times.len() > 20 {
        return Err(rt_err(format!(
            "{name}: table is too long (max 20 keypoints)"
        )));
    }
    if times.windows(2).any(|w| w[1] < w[0]) {
        return Err(rt_err(format!(
            "{name}: all keypoints must be ordered by time"
        )));
    }
    if times[0].abs() > 1e-4 {
        return Err(rt_err(format!("{name} must start at time=0.0")));
    }
    if (times[times.len() - 1] - 1.0).abs() > 1e-4 {
        return Err(rt_err(format!("{name} must end at time=1.0")));
    }
    Ok(())
}

fn keypoint_list<T: 'static + Clone>(t: &Table, what: &str) -> LResult<Vec<T>> {
    t.sequence_values::<Value>()
        .map(|v| {
            v.and_then(|v| {
                ud::<T>(&v).ok_or_else(|| rt_err(format!("{what} expected, got {}", v.type_name())))
            })
        })
        .collect()
}

// ------------------------------------------------------------------- Rect

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LRect {
    pub min: Vec2,
    pub max: Vec2,
}
from_ud!(LRect, "Rect");

impl UserData for LRect {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Rect");
        f.add_field_method_get("Min", |_, this| Ok(LVector2(this.min)));
        f.add_field_method_get("Max", |_, this| Ok(LVector2(this.max)));
        f.add_field_method_get("Width", |_, this| Ok((this.max.x - this.min.x) as f64));
        f.add_field_method_get("Height", |_, this| Ok((this.max.y - this.min.y) as f64));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "{}, {}, {}, {}",
                fnum(this.min.x),
                fnum(this.min.y),
                fnum(this.max.x),
                fnum(this.max.y)
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LRect| Ok(*this == o));
    }
}

// ----------------------------------------------------- PhysicalProperties

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LPhysicalProperties {
    pub density: f32,
    pub friction: f32,
    pub elasticity: f32,
    pub friction_weight: f32,
    pub elasticity_weight: f32,
    pub acoustic_absorption: f32,
    /// Studio's "default" value (`PhysicalProperties.new('a')`: tostring
    /// "default"; a part given it has no custom properties).
    pub default: bool,
}
from_ud!(LPhysicalProperties, "PhysicalProperties");

/// Default AcousticAbsorption: the reference lists the property but no
/// default value, so this is an estimate.
const DEFAULT_ACOUSTIC_ABSORPTION: f32 = 1.0;

/// (material, density, elasticity, friction, elasticityWeight, frictionWeight)
/// from the "Materials" page of the Roblox creator docs.
const MATERIAL_PHYSICS: &[(&str, f32, f32, f32, f32, f32)] = &[
    ("Asphalt", 2.36, 0.2, 0.8, 1.0, 0.3),
    ("Basalt", 2.691, 0.15, 0.7, 1.0, 0.3),
    ("Brick", 1.922, 0.15, 0.8, 1.0, 0.3),
    ("Cardboard", 0.7, 0.05, 0.5, 2.0, 1.0),
    ("Carpet", 1.1, 0.25, 0.4, 2.0, 1.0),
    ("CeramicTiles", 2.4, 0.2, 0.51, 1.0, 1.0),
    ("ClayRoofTiles", 2.0, 0.2, 0.51, 1.0, 1.0),
    ("Cobblestone", 2.691, 0.17, 0.5, 1.0, 1.0),
    ("Concrete", 2.403, 0.2, 0.7, 1.0, 0.3),
    ("CorrodedMetal", 7.85, 0.2, 0.7, 1.0, 1.0),
    ("CrackedLava", 2.691, 0.15, 0.65, 1.0, 1.0),
    ("DiamondPlate", 7.85, 0.25, 0.35, 1.0, 1.0),
    ("Fabric", 0.7, 0.05, 0.35, 1.0, 1.0),
    ("Foil", 2.7, 0.25, 0.4, 1.0, 1.0),
    ("ForceField", 2.403, 0.2, 0.25, 1.0, 1.0),
    ("Glacier", 0.919, 0.15, 0.05, 1.0, 2.0),
    ("Glass", 2.403, 0.2, 0.25, 1.0, 1.0),
    ("Granite", 2.691, 0.2, 0.4, 1.0, 1.0),
    ("Grass", 0.9, 0.1, 0.4, 1.5, 1.0),
    ("Ground", 0.9, 0.1, 0.45, 1.0, 1.0),
    ("Ice", 0.919, 0.15, 0.02, 1.0, 3.0),
    ("LeafyGrass", 0.9, 0.1, 0.4, 2.0, 2.0),
    ("Leather", 0.86, 0.25, 0.35, 1.0, 1.0),
    ("Limestone", 2.691, 0.15, 0.5, 1.0, 1.0),
    ("Marble", 2.563, 0.17, 0.2, 1.0, 1.0),
    ("Metal", 7.85, 0.25, 0.4, 1.0, 1.0),
    ("Mud", 0.9, 0.07, 0.3, 4.0, 3.0),
    ("Neon", 0.7, 0.2, 0.3, 1.0, 1.0),
    ("Pavement", 2.691, 0.17, 0.5, 1.0, 0.3),
    ("Pebble", 2.403, 0.17, 0.4, 1.5, 1.0),
    ("Plaster", 0.75, 0.2, 0.6, 1.0, 0.3),
    ("Plastic", 0.7, 0.5, 0.3, 1.0, 1.0),
    ("Rock", 2.691, 0.17, 0.5, 1.0, 1.0),
    ("RoofShingles", 2.36, 0.2, 0.8, 1.0, 0.3),
    ("Rubber", 1.3, 0.95, 1.5, 2.0, 3.0),
    ("Salt", 2.165, 0.05, 0.5, 1.0, 1.0),
    ("Sand", 1.602, 0.05, 0.5, 2.5, 5.0),
    ("Sandstone", 2.691, 0.15, 0.5, 1.0, 5.0),
    ("SmoothPlastic", 0.7, 0.5, 0.2, 1.0, 1.0),
    ("Slate", 2.691, 0.2, 0.4, 1.0, 1.0),
    ("Snow", 0.9, 0.03, 0.3, 4.0, 3.0),
    ("Wood", 0.35, 0.2, 0.48, 1.0, 1.0),
    ("WoodPlanks", 0.35, 0.2, 0.48, 1.0, 1.0),
];

impl LPhysicalProperties {
    /// Default physical properties of a material (by Enum.Material name).
    /// Materials missing from the docs table (e.g. Air, Water, newer
    /// materials) fall back to Plastic — an estimate.
    pub fn for_material(name: &str) -> Self {
        let row = MATERIAL_PHYSICS
            .iter()
            .find(|r| r.0 == name)
            .or_else(|| MATERIAL_PHYSICS.iter().find(|r| r.0 == "Plastic"))
            .copied();
        let (_, density, elasticity, friction, ew, fw) =
            row.unwrap_or(("Plastic", 0.7, 0.5, 0.3, 1.0, 1.0));
        LPhysicalProperties {
            density,
            friction,
            elasticity,
            friction_weight: fw,
            elasticity_weight: ew,
            acoustic_absorption: DEFAULT_ACOUSTIC_ABSORPTION,
            default: false,
        }
    }

    /// Clamp to plausible ranges (estimated: density 0.0001..100,
    /// friction 0..2, elasticity 0..1, weights 0..100).
    fn clamped(self) -> Self {
        LPhysicalProperties {
            density: self.density.clamp(0.0001, 100.0),
            friction: self.friction.clamp(0.0, 2.0),
            elasticity: self.elasticity.clamp(0.0, 1.0),
            friction_weight: self.friction_weight.clamp(0.0, 100.0),
            elasticity_weight: self.elasticity_weight.clamp(0.0, 100.0),
            acoustic_absorption: self.acoustic_absorption.max(0.0),
            default: self.default,
        }
    }
}

impl UserData for LPhysicalProperties {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "PhysicalProperties");
        f.add_field_method_get("Density", |_, this| Ok(this.density as f64));
        f.add_field_method_get("Friction", |_, this| Ok(this.friction as f64));
        f.add_field_method_get("Elasticity", |_, this| Ok(this.elasticity as f64));
        f.add_field_method_get("FrictionWeight", |_, this| Ok(this.friction_weight as f64));
        f.add_field_method_get("ElasticityWeight", |_, this| {
            Ok(this.elasticity_weight as f64)
        });
        f.add_field_method_get("AcousticAbsorption", |_, this| {
            Ok(this.acoustic_absorption as f64)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            if this.default {
                return Ok("default".to_string());
            }
            Ok(format!(
                "{}, {}, {}, {}, {}, {}",
                g6(this.density),
                g6(this.friction),
                g6(this.elasticity),
                g6(this.friction_weight),
                g6(this.elasticity_weight),
                g6(this.acoustic_absorption)
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LPhysicalProperties| {
            Ok(*this == o)
        });
    }
}

// --------------------------------------------------------------- TweenInfo

#[derive(Debug, Clone, PartialEq)]
pub struct LTweenInfo {
    pub time: f64,
    pub easing_style: LEnumItem,
    pub easing_direction: LEnumItem,
    pub repeat_count: i32,
    pub reverses: bool,
    pub delay_time: f64,
}
from_ud!(LTweenInfo, "TweenInfo");

impl UserData for LTweenInfo {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "TweenInfo");
        f.add_field_method_get("Time", |_, this| Ok(this.time));
        f.add_field_method_get("EasingStyle", |_, this| Ok(this.easing_style.clone()));
        f.add_field_method_get("EasingDirection", |_, this| {
            Ok(this.easing_direction.clone())
        });
        f.add_field_method_get("RepeatCount", |_, this| Ok(this.repeat_count as f64));
        f.add_field_method_get("Reverses", |_, this| Ok(this.reverses));
        f.add_field_method_get("DelayTime", |_, this| Ok(this.delay_time));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "Time:{} DelayTime:{} RepeatCount:{} Reverses:{} EasingDirection:{} EasingStyle:{}",
                g6(this.time as f32),
                g6(this.delay_time as f32),
                this.repeat_count,
                if this.reverses { "True" } else { "False" },
                this.easing_direction.name,
                this.easing_style.name
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LTweenInfo| Ok(*this == o));
    }
}

// ------------------------------------------------------------------ Random

/// PCG32 (XSH-RR). Deterministic for a given seed; not Roblox's exact stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LRandom {
    state: u64,
    inc: u64,
}
from_ud!(LRandom, "Random");

impl LRandom {
    /// PCG32 as Studio's Random (from Studio, round 21: step at
    /// increment 105): pcg32_srandom with that increment.
    pub fn new(seed: u64) -> Self {
        let mut r = LRandom {
            state: 0,
            inc: crate::mech::RANDOM_PCG_INC,
        };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    fn from_entropy() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        LRandom::new(
            t ^ COUNTER
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15),
        )
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        xorshifted.rotate_right((old >> 59) as u32)
    }

    /// Two draws, the second in the high half.
    pub fn next_u64(&mut self) -> u64 {
        let lo = self.next_u32() as u64;
        ((self.next_u32() as u64) << 32) | lo
    }

    /// NextNumber(): the 64 bits × 2⁻⁶⁴ (ldexp, rounded to a double).
    pub fn next_unit(&mut self) -> f64 {
        self.next_u64() as f64 * (-64f64).exp2()
    }

    /// NextNumber(a, b): ends put in order, then
    /// min·(1 − t) + max·t kept inside [min, max].
    pub fn next_range(&mut self, a: f64, b: f64) -> f64 {
        let (lo, hi) = if a > b { (b, a) } else { (a, b) };
        let t = self.next_unit();
        let r = lo * (1.0 - t) + hi * t;
        if r < lo {
            lo
        } else {
            hi.min(r)
        }
    }

    /// Uniform integer in [min, max], both inclusive: one 32-bit draw
    /// scaled by the span (Studio's NextInteger, matched to its sequence;
    /// wider spans take 64 bits).
    pub fn next_int(&mut self, min: i64, max: i64) -> i64 {
        let span = (max.wrapping_sub(min) as u64).wrapping_add(1);
        if span == 0 {
            return self.next_u64() as i64;
        }
        if span <= 1 << 32 {
            return min.wrapping_add(((self.next_u32() as u64 * span) >> 32) as i64);
        }
        min.wrapping_add(((self.next_u64() as u128 * span as u128) >> 64) as i64)
    }
}

impl UserData for LRandom {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Random");
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method_mut(
            "NextNumber",
            |_, this, (a, b): (Option<f64>, Option<f64>)| {
                match (a, b) {
                    (None, None) => Ok(this.next_unit()),
                    (Some(lo), Some(hi)) => Ok(this.next_range(lo, hi)),
                    _ => Err(rt_err("Random:NextNumber requires 0 or 2 arguments")),
                }
            },
        );
        m.add_method_mut("NextInteger", |_, this, (a, b): (f64, f64)| {
            let (lo, hi) = (a.floor() as i64, b.floor() as i64);
            if hi < lo {
                return Err(rt_err(
                    "invalid argument #2 to 'NextInteger' (interval is empty)",
                ));
            }
            Ok(this.next_int(lo, hi) as f64)
        });
        m.add_method_mut("NextUnitVector", |_, this, ()| {
            let z = this.next_unit() * 2.0 - 1.0;
            let phi = this.next_unit() * std::f64::consts::TAU;
            let r = (1.0 - z * z).max(0.0).sqrt();
            Ok(LVector3(Vec3::new(
                (r * phi.cos()) as f32,
                (r * phi.sin()) as f32,
                z as f32,
            )))
        });
        m.add_method_mut("Shuffle", |_, this, t: Table| {
            let mut i = t.raw_len() as i64;
            while i > 1 {
                let j = this.next_int(1, i);
                if j != i {
                    let a: Value = t.raw_get(i)?;
                    let b: Value = t.raw_get(j)?;
                    t.raw_set(i, b)?;
                    t.raw_set(j, a)?;
                }
                i -= 1;
            }
            Ok(())
        });
        m.add_method("Clone", |_, this, ()| Ok(*this));
        m.add_meta_method(MetaMethod::ToString, |_, _, ()| Ok("Random"));
        // Studio: Random.new(42) == Random.new(42) (same state).
        m.add_meta_function(MetaMethod::Eq, |_, (a, b): (mlua::AnyUserData, mlua::AnyUserData)| {
            Ok(match (a.borrow::<LRandom>(), b.borrow::<LRandom>()) {
                (Ok(a), Ok(b)) => *a == *b,
                _ => false,
            })
        });
    }
}

// ---------------------------------------------------------------- DateTime

/// Days since 1970-01-01 for a proleptic Gregorian date (Hinnant's
/// days_from_civil).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 31,
    }
}

/// Broken-down UTC time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Civil {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    milli: i64,
    weekday: i64,
    yday: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LDateTime {
    pub millis: i64,
}
from_ud!(LDateTime, "DateTime");

impl LDateTime {
    pub fn now() -> Self {
        let ms = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_millis() as i64,
            Err(e) => -(e.duration().as_millis() as i64),
        };
        LDateTime { millis: ms }
    }

    /// Date fields must form a valid date; time fields may overflow.
    pub fn from_universal(
        y: i64,
        mo: i64,
        d: i64,
        h: i64,
        mi: i64,
        s: i64,
        ms: i64,
    ) -> Option<Self> {
        if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) {
            return None;
        }
        let days = days_from_civil(y, mo, d);
        Some(LDateTime {
            millis: days * 86_400_000 + h * 3_600_000 + mi * 60_000 + s * 1000 + ms,
        })
    }

    fn civil(&self) -> Civil {
        let days = self.millis.div_euclid(86_400_000);
        let rem = self.millis.rem_euclid(86_400_000);
        let (year, month, day) = civil_from_days(days);
        Civil {
            year,
            month,
            day,
            hour: rem / 3_600_000,
            minute: rem / 60_000 % 60,
            second: rem / 1000 % 60,
            milli: rem % 1000,
            weekday: (days + 4).rem_euclid(7),
            yday: days - days_from_civil(year, 1, 1) + 1,
        }
    }

    /// ISO 8601, e.g. `2020-01-02T10:30:45Z` (milliseconds appended only when
    /// non-zero — estimated).
    pub fn iso(&self) -> String {
        let c = self.civil();
        let mut s = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            c.year, c.month, c.day, c.hour, c.minute, c.second
        );
        if c.milli != 0 {
            s.push_str(&format!(".{:03}", c.milli));
        }
        s.push('Z');
        s
    }

    /// Parses `YYYY-MM-DD[THH:MM[:SS[.fff]]][Z|±HH[:MM]]`.
    pub fn parse_iso(s: &str) -> Option<Self> {
        let s = s.trim();
        let b = s.as_bytes();
        let int = |r: std::ops::Range<usize>| -> Option<i64> {
            let t = s.get(r)?;
            if t.is_empty() || !t.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            t.parse().ok()
        };
        if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let (y, mo, d) = (int(0..4)?, int(5..7)?, int(8..10)?);
        let (mut h, mut mi, mut sec, mut ms) = (0, 0, 0, 0);
        let mut i = 10;
        if i < b.len() && (b[i] == b'T' || b[i] == b't' || b[i] == b' ') {
            h = int(i + 1..i + 3)?;
            if b.get(i + 3) != Some(&b':') {
                return None;
            }
            mi = int(i + 4..i + 6)?;
            i += 6;
            if b.get(i) == Some(&b':') {
                sec = int(i + 1..i + 3)?;
                i += 3;
                if b.get(i) == Some(&b'.') || b.get(i) == Some(&b',') {
                    let start = i + 1;
                    let mut end = start;
                    while end < b.len() && b[end].is_ascii_digit() {
                        end += 1;
                    }
                    if end == start {
                        return None;
                    }
                    let frac = &s[start..end.min(start + 3)];
                    ms = frac.parse::<i64>().ok()? * 10i64.pow(3 - frac.len() as u32);
                    i = end;
                }
            }
        }
        let mut offset = 0i64;
        match b.get(i) {
            None => {}
            Some(b'Z') | Some(b'z') if i + 1 == b.len() => {}
            Some(&sign @ (b'+' | b'-')) => {
                let oh = int(i + 1..i + 3)?;
                let rest = s.get(i + 3..)?;
                let om = match rest.len() {
                    0 => 0,
                    2 => int(i + 3..i + 5)?,
                    3 if rest.starts_with(':') => int(i + 4..i + 6)?,
                    _ => return None,
                };
                offset = (oh * 60 + om) * if sign == b'+' { 1 } else { -1 };
            }
            _ => return None,
        }
        if h > 24 || mi > 59 || sec > 60 {
            return None;
        }
        let dt = LDateTime::from_universal(y, mo, d, h, mi, sec, ms)?;
        Some(LDateTime {
            millis: dt.millis - offset * 60_000,
        })
    }

    /// Moment.js-style formatting (the token set the reference lists), en-us
    /// only; `[text]` is literal.
    pub fn format(&self, fmt: &str) -> String {
        const MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        const DAYS: [&str; 7] = [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ];
        const TOKENS: &[&str] = &[
            "LLLL", "llll", "LLL", "lll", "LTS", "LL", "ll", "LT", "L", "l", "YYYY", "YY", "MMMM",
            "MMM", "MM", "M", "DDDD", "DDD", "DD", "D", "dddd", "ddd", "dd", "d", "HH", "H", "hh",
            "h", "mm", "m", "ss", "s", "SSS", "SS", "S", "A", "a",
        ];
        let c = self.civil();
        let h12 = if c.hour % 12 == 0 { 12 } else { c.hour % 12 };
        let mut out = String::new();
        let mut rest = fmt;
        while let Some(ch) = rest.chars().next() {
            if ch == '[' {
                if let Some(end) = rest.find(']') {
                    out.push_str(&rest[1..end]);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
            let Some(tok) = TOKENS.iter().find(|t| rest.starts_with(**t)) else {
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
                continue;
            };
            rest = &rest[tok.len()..];
            let text = match *tok {
                "LT" => self.format("h:mm A"),
                "LTS" => self.format("h:mm:ss A"),
                "L" => self.format("MM/DD/YYYY"),
                "l" => self.format("M/D/YYYY"),
                "LL" => self.format("MMMM D, YYYY"),
                "ll" => self.format("MMM D, YYYY"),
                "LLL" => self.format("MMMM D, YYYY h:mm A"),
                "lll" => self.format("MMM D, YYYY h:mm A"),
                "LLLL" => self.format("dddd, MMMM D, YYYY h:mm A"),
                "llll" => self.format("ddd, MMM D, YYYY h:mm A"),
                "YYYY" => format!("{:04}", c.year),
                "YY" => format!("{:02}", c.year.rem_euclid(100)),
                "MMMM" => MONTHS[(c.month - 1) as usize].to_string(),
                "MMM" => MONTHS[(c.month - 1) as usize][..3].to_string(),
                "MM" => format!("{:02}", c.month),
                "M" => c.month.to_string(),
                "DDDD" => format!("{:03}", c.yday),
                "DDD" => c.yday.to_string(),
                "DD" => format!("{:02}", c.day),
                "D" => c.day.to_string(),
                "dddd" => DAYS[c.weekday as usize].to_string(),
                "ddd" => DAYS[c.weekday as usize][..3].to_string(),
                "dd" => DAYS[c.weekday as usize][..2].to_string(),
                "d" => c.weekday.to_string(),
                "HH" => format!("{:02}", c.hour),
                "H" => c.hour.to_string(),
                "hh" => format!("{:02}", h12),
                "h" => h12.to_string(),
                "mm" => format!("{:02}", c.minute),
                "m" => c.minute.to_string(),
                "ss" => format!("{:02}", c.second),
                "s" => c.second.to_string(),
                "SSS" => format!("{:03}", c.milli),
                "SS" => format!("{:02}", c.milli / 10),
                "S" => (c.milli / 100).to_string(),
                "A" => (if c.hour < 12 { "AM" } else { "PM" }).to_string(),
                _ => (if c.hour < 12 { "am" } else { "pm" }).to_string(),
            };
            out.push_str(&text);
        }
        out
    }

    fn table(&self, lua: &Lua) -> LResult<Table> {
        let c = self.civil();
        let t = lua.create_table()?;
        for (k, v) in [
            ("Year", c.year),
            ("Month", c.month),
            ("Day", c.day),
            ("Hour", c.hour),
            ("Minute", c.minute),
            ("Second", c.second),
            ("Millisecond", c.milli),
        ] {
            t.set(k, v as f64)?;
        }
        Ok(t)
    }
}

impl UserData for LDateTime {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "DateTime");
        f.add_field_method_get("UnixTimestamp", |_, this| {
            Ok(this.millis.div_euclid(1000) as f64)
        });
        f.add_field_method_get("UnixTimestampMillis", |_, this| Ok(this.millis as f64));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // No timezone database is used, so local time is treated as UTC
        // (estimated).
        m.add_method("ToUniversalTime", |lua, this, ()| this.table(lua));
        m.add_method("ToLocalTime", |lua, this, ()| this.table(lua));
        m.add_method("ToIsoDate", |_, this, ()| Ok(this.iso()));
        m.add_method(
            "FormatUniversalTime",
            |_, this, (fmt, _locale): (String, Option<String>)| Ok(this.format(&fmt)),
        );
        m.add_method(
            "FormatLocalTime",
            |_, this, (fmt, _locale): (String, Option<String>)| Ok(this.format(&fmt)),
        );
        // Studio prints the Unix time in milliseconds (`DateTime.fromUnixTimestamp(0)` → "0").
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(this.millis.to_string()));
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LDateTime| Ok(*this == o));
        m.add_meta_function(
            MetaMethod::Lt,
            |_, (a, b): (LDateTime, LDateTime)| Ok(a < b),
        );
        m.add_meta_function(MetaMethod::Le, |_, (a, b): (LDateTime, LDateTime)| {
            Ok(a <= b)
        });
    }
}

// ------------------------------------------------------------ Faces / Axes

/// Bit layout matches rbx_types: Right 1, Top 2, Back 4, Left 8, Bottom 16, Front 32.
const FACE_NAMES: [&str; 6] = ["Right", "Top", "Back", "Left", "Bottom", "Front"];
const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LFaces(pub u8);
from_ud!(LFaces, "Faces");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LAxes(pub u8);
from_ud!(LAxes, "Axes");

fn bit_names(bits: u8, names: &[&str]) -> String {
    names
        .iter()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .map(|(_, n)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

impl UserData for LFaces {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Faces");
        for (i, name) in FACE_NAMES.iter().enumerate() {
            f.add_field_method_get(*name, move |_, this| Ok(this.0 & (1 << i) != 0));
        }
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(bit_names(this.0, &FACE_NAMES))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LFaces| Ok(*this == o));
    }
}

impl UserData for LAxes {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Axes");
        for (i, name) in AXIS_NAMES.iter().enumerate() {
            f.add_field_method_get(*name, move |_, this| Ok(this.0 & (1 << i) != 0));
        }
        // Face aliases: Right/Left ↔ X, Top/Bottom ↔ Y, Back/Front ↔ Z.
        for (name, i) in [
            ("Right", 0),
            ("Left", 0),
            ("Top", 1),
            ("Bottom", 1),
            ("Back", 2),
            ("Front", 2),
        ] {
            f.add_field_method_get(name, move |_, this| Ok(this.0 & (1 << i) != 0));
        }
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(bit_names(this.0, &AXIS_NAMES))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: LAxes| Ok(*this == o));
    }
}

// ----------------------------------------- RaycastParams / OverlapParams

/// Shared state of RaycastParams and OverlapParams, readable by the runtime
/// through [`raycast_params`] / [`overlap_params`].
#[derive(Debug, Clone)]
pub struct QueryParams {
    /// Whatever the script assigned (normally Instance userdata).
    pub filter: Vec<Value>,
    /// Enum.RaycastFilterType item.
    pub filter_type: LEnumItem,
    pub ignore_water: bool,
    pub collision_group: String,
    pub respect_can_collide: bool,
    pub brute_force_all_slow: bool,
    /// OverlapParams only; 0 = unlimited.
    pub max_parts: i32,
}

impl Default for QueryParams {
    fn default() -> Self {
        QueryParams {
            filter: Vec::new(),
            filter_type: default_filter_type(),
            ignore_water: false,
            collision_group: "Default".into(),
            respect_can_collide: false,
            brute_force_all_slow: false,
            max_parts: 0,
        }
    }
}

impl QueryParams {
    /// True for Include (formerly Whitelist), false for Exclude/Blacklist.
    #[allow(dead_code)]
    pub fn is_include(&self) -> bool {
        matches!(self.filter_type.name.as_str(), "Include" | "Whitelist")
    }
}

fn default_filter_type() -> LEnumItem {
    LEnumItem::from_name("RaycastFilterType", "Exclude")
        .or_else(|| LEnumItem::from_name("RaycastFilterType", "Blacklist"))
        .unwrap_or(LEnumItem {
            enum_type: "RaycastFilterType".into(),
            name: "Exclude".into(),
            value: 0,
        })
}

trait HasQuery: Sized + 'static {
    const NAME: &'static str;
    const OVERLAP: bool;
    fn q(&self) -> &QueryParams;
    fn q_mut(&mut self) -> &mut QueryParams;
}

#[derive(Debug, Clone, Default)]
pub struct LRaycastParams(pub QueryParams);
from_ud!(LRaycastParams, "RaycastParams");

#[derive(Debug, Clone, Default)]
pub struct LOverlapParams(pub QueryParams);
from_ud!(LOverlapParams, "OverlapParams");

impl HasQuery for LRaycastParams {
    const NAME: &'static str = "RaycastParams";
    const OVERLAP: bool = false;
    fn q(&self) -> &QueryParams {
        &self.0
    }
    fn q_mut(&mut self) -> &mut QueryParams {
        &mut self.0
    }
}

impl HasQuery for LOverlapParams {
    const NAME: &'static str = "OverlapParams";
    const OVERLAP: bool = true;
    fn q(&self) -> &QueryParams {
        &self.0
    }
    fn q_mut(&mut self) -> &mut QueryParams {
        &mut self.0
    }
}

fn query_fields<T: HasQuery, F: UserDataFields<T>>(f: &mut F) {
    f.add_meta_field("__type", T::NAME);
    f.add_field_method_get("FilterDescendantsInstances", |lua, this| {
        lua.create_sequence_from(this.q().filter.iter().cloned())
    });
    f.add_field_method_set("FilterDescendantsInstances", |_, this, t: Table| {
        this.q_mut().filter = t.sequence_values::<Value>().collect::<LResult<Vec<_>>>()?;
        Ok(())
    });
    f.add_field_method_get("FilterType", |_, this| Ok(this.q().filter_type.clone()));
    f.add_field_method_set("FilterType", |_, this, v: Value| {
        this.q_mut().filter_type = enum_arg(Some(&v), "RaycastFilterType", "Exclude")?;
        Ok(())
    });
    f.add_field_method_get("IgnoreWater", |_, this| Ok(this.q().ignore_water));
    f.add_field_method_set("IgnoreWater", |_, this, v: bool| {
        this.q_mut().ignore_water = v;
        Ok(())
    });
    f.add_field_method_get("CollisionGroup", |_, this| {
        Ok(this.q().collision_group.clone())
    });
    f.add_field_method_set("CollisionGroup", |_, this, v: String| {
        this.q_mut().collision_group = v;
        Ok(())
    });
    f.add_field_method_get("RespectCanCollide", |_, this| {
        Ok(this.q().respect_can_collide)
    });
    f.add_field_method_set("RespectCanCollide", |_, this, v: bool| {
        this.q_mut().respect_can_collide = v;
        Ok(())
    });
    f.add_field_method_get("BruteForceAllSlow", |_, this| {
        Ok(this.q().brute_force_all_slow)
    });
    f.add_field_method_set("BruteForceAllSlow", |_, this, v: bool| {
        this.q_mut().brute_force_all_slow = v;
        Ok(())
    });
    if T::OVERLAP {
        f.add_field_method_get("MaxParts", |_, this| Ok(this.q().max_parts as f64));
        f.add_field_method_set("MaxParts", |_, this, v: f64| {
            this.q_mut().max_parts = v.max(0.0) as i32;
            Ok(())
        });
    }
}

fn query_methods<T: HasQuery, M: UserDataMethods<T>>(m: &mut M) {
    m.add_method_mut("AddToFilter", |_, this, v: Value| {
        match &v {
            Value::Table(t) => {
                for item in t.sequence_values::<Value>() {
                    this.q_mut().filter.push(item?);
                }
            }
            Value::Nil => {}
            _ => this.q_mut().filter.push(v),
        }
        Ok(())
    });
    // Studio's text.
    m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
        let q = this.q();
        let filter = q
            .filter
            .iter()
            .map(|v| v.to_string().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(", ");
        let group = if q.collision_group.is_empty() { "Default" } else { q.collision_group.as_str() };
        Ok(if T::OVERLAP {
            format!(
                "OverlapParams{{MaxParts={}, Tolerance=0, BruteForceAllSlow={}, RespectCanCollide={}, CollisionGroup={group}, FilterDescendantsInstances={{{filter}}}}}",
                q.max_parts, q.brute_force_all_slow, q.respect_can_collide
            )
        } else {
            format!(
                "RaycastParams{{IgnoreWater={}, BruteForceAllSlow={}, RespectCanCollide={}, CollisionGroup={group}, FilterDescendantsInstances={{{filter}}}}}",
                q.ignore_water, q.brute_force_all_slow, q.respect_can_collide
            )
        })
    });
    // Studio: two fresh params are ==.
    m.add_meta_function(MetaMethod::Eq, |_, (a, b): (mlua::AnyUserData, mlua::AnyUserData)| {
        let (Ok(a), Ok(b)) = (a.borrow::<T>(), b.borrow::<T>()) else { return Ok(false) };
        let (a, b) = (a.q(), b.q());
        Ok(a.filter == b.filter
            && a.filter_type == b.filter_type
            && a.ignore_water == b.ignore_water
            && a.collision_group == b.collision_group
            && a.respect_can_collide == b.respect_can_collide
            && a.brute_force_all_slow == b.brute_force_all_slow
            && a.max_parts == b.max_parts)
    });
}

impl UserData for LRaycastParams {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        query_fields(f);
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        query_methods(m);
    }
}

impl UserData for LOverlapParams {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        query_fields(f);
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        query_methods(m);
    }
}

/// The RaycastParams behind a Lua value (a snapshot), for WorldRoot:Raycast.
#[allow(dead_code)]
pub fn raycast_params(v: &Value) -> Option<QueryParams> {
    ud::<LRaycastParams>(v).map(|p| p.0)
}

/// The OverlapParams behind a Lua value (a snapshot), for GetPartBoundsInBox &co.
#[allow(dead_code)]
pub fn overlap_params(v: &Value) -> Option<QueryParams> {
    ud::<LOverlapParams>(v).map(|p| p.0)
}

// ----------------------------------------------------------- RaycastResult

/// WorldRoot:Raycast's answer: read-only userdata as in Studio
///, `==` by content.
#[derive(Clone)]
pub struct LRaycastResult {
    pub instance: Value,
    /// The hit instance's name when the ray was cast (tostring).
    pub name: String,
    pub position: Vec3,
    pub normal: Vec3,
    pub distance: f32,
    pub material: Option<LEnumItem>,
}

impl UserData for LRaycastResult {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "RaycastResult");
        f.add_field_method_get("Instance", |_, r| Ok(r.instance.clone()));
        f.add_field_method_get("Position", |_, r| Ok(LVector3(r.position)));
        f.add_field_method_get("Normal", |_, r| Ok(LVector3(r.normal)));
        f.add_field_method_get("Distance", |_, r| Ok(r.distance as f64));
        f.add_field_method_get("Material", |lua, r| match &r.material {
            Some(m) => enum_value(lua, m.clone()),
            None => Ok(Value::Nil),
        });
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, r, ()| {
            let v = |p: Vec3| format!("{}, {}, {}", fmt_g(p.x as f64, 9), fmt_g(p.y as f64, 9), fmt_g(p.z as f64, 9));
            Ok(format!(
                "RaycastResult{{{} @ {}; normal = {}; material = {}}}",
                r.name,
                v(r.position),
                v(r.normal),
                r.material.as_ref().map(|m| m.name.as_str()).unwrap_or("")
            ))
        });
        m.add_meta_function(MetaMethod::Eq, |_, (a, b): (Value, Value)| {
            let get = |v: &Value| match v {
                Value::UserData(u) => u.borrow::<LRaycastResult>().ok().map(|r| r.clone()),
                _ => None,
            };
            Ok(match (get(&a), get(&b)) {
                (Some(x), Some(y)) => {
                    x.instance == y.instance
                        && x.position == y.position
                        && x.normal == y.normal
                        && x.distance == y.distance
                        && x.material == y.material
                }
                _ => false,
            })
        });
    }
}

impl LRaycastResult {
    /// A throwaway value (for building the type's metatable).
    pub fn sample() -> Self {
        LRaycastResult {
            instance: Value::Nil,
            name: String::new(),
            position: Vec3::ZERO,
            normal: Vec3::Y,
            distance: 0.0,
            material: None,
        }
    }
}

// ----------------------------------------------------- CatalogSearchParams

/// A plain mutable property bag with the documented defaults.
#[derive(Debug, Clone)]
pub struct LCatalogSearchParams {
    pub props: Vec<(&'static str, Value)>,
}

impl UserData for LCatalogSearchParams {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "CatalogSearchParams");
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Index, |_, this, key: String| {
            this.props
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| {
                    rt_err(format!(
                        "'{key}' is not a valid member of CatalogSearchParams"
                    ))
                })
        });
        m.add_meta_method_mut(
            MetaMethod::NewIndex,
            |_, this, (key, v): (String, Value)| {
                let slot = this
                    .props
                    .iter_mut()
                    .find(|(k, _)| *k == key)
                    .ok_or_else(|| {
                        rt_err(format!(
                            "{key} is not a valid member of CatalogSearchParams"
                        ))
                    })?;
                slot.1 = v;
                Ok(())
            },
        );
        // CatalogSearchParams{SearchKeyword=, CategoryFilter=None, …}: names
        // for enums, lists joined, in Studio's order.
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let text = |v: &Value| -> String {
                match v {
                    Value::Table(t) => t
                        .clone()
                        .sequence_values::<Value>()
                        .filter_map(|x| x.ok())
                        .map(|x| match ud::<LEnumItem>(&x) {
                            Some(e) => e.name,
                            None => x.to_string().unwrap_or_default(),
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                    Value::Boolean(b) => b.to_string(),
                    other => match ud::<LEnumItem>(other) {
                        Some(e) => e.name,
                        None => other.to_string().unwrap_or_default(),
                    },
                }
            };
            // (the prop is IncludeOffSaleItems; Studio's text says IncludeOffSale)
            let get = |k: &str| {
                let k = if k == "IncludeOffSale" { "IncludeOffSaleItems" } else { k };
                this.props.iter().find(|(n, _)| *n == k).map(|(_, v)| text(v)).unwrap_or_default()
            };
            let keys = [
                "SearchKeyword", "CategoryFilter", "SortType", "SortAggregation", "SalesTypeFilter",
                "CreatorName", "Limit", "CreatorType", "BundleTypes", "AssetTypes", "IncludeOffSale",
            ];
            let body = keys.iter().map(|k| format!("{k}={}", get(k))).collect::<Vec<_>>().join(", ");
            Ok(format!("CatalogSearchParams{{{body}}}"))
        });
        m.add_meta_function(MetaMethod::Eq, |_, (a, b): (mlua::AnyUserData, mlua::AnyUserData)| {
            let (Ok(a), Ok(b)) = (a.borrow::<LCatalogSearchParams>(), b.borrow::<LCatalogSearchParams>()) else {
                return Ok(false);
            };
            // Fresh params hold fresh (empty) tables: compare those by contents.
            let same = |x: &Value, y: &Value| match (x, y) {
                (Value::Table(p), Value::Table(q)) => {
                    p.raw_len() == q.raw_len()
                        && p.clone().sequence_values::<Value>().zip(q.clone().sequence_values::<Value>()).all(
                            |(u, v)| matches!((u, v), (Ok(u), Ok(v)) if u == v),
                        )
                }
                _ => x == y,
            };
            Ok(a.props.len() == b.props.len()
                && a.props.iter().zip(b.props.iter()).all(|((k, x), (l, y))| k == l && same(x, y)))
        });
    }
}

fn catalog_search_params(lua: &Lua) -> LResult<LCatalogSearchParams> {
    let e = |ty: &str, name: &str| -> LResult<Value> {
        match LEnumItem::from_name(ty, name) {
            Some(item) => Ok(enum_value(lua, item)?),
            None => Ok(Value::Nil),
        }
    };
    Ok(LCatalogSearchParams {
        props: vec![
            ("SearchKeyword", Value::String(lua.create_string("")?)),
            ("MinPrice", Value::Number(0.0)),
            ("MaxPrice", Value::Number(2_147_483_647.0)),
            ("SortType", e("CatalogSortType", "Relevance")?),
            ("SortAggregation", e("CatalogSortAggregation", "AllTime")?),
            ("CategoryFilter", e("CatalogCategoryFilter", "None")?),
            ("SalesTypeFilter", e("SalesTypeFilter", "All")?),
            ("BundleTypes", Value::Table(lua.create_table()?)),
            ("AssetTypes", Value::Table(lua.create_table()?)),
            ("IncludeOffSaleItems", Value::Boolean(false)),
            ("CreatorName", Value::String(lua.create_string("")?)),
            ("CreatorType", e("CreatorTypeFilter", "All")?),
            ("CreatorId", Value::Number(0.0)),
            ("Limit", Value::Number(30.0)),
        ],
    })
}

// ------------------------------------------------------ Variant conversion

fn rv3(v: Vec3) -> rbx_types::Vector3 {
    rbx_types::Vector3::new(v.x, v.y, v.z)
}

fn rv3i(v: LVector3int16) -> rbx_types::Vector3int16 {
    rbx_types::Vector3int16::new(v.0[0], v.0[1], v.0[2])
}

fn gv3(v: rbx_types::Vector3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn lv3i(v: rbx_types::Vector3int16) -> LVector3int16 {
    LVector3int16([v.x, v.y, v.z])
}

/// Lua value → property Variant for the datatypes above (NumberSequence, ColorSequence, NumberRange, Rect, PhysicalProperties (as Variant::PhysicalProperties(CustomPhysicalProperties)), Vector3int16, Vector2int16, Region3int16, Faces, Axes). None when `v` isn't one of them.
/// (Ray and Region3 are converted too.)
#[allow(dead_code)]
pub fn datatype_to_variant(v: &mlua::Value) -> Option<rbx_types::Variant> {
    use rbx_types::Variant as V;
    if let Some(p) = v3_opt(v) {
        return Some(V::Vector3(rbx_types::Vector3::new(p.0.x, p.0.y, p.0.z)));
    }
    let Value::UserData(u) = v else { return None };
    if let Ok(x) = u.borrow::<LNumberSequence>() {
        let keypoints = x
            .keypoints
            .iter()
            .map(|k| rbx_types::NumberSequenceKeypoint::new(k.time, k.value, k.envelope))
            .collect();
        return Some(V::NumberSequence(rbx_types::NumberSequence { keypoints }));
    }
    if let Ok(x) = u.borrow::<LColorSequence>() {
        let keypoints = x
            .keypoints
            .iter()
            .map(|k| {
                rbx_types::ColorSequenceKeypoint::new(
                    k.time,
                    rbx_types::Color3::new(k.value[0], k.value[1], k.value[2]),
                )
            })
            .collect();
        return Some(V::ColorSequence(rbx_types::ColorSequence { keypoints }));
    }
    if let Ok(x) = u.borrow::<LNumberRange>() {
        return Some(V::NumberRange(rbx_types::NumberRange::new(x.min, x.max)));
    }
    if let Ok(x) = u.borrow::<LRect>() {
        return Some(V::Rect(rbx_types::Rect::new(
            rbx_types::Vector2::new(x.min.x, x.min.y),
            rbx_types::Vector2::new(x.max.x, x.max.y),
        )));
    }
    if let Ok(x) = u.borrow::<LPhysicalProperties>() {
        if x.default {
            return Some(V::PhysicalProperties(rbx_types::PhysicalProperties::Default));
        }
        let c = rbx_types::CustomPhysicalProperties::new(
            x.density,
            x.friction,
            x.elasticity,
            x.friction_weight,
            x.elasticity_weight,
            x.acoustic_absorption,
        );
        return Some(V::PhysicalProperties(
            rbx_types::PhysicalProperties::Custom(c),
        ));
    }
    if let Ok(x) = u.borrow::<LVector3int16>() {
        return Some(V::Vector3int16(rv3i(*x)));
    }
    if let Ok(x) = u.borrow::<LVector2int16>() {
        return Some(V::Vector2int16(rbx_types::Vector2int16::new(
            x.0[0], x.0[1],
        )));
    }
    if let Ok(x) = u.borrow::<LRegion3int16>() {
        return Some(V::Region3int16(rbx_types::Region3int16::new(
            rv3i(x.min),
            rv3i(x.max),
        )));
    }
    if let Ok(x) = u.borrow::<LFaces>() {
        return rbx_types::Faces::from_bits(x.0).map(V::Faces);
    }
    if let Ok(x) = u.borrow::<LAxes>() {
        return rbx_types::Axes::from_bits(x.0).map(V::Axes);
    }
    if let Ok(x) = u.borrow::<LRay>() {
        return Some(V::Ray(rbx_types::Ray::new(rv3(x.origin), rv3(x.direction))));
    }
    if let Ok(x) = u.borrow::<LRegion3>() {
        return Some(V::Region3(rbx_types::Region3::new(rv3(x.min), rv3(x.max))));
    }
    None
}

/// Property Variant → Lua value for the same types (None for other variants).
/// `PhysicalProperties::Default` maps to `Some(nil)`, like CustomPhysicalProperties.
#[allow(dead_code)]
pub fn variant_to_datatype(
    lua: &mlua::Lua,
    v: &rbx_types::Variant,
) -> mlua::Result<Option<mlua::Value>> {
    use rbx_types::Variant as V;
    fn u<T: UserData + 'static>(lua: &Lua, x: T) -> LResult<Option<Value>> {
        Ok(Some(Value::UserData(lua.create_userdata(x)?)))
    }
    match v {
        V::NumberSequence(s) => u(
            lua,
            LNumberSequence {
                keypoints: s
                    .keypoints
                    .iter()
                    .map(|k| LNumberSequenceKeypoint {
                        time: k.time,
                        value: k.value,
                        envelope: k.envelope,
                    })
                    .collect(),
            },
        ),
        V::ColorSequence(s) => u(
            lua,
            LColorSequence {
                keypoints: s
                    .keypoints
                    .iter()
                    .map(|k| LColorSequenceKeypoint {
                        time: k.time,
                        value: [k.color.r, k.color.g, k.color.b],
                    })
                    .collect(),
            },
        ),
        V::NumberRange(r) => u(
            lua,
            LNumberRange {
                min: r.min,
                max: r.max,
            },
        ),
        V::Rect(r) => u(
            lua,
            LRect {
                min: Vec2::new(r.min.x, r.min.y),
                max: Vec2::new(r.max.x, r.max.y),
            },
        ),
        V::PhysicalProperties(rbx_types::PhysicalProperties::Custom(c)) => u(
            lua,
            LPhysicalProperties {
                density: c.density(),
                friction: c.friction(),
                elasticity: c.elasticity(),
                friction_weight: c.friction_weight(),
                elasticity_weight: c.elasticity_weight(),
                acoustic_absorption: c.acoustic_absorption(),
                default: false,
            },
        ),
        V::PhysicalProperties(_) => Ok(Some(Value::Nil)),
        V::Vector3int16(x) => u(lua, lv3i(*x)),
        V::Vector2int16(x) => u(lua, LVector2int16([x.x, x.y])),
        V::Region3int16(r) => u(
            lua,
            LRegion3int16 {
                min: lv3i(r.min),
                max: lv3i(r.max),
            },
        ),
        V::Faces(x) => u(lua, LFaces(x.bits())),
        V::Axes(x) => u(lua, LAxes(x.bits())),
        V::Ray(r) => u(
            lua,
            LRay {
                origin: gv3(r.origin),
                direction: gv3(r.direction),
            },
        ),
        V::Region3(r) => u(
            lua,
            LRegion3 {
                min: gv3(r.min),
                max: gv3(r.max),
            },
        ),
        _ => Ok(None),
    }
}

// ------------------------------------------------------------ constructors

fn install_datatypes(lua: &Lua, g: &Table) -> LResult<()> {
    let ray = lua.create_table()?;
    ray.set(
        "new",
        lua.create_function(|_, (o, d): (Option<LVector3>, Option<LVector3>)| {
            Ok(LRay {
                origin: o.map(|v| v.0).unwrap_or(Vec3::ZERO),
                direction: d.map(|v| v.0).unwrap_or(Vec3::ZERO),
            })
        })?,
    )?;
    g.set("Ray", ray)?;

    let r3 = lua.create_table()?;
    r3.set(
        "new",
        lua.create_function(|_, (a, b): (Option<LVector3>, Option<LVector3>)| {
            let (a, b) = (
                a.map(|v| v.0).unwrap_or(Vec3::ZERO),
                b.map(|v| v.0).unwrap_or(Vec3::ZERO),
            );
            Ok(LRegion3 {
                min: a.min(b),
                max: a.max(b),
            })
        })?,
    )?;
    g.set("Region3", r3)?;

    let r3i = lua.create_table()?;
    r3i.set(
        "new",
        lua.create_function(
            |_, (a, b): (Option<LVector3int16>, Option<LVector3int16>)| {
                let z = LVector3int16([0; 3]);
                Ok(LRegion3int16 {
                    min: a.unwrap_or(z),
                    max: b.unwrap_or(z),
                })
            },
        )?,
    )?;
    g.set("Region3int16", r3i)?;

    let v3i = lua.create_table()?;
    v3i.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            Ok(LVector3int16(
                [0, 1, 2].map(|i| to_i16(a.get(i).and_then(num).unwrap_or(0.0))),
            ))
        })?,
    )?;
    g.set("Vector3int16", v3i)?;

    let v2i = lua.create_table()?;
    v2i.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            Ok(LVector2int16(
                [0, 1].map(|i| to_i16(a.get(i).and_then(num).unwrap_or(0.0))),
            ))
        })?,
    )?;
    g.set("Vector2int16", v2i)?;

    let nr = lua.create_table()?;
    nr.set(
        "new",
        lua.create_function(|_, (lo, hi): (f32, Option<f32>)| {
            let hi = hi.unwrap_or(lo);
            if hi < lo {
                return Err(rt_err("NumberRange: invalid range (max must be >= min)"));
            }
            Ok(LNumberRange { min: lo, max: hi })
        })?,
    )?;
    g.set("NumberRange", nr)?;

    let nsk = lua.create_table()?;
    nsk.set(
        "new",
        lua.create_function(|_, (t, v, e): (f32, f32, Option<f32>)| {
            Ok(LNumberSequenceKeypoint {
                time: t,
                value: v,
                envelope: e.unwrap_or(0.0),
            })
        })?,
    )?;
    g.set("NumberSequenceKeypoint", nsk)?;

    let fck = lua.create_table()?;
    fck.set(
        "new",
        lua.create_function(|_, (t, v, mode): (f32, f32, Option<Value>)| {
            Ok(LFloatCurveKey {
                time: t,
                value: v,
                interpolation: enum_arg(mode.as_ref(), "KeyInterpolationMode", "Cubic")?,
                left_tangent: None,
                right_tangent: None,
            })
        })?,
    )?;
    g.set("FloatCurveKey", fck)?;

    let ns = lua.create_table()?;
    ns.set("new", lua.create_function(|_, a: mlua::Variadic<Value>| {
        let kp = |time, value| LNumberSequenceKeypoint { time, value, envelope: 0.0 };
        let keypoints = match (a.first(), a.get(1)) {
            (Some(Value::Table(t)), _) => keypoint_list::<LNumberSequenceKeypoint>(t, "NumberSequenceKeypoint")?,
            (Some(x), None) if num(x).is_some() => vec![kp(0.0, arg_f32(&a, 0)), kp(1.0, arg_f32(&a, 0))],
            (Some(x), Some(y)) if num(x).is_some() && num(y).is_some() => vec![kp(0.0, arg_f32(&a, 0)), kp(1.0, arg_f32(&a, 1))],
            _ => return Err(rt_err("NumberSequence.new(): table of NumberSequenceKeypoints expected.")),
        };
        check_keypoints("NumberSequence", &keypoints.iter().map(|k| k.time).collect::<Vec<_>>())?;
        Ok(LNumberSequence { keypoints })
    })?)?;
    g.set("NumberSequence", ns)?;

    let csk = lua.create_table()?;
    csk.set(
        "new",
        lua.create_function(|_, (t, c): (f32, LColor3)| {
            Ok(LColorSequenceKeypoint {
                time: t,
                value: c.0,
            })
        })?,
    )?;
    g.set("ColorSequenceKeypoint", csk)?;

    let cs = lua.create_table()?;
    cs.set("new", lua.create_function(|_, a: mlua::Variadic<Value>| {
        let kp = |time, c: LColor3| LColorSequenceKeypoint { time, value: c.0 };
        let keypoints = match (a.first(), a.first().and_then(ud::<LColor3>)) {
            (Some(Value::Table(t)), _) => keypoint_list::<LColorSequenceKeypoint>(t, "ColorSequenceKeypoint")?,
            (_, Some(c0)) => {
                let c1 = a.get(1).and_then(ud::<LColor3>).unwrap_or(c0);
                vec![kp(0.0, c0), kp(1.0, c1)]
            }
            _ => return Err(rt_err("ColorSequence.new(): table expected.")),
        };
        check_keypoints("ColorSequence", &keypoints.iter().map(|k| k.time).collect::<Vec<_>>())?;
        Ok(LColorSequence { keypoints })
    })?)?;
    g.set("ColorSequence", cs)?;

    let rect = lua.create_table()?;
    rect.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            if !matches!(a.len(), 0 | 2 | 4) {
                return Err(rt_err(format!("Invalid number of arguments: {}", a.len())));
            }
            match a.first().and_then(ud::<LVector2>) {
                Some(lo) => Ok(LRect {
                    min: lo.0,
                    max: a
                        .get(1)
                        .and_then(ud::<LVector2>)
                        .map(|v| v.0)
                        .unwrap_or(Vec2::ZERO),
                }),
                None => Ok(LRect {
                    min: Vec2::new(arg_f32(&a, 0), arg_f32(&a, 1)),
                    max: Vec2::new(arg_f32(&a, 2), arg_f32(&a, 3)),
                }),
            }
        })?,
    )?;
    g.set("Rect", rect)?;

    let pp = lua.create_table()?;
    pp.set("new", lua.create_function(|_, a: mlua::Variadic<Value>| {
        if let Some(m) = a.first().and_then(ud::<LEnumItem>) {
            if m.enum_type != "Material" {
                return Err(rt_err(format!("Enum.Material expected, got Enum.{}", m.enum_type)));
            }
            return Ok(LPhysicalProperties::for_material(&m.name));
        }
        // Studio casts a lone non-number to Enum.Material: a Material's name
        // gives its properties, anything else the "default" value.
        if let Some(Value::String(s)) = a.first().filter(|v| num(v).is_none()) {
            let name = s.to_string_lossy();
            if LEnumItem::from_name("Material", &name).is_some() {
                return Ok(LPhysicalProperties::for_material(&name));
            }
            return Ok(LPhysicalProperties { default: true, ..LPhysicalProperties::for_material("Plastic") });
        }
        if a.len() < 3 || a.iter().take(3).any(|v| num(v).is_none()) {
            return Err(rt_err("PhysicalProperties.new expects a Material or (density, friction, elasticity[, frictionWeight, elasticityWeight[, acousticAbsorption]])"));
        }
        let opt = |i: usize, d: f32| a.get(i).and_then(num).map(|x| x as f32).unwrap_or(d);
        Ok(LPhysicalProperties {
            density: arg_f32(&a, 0),
            friction: arg_f32(&a, 1),
            elasticity: arg_f32(&a, 2),
            friction_weight: opt(3, 1.0),
            elasticity_weight: opt(4, 1.0),
            acoustic_absorption: opt(5, DEFAULT_ACOUSTIC_ABSORPTION),
            default: false,
        }
        .clamped())
    })?)?;
    g.set("PhysicalProperties", pp)?;

    let ti = lua.create_table()?;
    ti.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            if a.first().is_some_and(|v| !v.is_nil() && num(v).is_none()) {
                return Err(rt_err("TweenInfo.new first argument expects a number for time."));
            }
            let n = |i: usize, d: f64| a.get(i).and_then(num).unwrap_or(d);
            Ok(LTweenInfo {
                time: n(0, 1.0),
                easing_style: enum_arg(a.get(1), "EasingStyle", "Quad")?,
                easing_direction: enum_arg(a.get(2), "EasingDirection", "Out")?,
                repeat_count: n(3, 0.0) as i32,
                reverses: matches!(a.get(4), Some(Value::Boolean(true))),
                delay_time: n(5, 0.0),
            })
        })?,
    )?;
    g.set("TweenInfo", ti)?;

    let rnd = lua.create_table()?;
    rnd.set(
        "new",
        lua.create_function(|_, seed: Option<f64>| {
            Ok(match seed {
                Some(s) if s.is_finite() => LRandom::new(s.trunc() as i64 as u64),
                _ => LRandom::from_entropy(),
            })
        })?,
    )?;
    g.set("Random", rnd)?;

    let dt = lua.create_table()?;
    dt.set("now", lua.create_function(|_, ()| Ok(LDateTime::now()))?)?;
    dt.set(
        "fromUnixTimestamp",
        lua.create_function(|_, s: f64| {
            Ok(LDateTime {
                millis: (s * 1000.0).floor() as i64,
            })
        })?,
    )?;
    dt.set(
        "fromUnixTimestampMillis",
        lua.create_function(|_, ms: f64| {
            Ok(LDateTime {
                millis: ms.floor() as i64,
            })
        })?,
    )?;
    let from_parts = lua.create_function(|_, a: mlua::Variadic<Value>| {
        let p = |i: usize, d: f64| a.get(i).and_then(num).unwrap_or(d).floor() as i64;
        LDateTime::from_universal(
            p(0, 1970.0),
            p(1, 1.0),
            p(2, 1.0),
            p(3, 0.0),
            p(4, 0.0),
            p(5, 0.0),
            p(6, 0.0),
        )
        .ok_or_else(|| rt_err("DateTime: invalid date"))
    })?;
    dt.set("fromUniversalTime", from_parts.clone())?;
    dt.set("fromLocalTime", from_parts)?;
    dt.set(
        "fromIsoDate",
        lua.create_function(|_, s: String| Ok(LDateTime::parse_iso(&s)))?,
    )?;
    g.set("DateTime", dt)?;

    let faces = lua.create_table()?;
    faces.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            let mut bits = 0u8;
            for v in a.iter() {
                // Studio ignores anything else (Faces.new("a") is empty).
                let Some(e) = ud::<LEnumItem>(v) else { continue };
                if let Some(i) = FACE_NAMES.iter().position(|n| *n == e.name) {
                    bits |= 1 << i;
                }
            }
            Ok(LFaces(bits))
        })?,
    )?;
    g.set("Faces", faces)?;

    let axes = lua.create_table()?;
    axes.set(
        "new",
        lua.create_function(|_, a: mlua::Variadic<Value>| {
            let mut bits = 0u8;
            for v in a.iter() {
                let Some(e) = ud::<LEnumItem>(v) else { continue };
                let i = match e.name.as_str() {
                    "X" | "Right" | "Left" => 0,
                    "Y" | "Top" | "Bottom" => 1,
                    "Z" | "Back" | "Front" => 2,
                    _ => continue,
                };
                bits |= 1 << i;
            }
            Ok(LAxes(bits))
        })?,
    )?;
    g.set("Axes", axes)?;

    let rp = lua.create_table()?;
    rp.set(
        "new",
        lua.create_function(|_, ()| Ok(LRaycastParams::default()))?,
    )?;
    g.set("RaycastParams", rp)?;
    let op = lua.create_table()?;
    op.set(
        "new",
        lua.create_function(|_, ()| Ok(LOverlapParams::default()))?,
    )?;
    g.set("OverlapParams", op)?;
    let csp = lua.create_table()?;
    csp.set(
        "new",
        lua.create_function(|lua, ()| catalog_search_params(lua))?,
    )?;
    g.set("CatalogSearchParams", csp)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> Lua {
        let lua = Lua::new();
        install(&lua, &lua.globals()).expect("install");
        lua
    }

    fn eval<T: FromLua>(lua: &Lua, src: &str) -> T {
        lua.load(src)
            .eval()
            .unwrap_or_else(|e| panic!("{src}\n=> {e}"))
    }

    /// Studio's answers (offline Studio, round 19), as GetComponents' 9 rotation numbers.
    #[test]
    fn look_at_vertical_like_studio() {
        let lua = lua();
        let cases: [(&str, [f64; 9]); 5] = [
            ("CFrame.lookAt(Vector3.zero, Vector3.new(0, -1, 0))", [0., 1., 0., 0., 0., 1., 1., 0., 0.]),
            ("CFrame.lookAt(Vector3.zero, Vector3.new(0, 1, 0))", [0., 1., 0., 0., 0., -1., -1., 0., 0.]),
            ("CFrame.lookAt(Vector3.zero, Vector3.new(0, -1, -0.00001))", [1., 0., 0., 0., 1e-5, 1., 0., -1., 1e-5]),
            ("CFrame.lookAt(Vector3.zero, Vector3.new(0, -1, 0), Vector3.xAxis)", [0., 1., 0., 0., 0., 1., 1., 0., 0.]),
            ("CFrame.new(Vector3.zero, Vector3.new(0, -1, -0.00001))", [0., 1., 0., -1e-5, 0., 1., 1., 0., 1e-5]),
        ];
        for (src, want) in cases {
            let got: Vec<f64> = eval(&lua, &format!("{{select(4, ({src}):GetComponents())}}"));
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() < 1e-4, "{src}: {got:?} vs Studio {want:?}");
            }
        }
    }

    #[test]
    fn ltypes_typeof_all() {
        let lua = lua();
        let cases = [
            ("Vector3.new()", "Vector3"),
            ("Vector2.new()", "Vector2"),
            ("CFrame.new()", "CFrame"),
            ("Color3.new()", "Color3"),
            ("BrickColor.new(1)", "BrickColor"),
            ("UDim.new()", "UDim"),
            ("UDim2.new()", "UDim2"),
            ("Enum.Material.Plastic", "EnumItem"),
            ("Enum.Material", "Enum"),
            ("Enum", "Enums"),
            ("Ray.new(Vector3.new(), Vector3.new(0,0,1))", "Ray"),
            ("Region3.new(Vector3.new(), Vector3.one)", "Region3"),
            (
                "Region3int16.new(Vector3int16.new(), Vector3int16.new(1,1,1))",
                "Region3int16",
            ),
            ("Vector3int16.new(1,2,3)", "Vector3int16"),
            ("Vector2int16.new(1,2)", "Vector2int16"),
            ("NumberRange.new(1)", "NumberRange"),
            ("NumberSequenceKeypoint.new(0, 1)", "NumberSequenceKeypoint"),
            ("NumberSequence.new(1)", "NumberSequence"),
            (
                "ColorSequenceKeypoint.new(0, Color3.new())",
                "ColorSequenceKeypoint",
            ),
            ("ColorSequence.new(Color3.new())", "ColorSequence"),
            ("Rect.new(0,0,1,1)", "Rect"),
            (
                "PhysicalProperties.new(Enum.Material.Wood)",
                "PhysicalProperties",
            ),
            ("TweenInfo.new()", "TweenInfo"),
            ("Random.new(1)", "Random"),
            ("DateTime.now()", "DateTime"),
            ("Faces.new()", "Faces"),
            ("Axes.new()", "Axes"),
            ("RaycastParams.new()", "RaycastParams"),
            ("OverlapParams.new()", "OverlapParams"),
            ("CatalogSearchParams.new()", "CatalogSearchParams"),
        ];
        for (expr, ty) in cases {
            let got: String = eval(&lua, &format!("return typeof({expr})"));
            assert_eq!(got, ty, "typeof({expr})");
        }
    }

    #[test]
    fn ltypes_cframe_and_vector_extras() {
        let lua = lua();
        let ok: bool = eval(
            &lua,
            r#"
            local a = CFrame.fromEulerAnglesXYZ(0.3, 0.5, 0.7)
            local b = CFrame.fromEulerAngles(0.3, 0.5, 0.7, Enum.RotationOrder.XYZ)
            local c = CFrame.fromEulerAngles(0.3, 0.5, 0.7)
            local d = CFrame.fromEulerAnglesYXZ(0.3, 0.5, 0.7)
            local e = CFrame.fromEulerAngles(0.3, 0.5, 0.7, Enum.RotationOrder.YXZ)
            local x, y, z = d:ToEulerAnglesYXZ()
            local x2, y2, z2 = a:ToEulerAngles(Enum.RotationOrder.XYZ)
            local m = CFrame.fromMatrix(Vector3.new(1,2,3), Vector3.xAxis, Vector3.yAxis)
            local la = CFrame.lookAlong(Vector3.new(), Vector3.new(0,0,-5))
            local o = CFrame.new(0,0,0, 2,0,0, 0,1,0, 0,0,1):Orthonormalize()
            local _h, _s, v = Color3.toHSV(Color3.new(0.5, 0.25, 0))
            return a:FuzzyEq(b) and a:FuzzyEq(c) and d:FuzzyEq(e)
                and math.abs(x - 0.3) < 1e-4 and math.abs(y - 0.5) < 1e-4 and math.abs(z - 0.7) < 1e-4
                and math.abs(x2 - 0.3) < 1e-4 and math.abs(y2 - 0.5) < 1e-4 and math.abs(z2 - 0.7) < 1e-4
                and m.Position == Vector3.new(1,2,3) and m.ZVector == Vector3.zAxis
                and la.LookVector:FuzzyEq(Vector3.new(0,0,-1))
                and o.RightVector:FuzzyEq(Vector3.xAxis)
                and Vector3.FromAxis(Enum.Axis.Y) == Vector3.yAxis
                and math.abs(v - 0.5) < 1e-6
                and typeof(BrickColor.random()) == "BrickColor"
            "#,
        );
        assert!(ok);
    }

    #[test]
    fn ltypes_basic_constructors() {
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local r = Ray.new(Vector3.new(0,0,0), Vector3.new(0,0,10))
            local out = {}
            table.insert(out, tostring(r.Unit.Direction))
            table.insert(out, tostring(r:ClosestPoint(Vector3.new(3,0,4))))
            table.insert(out, tostring(r:Distance(Vector3.new(3,0,4))))
            local reg = Region3.new(Vector3.new(0,0,0), Vector3.new(2,4,6))
            table.insert(out, tostring(reg.CFrame.Position) .. "|" .. tostring(reg.Size))
            local g = Region3.new(Vector3.new(0.5,0.5,0.5), Vector3.new(3.5,3.5,3.5)):ExpandToGrid(4)
            table.insert(out, tostring(g.Size))
            local v = Vector3int16.new(1,2,3) + Vector3int16.new(1,1,1)
            table.insert(out, tostring(v * 2) .. "|" .. tostring(v / 2) .. "|" .. v.Z)
            table.insert(out, tostring(Vector2int16.new(5, -6) - Vector2int16.new(1, 1)))
            table.insert(out, tostring(Region3int16.new(Vector3int16.new(0,0,0), Vector3int16.new(1,2,3)).Max))
            local rc = Rect.new(Vector2.new(1,2), Vector2.new(4,6))
            table.insert(out, rc.Width .. "," .. rc.Height .. "|" .. tostring(Rect.new(0,0,3,3)))
            local nr = NumberRange.new(2)
            table.insert(out, nr.Min .. "," .. nr.Max .. "|" .. tostring(NumberRange.new(1, 5)))
            table.insert(out, tostring(pcall(NumberRange.new, 5, 1)))
            return table.concat(out, ";")
            "#,
        );
        assert_eq!(s, "0, 0, 1;0, 0, 4;3;1, 2, 3|2, 4, 6;4, 4, 4;4, 6, 8|1, 1, 2|4;4, -7;1, 2, 3;3,4|0, 0, 3, 3;2,2|1 5 ;false");
    }

    #[test]
    fn ltypes_sequences() {
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local out = {}
            local a = NumberSequence.new(0.5)
            table.insert(out, tostring(a))
            local b = NumberSequence.new({NumberSequenceKeypoint.new(0, 1, 0.5), NumberSequenceKeypoint.new(0.5, 2), NumberSequenceKeypoint.new(1, 3)})
            table.insert(out, #b.Keypoints .. " " .. b.Keypoints[1].Envelope .. " " .. b.Keypoints[2].Value)
            table.insert(out, tostring(pcall(NumberSequence.new, {NumberSequenceKeypoint.new(0.2, 1), NumberSequenceKeypoint.new(1, 1)})))
            table.insert(out, tostring(pcall(NumberSequence.new, {NumberSequenceKeypoint.new(0, 1), NumberSequenceKeypoint.new(0.5, 1)})))
            local c = ColorSequence.new(Color3.new(1, 0, 0), Color3.new(0, 0, 1))
            table.insert(out, tostring(c.Keypoints[2].Value) .. " " .. typeof(c.Keypoints[1]))
            table.insert(out, tostring(NumberSequence.new(1, 2) == NumberSequence.new(1, 2)))
            return table.concat(out, ";")
            "#,
        );
        assert_eq!(
            s,
            "0 0.5 0 1 0.5 0 ;3 0.5 2;false;false;0, 0, 1 ColorSequenceKeypoint;true"
        );
    }

    #[test]
    fn ltypes_physical_and_tween() {
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local p = PhysicalProperties.new(Enum.Material.Ice)
            local q = PhysicalProperties.new(1, 0.5, 0.25)
            local t = TweenInfo.new()
            local u = TweenInfo.new(2, Enum.EasingStyle.Linear, Enum.EasingDirection.InOut, 3, true, 0.5)
            return table.concat({
                string.format("%.3f %.2f %.2f %g", p.Density, p.Friction, p.Elasticity, p.FrictionWeight),
                tostring(q), q.ElasticityWeight,
                t.Time .. " " .. tostring(t.EasingStyle) .. " " .. tostring(t.EasingDirection) .. " " .. t.RepeatCount .. " " .. tostring(t.Reverses) .. " " .. t.DelayTime,
                tostring(t.EasingStyle == Enum.EasingStyle.Quad),
                tostring(u),
            }, ";")
            "#,
        );
        assert_eq!(
            s,
            "0.919 0.02 0.15 3;1, 0.5, 0.25, 1, 1, 1;1;1 Enum.EasingStyle.Quad Enum.EasingDirection.Out 0 false 0;true;\
             Time:2 DelayTime:0.5 RepeatCount:3 Reverses:True EasingDirection:InOut EasingStyle:Linear"
        );
    }

    #[test]
    fn ltypes_random_matches_studio() {
        // Studio's own numbers (tools/fidelity place_c_terrain.luau FIDRNG).
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local out = {}
            for _, seed in { 0, 1001, 123456789 } do
                local r = Random.new(seed)
                table.insert(out, string.format("%.17g,%.17g,%d,%d", r:NextNumber(), r:NextNumber(0, 1000), r:NextInteger(1, 1000000), r:NextInteger(-5, 5)))
            end
            return table.concat(out, ";")
            "#,
        );
        assert_eq!(
            s,
            "0.55145569047490739,17.231024361130142,334180,4;\
             0.11421535570982778,692.03661140012446,905151,-1;\
             0.16504106969308055,130.56711073615963,871626,4"
        );
    }

    /// GetEnumItems hands out the same EnumItem objects as Enum.X.Y, so
    /// they work as table keys (round 26: an admin script's mock DataStore).
    #[test]
    fn ltypes_enum_items_are_unique() {
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local t = {}
            for _, e in Enum.DataStoreRequestType:GetEnumItems() do t[e] = e.Name end
            return tostring(t[Enum.DataStoreRequestType.SetIncrementAsync]) .. " " .. tostring(rawequal(Enum.KeyCode.W, Enum.KeyCode:GetEnumItems()[table.find(Enum.KeyCode:GetEnumItems(), Enum.KeyCode.W)]))
            "#,
        );
        assert_eq!(s, "SetIncrementAsync true");
    }

    /// FloatCurveKey.new (round 26: Place D's second admin script haptics).
    #[test]
    fn ltypes_float_curve_key() {
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local k = FloatCurveKey.new(1, 0.5, Enum.KeyInterpolationMode.Constant)
            local c = FloatCurveKey.new(0, 2)
            c.RightTangent = 3
            return table.concat({typeof(k), k.Time, k.Value, k.Interpolation.Name,
                tostring(k.LeftTangent), c.Interpolation.Name, c.RightTangent}, " ")
            "#,
        );
        assert_eq!(s, "FloatCurveKey 1 0.5 Constant nil Cubic 3");
    }

    #[test]
    fn ltypes_random_deterministic() {
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local a, b = Random.new(42), Random.new(42)
            for _ = 1, 100 do
                local x, y = a:NextNumber(), b:NextNumber()
                assert(x == y and x >= 0 and x <= 1)
                local i, j = a:NextInteger(-3, 3), b:NextInteger(-3, 3)
                assert(i == j and i >= -3 and i <= 3 and i == math.floor(i))
                local n = a:NextNumber(5, 6); b:NextNumber(5, 6)
                assert(n >= 5 and n <= 6)
            end
            local c = a:Clone()
            assert(c:NextInteger(1, 1000000) == a:NextInteger(1, 1000000))
            local v = a:NextUnitVector()
            assert(math.abs(v.Magnitude - 1) < 1e-4)
            local t = {1, 2, 3, 4, 5, 6, 7, 8}
            Random.new(7):Shuffle(t)
            local t2 = {1, 2, 3, 4, 5, 6, 7, 8}
            Random.new(7):Shuffle(t2)
            local sum = 0
            for i = 1, 8 do sum += t[i]; assert(t[i] == t2[i]) end
            assert(sum == 36)
            assert(not pcall(function() a:NextInteger(5, 1) end))
            return tostring(Random.new(1):NextInteger(1, 1e9) == Random.new(1):NextInteger(1, 1e9))
                .. tostring(Random.new(1):NextNumber() ~= Random.new(2):NextNumber())
            "#,
        );
        assert_eq!(s, "truetrue");
    }

    #[test]
    fn ltypes_datetime() {
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local d = DateTime.fromUniversalTime(2000, 1, 1)
            local out = { d.UnixTimestamp, d:ToIsoDate() }
            local e = DateTime.fromUnixTimestamp(946684800)
            table.insert(out, tostring(e == d))
            local t = DateTime.fromUnixTimestampMillis(1583188245123):ToUniversalTime()
            table.insert(out, string.format("%d-%d-%d %d:%d:%d.%d", t.Year, t.Month, t.Day, t.Hour, t.Minute, t.Second, t.Millisecond))
            local f = DateTime.fromUniversalTime(2020, 1, 2, 10, 30, 45)
            table.insert(out, f:ToIsoDate())
            table.insert(out, DateTime.fromIsoDate("2020-01-02T10:30:45Z").UnixTimestamp == f.UnixTimestamp and "iso-ok" or "iso-bad")
            table.insert(out, tostring(DateTime.fromIsoDate("2020-01-02T12:30:45+02:00") == f))
            table.insert(out, tostring(DateTime.fromIsoDate("not a date")))
            table.insert(out, f:FormatUniversalTime("dddd, MMMM D YYYY HH:mm:ss A [at] LT", "en-us"))
            table.insert(out, tostring(pcall(DateTime.fromUniversalTime, 2021, 2, 29)))
            table.insert(out, DateTime.fromUniversalTime(2024, 2, 29, 25):ToIsoDate())
            table.insert(out, DateTime.fromUniversalTime(1969, 12, 31, 23, 59, 59, 500).UnixTimestampMillis)
            table.insert(out, tostring(DateTime.fromUnixTimestamp(0) < DateTime.now()))
            return table.concat(out, ";")
            "#,
        );
        assert_eq!(
            s,
            "946684800;2000-01-01T00:00:00Z;true;2020-3-2 22:30:45.123;2020-01-02T10:30:45Z;iso-ok;true;nil;\
             Thursday, January 2 2020 10:30:45 AM at 10:30 AM;false;2024-03-01T01:00:00Z;-500;true"
        );
        // Calendar maths round trip across a wide range.
        for days in (-800_000i64..800_000).step_by(997) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn ltypes_faces_axes_params() {
        let lua = lua();
        let s: String = eval(
            &lua,
            r#"
            local f = Faces.new(Enum.NormalId.Top, Enum.NormalId.Front)
            local a = Axes.new(Enum.Axis.X, Enum.NormalId.Top)
            local rp = RaycastParams.new()
            local out = { tostring(f), tostring(f.Top), tostring(f.Left), tostring(a), tostring(a.Y), tostring(a.Z) }
            table.insert(out, tostring(rp.FilterType) .. " " .. #rp.FilterDescendantsInstances .. " " .. tostring(rp.IgnoreWater) .. " " .. rp.CollisionGroup)
            rp.FilterDescendantsInstances = {1, 2}
            rp.FilterType = Enum.RaycastFilterType.Include
            rp.IgnoreWater = true
            rp:AddToFilter({3})
            rp:AddToFilter(4)
            table.insert(out, tostring(rp.FilterType) .. " " .. #rp.FilterDescendantsInstances .. " " .. tostring(rp.IgnoreWater))
            local op = OverlapParams.new()
            op.MaxParts = 5
            table.insert(out, op.MaxParts .. " " .. tostring(pcall(function() rp.MaxParts = 1 end)))
            local c = CatalogSearchParams.new()
            c.SearchKeyword = "hat"
            table.insert(out, c.SearchKeyword .. " " .. c.Limit)
            return table.concat(out, ";")
            "#,
        );
        assert_eq!(s, "Top, Front;true;false;X, Y;true;false;Enum.RaycastFilterType.Exclude 0 false Default;Enum.RaycastFilterType.Include 4 true;5 false;hat 30");
    }

    #[test]
    fn ltypes_param_accessor() {
        let lua = lua();
        let v: Value = eval(&lua, "local p = RaycastParams.new(); p.FilterType = Enum.RaycastFilterType.Include; p.CollisionGroup = 'G'; return p");
        let q = raycast_params(&v).expect("params");
        assert!(q.is_include());
        assert_eq!(q.collision_group, "G");
        assert!(overlap_params(&v).is_none());
    }

    #[test]
    fn ltypes_variant_round_trips() {
        let lua = lua();
        let exprs = [
            "NumberSequence.new({NumberSequenceKeypoint.new(0, 1, 0.5), NumberSequenceKeypoint.new(0.4, 2), NumberSequenceKeypoint.new(1, 3)})",
            "ColorSequence.new({ColorSequenceKeypoint.new(0, Color3.new(1, 0, 0)), ColorSequenceKeypoint.new(1, Color3.new(0, 1, 0.5))})",
            "NumberRange.new(1, 4)",
            "Rect.new(1, 2, 3, 4)",
            "PhysicalProperties.new(2, 0.4, 0.3, 2, 3)",
            "Vector3int16.new(1, -2, 3)",
            "Vector2int16.new(7, 8)",
            "Region3int16.new(Vector3int16.new(0, 1, 2), Vector3int16.new(3, 4, 5))",
            "Faces.new(Enum.NormalId.Left, Enum.NormalId.Back)",
            "Axes.new(Enum.Axis.Z)",
            "Ray.new(Vector3.new(1, 2, 3), Vector3.new(0, -1, 0))",
            "Region3.new(Vector3.new(0, 0, 0), Vector3.new(1, 2, 3))",
        ];
        let check: mlua::Function = eval(
            &lua,
            "return function(a, b) return a == b and tostring(a) == tostring(b) end",
        );
        for expr in exprs {
            let v: Value = eval(&lua, &format!("return {expr}"));
            let variant =
                datatype_to_variant(&v).unwrap_or_else(|| panic!("no variant for {expr}"));
            let back = variant_to_datatype(&lua, &variant)
                .expect("convert")
                .expect("some");
            assert_eq!(datatype_to_variant(&back), Some(variant), "{expr}");
            assert!(check.call::<bool>((v, back)).expect("call"), "{expr}");
        }
        assert!(datatype_to_variant(&Value::Number(1.0)).is_none());
        let none = variant_to_datatype(&lua, &rbx_types::Variant::Bool(true)).expect("ok");
        assert!(none.is_none());
        let nil = variant_to_datatype(
            &lua,
            &rbx_types::Variant::PhysicalProperties(rbx_types::PhysicalProperties::Default),
        )
        .expect("ok");
        assert!(matches!(nil, Some(Value::Nil)));
    }
}

// ------------------------------------------------------------------ Font

/// Roblox `Font`: a font family (asset path of a family JSON) plus weight and
/// style. Text isn't drawn yet; scripts can build, read and assign fonts.
#[derive(Debug, Clone, PartialEq)]
pub struct LFont(pub rbx_types::Font);

fn font_weight_item(w: rbx_types::FontWeight) -> Option<LEnumItem> {
    LEnumItem::from_value("FontWeight", w.as_u16() as u32)
}

fn font_style_item(s: rbx_types::FontStyle) -> Option<LEnumItem> {
    LEnumItem::from_value("FontStyle", s.as_u8() as u32)
}

fn font_from_parts(
    family: String,
    weight: Option<&Value>,
    style: Option<&Value>,
) -> LResult<LFont> {
    let weight = match weight.and_then(|v| ud::<LEnumItem>(v)) {
        Some(e) => rbx_types::FontWeight::from_u16(e.value as u16).unwrap_or_default(),
        None => rbx_types::FontWeight::default(),
    };
    let style = match style.and_then(|v| ud::<LEnumItem>(v)) {
        Some(e) => rbx_types::FontStyle::from_u8(e.value as u8).unwrap_or_default(),
        None => rbx_types::FontStyle::default(),
    };
    Ok(LFont(rbx_types::Font {
        family,
        weight,
        style,
        cached_face_id: None,
    }))
}

impl UserData for LFont {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Font");
        f.add_field_method_get("Family", |_, this| Ok(this.0.family.clone()));
        f.add_field_method_get("Weight", |lua, this| {
            match font_weight_item(this.0.weight) {
                Some(i) => Ok(enum_value(lua, i)?),
                None => Ok(Value::Nil),
            }
        });
        f.add_field_method_get("Style", |lua, this| match font_style_item(this.0.style) {
            Some(i) => Ok(enum_value(lua, i)?),
            None => Ok(Value::Nil),
        });
        f.add_field_method_get("Bold", |_, this| Ok(this.0.weight.as_u16() >= 600));
        // Font is mutable in Studio.
        f.add_field_method_set("Family", |_, this, v: String| {
            this.0.family = v;
            this.0.cached_face_id = None;
            Ok(())
        });
        f.add_field_method_set("Weight", |_, this, v: LEnumItem| {
            this.0.weight = rbx_types::FontWeight::from_u16(v.value as u16).unwrap_or_default();
            Ok(())
        });
        f.add_field_method_set("Style", |_, this, v: LEnumItem| {
            this.0.style = rbx_types::FontStyle::from_u8(v.value as u8).unwrap_or_default();
            Ok(())
        });
        f.add_field_method_set("Bold", |_, this, v: bool| {
            let w = if v { 700 } else { 400 };
            this.0.weight = rbx_types::FontWeight::from_u16(w).unwrap_or_default();
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "Font {{ Family = {}, Weight = {:?}, Style = {:?} }}",
                this.0.family, this.0.weight, this.0.style
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::AnyUserData| {
            Ok(other.borrow::<LFont>().is_ok_and(|o| *o == *this))
        });
    }
}

/// Installs the `Font` global.
/// Roblox's InputObject: a userdata (typeof gives "InputObject") whose
/// fields live in a Lua table and whose methods come from the library.
pub struct LInputObject {
    data: Table,
    methods: Table,
}

impl UserData for LInputObject {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "InputObject");
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::Index, |_, this, key: Value| {
            let v: Value = this.data.raw_get(key.clone())?;
            if !v.is_nil() {
                return Ok(v);
            }
            let f: Value = this.methods.raw_get(key.clone())?;
            if !f.is_nil() {
                return Ok(f);
            }
            Err(mlua::Error::RuntimeError(format!(
                "{} is not a valid member of InputObject \"InputObject\"",
                key.to_string().unwrap_or_default()
            )))
        });
        m.add_meta_method(
            mlua::MetaMethod::NewIndex,
            |_, this, (k, v): (Value, Value)| this.data.raw_set(k, v),
        );
        m.add_meta_method(mlua::MetaMethod::ToString, |_, _, ()| Ok("InputObject"));
    }
}

/// Roblox's RBXScriptSignal: a userdata (type "userdata", typeof
/// "RBXScriptSignal") over the prelude's signal table, its methods from the
/// prelude's Signal. Round 26: as tables, libraries took signals for plain
/// tables (an admin script's Wrap copied their fields and lost Connect; another admin script
/// Admin's Flux assigned to FocusLost).
pub struct LSignal {
    data: Table,
    methods: Table,
}

impl LSignal {
    /// The event's state table (shared by every object for one event).
    pub fn data(&self) -> &Table {
        &self.data
    }
    pub fn methods(&self) -> &Table {
        &self.methods
    }
    pub fn new(data: Table, methods: Table) -> LSignal {
        LSignal { data, methods }
    }
}

const SIGNAL: crate::cmeta::Spec =
    crate::cmeta::Spec { self_types: &[c"RBXScriptSignal"], key_first: true };

impl UserData for LSignal {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "RBXScriptSignal");
        // C functions checking the key, then self. A
        // method is a new named C function on every read, as Studio's
        // (`sig.Connect ~= sig.Connect`), over the prelude's Lua one.
        f.add_meta_field_with("__index", |lua| {
            crate::cmeta::cached(lua, "rs_signal_index", |lua| {
                let inner = lua.create_function(|lua, (this, key): (mlua::AnyUserData, String)| {
                    let this = this.borrow::<LSignal>()?;
                    let v: Value = this.data.raw_get(key.as_str())?;
                    if !v.is_nil() {
                        return Ok(v);
                    }
                    if let Value::Function(f) = this.methods.raw_get::<Value>(key.as_str())? {
                        // ConnectParallel checks its Actor rule before self
                        //.
                        let self_type = (key != "ConnectParallel").then_some("RBXScriptSignal");
                        return crate::cmeta::method(lua, &key, None, Some(f), self_type)
                            .map(Value::Function);
                    }
                    Err(rt_err(format!("{key} is not a valid member of RBXScriptSignal")))
                })?;
                crate::cmeta::wrap_meta(lua, inner, false, &SIGNAL)
            })
        });
        f.add_meta_field_with("__newindex", |lua| {
            crate::cmeta::cached(lua, "rs_signal_newindex", |lua| {
                let inner = lua.create_function(|_, (_, key): (Value, String)| {
                    Err::<(), _>(rt_err(format!("{key} is not a valid member of RBXScriptSignal")))
                })?;
                crate::cmeta::wrap_meta(lua, inner, false, &SIGNAL)
            })
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        // Every read of an event is a new object (Studio probes
        // RBXScriptSignal.values: `a == b`, not rawequal); equal when they
        // are the same event.
        m.add_meta_function(mlua::MetaMethod::Eq, |_, (a, b): (Value, Value)| {
            let data = |v: &Value| match v {
                Value::UserData(u) => u.borrow::<LSignal>().ok().map(|s| s.data.clone()),
                _ => None,
            };
            Ok(match (data(&a), data(&b)) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            })
        });
        m.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            let name: Option<String> = this.data.raw_get("__name")?;
            Ok(format!("Signal {}", name.as_deref().unwrap_or("Event")))
        });
    }
}

pub fn install_font(lua: &Lua, g: &Table) -> LResult<()> {
    g.set(
        "__rs_make_input_object",
        lua.create_function(|_, (data, methods): (Table, Table)| {
            Ok(LInputObject { data, methods })
        })?,
    )?;
    g.set(
        "__rs_make_signal",
        lua.create_function(|_, (data, methods): (Table, Table)| Ok(LSignal { data, methods }))?,
    )?;
    let t = lua.create_table()?;
    t.set(
        "new",
        lua.create_function(
            |_, (family, weight, style): (String, Option<Value>, Option<Value>)| {
                font_from_parts(family, weight.as_ref(), style.as_ref())
            },
        )?,
    )?;
    t.set(
        "fromName",
        lua.create_function(
            |_, (name, weight, style): (String, Option<Value>, Option<Value>)| {
                font_from_parts(
                    format!("rbxasset://fonts/families/{name}.json"),
                    weight.as_ref(),
                    style.as_ref(),
                )
            },
        )?,
    )?;
    t.set(
        "fromId",
        lua.create_function(
            |_, (id, weight, style): (f64, Option<Value>, Option<Value>)| {
                font_from_parts(
                    format!("rbxassetid://{}", id as u64),
                    weight.as_ref(),
                    style.as_ref(),
                )
            },
        )?,
    )?;
    t.set(
        "fromEnum",
        lua.create_function(|_, e: Value| {
            // Enum.Font items name their family (Enum.Font.Oswald → Oswald.json).
            let name = ud::<LEnumItem>(&e)
                .map(|i| i.name)
                .unwrap_or_else(|| "SourceSansPro".into());
            let (family, bold) = match name.strip_suffix("Bold") {
                Some(base) if !base.is_empty() => (base.to_string(), true),
                _ => (name, false),
            };
            let mut f = font_from_parts(
                format!("rbxasset://fonts/families/{family}.json"),
                None,
                None,
            )?;
            if bold {
                f.0.weight = rbx_types::FontWeight::Bold;
            }
            Ok(f)
        })?,
    )?;
    g.set("Font", t)?;
    Ok(())
}

/// Locks the Enum types' metatables (cmeta::lock).
pub fn lock_enums(lua: &Lua) -> LResult<()> {
    crate::cmeta::lock(lua, &lua.create_userdata(LEnums)?)?;
    crate::cmeta::lock(lua, &lua.create_userdata(LEnum(String::new()))?)?;
    crate::cmeta::lock(
        lua,
        &lua.create_userdata(LEnumItem { enum_type: String::new(), name: String::new(), value: 0 })?,
    )
}

/// The one Lua object for an EnumItem (Roblox EnumItems are unique, so
/// `Enum.KeyCode.W == Enum.KeyCode.W` and they work as table keys).
pub fn enum_value(lua: &Lua, item: LEnumItem) -> LResult<Value> {
    let cache: Table = match lua.named_registry_value::<Option<Table>>("rs_enum_items")? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            lua.set_named_registry_value("rs_enum_items", t.clone())?;
            t
        }
    };
    let key = format!("{}.{}", item.enum_type, item.value);
    if let Value::UserData(u) = cache.raw_get::<Value>(key.as_str())? {
        return Ok(Value::UserData(u));
    }
    let u = lua.create_userdata(item)?;
    cache.raw_set(key, u.clone())?;
    Ok(Value::UserData(u))
}

#[cfg(test)]
mod direct_call_tests {
    /// tools/parity_probe/datatypes/crash.txt: a script that captures the
    /// function raising a datatype operation (`debug.info(2, "f")`) and
    /// calls it with no arguments reached `lua_error` with an empty stack
    /// (mlua's internal `error`), a Luau API assert left on in release
    /// builds -> SIGILL. Every raiser of the probe corpus, called direct.
    #[test]
    fn captured_raisers_called_direct_do_not_crash() {
        let lua = mlua::Lua::new();
        super::install(&lua, &lua.globals()).unwrap();
        let n: i64 = lua
            .load(
                r#"
                local makers = {
                    function() return Vector3.new(1/3, -0, 1e20) end, function() return Vector2.new(1/3, -0) end,
                    function() return Vector3int16.new(1, 2, 3) end, function() return Vector2int16.new(1, 2) end,
                    function() return CFrame.new(1/3, 2, 3) end, function() return Color3.new(1/3, 0.5, 1) end,
                    function() return BrickColor.new(21) end, function() return UDim.new(1/3, 7) end,
                    function() return UDim2.new(1/3, 7, 0.5, -8) end, function() return Rect.new(1, 2, 3, 4) end,
                    function() return Ray.new(Vector3.new(1, 2, 3), Vector3.new(4, 5, 6)) end,
                    function() return Region3.new(Vector3.zero, Vector3.one) end,
                    function() return NumberRange.new(1/3, 2) end, function() return NumberSequence.new(0.5) end,
                    function() return NumberSequenceKeypoint.new(0, 0.5, 0.1) end,
                    function() return ColorSequence.new(Color3.new(1, 0, 0)) end,
                    function() return ColorSequenceKeypoint.new(0, Color3.new(1, 0, 0)) end,
                    function() return TweenInfo.new(1/3) end, function() return PhysicalProperties.new(1, 0.5, 0.5) end,
                    function() return Faces.new(Enum.NormalId.Top) end, function() return Axes.new(Enum.Axis.X) end,
                    function() return DateTime.fromUnixTimestamp(0) end, function() return Random.new(42) end,
                    function() return OverlapParams.new() end, function() return RaycastParams.new() end,
                    function() return CatalogSearchParams.new() end,
                }
                local ops = {
                    function(o) return o.____________ end, function(o) o.____________ = 1 end,
                    function(o) o.X = 1 end, function(o) return o:____________() end,
                    function(o) return o + 1 end, function(o) return o * Color3.new() end,
                    function(o) return o < o end, function(o) return o <= o end, function(o) return #o end,
                    function(o) return o .. "x" end, function(o) return -o end,
                    function(o) return o // 2 end, function(o) return o % 2 end,
                }
                local calls = 0
                for _, make in ipairs(makers) do
                    local o = make()
                    for _, op in ipairs(ops) do
                        local f
                        xpcall(function() return op(o) end, function() f = debug.info(2, "f") end)
                        if f then
                            pcall(f); pcall(f, {}, {}); pcall(f, o)
                            pcall(function() return f() end)
                            calls += 1
                        end
                    end
                end
                return calls
                "#,
            )
            .eval()
            .unwrap();
        assert!(n > 100, "{n} raisers captured");
    }
}
