//! EditableMesh's data and methods, measured against Studio (offline Studio, slot
//! of my own; notes in checks/parity/geometry.md):
//!
//! - ids are int64: `((generation - 1) * 8 + kind) << 32 | slot`, slots
//!   1-based; kinds Vertex 1, Normal 2, Color 3, UV 4, Face 5, Bone 6. A
//!   removed slot is reused with the next generation (`IdDebugString`
//!   shows `f1,2`). The Get* lists come in creation order.
//! - AddTriangle / AddFace (3 or 4 vertices) give each corner its vertex's
//!   default normal / UV / colour, made on the vertex's first face: an
//!   automatic (smooth, area-weighted) normal, UV (0,0), white alpha 1.
//! - bones (Name, CFrame, Virtual, optional ParentId), per-vertex bone ids
//!   and weights (stored as bytes: 0.5 reads back 128/255), FACS poses and
//!   correctives (virtual bones only).
//! - MergeVertices(tolerance) greedily maps later vertices within tolerance
//!   onto the first, rewrites faces and keeps the vertices (as Studio).

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{Vec2, Vec3};
use mlua::{Lua, MultiValue, Table, Value};

use crate::ltypes::*;
use crate::math::Xform;

pub const K_VERTEX: u64 = 1;
pub const K_NORMAL: u64 = 2;
pub const K_COLOR: u64 = 3;
pub const K_UV: u64 = 4;
pub const K_FACE: u64 = 5;
pub const K_BONE: u64 = 6;

fn kind_name(k: u64) -> &'static str {
    match k {
        K_VERTEX => "Vertex",
        K_NORMAL => "Normal",
        K_COLOR => "Color",
        K_UV => "UV",
        K_FACE => "Face",
        K_BONE => "Bone",
        _ => "Unknown",
    }
}

fn kind_letter(k: u64) -> &'static str {
    match k {
        K_VERTEX => "v",
        K_NORMAL => "n",
        K_COLOR => "c",
        K_UV => "u",
        K_FACE => "f",
        K_BONE => "b",
        _ => "?",
    }
}

pub fn mk(kind: u64, slot: usize, gen: u32) -> u64 {
    (((gen as u64 - 1) * 8 + kind) << 32) | (slot as u64 + 1)
}

/// (kind, slot, generation) of an id, or None when it isn't one.
pub fn parse(id: f64) -> Option<(u64, usize, u32)> {
    if !(id >= 1.0) || id.fract() != 0.0 || id >= 9.007e15 {
        return None;
    }
    let id = id as u64;
    let low = id & 0xffff_ffff;
    let high = id >> 32;
    let kind = high % 8;
    if low == 0 || !(1..=6).contains(&kind) {
        return None;
    }
    Some((kind, low as usize - 1, (high / 8) as u32 + 1))
}

#[derive(Clone, Debug)]
pub struct Slab<T> {
    items: Vec<Option<T>>,
    gens: Vec<u32>,
    order: Vec<u32>,
    free: Vec<u32>,
    kind: u64,
}

impl<T> Slab<T> {
    fn new(kind: u64) -> Self {
        Slab { items: Vec::new(), gens: Vec::new(), order: Vec::new(), free: Vec::new(), kind }
    }
    fn add(&mut self, v: T) -> u64 {
        let slot = match self.free.pop() {
            Some(s) => s as usize,
            None => {
                self.items.push(None);
                self.gens.push(0);
                self.items.len() - 1
            }
        };
        self.gens[slot] += 1;
        self.items[slot] = Some(v);
        self.order.push(slot as u32);
        mk(self.kind, slot, self.gens[slot])
    }
    fn slot(&self, id: u64) -> Option<usize> {
        let (k, s, g) = parse(id as f64)?;
        (k == self.kind && self.gens.get(s) == Some(&g) && self.items[s].is_some()).then_some(s)
    }
    pub fn get(&self, id: u64) -> Option<&T> {
        self.slot(id).and_then(|s| self.items[s].as_ref())
    }
    fn get_mut(&mut self, id: u64) -> Option<&mut T> {
        let s = self.slot(id)?;
        self.items[s].as_mut()
    }
    fn remove(&mut self, id: u64) -> Option<T> {
        let s = self.slot(id)?;
        self.order.retain(|x| *x as usize != s);
        self.free.push(s as u32);
        self.items[s].take()
    }
    pub fn ids(&self) -> Vec<u64> {
        self.order.iter().map(|&s| mk(self.kind, s as usize, self.gens[s as usize])).collect()
    }
    pub fn iter(&self) -> impl Iterator<Item = (u64, &T)> {
        self.order.iter().filter_map(move |&s| {
            let s = s as usize;
            self.items[s].as_ref().map(|v| (mk(self.kind, s, self.gens[s]), v))
        })
    }
}

#[derive(Clone, Debug)]
pub struct Vertex {
    pub p: Vec3,
    dflt: [Option<u64>; 3],
    bones: Vec<u64>,
    weights: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct Normal {
    v: Vec3,
    auto: bool,
}

#[derive(Clone, Debug)]
pub struct Face {
    pub v: Vec<u64>,
    pub n: Vec<u64>,
    pub uv: Vec<u64>,
    pub c: Vec<u64>,
}

#[derive(Clone, Debug)]
pub struct Bone {
    name: String,
    cf: Xform,
    parent: u64,
    virt: bool,
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub n: u64,
    pub gen: u64,
    pub fixed: bool,
    pub verts: Slab<Vertex>,
    pub normals: Slab<Normal>,
    pub uvs: Slab<Vec2>,
    pub colors: Slab<([f32; 3], f32)>,
    pub faces: Slab<Face>,
    bones: Slab<Bone>,
    /// FacsActionUnit value → (bone, cframe) pairs.
    facs: BTreeMap<u32, Vec<(u64, Xform)>>,
    /// Sorted FacsActionUnit values (2 or 3) → pose.
    correctives: Vec<(Vec<u32>, Vec<(u64, Xform)>)>,
}

impl Default for Mesh {
    fn default() -> Self {
        Mesh {
            n: 0,
            gen: 0,
            fixed: false,
            verts: Slab::new(K_VERTEX),
            normals: Slab::new(K_NORMAL),
            uvs: Slab::new(K_UV),
            colors: Slab::new(K_COLOR),
            faces: Slab::new(K_FACE),
            bones: Slab::new(K_BONE),
            facs: BTreeMap::new(),
            correctives: Vec::new(),
        }
    }
}

/// Studio's id error: the wrong kind, or not an id at all.
fn want(id: f64, kind: u64) -> mlua::Error {
    match parse(id) {
        Some((k, _, _)) if k != kind => rt_err(format!(
            "Unexpected EditableMesh id type. Expected {} id, received {} id",
            kind_name(kind),
            kind_name(k)
        )),
        Some(_) => rt_err("Invalid id"),
        None => rt_err(format!(
            "Unexpected EditableMesh id type. Expected {} id, received value that doesn't appear to be an id",
            kind_name(kind)
        )),
    }
}

impl Mesh {
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self.verts.iter().map(|(_, v)| v.p);
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }

    pub fn pos(&self, id: u64) -> Option<Vec3> {
        self.verts.get(id).map(|v| v.p)
    }

    fn face_normal(&self, f: &Face) -> Vec3 {
        let p: Vec<Vec3> = f.v.iter().filter_map(|v| self.pos(*v)).collect();
        if p.len() < 3 {
            return Vec3::ZERO;
        }
        let mut n = Vec3::ZERO;
        for i in 1..p.len() - 1 {
            n += (p[i] - p[0]).cross(p[i + 1] - p[0]);
        }
        n
    }

    /// A normal's value: an automatic one is the area-weighted average of
    /// the faces that use it.
    pub fn normal_value(&self, id: u64) -> Option<Vec3> {
        let n = self.normals.get(id)?;
        if !n.auto {
            return Some(n.v);
        }
        let mut s = Vec3::ZERO;
        for (_, f) in self.faces.iter() {
            if f.n.contains(&id) {
                s += self.face_normal(f);
            }
        }
        Some(s.normalize_or_zero())
    }

    /// Triangles as (face id, [corner index; 3]) (quads split 0-1-2, 0-2-3).
    pub fn tris(&self) -> Vec<(u64, &Face, [usize; 3])> {
        let mut out = Vec::new();
        for (id, f) in self.faces.iter() {
            for i in 1..f.v.len().saturating_sub(1) {
                out.push((id, f, [0, i, i + 1]));
            }
        }
        out
    }

    fn vertex_default(&mut self, v: u64, which: usize) -> u64 {
        if let Some(id) = self.verts.get(v).and_then(|x| x.dflt[which]) {
            let alive = match which {
                0 => self.normals.get(id).is_some(),
                1 => self.uvs.get(id).is_some(),
                _ => self.colors.get(id).is_some(),
            };
            if alive {
                return id;
            }
        }
        let id = match which {
            0 => self.normals.add(Normal { v: Vec3::ZERO, auto: true }),
            1 => self.uvs.add(Vec2::ZERO),
            _ => self.colors.add(([1.0; 3], 1.0)),
        };
        if let Some(x) = self.verts.get_mut(v) {
            x.dflt[which] = Some(id);
        }
        id
    }

    pub fn add_face(&mut self, vs: &[u64]) -> mlua::Result<u64> {
        for v in vs {
            if self.verts.get(*v).is_none() {
                return Err(want(*v as f64, K_VERTEX));
            }
        }
        let n = vs.iter().map(|v| self.vertex_default(*v, 0)).collect();
        let uv = vs.iter().map(|v| self.vertex_default(*v, 1)).collect();
        let c = vs.iter().map(|v| self.vertex_default(*v, 2)).collect();
        Ok(self.faces.add(Face { v: vs.to_vec(), n, uv, c }))
    }

    pub fn add_vertex(&mut self, p: Vec3) -> u64 {
        self.verts.add(Vertex { p, dflt: [None; 3], bones: Vec::new(), weights: Vec::new() })
    }

    pub fn add_normal(&mut self, v: Option<Vec3>) -> u64 {
        self.normals.add(Normal { v: v.unwrap_or(Vec3::ZERO), auto: v.is_none() })
    }

    pub fn add_uv(&mut self, v: Vec2) -> u64 {
        self.uvs.add(v)
    }

    pub fn add_color(&mut self, c: [f32; 3], a: f32) -> u64 {
        self.colors.add((c, a))
    }

    /// Positions, normals, uvs and colours as triangle corners (for the
    /// renderer snapshot and CSG).
    pub fn corners(&self) -> Vec<(Vec3, Vec3, Vec2, [f32; 4])> {
        let mut out = Vec::new();
        for (_, f, k) in self.tris() {
            let Some(p) = k.iter().map(|&i| self.pos(f.v[i])).collect::<Option<Vec<_>>>() else { continue };
            let flat = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            for (j, &i) in k.iter().enumerate() {
                let n = f.n.get(i).and_then(|n| self.normal_value(*n)).filter(|n| n.length_squared() > 1e-12).unwrap_or(flat);
                let uv = f.uv.get(i).and_then(|u| self.uvs.get(*u).copied()).unwrap_or(Vec2::ZERO);
                let c = f.c.get(i).and_then(|c| self.colors.get(*c).copied()).unwrap_or(([1.0; 3], 1.0));
                out.push((p[j], n, uv, [c.0[0], c.0[1], c.0[2], c.1]));
            }
        }
        out
    }

    /// The mesh as binary 2.00 mesh bytes (one vertex per face corner) and
    /// its bounds.
    pub fn to_mesh_bytes(&self) -> (Vec<u8>, Vec3, Vec3) {
        let corners = self.corners();
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let mut verts: Vec<u8> = Vec::with_capacity(corners.len() * 40);
        for (p, n, uv, c) in &corners {
            lo = lo.min(*p);
            hi = hi.max(*p);
            for x in [p.x, p.y, p.z, n.x, n.y, n.z, uv.x, uv.y, 0.0] {
                verts.extend_from_slice(&x.to_le_bytes());
            }
            for x in c {
                verts.push((x.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        let nfaces = (corners.len() / 3) as u32;
        let mut out = b"version 2.00\n".to_vec();
        out.extend_from_slice(&12u16.to_le_bytes());
        out.push(40);
        out.push(12);
        out.extend_from_slice(&(nfaces * 3).to_le_bytes());
        out.extend_from_slice(&nfaces.to_le_bytes());
        out.extend_from_slice(&verts);
        for i in 0..nfaces * 3 {
            out.extend_from_slice(&i.to_le_bytes());
        }
        if nfaces == 0 {
            (lo, hi) = (Vec3::ZERO, Vec3::ZERO);
        }
        (out, lo, hi)
    }

    /// A mesh file's triangles: corners at one position share a vertex, as
    /// the file's indices do.
    pub fn from_mesh_data(d: &crate::mesh::MeshData) -> Mesh {
        let mut m = Mesh::default();
        let key3 = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        let mut vmap: HashMap<[u32; 3], u64> = HashMap::new();
        let mut nmap: HashMap<[u32; 3], u64> = HashMap::new();
        let mut umap: HashMap<[u32; 2], u64> = HashMap::new();
        let mut cmap: HashMap<[u32; 4], u64> = HashMap::new();
        for t in 0..d.positions.len() / 3 {
            let mut f = Face { v: vec![], n: vec![], uv: vec![], c: vec![] };
            for k in 0..3 {
                let i = t * 3 + k;
                let p = d.positions[i];
                let v = *vmap.entry(key3(p)).or_insert_with(|| m.add_vertex(p));
                f.v.push(v);
                let n = d.normals.get(i).copied().unwrap_or(Vec3::Y);
                f.n.push(*nmap.entry(key3(n)).or_insert_with(|| m.normals.add(Normal { v: n, auto: false })));
                let uv = d.uvs.get(i).copied().unwrap_or(Vec2::ZERO);
                f.uv.push(*umap.entry([uv.x.to_bits(), uv.y.to_bits()]).or_insert_with(|| m.uvs.add(uv)));
                let c = d.colors.get(i).copied().unwrap_or(glam::Vec4::ONE);
                f.c.push(*cmap.entry(c.to_array().map(f32::to_bits)).or_insert_with(|| m.colors.add(([c.x, c.y, c.z], c.w))));
            }
            for (k, v) in f.v.iter().enumerate() {
                if let Some(x) = m.verts.get_mut(*v) {
                    x.dflt[0].get_or_insert(f.n[k]);
                    x.dflt[1].get_or_insert(f.uv[k]);
                    x.dflt[2].get_or_insert(f.c[k]);
                }
            }
            m.faces.add(f);
        }
        m
    }
}

// ------------------------------------------------------------------ Lua

fn arg(args: &MultiValue, i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}

fn idv(v: &Value) -> f64 {
    num(v).unwrap_or(-1.0)
}

fn ud<T: Clone + 'static>(v: &Value) -> Option<T> {
    if let Some(x) = crate::ltypes::vec_any::<T>(v) {
        return Some(x);
    }
    match v {
        Value::UserData(u) => u.borrow::<T>().ok().map(|x| x.clone()),
        _ => None,
    }
}

fn v3(v: &Value) -> Option<Vec3> {
    ud::<LVector3>(v).map(|v| v.0)
}

fn v2(v: &Value) -> Option<Vec2> {
    ud::<LVector2>(v).map(|v| v.0)
}

fn cf(v: &Value) -> Option<Xform> {
    ud::<LCFrame>(v).map(|v| v.0)
}

fn enum_of(v: &Value) -> Option<LEnumItem> {
    ud::<LEnumItem>(v)
}

fn ids_of(v: &Value) -> Vec<f64> {
    match v {
        Value::Table(t) => t.clone().sequence_values::<Value>().filter_map(Result::ok).map(|v| idv(&v)).collect(),
        _ => Vec::new(),
    }
}

fn table_arg(v: &Value, name: &str, i: usize) -> mlua::Result<Table> {
    match v {
        Value::Table(t) => Ok(t.clone()),
        Value::Nil => Err(rt_err(format!("Argument {i} missing or nil"))),
        other => Err(rt_err(format!("Unable to cast {} to Array", lua_type_name(other)))).map_err(|e| {
            let _ = name;
            e
        }),
    }
}

fn lua_type_name(v: &Value) -> &'static str {
    match v {
        Value::Number(_) | Value::Integer(_) => "double",
        Value::String(_) => "string",
        Value::Boolean(_) => "bool",
        _ => "value",
    }
}

fn list(lua: &Lua, ids: impl IntoIterator<Item = u64>) -> mlua::Result<Value> {
    let t = lua.create_table()?;
    for (i, id) in ids.into_iter().enumerate() {
        t.raw_set(i + 1, id as f64)?;
    }
    Ok(Value::Table(t))
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([v]))
}

fn none() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}

fn v3v(lua: &Lua, v: Vec3) -> mlua::Result<Value> {
    Ok(LVector3(v).into_value())
}

fn v2v(lua: &Lua, v: Vec2) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(LVector2(v))?))
}

fn cfv(lua: &Lua, x: Xform) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(LCFrame(x))?))
}

fn colv(lua: &Lua, c: [f32; 3]) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(LColor3(c))?))
}

fn facs_value(lua: &Lua, v: u32) -> mlua::Result<Value> {
    match LEnumItem::from_value("FacsActionUnit", v) {
        Some(e) => enum_value(lua, e),
        None => Ok(Value::Number(v as f64)),
    }
}

#[allow(dead_code)]
pub const METHODS: &[(&str, &str)] = &[
    ("Destroy", "EditableMesh"),
    ("AddVertex", "EditableMesh"),
    ("AddNormal", "EditableMesh"),
    ("AddUV", "EditableMesh"),
    ("AddColor", "EditableMesh"),
    ("AddTriangle", "EditableMesh"),
    ("AddFace", "EditableMesh"),
    ("AddBone", "EditableMesh"),
    ("RemoveFace", "EditableMesh"),
    ("RemoveUnused", "EditableMesh"),
    ("RemoveBone", "EditableMesh"),
    ("Clear", "EditableMesh"),
    ("GetVertices", "EditableMesh"),
    ("GetFaces", "EditableMesh"),
    ("GetNormals", "EditableMesh"),
    ("GetUVs", "EditableMesh"),
    ("GetColors", "EditableMesh"),
    ("GetBones", "EditableMesh"),
    ("GetPosition", "EditableMesh"),
    ("SetPosition", "EditableMesh"),
    ("GetNormal", "EditableMesh"),
    ("SetNormal", "EditableMesh"),
    ("ResetNormal", "EditableMesh"),
    ("GetUV", "EditableMesh"),
    ("SetUV", "EditableMesh"),
    ("GetColor", "EditableMesh"),
    ("SetColor", "EditableMesh"),
    ("GetColorAlpha", "EditableMesh"),
    ("SetColorAlpha", "EditableMesh"),
    ("GetFaceVertices", "EditableMesh"),
    ("SetFaceVertices", "EditableMesh"),
    ("GetFaceNormals", "EditableMesh"),
    ("SetFaceNormals", "EditableMesh"),
    ("GetFaceUVs", "EditableMesh"),
    ("SetFaceUVs", "EditableMesh"),
    ("GetFaceColors", "EditableMesh"),
    ("SetFaceColors", "EditableMesh"),
    ("GetVertexFaces", "EditableMesh"),
    ("GetAdjacentVertices", "EditableMesh"),
    ("GetAdjacentFaces", "EditableMesh"),
    ("GetCenter", "EditableMesh"),
    ("GetSize", "EditableMesh"),
    ("RaycastLocal", "EditableMesh"),
    ("FindClosestVertex", "EditableMesh"),
    ("FindVerticesWithinSphere", "EditableMesh"),
    ("FindClosestPointOnSurface", "EditableMesh"),
    ("BatchAdd", "EditableMesh"),
    ("BatchSetValues", "EditableMesh"),
    ("BatchGetValues", "EditableMesh"),
    ("BatchRemove", "EditableMesh"),
    ("BatchGetFaceAttributes", "EditableMesh"),
    ("BatchGetVertexAttributes", "EditableMesh"),
    ("BatchGetVertexFaceAttributes", "EditableMesh"),
    ("BatchSetFaceAttributes", "EditableMesh"),
    ("BatchSetVertexFaceAttributes", "EditableMesh"),
    ("GetBoneByName", "EditableMesh"),
    ("GetBoneCFrame", "EditableMesh"),
    ("GetBoneIsVirtual", "EditableMesh"),
    ("GetBoneName", "EditableMesh"),
    ("GetBoneParent", "EditableMesh"),
    ("SetBoneCFrame", "EditableMesh"),
    ("SetBoneIsVirtual", "EditableMesh"),
    ("SetBoneName", "EditableMesh"),
    ("SetBoneParent", "EditableMesh"),
    ("GetVertexBones", "EditableMesh"),
    ("GetVertexBoneWeights", "EditableMesh"),
    ("SetVertexBones", "EditableMesh"),
    ("SetVertexBoneWeights", "EditableMesh"),
    ("GetFacesWithAttribute", "EditableMesh"),
    ("GetFacesWithColor", "EditableMesh"),
    ("GetFacesWithNormal", "EditableMesh"),
    ("GetFacesWithUV", "EditableMesh"),
    ("GetVerticesWithAttribute", "EditableMesh"),
    ("GetVerticesWithColor", "EditableMesh"),
    ("GetVerticesWithNormal", "EditableMesh"),
    ("GetVerticesWithUV", "EditableMesh"),
    ("GetVertexNormals", "EditableMesh"),
    ("GetVertexUVs", "EditableMesh"),
    ("GetVertexColors", "EditableMesh"),
    ("GetVertexFaceNormal", "EditableMesh"),
    ("GetVertexFaceUV", "EditableMesh"),
    ("GetVertexFaceColor", "EditableMesh"),
    ("SetVertexFaceNormal", "EditableMesh"),
    ("SetVertexFaceUV", "EditableMesh"),
    ("SetVertexFaceColor", "EditableMesh"),
    ("IdDebugString", "EditableMesh"),
    ("MergeVertices", "EditableMesh"),
    ("Triangulate", "EditableMesh"),
    ("GetFacsPose", "EditableMesh"),
    ("GetFacsPoses", "EditableMesh"),
    ("GetFacsCorrectivePose", "EditableMesh"),
    ("GetFacsCorrectivePoses", "EditableMesh"),
    ("SetFacsPose", "EditableMesh"),
    ("SetFacsBonePose", "EditableMesh"),
    ("SetFacsCorrectivePose", "EditableMesh"),
];

/// Which corner attribute list (0 normal, 1 uv, 2 colour) a kind is.
fn attr_slot(kind: u64) -> Option<usize> {
    match kind {
        K_NORMAL => Some(0),
        K_UV => Some(1),
        K_COLOR => Some(2),
        _ => None,
    }
}

fn corner_list(f: &Face, which: usize) -> &Vec<u64> {
    match which {
        0 => &f.n,
        1 => &f.uv,
        _ => &f.c,
    }
}

fn corner_list_mut(f: &mut Face, which: usize) -> &mut Vec<u64> {
    match which {
        0 => &mut f.n,
        1 => &mut f.uv,
        _ => &mut f.c,
    }
}

/// Enum.MeshAttribute: Vertex 0, Normal 1, Color 2, UV 3, Face 4 → kind.
fn mesh_attr_kind(v: &Value) -> mlua::Result<u64> {
    let e = enum_of(v).filter(|e| e.enum_type == "MeshAttribute").ok_or_else(|| rt_err("Enum.MeshAttribute expected"))?;
    Ok(match e.value {
        0 => K_VERTEX,
        1 => K_NORMAL,
        2 => K_COLOR,
        3 => K_UV,
        _ => K_FACE,
    })
}

impl Mesh {
    fn check(&self, id: f64, kind: u64) -> mlua::Result<u64> {
        let ok = match kind {
            K_VERTEX => self.verts.get(id as u64).is_some(),
            K_NORMAL => self.normals.get(id as u64).is_some(),
            K_UV => self.uvs.get(id as u64).is_some(),
            K_COLOR => self.colors.get(id as u64).is_some(),
            K_FACE => self.faces.get(id as u64).is_some(),
            _ => self.bones.get(id as u64).is_some(),
        };
        if ok && parse(id).is_some() {
            Ok(id as u64)
        } else {
            Err(want(id, kind))
        }
    }

    fn face(&self, id: f64) -> mlua::Result<&Face> {
        let id = self.check(id, K_FACE)?;
        Ok(self.faces.get(id).expect("checked"))
    }

    fn bone(&self, id: f64) -> mlua::Result<&Bone> {
        let id = self.check(id, K_BONE)?;
        Ok(self.bones.get(id).expect("checked"))
    }

    fn corner(&self, v: f64, f: f64) -> mlua::Result<(u64, usize)> {
        let v = self.check(v, K_VERTEX)?;
        let fid = self.check(f, K_FACE)?;
        let face = self.faces.get(fid).expect("checked");
        let k = face.v.iter().position(|x| *x == v).ok_or_else(|| rt_err("Vertex is not part of face"))?;
        Ok((fid, k))
    }

    /// Faces / vertices whose corners use `id` (an attribute of any kind).
    fn users(&self, id: u64) -> (Vec<u64>, Vec<u64>) {
        let Some((kind, _, _)) = parse(id as f64) else { return (vec![], vec![]) };
        let Some(which) = attr_slot(kind) else { return (vec![], vec![]) };
        let mut faces = Vec::new();
        let mut verts: Vec<u64> = Vec::new();
        for (fid, f) in self.faces.iter() {
            let l = corner_list(f, which);
            let mut hit = false;
            for (k, x) in l.iter().enumerate() {
                if *x == id {
                    hit = true;
                    if !verts.contains(&f.v[k]) {
                        verts.push(f.v[k]);
                    }
                }
            }
            if hit {
                faces.push(fid);
            }
        }
        (faces, verts)
    }

    fn vertex_corners(&self, v: u64, which: usize) -> Vec<u64> {
        let mut out = Vec::new();
        for (_, f) in self.faces.iter() {
            for (k, x) in f.v.iter().enumerate() {
                if *x == v {
                    out.push(corner_list(f, which)[k]);
                }
            }
        }
        out
    }

    fn check_virtual(&self, bone: u64) -> mlua::Result<()> {
        let b = self.bones.get(bone).expect("checked");
        if !b.virt {
            return Err(rt_err(format!(
                "Only virtual bones can be used in Facs Poses, unable to use bone \"{}\", id {}",
                b.name, bone
            )));
        }
        Ok(())
    }

    fn closest_on_tri(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, Vec3) {
        // Ericson, Real-Time Collision Detection 5.1.5, with barycentrics.
        let ab = b - a;
        let ac = c - a;
        let ap = p - a;
        let d1 = ab.dot(ap);
        let d2 = ac.dot(ap);
        if d1 <= 0.0 && d2 <= 0.0 {
            return (a, Vec3::X);
        }
        let bp = p - b;
        let d3 = ab.dot(bp);
        let d4 = ac.dot(bp);
        if d3 >= 0.0 && d4 <= d3 {
            return (b, Vec3::Y);
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let v = d1 / (d1 - d3);
            return (a + ab * v, Vec3::new(1.0 - v, v, 0.0));
        }
        let cp = p - c;
        let d5 = ab.dot(cp);
        let d6 = ac.dot(cp);
        if d6 >= 0.0 && d5 <= d6 {
            return (c, Vec3::Z);
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let w = d2 / (d2 - d6);
            return (a + ac * w, Vec3::new(1.0 - w, 0.0, w));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            return (b + (c - b) * w, Vec3::new(0.0, 1.0 - w, w));
        }
        let denom = 1.0 / (va + vb + vc);
        let v = vb * denom;
        let w = vc * denom;
        (a + ab * v + ac * w, Vec3::new(1.0 - v - w, v, w))
    }
}

fn ray_tri(o: Vec3, d: Vec3, t: [Vec3; 3]) -> Option<(f32, Vec3)> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - t[0];
    let u = s.dot(p) * inv;
    if !(-1e-6..=1.0 + 1e-6).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -1e-6 || u + v > 1.0 + 1e-6 {
        return None;
    }
    let tt = e2.dot(q) * inv;
    (tt >= 0.0).then_some((tt, Vec3::new(1.0 - u - v, u, v)))
}

/// One EditableMesh method on `m`. Returns Ok(None) for a name that isn't
/// an EditableMesh method.
pub fn call(lua: &Lua, m: &mut Mesh, name: &str, args: &MultiValue) -> mlua::Result<Option<MultiValue>> {
    let a1 = arg(args, 1);
    let a2 = arg(args, 2);
    let a3 = arg(args, 3);
    let id1 = idv(&a1);
    let r = match name {
        "AddVertex" => {
            let p = v3(&a1).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            one(Value::Number(m.add_vertex(p) as f64))
        }
        "AddNormal" => {
            let n = v3(&a1);
            one(Value::Number(m.add_normal(n) as f64))
        }
        "AddUV" => {
            let p = v2(&a1).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            one(Value::Number(m.add_uv(p) as f64))
        }
        "AddColor" => {
            let c = ud::<LColor3>(&a1).map(|c| c.0).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let a = num(&a2).ok_or_else(|| rt_err("Argument 2 missing or nil"))? as f32;
            one(Value::Number(m.add_color(c, a) as f64))
        }
        "AddTriangle" => {
            let vs = [id1, idv(&a2), idv(&a3)];
            let mut ok = Vec::with_capacity(3);
            for v in vs {
                ok.push(m.check(v, K_VERTEX)?);
            }
            one(Value::Number(m.add_face(&ok)? as f64))
        }
        "AddFace" => {
            let t = table_arg(&a1, name, 1)?;
            let vs: Vec<f64> = ids_of(&Value::Table(t));
            if vs.len() != 3 && vs.len() != 4 {
                return Err(rt_err(format!(
                    "AddFace called with {} vertices, currently only supports 3 or 4 vertices",
                    vs.len()
                )));
            }
            let mut ok = Vec::with_capacity(4);
            for v in vs {
                ok.push(m.check(v, K_VERTEX)?);
            }
            one(Value::Number(m.add_face(&ok)? as f64))
        }
        "RemoveFace" => {
            let f = m.check(id1, K_FACE)?;
            m.faces.remove(f);
            none()
        }
        "BatchRemove" => {
            let ids = ids_of(&a1);
            let mut ok = Vec::new();
            for id in ids {
                ok.push(m.check(id, K_FACE)?);
            }
            for f in ok {
                m.faces.remove(f);
            }
            none()
        }
        "RemoveUnused" => {
            let mut used: HashSet<u64> = HashSet::new();
            for (_, f) in m.faces.iter() {
                used.extend(f.v.iter().chain(&f.n).chain(&f.uv).chain(&f.c));
            }
            let mut removed = Vec::new();
            for id in m.verts.ids() {
                if !used.contains(&id) {
                    m.verts.remove(id);
                    removed.push(id);
                }
            }
            for id in m.normals.ids() {
                if !used.contains(&id) {
                    m.normals.remove(id);
                    removed.push(id);
                }
            }
            for id in m.uvs.ids() {
                if !used.contains(&id) {
                    m.uvs.remove(id);
                    removed.push(id);
                }
            }
            for id in m.colors.ids() {
                if !used.contains(&id) {
                    m.colors.remove(id);
                    removed.push(id);
                }
            }
            one(list(lua, removed)?)
        }
        "Clear" => {
            let (n, gen, fixed) = (m.n, m.gen, m.fixed);
            *m = Mesh { n, gen, fixed, ..Default::default() };
            none()
        }
        "GetVertices" => one(list(lua, m.verts.ids())?),
        "GetFaces" => one(list(lua, m.faces.ids())?),
        "GetNormals" => one(list(lua, m.normals.ids())?),
        "GetUVs" => one(list(lua, m.uvs.ids())?),
        "GetColors" => one(list(lua, m.colors.ids())?),
        "GetBones" => one(list(lua, m.bones.ids())?),
        "GetPosition" => {
            let v = m.check(id1, K_VERTEX)?;
            one(v3v(lua, m.pos(v).unwrap_or(Vec3::ZERO))?)
        }
        "SetPosition" => {
            let v = m.check(id1, K_VERTEX)?;
            let p = v3(&a2).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            m.verts.get_mut(v).expect("checked").p = p;
            none()
        }
        "GetNormal" => {
            let n = m.check(id1, K_NORMAL)?;
            one(v3v(lua, m.normal_value(n).unwrap_or(Vec3::ZERO))?)
        }
        "SetNormal" => {
            let n = m.check(id1, K_NORMAL)?;
            let v = v3(&a2).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            *m.normals.get_mut(n).expect("checked") = Normal { v, auto: false };
            none()
        }
        "ResetNormal" => {
            let n = m.check(id1, K_NORMAL)?;
            m.normals.get_mut(n).expect("checked").auto = true;
            none()
        }
        "GetUV" => {
            let u = m.check(id1, K_UV)?;
            one(v2v(lua, *m.uvs.get(u).expect("checked"))?)
        }
        "SetUV" => {
            let u = m.check(id1, K_UV)?;
            let v = v2(&a2).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            *m.uvs.get_mut(u).expect("checked") = v;
            none()
        }
        "GetColor" => {
            let c = m.check(id1, K_COLOR)?;
            one(colv(lua, m.colors.get(c).expect("checked").0)?)
        }
        "GetColorAlpha" => {
            let c = m.check(id1, K_COLOR)?;
            one(Value::Number(m.colors.get(c).expect("checked").1 as f64))
        }
        "SetColor" => {
            let c = m.check(id1, K_COLOR)?;
            let v = ud::<LColor3>(&a2).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            m.colors.get_mut(c).expect("checked").0 = v.0;
            none()
        }
        "SetColorAlpha" => {
            let c = m.check(id1, K_COLOR)?;
            let a = num(&a2).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            m.colors.get_mut(c).expect("checked").1 = a as f32;
            none()
        }
        "GetFaceVertices" | "GetFaceNormals" | "GetFaceUVs" | "GetFaceColors" => {
            let f = m.face(id1)?;
            let l = match name {
                "GetFaceVertices" => &f.v,
                "GetFaceNormals" => &f.n,
                "GetFaceUVs" => &f.uv,
                _ => &f.c,
            };
            one(list(lua, l.clone())?)
        }
        "SetFaceVertices" | "SetFaceNormals" | "SetFaceUVs" | "SetFaceColors" => {
            let fid = m.check(id1, K_FACE)?;
            let kind = match name {
                "SetFaceVertices" => K_VERTEX,
                "SetFaceNormals" => K_NORMAL,
                "SetFaceUVs" => K_UV,
                _ => K_COLOR,
            };
            let ids = ids_of(&a2);
            let n = m.faces.get(fid).expect("checked").v.len();
            if ids.len() != n {
                return Err(rt_err(format!("Expected {n} ids, received {}", ids.len())));
            }
            let mut ok = Vec::with_capacity(n);
            for id in ids {
                ok.push(m.check(id, kind)?);
            }
            let f = m.faces.get_mut(fid).expect("checked");
            match kind {
                K_VERTEX => f.v = ok,
                K_NORMAL => f.n = ok,
                K_UV => f.uv = ok,
                _ => f.c = ok,
            }
            none()
        }
        "GetVertexFaces" => {
            let v = m.check(id1, K_VERTEX)?;
            one(list(lua, m.faces.iter().filter(|(_, f)| f.v.contains(&v)).map(|(i, _)| i))?)
        }
        "GetAdjacentVertices" => {
            let v = m.check(id1, K_VERTEX)?;
            let mut out: Vec<u64> = Vec::new();
            for (_, f) in m.faces.iter() {
                let n = f.v.len();
                for k in 0..n {
                    if f.v[k] == v {
                        for x in [f.v[(k + n - 1) % n], f.v[(k + 1) % n]] {
                            if x != v && !out.contains(&x) {
                                out.push(x);
                            }
                        }
                    }
                }
            }
            one(list(lua, out)?)
        }
        "GetAdjacentFaces" => {
            let fid = m.check(id1, K_FACE)?;
            let f = m.faces.get(fid).expect("checked");
            let edges = |f: &Face| -> Vec<(u64, u64)> {
                let n = f.v.len();
                (0..n).map(|k| (f.v[k], f.v[(k + 1) % n])).collect()
            };
            let mine = edges(f);
            let out: Vec<u64> = m
                .faces
                .iter()
                .filter(|(id, g)| *id != fid && edges(g).iter().any(|(a, b)| mine.contains(&(*b, *a))))
                .map(|(id, _)| id)
                .collect();
            one(list(lua, out)?)
        }
        "GetCenter" => one(v3v(lua, m.bounds().map(|(lo, hi)| (lo + hi) * 0.5).unwrap_or(Vec3::ZERO))?),
        "GetSize" => one(v3v(lua, m.bounds().map(|(lo, hi)| hi - lo).unwrap_or(Vec3::ZERO))?),
        "RaycastLocal" => {
            let o = v3(&a1).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let d = v3(&a2).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            let mut best: Option<(f32, u64, Vec3)> = None;
            for (fid, f, k) in m.tris() {
                let Some(p) = k.iter().map(|&i| m.pos(f.v[i])).collect::<Option<Vec<_>>>() else { continue };
                if let Some((t, bary)) = ray_tri(o, d, [p[0], p[1], p[2]]) {
                    if best.is_none_or(|b| t < b.0) {
                        best = Some((t, fid, bary));
                    }
                }
            }
            match best {
                None => one(Value::Nil),
                Some((t, f, b)) => Ok(MultiValue::from_iter([Value::Number(f as f64), v3v(lua, o + d * t)?, v3v(lua, b)?])),
            }
        }
        "FindClosestPointOnSurface" => {
            let p = v3(&a1).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let mut best: Option<(f32, u64, Vec3, Vec3, [u64; 3])> = None;
            for (fid, f, k) in m.tris() {
                let Some(q) = k.iter().map(|&i| m.pos(f.v[i])).collect::<Option<Vec<_>>>() else { continue };
                let (c, bary) = Mesh::closest_on_tri(p, q[0], q[1], q[2]);
                let d = c.distance_squared(p);
                // Ties go to the later face (Studio picks the newest).
                if best.is_none_or(|b| d <= b.0) {
                    best = Some((d, fid, c, bary, [f.v[k[0]], f.v[k[1]], f.v[k[2]]]));
                }
            }
            match best {
                None => one(Value::Nil),
                Some((_, f, c, b, vs)) => Ok(MultiValue::from_iter([
                    Value::Number(f as f64),
                    v3v(lua, c)?,
                    v3v(lua, b)?,
                    Value::Number(vs[0] as f64),
                    Value::Number(vs[1] as f64),
                    Value::Number(vs[2] as f64),
                ])),
            }
        }
        "FindClosestVertex" => {
            let p = v3(&a1).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let best = m.verts.iter().map(|(id, v)| (id, v.p.distance_squared(p))).min_by(|a, b| a.1.total_cmp(&b.1));
            one(Value::Number(best.map(|b| b.0).unwrap_or(0) as f64))
        }
        "FindVerticesWithinSphere" => {
            let c = v3(&a1).ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let r = num(&a2).unwrap_or(0.0) as f32;
            one(list(lua, m.verts.iter().filter(|(_, v)| v.p.distance(c) <= r).map(|(i, _)| i))?)
        }
        "BatchAdd" => {
            let kind = mesh_attr_kind(&a1)?;
            let t = table_arg(&a2, name, 2)?;
            let vals: Vec<Value> = t.sequence_values::<Value>().filter_map(Result::ok).collect();
            let mut out = Vec::with_capacity(vals.len());
            for v in &vals {
                let id = match kind {
                    K_VERTEX => m.add_vertex(v3(v).ok_or_else(|| rt_err("BatchAdd: Vector3 expected"))?),
                    K_NORMAL => m.add_normal(v3(v)),
                    K_UV => m.add_uv(v2(v).ok_or_else(|| rt_err("BatchAdd: Vector2 expected"))?),
                    K_COLOR => m.add_color(ud::<LColor3>(v).map(|c| c.0).ok_or_else(|| rt_err("BatchAdd: Color3 expected"))?, 1.0),
                    _ => {
                        let vs = ids_of(v);
                        if vs.len() != 3 && vs.len() != 4 {
                            return Err(rt_err(format!(
                                "AddFace called with {} vertices, currently only supports 3 or 4 vertices",
                                vs.len()
                            )));
                        }
                        let mut ok = Vec::new();
                        for x in vs {
                            ok.push(m.check(x, K_VERTEX)?);
                        }
                        m.add_face(&ok)?
                    }
                };
                out.push(id);
            }
            one(list(lua, out)?)
        }
        "BatchSetValues" => {
            let ids = ids_of(&a1);
            let vals: Vec<Value> = match &a2 {
                Value::Table(t) => t.clone().sequence_values::<Value>().filter_map(Result::ok).collect(),
                _ => Vec::new(),
            };
            if ids.len() != vals.len() {
                return Err(rt_err("Number of ids and values must match"));
            }
            for (id, v) in ids.iter().zip(&vals) {
                let kind = parse(*id).map(|p| p.0).unwrap_or(0);
                let id = m.check(*id, if kind == 0 { K_VERTEX } else { kind })?;
                match kind {
                    K_VERTEX => m.verts.get_mut(id).expect("checked").p = v3(v).ok_or_else(|| rt_err("Vector3 expected"))?,
                    K_NORMAL => *m.normals.get_mut(id).expect("checked") = Normal { v: v3(v).ok_or_else(|| rt_err("Vector3 expected"))?, auto: false },
                    K_UV => *m.uvs.get_mut(id).expect("checked") = v2(v).ok_or_else(|| rt_err("Vector2 expected"))?,
                    K_COLOR => m.colors.get_mut(id).expect("checked").0 = ud::<LColor3>(v).ok_or_else(|| rt_err("Color3 expected"))?.0,
                    _ => {
                        let vs = ids_of(v);
                        let mut ok = Vec::new();
                        for x in vs {
                            ok.push(m.check(x, K_VERTEX)?);
                        }
                        m.faces.get_mut(id).expect("checked").v = ok;
                    }
                }
            }
            none()
        }
        "BatchGetValues" => {
            let ids = ids_of(&a1);
            let t = lua.create_table()?;
            for (i, id) in ids.iter().enumerate() {
                let kind = parse(*id).map(|p| p.0).unwrap_or(0);
                let id = m.check(*id, if kind == 0 { K_VERTEX } else { kind })?;
                let v = match kind {
                    K_VERTEX => v3v(lua, m.pos(id).unwrap_or(Vec3::ZERO))?,
                    K_NORMAL => v3v(lua, m.normal_value(id).unwrap_or(Vec3::ZERO))?,
                    K_UV => v2v(lua, *m.uvs.get(id).expect("checked"))?,
                    K_COLOR => colv(lua, m.colors.get(id).expect("checked").0)?,
                    K_FACE => list(lua, m.faces.get(id).expect("checked").v.clone())?,
                    _ => Value::Nil,
                };
                t.raw_set(i + 1, v)?;
            }
            one(Value::Table(t))
        }
        "BatchGetVertexAttributes" => {
            let kind = mesh_attr_kind(&a1)?;
            let t = lua.create_table()?;
            for (i, v) in ids_of(&a2).into_iter().enumerate() {
                let v = m.check(v, K_VERTEX)?;
                let x = match attr_slot(kind) {
                    Some(w) => list(lua, m.vertex_corners(v, w))?,
                    None if kind == K_FACE => list(lua, m.faces.iter().filter(|(_, f)| f.v.contains(&v)).map(|(i, _)| i))?,
                    None => v3v(lua, m.pos(v).unwrap_or(Vec3::ZERO))?,
                };
                t.raw_set(i + 1, x)?;
            }
            one(Value::Table(t))
        }
        "BatchGetFaceAttributes" => {
            let kind = mesh_attr_kind(&a1)?;
            let t = lua.create_table()?;
            for (i, f) in ids_of(&a2).into_iter().enumerate() {
                let f = m.face(f)?;
                let l = match attr_slot(kind) {
                    Some(w) => corner_list(f, w).clone(),
                    None => f.v.clone(),
                };
                t.raw_set(i + 1, list(lua, l)?)?;
            }
            one(Value::Table(t))
        }
        "BatchGetVertexFaceAttributes" => {
            let kind = mesh_attr_kind(&a1)?;
            let vs = ids_of(&a2);
            let fs = ids_of(&a3);
            if vs.len() != fs.len() {
                return Err(rt_err("Number of vertex ids and face ids must match"));
            }
            let mut out = Vec::new();
            for (v, f) in vs.into_iter().zip(fs) {
                let (fid, k) = m.corner(v, f)?;
                let face = m.faces.get(fid).expect("checked");
                out.push(match attr_slot(kind) {
                    Some(w) => corner_list(face, w)[k],
                    None => face.v[k],
                });
            }
            one(list(lua, out)?)
        }
        "BatchSetFaceAttributes" => {
            let fs = ids_of(&a1);
            let arrays: Vec<Value> = match &a2 {
                Value::Table(t) => t.clone().sequence_values::<Value>().filter_map(Result::ok).collect(),
                _ => Vec::new(),
            };
            if fs.len() != arrays.len() {
                return Err(rt_err("Number of face ids and attribute id arrays must match"));
            }
            for (f, arr) in fs.into_iter().zip(arrays) {
                let fid = m.check(f, K_FACE)?;
                let ids = ids_of(&arr);
                let n = m.faces.get(fid).expect("checked").v.len();
                if ids.len() != n {
                    return Err(rt_err(format!("Expected {n} ids, received {}", ids.len())));
                }
                let kind = parse(ids[0]).map(|p| p.0).unwrap_or(0);
                let mut ok = Vec::new();
                for id in &ids {
                    ok.push(m.check(*id, if kind == 0 { K_NORMAL } else { kind })?);
                }
                let face = m.faces.get_mut(fid).expect("checked");
                match attr_slot(kind) {
                    Some(w) => *corner_list_mut(face, w) = ok,
                    None => face.v = ok,
                }
            }
            none()
        }
        "BatchSetVertexFaceAttributes" => {
            let vs = ids_of(&a1);
            let fs = ids_of(&a2);
            let xs = ids_of(&a3);
            if vs.len() != fs.len() || vs.len() != xs.len() {
                return Err(rt_err("Number of vertex ids, face ids and attribute ids must match"));
            }
            for ((v, f), x) in vs.into_iter().zip(fs).zip(xs) {
                let (fid, k) = m.corner(v, f)?;
                let kind = parse(x).map(|p| p.0).unwrap_or(0);
                let which = attr_slot(kind).ok_or_else(|| want(x, K_NORMAL))?;
                let x = m.check(x, kind)?;
                corner_list_mut(m.faces.get_mut(fid).expect("checked"), which)[k] = x;
            }
            none()
        }
        "GetVertexFaceNormal" | "GetVertexFaceUV" | "GetVertexFaceColor" => {
            let (fid, k) = m.corner(id1, idv(&a2))?;
            let w = match name {
                "GetVertexFaceNormal" => 0,
                "GetVertexFaceUV" => 1,
                _ => 2,
            };
            one(Value::Number(corner_list(m.faces.get(fid).expect("checked"), w)[k] as f64))
        }
        "SetVertexFaceNormal" | "SetVertexFaceUV" | "SetVertexFaceColor" => {
            let (fid, k) = m.corner(id1, idv(&a2))?;
            let (w, kind) = match name {
                "SetVertexFaceNormal" => (0, K_NORMAL),
                "SetVertexFaceUV" => (1, K_UV),
                _ => (2, K_COLOR),
            };
            let x = m.check(idv(&a3), kind)?;
            corner_list_mut(m.faces.get_mut(fid).expect("checked"), w)[k] = x;
            none()
        }
        "GetVertexNormals" | "GetVertexUVs" | "GetVertexColors" => {
            let v = m.check(id1, K_VERTEX)?;
            let w = match name {
                "GetVertexNormals" => 0,
                "GetVertexUVs" => 1,
                _ => 2,
            };
            one(list(lua, m.vertex_corners(v, w))?)
        }
        "GetFacesWithColor" | "GetFacesWithNormal" | "GetFacesWithUV" | "GetVerticesWithColor"
        | "GetVerticesWithNormal" | "GetVerticesWithUV" => {
            let kind = if name.ends_with("Color") {
                K_COLOR
            } else if name.ends_with("Normal") {
                K_NORMAL
            } else {
                K_UV
            };
            let id = m.check(id1, kind)?;
            let (faces, verts) = m.users(id);
            one(list(lua, if name.starts_with("GetFaces") { faces } else { verts })?)
        }
        "GetFacesWithAttribute" | "GetVerticesWithAttribute" => {
            let kind = parse(id1).map(|p| p.0).filter(|k| attr_slot(*k).is_some()).ok_or_else(|| want(id1, K_NORMAL))?;
            let id = m.check(id1, kind)?;
            let (faces, verts) = m.users(id);
            one(list(lua, if name.starts_with("GetFaces") { faces } else { verts })?)
        }
        "IdDebugString" => {
            let s = match parse(id1) {
                Some((k, slot, g)) if g > 1 => format!("{}{},{}", kind_letter(k), slot + 1, g),
                Some((k, slot, _)) => format!("{}{}", kind_letter(k), slot + 1),
                None => format!("?{}", id1 as i64),
            };
            one(Value::String(lua.create_string(&s)?))
        }
        "MergeVertices" => {
            let tol = num(&a1).ok_or_else(|| rt_err("Argument 1 missing or nil"))? as f32;
            let vs: Vec<(u64, Vec3)> = m.verts.iter().map(|(i, v)| (i, v.p)).collect();
            let mut map: Vec<(u64, u64)> = Vec::new();
            let mut gone: HashSet<u64> = HashSet::new();
            for (i, (a, pa)) in vs.iter().enumerate() {
                if gone.contains(a) {
                    continue;
                }
                for (b, pb) in &vs[i + 1..] {
                    if !gone.contains(b) && pa.distance(*pb) <= tol {
                        gone.insert(*b);
                        map.push((*b, *a));
                    }
                }
            }
            let lookup: HashMap<u64, u64> = map.iter().copied().collect();
            for s in m.faces.order.clone() {
                if let Some(f) = m.faces.items[s as usize].as_mut() {
                    for v in f.v.iter_mut() {
                        if let Some(t) = lookup.get(v) {
                            *v = *t;
                        }
                    }
                }
            }
            let t = lua.create_table()?;
            for (a, b) in map {
                t.raw_set(a as f64, b as f64)?;
            }
            one(Value::Table(t))
        }
        "Triangulate" => {
            for fid in m.faces.ids() {
                let f = m.faces.get(fid).expect("listed").clone();
                if f.v.len() != 4 {
                    continue;
                }
                let pick = |l: &Vec<u64>, k: [usize; 3]| k.iter().map(|&i| l[i]).collect::<Vec<_>>();
                let tri = |k: [usize; 3]| Face { v: pick(&f.v, k), n: pick(&f.n, k), uv: pick(&f.uv, k), c: pick(&f.c, k) };
                m.faces.add(tri([0, 1, 2]));
                m.faces.remove(fid);
                m.faces.add(tri([2, 3, 0]));
            }
            none()
        }
        // ---------------------------------------------------------- bones
        "AddBone" => {
            let t = match &a1 {
                Value::Table(t) => t.clone(),
                _ => return Err(rt_err("Argument 1 missing or nil")),
            };
            for pair in t.clone().pairs::<Value, Value>() {
                let (k, _) = pair?;
                let k = match &k {
                    Value::String(s) => s.to_str()?.to_string(),
                    other => format!("{other:?}"),
                };
                if !matches!(k.as_str(), "Name" | "CFrame" | "Virtual" | "ParentId") {
                    return Err(rt_err(format!("Unexpected entry in table: {k}")));
                }
            }
            let name = match t.get::<Value>("Name")? {
                Value::String(s) => s.to_str()?.to_string(),
                _ => return Err(rt_err("Missing bone Name value")),
            };
            let cframe = cf(&t.get::<Value>("CFrame")?).ok_or_else(|| rt_err("Missing bone CFrame value"))?;
            let virt = match t.get::<Value>("Virtual")? {
                Value::Boolean(b) => b,
                _ => return Err(rt_err("Missing bone Virtual value")),
            };
            let parent = match t.get::<Value>("ParentId")? {
                Value::Nil => 0,
                v => m.check(idv(&v), K_BONE)?,
            };
            one(Value::Number(m.bones.add(Bone { name, cf: cframe, parent, virt }) as f64))
        }
        "RemoveBone" => {
            let b = m.check(id1, K_BONE)?;
            m.bones.remove(b);
            for s in m.bones.order.clone() {
                if let Some(x) = m.bones.items[s as usize].as_mut() {
                    if x.parent == b {
                        x.parent = 0;
                    }
                }
            }
            for s in m.verts.order.clone() {
                if let Some(v) = m.verts.items[s as usize].as_mut() {
                    if let Some(i) = v.bones.iter().position(|x| *x == b) {
                        v.bones.remove(i);
                        if i < v.weights.len() {
                            v.weights.remove(i);
                        }
                    }
                }
            }
            for pose in m.facs.values_mut() {
                pose.retain(|(x, _)| *x != b);
            }
            for (_, pose) in m.correctives.iter_mut() {
                pose.retain(|(x, _)| *x != b);
            }
            none()
        }
        "GetBoneByName" => {
            let s = match &a1 {
                Value::String(s) => s.to_str()?.to_string(),
                _ => return Err(rt_err("Argument 1 missing or nil")),
            };
            let b = m.bones.iter().find(|(_, b)| b.name == s).map(|(i, _)| i);
            one(Value::Number(b.ok_or_else(|| rt_err(format!("No bone named \"{s}\"")))? as f64))
        }
        "GetBoneCFrame" => one(cfv(lua, m.bone(id1)?.cf)?),
        "GetBoneIsVirtual" => one(Value::Boolean(m.bone(id1)?.virt)),
        "GetBoneName" => one(Value::String(lua.create_string(&m.bone(id1)?.name)?)),
        "GetBoneParent" => one(Value::Number(m.bone(id1)?.parent as f64)),
        "SetBoneCFrame" => {
            let b = m.check(id1, K_BONE)?;
            let x = cf(&a2).ok_or_else(|| rt_err("Argument 2 missing or nil"))?;
            m.bones.get_mut(b).expect("checked").cf = x;
            none()
        }
        "SetBoneIsVirtual" => {
            let b = m.check(id1, K_BONE)?;
            let v = matches!(a2, Value::Boolean(true));
            m.bones.get_mut(b).expect("checked").virt = v;
            none()
        }
        "SetBoneName" => {
            let b = m.check(id1, K_BONE)?;
            let s = match &a2 {
                Value::String(s) => s.to_str()?.to_string(),
                _ => return Err(rt_err("Argument 2 missing or nil")),
            };
            m.bones.get_mut(b).expect("checked").name = s;
            none()
        }
        "SetBoneParent" => {
            let b = m.check(id1, K_BONE)?;
            let p = idv(&a2);
            let p = if p == 0.0 { 0 } else { m.check(p, K_BONE)? };
            // A bone can't become its own ancestor.
            let mut at = p;
            while at != 0 {
                if at == b {
                    return Err(rt_err("Setting this parent would create a cycle"));
                }
                at = m.bones.get(at).map(|x| x.parent).unwrap_or(0);
            }
            m.bones.get_mut(b).expect("checked").parent = p;
            none()
        }
        "GetVertexBones" => {
            let v = m.check(id1, K_VERTEX)?;
            one(list(lua, m.verts.get(v).expect("checked").bones.clone())?)
        }
        "GetVertexBoneWeights" => {
            let v = m.check(id1, K_VERTEX)?;
            let t = lua.create_table()?;
            for (i, w) in m.verts.get(v).expect("checked").weights.iter().enumerate() {
                t.raw_set(i + 1, *w as f64 / 255.0)?;
            }
            one(Value::Table(t))
        }
        "SetVertexBones" => {
            let v = m.check(id1, K_VERTEX)?;
            let mut ok = Vec::new();
            for b in ids_of(&a2) {
                ok.push(m.check(b, K_BONE)?);
            }
            let x = m.verts.get_mut(v).expect("checked");
            x.weights.resize(ok.len(), 0);
            x.bones = ok;
            none()
        }
        "SetVertexBoneWeights" => {
            let v = m.check(id1, K_VERTEX)?;
            let ws: Vec<f64> = match &a2 {
                Value::Table(t) => t.clone().sequence_values::<f64>().filter_map(Result::ok).collect(),
                _ => Vec::new(),
            };
            let x = m.verts.get_mut(v).expect("checked");
            if ws.len() != x.bones.len() {
                return Err(rt_err("Number of bone weights provided does not match number of bone ids"));
            }
            x.weights = ws.iter().map(|w| (w.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            none()
        }
        // ----------------------------------------------------------- FACS
        "GetFacsPoses" => {
            let t = lua.create_table()?;
            for (i, a) in m.facs.iter().filter(|(_, p)| !p.is_empty()).map(|(a, _)| *a).enumerate() {
                t.raw_set(i + 1, facs_value(lua, a)?)?;
            }
            one(Value::Table(t))
        }
        "GetFacsPose" => {
            let a = enum_of(&a1).filter(|e| e.enum_type == "FacsActionUnit").ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let pose = m.facs.get(&a.value).cloned().unwrap_or_default();
            pose_out(lua, &pose)
        }
        "SetFacsPose" => {
            let a = enum_of(&a1).filter(|e| e.enum_type == "FacsActionUnit").ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let pose = pose_in(m, &a2, &a3)?;
            m.facs.insert(a.value, pose);
            none()
        }
        "SetFacsBonePose" => {
            let a = enum_of(&a1).filter(|e| e.enum_type == "FacsActionUnit").ok_or_else(|| rt_err("Argument 1 missing or nil"))?;
            let b = m.check(idv(&a2), K_BONE)?;
            m.check_virtual(b)?;
            let x = cf(&a3).ok_or_else(|| rt_err("Argument 3 missing or nil"))?;
            let pose = m.facs.entry(a.value).or_default();
            match pose.iter_mut().find(|(y, _)| *y == b) {
                Some(p) => p.1 = x,
                None => pose.push((b, x)),
            }
            none()
        }
        "GetFacsCorrectivePoses" => {
            let t = lua.create_table()?;
            for (i, (acts, _)) in m.correctives.iter().enumerate() {
                let inner = lua.create_table()?;
                for (j, a) in acts.iter().enumerate() {
                    inner.raw_set(j + 1, facs_value(lua, *a)?)?;
                }
                t.raw_set(i + 1, inner)?;
            }
            one(Value::Table(t))
        }
        "GetFacsCorrectivePose" => {
            let acts = corrective_key(&a1)?;
            let pose = m.correctives.iter().find(|(k, _)| *k == acts).map(|(_, p)| p.clone()).unwrap_or_default();
            pose_out(lua, &pose)
        }
        "SetFacsCorrectivePose" => {
            let acts = corrective_key(&a1)?;
            let pose = pose_in(m, &a2, &a3)?;
            match m.correctives.iter_mut().find(|(k, _)| *k == acts) {
                Some(c) => c.1 = pose,
                None => m.correctives.push((acts, pose)),
            }
            none()
        }
        _ => return Ok(None),
    };
    r.map(Some)
}

fn corrective_key(v: &Value) -> mlua::Result<Vec<u32>> {
    let mut acts: Vec<u32> = match v {
        Value::Table(t) => t
            .clone()
            .sequence_values::<Value>()
            .filter_map(Result::ok)
            .filter_map(|v| enum_of(&v).filter(|e| e.enum_type == "FacsActionUnit").map(|e| e.value))
            .collect(),
        _ => Vec::new(),
    };
    if !(2..=3).contains(&acts.len()) {
        return Err(rt_err("Correctives must have 2 or 3 FACS controls"));
    }
    acts.sort_unstable();
    acts.dedup();
    Ok(acts)
}

fn pose_in(m: &Mesh, bones: &Value, cframes: &Value) -> mlua::Result<Vec<(u64, Xform)>> {
    let bs = ids_of(bones);
    let cs: Vec<Value> = match cframes {
        Value::Table(t) => t.clone().sequence_values::<Value>().filter_map(Result::ok).collect(),
        _ => Vec::new(),
    };
    if bs.len() != cs.len() {
        return Err(rt_err("Number of bone ids and CFrames must match"));
    }
    let mut out = Vec::new();
    for (b, c) in bs.into_iter().zip(cs) {
        let b = m.check(b, K_BONE)?;
        m.check_virtual(b)?;
        out.push((b, cf(&c).ok_or_else(|| rt_err("CFrame expected"))?));
    }
    Ok(out)
}

fn pose_out(lua: &Lua, pose: &[(u64, Xform)]) -> mlua::Result<MultiValue> {
    let bs = lua.create_table()?;
    let cs = lua.create_table()?;
    for (i, (b, x)) in pose.iter().enumerate() {
        bs.raw_set(i + 1, *b as f64)?;
        cs.raw_set(i + 1, cfv(lua, *x)?)?;
    }
    Ok(MultiValue::from_iter([Value::Table(bs), Value::Table(cs)]))
}

