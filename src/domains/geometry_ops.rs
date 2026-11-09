//! BasePart:UnionAsync / SubtractAsync / IntersectAsync, GeometryService's
//! CSG calls (UnionAsync, SubtractAsync, IntersectAsync, FragmentAsync,
//! GenerateFragmentSites, SweepPartAsync, BakeToMeshAsync,
//! CalculateConstraintsToPreserve, CreateBasicMeshPart),
//! PartOperation:SubstituteGeometry and MeshPart:ApplyMesh.
//!
//! As Studio (measured, checks/parity/geometry.md): results are unparented
//! UnionOperation "Union" / IntersectOperation "Intersection" with the main
//! part's properties, its rotation, a world box around the result (Size)
//! and UsePartColor false (each corner keeps its source part's colour);
//! NegateOperations in a union's list are subtracted; an empty
//! intersection is nil (BasePart) or {} (GeometryService); GeometryService
//! splits disconnected pieces (SplitApart) and checks CSGOptions keys.
//! GeometryService keeps the main part's CFrame, its geometry off-centre in
//! the Size box (mesh_bvh::union_offset places it for drawing, colliders and
//! queries); BasePart's calls centre the result on its geometry. Both keep
//! the main part's rotation.

use glam::{DVec3, Mat3, Vec3};
use mlua::{Lua, MultiValue, Value};
use rbx_dom_weak::types::{Enum, Ref, SharedString, Variant};
use rbx_dom_weak::{ustr, InstanceBuilder};

use super::csg::{self, Poly, Shape};
use crate::doc::{self, Document};
use crate::ltypes::*;
use crate::math::Xform;
use crate::runtime::*;

pub const METHODS: &[(&str, &str)] = &[
    ("CalculateConstraintsToPreserve", "GeometryService"),
    ("CreateBasicMeshPart", "GeometryService"),
    ("GenerateFragmentSites", "GeometryService"),
    ("SubstituteGeometry", "PartOperation"),
    ("ApplyMesh", "MeshPart"),
];

pub const LUA_METHODS: &[(&str, &str)] = &[
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

const UNKNOWN: &str = "CSG: One of the primitives was of unknown type (error -4)";
const MAX_TRIS: usize = 20000;

#[derive(Clone, Copy, PartialEq)]
enum Op {
    Union,
    Subtract,
    Intersect,
}

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

fn color_of(doc: &Document, r: Ref) -> [f32; 3] {
    match doc.prop(r, "Color") {
        Some(Variant::Color3uint8(c)) => [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0],
        Some(Variant::Color3(c)) => [c.r, c.g, c.b],
        Some(Variant::BrickColor(b)) => {
            let c = b.to_color3uint8();
            [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]
        }
        _ => [163.0 / 255.0, 162.0 / 255.0, 165.0 / 255.0],
    }
}

/// A part's CSG shape (Studio's error for anything else).
fn shape_of(doc: &Document, r: Ref) -> mlua::Result<Shape> {
    let class = doc.class_of(r).ok_or_else(|| rt_err(UNKNOWN))?;
    let class = class.as_str();
    if class == "WedgePart" {
        return Ok(Shape::Wedge);
    }
    if class == "CornerWedgePart" {
        return Ok(Shape::CornerWedge);
    }
    if doc::is_a(class, "Part") {
        return Ok(match crate::scene::enum_u32(doc.prop(r, "Shape")) {
            Some(0) => Shape::Ball,
            Some(2) => Shape::Cylinder,
            Some(3) => Shape::Wedge,
            Some(4) => Shape::CornerWedge,
            _ => Shape::Block,
        });
    }
    if class == "TrussPart" {
        return Ok(Shape::Block);
    }
    let mesh_id = if class == "MeshPart" {
        crate::scene::content_string(doc.prop(r, "MeshId")).or_else(|| crate::scene::content_string(doc.prop(r, "MeshContent")))
    } else if doc::is_a(class, "PartOperation") {
        crate::scene::union_mesh(doc, r).map(|m| m.id)
    } else {
        return Err(rt_err(UNKNOWN));
    };
    let Some(id) = mesh_id else {
        // An empty MeshPart is its box.
        return Ok(Shape::Block);
    };
    let off = crate::mesh_bvh::union_offset(&id);
    match crate::assets::mesh_triangles(&id) {
        Some(t) => Ok(Shape::Mesh(t, off)),
        None => Err(rt_err("CSG: Improperly formed or stored Union. Missing child data. (error -6)")),
    }
}

fn polys_of(doc: &Document, r: Ref) -> mlua::Result<(Vec<Poly>, Xform)> {
    let class = doc.class_of(r).ok_or_else(|| rt_err(UNKNOWN))?;
    if !doc::is_a(&class, "BasePart") || class == "Terrain" {
        return Err(rt_err(UNKNOWN));
    }
    let xf = crate::scene::xform_of(doc, r).ok_or_else(|| rt_err(UNKNOWN))?;
    let size = crate::scene::vec3_of(doc.prop(r, "Size")).unwrap_or(Vec3::ONE);
    let shape = shape_of(doc, r)?;
    Ok((csg::part_polys(&shape, &xf, size, color_of(doc, r)), xf))
}

/// Properties a result takes from the main part.
const COPIED: &[&str] = &[
    "Anchored", "Archivable", "CanCollide", "CanQuery", "CanTouch", "CastShadow", "CollisionGroup",
    "CollisionGroupId", "Color", "CustomPhysicalProperties", "EnableFluidForces", "Locked", "Massless",
    "Material", "MaterialVariant", "MaterialVariantSerialized", "Reflectance", "RootPriority", "Transparency",
    "BackSurface", "BottomSurface", "FrontSurface", "LeftSurface", "RightSurface", "TopSurface",
];

pub struct Piece {
    pub xf: Xform,
    pub size: Vec3,
    pub blob: Vec<u8>,
    pub tris: usize,
}

/// Lays a result's world triangles out in a frame with `rot`: its centre,
/// size and CSGMDL blob.
/// With `centred` false the result keeps `frame` (its CFrame) and the
/// geometry stays where it is in it (GeometryService).
pub fn piece_in(tris: &[([DVec3; 3], [f32; 3])], frame: Xform, centred: bool) -> mlua::Result<Piece> {
    let rot = frame.rot;
    let inv = rot.transpose().as_dmat3();
    let origin = if centred { DVec3::ZERO } else { frame.pos.as_dvec3() };
    let local: Vec<([DVec3; 3], [f32; 3])> = tris.iter().map(|(t, c)| (t.map(|p| inv * (p - origin)), *c)).collect();
    let mut lo = DVec3::splat(f64::MAX);
    let mut hi = DVec3::splat(f64::MIN);
    for (t, _) in &local {
        for p in t {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
    }
    let center = if centred { (lo + hi) * 0.5 } else { DVec3::ZERO };
    let out: Vec<([Vec3; 3], [f32; 3])> = local.iter().map(|(t, c)| (t.map(|p| (p - center).as_vec3()), *c)).collect();
    if out.len() > MAX_TRIS {
        return Err(rt_err("CSG: Resulting mesh after simplification has triangle count above the supported 20k limit! (error -14)"));
    }
    let blob = csg::csgmdl(&out).map_err(|_| rt_err("CSG: Resulting mesh after simplification has triangle count above the supported 20k limit! (error -14)"))?;
    Ok(Piece {
        xf: if centred { Xform { pos: (rot.as_dmat3() * center).as_vec3(), rot } } else { frame },
        size: (hi - lo).as_vec3().max(Vec3::splat(0.001)),
        blob,
        tris: out.len(),
    })
}

/// A result centred on its geometry, with rotation `rot`.
pub fn piece(tris: &[([DVec3; 3], [f32; 3])], rot: Mat3) -> mlua::Result<Piece> {
    piece_in(tris, Xform { pos: Vec3::ZERO, rot }, true)
}

pub struct Fid {
    pub collision: u32,
    pub render: u32,
}

/// Makes the result instance (unparented) from the main part.
fn make_result(lua: &Lua, main: Ref, class: &str, name: &str, p: &Piece, fid: &Fid) -> mlua::Result<Ref> {
    let mut w = world(lua)?;
    let d = w.dm();
    let mut b = InstanceBuilder::new(class).with_name(name);
    if let Some(inst) = d.dom.get_by_ref(main) {
        for k in COPIED {
            if let Some(v) = inst.properties.get(&ustr(k)) {
                b = b.with_property(*k, v.clone());
            }
        }
    }
    let (s, xf) = (p.size, p.xf);
    b = b
        .with_property("CFrame", Variant::CFrame(xf.to_cframe()))
        .with_property("Size", Variant::Vector3(rbx_dom_weak::types::Vector3::new(s.x, s.y, s.z)))
        .with_property("InitialSize", Variant::Vector3(rbx_dom_weak::types::Vector3::new(s.x, s.y, s.z)))
        .with_property("MeshData2", Variant::SharedString(SharedString::new(p.blob.clone())))
        .with_property("UsePartColor", Variant::Bool(false))
        .with_property("TriangleCount", Variant::Int32(p.tris as i32))
        .with_property("CollisionFidelity", Variant::Enum(Enum::from_u32(fid.collision)))
        .with_property("RenderFidelity", Variant::Enum(Enum::from_u32(fid.render)));
    Ok(d.dom.insert(Ref::none(), b))
}

/// Runs `op` of `main` with `others`: the result's world triangles, or
/// None when it's empty.
fn run_op(doc: &Document, op: Op, main: Ref, others: &[Ref]) -> mlua::Result<(Vec<([DVec3; 3], [f32; 3])>, Xform)> {
    let (mut acc, xf) = polys_of(doc, main)?;
    for r in others {
        if *r == main {
            continue;
        }
        let (p, _) = polys_of(doc, *r)?;
        let negate = doc.class_of(*r).is_some_and(|c| c == "NegateOperation");
        acc = match (op, negate) {
            (Op::Union, false) => csg::union(acc, p),
            (Op::Union, true) | (Op::Subtract, _) => csg::subtract(acc, p),
            (Op::Intersect, _) => csg::intersect(acc, p),
        };
    }
    Ok((csg::triangles(&acc), xf))
}

fn instances(v: &Value) -> mlua::Result<Vec<Ref>> {
    match v {
        Value::Table(t) => {
            let mut out = Vec::new();
            for x in t.clone().sequence_values::<Value>() {
                let x = x?;
                out.push(opt_inst(&x).ok_or_else(|| rt_err(UNKNOWN))?);
            }
            Ok(out)
        }
        Value::Nil => Err(rt_err("Argument 1 missing or nil")),
        _ => Err(rt_err("Unable to cast value to Objects")),
    }
}

fn enum_arg(v: &Value, ty: &str, default: u32) -> mlua::Result<u32> {
    match v {
        Value::Nil => Ok(default),
        Value::UserData(_) => match opt_ud::<LEnumItem>(v) {
            Some(e) if e.enum_type == ty => Ok(e.value),
            _ => Err(rt_err(format!("Unable to cast value to {ty}"))),
        },
        Value::Number(n) => Ok(*n as u32),
        Value::Integer(n) => Ok(*n as u32),
        Value::String(s) => LEnumItem::from_name(ty, &s.to_str()?).map(|e| e.value).ok_or_else(|| rt_err(format!("Unable to cast string to {ty}"))),
        _ => Err(rt_err(format!("Unable to cast value to {ty}"))),
    }
}

struct Opts {
    fid: Fid,
    split: bool,
}

fn csg_options(lua: &Lua, v: &Value) -> mlua::Result<Opts> {
    let mut o = Opts { fid: Fid { collision: 0, render: 0 }, split: true };
    let Value::Table(t) = v else { return Ok(o) };
    for pair in t.clone().pairs::<Value, Value>() {
        let (k, val) = pair?;
        let key = match &k {
            Value::String(s) => s.to_str()?.to_string(),
            _ => continue,
        };
        let want_bool = |val: &Value| match val {
            Value::Boolean(b) => Ok(*b),
            _ => Err(rt_err(format!("argument for Key '{key}' needs to be a bool"))),
        };
        match key.as_str() {
            "CollisionFidelity" => o.fid.collision = enum_arg(&val, "CollisionFidelity", 0)?,
            "RenderFidelity" => o.fid.render = enum_arg(&val, "RenderFidelity", 0)?,
            "FluidFidelity" => {
                enum_arg(&val, "FluidFidelity", 0)?;
            }
            "SplitApart" => o.split = want_bool(&val)?,
            "RebuildHistory" => {
                want_bool(&val)?;
            }
            "UseDynamicConvexDecomposition" => {
                world(lua)?.output.push((crate::app::Level::Warn, "UseDynamicConvexDecomposition is ignored because it is deprecated.".into()));
            }
            _ => return Err(rt_err(format!("Key '{key}' is not a valid member of CSGOptions"))),
        }
    }
    Ok(o)
}

fn op_of(name: &str) -> Op {
    match name {
        "UnionAsync" => Op::Union,
        "SubtractAsync" => Op::Subtract,
        _ => Op::Intersect,
    }
}

fn result_class(op: Op) -> (&'static str, &'static str) {
    match op {
        Op::Intersect => ("IntersectOperation", "Intersection"),
        _ => ("UnionOperation", "Union"),
    }
}

/// BasePart:UnionAsync(parts, collisionFidelity, renderFidelity).
fn part_op(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let op = op_of(name);
    let others = instances(&arg(args, 0))?;
    let fid = Fid {
        collision: enum_arg(&arg(args, 1), "CollisionFidelity", 0)?,
        render: enum_arg(&arg(args, 2), "RenderFidelity", 0)?,
    };
    let (tris, xf) = {
        let w = world(lua)?;
        run_op(w.d(), op, this, &others)?
    };
    if tris.is_empty() {
        if op == Op::Intersect {
            return Ok(MultiValue::from_iter([Value::Nil]));
        }
        return Err(rt_err("CSG: Result is empty (error -3)"));
    }
    let p = piece(&tris, xf.rot)?;
    let (class, n) = result_class(op);
    let r = make_result(lua, this, class, n, &p, &fid)?;
    Ok(MultiValue::from_iter([inst_value(lua, r)?]))
}

/// GeometryService:UnionAsync(part, parts, options) → array.
fn service_op(lua: &Lua, name: &str, args: &MultiValue) -> mlua::Result<MultiValue> {
    let op = op_of(name);
    let main = opt_inst(&arg(args, 0)).ok_or_else(|| rt_err(UNKNOWN))?;
    let others = instances(&arg(args, 1))?;
    let o = csg_options(lua, &arg(args, 2))?;
    let (tris, xf) = {
        let w = world(lua)?;
        run_op(w.d(), op, main, &others)?
    };
    let t = lua.create_table()?;
    if tris.is_empty() {
        return Ok(MultiValue::from_iter([Value::Table(t)]));
    }
    let groups = if o.split { csg::split_apart(tris) } else { vec![tris] };
    let (class, n) = result_class(op);
    for (i, g) in groups.iter().enumerate() {
        let p = piece_in(g, xf, false)?;
        let r = make_result(lua, main, class, n, &p, &o.fid)?;
        t.raw_set(i + 1, inst_value(lua, r)?)?;
    }
    Ok(MultiValue::from_iter([Value::Table(t)]))
}

// ------------------------------------------------------------ fragments

fn v3(v: &Value) -> Option<Vec3> {
    opt_ud::<LVector3>(v).map(|v| v.0)
}

/// A random number in [0, 1) (Studio's fragment sites differ every call).
fn rand01() -> f32 {
    use std::cell::Cell;
    thread_local! {
        static SEED: Cell<u64> = Cell::new(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1);
    }
    SEED.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 40) as f32 / (1u64 << 24) as f32
    })
}

fn option_keys(v: &Value, allowed: &[&str]) -> mlua::Result<()> {
    if let Value::Table(t) = v {
        for pair in t.clone().pairs::<Value, Value>() {
            let (k, _) = pair?;
            let key = match &k {
                Value::String(s) => s.to_str()?.to_string(),
                _ => continue,
            };
            if !allowed.contains(&key.as_str()) {
                return Err(rt_err(format!("Key '{key}' is not a valid option.")));
            }
        }
    }
    Ok(())
}

/// GenerateFragmentSites(part, {SiteSpacing?, Origin?, Radius?}), as Studio
/// (measured): random points inside the part, one per grid cell, 2 per axis
/// by default or ceil(size / SiteSpacing). With Origin and Radius: a grid
/// of 2·ceil(1.5 / SiteSpacing)+1 per axis (SiteSpacing 0.5 by default:
/// 343 sites, whatever the part's size), the sites inside the sphere one
/// by one and every other site in one array first, so FragmentAsync leaves
/// the rest of the part whole.
fn fragment_sites(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let part = opt_inst(&arg(args, 0)).ok_or_else(|| rt_err("GeometryService:GenerateFragmentSites: Parameter 1 must be a BasePart."))?;
    let (xf, size) = {
        let w = world(lua)?;
        let d = w.d();
        if !d.class_of(part).is_some_and(|c| doc::is_a(&c, "BasePart")) {
            return Err(rt_err("GeometryService:GenerateFragmentSites: Parameter 1 must be a BasePart."));
        }
        (crate::scene::xform_of(d, part).unwrap_or(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }), crate::scene::vec3_of(d.prop(part, "Size")).unwrap_or(Vec3::ONE))
    };
    let opts = arg(args, 1);
    option_keys(&opts, &["SiteSpacing", "Origin", "Radius"])?;
    let get = |k: &str| match &opts {
        Value::Table(t) => t.get::<Value>(k).unwrap_or(Value::Nil),
        _ => Value::Nil,
    };
    let spacing = match get("SiteSpacing") {
        Value::Nil => None,
        v => match num(&v) {
            Some(n) if n > 0.0 => Some(n as f32),
            _ => return Err(rt_err("GeometryService:GenerateFragmentSites: SiteSpacing must be a positive number.")),
        },
    };
    let origin = v3(&get("Origin"));
    let radius = match get("Radius") {
        Value::Nil => None,
        v => match num(&v) {
            Some(n) if n > 0.0 => Some(n as f32),
            _ => return Err(rt_err("GeometryService:GenerateFragmentSites: Radius must be a positive number.")),
        },
    };
    if origin.is_some() != radius.is_some() {
        return Err(rt_err("GeometryService:GenerateFragmentSites: Either both origin and radius should be provided, or neither."));
    }
    let sphere = origin.zip(radius);
    let n = match (sphere, spacing) {
        (Some(_), sp) => Vec3::splat(2.0 * (1.5 / sp.unwrap_or(0.5)).ceil() + 1.0),
        (None, Some(sp)) => (size / sp).ceil().max(Vec3::ONE),
        (None, None) => Vec3::splat(2.0),
    };
    let cell = size / n;
    let mut inside = Vec::new();
    let mut rest = Vec::new();
    for ix in 0..n.x as i32 {
        for iy in 0..n.y as i32 {
            for iz in 0..n.z as i32 {
                let local = -size * 0.5 + cell * (Vec3::new(ix as f32, iy as f32, iz as f32) + Vec3::new(rand01(), rand01(), rand01()));
                let p = xf.apply(local);
                match sphere {
                    Some((o, r)) if p.distance(o) > r => rest.push(p),
                    _ => inside.push(p),
                }
            }
        }
    }
    let t = lua.create_table()?;
    let mut k = 0;
    if !rest.is_empty() {
        let g = lua.create_table()?;
        for (i, p) in rest.into_iter().enumerate() {
            g.raw_set(i + 1, LVector3(p))?;
        }
        k += 1;
        t.raw_set(k, g)?;
    }
    for p in inside {
        k += 1;
        t.raw_set(k, LVector3(p))?;
    }
    Ok(MultiValue::from_iter([Value::Table(t)]))
}

/// FragmentAsync(part, sites, options): the part cut into the Voronoi cells
/// of `sites` (an array of sites makes one fragment of their cells). As
/// Studio: an array of {Index = the site entry, Instance = MeshPart}, each
/// MeshPart unparented with the part's CFrame and properties, its geometry
/// off-centre in it, Size its extents.
fn fragment(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let part = opt_inst(&arg(args, 0)).ok_or_else(|| rt_err(UNKNOWN))?;
    let bad = || rt_err("GeometryService:FragmentAsync: `sites` must be an array where each element is either a Vector3 or an array of Vector3");
    let groups: Vec<Vec<Vec3>> = match arg(args, 1) {
        Value::Table(t) => {
            let mut g = Vec::new();
            for v in t.sequence_values::<Value>() {
                let v = v?;
                if let Some(p) = v3(&v) {
                    g.push(vec![p]);
                } else if let Value::Table(inner) = v {
                    let mut ps = Vec::new();
                    for x in inner.sequence_values::<Value>() {
                        ps.push(v3(&x?).ok_or_else(bad)?);
                    }
                    g.push(ps);
                } else {
                    return Err(bad());
                }
            }
            g
        }
        _ => return Err(bad()),
    };
    let all: Vec<Vec3> = groups.iter().flatten().copied().collect();
    if all.len() > 10000 {
        return Err(rt_err("GeometryService:FragmentAsync: `sites` exceeds the maximum of 10000 elements."));
    }
    option_keys(&arg(args, 2), &["CollisionFidelity", "RenderFidelity", "FluidFidelity"])?;
    let o = csg_options(lua, &arg(args, 2))?;
    let _ = &o;
    let (base, xf) = {
        let w = world(lua)?;
        polys_of(w.d(), part)?
    };
    // A cell: the part's polygons less every bisector's far half-space.
    let (lo, hi) = base.iter().flat_map(|p| p.v.iter()).fold((DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)), |(a, b), v| (a.min(*v), b.max(*v)));
    let big = (hi - lo).length() * 2.0 + 10.0;
    let t = lua.create_table()?;
    let mut k = 0;
    for (gi, g) in groups.iter().enumerate() {
        let mut acc: Vec<Poly> = Vec::new();
        for s in g {
            let s = s.as_dvec3();
            let mut cell = base.clone();
            for o2 in &all {
                let o2 = o2.as_dvec3();
                if g.iter().any(|x| x.as_dvec3() == o2) || o2 == s {
                    continue;
                }
                let n = (o2 - s).normalize_or_zero();
                if n == DVec3::ZERO {
                    continue;
                }
                let mid = (s + o2) * 0.5;
                let box_center = mid + n * big * 0.5;
                let half = Xform { pos: box_center.as_vec3(), rot: basis(n).as_mat3() };
                let cutter = csg::part_polys(&Shape::Block, &half, Vec3::splat(big as f32), [1.0; 3]);
                cell = csg::subtract(cell, cutter);
                if cell.is_empty() {
                    break;
                }
            }
            acc = csg::union(acc, cell);
        }
        let tris = csg::triangles(&acc);
        if tris.is_empty() {
            continue;
        }
        let r = mesh_part_in(lua, Some(part), piece_in(&tris, xf, false)?)?;
        let row = lua.create_table()?;
        row.set("Index", (gi + 1) as f64)?;
        row.set("Instance", inst_value(lua, r)?)?;
        k += 1;
        t.raw_set(k, row)?;
    }
    Ok(MultiValue::from_iter([Value::Table(t)]))
}

/// An orthonormal basis whose Z axis is `n`.
fn basis(n: DVec3) -> glam::DMat3 {
    let up = if n.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
    let x = up.cross(n).normalize();
    let y = n.cross(x);
    glam::DMat3::from_cols(x, y, n)
}

// ------------------------------------------------------------ meshes

/// A MeshPart from world triangles (BakeToMeshAsync, SweepPartAsync): the
/// mesh registered as generated content.
fn mesh_part(lua: &Lua, main: Option<Ref>, tris: &[([DVec3; 3], [f32; 3])], rot: Mat3) -> mlua::Result<Ref> {
    mesh_part_in(lua, main, piece(tris, rot)?)
}

/// A MeshPart from a laid-out piece (its mesh registered as generated
/// content; off-centre pieces keep their frame, mesh_bvh::union_offset).
fn mesh_part_in(lua: &Lua, main: Option<Ref>, p: Piece) -> mlua::Result<Ref> {
    let n = {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    };
    let id = format!("rbxtemp://rustudio/geometry/mesh/{n}");
    crate::assets::register_generated(&id, p.blob.clone());
    let mut w = world(lua)?;
    let d = w.dm();
    let mut b = InstanceBuilder::new("MeshPart").with_name("MeshPart");
    if let Some(inst) = main.and_then(|m| d.dom.get_by_ref(m)) {
        for k in COPIED {
            if let Some(v) = inst.properties.get(&ustr(k)) {
                b = b.with_property(*k, v.clone());
            }
        }
    }
    let s = p.size;
    b = b
        .with_property("CFrame", Variant::CFrame(p.xf.to_cframe()))
        .with_property("Size", Variant::Vector3(rbx_dom_weak::types::Vector3::new(s.x, s.y, s.z)))
        .with_property("MeshId", Variant::ContentId(id.clone().into()))
        .with_property("MeshContent", Variant::Content(rbx_dom_weak::types::Content::from_uri(id)));
    Ok(d.dom.insert(Ref::none(), b))
}

fn bake(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let input = opt_inst(&arg(args, 0)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
    let (tris, rot, main) = {
        let w = world(lua)?;
        let d = w.d();
        let class = d.class_of(input).ok_or_else(|| rt_err(UNKNOWN))?;
        let parts: Vec<Ref> = if doc::is_a(&class, "BasePart") {
            vec![input]
        } else if doc::is_a(&class, "Model") {
            d.subtree(input).into_iter().filter(|r| d.class_of(*r).is_some_and(|c| doc::is_a(&c, "BasePart"))).collect()
        } else {
            return Err(rt_err("Selected instance is not a Model"));
        };
        let mut all = Vec::new();
        let mut rot = Mat3::IDENTITY;
        for (i, r) in parts.iter().enumerate() {
            let (p, xf) = polys_of(d, *r)?;
            if i == 0 {
                rot = xf.rot;
            }
            all.extend(csg::triangles(&p));
        }
        (all, rot, parts.first().copied())
    };
    if tris.is_empty() {
        return Err(rt_err("CSG: Result is empty (error -3)"));
    }
    let r = mesh_part(lua, main, &tris, rot)?;
    Ok(MultiValue::from_iter([inst_value(lua, r)?]))
}

/// SweepPartAsync(part, cframes, options): the union of the part's convex
/// hull swept between consecutive CFrames (world placements of the part's
/// shape). As Studio (measured): a MeshPart keeping the part's CFrame, its
/// geometry off-centre in it, Size its extents in the part's frame; option
/// keys as CSG's, others "Key 'X' is not a valid option.". (Studio crashes
/// on an empty or non-CFrame list; rustudio errors with the messages its
/// program carries.)
fn sweep(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let part = opt_inst(&arg(args, 0)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
    let cfs: Vec<Xform> = match arg(args, 1) {
        Value::Table(t) => {
            let mut v = Vec::new();
            for x in t.sequence_values::<Value>() {
                let x = x?;
                v.push(opt_ud::<LCFrame>(&x).map(|c| c.0).ok_or_else(|| rt_err("GeometryService:SweepPartAsync: CFrames contains a non-CFrame value"))?);
            }
            v
        }
        _ => Vec::new(),
    };
    if cfs.is_empty() {
        return Err(rt_err("GeometryService:SweepPartAsync: CFrames must be a non-empty array of Vector3"));
    }
    option_keys(&arg(args, 2), &["CollisionFidelity", "RenderFidelity", "FluidFidelity"])?;
    let (local_pts, color, part_xf) = {
        let w = world(lua)?;
        let d = w.d();
        let (p, xf) = polys_of(d, part)?;
        let inv = xf.inverse();
        let pts: Vec<Vec3> = p.iter().flat_map(|q| q.v.iter().map(|v| inv.apply(v.as_vec3()))).collect();
        (pts, color_of(d, part), xf)
    };
    let mut acc: Vec<Poly> = Vec::new();
    let pairs: Vec<(Xform, Xform)> = if cfs.len() == 1 { vec![(cfs[0], cfs[0])] } else { cfs.windows(2).map(|w| (w[0], w[1])).collect() };
    for (a, b) in pairs {
        let pts: Vec<rapier3d::math::Vector> = local_pts
            .iter()
            .flat_map(|p| [a.apply(*p), b.apply(*p)])
            .map(|p| rapier3d::math::Vector::new(p.x, p.y, p.z))
            .collect();
        let Ok((verts, idx)) = rapier3d::parry::transformation::try_convex_hull(&pts) else { continue };
        let hull: Vec<Poly> = idx
            .iter()
            .filter_map(|t| {
                let v = t.iter().map(|&i| DVec3::new(verts[i as usize].x as f64, verts[i as usize].y as f64, verts[i as usize].z as f64)).collect();
                Poly::new(v, color)
            })
            .collect();
        acc = csg::union(acc, hull);
    }
    let tris = csg::triangles(&acc);
    if tris.is_empty() {
        return Err(rt_err("CSG: Result is empty (error -3)"));
    }
    let r = mesh_part_in(lua, Some(part), piece_in(&tris, part_xf, false)?)?;
    Ok(MultiValue::from_iter([inst_value(lua, r)?]))
}

// ------------------------------------------------------------ constraints

fn ref_prop(d: &Document, r: Ref, k: &str) -> Ref {
    match d.prop(r, k) {
        Some(Variant::Ref(a)) => a,
        _ => Ref::none(),
    }
}

/// CalculateConstraintsToPreserve(source, destination, options), as Studio
/// (measured):
/// - per constraint on one of the source's attachments: {Attachment,
///   AttachmentParent, Constraint, ConstraintParent}; the parent is the
///   destination part holding the attachment (within `tolerance`), left
///   out when none does; ConstraintParent only when the constraint was
///   parented to the source. Attachments without constraints aren't listed.
/// - per NoCollisionConstraint with the source, one row per destination
///   part: the source replaced by it, NoCollisionParent it when the
///   constraint was under the source.
/// - per WeldConstraint with the source (weldConstraintPreserve All 0,
///   None 1, Touching 2 = default): one row per destination part (All) or
///   per destination touching the other part (Touching), the source
///   replaced; WeldConstraintParent only when the weld was under the
///   source; with no such destination, one row as it is.
fn constraints_to_preserve(lua: &Lua, args: &MultiValue) -> mlua::Result<MultiValue> {
    let source = opt_inst(&arg(args, 0)).ok_or_else(|| rt_err("CalculateConstraintsToPreserve: source cannot be nil"))?;
    let dest: Vec<Ref> = match arg(args, 1) {
        Value::Table(t) => {
            let n = t.raw_len();
            let mut v = Vec::new();
            for i in 1..=n.max(1) {
                let x: Value = t.raw_get(i)?;
                if x.is_nil() && n == 0 {
                    break;
                }
                v.push(opt_inst(&x).ok_or_else(|| rt_err("CalculateConstraintsToPreserve: cannot have nil as a destination"))?);
            }
            // A hole ({a, nil, b}) is a nil destination too.
            let max_key = t.clone().pairs::<Value, Value>().filter_map(|p| p.ok()).filter_map(|(k, _)| num(&k)).fold(0.0, f64::max) as usize;
            if max_key > v.len() {
                return Err(rt_err("CalculateConstraintsToPreserve: cannot have nil as a destination"));
            }
            v
        }
        _ => return Err(rt_err("CalculateConstraintsToPreserve: cannot have nil as a destination")),
    };
    let opts = arg(args, 2);
    let mut tolerance = 0.0f32;
    let mut weld_mode = 2u32;
    if let Value::Table(t) = &opts {
        for pair in t.clone().pairs::<Value, Value>() {
            let (k, v) = pair?;
            let key = match &k {
                Value::String(s) => s.to_str()?.to_string(),
                _ => continue,
            };
            match key.as_str() {
                "tolerance" => tolerance = num(&v).ok_or_else(|| rt_err("argument for Key 'tolerance' needs to be a number"))? as f32,
                "weldConstraintPreserve" => weld_mode = enum_arg(&v, "WeldConstraintPreserve", 2)?,
                "dropAttachmentsWithoutConstraints" => {}
                _ => return Err(rt_err(format!("Key '{key}' is not a valid member of CSGConstraintOptions"))),
            }
        }
    }
    type Row = Vec<(&'static str, Ref)>;
    let rows: Vec<Row> = {
        let w = world(lua)?;
        let d = w.d();
        let boxes: Vec<(Ref, Xform, Vec3)> = dest
            .iter()
            .filter_map(|r| Some((*r, crate::scene::xform_of(d, *r)?, crate::scene::vec3_of(d.prop(*r, "Size"))?)))
            .collect();
        let home = |p: Vec3| -> Option<Ref> {
            let mut best: Option<(Ref, f32)> = None;
            for (r, xf, size) in &boxes {
                let q = xf.inverse().apply(p);
                let out = (q.abs() - *size * 0.5).max(Vec3::ZERO).length();
                if out <= tolerance + 1e-3 && best.is_none_or(|b| out < b.1) {
                    best = Some((*r, out));
                }
            }
            best.map(|b| b.0)
        };
        let touching = |other: Ref| -> Vec<Ref> {
            let (Some(oxf), Some(os)) = (crate::scene::xform_of(d, other), crate::scene::vec3_of(d.prop(other, "Size"))) else { return Vec::new() };
            let grow = Vec3::splat(2.0 * (tolerance + 0.01));
            boxes.iter().filter(|(_, xf, s)| crate::query_shapes::obb_obb(xf, *s + grow, &oxf, os)).map(|b| b.0).collect()
        };
        let atts: Vec<Ref> = d
            .dom
            .get_by_ref(source)
            .map(|i| i.children().iter().copied().filter(|c| d.class_of(*c).is_some_and(|k| k == "Attachment")).collect())
            .unwrap_or_default();
        // Constraints under the source come first (Studio's order).
        let mine = d.subtree(source);
        let all: Vec<Ref> = mine.iter().copied().chain(d.subtree(d.dom.root_ref()).into_iter().filter(|r| !mine.contains(r))).collect();
        let parent_of = |r: Ref| d.dom.get_by_ref(r).map(|i| i.parent()).unwrap_or(Ref::none());
        let (mut att_rows, mut nc_rows, mut weld_rows) = (Vec::new(), Vec::new(), Vec::new());
        for c in &all {
            let Some(class) = d.class_of(*c) else { continue };
            if class == "WeldConstraint" || class == "NoCollisionConstraint" {
                let (p0, p1) = (ref_prop(d, *c, "Part0"), ref_prop(d, *c, "Part1"));
                if p0 != source && p1 != source {
                    continue;
                }
                let other = if p0 == source { p1 } else { p0 };
                let under = parent_of(*c) == source;
                let swap = |t: Ref| if p0 == source { (t, p1) } else { (p0, t) };
                if class == "NoCollisionConstraint" {
                    for t in &dest {
                        let (n0, n1) = swap(*t);
                        let mut row: Row = vec![("NoCollisionConstraint", *c)];
                        if under {
                            row.push(("NoCollisionParent", *t));
                        }
                        row.extend([("NoCollisionPart0", n0), ("NoCollisionPart1", n1)]);
                        nc_rows.push(row);
                    }
                    continue;
                }
                let targets = match weld_mode {
                    1 => continue,
                    0 => dest.clone(),
                    _ => touching(other),
                };
                if targets.is_empty() {
                    weld_rows.push(vec![("WeldConstraint", *c), ("WeldConstraintPart0", p0), ("WeldConstraintPart1", p1)]);
                }
                for t in targets {
                    let (n0, n1) = swap(t);
                    let mut row: Row = vec![("WeldConstraint", *c)];
                    if under {
                        row.push(("WeldConstraintParent", t));
                    }
                    row.extend([("WeldConstraintPart0", n0), ("WeldConstraintPart1", n1)]);
                    weld_rows.push(row);
                }
            } else if doc::is_a(&class, "Constraint") {
                for a in [ref_prop(d, *c, "Attachment0"), ref_prop(d, *c, "Attachment1")] {
                    if !atts.contains(&a) {
                        continue;
                    }
                    let p = crate::attachment::world(d, a, true).map(|x| x.pos).unwrap_or(Vec3::ZERO);
                    let h = home(p);
                    let mut row: Row = vec![("Attachment", a)];
                    if let Some(h) = h {
                        row.push(("AttachmentParent", h));
                    }
                    row.push(("Constraint", *c));
                    if let (Some(h), true) = (h, parent_of(*c) == source) {
                        row.push(("ConstraintParent", h));
                    }
                    att_rows.push(row);
                }
            }
        }
        att_rows.into_iter().chain(nc_rows).chain(weld_rows).collect()
    };
    let t = lua.create_table()?;
    for (i, row) in rows.into_iter().enumerate() {
        let e = lua.create_table()?;
        for (k, r) in row {
            e.set(k, inst_value(lua, r)?)?;
        }
        t.raw_set(i + 1, e)?;
    }
    Ok(MultiValue::from_iter([Value::Table(t)]))
}

// ------------------------------------------------------------ dispatch

const MESH_PROPS: &[&str] = &["MeshData", "MeshData2", "SolidMeshHolder", "PhysicalConfigData", "PhysicsData", "AssetId", "ChildData", "ChildData2", "InitialSize", "MeshSize", "TriangleCount", "FormFactor"];

pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if !METHODS.iter().any(|(m, _)| *m == name) {
        return None;
    }
    let class = world(lua).ok()?.d().class_of(this)?;
    let class = class.as_str();
    // The method's own arguments (args[0] is self).
    let rest: MultiValue = args.iter().skip(1).cloned().collect();
    let args = &rest;
    let a = |i: usize| arg(args, i);
    Some((|| match (class, name) {
        ("GeometryService", "CalculateConstraintsToPreserve") => constraints_to_preserve(lua, args),
        ("GeometryService", "GenerateFragmentSites") => fragment_sites(lua, args),
        ("GeometryService", "CreateBasicMeshPart") => {
            // Studio, in a game script: the API isn't released.
            let _ = a(0);
            Err(rt_err("GeometryService:CreateBasicMeshPart: This API is not yet released!"))
        }
        (c, "SubstituteGeometry") if doc::is_a(c, "PartOperation") => {
            // Studio: the source's geometry and Size; this part keeps its
            // CFrame. A plain part source works too (its shape becomes the
            // geometry).
            let src = opt_inst(&a(0)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let (props, built) = {
                let w = world(lua)?;
                let d = w.d();
                let k = d.class_of(src).ok_or_else(|| rt_err(UNKNOWN))?;
                if doc::is_a(&k, "PartOperation") {
                    let mut props: Vec<(String, Variant)> = MESH_PROPS
                        .iter()
                        .filter_map(|k| d.dom.get_by_ref(src).and_then(|i| i.properties.get(&ustr(k)).cloned()).map(|v| (k.to_string(), v)))
                        .collect();
                    if let Some(v) = d.prop(src, "Size") {
                        props.push(("Size".into(), v));
                    }
                    (props, None)
                } else {
                    let (polys, xf) = polys_of(d, src)?;
                    (Vec::new(), Some(piece(&csg::triangles(&polys), xf.rot)?))
                }
            };
            for (k, v) in props {
                store_prop(lua, this, &k, v)?;
            }
            if let Some(p) = built {
                let s = p.size;
                store_prop(lua, this, "MeshData2", Variant::SharedString(SharedString::new(p.blob)))?;
                store_prop(lua, this, "TriangleCount", Variant::Int32(p.tris as i32))?;
                store_prop(lua, this, "Size", Variant::Vector3(rbx_dom_weak::types::Vector3::new(s.x, s.y, s.z)))?;
            }
            Ok(MultiValue::new())
        }
        (c, "ApplyMesh") if c == "MeshPart" => {
            let src = opt_inst(&a(0)).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let props: Vec<(&str, Variant)> = {
                let w = world(lua)?;
                let d = w.d();
                if !d.class_of(src).is_some_and(|k| k == "MeshPart") {
                    return Err(rt_err("ApplyMesh requires an MeshPart object"));
                }
                ["MeshId", "MeshContent", "InitialSize", "PhysicalConfigData", "PhysicsData", "HasSkinnedMesh", "HasJointOffset", "JointOffset"]
                    .into_iter()
                    .filter_map(|k| d.prop(src, k).map(|v| (k, v)))
                    .collect()
            };
            for (k, v) in props {
                store_prop(lua, this, k, v)?;
            }
            Ok(MultiValue::new())
        }
        _ => Ok(MultiValue::new()),
    })())
}

/// `__geo_ops(name, self, ...)`: the yielding calls' work.
fn ops_call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    let class = world(lua)?.d().class_of(this).map(|c| c.to_string()).unwrap_or_default();
    match (class.as_str(), name) {
        ("GeometryService", "UnionAsync" | "SubtractAsync" | "IntersectAsync") => service_op(lua, name, args),
        ("GeometryService", "FragmentAsync") => fragment(lua, args),
        ("GeometryService", "SweepPartAsync") => sweep(lua, args),
        // Studio (this build) hasn't the member switched on.
        ("GeometryService", "BakeToMeshAsync") => {
            let _ = bake;
            Err(rt_err("BakeToMeshAsync is not a valid member of GeometryService \"GeometryService\""))
        }
        // Studio (this build): the API is switched off.
        ("MeshPart", "GetSurfaceSlotsAsync") => Err(rt_err("MeshPart:GetSurfaceSlotsAsync is not yet enabled")),
        (c, "UnionAsync" | "SubtractAsync" | "IntersectAsync") if doc::is_a(c, "BasePart") => part_op(lua, name, this, args),
        _ => Err(rt_err(format!("{name} is not a valid member of {class}"))),
    }
}

pub fn install(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "__geo_ops",
        lua.create_function(|lua, (name, this, rest): (String, Value, MultiValue)| {
            let r = opt_inst(&this).ok_or_else(|| rt_err(format!("Expected ':' not '.' calling member function {name}")))?;
            // (true, results…) or (false, Studio's message) for the Luau side.
            Ok(match ops_call(lua, &name, r, &rest) {
                Ok(v) => std::iter::once(Value::Boolean(true)).chain(v).collect::<MultiValue>(),
                Err(e) => MultiValue::from_iter([Value::Boolean(false), Value::String(lua.create_string(super::plain_error(&e))?)]),
            })
        })?,
    )?;
    lua.load(
        r#"
local M = __rs_lua_methods
local ops = __geo_ops
for _, name in {"UnionAsync", "SubtractAsync", "IntersectAsync", "FragmentAsync", "SweepPartAsync", "BakeToMeshAsync", "GetSurfaceSlotsAsync"} do
    M[name] = function(self, ...)
        local r = table.pack(ops(name, self, ...))
        task.wait()
        if not r[1] then error(r[2], 0) end
        return table.unpack(r, 2, r.n)
    end
end
"#,
    )
    .set_name("geometry_ops")
    .exec()?;
    Ok(())
}
