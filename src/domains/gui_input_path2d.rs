//! Path2D's control points and curve queries, and the Path2DControlPoint
//! type, as measured in Studio (offline Studio, 2026-11-07; gui_input.md):
//!
//! - a segment joins two points; its shape follows the tangents: both zero
//!   is a straight line, one zero a quadratic Bézier through the other
//!   tangent's end, both set a cubic (P0, P0 + RightTangent, P1 +
//!   LeftTangent, P1); `t` spreads evenly over the segments;
//! - positions come back as a UDim2 of the parent's AbsoluteSize (offset,
//!   with a warning, when the parent has no area); tangents are pixels per
//!   unit of the segment's own t;
//! - GetLength subdivides each segment until a halving no longer lengthens
//!   it (a symmetric S curve whose midpoint is on its chord measures its
//!   chord: Studio's 141.42 for one that is 149.5 long), and the arc-length
//!   queries walk that many pixels along the true curve;
//! - GetBoundingRect is the curve's extent in screen pixels grown by
//!   ceil(Thickness / 2); with no segment it is ±FLT_MAX.

use mlua::{Lua, MetaMethod, MultiValue, ObjectLike, UserData, UserDataFields, UserDataMethods, Value};
use rbx_dom_weak::types::Ref;
use std::collections::HashMap;

use crate::ltypes::*;
use crate::runtime::*;
use glam::Vec2;

const MAX_POINTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LPath2DControlPoint {
    pub pos: LUDim2,
    pub left: LUDim2,
    pub right: LUDim2,
}

fn zero() -> LUDim2 {
    LUDim2 { x: LUDim { scale: 0.0, offset: 0 }, y: LUDim { scale: 0.0, offset: 0 } }
}

fn udim2_text(u: &LUDim2) -> String {
    let n = |x: f32| {
        if x == x.trunc() && x.abs() < 1e15 {
            format!("{}", x as i64)
        } else {
            format!("{x}")
        }
    };
    format!("{{{}, {}}}, {{{}, {}}}", n(u.x.scale), u.x.offset, n(u.y.scale), u.y.offset)
}

impl UserData for LPath2DControlPoint {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Path2DControlPoint");
        f.add_field_method_get("Position", |_, this| Ok(this.pos));
        f.add_field_method_get("LeftTangent", |_, this| Ok(this.left));
        f.add_field_method_get("RightTangent", |_, this| Ok(this.right));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "Path2DControlPoint {{ Position = {}, LeftTangent = {}, RightTangent = {} }}",
                udim2_text(&this.pos),
                udim2_text(&this.left),
                udim2_text(&this.right)
            ))
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, o: Value| {
            Ok(match o {
                Value::UserData(u) => u.borrow::<LPath2DControlPoint>().is_ok_and(|o| *o == *this),
                _ => false,
            })
        });
    }
}

fn cp_of(v: &Value) -> Option<LPath2DControlPoint> {
    match v {
        Value::UserData(u) => u.borrow::<LPath2DControlPoint>().ok().map(|c| *c),
        _ => None,
    }
}

fn udim2_of(v: &Value) -> Option<LUDim2> {
    match v {
        Value::UserData(u) => u.borrow::<LUDim2>().ok().map(|c| *c),
        _ => None,
    }
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.set(
        "new",
        lua.create_function(|_, args: MultiValue| {
            let mut v = [zero(); 3];
            for (i, a) in args.iter().take(3).enumerate() {
                match a {
                    Value::Nil => {}
                    a => {
                        v[i] = udim2_of(a).ok_or_else(|| {
                            rt_err(format!("invalid argument #{} to 'new' (UDim2 expected, got {})", i + 1, a.type_name()))
                        })?
                    }
                }
            }
            Ok(LPath2DControlPoint { pos: v[0], left: v[1], right: v[2] })
        })?,
    )?;
    lua.globals().set("Path2DControlPoint", t)?;
    lua.set_app_data(Paths::default());
    Ok(())
}

#[derive(Default)]
struct Paths(HashMap<Ref, Vec<LPath2DControlPoint>>);

fn points(lua: &Lua, r: Ref) -> Vec<LPath2DControlPoint> {
    lua.app_data_ref::<Paths>()
        .and_then(|p| p.0.get(&r).cloned())
        .unwrap_or_default()
}

fn set_points(lua: &Lua, r: Ref, v: Vec<LPath2DControlPoint>) -> mlua::Result<()> {
    if let Some(mut p) = lua.app_data_mut::<Paths>() {
        p.0.insert(r, v);
    }
    world(lua)?.fire(r, "ControlPointChanged", vec![]);
    Ok(())
}

#[derive(Clone, Copy)]
enum Seg {
    Line(Vec2, Vec2),
    Quad(Vec2, Vec2, Vec2),
    Cubic(Vec2, Vec2, Vec2, Vec2),
}

impl Seg {
    fn at(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        match *self {
            Seg::Line(a, b) => a + (b - a) * t,
            Seg::Quad(a, c, b) => a * u * u + c * 2.0 * u * t + b * t * t,
            Seg::Cubic(a, c1, c2, b) => a * u * u * u + c1 * 3.0 * u * u * t + c2 * 3.0 * u * t * t + b * t * t * t,
        }
    }

    fn tangent(&self, t: f32) -> Vec2 {
        let u = 1.0 - t;
        match *self {
            Seg::Line(a, b) => b - a,
            Seg::Quad(a, c, b) => (c - a) * 2.0 * u + (b - c) * 2.0 * t,
            Seg::Cubic(a, c1, c2, b) => {
                (c1 - a) * 3.0 * u * u + (c2 - c1) * 6.0 * u * t + (b - c2) * 3.0 * t * t
            }
        }
    }

    /// Halves until a split no longer lengthens a piece.
    fn length(&self) -> f32 {
        fn rec(s: &Seg, t0: f32, p0: Vec2, t1: f32, p1: Vec2, depth: u32) -> f32 {
            let tm = (t0 + t1) * 0.5;
            let pm = s.at(tm);
            let chord = p0.distance(p1);
            let split = p0.distance(pm) + pm.distance(p1);
            if depth >= 18 || split - chord <= 1e-4 * split.max(1.0) {
                return split;
            }
            rec(s, t0, p0, tm, pm, depth + 1) + rec(s, tm, pm, t1, p1, depth + 1)
        }
        rec(self, 0.0, self.at(0.0), 1.0, self.at(1.0), 0)
    }
}

/// The parent's AbsolutePosition and AbsoluteSize, or Studio's error.
fn parent_box(lua: &Lua, r: Ref) -> mlua::Result<(Vec2, Vec2)> {
    let parent = {
        let w = world(lua)?;
        let d = w.d();
        let p = d.dom.get_by_ref(r).map(|i| i.parent()).unwrap_or_else(Ref::none);
        let ok = d.class_of(p).is_some_and(|c| crate::doc::is_a(&c, "GuiBase2d"));
        ok.then_some(p)
    };
    let p = parent.ok_or_else(|| rt_err("Attempting to use Path2D with invalid parent"))?;
    let Value::UserData(u) = inst_value(lua, p)? else {
        return Err(rt_err("Attempting to use Path2D with invalid parent"));
    };
    let v2 = |k: &str| -> mlua::Result<Vec2> {
        match u.get::<Value>(k)? {
            Value::UserData(x) => Ok(x.borrow::<LVector2>().map(|v| v.0).unwrap_or(Vec2::ZERO)),
            _ => Ok(Vec2::ZERO),
        }
    };
    Ok((v2("AbsolutePosition")?, v2("AbsoluteSize")?))
}

fn resolve(u: &LUDim2, size: Vec2) -> Vec2 {
    Vec2::new(u.x.scale * size.x + u.x.offset as f32, u.y.scale * size.y + u.y.offset as f32)
}

fn segments(lua: &Lua, r: Ref, pts: &[LPath2DControlPoint], origin: Vec2, size: Vec2) -> Vec<Seg> {
    let closed = matches!(world(lua).ok().and_then(|w| w.d().prop(r, "Closed")), Some(Variant::Bool(true)));
    let n = pts.len();
    let mut out = Vec::new();
    let count = if closed && n > 2 { n } else { n.saturating_sub(1) };
    for i in 0..count {
        let a = &pts[i];
        let b = &pts[(i + 1) % n];
        let p0 = origin + resolve(&a.pos, size);
        let p1 = origin + resolve(&b.pos, size);
        let rt = resolve(&a.right, size);
        let lt = resolve(&b.left, size);
        out.push(match (rt == Vec2::ZERO, lt == Vec2::ZERO) {
            (true, true) => Seg::Line(p0, p1),
            (false, true) => Seg::Quad(p0, p0 + rt, p1),
            (true, false) => Seg::Quad(p0, p1 + lt, p1),
            (false, false) => Seg::Cubic(p0, p0 + rt, p1 + lt, p1),
        });
    }
    out
}

fn seg_count(lua: &Lua, r: Ref, n: usize) -> usize {
    let closed = matches!(world(lua).ok().and_then(|w| w.d().prop(r, "Closed")), Some(Variant::Bool(true)));
    if closed && n > 2 {
        n
    } else {
        n.saturating_sub(1)
    }
}

fn number(args: &MultiValue, i: usize, what: &str) -> mlua::Result<f64> {
    match args.get(i) {
        Some(Value::Number(n)) => Ok(*n),
        Some(Value::Integer(n)) => Ok(*n as f64),
        Some(Value::String(s)) => s
            .to_str()
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| rt_err(format!("Unable to cast string to {what}"))),
        Some(v) => Err(rt_err(format!("Unable to cast {} to {what}", v.type_name()))),
        None => Err(rt_err(format!("Argument {i} missing or nil"))),
    }
}

/// A point of the curve as Studio gives it: UDim2 of the parent's size.
fn as_udim2(lua: &Lua, p: Vec2, origin: Vec2, size: Vec2) -> mlua::Result<Value> {
    let rel = p - origin;
    let u = if size.x * size.y == 0.0 {
        world(lua)?
            .output
            .push((crate::app::Level::Warn, "Path2D parent rectangle has zero area.".into()));
        LUDim2 {
            x: LUDim { scale: 0.0, offset: rel.x.round() as i32 },
            y: LUDim { scale: 0.0, offset: rel.y.round() as i32 },
        }
    } else {
        LUDim2 {
            x: LUDim { scale: rel.x / size.x, offset: 0 },
            y: LUDim { scale: rel.y / size.y, offset: 0 },
        }
    };
    Ok(Value::UserData(lua.create_userdata(u)?))
}

/// The segment and its own t for a curve t (even over segments, clamped).
fn locate(t: f32, n: usize) -> (usize, f32) {
    let t = t.clamp(0.0, 1.0) * n as f32;
    let i = (t.floor() as usize).min(n - 1);
    (i, t - i as f32)
}

/// Segment and local t after walking `dist` pixels along the true curve.
fn walk(segs: &[Seg], dist: f32) -> (usize, f32) {
    const N: usize = 1024;
    let mut left = dist.max(0.0);
    for (i, s) in segs.iter().enumerate() {
        let mut prev = s.at(0.0);
        for k in 1..=N {
            let t = k as f32 / N as f32;
            let p = s.at(t);
            let d = prev.distance(p);
            if d >= left && d > 0.0 {
                return (i, (k as f32 - 1.0 + left / d) / N as f32);
            }
            left -= d;
            prev = p;
        }
    }
    (segs.len() - 1, 1.0)
}

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let one = |v: Value| Ok(MultiValue::from_iter([v]));
    let ud = |v: LUDim2| -> mlua::Result<Value> { Ok(Value::UserData(lua.create_userdata(v)?)) };
    let _ = ud;
    let pts = points(lua, this);
    let index = |i: usize, what: &str, n: usize| -> mlua::Result<usize> {
        let k = number(args, i, "int")? as i64;
        if k < 1 || k as usize > n {
            return Err(rt_err(format!("Path2D:{what}() at index {k} access out of bounds")));
        }
        Ok(k as usize - 1)
    };
    match name {
        "GetMaxControlPoints" => one(Value::Integer(MAX_POINTS as i64)),
        "GetControlPoints" => {
            let t = lua.create_table()?;
            for p in &pts {
                t.push(*p)?;
            }
            one(Value::Table(t))
        }
        "GetControlPoint" => {
            let i = index(1, "GetControlPoint", pts.len())?;
            one(Value::UserData(lua.create_userdata(pts[i])?))
        }
        "SetControlPoints" => {
            let Some(Value::Table(t)) = args.get(1) else {
                return Err(rt_err("Unable to cast value to Array"));
            };
            let mut v = Vec::new();
            for x in t.clone().sequence_values::<Value>() {
                v.push(cp_of(&x?).ok_or_else(|| rt_err("Variant cast failed"))?);
            }
            if v.len() > MAX_POINTS {
                v.truncate(MAX_POINTS);
                set_points(lua, this, v)?;
                return Err(rt_err("Path2D:SetControlPoints() only supports 100 control points."));
            }
            set_points(lua, this, v)?;
            Ok(MultiValue::new())
        }
        "InsertControlPoint" => {
            let k = number(args, 1, "int")? as i64;
            let p = args.get(2).and_then(cp_of).ok_or_else(|| rt_err("Variant cast failed"))?;
            if k < 1 || k as usize > pts.len() + 1 {
                return Err(rt_err(format!("Path2D:InsertControlPoint() at index {k} access out of bounds")));
            }
            if pts.len() >= MAX_POINTS {
                return Err(rt_err("Path2D:InsertControlPoint() only supports 100 control points."));
            }
            let mut v = pts;
            v.insert(k as usize - 1, p);
            set_points(lua, this, v)?;
            Ok(MultiValue::new())
        }
        "UpdateControlPoint" => {
            let i = index(1, "UpdateControlPoint", pts.len())?;
            let p = args.get(2).and_then(cp_of).ok_or_else(|| rt_err("Variant cast failed"))?;
            let mut v = pts;
            v[i] = p;
            set_points(lua, this, v)?;
            Ok(MultiValue::new())
        }
        "RemoveControlPoint" => {
            let i = index(1, "RemoveControlPoint", pts.len())?;
            let mut v = pts;
            v.remove(i);
            set_points(lua, this, v)?;
            Ok(MultiValue::new())
        }
        "GetLength" => {
            if seg_count(lua, this, pts.len()) == 0 {
                return one(Value::Number(0.0));
            }
            let (o, s) = parent_box(lua, this)?;
            let len: f32 = segments(lua, this, &pts, o, s).iter().map(Seg::length).sum();
            one(Value::Number(len as f64))
        }
        "GetBoundingRect" => {
            if seg_count(lua, this, pts.len()) == 0 {
                let m = f32::MAX;
                return one(Value::UserData(lua.create_userdata(LRect { min: Vec2::splat(-m), max: Vec2::splat(m) })?));
            }
            let (o, s) = parent_box(lua, this)?;
            let segs = segments(lua, this, &pts, o, s);
            let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
            for sg in &segs {
                for k in 0..=256 {
                    let p = sg.at(k as f32 / 256.0);
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
            let th = match world(lua)?.d().prop(this, "Thickness") {
                Some(Variant::Float32(t)) => t,
                _ => 1.0,
            };
            let pad = (th * 0.5).ceil();
            one(Value::UserData(lua.create_userdata(LRect { min: lo - pad, max: hi + pad })?))
        }
        "GetPositionOnCurve" | "GetTangentOnCurve" | "GetPositionOnCurveArcLength" | "GetTangentOnCurveArcLength" => {
            let t = number(args, 1, "float")? as f32;
            let n = seg_count(lua, this, pts.len());
            if n == 0 {
                return Err(rt_err(format!("Path2D:{name}() attempting to query with zero segments")));
            }
            let (o, s) = parent_box(lua, this)?;
            let segs = segments(lua, this, &pts, o, s);
            let (i, lt) = if name.ends_with("ArcLength") {
                let total: f32 = segs.iter().map(Seg::length).sum();
                walk(&segs, t.clamp(0.0, 1.0) * total)
            } else {
                locate(t, segs.len())
            };
            if name.starts_with("GetPosition") {
                one(as_udim2(lua, segs[i].at(lt), o, s)?)
            } else {
                one(Value::UserData(lua.create_userdata(LVector2(segs[i].tangent(lt)))?))
            }
        }
        _ => Err(rt_err(format!("{name} is not a valid member of Path2D"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path2d_lengths_as_studio() {
        // Studio: 141.42135 (midpoint on the chord) and 408.86673.
        let s = Seg::Cubic(Vec2::ZERO, Vec2::new(50.0, 0.0), Vec2::new(50.0, 100.0), Vec2::new(100.0, 100.0));
        assert!((s.length() - 141.42135).abs() < 1e-3, "{}", s.length());
        let l = Seg::Cubic(Vec2::ZERO, Vec2::new(300.0, 0.0), Vec2::new(100.0, 300.0), Vec2::new(100.0, 0.0));
        assert!((l.length() - 408.8667).abs() < 0.05, "{}", l.length());
        // Arc 1.0 of the first stops short of the end: (91.97, 99.07).
        let (i, t) = walk(&[s], s.length());
        let p = [s][i].at(t);
        assert!((p - Vec2::new(91.9652, 99.0735)).length() < 0.05, "{p}");
        // One zero tangent: a quadratic; (25, 18.75) at a quarter.
        let q = Seg::Quad(Vec2::ZERO, Vec2::new(50.0, 50.0), Vec2::new(100.0, 0.0));
        assert!((q.at(0.25) - Vec2::new(25.0, 18.75)).length() < 1e-4);
        assert!((q.tangent(0.0) - Vec2::new(100.0, 100.0)).length() < 1e-4);
    }
}
