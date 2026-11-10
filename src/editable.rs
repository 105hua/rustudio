//! EditableMesh / EditableImage and the Content datatype.
//!
//! The two Editable* objects are unparented DOM instances (as before) whose
//! data lives here, keyed by their Ref, so the renderer can reach it without
//! the Lua state:
//!
//! - an EditableImage is RGBA8 pixels plus a generation that every write
//!   bumps. A `Content` pointing at it resolves (doc.rs `prop`, scene.rs
//!   `content_string`) to `rbxtemp://rustudio/editable/img/<n>@<gen>`; the
//!   texture caches (assets.rs, gui.rs) read the pixels straight from here
//!   and re-upload in place when the generation moves on.
//! - an EditableMesh keeps vertices, normals, UVs, colours and faces under
//!   typed ids (`index * 8 + kind`, so the Batch API can tell them apart).
//!   AssetService:CreateMeshPartAsync snapshots it as a binary 2.00 mesh
//!   registered under `rbxtemp://rustudio/editable/mesh/<n>@<gen>`, so the
//!   MeshPart goes through the ordinary mesh path (drawing, ray tests,
//!   collision).
//!
//! The `Content` global and the `Content` userdata (Content.none, fromUri,
//! fromAssetId, fromObject; SourceType / Uri / Object) are here too.

use std::sync::{Arc, Mutex};

use glam::Vec2;
use rbx_dom_weak::types::ContentType;

use super::*;

// ------------------------------------------------------------------ store

pub struct Image {
    pub n: u64,
    pub w: u32,
    pub h: u32,
    pub px: Arc<Vec<u8>>,
    pub gen: u64,
    /// Instances / properties that showed this image (doc.rs `prop`): marked
    /// changed when its pixels change so parts redraw.
    users: HashSet<(Ref, rbx_dom_weak::Ustr)>,
}

#[path = "editable_mesh.rs"]
pub mod em;
#[path = "editable_project.rs"]
mod project;
pub use em::Mesh;

type SharedMesh = Arc<Mutex<Mesh>>;

fn lock(m: &SharedMesh) -> std::sync::MutexGuard<'_, Mesh> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct Store {
    images: HashMap<Ref, Image>,
    meshes: HashMap<Ref, SharedMesh>,
    next: u64,
    /// Image number → its object (renderer lookups by id).
    image_by_n: HashMap<u64, Ref>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Store) -> R) -> R {
    let mut g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Store::default))
}

const IMG_PREFIX: &str = "rbxtemp://rustudio/editable/img/";
const MESH_PREFIX: &str = "rbxtemp://rustudio/editable/";
/// Content ids of engine-made images Roblox shows as ContentSourceType.Opaque.
pub const OPAQUE_PREFIX: &str = "rbxtemp://rustudio/opaque/";

/// The id a Content pointing at `obj` shows as (an EditableImage's current
/// pixels, an EditableMesh's last MeshPart snapshot); `user` (instance,
/// property) is remembered so a later write redraws it.
pub fn content_uri(obj: Ref, user: Option<(Ref, &str)>) -> Option<String> {
    with(|s| {
        if let Some(img) = s.images.get_mut(&obj) {
            if let Some((r, p)) = user {
                img.users.insert((r, rbx_dom_weak::ustr(p)));
            }
            return Some(format!("{IMG_PREFIX}{}@{}", img.n, img.gen));
        }
        let m = lock(s.meshes.get(&obj)?);
        (m.gen > 0).then(|| format!("{MESH_PREFIX}mesh/{}@{}", m.n, m.gen))
    })
}

/// For a texture cache: `rbxtemp://rustudio/editable/img/<n>@<gen>` → the
/// key without the generation and the generation.
pub fn image_key(id: &str) -> Option<(&str, u64)> {
    let rest = id.strip_prefix(IMG_PREFIX)?;
    let at = rest.find('@')?;
    let gen = rest[at + 1..].parse().ok()?;
    Some((&id[..IMG_PREFIX.len() + at], gen))
}

/// A cache key with every editable image generation taken out (composites
/// name several ids), or None when it names none.
pub fn strip_generations(key: &str) -> Option<String> {
    if !key.contains(IMG_PREFIX) {
        return None;
    }
    let mut out = String::with_capacity(key.len());
    let mut rest = key;
    while let Some(i) = rest.find(IMG_PREFIX) {
        let after = i + IMG_PREFIX.len();
        out.push_str(&rest[..after]);
        let tail = &rest[after..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        out.push_str(&tail[..digits]);
        let mut t = &tail[digits..];
        if let Some(g) = t.strip_prefix('@') {
            let gd = g.bytes().take_while(u8::is_ascii_digit).count();
            t = &g[gd..];
        }
        rest = t;
    }
    out.push_str(rest);
    Some(out)
}

/// The pixels of an editable image id (any generation: the current ones).
pub fn image_pixels(id: &str) -> Option<(u64, crate::texture::Rgba)> {
    let rest = id.strip_prefix(IMG_PREFIX)?;
    let n: u64 = rest.split('@').next()?.parse().ok()?;
    with(|s| {
        let r = *s.image_by_n.get(&n)?;
        let img = s.images.get(&r)?;
        Some((
            img.gen,
            crate::texture::Rgba { width: img.w.max(1), height: img.h.max(1), pixels: (*img.px).clone() },
        ))
    })
}

fn is_editable_image(r: Ref) -> bool {
    with(|s| s.images.contains_key(&r))
}

fn is_editable_mesh(r: Ref) -> bool {
    with(|s| s.meshes.contains_key(&r))
}

fn new_image(w: u32, h: u32, px: Vec<u8>) -> Image {
    Image { n: 0, w, h, px: Arc::new(px), gen: 1, users: HashSet::new() }
}

// ------------------------------------------------------------------ Content

/// The Content datatype (typeof "Content").
#[derive(Clone, Debug, PartialEq)]
pub struct LContent(pub Content);

impl LContent {
    pub fn variant(&self) -> Variant {
        Variant::Content(self.0.clone())
    }
}

impl UserData for LContent {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_meta_field("__type", "Content");
        f.add_field_method_get("SourceType", |_, this| {
            let v = match this.0.value() {
                ContentType::None => 0,
                // Engine-made images (VideoSampler frames) are Opaque in Roblox.
                ContentType::Uri(u) if u.starts_with(OPAQUE_PREFIX) => 3,
                ContentType::Uri(_) => 1,
                ContentType::Object(_) => 2,
                _ => 3,
            };
            Ok(LEnumItem::from_value("ContentSourceType", v).unwrap_or(LEnumItem {
                enum_type: "ContentSourceType".into(),
                name: ["None", "Uri", "Object", "Opaque"][v as usize].into(),
                value: v,
            }))
        });
        f.add_field_method_get("Uri", |lua, this| match this.0.as_uri() {
            Some(u) if u.starts_with(OPAQUE_PREFIX) => Ok(Value::Nil),
            Some(u) => Ok(Value::String(lua.create_string(u)?)),
            None => Ok(Value::Nil),
        });
        f.add_field_method_get("Object", |lua, this| match this.0.as_object() {
            Some(r) => inst_value(lua, r),
            None => Ok(Value::Nil),
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            // Studio: Content{SourceType=Uri, Uri=rbxassetid://123}.
            Ok(match this.0.value() {
                ContentType::Uri(u) => format!("Content{{SourceType=Uri, Uri={u}}}"),
                ContentType::Object(_) => "Content{SourceType=Object}".to_string(),
                _ => "Content{SourceType=None}".to_string(),
            })
        });
        m.add_meta_function(MetaMethod::Eq, |_, (a, b): (Value, Value)| {
            Ok(match (opt_ud::<LContent>(&a), opt_ud::<LContent>(&b)) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            })
        });
    }
}

/// A Lua value as a Content (a Content, or a URI string as older code passes).
pub fn content_of(v: &Value) -> Option<Content> {
    match v {
        Value::UserData(_) => opt_ud::<LContent>(v).map(|c| c.0),
        Value::String(s) => Some(Content::from(s.to_str().ok()?.to_string())),
        Value::Nil => Some(Content::none()),
        _ => None,
    }
}

pub fn content_value(lua: &Lua, c: &Content) -> mlua::Result<Value> {
    Ok(Value::UserData(lua.create_userdata(LContent(c.clone()))?))
}

/// Puts the `Content` global in place (over library.luau's string version).
pub fn install(lua: &Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.set("none", LContent(Content::none()))?;
    t.set(
        "fromUri",
        lua.create_function(|_, uri: Value| match uri {
            Value::String(s) => Ok(LContent(Content::from(s.to_str()?.to_string()))),
            v => Err(rt_err(format!("Content.fromUri: string expected, got {}", v.type_name()))),
        })?,
    )?;
    t.set(
        "fromAssetId",
        lua.create_function(|_, id: Value| {
            let n = num(&id).ok_or_else(|| rt_err("Content.fromAssetId: number expected"))?;
            Ok(LContent(Content::from(format!("rbxassetid://{}", n as u64))))
        })?,
    )?;
    t.set(
        "fromObject",
        lua.create_function(|_, obj: Value| {
            let r = opt_inst(&obj).filter(|r| is_editable_image(*r) || is_editable_mesh(*r)).ok_or_else(|| {
                rt_err("Content.fromObject: EditableImage or EditableMesh expected")
            })?;
            Ok(LContent(Content::from_referent(r)))
        })?,
    )?;
    lua.globals().set("Content", t)?;
    Ok(())
}

// ------------------------------------------------------------------ methods

pub const METHODS: &[(&str, &str)] = &[
    ("CreateEditableMesh", "AssetService"),
    ("CreateEditableImage", "AssetService"),
    ("CreateEditableMeshAsync", "AssetService"),
    ("CreateEditableImageAsync", "AssetService"),
    ("CreateSurfaceAppearanceAsync", "AssetService"),
    // EditableImage
    ("Destroy", "EditableImage"),
    ("ReadPixelsBuffer", "EditableImage"),
    ("WritePixelsBuffer", "EditableImage"),
    ("DrawRectangle", "EditableImage"),
    ("DrawCircle", "EditableImage"),
    ("DrawLine", "EditableImage"),
    ("DrawTriangle", "EditableImage"),
    ("DrawImage", "EditableImage"),
    ("DrawImageTransformed", "EditableImage"),
    ("DrawImageProjected", "EditableImage"),
    ("SampleImageProjected", "EditableImage"),
    // EditableMesh (editable_mesh.rs, same list as em::METHODS)
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

fn names() -> &'static HashSet<&'static str> {
    static N: std::sync::OnceLock<HashSet<&'static str>> = std::sync::OnceLock::new();
    N.get_or_init(|| METHODS.iter().map(|(m, _)| *m).collect())
}

/// EditableImage.Size / EditableMesh.FixedSize.
pub fn prop_get(lua: &Lua, r: Ref, class: &str, key: &str) -> mlua::Result<Option<Value>> {
    match (class, key) {
        ("EditableImage", "Size") => {
            let s = with(|s| s.images.get(&r).map(|i| (i.w, i.h)));
            Ok(s.map(|(w, h)| LVector2(Vec2::new(w as f32, h as f32))).map(|v| lua.create_userdata(v)).transpose()?.map(Value::UserData))
        }
        ("EditableMesh", "FixedSize") => {
            // Any EditableMesh (one not made through AssetService reads false).
            Ok(Some(Value::Boolean(with(|s| s.meshes.get(&r).map(|m| lock(m).fixed)).unwrap_or(false))))
        }
        _ => Ok(None),
    }
}

fn one(v: Value) -> mlua::Result<MultiValue> {
    Ok(MultiValue::from_iter([v]))
}

fn none() -> mlua::Result<MultiValue> {
    Ok(MultiValue::new())
}


fn v2(v: &Value) -> Option<Vec2> {
    opt_ud::<LVector2>(v).map(|v| v.0)
}

fn field(v: &Value, key: &str) -> Value {
    match v {
        Value::Table(t) => t.get::<Value>(key).unwrap_or(Value::Nil),
        _ => Value::Nil,
    }
}

fn enum_val(v: &Value) -> Option<u32> {
    opt_ud::<LEnumItem>(v).map(|e| e.value)
}





/// The image's users marked changed (they redraw with the new pixels).
fn touched(lua: &Lua, r: Ref) -> mlua::Result<()> {
    let users: Vec<(Ref, rbx_dom_weak::Ustr)> = with(|s| {
        s.images.get_mut(&r).map(|i| {
            i.gen += 1;
            i.users.iter().copied().collect()
        })
    })
    .unwrap_or_default();
    if users.is_empty() {
        return Ok(());
    }
    let mut w = world(lua)?;
    let d = w.dm();
    for (u, p) in users {
        if d.exists(u) {
            d.mark_prop(u, p);
        }
    }
    Ok(())
}

/// The editable-object and AssetService calls; None when `name` isn't one.
pub fn call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> Option<mlua::Result<MultiValue>> {
    if !names().contains(name) {
        return None;
    }
    let class = world(lua).ok()?.d().class_of(this)?;
    match class.as_str() {
        "AssetService" => Some(asset_call(lua, name, args)),
        "EditableImage" => Some(image_call(lua, name, this, args)),
        "EditableMesh" => Some(mesh_call(lua, name, this, args)),
        _ => None,
    }
}

fn new_object(lua: &Lua, class: &str) -> mlua::Result<Ref> {
    let r = ext::create_any(lua, class)?;
    if let Some(i) = world(lua)?.dm().dom.get_by_ref_mut(r) {
        i.name = class.to_string();
    }
    Ok(r)
}

fn add_image(lua: &Lua, img: Image) -> mlua::Result<Value> {
    let r = new_object(lua, "EditableImage")?;
    with(|s| {
        s.next += 1;
        let n = s.next;
        s.image_by_n.insert(n, r);
        s.images.insert(r, Image { n, ..img });
    });
    inst_value(lua, r)
}

/// Pixels shown by something other than an EditableImage (a playing
/// video's current frame, media domain), stored under `key` with a new
/// generation per call so the texture caches re-upload; returns its id.
pub fn external_image(key: Ref, w: u32, h: u32, px: Vec<u8>) -> String {
    with(|s| {
        if let Some(img) = s.images.get_mut(&key) {
            img.w = w;
            img.h = h;
            img.px = Arc::new(px);
            img.gen += 1;
        } else {
            s.next += 1;
            let n = s.next;
            s.image_by_n.insert(n, key);
            s.images.insert(key, Image { n, ..new_image(w, h, px) });
        }
        let img = &s.images[&key];
        format!("{IMG_PREFIX}{}@{}", img.n, img.gen)
    })
}

/// Forgets an [`external_image`].
pub fn drop_external_image(key: Ref) {
    with(|s| {
        if let Some(img) = s.images.remove(&key) {
            s.image_by_n.remove(&img.n);
        }
    })
}

fn add_mesh(lua: &Lua, m: Mesh) -> mlua::Result<Value> {
    let r = new_object(lua, "EditableMesh")?;
    with(|s| {
        s.next += 1;
        let n = s.next;
        let mut m = m;
        m.n = n;
        m.gen = 0;
        s.meshes.insert(r, Arc::new(Mutex::new(m)));
    });
    inst_value(lua, r)
}

fn content_arg(args: &MultiValue, i: usize, what: &str) -> mlua::Result<Content> {
    content_of(&arg(args, i)).ok_or_else(|| rt_err(format!("{what}: Content expected")))
}

fn asset_call(lua: &Lua, name: &str, args: &MultiValue) -> mlua::Result<MultiValue> {
    match name {
        "CreateEditableMesh" => {
            let fixed = field(&arg(args, 1), "FixedSize") == Value::Boolean(true);
            one(add_mesh(lua, { let mut m = Mesh::default(); m.fixed = fixed; m })?)
        }
        "CreateEditableImage" => {
            let size = v2(&field(&arg(args, 1), "Size")).unwrap_or(Vec2::splat(512.0));
            if !(size.x >= 1.0 && size.y >= 1.0 && size.x <= 1024.0 && size.y <= 1024.0) {
                return Err(rt_err("CreateEditableImage: Size must be between 1 and 1024"));
            }
            let (w, h) = (size.x as u32, size.y as u32);
            one(add_image(lua, new_image(w, h, vec![0; (w * h * 4) as usize]))?)
        }
        "CreateEditableMeshAsync" => {
            let c = content_arg(args, 1, name)?;
            let fixed = field(&arg(args, 2), "FixedSize") == Value::Boolean(true);
            let mut m = match c.value() {
                ContentType::Object(r) => with(|s| s.meshes.get(r).map(|m| lock(m).clone()))
                    .ok_or_else(|| rt_err("CreateEditableMeshAsync: the object is not an EditableMesh"))?,
                ContentType::Uri(u) => mesh_from_uri(u)?,
                _ => return Err(rt_err("CreateEditableMeshAsync: empty Content")),
            };
            m.fixed = fixed;
            one(add_mesh(lua, m)?)
        }
        "CreateEditableImageAsync" => {
            let c = content_arg(args, 1, name)?;
            let img = match c.value() {
                ContentType::Object(r) => with(|s| s.images.get(r).map(|i| new_image(i.w, i.h, (*i.px).clone())))
                    .ok_or_else(|| rt_err("CreateEditableImageAsync: the object is not an EditableImage"))?,
                ContentType::Uri(u) => {
                    let bytes = crate::assets::script_content_blocking(u).map_err(|e| rt_err(format!("CreateEditableImageAsync: {e}")))?;
                    let d = crate::texture::decode(&bytes).map_err(|e| rt_err(format!("CreateEditableImageAsync: {e}")))?;
                    new_image(d.width, d.height, d.pixels)
                }
                _ => return Err(rt_err("CreateEditableImageAsync: empty Content")),
            };
            one(add_image(lua, img)?)
        }
        "CreateSurfaceAppearanceAsync" => {
            let maps = arg(args, 1);
            let r = ext::create_any(lua, "SurfaceAppearance")?;
            for key in ["ColorMap", "NormalMap", "RoughnessMap", "MetalnessMap", "EmissiveMask"] {
                if let Some(c) = content_of(&field(&maps, key)).filter(|c| !matches!(c.value(), ContentType::None)) {
                    store_prop(lua, r, &format!("{key}Content"), Variant::Content(c))?;
                }
            }
            one(inst_value(lua, r)?)
        }
        _ => none(),
    }
}

// ------------------------------------------------------------------ images

fn rgba_of(color: &Value, transparency: &Value) -> [f32; 4] {
    let c = opt_ud::<LColor3>(color).map(|c| c.0).unwrap_or([1.0; 3]);
    let a = 1.0 - num(transparency).unwrap_or(0.0).clamp(0.0, 1.0) as f32;
    [c[0], c[1], c[2], a]
}

/// ImageCombineType: BlendSourceOver 1 (default), Overwrite 2, Add 3,
/// Multiply 4, AlphaBlend 5, Subtract 7.
fn blend(dst: &mut [u8], src: [f32; 4], mode: u32) {
    let d = [dst[0] as f32 / 255.0, dst[1] as f32 / 255.0, dst[2] as f32 / 255.0, dst[3] as f32 / 255.0];
    let a = src[3];
    let out = match mode {
        2 => src,
        3 => [d[0] + src[0] * a, d[1] + src[1] * a, d[2] + src[2] * a, d[3]],
        4 => [
            d[0] * (1.0 - a + src[0] * a),
            d[1] * (1.0 - a + src[1] * a),
            d[2] * (1.0 - a + src[2] * a),
            d[3],
        ],
        7 => [d[0] - src[0] * a, d[1] - src[1] * a, d[2] - src[2] * a, d[3]],
        _ => {
            let oa = a + d[3] * (1.0 - a);
            if oa <= 1e-6 {
                [0.0; 4]
            } else {
                let f = |s: f32, t: f32| (s * a + t * d[3] * (1.0 - a)) / oa;
                [f(src[0], d[0]), f(src[1], d[1]), f(src[2], d[2]), oa]
            }
        }
    };
    for k in 0..4 {
        dst[k] = (out[k].clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

fn edit_pixels<R>(r: Ref, f: impl FnOnce(u32, u32, &mut Vec<u8>) -> R) -> mlua::Result<R> {
    with(|s| {
        let img = s.images.get_mut(&r).ok_or_else(|| rt_err("the EditableImage was destroyed"))?;
        let (w, h) = (img.w, img.h);
        Ok(f(w, h, Arc::make_mut(&mut img.px)))
    })
}

fn combine_arg(v: &Value) -> u32 {
    enum_val(v).unwrap_or(1)
}

fn image_call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    match name {
        "Destroy" => {
            with(|s| {
                if let Some(i) = s.images.remove(&this) {
                    s.image_by_n.remove(&i.n);
                }
            });
            none()
        }
        "ReadPixelsBuffer" => {
            let pos = v2(&arg(args, 1)).ok_or_else(|| rt_err("ReadPixelsBuffer: Vector2 position expected"))?;
            let size = v2(&arg(args, 2)).ok_or_else(|| rt_err("ReadPixelsBuffer: Vector2 size expected"))?;
            let out = with(|s| {
                let img = s.images.get(&this).ok_or_else(|| rt_err("the EditableImage was destroyed"))?;
                let (x0, y0, w, h) = (pos.x as i64, pos.y as i64, size.x as i64, size.y as i64);
                if w < 1 || h < 1 || x0 < 0 || y0 < 0 || x0 + w > img.w as i64 || y0 + h > img.h as i64 {
                    return Err(rt_err("ReadPixelsBuffer: the region is outside the image"));
                }
                let mut out = Vec::with_capacity((w * h * 4) as usize);
                for y in y0..y0 + h {
                    let o = ((y * img.w as i64 + x0) * 4) as usize;
                    out.extend_from_slice(&img.px[o..o + (w * 4) as usize]);
                }
                Ok(out)
            })?;
            one(Value::Buffer(lua.create_buffer(out)?))
        }
        "WritePixelsBuffer" => {
            let pos = v2(&arg(args, 1)).ok_or_else(|| rt_err("WritePixelsBuffer: Vector2 position expected"))?;
            let size = v2(&arg(args, 2)).ok_or_else(|| rt_err("WritePixelsBuffer: Vector2 size expected"))?;
            let Value::Buffer(b) = arg(args, 3) else {
                return Err(rt_err("WritePixelsBuffer: buffer expected"));
            };
            let data = b.to_vec();
            edit_pixels(this, |iw, ih, px| {
                let (x0, y0, w, h) = (pos.x as i64, pos.y as i64, size.x as i64, size.y as i64);
                if w < 1 || h < 1 || x0 < 0 || y0 < 0 || x0 + w > iw as i64 || y0 + h > ih as i64 {
                    return Err(rt_err("WritePixelsBuffer: the region is outside the image"));
                }
                if data.len() < (w * h * 4) as usize {
                    return Err(rt_err("WritePixelsBuffer: the buffer is smaller than the region"));
                }
                for y in 0..h {
                    let o = (((y0 + y) * iw as i64 + x0) * 4) as usize;
                    let s = (y * w * 4) as usize;
                    px[o..o + (w * 4) as usize].copy_from_slice(&data[s..s + (w * 4) as usize]);
                }
                Ok(())
            })??;
            touched(lua, this)?;
            none()
        }
        "DrawRectangle" => {
            let pos = v2(&arg(args, 1)).unwrap_or(Vec2::ZERO);
            let size = v2(&arg(args, 2)).unwrap_or(Vec2::ZERO);
            let src = rgba_of(&arg(args, 3), &arg(args, 4));
            let mode = combine_arg(&arg(args, 5));
            edit_pixels(this, |w, h, px| {
                let x0 = (pos.x.round() as i64).clamp(0, w as i64);
                let y0 = (pos.y.round() as i64).clamp(0, h as i64);
                let x1 = ((pos.x + size.x).round() as i64).clamp(0, w as i64);
                let y1 = ((pos.y + size.y).round() as i64).clamp(0, h as i64);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let o = ((y * w as i64 + x) * 4) as usize;
                        blend(&mut px[o..o + 4], src, mode);
                    }
                }
            })?;
            touched(lua, this)?;
            none()
        }
        "DrawCircle" => {
            let c = v2(&arg(args, 1)).unwrap_or(Vec2::ZERO);
            let rad = num(&arg(args, 2)).unwrap_or(0.0) as f32;
            let src = rgba_of(&arg(args, 3), &arg(args, 4));
            let mode = combine_arg(&arg(args, 5));
            edit_pixels(this, |w, h, px| {
                let x0 = ((c.x - rad).floor() as i64).max(0);
                let y0 = ((c.y - rad).floor() as i64).max(0);
                let x1 = ((c.x + rad).ceil() as i64).min(w as i64 - 1);
                let y1 = ((c.y + rad).ceil() as i64).min(h as i64 - 1);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - c;
                        if d.length_squared() <= rad * rad {
                            let o = ((y * w as i64 + x) * 4) as usize;
                            blend(&mut px[o..o + 4], src, mode);
                        }
                    }
                }
            })?;
            touched(lua, this)?;
            none()
        }
        "DrawLine" => {
            let p0 = v2(&arg(args, 1)).unwrap_or(Vec2::ZERO);
            let p1 = v2(&arg(args, 2)).unwrap_or(Vec2::ZERO);
            let src = rgba_of(&arg(args, 3), &arg(args, 4));
            let mode = combine_arg(&arg(args, 5));
            edit_pixels(this, |w, h, px| {
                let steps = (p1 - p0).abs().max_element().ceil().max(1.0) as i64;
                let mut last = (-1i64, -1i64);
                for i in 0..=steps {
                    let p = p0 + (p1 - p0) * (i as f32 / steps as f32);
                    let (x, y) = (p.x.floor() as i64, p.y.floor() as i64);
                    if (x, y) == last || x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                        continue;
                    }
                    last = (x, y);
                    let o = ((y * w as i64 + x) * 4) as usize;
                    blend(&mut px[o..o + 4], src, mode);
                }
            })?;
            touched(lua, this)?;
            none()
        }
        "DrawTriangle" => {
            let a = v2(&arg(args, 1)).unwrap_or(Vec2::ZERO);
            let b = v2(&arg(args, 2)).unwrap_or(Vec2::ZERO);
            let c = v2(&arg(args, 3)).unwrap_or(Vec2::ZERO);
            let src = rgba_of(&arg(args, 4), &arg(args, 5));
            let mode = combine_arg(&arg(args, 6));
            edit_pixels(this, |w, h, px| {
                let lo = a.min(b).min(c).floor().max(Vec2::ZERO);
                let hi = a.max(b).max(c).ceil().min(Vec2::new(w as f32, h as f32));
                let edge = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
                let area = edge(a, b, c);
                if area.abs() < 1e-6 {
                    return;
                }
                for y in lo.y as i64..hi.y as i64 {
                    for x in lo.x as i64..hi.x as i64 {
                        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let (e0, e1, e2) = (edge(a, b, p), edge(b, c, p), edge(c, a, p));
                        if (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0) {
                            let o = ((y * w as i64 + x) * 4) as usize;
                            blend(&mut px[o..o + 4], src, mode);
                        }
                    }
                }
            })?;
            touched(lua, this)?;
            none()
        }
        "DrawImageProjected" => project::draw_projected(lua, this, args),
        "SampleImageProjected" => project::sample_projected(lua, this, args),
        "DrawImage" | "DrawImageTransformed" => {
            // DrawImage(position, image, combineType);
            // DrawImageTransformed(position, scale, rotation, image, options).
            let (pos, scale, rot, img_v, mode, pivot) = if name == "DrawImage" {
                (v2(&arg(args, 1)).unwrap_or(Vec2::ZERO), Vec2::ONE, 0.0f32, arg(args, 2), combine_arg(&arg(args, 3)), None)
            } else {
                let opts = arg(args, 5);
                (
                    v2(&arg(args, 1)).unwrap_or(Vec2::ZERO),
                    v2(&arg(args, 2)).unwrap_or(Vec2::ONE),
                    num(&arg(args, 3)).unwrap_or(0.0) as f32,
                    arg(args, 4),
                    combine_arg(&field(&opts, "CombineType")),
                    Some(v2(&field(&opts, "PivotPoint"))),
                )
            };
            let src_r = opt_inst(&img_v).ok_or_else(|| rt_err(format!("{name}: EditableImage expected")))?;
            let (sw, sh, spx) = with(|s| s.images.get(&src_r).map(|i| (i.w, i.h, i.px.clone())))
                .ok_or_else(|| rt_err(format!("{name}: EditableImage expected")))?;
            edit_pixels(this, |w, h, px| {
                let src_px = |x: i64, y: i64| -> [f32; 4] {
                    let o = ((y * sw as i64 + x) * 4) as usize;
                    [spx[o] as f32 / 255.0, spx[o + 1] as f32 / 255.0, spx[o + 2] as f32 / 255.0, spx[o + 3] as f32 / 255.0]
                };
                match pivot {
                    None => {
                        for y in 0..sh as i64 {
                            for x in 0..sw as i64 {
                                let (dx, dy) = (pos.x as i64 + x, pos.y as i64 + y);
                                if dx < 0 || dy < 0 || dx >= w as i64 || dy >= h as i64 {
                                    continue;
                                }
                                let o = ((dy * w as i64 + dx) * 4) as usize;
                                blend(&mut px[o..o + 4], src_px(x, y), mode);
                            }
                        }
                    }
                    Some(pv) => {
                        // `pos` is where the source's pivot lands (centre by default).
                        let pv = pv.unwrap_or(Vec2::new(sw as f32, sh as f32) * 0.5);
                        let (sn, cs) = rot.to_radians().sin_cos();
                        let fwd = |p: Vec2| {
                            let q = (p - pv) * scale;
                            pos + Vec2::new(q.x * cs - q.y * sn, q.x * sn + q.y * cs)
                        };
                        let corners = [fwd(Vec2::ZERO), fwd(Vec2::new(sw as f32, 0.0)), fwd(Vec2::new(0.0, sh as f32)), fwd(Vec2::new(sw as f32, sh as f32))];
                        let lo = corners.iter().fold(Vec2::splat(f32::MAX), |a, c| a.min(*c)).floor().max(Vec2::ZERO);
                        let hi = corners.iter().fold(Vec2::splat(f32::MIN), |a, c| a.max(*c)).ceil().min(Vec2::new(w as f32, h as f32));
                        if scale.x.abs() < 1e-6 || scale.y.abs() < 1e-6 {
                            return;
                        }
                        for y in lo.y as i64..hi.y as i64 {
                            for x in lo.x as i64..hi.x as i64 {
                                let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - pos;
                                let q = Vec2::new(d.x * cs + d.y * sn, -d.x * sn + d.y * cs) / scale + pv;
                                let (sx, sy) = (q.x.floor() as i64, q.y.floor() as i64);
                                if sx < 0 || sy < 0 || sx >= sw as i64 || sy >= sh as i64 {
                                    continue;
                                }
                                let o = ((y * w as i64 + x) * 4) as usize;
                                blend(&mut px[o..o + 4], src_px(sx, sy), mode);
                            }
                        }
                    }
                }
            })?;
            touched(lua, this)?;
            none()
        }
        _ => none(),
    }
}

// ------------------------------------------------------------------ meshes

fn mesh_from_uri(uri: &str) -> mlua::Result<Mesh> {
    let bytes = crate::assets::generated(uri)
        .map(Ok)
        .unwrap_or_else(|| crate::assets::script_content_blocking(uri))
        .map_err(|e| rt_err(format!("CreateEditableMeshAsync: {e}")))?;
    let d = crate::mesh::parse(&bytes).map_err(|e| rt_err(format!("CreateEditableMeshAsync: {e}")))?;
    Ok(Mesh::from_mesh_data(&d))
}

/// The mesh registered as generated content (a new id per snapshot).
fn snapshot(r: Ref) -> Option<(String, Vec3, Vec3)> {
    let (bytes, lo, hi, id) = with(|s| {
        let mut m = lock(s.meshes.get(&r)?);
        m.gen += 1;
        let (b, lo, hi) = m.to_mesh_bytes();
        Some((b, lo, hi, format!("{MESH_PREFIX}mesh/{}@{}", m.n, m.gen)))
    })?;
    crate::assets::register_generated(&id, bytes);
    Some((id, lo, hi))
}

/// AssetService:CreateMeshPartAsync(Content.fromObject(editableMesh)):
/// the generated mesh id and the mesh's bounds size.
pub fn mesh_part_source(c: &Content) -> Option<mlua::Result<(String, Vec3)>> {
    let r = c.as_object()?;
    if !is_editable_mesh(r) {
        return Some(Err(rt_err("CreateMeshPartAsync: the object is not an EditableMesh")));
    }
    let (id, lo, hi) = snapshot(r)?;
    if hi.cmplt(lo).any() || lo == hi && lo == Vec3::ZERO && hi == Vec3::ZERO {
        return Some(Err(rt_err("CreateMeshPartAsync: the EditableMesh has no faces")));
    }
    Some(Ok((id, (hi - lo).max(Vec3::splat(0.001)))))
}

fn mesh_call(lua: &Lua, name: &str, this: Ref, args: &MultiValue) -> mlua::Result<MultiValue> {
    if name == "Destroy" {
        with(|s| s.meshes.remove(&this));
        return none();
    }
    let m = with(|s| s.meshes.get(&this).cloned()).ok_or_else(|| rt_err("the EditableMesh was destroyed"))?;
    let mut g = lock(&m);
    Ok(em::call(lua, &mut g, name, args)?.unwrap_or_default())
}
