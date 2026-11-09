//! FBX model import for `InsertService:LoadAsset` on FBX uploads (binary FBX
//! 7.x, as Blender and most tools write it).
//!
//! What Roblox's importer makes of an FBX upload, and what this reproduces:
//! a Model holding one MeshPart per mesh node, named after the node, with
//! empty nodes becoming Models and child meshes nested under their parent.
//! Every MeshPart is re-centred on its own bounding box (its CFrame sits at
//! the box centre, Size = the box). Units: one metre in the file (after the
//! node transforms and the file's UnitScaleFactor, which counts centimetres)
//! is one stud; axes: the file's Y-up space turned 180° about Y. Those rules
//! were checked against Place A' gun packs, whose generator records where
//! the uploaded meshes ended up (Shared/GunMeshes: same sizes and centres,
//! "turned 180 degrees about Y"). Materials give the part its Color; an
//! embedded texture (Video content) becomes the MeshPart's TextureID.

use std::collections::HashMap;
use std::io::Read;

use glam::{DMat4, DVec3, Vec2, Vec3};

#[derive(Debug, Clone)]
#[allow(dead_code)] // every FBX property type is parsed; not all are read
pub enum Prop {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Vec<u8>),
    Raw(Vec<u8>),
    Floats(Vec<f64>),
    Ints(Vec<i64>),
}

impl Prop {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Prop::Int(i) => Some(*i as f64),
            Prop::Float(f) => Some(*f),
            _ => None,
        }
    }
    fn as_i64(&self) -> Option<i64> {
        match self {
            Prop::Int(i) => Some(*i),
            Prop::Float(f) => Some(*f as i64),
            _ => None,
        }
    }
    fn as_str(&self) -> Option<&[u8]> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub kids: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.kids.iter().find(|k| k.name == name)
    }
    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }
    /// Properties70 entries: name → values after the 4 header fields.
    fn props70(&self) -> HashMap<String, Vec<Prop>> {
        let mut out = HashMap::new();
        if let Some(p) = self.child("Properties70") {
            for e in p.children("P") {
                if let Some(Prop::Str(n)) = e.props.first() {
                    out.insert(
                        String::from_utf8_lossy(n).into_owned(),
                        e.props.iter().skip(4).cloned().collect(),
                    );
                }
            }
        }
        out
    }
}

const MAGIC: &[u8] = b"Kaydara FBX Binary  \0";

pub fn is_fbx(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC) || bytes.starts_with(b"; FBX")
}

struct Reader<'a> {
    b: &'a [u8],
    wide: bool,
}

impl Reader<'_> {
    fn u32(&self, o: usize) -> Result<u32, String> {
        self.b
            .get(o..o + 4)
            .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
            .ok_or_else(|| "FBX: truncated".to_string())
    }
    fn u64(&self, o: usize) -> Result<u64, String> {
        self.b
            .get(o..o + 8)
            .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
            .ok_or_else(|| "FBX: truncated".to_string())
    }
    fn slice(&self, o: usize, n: usize) -> Result<&[u8], String> {
        self.b
            .get(o..o.checked_add(n).ok_or("FBX: bad length")?)
            .ok_or_else(|| "FBX: truncated".to_string())
    }

    /// One node record at `o`; None for the null record that ends a list.
    fn node(&self, o: usize, depth: usize) -> Result<(Option<Node>, usize), String> {
        if depth > 64 {
            return Err("FBX: nodes nested too deeply".into());
        }
        let (end, nprops, mut o) = if self.wide {
            (self.u64(o)? as usize, self.u64(o + 8)? as usize, o + 24)
        } else {
            (self.u32(o)? as usize, self.u32(o + 4)? as usize, o + 12)
        };
        let name_len = *self.b.get(o).ok_or("FBX: truncated")? as usize;
        o += 1;
        if end == 0 {
            return Ok((None, o + name_len));
        }
        if end > self.b.len() || end < o {
            return Err("FBX: node runs past the end of the file".into());
        }
        let name = String::from_utf8_lossy(self.slice(o, name_len)?).into_owned();
        o += name_len;
        let mut props = Vec::with_capacity(nprops.min(64));
        for _ in 0..nprops {
            let t = *self.b.get(o).ok_or("FBX: truncated")?;
            o += 1;
            let (p, n) = self.prop(t, o)?;
            props.push(p);
            o += n;
        }
        let mut kids = Vec::new();
        while o < end {
            let (k, next) = self.node(o, depth + 1)?;
            o = next;
            match k {
                Some(k) => kids.push(k),
                None => break,
            }
        }
        Ok((Some(Node { name, props, kids }), end))
    }

    fn prop(&self, t: u8, o: usize) -> Result<(Prop, usize), String> {
        let f = |n: usize| self.slice(o, n);
        Ok(match t {
            b'Y' => (
                Prop::Int(i16::from_le_bytes(f(2)?.try_into().unwrap()) as i64),
                2,
            ),
            b'C' => (Prop::Bool(f(1)?[0] != 0), 1),
            b'I' => (
                Prop::Int(i32::from_le_bytes(f(4)?.try_into().unwrap()) as i64),
                4,
            ),
            b'F' => (
                Prop::Float(f32::from_le_bytes(f(4)?.try_into().unwrap()) as f64),
                4,
            ),
            b'D' => (
                Prop::Float(f64::from_le_bytes(f(8)?.try_into().unwrap())),
                8,
            ),
            b'L' => (Prop::Int(i64::from_le_bytes(f(8)?.try_into().unwrap())), 8),
            b'S' | b'R' => {
                let n = self.u32(o)? as usize;
                let s = self.slice(o + 4, n)?.to_vec();
                (
                    if t == b'S' {
                        Prop::Str(s)
                    } else {
                        Prop::Raw(s)
                    },
                    4 + n,
                )
            }
            b'f' | b'd' | b'l' | b'i' | b'b' => {
                let count = self.u32(o)? as usize;
                let enc = self.u32(o + 4)?;
                let clen = self.u32(o + 8)? as usize;
                let raw = self.slice(o + 12, clen)?;
                let elem = match t {
                    b'f' | b'i' => 4,
                    b'd' | b'l' => 8,
                    _ => 1,
                };
                let data = if enc == 1 {
                    let mut out = Vec::with_capacity(count.saturating_mul(elem).min(1 << 28));
                    flate2::read::ZlibDecoder::new(raw)
                        .read_to_end(&mut out)
                        .map_err(|e| format!("FBX: bad compressed array: {e}"))?;
                    out
                } else {
                    raw.to_vec()
                };
                if data.len() < count.saturating_mul(elem) {
                    return Err("FBX: array shorter than its count".into());
                }
                let p = match t {
                    b'f' => Prop::Floats(
                        data.chunks_exact(4)
                            .take(count)
                            .map(|c| f32::from_le_bytes(c.try_into().unwrap()) as f64)
                            .collect(),
                    ),
                    b'd' => Prop::Floats(
                        data.chunks_exact(8)
                            .take(count)
                            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                            .collect(),
                    ),
                    b'i' => Prop::Ints(
                        data.chunks_exact(4)
                            .take(count)
                            .map(|c| i32::from_le_bytes(c.try_into().unwrap()) as i64)
                            .collect(),
                    ),
                    b'l' => Prop::Ints(
                        data.chunks_exact(8)
                            .take(count)
                            .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
                            .collect(),
                    ),
                    _ => Prop::Ints(data.iter().take(count).map(|b| *b as i64).collect()),
                };
                (p, 12 + clen)
            }
            other => return Err(format!("FBX: unknown property type {:?}", other as char)),
        })
    }
}

/// Parses a binary FBX file into its top-level nodes.
pub fn parse(bytes: &[u8]) -> Result<(u32, Vec<Node>), String> {
    if bytes.starts_with(b"; FBX") {
        return Err("ASCII FBX files aren't supported (export as binary FBX)".into());
    }
    if !bytes.starts_with(MAGIC) || bytes.len() < 27 {
        return Err("not an FBX file".into());
    }
    let version = u32::from_le_bytes(bytes[23..27].try_into().unwrap());
    let r = Reader {
        b: bytes,
        wide: version >= 7500,
    };
    let mut o = 27;
    let mut top = Vec::new();
    while o + 13 < bytes.len() {
        let (n, next) = r.node(o, 0)?;
        match n {
            Some(n) => top.push(n),
            None => break,
        }
        o = next;
    }
    Ok((version, top))
}

/// Triangle geometry in Roblox space (studs), de-indexed, UVs top-left origin.
#[derive(Debug, Clone, Default)]
pub struct Geometry {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
}

#[derive(Debug, Clone)]
pub struct ImportNode {
    pub name: String,
    /// Index of the parent node in the list (None: the top Model).
    pub parent: Option<usize>,
    pub mesh: Option<Geometry>,
    pub color: Option<[f32; 3]>,
    /// Embedded texture bytes (PNG / JPEG / ...) and file name.
    pub texture: Option<(String, Vec<u8>)>,
}

fn vec3_prop(p: &HashMap<String, Vec<Prop>>, name: &str, default: DVec3) -> DVec3 {
    match p.get(name) {
        Some(v) if v.len() >= 3 => DVec3::new(
            v[0].as_f64().unwrap_or(default.x),
            v[1].as_f64().unwrap_or(default.y),
            v[2].as_f64().unwrap_or(default.z),
        ),
        _ => default,
    }
}

/// Euler angles (degrees) in FBX's default XYZ order: X applied first.
fn euler(deg: DVec3) -> DMat4 {
    let r = deg * (std::f64::consts::PI / 180.0);
    DMat4::from_rotation_z(r.z) * DMat4::from_rotation_y(r.y) * DMat4::from_rotation_x(r.x)
}

/// A node's local transform (translation, pre-rotation, rotation, scaling and
/// their pivots, as FBX defines them).
fn local_matrix(p: &HashMap<String, Vec<Prop>>) -> DMat4 {
    let t = vec3_prop(p, "Lcl Translation", DVec3::ZERO);
    let r = vec3_prop(p, "Lcl Rotation", DVec3::ZERO);
    let s = vec3_prop(p, "Lcl Scaling", DVec3::ONE);
    let pre = vec3_prop(p, "PreRotation", DVec3::ZERO);
    let post = vec3_prop(p, "PostRotation", DVec3::ZERO);
    let roff = vec3_prop(p, "RotationOffset", DVec3::ZERO);
    let rpiv = vec3_prop(p, "RotationPivot", DVec3::ZERO);
    let soff = vec3_prop(p, "ScalingOffset", DVec3::ZERO);
    let spiv = vec3_prop(p, "ScalingPivot", DVec3::ZERO);
    DMat4::from_translation(t)
        * DMat4::from_translation(roff)
        * DMat4::from_translation(rpiv)
        * euler(pre)
        * euler(r)
        * euler(post).inverse()
        * DMat4::from_translation(-rpiv)
        * DMat4::from_translation(soff)
        * DMat4::from_translation(spiv)
        * DMat4::from_scale(s)
        * DMat4::from_translation(-spiv)
}

fn floats(n: Option<&Node>) -> Vec<f64> {
    match n.and_then(|n| n.props.first()) {
        Some(Prop::Floats(v)) => v.clone(),
        _ => Vec::new(),
    }
}

fn ints(n: Option<&Node>) -> Vec<i64> {
    match n.and_then(|n| n.props.first()) {
        Some(Prop::Ints(v)) => v.clone(),
        _ => Vec::new(),
    }
}

fn text(n: Option<&Node>) -> String {
    n.and_then(|n| n.props.first())
        .and_then(|p| p.as_str())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .unwrap_or_default()
}

/// A per-corner value from a LayerElement (normals or UVs).
fn layer_value(
    mapping: &str,
    reference: &str,
    index: &[i64],
    corner: usize,
    vertex: usize,
    polygon: usize,
) -> Option<usize> {
    let i = match mapping {
        "ByPolygonVertex" => corner,
        "ByVertice" | "ByVertex" => vertex,
        "ByPolygon" => polygon,
        "AllSame" => 0,
        _ => corner,
    };
    if reference == "IndexToDirect" || reference == "Index" {
        index.get(i).and_then(|v| usize::try_from(*v).ok())
    } else {
        Some(i)
    }
}

/// Triangles of a Geometry node, transformed by `m` (normals by its rotation).
fn geometry(g: &Node, m: DMat4) -> Geometry {
    let verts = floats(g.child("Vertices"));
    let poly = ints(g.child("PolygonVertexIndex"));
    let nl = g.child("LayerElementNormal");
    let (n_map, n_ref) = (
        text(nl.and_then(|l| l.child("MappingInformationType"))),
        text(nl.and_then(|l| l.child("ReferenceInformationType"))),
    );
    let normals = floats(nl.and_then(|l| l.child("Normals")));
    let n_index = ints(nl.and_then(|l| l.child("NormalsIndex")));
    let ul = g.child("LayerElementUV");
    let (u_map, u_ref) = (
        text(ul.and_then(|l| l.child("MappingInformationType"))),
        text(ul.and_then(|l| l.child("ReferenceInformationType"))),
    );
    let uvs = floats(ul.and_then(|l| l.child("UV")));
    let u_index = ints(ul.and_then(|l| l.child("UVIndex")));
    let nm = m.inverse().transpose();
    let pos = |v: usize| -> Option<Vec3> {
        let p = DVec3::new(
            *verts.get(v * 3)?,
            *verts.get(v * 3 + 1)?,
            *verts.get(v * 3 + 2)?,
        );
        Some(m.transform_point3(p).as_vec3())
    };
    let mut out = Geometry::default();
    let mut face: Vec<(usize, usize)> = Vec::new(); // (vertex, corner)
    let mut polygon = 0;
    for (corner, raw) in poly.iter().enumerate() {
        let last = *raw < 0;
        let v = if last {
            (-raw - 1) as usize
        } else {
            *raw as usize
        };
        face.push((v, corner));
        if !last {
            continue;
        }
        for k in 1..face.len().saturating_sub(1) {
            let tri = [face[0], face[k], face[k + 1]];
            let ps: Option<Vec<Vec3>> = tri.iter().map(|(v, _)| pos(*v)).collect();
            let Some(ps) = ps else { continue };
            let flat = (ps[1] - ps[0]).cross(ps[2] - ps[0]).normalize_or_zero();
            for (j, (v, c)) in tri.iter().enumerate() {
                out.positions.push(ps[j]);
                let n = layer_value(&n_map, &n_ref, &n_index, *c, *v, polygon)
                    .and_then(|i| {
                        Some(DVec3::new(
                            *normals.get(i * 3)?,
                            *normals.get(i * 3 + 1)?,
                            *normals.get(i * 3 + 2)?,
                        ))
                    })
                    .map(|n| nm.transform_vector3(n).as_vec3().normalize_or_zero())
                    .unwrap_or(flat);
                out.normals.push(n);
                let uv = layer_value(&u_map, &u_ref, &u_index, *c, *v, polygon)
                    .and_then(|i| {
                        Some(Vec2::new(
                            *uvs.get(i * 2)? as f32,
                            1.0 - *uvs.get(i * 2 + 1)? as f32,
                        ))
                    })
                    .unwrap_or(Vec2::ZERO);
                out.uvs.push(uv);
            }
        }
        face.clear();
        polygon += 1;
    }
    out
}

/// The nodes Roblox's importer would make from an FBX file.
pub fn import(bytes: &[u8]) -> Result<Vec<ImportNode>, String> {
    let (_, top) = parse(bytes)?;
    let find = |name: &str| top.iter().find(|n| n.name == name);
    let objects = find("Objects").ok_or("FBX: no Objects section")?;
    // Unit scale: the file counts centimetres × UnitScaleFactor; a metre is a stud.
    let unit = find("GlobalSettings")
        .map(|g| g.props70())
        .and_then(|p| p.get("UnitScaleFactor").and_then(|v| v.first()?.as_f64()))
        .unwrap_or(1.0);
    let to_roblox = DMat4::from_rotation_y(std::f64::consts::PI)
        * DMat4::from_scale(DVec3::splat(unit / 100.0));

    let mut by_id: HashMap<i64, &Node> = HashMap::new();
    for o in &objects.kids {
        if let Some(id) = o.props.first().and_then(|p| p.as_i64()) {
            by_id.insert(id, o);
        }
    }
    // Object-object links: child → parent, and parent → children (in file order).
    let mut parent: HashMap<i64, i64> = HashMap::new();
    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    if let Some(c) = find("Connections") {
        for l in c.children("C") {
            let (Some(a), Some(b)) = (
                l.props.get(1).and_then(|p| p.as_i64()),
                l.props.get(2).and_then(|p| p.as_i64()),
            ) else {
                continue;
            };
            parent.entry(a).or_insert(b);
            children.entry(b).or_default().push(a);
        }
    }
    let is = |id: i64, class: &str| by_id.get(&id).is_some_and(|n| n.name == class);
    let obj_name = |n: &Node| -> String {
        let raw = n.props.get(1).and_then(|p| p.as_str()).unwrap_or_default();
        let s = raw.split(|b| *b == 0).next().unwrap_or_default();
        String::from_utf8_lossy(s).into_owned()
    };

    let mut out = Vec::new();
    // Depth-first from the root (id 0) through Model nodes.
    let mut stack: Vec<(i64, Option<usize>, DMat4)> = children
        .get(&0)
        .map(|v| {
            v.iter()
                .rev()
                .filter(|c| is(**c, "Model"))
                .map(|c| (*c, None, DMat4::IDENTITY))
                .collect()
        })
        .unwrap_or_default();
    while let Some((id, up, parent_m)) = stack.pop() {
        let node = by_id[&id];
        let props = node.props70();
        let world = parent_m * local_matrix(&props);
        let kids = children.get(&id).cloned().unwrap_or_default();
        let geo = kids
            .iter()
            .find(|k| is(**k, "Geometry"))
            .map(|g| geometry(by_id[g], to_roblox * world));
        let mut color = None;
        let mut texture = None;
        if let Some(mat) = kids.iter().find(|k| is(**k, "Material")) {
            let mp = by_id[mat].props70();
            if let Some(c) = mp.get("DiffuseColor").filter(|c| c.len() >= 3) {
                color = Some([
                    c[0].as_f64().unwrap_or(1.0) as f32,
                    c[1].as_f64().unwrap_or(1.0) as f32,
                    c[2].as_f64().unwrap_or(1.0) as f32,
                ]);
            }
            // Material ← Texture ← Video (embedded Content).
            for t in children
                .get(mat)
                .into_iter()
                .flatten()
                .filter(|t| is(**t, "Texture"))
            {
                for v in children
                    .get(t)
                    .into_iter()
                    .flatten()
                    .filter(|v| is(**v, "Video"))
                {
                    let vn = by_id[v];
                    if let Some(Prop::Raw(bytes)) =
                        vn.child("Content").and_then(|c| c.props.first())
                    {
                        if !bytes.is_empty() {
                            texture = Some((text(vn.child("RelativeFilename")), bytes.clone()));
                        }
                    }
                }
            }
        }
        let kind_mesh = geo.as_ref().is_some_and(|g| !g.positions.is_empty());
        out.push(ImportNode {
            name: obj_name(node),
            parent: up,
            mesh: if kind_mesh { geo } else { None },
            color,
            texture,
        });
        let me = out.len() - 1;
        for k in kids.iter().rev().filter(|k| is(**k, "Model")) {
            stack.push((*k, Some(me), world));
        }
    }
    if out.is_empty() {
        return Err("FBX: no models in the file".into());
    }
    Ok(out)
}

/// A Roblox "version 2.00" mesh holding de-indexed triangles.
pub fn mesh_v2(g: &Geometry) -> Vec<u8> {
    let n = g.positions.len();
    let mut b = Vec::with_capacity(32 + n * 36 + (n / 3) * 12);
    b.extend_from_slice(b"version 2.00\n");
    b.extend_from_slice(&12u16.to_le_bytes());
    b.push(36);
    b.push(12);
    b.extend_from_slice(&(n as u32).to_le_bytes());
    b.extend_from_slice(&((n / 3) as u32).to_le_bytes());
    for i in 0..n {
        let p = g.positions[i];
        let nn = g.normals.get(i).copied().unwrap_or(Vec3::Y);
        let uv = g.uvs.get(i).copied().unwrap_or(Vec2::ZERO);
        for f in [p.x, p.y, p.z, nn.x, nn.y, nn.z, uv.x, uv.y, 0.0] {
            b.extend_from_slice(&f.to_le_bytes());
        }
    }
    for t in 0..n / 3 {
        for k in 0..3 {
            b.extend_from_slice(&((t * 3 + k) as u32).to_le_bytes());
        }
    }
    b
}

/// A small binary FBX (7.4) for the self-test: a group "Pack" holding one box
/// mesh "Crate" of `size_cm` centimetres, moved `at_cm` from the origin.
pub fn sample(size_cm: [f64; 3], at_cm: [f64; 3]) -> Vec<u8> {
    struct N {
        name: &'static str,
        props: Vec<Prop>,
        kids: Vec<N>,
    }
    fn n(name: &'static str, props: Vec<Prop>, kids: Vec<N>) -> N {
        N { name, props, kids }
    }
    fn write(node: &N, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&[0; 12]);
        out.push(node.name.len() as u8);
        out.extend_from_slice(node.name.as_bytes());
        let props_at = out.len();
        for p in &node.props {
            match p {
                Prop::Int(i) => {
                    out.push(b'L');
                    out.extend_from_slice(&i.to_le_bytes());
                }
                Prop::Float(f) => {
                    out.push(b'D');
                    out.extend_from_slice(&f.to_le_bytes());
                }
                Prop::Str(s) => {
                    out.push(b'S');
                    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
                    out.extend_from_slice(s);
                }
                Prop::Floats(v) => {
                    out.push(b'd');
                    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&((v.len() * 8) as u32).to_le_bytes());
                    for f in v {
                        out.extend_from_slice(&f.to_le_bytes());
                    }
                }
                Prop::Ints(v) => {
                    out.push(b'i');
                    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&((v.len() * 4) as u32).to_le_bytes());
                    for i in v {
                        out.extend_from_slice(&(*i as i32).to_le_bytes());
                    }
                }
                _ => {}
            }
        }
        let prop_len = (out.len() - props_at) as u32;
        for k in &node.kids {
            write(k, out);
        }
        if !node.kids.is_empty() {
            out.extend_from_slice(&[0; 13]);
        }
        let end = out.len() as u32;
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&(node.props.len() as u32).to_le_bytes());
        out[start + 8..start + 12].copy_from_slice(&prop_len.to_le_bytes());
    }
    let s = |t: &str| Prop::Str(t.as_bytes().to_vec());
    let class_name = |name: &str, class: &str| {
        Prop::Str([name.as_bytes(), b"\x00\x01", class.as_bytes()].concat())
    };
    let p70 = |name: &str, v: [f64; 3]| {
        n(
            "P",
            vec![
                s(name),
                s("Lcl Translation"),
                s(""),
                s("A"),
                Prop::Float(v[0]),
                Prop::Float(v[1]),
                Prop::Float(v[2]),
            ],
            vec![],
        )
    };
    let h = size_cm.map(|v| v * 0.5);
    let mut verts = Vec::new();
    for z in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for x in [-1.0, 1.0] {
                verts.extend_from_slice(&[x * h[0], y * h[1], z * h[2]]);
            }
        }
    }
    // Six quads (the last index of each polygon is stored as -(i+1)).
    let quads: [[i64; 4]; 6] = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let poly: Vec<i64> = quads
        .iter()
        .flat_map(|q| [q[0], q[1], q[2], -(q[3] + 1)])
        .collect();
    let top = vec![
        n(
            "GlobalSettings",
            vec![],
            vec![n(
                "Properties70",
                vec![],
                vec![n(
                    "P",
                    vec![
                        s("UnitScaleFactor"),
                        s("double"),
                        s("Number"),
                        s(""),
                        Prop::Float(1.0),
                    ],
                    vec![],
                )],
            )],
        ),
        n(
            "Objects",
            vec![],
            vec![
                n(
                    "Geometry",
                    vec![Prop::Int(10), class_name("Crate", "Geometry"), s("Mesh")],
                    vec![
                        n("Vertices", vec![Prop::Floats(verts)], vec![]),
                        n("PolygonVertexIndex", vec![Prop::Ints(poly)], vec![]),
                    ],
                ),
                n(
                    "Model",
                    vec![Prop::Int(20), class_name("Crate", "Model"), s("Mesh")],
                    vec![n(
                        "Properties70",
                        vec![],
                        vec![p70("Lcl Translation", at_cm)],
                    )],
                ),
                n(
                    "Model",
                    vec![Prop::Int(30), class_name("Pack", "Model"), s("Null")],
                    vec![],
                ),
            ],
        ),
        n(
            "Connections",
            vec![],
            vec![
                n("C", vec![s("OO"), Prop::Int(10), Prop::Int(20)], vec![]),
                n("C", vec![s("OO"), Prop::Int(20), Prop::Int(30)], vec![]),
                n("C", vec![s("OO"), Prop::Int(30), Prop::Int(0)], vec![]),
            ],
        ),
    ];
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[0x1a, 0x00]);
    out.extend_from_slice(&7400u32.to_le_bytes());
    for t in &top {
        write(t, &mut out);
    }
    out.extend_from_slice(&[0; 13]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_round_trip() {
        let nodes = import(&sample([200.0, 100.0, 300.0], [500.0, 0.0, 0.0])).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].name, "Pack");
        assert!(nodes[0].mesh.is_none());
        assert_eq!(nodes[1].parent, Some(0));
        let g = nodes[1].mesh.as_ref().unwrap();
        assert_eq!(g.positions.len(), 36);
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &g.positions {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        assert!(((hi - lo) - Vec3::new(2.0, 1.0, 3.0)).abs().max_element() < 1e-4);
        assert!(
            (((hi + lo) * 0.5) - Vec3::new(-5.0, 0.0, 0.0))
                .abs()
                .max_element()
                < 1e-4
        );
    }

    /// Every Place A gun pack on this machine: each imported piece matches
    /// the size and (180°-turned) centre Shared/GunMeshes recorded for it.
    /// (Mag pieces, placed from a separate mag model, are left out: dingo,
    /// kuda, vesper and vmp's disagree about X rotation and no Studio import
    /// says which side is right.)
    #[test]
    fn place_a_all_gun_packs() {
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let root = home.join("games/place-a");
        let Ok(spec) = std::fs::read_to_string(root.join("src/Shared/GunMeshes.luau")) else {
            return;
        };
        let v3 = |s: &str| -> Vec3 {
            let n: Vec<f32> = s.split(',').map(|x| x.trim().parse().unwrap()).collect();
            Vec3::new(n[0], n[1], n[2])
        };
        let mut bad = Vec::new();
        let mut checked = 0;
        let mut model = String::new();
        let mut nodes: Option<Vec<ImportNode>> = None;
        for line in spec.lines() {
            let t = line.trim();
            if let Some(name) = t.strip_suffix("{").and_then(|x| x.split_once(" = { pack")).map(|x| x.0) {
                model = name.trim().to_string();
                let p = root.join(format!("tools/guns/out/gun_mesh/fbx/pack_{model}.fbx"));
                nodes = std::fs::read(&p).ok().map(|b| import(&b).unwrap());
                continue;
            }
            let (Some(nodes), true) = (&nodes, t.starts_with("{ \"")) else {
                continue;
            };
            let piece = t.split('"').nth(1).unwrap();
            if t.split('"').nth(3) == Some("mag") {
                continue;
            }
            let vecs: Vec<&str> = t.split("Vector3.new(").skip(1).map(|s| s.split(')').next().unwrap()).collect();
            let (centre, size) = (v3(vecs[0]), v3(vecs[1]));
            let Some(n) = nodes.iter().find(|n| n.name == piece) else {
                bad.push(format!("{model}/{piece}: missing"));
                continue;
            };
            let g = n.mesh.as_ref().unwrap();
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for p in &g.positions {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            let want_c = Vec3::new(-centre.x, centre.y, -centre.z);
            let (gs, gc) = (hi - lo, (hi + lo) * 0.5);
            if (gs - size).abs().max_element() > 2e-3 || (gc - want_c).abs().max_element() > 2e-3 {
                bad.push(format!("{model}/{piece}: size {gs} want {size}, centre {gc} want {want_c}"));
            }
            checked += 1;
        }
        assert!(bad.is_empty(), "{checked} checked; {}", bad.join("\n"));
    }

    /// Place A' sample gun pack, when it's on this machine: the imported
    /// pieces match the centres and sizes its generator recorded.
    #[test]
    fn place_a_argus_pack() {
        let path = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join("games/place-a/tools/guns/out/gun_mesh/fbx/pack_argus.fbx");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let nodes = import(&bytes).unwrap();
        assert_eq!(nodes.len(), 18);
        let first = nodes.iter().find(|n| n.name == "argus__01").unwrap();
        let g = first.mesh.as_ref().unwrap();
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &g.positions {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let size = hi - lo;
        let centre = (hi + lo) * 0.5;
        // Shared/GunMeshes: argus__01 centre (-0.00122, 0.29795, -0.54663), size (0.13553, 0.42168, 1.25919),
        // in gun space; the upload is that turned 180° about Y.
        assert!(
            (size - Vec3::new(0.13553, 0.42168, 1.25919))
                .abs()
                .max_element()
                < 1e-3,
            "{size}"
        );
        assert!(
            (centre - Vec3::new(0.00122, 0.29795, 0.54663))
                .abs()
                .max_element()
                < 1e-3,
            "{centre}"
        );
    }
}
