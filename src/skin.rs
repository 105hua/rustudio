//! Skinned MeshParts: the mesh's bones and weights (mesh.rs `Skin`) driven
//! by the Bone instances in the part, as Roblox skins them. Each mesh bone
//! is matched by name with a Bone under the MeshPart; its current frame in
//! part space is the chain of Bone.CFrame × Bone.Transform from the part
//! (a mesh bone with no Bone instance follows its parent bone rigidly), its
//! bind frame is the mesh's own bone frame (mesh space, centred and scaled
//! to the part like the vertices). Every corner moves by the weighted sum of
//! (current × bind⁻¹) over its bones; normals turn with the rotations.
//! Done on the CPU when a part's bones move (a dynamic head is a few
//! thousand triangles), and only for parts that have Bones: a skinned mesh
//! whose bones are all at rest draws the shared mesh untouched.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use glam::{Mat3, Vec2, Vec3, Vec4};
use rbx_dom_weak::types::{Ref, Variant};

use crate::doc::Document;
use crate::math::Xform;

/// A skinned mesh in the renderer's normalized space (unit box), with what
/// is needed to deform it.
pub struct Base {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub colors: Vec<Vec4>,
    pub extent: Vec3,
    pub center: Vec3,
    pub skin: crate::mesh::Skin,
    /// Unique vertices: the first corner of each, and each corner's vertex.
    pub firsts: Vec<u32>,
    pub corner_unique: Vec<u32>,
    /// The mesh's FACS rig (dynamic heads), posed by a FaceControls child.
    pub facs: Option<crate::runtime::domains::avatar::face::Facs>,
}

/// Parsed skinned meshes by content id (None: not skinned / unreadable).
static BASES: std::sync::LazyLock<Mutex<HashMap<String, Option<Arc<Base>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// The skinned base of a mesh, parsed once from its bytes.
pub fn base(id: &str, bytes: &[u8]) -> Option<Arc<Base>> {
    if !crate::assets::place_content_ok(id) {
        return None;
    }
    if let Some(b) = BASES.lock().ok()?.get(id) {
        return b.clone();
    }
    let b = crate::mesh::parse(bytes).ok().and_then(|m| {
        let skin = m.skin.clone()?;
        if skin.corner_bones.len() != m.positions.len() || skin.bones.is_empty() {
            return None;
        }
        let extent = (m.max - m.min).max(Vec3::splat(1e-4));
        let center = (m.max + m.min) * 0.5;
        let (firsts, corner_unique) = uniques(&skin.corner_vertex, m.positions.len());
        Some(Arc::new(Base {
            firsts,
            corner_unique,
            positions: m.positions.iter().map(|p| (*p - center) / extent).collect(),
            normals: m.normals.iter().map(|n| *n * extent).collect(),
            uvs: m.uvs,
            colors: m.colors,
            extent,
            center,
            skin,
            facs: crate::runtime::domains::avatar::face::parse_mesh_facs(bytes),
        }))
    });
    if let Ok(mut g) = BASES.lock() {
        g.insert(id.to_string(), b.clone());
    }
    b
}

/// Groups corners by mesh vertex: (first corner of each vertex, corner →
/// vertex slot). Without vertex ids every corner is its own vertex.
fn uniques(corner_vertex: &[u32], corners: usize) -> (Vec<u32>, Vec<u32>) {
    if corner_vertex.len() != corners {
        return ((0..corners as u32).collect(), (0..corners as u32).collect());
    }
    let mut slot: HashMap<u32, u32> = HashMap::new();
    let mut firsts = Vec::new();
    let mut map = Vec::with_capacity(corners);
    for (c, v) in corner_vertex.iter().enumerate() {
        let s = *slot.entry(*v).or_insert_with(|| {
            firsts.push(c as u32);
            firsts.len() as u32 - 1
        });
        map.push(s);
    }
    (firsts, map)
}

/// Whether a part has Bones under it or a FaceControls child (only those
/// parts are skinned).
pub fn has_bones(doc: &Document, part: Ref) -> bool {
    doc.dom.descendants_of(part).any(|i| i.class == "Bone" || i.class == "FaceControls")
}

/// What a skinned part redraws on: its Bones and FaceControls.
pub fn deps_of(doc: &Document, part: Ref) -> Vec<Ref> {
    doc.dom
        .descendants_of(part)
        .filter(|i| i.class == "Bone" || i.class == "FaceControls")
        .map(|i| i.referent())
        .collect()
}

/// FACS offsets (bone name → local frame) from the part's FaceControls.
fn facs_offsets(doc: &Document, part: Ref, base: &Base, scale: Vec3) -> HashMap<String, Xform> {
    let mut out = HashMap::new();
    let Some(f) = &base.facs else { return out };
    let Some(fc) = doc
        .dom
        .get_by_ref(part)
        .and_then(|i| i.children().iter().copied().find(|c| doc.class_of(*c).is_some_and(|k| k == "FaceControls")))
    else {
        return out;
    };
    let value = |c: &str| match doc.prop(fc, c) {
        Some(Variant::Float32(v)) => v,
        Some(Variant::Float64(v)) => v as f32,
        _ => 0.0,
    };
    for (name, p, r) in crate::runtime::domains::avatar::face::pose(f, &value) {
        if p == [0.0; 3] && r == [0.0; 3] {
            continue;
        }
        let rot = Mat3::from_euler(glam::EulerRot::XYZ, r[0].to_radians(), r[1].to_radians(), r[2].to_radians());
        out.insert(name, Xform { pos: Vec3::from(p) * scale, rot });
    }
    out
}

/// The Bones under a part (descendants), by name: first one wins.
pub fn bones_of(doc: &Document, part: Ref) -> Vec<Ref> {
    doc.dom
        .descendants_of(part)
        .filter(|i| i.class == "Bone")
        .map(|i| i.referent())
        .collect()
}

fn cf(doc: &Document, r: Ref, k: &str) -> Xform {
    match doc.prop(r, k) {
        Some(Variant::CFrame(c)) => Xform::from_cframe(&c),
        _ => Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY },
    }
}

fn mul(a: &Xform, b: &Xform) -> Xform {
    Xform { pos: a.pos + a.rot * b.pos, rot: a.rot * b.rot }
}

fn inv(a: &Xform) -> Xform {
    let rt = a.rot.transpose();
    Xform { pos: -(rt * a.pos), rot: rt }
}

/// A Bone's current frame in its part's space: Bone.CFrame × Transform up
/// the chain of parent Bones.
fn bone_now(doc: &Document, part: Ref, b: Ref) -> Xform {
    let mut chain = Vec::new();
    let mut at = b;
    while at.is_some() && at != part {
        let Some(i) = doc.dom.get_by_ref(at) else { break };
        if i.class == "Bone" {
            chain.push(mul(&cf(doc, at, "CFrame"), &cf(doc, at, "Transform")));
        } else if i.class == "Attachment" {
            chain.push(cf(doc, at, "CFrame"));
        }
        at = i.parent();
    }
    chain
        .iter()
        .rev()
        .fold(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }, |acc, x| mul(&acc, x))
}

/// The skinning matrices (current × bind⁻¹, in part space), or None when
/// every bone is at its bind pose.
pub fn matrices(doc: &Document, part: Ref, base: &Base, scale: Vec3) -> Option<Vec<Xform>> {
    let bones = bones_of(doc, part);
    let mut by_name: HashMap<&str, Ref> = HashMap::new();
    for b in &bones {
        if let Some(i) = doc.dom.get_by_ref(*b) {
            by_name.entry(i.name.as_str()).or_insert(*b);
        }
    }
    let n = base.skin.bones.len();
    let bind: Vec<Xform> = base
        .skin
        .bones
        .iter()
        .map(|b| Xform { pos: (b.pos - base.center) * scale, rot: b.rot })
        .collect();
    let facs = facs_offsets(doc, part, base, scale);
    let mut now: Vec<Option<Xform>> = vec![None; n];
    // Parents come before children in Roblox meshes; resolve in passes.
    for _ in 0..n.max(1) {
        let mut progress = false;
        for i in 0..n {
            if now[i].is_some() {
                continue;
            }
            let sb = &base.skin.bones[i];
            if let Some(&b) = by_name.get(sb.name.as_str()) {
                now[i] = Some(bone_now(doc, part, b));
                progress = true;
            } else {
                // No Bone instance: the bind frame relative to its parent,
                // times the FaceControls' FACS offset for this bone.
                let local = facs.get(&sb.name).copied();
                match sb.parent {
                    None => {
                        now[i] = Some(match local {
                            Some(l) => mul(&bind[i], &l),
                            None => bind[i],
                        });
                        progress = true;
                    }
                    Some(p) => {
                        if let Some(pn) = now[p] {
                            let rest = mul(&pn, &mul(&inv(&bind[p]), &bind[i]));
                            now[i] = Some(match local {
                                Some(l) => mul(&rest, &l),
                                None => rest,
                            });
                            progress = true;
                        }
                    }
                }
            }
        }
        if !progress {
            break;
        }
    }
    let mats: Vec<Xform> = (0..n)
        .map(|i| mul(&now[i].unwrap_or(bind[i]), &inv(&bind[i])))
        .collect();
    let rest = mats
        .iter()
        .all(|m| m.pos.length_squared() < 1e-10 && m.rot.abs_diff_eq(Mat3::IDENTITY, 1e-5));
    (!rest).then_some(mats)
}

/// Deformed positions and normals (normalized space) for the matrices:
/// each mesh vertex once, copied to its corners.
pub fn deform(base: &Base, mats: &[Xform], scale: Vec3) -> (Vec<Vec3>, Vec<Vec3>) {
    let to_part = base.extent * scale;
    let from_part = Vec3::ONE / to_part.max(Vec3::splat(1e-6));
    let one = |i: usize| -> (Vec3, Vec3) {
        let pp = base.positions[i] * to_part;
        let n = base.normals[i] / base.extent;
        let ids = base.skin.corner_bones[i];
        let ws = base.skin.corner_weights[i];
        let mut q = Vec3::ZERO;
        let mut m = Vec3::ZERO;
        let mut total = 0.0;
        for k in 0..4 {
            let w = ws[k];
            if w <= 0.0 {
                continue;
            }
            if let Some(x) = mats.get(ids[k] as usize) {
                q += (x.pos + x.rot * pp) * w;
                m += (x.rot * n) * w;
                total += w;
            }
        }
        if total <= 0.0 {
            q = pp;
            m = n;
        }
        (q * from_part, m.normalize_or_zero() * base.extent)
    };
    let verts: Vec<(Vec3, Vec3)> = base.firsts.iter().map(|&c| one(c as usize)).collect();
    let mut pos = Vec::with_capacity(base.positions.len());
    let mut nrm = Vec::with_capacity(base.positions.len());
    for (c, u) in base.corner_unique.iter().enumerate() {
        match verts.get(*u as usize) {
            // A corner whose own normal differs (flat-shaded fallback) keeps
            // its own normal, turned like its vertex.
            Some((p, n)) => {
                pos.push(*p);
                nrm.push(*n);
            }
            None => {
                let (p, n) = one(c);
                pos.push(p);
                nrm.push(n);
            }
        }
    }
    (pos, nrm)
}

/// The part's Size / InitialSize (mesh units → studs), else Size / extent.
pub fn scale_of(doc: &Document, part: Ref, base: &Base) -> Vec3 {
    let v = |k: &str| match doc.prop(part, k) {
        Some(Variant::Vector3(v)) => Some(Vec3::new(v.x, v.y, v.z)),
        _ => None,
    };
    let size = v("Size").unwrap_or(base.extent);
    match v("InitialSize").or_else(|| v("MeshSize")) {
        Some(i) if i.min_element() > 1e-4 => size / i,
        _ => size / base.extent,
    }
}

/// Per-part mesh slots holding deformed copies.
static SLOTS: std::sync::LazyLock<Mutex<HashMap<Ref, (u32, usize)>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// The renderer mesh to draw a skinned part with this frame: a deformed
/// copy in the part's own slot (uploaded with `upload(slot, …)`, `slot` None
/// for a new one), or None to draw the shared mesh.
pub fn skinned_mesh(
    doc: &Document,
    part: Ref,
    base: &Base,
    upload: &mut dyn FnMut(Option<u32>, &[Vec3], &[Vec3], &[Vec2], &[Vec4]) -> u32,
) -> Option<u32> {
    let scale = scale_of(doc, part, base);
    let sc = crate::bench::scope("skin_matrices");
    let mats = matrices(doc, part, base, scale)?;
    drop(sc);
    let sc = crate::bench::scope("skin_deform");
    let (p, n) = deform(base, &mats, scale);
    drop(sc);
    let mut slots = SLOTS.lock().ok()?;
    let have = slots.get(&part).copied().filter(|(_, len)| *len == p.len()).map(|(i, _)| i);
    let _sc = crate::bench::scope("skin_upload");
    let idx = upload(have, &p, &n, &base.uvs, &base.colors);
    slots.insert(part, (idx, p.len()));
    Some(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Skin, SkinBone};

    fn base() -> Base {
        // Two corners: one fully on bone 0, one fully on bone 1 (child).
        Base {
            positions: vec![Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, -0.5, 0.0)],
            normals: vec![Vec3::Z, Vec3::Z],
            uvs: vec![Vec2::ZERO; 2],
            colors: Vec::new(),
            extent: Vec3::ONE,
            center: Vec3::ZERO,
            facs: None,
            firsts: vec![0, 1],
            corner_unique: vec![0, 1],
            skin: Skin {
                bones: vec![
                    SkinBone { name: "Root".into(), parent: None, rot: Mat3::IDENTITY, pos: Vec3::ZERO },
                    SkinBone { name: "Jaw".into(), parent: Some(0), rot: Mat3::IDENTITY, pos: Vec3::new(0.0, -0.5, 0.0) },
                ],
                corner_bones: vec![[0, 0, 0, 0], [1, 0, 0, 0]],
                corner_weights: vec![[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
                corner_vertex: vec![0, 1],
            },
        }
    }

    #[test]
    fn a_turned_child_bone_moves_only_its_corners() {
        let b = base();
        // Jaw rotated 90° about X around its own origin at (0,-0.5,0): the
        // corner sitting on the jaw origin stays, so move it off first.
        let mut b2 = base();
        b2.positions[1] = Vec3::new(0.0, -0.5, 0.25);
        let jaw_now = Xform { pos: Vec3::new(0.0, -0.5, 0.0), rot: Mat3::from_rotation_x(std::f32::consts::FRAC_PI_2) };
        let bind_jaw = Xform { pos: Vec3::new(0.0, -0.5, 0.0), rot: Mat3::IDENTITY };
        let mats = vec![Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }, mul(&jaw_now, &inv(&bind_jaw))];
        let (p, _) = deform(&b2, &mats, Vec3::ONE);
        assert!(p[0].abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 1e-5));
        // (0,0,0.25) relative to the jaw turned 90° about X → (0,-0.25,0).
        assert!(p[1].abs_diff_eq(Vec3::new(0.0, -0.75, 0.0), 1e-5), "{:?}", p[1]);
        let _ = b;
    }

    #[test]
    fn bone_instances_drive_the_mesh() {
        use rbx_dom_weak::InstanceBuilder;
        let mut doc = crate::doc::Document::new_place();
        let ws = doc.workspace().unwrap();
        let part = doc.dom.insert(ws, InstanceBuilder::new("MeshPart").with_name("Head")
            .with_property("Size", rbx_dom_weak::types::Vector3::new(1.0, 1.0, 1.0))
            .with_property("InitialSize", rbx_dom_weak::types::Vector3::new(1.0, 1.0, 1.0)));
        let bind = Xform { pos: Vec3::new(0.0, -0.5, 0.0), rot: Mat3::IDENTITY };
        let turn = Xform { pos: Vec3::ZERO, rot: Mat3::from_rotation_x(std::f32::consts::FRAC_PI_2) };
        doc.dom.insert(part, InstanceBuilder::new("Bone").with_name("Jaw")
            .with_property("CFrame", bind.to_cframe())
            .with_property("Transform", turn.to_cframe()));
        let mut b = base();
        b.positions[1] = Vec3::new(0.0, -0.5, 0.25);
        assert!(has_bones(&doc, part));
        let mats = matrices(&doc, part, &b, Vec3::ONE).expect("a moved bone");
        let (p, _) = deform(&b, &mats, Vec3::ONE);
        assert!(p[0].abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 1e-5));
        assert!(p[1].abs_diff_eq(Vec3::new(0.0, -0.75, 0.0), 1e-5), "{:?}", p[1]);
        // Back at rest: the shared mesh draws.
        let jaw = doc.dom.get_by_ref(part).unwrap().children()[0];
        doc.dom.get_by_ref_mut(jaw).unwrap().properties.insert(rbx_dom_weak::ustr("Transform"), Variant::CFrame(Xform { pos: Vec3::ZERO, rot: Mat3::IDENTITY }.to_cframe()));
        assert!(matrices(&doc, part, &b, Vec3::ONE).is_none());
    }
}

#[cfg(test)]
mod cache_probe {
    /// `SKIN_PROBE=path cargo test skin::cache_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_mesh_file() {
        let Ok(p) = std::env::var("SKIN_PROBE") else { return };
        let b = std::fs::read(p).unwrap();
        let m = crate::mesh::parse(&b).unwrap();
        println!("corners {} skin {:?}", m.positions.len(), m.skin.as_ref().map(|s| (s.bones.len(), s.corner_bones.len())));
        if let Some(s) = &m.skin {
            for bn in s.bones.iter().take(8) {
                println!("bone {} parent {:?} pos {:?}", bn.name, bn.parent, bn.pos);
            }
        }
        let f = crate::runtime::domains::avatar::face::parse_mesh_facs(&b);
        println!("facs {:?}", f.as_ref().map(|f| (f.bones.len(), f.controls.len(), f.cols, f.two.len(), f.three.len())));
        if let Some(f) = f {
            println!("controls {:?}", &f.controls);
            println!("bones {:?}", &f.bones[..f.bones.len().min(12)]);
        }
    }
}
