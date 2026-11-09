//! FaceControls: the 50 FACS properties really drive the face. Values are
//! clamped to [0, 1] and kept on the instance (Clone copies them, Changed
//! fires), as in Studio. When the FaceControls sits in a head MeshPart whose
//! mesh carries FACS data (mesh v4/v5 trailing block, or the v6/v7 FACS
//! chunk), every frame with a change poses the head's Bones the way Roblox
//! does: each bone's offset is the sum over controls of control value ×
//! that control's pose column, plus the two- and three-control correctives
//! weighted by the product of their controls; the result is written to
//! Bone.Transform (position in studs at the part's scale, rotation from the
//! XYZ Euler degrees in the data). Meshes are fetched once per id, off the
//! script thread. Mesh skinning in the renderer is outside this domain: the
//! Bones move (scripts and Bone.TransformedCFrame see them); the drawn
//! head deforms once the renderer skins meshes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use mlua::{Lua, Value};
use rbx_dom_weak::types::{Ref, Variant};

use crate::runtime::*;

pub const CONTROLS: &[&str] = &[
    "ChinRaiser", "ChinRaiserUpperLip", "Corrugator", "EyesLookDown", "EyesLookLeft", "EyesLookRight",
    "EyesLookUp", "FlatPucker", "Funneler", "JawDrop", "JawLeft", "JawRight", "LeftBrowLowerer",
    "LeftCheekPuff", "LeftCheekRaiser", "LeftDimpler", "LeftEyeClosed", "LeftEyeUpperLidRaiser",
    "LeftInnerBrowRaiser", "LeftLipCornerDown", "LeftLipCornerPuller", "LeftLipStretcher",
    "LeftLowerLipDepressor", "LeftNoseWrinkler", "LeftOuterBrowRaiser", "LeftUpperLipRaiser", "LipPresser",
    "LipsTogether", "LowerLipSuck", "MouthLeft", "MouthRight", "Pucker", "RightBrowLowerer", "RightCheekPuff",
    "RightCheekRaiser", "RightDimpler", "RightEyeClosed", "RightEyeUpperLidRaiser", "RightInnerBrowRaiser",
    "RightLipCornerDown", "RightLipCornerPuller", "RightLipStretcher", "RightLowerLipDepressor",
    "RightNoseWrinkler", "RightOuterBrowRaiser", "RightUpperLipRaiser", "TongueDown", "TongueOut", "TongueUp",
    "UpperLipSuck",
];

/// A mesh's FACS control code (Roblox's head meshes name them c_JD, l_EC …)
/// → the FaceControls property. Full names pass through.
pub fn control_property(code: &str) -> &str {
    match code {
        "c_COR" => "Corrugator", "c_CR" => "ChinRaiser", "c_CRUL" => "ChinRaiserUpperLip",
        "c_ELD" => "EyesLookDown", "c_ELL" => "EyesLookLeft", "c_ELR" => "EyesLookRight",
        "c_ELU" => "EyesLookUp", "c_FN" => "Funneler", "c_FP" => "FlatPucker", "c_JD" => "JawDrop",
        "c_JL" => "JawLeft", "c_JR" => "JawRight", "c_LLS" => "LowerLipSuck", "c_LP" => "LipPresser",
        "c_LPT" => "LipsTogether", "c_ML" => "MouthLeft", "c_MR" => "MouthRight", "c_PK" => "Pucker",
        "c_TD" => "TongueDown", "c_TO" => "TongueOut", "c_TU" => "TongueUp", "c_ULS" => "UpperLipSuck",
        "l_BL" => "LeftBrowLowerer", "l_CHP" => "LeftCheekPuff", "l_CHR" => "LeftCheekRaiser",
        "l_DM" => "LeftDimpler", "l_EC" => "LeftEyeClosed", "l_EULR" => "LeftEyeUpperLidRaiser",
        "l_IBR" => "LeftInnerBrowRaiser", "l_LCD" => "LeftLipCornerDown", "l_LCP" => "LeftLipCornerPuller",
        "l_LLD" => "LeftLowerLipDepressor", "l_LS" => "LeftLipStretcher", "l_NW" => "LeftNoseWrinkler",
        "l_OBR" => "LeftOuterBrowRaiser", "l_ULR" => "LeftUpperLipRaiser",
        "r_BL" => "RightBrowLowerer", "r_CHP" => "RightCheekPuff", "r_CHR" => "RightCheekRaiser",
        "r_DM" => "RightDimpler", "r_EC" => "RightEyeClosed", "r_EULR" => "RightEyeUpperLidRaiser",
        "r_IBR" => "RightInnerBrowRaiser", "r_LCD" => "RightLipCornerDown", "r_LCP" => "RightLipCornerPuller",
        "r_LLD" => "RightLowerLipDepressor", "r_LS" => "RightLipStretcher", "r_NW" => "RightNoseWrinkler",
        "r_OBR" => "RightOuterBrowRaiser", "r_ULR" => "RightUpperLipRaiser",
        other => other,
    }
}

/// A mesh's FACS rig: bone names, control names, per-bone per-column pose
/// (position, rotation in degrees), correctives as control-index tuples.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facs {
    pub bones: Vec<String>,
    pub controls: Vec<String>,
    /// [6][rows * cols]: px, py, pz, rx, ry, rz; column-major per row.
    pub mats: [Vec<f32>; 6],
    pub cols: usize,
    pub two: Vec<[u16; 2]>,
    pub three: Vec<[u16; 3]>,
}

struct Rd<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.o..self.o.checked_add(n)?)?;
        self.o += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

fn names(b: &[u8]) -> Vec<String> {
    b.split(|c| *c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Parses a FACS data block (the format both mesh layouts share).
pub fn parse_facs_block(b: &[u8]) -> Option<Facs> {
    let mut r = Rd { b, o: 0 };
    let bone_names = r.u32()? as usize;
    let control_names = r.u32()? as usize;
    let quant = r.u64()? as usize;
    let two = r.u32()? as usize;
    let three = r.u32()? as usize;
    let bones = names(r.take(bone_names)?);
    let controls = names(r.take(control_names)?);
    let qb = r.take(quant)?;
    let mut q = Rd { b: qb, o: 0 };
    let mut mats: [Vec<f32>; 6] = Default::default();
    let mut cols = 0;
    for m in mats.iter_mut() {
        let version = q.u16()?;
        let rows = q.u32()? as usize;
        let c = q.u32()? as usize;
        cols = c;
        let n = rows.checked_mul(c)?;
        if n > 1 << 24 {
            return None;
        }
        match version {
            1 => {
                for _ in 0..n {
                    m.push(q.f32()?);
                }
            }
            2 => {
                let lo = q.f32()?;
                let hi = q.f32()?;
                for _ in 0..n {
                    let v = q.u16()? as f32;
                    m.push(lo + v * (hi - lo) / 65535.0);
                }
            }
            _ => return None,
        }
    }
    let tb = r.take(two)?;
    let two = tb
        .chunks_exact(4)
        .map(|c| [u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]])])
        .collect();
    let hb = r.take(three)?;
    let three = hb
        .chunks_exact(6)
        .map(|c| {
            [
                u16::from_le_bytes([c[0], c[1]]),
                u16::from_le_bytes([c[2], c[3]]),
                u16::from_le_bytes([c[4], c[5]]),
            ]
        })
        .collect();
    Some(Facs { bones, controls, mats, cols, two, three })
}

/// The FACS rig of a mesh file, if it has one.
pub fn parse_mesh_facs(bytes: &[u8]) -> Option<Facs> {
    let nl = bytes.iter().position(|c| *c == b'\n')?;
    let ver = std::str::from_utf8(&bytes[..nl]).ok()?.trim().strip_prefix("version ")?.to_string();
    let body = &bytes[nl + 1..];
    if ver.starts_with("6.") || ver.starts_with("7.") {
        let mut r = Rd { b: body, o: 0 };
        while r.o + 16 <= body.len() {
            let kind = r.take(8)?;
            let _v = r.u32()?;
            let size = r.u32()? as usize;
            let data = r.take(size)?;
            if kind.starts_with(b"FACS") {
                let mut d = Rd { b: data, o: 0 };
                let n = d.u32()? as usize;
                return parse_facs_block(d.take(n)?);
            }
        }
        return None;
    }
    if !(ver.starts_with("4.") || ver.starts_with("5.")) {
        return None;
    }
    let mut r = Rd { b: body, o: 0 };
    let header = r.u16()? as usize;
    let _lod_type = r.u16()?;
    let verts = r.u32()? as usize;
    let faces = r.u32()? as usize;
    let lods = r.u16()? as usize;
    let bones = r.u16()? as usize;
    let bone_names = r.u32()? as usize;
    let subsets = r.u16()? as usize;
    let _hq = r.take(2)?;
    let (facs_format, facs_size) = if header >= 32 { (r.u32()?, r.u32()? as usize) } else { (0, 0) };
    if facs_size == 0 || facs_format == 0 {
        return None;
    }
    let mut off = header;
    off += verts * 40;
    if bones > 0 {
        off += verts * 8;
    }
    off += faces * 12 + lods * 4 + bones * 60 + bone_names + subsets * 72;
    parse_facs_block(body.get(off..off + facs_size)?)
}

/// Each face bone's pose for the given control values: (position, Euler
/// XYZ degrees).
pub fn pose(f: &Facs, value: &dyn Fn(&str) -> f32) -> Vec<(String, [f32; 3], [f32; 3])> {
    let n = f.controls.len();
    let mut w: Vec<f32> = f.controls.iter().map(|c| value(control_property(c))).collect();
    for t in &f.two {
        w.push(t.iter().map(|i| w.get(*i as usize).copied().unwrap_or(0.0)).product());
    }
    for t in &f.three {
        w.push(t.iter().map(|i| w.get(*i as usize).copied().unwrap_or(0.0)).product());
    }
    let cols = f.cols.max(1);
    let _ = n;
    // Only the columns with weight (a face usually moves a few controls).
    let live: Vec<(usize, f32)> = w.iter().copied().enumerate().take(cols).filter(|(_, x)| *x != 0.0).collect();
    f.bones
        .iter()
        .enumerate()
        .map(|(row, name)| {
            let mut acc = [0.0f32; 6];
            for (k, m) in f.mats.iter().enumerate() {
                for &(c, wc) in &live {
                    acc[k] += wc * m.get(row * cols + c).copied().unwrap_or(0.0);
                }
            }
            (name.clone(), [acc[0], acc[1], acc[2]], [acc[3], acc[4], acc[5]])
        })
        .collect()
}

/// Mesh FACS rigs by asset id: None while loading, Some(None) without FACS.
static RIGS: std::sync::LazyLock<Mutex<HashMap<u64, Option<Option<Arc<Facs>>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn rig_for(id: u64) -> Option<Arc<Facs>> {
    let mut g = RIGS.lock().unwrap_or_else(|e| e.into_inner());
    match g.get(&id) {
        Some(Some(r)) => return r.clone(),
        Some(None) => return None,
        None => {}
    }
    g.insert(id, None);
    std::thread::spawn(move || {
        let r = crate::assets::public_cached(id)
            .ok()
            .and_then(|b| parse_mesh_facs(&b))
            .map(Arc::new);
        if let Ok(mut g) = RIGS.lock() {
            g.insert(id, Some(r));
        }
    });
    None
}

/// FaceControls whose values changed (or that wait for their mesh).
#[derive(Default)]
pub struct Dirty(pub HashSet<Ref>);

pub fn is_control(key: &str) -> bool {
    CONTROLS.contains(&key)
}

pub fn prop_get(lua: &Lua, r: Ref, key: &str) -> mlua::Result<Option<Value>> {
    if !is_control(key) {
        return Ok(None);
    }
    let v = match world(lua)?.d().prop(r, key) {
        Some(Variant::Float32(f)) => f as f64,
        Some(Variant::Float64(f)) => f,
        _ => 0.0,
    };
    Ok(Some(Value::Number(v)))
}

pub fn newindex(lua: &Lua, r: Ref, key: &str, value: &Value) -> mlua::Result<bool> {
    if !is_control(key) {
        return Ok(false);
    }
    let n = num(value).ok_or_else(|| {
        rt_err(format!("Unable to assign property {key}. float expected, got {}", value.type_name()))
    })?;
    store_prop(lua, r, key, Variant::Float32((n as f32).clamp(0.0, 1.0)))?;
    if let Some(mut d) = lua.app_data_mut::<Dirty>() {
        d.0.insert(r);
    }
    Ok(true)
}

fn mesh_id(lua: &Lua, part: Ref) -> mlua::Result<Option<u64>> {
    let w = world(lua)?;
    let d = w.d();
    let s = ["MeshContent", "MeshId"].iter().find_map(|k| match d.prop(part, k) {
        Some(Variant::Content(c)) => c.as_uri().map(str::to_string),
        Some(Variant::ContentId(c)) => Some(c.as_str().to_string()),
        Some(Variant::String(s)) => Some(s),
        _ => None,
    });
    Ok(s.and_then(|s| crate::assets::asset_ref(&s)).map(|a| a.id))
}

fn v3(d: &crate::doc::Document, r: Ref, k: &str) -> Option<glam::Vec3> {
    match d.prop(r, k) {
        Some(Variant::Vector3(v)) => Some(glam::Vec3::new(v.x, v.y, v.z)),
        _ => None,
    }
}

/// Poses the bones of every changed FaceControls whose mesh has FACS.
pub fn step(lua: &Lua) -> mlua::Result<()> {
    let pending: Vec<Ref> = match lua.app_data_mut::<Dirty>() {
        Some(mut d) if !d.0.is_empty() => d.0.drain().collect(),
        _ => return Ok(()),
    };
    let mut again = Vec::new();
    for fc in pending {
        let part = {
            let w = world(lua)?;
            let d = w.d();
            let Some(p) = d.dom.get_by_ref(fc).map(|i| i.parent()) else { continue };
            if !d.class_of(p).is_some_and(|c| c == "MeshPart") {
                continue;
            }
            p
        };
        let Some(id) = mesh_id(lua, part)? else { continue };
        let Some(rig) = rig_for(id) else {
            // Still loading (or no FACS): look again next frame while loading.
            if RIGS.lock().map(|g| matches!(g.get(&id), Some(None))).unwrap_or(false) {
                again.push(fc);
            }
            continue;
        };
        let (values, scale, bones) = {
            let w = world(lua)?;
            let d = w.d();
            let values: HashMap<&str, f32> = CONTROLS
                .iter()
                .map(|c| (*c, match d.prop(fc, c) { Some(Variant::Float32(f)) => f, _ => 0.0 }))
                .collect();
            let scale = match (v3(d, part, "Size"), v3(d, part, "MeshSize").or_else(|| v3(d, part, "InitialSize"))) {
                (Some(s), Some(m)) if m.x > 0.0 && m.y > 0.0 && m.z > 0.0 => s / m,
                _ => glam::Vec3::ONE,
            };
            let bones: HashMap<String, Ref> = d
                .dom
                .descendants_of(part)
                .filter(|i| i.class == "Bone")
                .map(|i| (i.name.to_string(), i.referent()))
                .collect();
            (values, scale, bones)
        };
        for (name, p, r) in pose(&rig, &|c| values.get(c).copied().unwrap_or(0.0)) {
            let Some(&b) = bones.get(&name) else { continue };
            let rot = glam::Mat3::from_euler(glam::EulerRot::XYZ, r[0].to_radians(), r[1].to_radians(), r[2].to_radians());
            let xf = crate::math::Xform { pos: glam::Vec3::from(p) * scale, rot };
            store_prop(lua, b, "Transform", Variant::CFrame(xf.to_cframe()))?;
        }
    }
    if !again.is_empty() {
        if let Some(mut d) = lua.app_data_mut::<Dirty>() {
            d.0.extend(again);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> Vec<u8> {
        // 1 bone ("JawBone"), 2 controls (JawDrop, Pucker), one corrective.
        let bones = b"JawBone\0".to_vec();
        let controls = b"JawDrop\0Pucker\0".to_vec();
        let mut quant = Vec::new();
        for k in 0..6 {
            quant.extend(1u16.to_le_bytes());
            quant.extend(1u32.to_le_bytes());
            quant.extend(3u32.to_le_bytes()); // 2 controls + 1 corrective
            let vals: [f32; 3] = if k == 1 { [-0.5, 0.0, 0.25] } else if k == 3 { [20.0, 0.0, 0.0] } else { [0.0; 3] };
            for v in vals {
                quant.extend(v.to_le_bytes());
            }
        }
        let two: Vec<u8> = [0u16, 1u16].iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend((bones.len() as u32).to_le_bytes());
        b.extend((controls.len() as u32).to_le_bytes());
        b.extend((quant.len() as u64).to_le_bytes());
        b.extend((two.len() as u32).to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(bones);
        b.extend(controls);
        b.extend(quant);
        b.extend(two);
        b
    }

    #[test]
    fn facs_block_poses_bones_with_correctives() {
        let f = parse_facs_block(&block()).unwrap();
        assert_eq!(f.bones, vec!["JawBone"]);
        assert_eq!(f.controls, vec!["JawDrop", "Pucker"]);
        let p = pose(&f, &|c| if c == "JawDrop" { 1.0 } else if c == "Pucker" { 0.5 } else { 0.0 });
        // y: -0.5·1 + 0·0.5 + 0.25·(1·0.5); rx: 20·1
        assert!((p[0].1[1] - (-0.375)).abs() < 1e-6, "{p:?}");
        assert!((p[0].2[0] - 20.0).abs() < 1e-6);
    }

    #[test]
    fn chunked_mesh_finds_facs() {
        let blk = block();
        let mut m = b"version 7.00\n".to_vec();
        m.extend(b"COREMESH");
        m.extend(1u32.to_le_bytes());
        m.extend(0u32.to_le_bytes());
        m.extend(b"FACS\0\0\0\0");
        m.extend(1u32.to_le_bytes());
        m.extend(((blk.len() + 4) as u32).to_le_bytes());
        m.extend((blk.len() as u32).to_le_bytes());
        m.extend(&blk);
        assert_eq!(parse_mesh_facs(&m).unwrap().controls.len(), 2);
    }
}
