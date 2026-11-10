//! Turns the document into renderable parts and lighting.

use std::collections::HashMap;

use glam::{Mat3, Vec3, Vec4};
use rbx_dom_weak::types::{BrickColor, Ref, Variant};
use rbx_dom_weak::{Instance, Ustr};

use crate::doc::{self, DocKind, Document};
use crate::math::Xform;
use crate::mech;
use crate::render::{LineVertex, MeshKind, PartDraw};

/// A part in the scene, with the ref it came from.
#[derive(Debug, Clone)]
pub struct ScenePart {
    pub r: Ref,
    pub draw: PartDraw,
    /// The part's own Size (picking and gizmos use it, not the drawn shape).
    pub size: Vec3,
    /// Mesh geometry this part would like drawn, if it can be loaded.
    pub mesh_asset: Option<MeshAsset>,
    /// MeshPart TextureID, SurfaceAppearance ColorMap or SpecialMesh TextureId.
    pub texture_id: Option<String>,
    /// How that texture covers the part (Opaque, Over, Cutout).
    pub tex_kind: crate::render::TexMode,
    /// SurfaceAppearance emission: mask id and EmissiveTint² × EmissiveStrength.
    pub emissive: Option<(String, [f32; 3])>,
    /// Decal / Texture children (and a T-shirt graphic on the torso).
    pub decals: Vec<DecalDesc>,
    /// Classic clothing on a character body part: (part name, shirt id, pants id).
    pub clothing: Option<(String, Option<String>, Option<String>)>,
    /// Locked parts can't be clicked in the viewport.
    pub locked: bool,
    /// A MaterialVariant replaces the material's own texture pack.
    pub variant: bool,
    /// The box decals sit on: the part's Size, or a Head mesh's drawn size.
    pub decal_size: Vec3,
    /// Children / siblings whose changes alter this part's look (meshes,
    /// decals, clothing).
    pub deps: Vec<Ref>,
    /// Offset of the drawn shape from the part (mesh Offset), in part space.
    pub draw_offset: Vec3,
    /// Extra rotation of the drawn shape (vertical cylinder meshes).
    pub draw_rot: Mat3,
    /// The part's own CFrame (picking, boxes and gizmos use it with `size`).
    pub xf: Xform,
    /// The part as Studio's light grid voxelizes it (lightgrid.rs); None
    /// when it doesn't (CastShadow off, fully transparent, file meshes).
    pub light: Option<crate::lightgrid::Prim>,
}

/// A Decal or Texture on one face of a part.
#[derive(Debug, Clone)]
pub struct DecalDesc {
    /// Enum.NormalId: Right 0, Top 1, Back 2, Left 3, Bottom 4, Front 5.
    pub face: u32,
    pub id: String,
    /// Linear RGB tint (Color3).
    pub color: [f32; 3],
    pub transparency: f32,
    /// Generated base material layers inherit the host opacity; authored decals do not.
    pub base_surface: bool,
    /// Texture tiling: studs per tile (u, v) and offset studs (u, v).
    pub tiling: Option<[f32; 4]>,
}

/// A SurfaceAppearance saved with a TexturePack (newer Studio packs its maps
/// into one asset) draws as if absent while material packs are off: the
/// offline Studio reads the pack instead of ColorMap, can't fetch it
/// ("Failed to fetch TexturePack in …") and draws the part's plain Color
/// (round 26: Place B Newer's sky domes, orange and pale in Studio, clouds in
/// rustudio). With packs on rustudio keeps the ColorMap, the online look.
fn sa_unloaded(doc: &Document, sa: Ref) -> bool {
    !crate::matpack::enabled()
        && (content_string(doc.prop(sa, "TexturePack")).is_some()
            || content_string(doc.prop(sa, "TexturePackContent")).is_some())
}

/// Separate PBR maps, read as linear data (never sRGB).
pub fn surface_maps(doc: &Document, r: Ref) -> Option<[Option<String>; 3]> {
    let inst = doc.dom.get_by_ref(r)?;
    let sa = inst.children().iter().copied().find(|c| doc.class_of(*c).is_some_and(|k| k == "SurfaceAppearance"))?;
    if sa_unloaded(doc, sa) { return None; }
    let maps = ["NormalMap", "RoughnessMap", "MetalnessMap"].map(|n|
        content_string(doc.prop(sa, n)).or_else(|| content_string(doc.prop(sa, &format!("{n}Content")))));
    maps.iter().any(Option::is_some).then_some(maps)
}

/// A SurfaceAppearance's emission: EmissiveMaskContent with
/// EmissiveTint (squared) × EmissiveStrength.
fn surface_emissive(doc: &Document, inst: &Instance) -> Option<(String, [f32; 3])> {
    let sa = inst
        .children()
        .iter()
        .copied()
        .find(|c| doc.class_of(*c).is_some_and(|k| k == "SurfaceAppearance"))?;
    if sa_unloaded(doc, sa) {
        return None;
    }
    let mask = content_string(doc.prop(sa, "EmissiveMaskContent"))
        .or_else(|| content_string(doc.prop(sa, "EmissiveMask")))?;
    let strength = f32_of(doc.prop(sa, "EmissiveStrength")).unwrap_or(1.0);
    if strength <= 0.0 {
        return None;
    }
    let tint = color_of(doc.prop(sa, "EmissiveTint")).unwrap_or([255, 255, 255]);
    Some((
        mask,
        [
            srgb_to_linear(tint[0]) * strength,
            srgb_to_linear(tint[1]) * strength,
            srgb_to_linear(tint[2]) * strength,
        ],
    ))
}

/// Decals, Textures and SurfaceAppearance of a part.
fn surface_details(
    doc: &Document,
    inst: &Instance,
) -> (Option<(String, crate::render::TexMode)>, Vec<DecalDesc>) {
    let mut texture = None;
    let mut decals = Vec::new();
    for c in inst.children() {
        let Some(child) = doc.dom.get_by_ref(*c) else {
            continue;
        };
        match child.class.as_str() {
            "SurfaceAppearance" if sa_unloaded(doc, *c) => {}
            "SurfaceAppearance" => {
                // AlphaMode: Overlay 0 (alpha blends over Color), Transparency 1.
                let kind = if enum_u32(doc.prop(*c, "AlphaMode")) == Some(1) {
                    crate::render::TexMode::Cutout
                } else {
                    crate::render::TexMode::Over
                };
                texture = content_string(doc.prop(*c, "ColorMap"))
                    .or_else(|| content_string(doc.prop(*c, "ColorMapContent")))
                    .map(|t| (t, kind));
            }
            "Decal" | "Texture" => {
                let Some(id) = content_string(doc.prop(*c, "Texture"))
                    .or_else(|| content_string(doc.prop(*c, "TextureContent")))
                else {
                    continue;
                };
                let tint = match doc.prop(*c, "Color3") {
                    Some(Variant::Color3(col)) => [col.r * col.r, col.g * col.g, col.b * col.b],
                    _ => [1.0, 1.0, 1.0],
                };
                let tiling = (child.class == "Texture").then(|| {
                    [
                        f32_of(doc.prop(*c, "StudsPerTileU")).unwrap_or(2.0),
                        f32_of(doc.prop(*c, "StudsPerTileV")).unwrap_or(2.0),
                        f32_of(doc.prop(*c, "OffsetStudsU")).unwrap_or(0.0),
                        f32_of(doc.prop(*c, "OffsetStudsV")).unwrap_or(0.0),
                    ]
                });
                decals.push(DecalDesc {
                    face: enum_u32(doc.prop(*c, "Face")).unwrap_or(5),
                    id,
                    color: tint,
                    transparency: f32_of(doc.prop(*c, "Transparency")).unwrap_or(0.0),
                    base_surface: false,
                    tiling,
                });
            }
            _ => {}
        }
    }
    (texture, decals)
}

/// Shirt / Pants templates (and the T-shirt graphic) on the part's character model.
fn character_clothing(
    doc: &Document,
    inst: &Instance,
) -> (
    Option<(String, Option<String>, Option<String>)>,
    Option<String>,
) {
    if crate::assets::clothing_role(&inst.name).is_none() {
        return (None, None);
    }
    let Some(model) = doc.dom.get_by_ref(inst.parent()) else {
        return (None, None);
    };
    let mut shirt = None;
    let mut pants = None;
    let mut graphic = None;
    for c in model.children() {
        match doc.class_of(*c).map(|s| s.to_string()).as_deref() {
            Some("Shirt") => {
                shirt = content_string(doc.prop(*c, "ShirtTemplate"))
                    .or_else(|| content_string(doc.prop(*c, "ShirtTemplateContent")))
            }
            Some("Pants") => {
                pants = content_string(doc.prop(*c, "PantsTemplate"))
                    .or_else(|| content_string(doc.prop(*c, "PantsTemplateContent")))
            }
            Some("ShirtGraphic") => {
                graphic = content_string(doc.prop(*c, "Graphic"))
                    .or_else(|| content_string(doc.prop(*c, "GraphicContent")))
            }
            _ => {}
        }
    }
    let torso = inst.name == "Torso" || inst.name == "UpperTorso";
    let clothing = (shirt.is_some() || pants.is_some()).then(|| (inst.name.clone(), shirt, pants));
    (clothing, if torso { graphic } else { None })
}

/// A short content hash for a mesh blob without its own (MeshData).
fn blake3_hex(data: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut h);
    format!("{:016x}{:08x}", h.finish(), data.len())
}

/// A union's solved geometry (its SolidMeshHolder: Studio measurements "Unions"),
/// registered as generated content under an id from the blob's hash. With
/// UsePartColor the id ends in `#partcolor` and the loader drops the child
/// colours, so the union's own Color shows.
pub(crate) fn union_mesh(doc: &Document, r: Ref) -> Option<MeshAsset> {
    let v = doc.prop(r, "SolidMeshHolder");
    // The solved SolidMesh, else the legacy CSGMDL mesh (MeshData2 / MeshData).
    let blob: Option<(Vec<u8>, String)> = match &v {
        Some(Variant::NetAssetRef(h)) if !h.data().is_empty() => Some((
            h.data().to_vec(),
            h.hash().as_bytes().iter().take(16).map(|b| format!("{b:02x}")).collect(),
        )),
        _ => match doc.prop(r, "MeshData2") {
            Some(Variant::SharedString(s)) if !s.data().is_empty() => {
                Some((s.data().to_vec(), s.hash().as_bytes().iter().take(16).map(|b| format!("{b:02x}")).collect()))
            }
            _ => match doc.prop(r, "MeshData") {
                Some(Variant::BinaryString(b)) if !AsRef::<[u8]>::as_ref(&b).is_empty() => {
                    let data: Vec<u8> = AsRef::<[u8]>::as_ref(&b).to_vec();
                    let h = blake3_hex(&data);
                    Some((data, h))
                }
                _ => None,
            },
        },
    };
    let (bytes, hash) = blob?;
    if std::env::var_os("RUSTUDIO_UNION_DENSITY_TRACE").is_some() {
        static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
        if SEEN.get_or_init(Default::default).lock().unwrap().insert(hash.clone()) {
            if let Ok(m) = crate::mesh::parse(&bytes) {
                eprintln!("UNION_DENSITY {hash} size={:?} initial={:?} mesh_size={:?} bounds={:?}..{:?} tags={:?} (UV: signed projection of normalized_position * Size; material multiplier=pack.tiling / studs_per_tile)", doc.prop(r,"Size"), doc.prop(r,"InitialSize"), doc.prop(r,"MeshSize"), m.min, m.max, { let mut counts = [0usize; 16]; for &t in &m.projection_tags { counts[t as usize] += 1; } counts });
            }
        }
    }
    let base = format!("rbxtemp://rustudio/union/{hash}");
    if crate::assets::generated(&base).is_none() {
        crate::assets::register_generated(&base, bytes.clone());
    }
    let part_color = matches!(doc.prop(r, "UsePartColor"), Some(Variant::Bool(true)));
    let id = if part_color {
        let id = format!("{base}#partcolor");
        if crate::assets::generated(&id).is_none() {
            crate::assets::register_generated(&id, bytes);
        }
        id
    } else {
        base
    };
    Some(MeshAsset {
        id,
        fit_to_part: true,
        scale: Vec3::ONE,
        offset: Vec3::ZERO,
    })
}

/// A MeshPart's mesh (stretched to the part's Size) or a SpecialMesh file mesh
/// (drawn at its own size times Scale, shifted by Offset).
#[derive(Debug, Clone)]
pub struct MeshAsset {
    pub id: String,
    pub fit_to_part: bool,
    pub scale: Vec3,
    pub offset: Vec3,
}

/// The MaterialVariant a part is drawn with: its MaterialVariant name, else
/// MaterialService's `<Material>Name` override for its Material; the variant
/// (in MaterialService, same BaseMaterial) gives its ColorMap and StudsPerTile.
/// MaterialService.Use2022Materials (absent: false, the pre-2022 table,
/// which is what Studio draws for a place without the setting).
pub fn use_2022_materials(doc: &Document) -> bool {
    doc.find_child(doc.dom.root_ref(), "MaterialService")
        .and_then(|ms| {
            ["Use2022Materials", "Use2022MaterialsXml"]
                .iter()
                .find_map(|k| match doc.prop(ms, k) {
                    Some(Variant::Bool(b)) => Some(b),
                    _ => None,
                })
        })
        .unwrap_or(false)
}

fn material_variant(doc: &Document, part: Ref, material: u32) -> Option<(String, f32)> {
    let ms = doc.find_child(doc.dom.root_ref(), "MaterialService")?;
    let mat_name = crate::ltypes::LEnumItem::from_value("Material", material)?.name;
    let name = match doc.prop(part, "MaterialVariant") {
        Some(Variant::String(s)) if !s.is_empty() => s,
        _ => match doc.prop(ms, &format!("{mat_name}Name")) {
            Some(Variant::String(s)) if !s.is_empty() => s,
            _ => return None,
        },
    };
    let v = doc
        .dom
        .descendants_of(ms)
        .find(|i| {
            i.class == "MaterialVariant"
                && i.name == name
                && enum_u32(doc.prop(i.referent(), "BaseMaterial")).unwrap_or(256) == material
        })?
        .referent();
    let map = content_string(doc.prop(v, "ColorMap"))
        .or_else(|| content_string(doc.prop(v, "ColorMapContent")))?;
    Some((
        map,
        f32_of(doc.prop(v, "StudsPerTile"))
            .unwrap_or(10.0)
            .max(0.01),
    ))
}

pub fn content_string(v: Option<Variant>) -> Option<String> {
    match v? {
        Variant::ContentId(c) => Some(c.as_str().to_string()),
        Variant::Content(c) => c.as_uri().map(str::to_string).or_else(|| {
            c.as_object().and_then(|o| crate::runtime::editable::content_uri(o, None))
        }),
        Variant::String(s) => Some(s),
        _ => None,
    }
    // Roblox reads content ids trimmed (Place B has "…?id=125882923   ").
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// What changed in the scene since the viewport last asked.
#[derive(Default)]
pub struct SceneChanges {
    /// Everything was rebuilt.
    pub full: bool,
    /// Parts added or updated.
    pub changed: Vec<Ref>,
    /// Parts that only moved (CFrame changed).
    pub moved: Vec<Ref>,
    pub removed: Vec<Ref>,
}

/// The parts to draw, kept up to date from the document's change journal so
/// only changed parts are re-read each frame.
#[derive(Default)]
pub struct Scene {
    /// Document the parts came from (0 = none yet).
    pub doc_id: u64,
    pub parts: Vec<ScenePart>,
    pub index: HashMap<Ref, usize>,
    /// Bounding sphere per part (centre, radius), parallel to `parts`.
    pub bounds: Vec<Vec4>,
    is_part: HashMap<Ustr, bool>,
    /// Instance → parts whose look depends on it.
    owners: HashMap<Ref, Vec<Ref>>,
    changes: SceneChanges,
    /// The voxel terrain needs re-meshing.
    pub terrain_dirty: bool,
}

/// A colour channel in linear light the way Studio's shaders take it:
/// squared (gamma 2.0; docs/MECHANICS.md "Lighting").
pub fn srgb_to_linear(c: u8) -> f32 {
    (c as f32 / 255.0).powi(2)
}

fn color_of(v: Option<Variant>) -> Option<[u8; 3]> {
    match v? {
        Variant::Color3uint8(c) => Some([c.r, c.g, c.b]),
        Variant::Color3(c) => {
            let c: rbx_dom_weak::types::Color3uint8 = c.into();
            Some([c.r, c.g, c.b])
        }
        Variant::BrickColor(b) => {
            let c = b.to_color3uint8();
            Some([c.r, c.g, c.b])
        }
        _ => None,
    }
}

pub fn enum_u32(v: Option<Variant>) -> Option<u32> {
    match v? {
        Variant::Enum(e) => Some(e.to_u32()),
        Variant::EnumItem(e) => Some(e.value),
        _ => None,
    }
}

pub fn f32_of(v: Option<Variant>) -> Option<f32> {
    match v? {
        Variant::Float32(f) => Some(f),
        Variant::Float64(f) => Some(f as f32),
        Variant::Int32(i) => Some(i as f32),
        _ => None,
    }
}

pub fn vec3_of(v: Option<Variant>) -> Option<Vec3> {
    match v? {
        Variant::Vector3(v) => Some(Vec3::new(v.x, v.y, v.z)),
        _ => None,
    }
}

pub fn xform_of(doc: &Document, r: Ref) -> Option<Xform> {
    match doc.prop(r, "CFrame")? {
        // Old places can hold slightly skewed rotations: square them up so
        // nothing draws stretched.
        Variant::CFrame(cf) => Some(Xform::from_cframe(&cf).orthonormalized()),
        _ => None,
    }
}

impl Scene {
    pub fn is_part_class(&mut self, class: Ustr) -> bool {
        *self
            .is_part
            .entry(class)
            .or_insert_with(|| doc::is_a(&class, "BasePart") && class != "Terrain")
    }

    fn root(doc: &Document) -> Option<Ref> {
        match doc.kind {
            DocKind::Place => doc.workspace(),
            DocKind::Model => Some(doc.dom.root_ref()),
        }
    }

    /// Brings the part list up to date with the document.
    pub fn sync(&mut self, doc: &mut Document) {
        let marks = doc.marks_for(doc::Reader::Scene);
        let marks = if self.doc_id != doc.id {
            doc::Marks::All
        } else {
            marks
        };
        match marks {
            doc::Marks::All => self.rebuild(doc),
            doc::Marks::Some(m) if m.is_empty() => {}
            doc::Marks::Some(m) => self.apply(doc, &m),
        }
    }

    /// Changes since the last call.
    pub fn take_changes(&mut self) -> SceneChanges {
        std::mem::take(&mut self.changes)
    }

    fn rebuild(&mut self, doc: &Document) {
        self.doc_id = doc.id;
        self.parts.clear();
        self.index.clear();
        self.bounds.clear();
        self.owners.clear();
        self.changes = SceneChanges {
            full: true,
            ..Default::default()
        };
        self.terrain_dirty = true;
        let Some(root) = Self::root(doc) else { return };
        let refs: Vec<Ref> = doc.dom.descendants_of(root).map(|i| i.referent()).collect();
        for r in refs {
            let Some(inst) = doc.dom.get_by_ref(r) else {
                continue;
            };
            if !self.is_part_class(inst.class) {
                continue;
            }
            if let Some(p) = part_draw(doc, inst) {
                self.insert(p);
            }
        }
    }

    fn insert(&mut self, p: ScenePart) {
        for d in &p.deps {
            let o = self.owners.entry(*d).or_default();
            if !o.contains(&p.r) {
                o.push(p.r);
            }
        }
        let b = bound_of(&p);
        match self.index.get(&p.r) {
            Some(&i) => {
                self.parts[i] = p;
                self.bounds[i] = b;
            }
            None => {
                self.index.insert(p.r, self.parts.len());
                self.parts.push(p);
                self.bounds.push(b);
            }
        }
    }

    fn remove(&mut self, r: Ref) -> bool {
        let Some(i) = self.index.remove(&r) else {
            return false;
        };
        self.parts.swap_remove(i);
        self.bounds.swap_remove(i);
        if let Some(moved) = self.parts.get(i) {
            self.index.insert(moved.r, i);
        }
        true
    }

    fn under(doc: &Document, root: Ref, r: Ref) -> bool {
        let mut cur = r;
        while let Some(i) = doc.dom.get_by_ref(cur) {
            if cur == root {
                return true;
            }
            cur = i.parent();
        }
        false
    }

    /// Applies journal marks: re-reads only the parts they touch.
    fn apply(&mut self, doc: &Document, marks: &[doc::Mark]) {
        let Some(root) = Self::root(doc) else { return };
        let mut full: std::collections::HashSet<Ref> = Default::default();
        let mut moved: std::collections::HashSet<Ref> = Default::default();
        let cframe = rbx_dom_weak::ustr("CFrame");
        let marks_scope = crate::bench::scope("ss_marks");
        for m in marks {
            match *m {
                doc::Mark::Prop(r, name)
                    if matches!(
                        name.as_str(),
                        "SmoothGrid" | "MaterialColors" | "WaterColor" | "WaterTransparency" | "WaterReflectance"
                    ) =>
                {
                    if doc.class_of(r).is_some_and(|c| c == "Terrain") {
                        self.terrain_dirty = true;
                    }
                }
                doc::Mark::Tree(r) if doc.class_of(r).is_some_and(|c| c == "Terrain") => {
                    self.terrain_dirty = true;
                }
                doc::Mark::Prop(r, name) => {
                    if self.index.contains_key(&r) {
                        if name == cframe {
                            moved.insert(r);
                        } else {
                            full.insert(r);
                        }
                    } else if doc.class_of(r).is_some_and(|c| self.is_part_class(c)) {
                        full.insert(r);
                    }
                    if let Some(o) = self.owners.get(&r) {
                        full.extend(o.iter().copied());
                    }
                }
                doc::Mark::Tree(r) => {
                    for d in doc.subtree(r) {
                        if self.index.contains_key(&d)
                            || doc.class_of(d).is_some_and(|c| self.is_part_class(c))
                        {
                            full.insert(d);
                        }
                        if let Some(o) = self.owners.get(&d) {
                            full.extend(o.iter().copied());
                        }
                    }
                    // A new decal / mesh / clothing item changes its parent part
                    // or the body parts beside it.
                    if let Some(parent) = doc.dom.get_by_ref(r).map(|i| i.parent()) {
                        if self.index.contains_key(&parent) {
                            full.insert(parent);
                        }
                        let clothing = doc.class_of(r).is_some_and(|c| {
                            matches!(c.as_str(), "Shirt" | "Pants" | "ShirtGraphic")
                        });
                        if clothing {
                            for c in doc
                                .dom
                                .get_by_ref(parent)
                                .map(|i| i.children().to_vec())
                                .unwrap_or_default()
                            {
                                if self.index.contains_key(&c) {
                                    full.insert(c);
                                }
                            }
                        }
                    }
                }
            }
        }
        drop(marks_scope);
        let _draws_scope = crate::bench::scope("ss_part_draw");
        crate::bench::add("ss_full_count", 0.0, full.len() as u64);
        for r in full.iter().copied() {
            let keep = doc
                .dom
                .get_by_ref(r)
                .filter(|i| self.is_part_class(i.class) && Self::under(doc, root, r));
            let p = keep.and_then(|inst| part_draw(doc, inst));
            match p {
                Some(p) => {
                    self.insert(p);
                    self.changes.changed.push(r);
                }
                None => {
                    if self.remove(r) {
                        self.changes.removed.push(r);
                    }
                }
            }
        }
        for r in moved {
            if full.contains(&r) {
                continue;
            }
            let (Some(&i), Some(xf)) = (self.index.get(&r), xform_of(doc, r)) else {
                continue;
            };
            let p = &mut self.parts[i];
            p.xf = xf;
            p.draw.xf = Xform {
                pos: xf.pos + xf.rot * p.draw_offset,
                rot: xf.rot * p.draw_rot,
            };
            self.bounds[i] = bound_of(p);
            self.changes.moved.push(r);
        }
    }

    pub fn get(&self, r: Ref) -> Option<&ScenePart> {
        self.index.get(&r).map(|&i| &self.parts[i])
    }
}

fn bound_of(p: &ScenePart) -> Vec4 {
    let r = p.size.max(p.draw.size).length() * 0.5 + p.draw_offset.length();
    p.xf.pos.extend(r)
}

/// The six SurfaceTypes packed 3 bits per face (Enum.NormalId order) as
/// bands of the studs atlas: Studs 1, Weld and Glue 2, Inlet 3, Universal 4.
fn surfaces_of(doc: &Document, r: Ref) -> u32 {
    let mut out = 0;
    for (i, face) in [
        "RightSurface",
        "TopSurface",
        "BackSurface",
        "LeftSurface",
        "BottomSurface",
        "FrontSurface",
    ]
    .iter()
    .enumerate()
    {
        // Enum.SurfaceType: Smooth 0, Glue 1, Weld 2, Studs 3, Inlet 4, Universal 5.
        let band = match enum_u32(doc.prop(r, face)) {
            Some(3) => 1,
            Some(1) | Some(2) => 2,
            Some(4) => 3,
            Some(5) => 4,
            _ => 0,
        };
        out |= band << (3 * i);
    }
    out
}

pub(crate) fn part_draw(doc: &Document, inst: &Instance) -> Option<ScenePart> {
    let r = inst.referent();
    let xf = xform_of(doc, r)?;
    let size = vec3_of(doc.prop(r, "Size")).unwrap_or(Vec3::new(4.0, 1.0, 2.0));
    let rgb = color_of(doc.prop(r, "Color"))
        .or_else(|| color_of(doc.prop(r, "BrickColor")))
        .unwrap_or(mech::NEW_PART_COLOR);
    // LocalTransparencyModifier (client-side) fades the part further:
    // shown transparency = 1 - (1 - Transparency) × (1 - modifier).
    let modifier = f32_of(doc.prop(r, "LocalTransparencyModifier"))
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let transparency = 1.0
        - (1.0
            - f32_of(doc.prop(r, "Transparency"))
                .unwrap_or(0.0)
                .clamp(0.0, 1.0))
            * (1.0 - modifier);
    let mut alpha = 1.0 - transparency;
    // The light grid takes Transparency itself (the physics primitive's).
    let light_transparency = f32_of(doc.prop(r, "Transparency")).unwrap_or(0.0);
    // Reflectance mixes in the sky's reflection (fs_part, round 26).
    let reflectance = f32_of(doc.prop(r, "Reflectance")).unwrap_or(0.0).clamp(0.0, 1.0);
    let material_id = enum_u32(doc.prop(r, "Material")).unwrap_or(256);
    // Without its texture maps a material draws its table colour (squared);
    // the viewport swaps that for the maps once they're loaded.
    let albedo = crate::matpack::fallback_albedo(material_id, use_2022_materials(doc));
    let material = match material_id {
        288 => 1.0,  // Neon
        272 => 2.0,  // SmoothPlastic
        1568 => 3.0, // Glass
        1584 => {
            alpha *= 0.5; // ForceField
            3.0
        }
        _ => 0.0,
    };

    let class = inst.class.as_str();
    let mut mesh = match class {
        "WedgePart" => MeshKind::Wedge,
        "CornerWedgePart" => MeshKind::CornerWedge,
        // Part.Shape (Part and its subclasses such as SpawnLocation, Seat).
        _ if doc::is_a(class, "Part") => match enum_u32(doc.prop(r, "Shape")) {
            Some(0) => MeshKind::Ball,
            Some(2) => MeshKind::Cylinder,
            Some(3) => MeshKind::Wedge,
            Some(4) => MeshKind::CornerWedge,
            _ => MeshKind::Block,
        },
        _ => MeshKind::Block,
    };
    let mut placeholder = matches!(
        class,
        "MeshPart" | "UnionOperation" | "IntersectOperation" | "NegateOperation" | "PartOperation"
    );
    let part_class = doc::is_a(class, "Part");
    let mut draw_size = size;
    // Extra rotation of the drawn shape (vertical cylinders) and mesh Offset.
    let mut draw_rot = Mat3::IDENTITY;
    let mut draw_offset = Vec3::ZERO;
    let mut ellipsoid = false;
    let mut head_mesh = false;
    let mut deps = Vec::new();

    // A SpecialMesh child changes the drawn shape; file meshes become placeholders.
    let (sa_texture, mut decals) = surface_details(doc, inst);
    // A MaterialVariant's ColorMap covers every face, tiled every StudsPerTile.
    let variant = material_variant(doc, r, material_id);
    let has_variant = variant.is_some();
    if let Some((map, studs)) = variant {
        let tint = [
            srgb_to_linear(rgb[0]),
            srgb_to_linear(rgb[1]),
            srgb_to_linear(rgb[2]),
        ];
        // The variant is the base surface. Authored Decals/Textures must
        // follow it in the decal pass (e.g. SpawnLocation's star over Grid).
        for face in 0..6 {
            decals.insert(face as usize, DecalDesc {
                face,
                id: map.clone(),
                color: tint,
                transparency: 0.0,
                base_surface: true,
                tiling: Some([studs, studs, 0.0, 0.0]),
            });
        }
    }
    // Mesh textures cover the part outright, except on characters, where
    // transparent texels show the body Color (skin).
    let character_part = crate::assets::clothing_role(&inst.name).is_some()
        || (inst.name == "Head"
            && doc.dom.get_by_ref(inst.parent()).is_some_and(|m| {
                m.children()
                    .iter()
                    .any(|c| doc.class_of(*c).is_some_and(|k| k == "Humanoid"))
            }));
    let mesh_kind = if character_part {
        crate::render::TexMode::Over
    } else {
        crate::render::TexMode::Opaque
    };
    let mut tex_kind = sa_texture.as_ref().map(|t| t.1).unwrap_or(mesh_kind);
    let mut texture_id = sa_texture.map(|t| t.0);
    let mut texture_tint = None;
    if class == "MeshPart" && texture_id.is_none() {
        texture_id = content_string(doc.prop(r, "TextureID"))
            .or_else(|| content_string(doc.prop(r, "TextureContent")));
        tex_kind = mesh_kind;
    }
    let (clothing, tshirt) = character_clothing(doc, inst);
    if let Some(g) = tshirt {
        decals.push(DecalDesc {
            face: 5,
            id: g,
            color: [1.0, 1.0, 1.0],
            transparency: 0.0,
            base_surface: false,
            tiling: None,
        });
    }
    let mut mesh_asset = if class == "MeshPart" {
        content_string(doc.prop(r, "MeshId"))
            .or_else(|| content_string(doc.prop(r, "MeshContent")))
            .map(|id| MeshAsset {
                id,
                fit_to_part: true,
                scale: Vec3::ONE,
                offset: Vec3::ZERO,
            })
    } else if matches!(class, "UnionOperation" | "IntersectOperation" | "NegateOperation" | "PartOperation") {
        union_mesh(doc, r)
    } else {
        None
    };
    if mesh_asset.is_some() && class.ends_with("Operation") {
        placeholder = false;
        // GeometryService unions keep the main part's CFrame: their geometry
        // sits off-centre in the Size box (mesh_bvh::union_offset).
        if let Some(a) = &mesh_asset {
            draw_offset = crate::mesh_bvh::union_offset(&a.id) * size;
        }
    }
    // GeometryService's MeshParts (fragments) are off-centre the same way.
    if class == "MeshPart" {
        if let Some(a) = &mesh_asset {
            draw_offset = crate::mesh_bvh::union_offset(&a.id) * size;
        }
    }
    // CharacterMesh (R6 package bodies): the model's CharacterMesh for this
    // limb replaces its box (BodyPart Torso 1, LeftArm 2, RightArm 3,
    // LeftLeg 4, RightLeg 5), at the mesh's own size.
    if class == "Part" {
        let body_part = match inst.name.as_str() {
            "Torso" => Some(1),
            "Left Arm" => Some(2),
            "Right Arm" => Some(3),
            "Left Leg" => Some(4),
            "Right Leg" => Some(5),
            _ => None,
        };
        if let (Some(want), Some(model)) = (body_part, doc.dom.get_by_ref(inst.parent())) {
            for c in model.children() {
                if doc.class_of(*c).is_none_or(|k| k != "CharacterMesh") {
                    continue;
                }
                deps.push(*c);
                if enum_u32(doc.prop(*c, "BodyPart")) != Some(want) {
                    continue;
                }
                let id = match doc.prop(*c, "MeshId") {
                    Some(Variant::Int64(n)) if n > 0 => Some(format!("rbxassetid://{n}")),
                    _ => content_string(doc.prop(*c, "MeshContent")),
                };
                if let Some(id) = id {
                    mesh_asset = Some(MeshAsset { id, fit_to_part: false, scale: Vec3::ONE, offset: Vec3::ZERO });
                    placeholder = true;
                }
            }
        }
    }
    // Default R6 body geometry comes from Studio's shipped avatar meshes.
    // Explicit CharacterMesh and DataModelMesh children take precedence.
    // (The limb name is checked first: the Humanoid look scans the parent's
    // children, which for a plain Part in a 10k-part Folder made map loads
    // quadratic.)
    let builtin = match inst.name.as_str() {
        "Torso" => Some("torso"),
        "Left Arm" => Some("leftarm"),
        "Right Arm" => Some("rightarm"),
        "Left Leg" => Some("leftleg"),
        "Right Leg" => Some("rightleg"),
        _ => None,
    };
    if class == "Part" && mesh_asset.is_none() && builtin.is_some()
        && doc.dom.get_by_ref(inst.parent()).is_some_and(|model| {
            model.children().iter().any(|c| doc.class_of(*c).is_some_and(|k| k == "Humanoid"))
        })
        && !inst.children().iter().any(|c| {
            doc.class_of(*c).is_some_and(|k| matches!(k.as_str(), "SpecialMesh" | "BlockMesh" | "CylinderMesh"))
        })
    {
        if let Some(id) = builtin.and_then(|name| studio_avatar_mesh(&format!("meshes/{name}.mesh"))) {
            mesh_asset = Some(MeshAsset {
                id,
                fit_to_part: true,
                scale: Vec3::ONE,
                offset: Vec3::ZERO,
            });
            placeholder = true;
        }
    }
    // A vertical (Y-axis) cylinder: the X-axis cylinder turned onto Y.
    let vertical = |s: Vec3| {
        (
            Mat3::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(s.y, s.x, s.z),
        )
    };
    // The light grid's view of the mesh children (FUN_146654ff0): the
    // SpecialMesh type, or a BlockMesh / CylinderMesh.
    let mut light_special: Option<u32> = None;
    let mut light_legacy: Option<bool> = None;
    for c in inst.children() {
        let Some(child) = doc.dom.get_by_ref(*c) else {
            continue;
        };
        if !(class == "MeshPart" || class.ends_with("Operation")) {
            match child.class.as_str() {
                "SpecialMesh" => light_special = Some(enum_u32(doc.prop(*c, "MeshType")).unwrap_or(5)),
                "BlockMesh" => light_legacy = Some(false),
                "CylinderMesh" => light_legacy = Some(true),
                _ => {}
            }
        }
        if matches!(
            child.class.as_str(),
            "Decal"
                | "Texture"
                | "SurfaceAppearance"
                | "SpecialMesh"
                | "BlockMesh"
                | "CylinderMesh"
        ) {
            deps.push(*c);
        }
        // A MeshPart (or union) keeps its own shape: Roblox ignores
        // SpecialMesh / BlockMesh / CylinderMesh under it (Place B has a
        // MeshPart whose empty SpecialMesh hid its mesh).
        if class == "MeshPart" || class.ends_with("Operation") {
            continue;
        }
        if matches!(child.class.as_str(), "BlockMesh" | "CylinderMesh") {
            // Legacy meshes: stretch with Scale, shift by Offset.
            let scale = vec3_of(doc.prop(*c, "Scale")).unwrap_or(Vec3::ONE);
            draw_offset = vec3_of(doc.prop(*c, "Offset")).unwrap_or(Vec3::ZERO);
            draw_size = size * scale;
            if child.class == "CylinderMesh" {
                (draw_rot, draw_size) = vertical(draw_size);
                mesh = MeshKind::Cylinder;
                ellipsoid = true;
            } else {
                mesh = MeshKind::Block;
            }
        } else if child.class == "SpecialMesh" {
            let scale = vec3_of(doc.prop(*c, "Scale")).unwrap_or(Vec3::ONE);
            let mesh_type = enum_u32(doc.prop(*c, "MeshType")).unwrap_or(5);
            if mesh_type != 5 {
                draw_offset = vec3_of(doc.prop(*c, "Offset")).unwrap_or(Vec3::ZERO);
                draw_size = size * scale;
            }
            match mesh_type {
                // Head: Studio's own head mesh (content/avatar/heads/head.mesh),
                // round across the part's smaller horizontal side.
                0 => {
                    head_mesh = true;
                    let m = size.x.min(size.z);
                    draw_size = Vec3::new(m, size.y, m) * scale;
                    if let Some(head) = studio_avatar_mesh("heads/head.mesh") {
                        mesh_asset = Some(MeshAsset {
                            id: head,
                            fit_to_part: true,
                            scale: Vec3::ONE,
                            offset: Vec3::ZERO,
                        });
                        placeholder = true;
                    } else {
                        (draw_rot, draw_size) = vertical(draw_size);
                        mesh = MeshKind::Cylinder;
                        ellipsoid = true;
                    }
                }
                1 | 6 => mesh = MeshKind::Block,
                2 => mesh = MeshKind::Wedge,
                3 => {
                    mesh = MeshKind::Ball;
                    ellipsoid = true;
                }
                4 => {
                    mesh = MeshKind::Cylinder;
                    ellipsoid = true;
                }
                11 => mesh = MeshKind::CornerWedge,
                5 => {
                    placeholder = true;
                    if texture_id.is_none() {
                        texture_id = content_string(doc.prop(*c, "TextureId"))
                            .or_else(|| content_string(doc.prop(*c, "TextureContent")));
                        tex_kind = mesh_kind;
                        if texture_id.is_some() && mesh_kind == crate::render::TexMode::Opaque {
                            // DefaultUnifiedFlatOpaqueFS multiplies diffuse texels by
                            // vertex colour before squaring into linear light.
                            let tint = vec3_of(doc.prop(*c, "VertexColor")).unwrap_or(Vec3::ONE);
                            texture_tint = Some(tint * tint);
                            tex_kind = crate::render::TexMode::OpaqueTinted;
                        }
                    }
                    mesh_asset = content_string(doc.prop(*c, "MeshId"))
                        .or_else(|| content_string(doc.prop(*c, "MeshContent")))
                        .map(|id| MeshAsset {
                            id,
                            fit_to_part: false,
                            scale: vec3_of(doc.prop(*c, "Scale")).unwrap_or(Vec3::ONE),
                            offset: vec3_of(doc.prop(*c, "Offset")).unwrap_or(Vec3::ZERO),
                        });
                }
                // Prism, Pyramid, ramps: no shape of their own yet.
                _ => mesh = MeshKind::Block,
            }
        }
    }
    if crate::assets::clothing_role(&inst.name).is_some() {
        if let Some(model) = doc.dom.get_by_ref(inst.parent()) {
            deps.extend(model.children().iter().copied().filter(|c| {
                doc.class_of(*c)
                    .is_some_and(|k| matches!(k.as_str(), "Shirt" | "Pants" | "ShirtGraphic"))
            }));
        }
    }
    if class == "NegateOperation" {
        alpha = alpha.min(0.5);
    }
    // Part balls and cylinders keep round ends (the shortest side wins);
    // mesh ellipsoids stretch to the size.
    match mesh {
        MeshKind::Ball if !ellipsoid => draw_size = Vec3::splat(draw_size.min_element()),
        MeshKind::Cylinder if !ellipsoid => {
            let m = draw_size.y.min(draw_size.z);
            draw_size = Vec3::new(draw_size.x, m, m);
        }
        _ => {}
    }
    if placeholder {
        mesh = MeshKind::Block;
    }
    let cast_shadow = !matches!(doc.prop(r, "CastShadow"), Some(Variant::Bool(false)));
    // MeshParts and unions: Studio fills a blob per convex piece of their
    // physics (record type 9); the viewport supplies cached convex-piece
    // bounds, with the part box as a fallback (lightgrid.rs Kind::Mesh).
    let mesh_like = class == "MeshPart" || class.ends_with("Operation");
    let light = (cast_shadow && light_transparency < 1.0 && class != "NegateOperation")
        .then(|| {
            if mesh_like {
                return Some(crate::lightgrid::Kind::Mesh);
            }
            let shape = match class {
                "WedgePart" => MeshKind::Wedge,
                "CornerWedgePart" => MeshKind::CornerWedge,
                _ if part_class => match enum_u32(doc.prop(r, "Shape")) {
                    Some(0) => MeshKind::Ball,
                    Some(2) => MeshKind::Cylinder,
                    Some(3) => MeshKind::Wedge,
                    Some(4) => MeshKind::CornerWedge,
                    _ => MeshKind::Block,
                },
                _ => MeshKind::Block,
            };
            crate::lightgrid::kind_of(light_special, light_legacy, shape)
        })
        .flatten()
        // A truss lets most light through (geometry 3: Transparency × 0.25 + 0.75).
        .map(|kind| {
            let t = if class == "TrussPart" { light_transparency * 0.25 + 0.75 } else { light_transparency };
            crate::lightgrid::Prim::new(kind, size, &xf, t)
        });
    let locked = matches!(doc.prop(r, "Locked"), Some(Variant::Bool(true)));
    // Only the Plastic shader samples StudsMapTexture. SmoothPlastic and
    // textured material shaders do not (DefaultUnifiedPlasticFS vs Flat/Detiled).
    // This applies to both built-in material tables.
    let surfaces = if part_class
        && mesh == MeshKind::Block
        && material_id == 256
        && material_variant(doc, r, material_id).is_none()
        && !placeholder
        && mesh_asset.is_none()
        && draw_size == size
    {
        surfaces_of(doc, r)
    } else {
        0
    };
    Some(ScenePart {
        r,
        xf,
        size,
        mesh_asset,
        texture_id,
        tex_kind,
        emissive: surface_emissive(doc, inst),
        decals,
        clothing,
        locked,
        variant: has_variant,
        decal_size: if head_mesh { draw_size } else { size },
        // A skinned MeshPart redraws when a Bone or its FaceControls changes (skin.rs).
        deps: if class == "MeshPart" {
            let mut deps = deps;
            deps.extend(crate::skin::deps_of(doc, r));
            deps
        } else {
            deps
        },
        draw_offset,
        draw_rot,
        light,
        draw: PartDraw {
            xf: Xform {
                pos: xf.pos + xf.rot * draw_offset,
                rot: xf.rot * draw_rot,
            },
            size: draw_size,
            // Textured meshes: Studio's mesh shaders square texel × vertex /
            // instance colour only; the per-Material darkening (measured on
            // untextured cubes) doesn't apply (Map 2 in an offline Studio: a
            // CorrodedMetal MeshPart reads exactly mean(texel²), 2026-11-08).
            color: texture_tint.map(|t| t.to_array()).unwrap_or([
                srgb_to_linear(rgb[0]) * albedo[0],
                srgb_to_linear(rgb[1]) * albedo[1],
                srgb_to_linear(rgb[2]) * albedo[2],
            ]),
            alpha,
            mesh,
            material,
            placeholder,
            hover: false,
            reflectance,
            custom_mesh: None,
            texture: None,
            tex_mode: crate::render::TexMode::None,
            uvt: PartDraw::NO_UVT,
            emissive: [0.0; 3],
            tex_albedo: 1.0,
            surfaces,
            material_id,
            mat_layer: -1,
            cast_shadow,
        },
    })
}

/// The ground grid, drawn just above the baseplate top.
pub fn grid_lines(out: &mut Vec<LineVertex>, step: f32) {
    let ext = mech::GRID_EXTENT;
    let step = if step > 0.0 { step } else { mech::GRID_SPACING };
    let n = (ext / step) as i32;
    let y = 0.1;
    for i in -n..=n {
        let t = i as f32 * step;
        let major = i % 4 == 0;
        let c = if major {
            Vec4::new(1.0, 1.0, 1.0, 0.32)
        } else {
            Vec4::new(1.0, 1.0, 1.0, 0.14)
        };
        for (a, b) in [
            (Vec3::new(t, y, -ext), Vec3::new(t, y, ext)),
            (Vec3::new(-ext, y, t), Vec3::new(ext, y, t)),
        ] {
            out.push(LineVertex {
                pos: a.to_array(),
                color: c.to_array(),
            });
            out.push(LineVertex {
                pos: b.to_array(),
                color: c.to_array(),
            });
        }
    }
}

/// BrickColor lookup for the properties panel swatch.
pub fn brick_rgb(b: BrickColor) -> [u8; 3] {
    let c = b.to_color3uint8();
    [c.r, c.g, c.b]
}

#[cfg(test)]
mod tint_tests {
    use super::*;
    use rbx_dom_weak::{InstanceBuilder, WeakDom};

    #[test]
    fn file_mesh_texture_uses_vertex_color() {
        let mesh = InstanceBuilder::new("SpecialMesh")
            .with_property("MeshType", rbx_dom_weak::types::Enum::from_u32(5))
            .with_property("MeshId", rbx_dom_weak::types::ContentId::from("rbxassetid://1"))
            .with_property("TextureId", rbx_dom_weak::types::ContentId::from("rbxassetid://2"))
            .with_property("VertexColor", rbx_dom_weak::types::Vector3::new(0.5, 1.0, 0.25));
        let part = InstanceBuilder::new("Part").with_child(mesh);
        let r = part.referent();
        let doc = Document::from_play_dom(WeakDom::new(
            InstanceBuilder::new("DataModel").with_child(part)), "tint".into());
        let p = part_draw(&doc, doc.dom.get_by_ref(r).unwrap()).unwrap();
        assert_eq!(p.tex_kind, crate::render::TexMode::OpaqueTinted);
        assert!((p.draw.color[0] / p.draw.color[1] - 0.25).abs() < 1e-6);
        assert!((p.draw.color[2] / p.draw.color[1] - 0.0625).abs() < 1e-6);
    }
}

/// A mesh of the Studio install's content/avatar (the classic head, R6
/// limbs) as an engine-generated id: the engine picks these files, not the
/// place, so they still draw while scripts run, when local paths aren't
/// place content (assets::place_content_allowed). Read once per file.
fn studio_avatar_mesh(rel: &str) -> Option<String> {
    static IDS: std::sync::Mutex<Option<std::collections::HashMap<String, Option<String>>>> =
        std::sync::Mutex::new(None);
    let mut g = IDS.lock().ok()?;
    g.get_or_insert_with(Default::default)
        .entry(rel.to_string())
        .or_insert_with(|| {
            let bytes = std::fs::read(crate::default_scripts::studio_dir()?.join("content/avatar").join(rel)).ok()?;
            let id = format!("rbxtemp://rustudio/avatar/{rel}");
            crate::assets::register_generated(&id, bytes);
            Some(id)
        })
        .clone()
}
