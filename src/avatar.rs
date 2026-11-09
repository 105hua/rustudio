//! The signed-in person's avatar for Play: rig type, body colours, scales,
//! classic clothing, face, accessories and layered clothing, from the
//! session's own avatar description (avatar.roblox.com/v1/avatar) and asset
//! delivery with the session. Signed out, the default body is used.

use rbx_dom_weak::types::{Content, ContentId, Enum, Ref, Variant};
use rbx_dom_weak::{ustr, InstanceBuilder, WeakDom};
use rustudio_account::avatar::{parse_avatar, AvatarInfo};

use crate::app::Editor;
use crate::assets::SharedClient;
use crate::doc;
use crate::rig::{BodyColors, Scales};

/// What a character should look like.
#[derive(Debug, Clone)]
pub struct AvatarLook {
    pub user_id: u64,
    pub r15: bool,
    pub colors: BodyColors,
    pub scales: Scales,
    /// Clothing template / graphic content ids.
    pub shirt: Option<String>,
    pub pants: Option<String>,
    pub tshirt: Option<String>,
    pub face: Option<String>,
    /// Accessory models (.rbxm bytes).
    pub accessories: Vec<Vec<u8>>,
    /// Body part packages (Torso / arms / legs: .rbxm bytes with R6
    /// CharacterMeshes and R15 MeshParts) and the head (a SpecialMesh).
    pub body_parts: Vec<Vec<u8>>,
    pub head: Option<Vec<u8>>,
    /// Worn items rustudio can't show yet (animations, mood…).
    pub skipped: Vec<String>,
}

/// A content property, under its classic name or the newer `…Content` name
/// files are migrated to (e.g. ShirtTemplate → ShirtTemplateContent).
fn content_of(dom: &WeakDom, class: &str, prop: &str) -> Option<String> {
    let i = dom.descendants().find(|i| i.class == class)?;
    [prop.to_string(), format!("{prop}Content")]
        .iter()
        .find_map(|name| match i.properties.get(&ustr(name)) {
            Some(Variant::ContentId(c)) => Some(c.as_str().to_string()),
            Some(Variant::Content(c)) => c.as_uri().map(str::to_string),
            Some(Variant::String(s)) => Some(s.clone()),
            _ => None,
        })
        .filter(|s| !s.is_empty())
}

fn decode_model(bytes: &[u8]) -> Result<WeakDom, String> {
    doc::Document::decode(bytes)
}

/// Enum.AssetType name for an id (from the bundled reflection database).
fn asset_type_name(info_name: &str, id: u32) -> String {
    if !info_name.is_empty() {
        return info_name.to_string();
    }
    doc::db()
        .enums
        .get("AssetType")
        .and_then(|e| {
            e.items
                .iter()
                .find(|(_, v)| **v == id)
                .map(|(k, _)| k.to_string())
        })
        .unwrap_or_default()
}

/// Body colours from avatar.roblox.com/v1/avatar's `bodyColors`, which
/// gives BrickColor numbers (headColorId …) rather than hex.
fn v1_colors(v: &serde_json::Value) -> Option<[[u8; 3]; 6]> {
    let c = v.get("bodyColors")?;
    let col = |k: &str| -> Option<[u8; 3]> {
        let n = c.get(k)?.as_u64()? as u16;
        let b = rbx_dom_weak::types::BrickColor::from_number(n)?.to_color3uint8();
        Some([b.r, b.g, b.b])
    };
    Some([
        col("headColorId")?,
        col("torsoColorId")?,
        col("leftArmColorId")?,
        col("rightArmColorId")?,
        col("leftLegColorId")?,
        col("rightLegColorId")?,
    ])
}

/// Enum.AssetType names of body part packages.
fn is_body_part(kind: &str) -> bool {
    matches!(kind, "Torso" | "LeftArm" | "RightArm" | "LeftLeg" | "RightLeg")
}

/// Fetches and resolves the signed-in person's avatar (runs on a worker
/// thread): the session's own description, avatar.roblox.com/v1/avatar.
pub fn load(client: &SharedClient, log: &mut dyn FnMut(String)) -> Result<AvatarLook, String> {
    // As Studio does (sub_1407db6c0): the avatar-fetch description for this
    // place, whose resolvedAvatarType is the rig Roblox resolved. A local
    // file has no place id, so place 0. Falls back to the session's own
    // avatar.roblox.com/v1/avatar.
    let (user_id, json) = {
        let guard = client.lock().map_err(|_| "account busy".to_string())?;
        let c = guard.as_ref().ok_or("signed out")?;
        let id = c.user_id();
        let fetched = c
            .session
            .get(&rustudio_account::avatar::avatar_fetch_url(0, id))
            .ok()
            .filter(|(status, _)| *status == 200)
            .and_then(|(_, text)| serde_json::from_str::<serde_json::Value>(&text).ok())
            .filter(|v| v.get("resolvedAvatarType").is_some());
        match fetched {
            Some(v) => (id, v),
            None => (id, c.avatar_json().map_err(|e| e.to_string())?),
        }
    };
    let mut info: AvatarInfo = parse_avatar(user_id, &json);
    if let Some(c) = v1_colors(&json) {
        info.colors = c;
    }
    log(format!(
        "Avatar of user {user_id}: {} with {} worn items",
        if info.r15 { "R15" } else { "R6" },
        info.assets.len()
    ));
    let c = info.colors;
    let mut look = AvatarLook {
        user_id,
        r15: info.r15,
        colors: BodyColors {
            head: c[0],
            torso: c[1],
            left_arm: c[2],
            right_arm: c[3],
            left_leg: c[4],
            right_leg: c[5],
        },
        scales: Scales {
            height: info.height,
            width: info.width,
            depth: info.depth,
            head: info.head,
        },
        shirt: None,
        pants: None,
        tshirt: None,
        face: None,
        accessories: Vec::new(),
        body_parts: Vec::new(),
        head: None,
        skipped: Vec::new(),
    };
    for a in &info.assets {
        let kind = asset_type_name(&a.type_name, a.type_id);
        // Layered clothing (Jacket, Sweater, Shorts, DressSkirt, TShirt /
        // Shirt / Pants accessories, shoes) is an Accessory with a WrapLayer:
        // wrap.rs fits it onto R15 bodies.
        let wanted = matches!(
            kind.as_str(),
            "Shirt" | "Pants" | "TShirt" | "Face" | "Hat" | "Head" | "DynamicHead"
        ) || is_body_part(&kind)
            || kind.ends_with("Accessory");
        if !wanted {
            look.skipped.push(format!("{} ({kind})", a.name));
            continue;
        }
        let bytes = {
            let mut guard = client.lock().map_err(|_| "account busy".to_string())?;
            let c = guard.as_mut().ok_or("signed out")?;
            match c.download_asset(a.id, None) {
                Ok(b) => b,
                Err(e) => {
                    log(format!(
                        "Avatar item {} ({kind}) not downloaded: {e}",
                        a.name
                    ));
                    continue;
                }
            }
        };
        let dom = match decode_model(&bytes) {
            Ok(d) => d,
            Err(e) => {
                log(format!(
                    "Avatar item {} ({kind}) could not be read: {e}",
                    a.name
                ));
                continue;
            }
        };
        match kind.as_str() {
            "Shirt" => look.shirt = content_of(&dom, "Shirt", "ShirtTemplate"),
            "Pants" => look.pants = content_of(&dom, "Pants", "PantsTemplate"),
            "TShirt" => look.tshirt = content_of(&dom, "ShirtGraphic", "Graphic"),
            "Face" => look.face = content_of(&dom, "Decal", "Texture"),
            "Head" | "DynamicHead" => look.head = Some(bytes),
            k if is_body_part(k) => look.body_parts.push(bytes),
            _ => look.accessories.push(bytes),
        }
    }
    if !look.skipped.is_empty() {
        log(format!("Not shown yet: {}", look.skipped.join(", ")));
    }
    Ok(look)
}

impl Editor {
    /// Puts the avatar's clothing, face and accessories on a new character
    /// (server DataModel; they replicate like any other instances).
    pub fn dress_character(&mut self, model: Ref) {
        let Some(look) = self.account.avatar().cloned() else {
            return;
        };
        dress(&mut self.doc, model, &look);
    }
}

/// Puts a look's clothing, face and accessories on a character model.
pub fn dress(doc: &mut doc::Document, model: Ref, look: &AvatarLook) {
    let no_face = put_on_body(doc, model, look);
    {
        let add = |doc: &mut doc::Document, class: &str, prop: &str, id: &Option<String>| {
            if let Some(id) = id {
                doc.dom.insert(
                    model,
                    InstanceBuilder::new(class)
                        .with_name(class)
                        .with_property(prop, ContentId::from(id.clone())),
                );
            }
        };
        add(doc, "Shirt", "ShirtTemplate", &look.shirt);
        add(doc, "Pants", "PantsTemplate", &look.pants);
        add(doc, "ShirtGraphic", "Graphic", &look.tshirt);
        let face = look.face.as_ref().filter(|_| !no_face);
        if let (Some(face), Some(head)) = (face, doc.find_named(model, "Head")) {
            // A face the rig already has (Roblox's default) is replaced.
            if let Some(old) = doc.find_named(head, "face") {
                doc.dom.destroy(old);
            }
            doc.dom.insert(
                head,
                InstanceBuilder::new("Decal")
                    .with_name("face")
                    .with_property("Texture", Content::from(face.clone()))
                    .with_property("Face", Enum::from_u32(5)),
            );
        }
        for bytes in &look.accessories {
            let Ok(mut tmp) = decode_model(bytes) else {
                continue;
            };
            let roots: Vec<Ref> = tmp.root().children().to_vec();
            for r in roots {
                let is_acc = tmp
                    .get_by_ref(r)
                    .is_some_and(|i| doc::is_a(&i.class, "Accoutrement"));
                if !is_acc {
                    continue;
                }
                // Handles ride along with the body; they don't collide.
                let parts: Vec<Ref> = tmp
                    .descendants_of(r)
                    .filter(|i| doc::is_a(&i.class, "BasePart"))
                    .map(|i| i.referent())
                    .collect();
                for p in parts {
                    if let Some(i) = tmp.get_by_ref_mut(p) {
                        i.properties.insert(ustr("Anchored"), Variant::Bool(false));
                        i.properties
                            .insert(ustr("CanCollide"), Variant::Bool(false));
                    }
                }
                tmp.transfer(r, &mut doc.dom, model);
                weld_accessory(&mut doc.dom, model, r);
            }
        }
        doc.mark_tree(model);
    }
}

/// Welds an accessory to the body as Roblox's Humanoid:AddAccessory does:
/// a Weld "AccessoryWeld" in the Handle (Part0 Handle, Part1 the body part,
/// C0 / C1 the matching attachments' CFrames), the Handle moved into
/// place. Models made by CreateHumanoidModelFromDescription carry these,
/// and display copies of them (Place A's lobby crew) are posed from their
/// joints alone. Legacy hats with no matching attachment go on the Head by
/// AttachmentPoint. A Handle that already has an AccessoryWeld is left be.
pub fn weld_accessory(dom: &mut WeakDom, model: Ref, acc: Ref) {
    use crate::math::Xform;
    let ident = Xform {
        pos: glam::Vec3::ZERO,
        rot: glam::Mat3::IDENTITY,
    };
    let compose = |a: &Xform, b: &Xform| Xform {
        pos: a.apply(b.pos),
        rot: a.rot * b.rot,
    };
    let kids = |dom: &WeakDom, r: Ref| -> Vec<Ref> {
        dom.get_by_ref(r).map(|i| i.children().to_vec()).unwrap_or_default()
    };
    let named = |dom: &WeakDom, r: Ref, name: &str| {
        kids(dom, r)
            .into_iter()
            .find(|c| dom.get_by_ref(*c).is_some_and(|i| i.name == name))
    };
    let cf = |dom: &WeakDom, r: Ref, k: &str| match dom.get_by_ref(r).and_then(|i| i.properties.get(&ustr(k))) {
        Some(Variant::CFrame(c)) => Xform::from_cframe(c),
        _ => ident,
    };
    let Some(handle) = named(dom, acc, "Handle")
        .filter(|h| dom.get_by_ref(*h).is_some_and(|i| doc::is_a(&i.class, "BasePart")))
    else {
        return;
    };
    if named(dom, handle, "AccessoryWeld").is_some() {
        return;
    }
    let body: Vec<Ref> = kids(dom, model)
        .into_iter()
        .filter(|c| dom.get_by_ref(*c).is_some_and(|i| doc::is_a(&i.class, "BasePart")))
        .collect();
    let att_named = |dom: &WeakDom, part: Ref, name: &str| {
        kids(dom, part).into_iter().find(|c| {
            dom.get_by_ref(*c)
                .is_some_and(|i| i.class == "Attachment" && i.name == name)
        })
    };
    let found = kids(dom, handle)
        .into_iter()
        .filter(|a| dom.get_by_ref(*a).is_some_and(|i| i.class == "Attachment"))
        .find_map(|a| {
            let name = dom.get_by_ref(a)?.name.clone();
            body.iter()
                .find_map(|p| att_named(dom, *p, &name).map(|b| (*p, cf(dom, a, "CFrame"), cf(dom, b, "CFrame"))))
        });
    let found = found.or_else(|| {
        let head = body
            .iter()
            .copied()
            .find(|p| dom.get_by_ref(*p).is_some_and(|i| i.name == "Head"))?;
        let half = match dom.get_by_ref(head)?.properties.get(&ustr("Size")) {
            Some(Variant::Vector3(s)) => s.y / 2.0,
            _ => 0.5,
        };
        let c1 = Xform {
            pos: glam::Vec3::Y * half,
            rot: glam::Mat3::IDENTITY,
        };
        Some((head, cf(dom, acc, "AttachmentPoint"), c1))
    });
    let Some((part, c0, c1)) = found else {
        return;
    };
    let part_cf = cf(dom, part, "CFrame");
    let world = compose(&compose(&part_cf, &c1), &c0.inverse());
    if let Some(i) = dom.get_by_ref_mut(handle) {
        i.properties
            .insert(ustr("CFrame"), Variant::CFrame(world.to_cframe()));
    }
    dom.insert(
        handle,
        InstanceBuilder::new("Weld")
            .with_name("AccessoryWeld")
            .with_property("Part0", Variant::Ref(handle))
            .with_property("Part1", Variant::Ref(part))
            .with_property("C0", Variant::CFrame(c0.to_cframe()))
            .with_property("C1", Variant::CFrame(c1.to_cframe())),
    );
}

/// A direct child by name.
fn child_named(doc: &doc::Document, parent: Ref, name: &str) -> Option<Ref> {
    doc.dom
        .get_by_ref(parent)?
        .children()
        .iter()
        .copied()
        .find(|c| doc.dom.get_by_ref(*c).is_some_and(|i| i.name == name))
}

/// Swaps the character's body parts for the avatar's body part packages
/// and head, as Roblox does: R6 gets each package's CharacterMesh (and the
/// head's SpecialMesh); R15 parts take the package MeshPart's mesh, size,
/// attachments and cage, then the joints are re-seated. True when the head
/// carries no face of its own (a dynamic head's "NoFace" tag).
fn put_on_body(doc: &mut doc::Document, model: Ref, look: &AvatarLook) -> bool {
    let r6 = child_named(doc, model, "Torso").is_some();
    let r15 = child_named(doc, model, "UpperTorso").is_some();
    let mut no_face = false;
    let mut changed_r15 = false;
    let packages = look.body_parts.iter().chain(look.head.iter());
    for bytes in packages {
        let Ok(mut pkg) = decode_model(bytes) else {
            continue;
        };
        let root = pkg.root_ref();
        let folder = |pkg: &WeakDom, name: &str| {
            pkg.get_by_ref(root)?
                .children()
                .iter()
                .copied()
                .find(|c| pkg.get_by_ref(*c).is_some_and(|i| i.name == name && i.class == "Folder"))
        };
        if r6 {
            // Body parts: the R6 folder's CharacterMeshes, replacing any
            // the character has for the same BodyPart.
            if let Some(f) = folder(&pkg, "R6") {
                let meshes: Vec<Ref> = pkg
                    .get_by_ref(f)
                    .map(|i| i.children().to_vec())
                    .unwrap_or_default();
                for cm in meshes {
                    let Some(part) = pkg
                        .get_by_ref(cm)
                        .filter(|i| i.class == "CharacterMesh")
                        .map(|i| i.properties.get(&ustr("BodyPart")).cloned())
                    else {
                        continue;
                    };
                    let old: Vec<Ref> = doc
                        .dom
                        .get_by_ref(model)
                        .map(|m| m.children().to_vec())
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|c| {
                            doc.dom.get_by_ref(*c).is_some_and(|i| {
                                i.class == "CharacterMesh"
                                    && i.properties.get(&ustr("BodyPart")).cloned() == part
                            })
                        })
                        .collect();
                    for o in old {
                        doc.dom.destroy(o);
                    }
                    pkg.transfer(cm, &mut doc.dom, model);
                }
            }
            // A head: its SpecialMesh replaces the Head's mesh.
            let special = pkg.get_by_ref(root).and_then(|i| {
                i.children()
                    .iter()
                    .copied()
                    .find(|c| pkg.get_by_ref(*c).is_some_and(|i| i.class == "SpecialMesh"))
            });
            if let (Some(sm), Some(head)) = (special, child_named(doc, model, "Head")) {
                let old: Vec<Ref> = doc
                    .dom
                    .get_by_ref(head)
                    .map(|h| h.children().to_vec())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|c| {
                        doc.dom
                            .get_by_ref(*c)
                            .is_some_and(|i| doc::is_a(&i.class, "DataModelMesh"))
                    })
                    .collect();
                for o in old {
                    doc.dom.destroy(o);
                }
                let tagged_no_face = pkg.get_by_ref(sm).is_some_and(|i| {
                    matches!(i.properties.get(&ustr("Tags")),
                        Some(Variant::Tags(t)) if t.iter().any(|t| t == "NoFace"))
                });
                if tagged_no_face {
                    no_face = true;
                    if let Some(face) = child_named(doc, head, "face") {
                        doc.dom.destroy(face);
                    }
                }
                pkg.transfer(sm, &mut doc.dom, head);
            }
        }
        if r15 {
            let Some(f) = ["R15Fixed", "R15ArtistIntent", "R15"]
                .iter()
                .find_map(|n| folder(&pkg, n))
            else {
                continue;
            };
            let parts: Vec<Ref> = pkg
                .get_by_ref(f)
                .map(|i| i.children().to_vec())
                .unwrap_or_default();
            for p in parts {
                let Some(src) = pkg.get_by_ref(p).filter(|i| i.class == "MeshPart") else {
                    continue;
                };
                let Some(dst) = child_named(doc, model, &src.name) else {
                    continue;
                };
                put_on_r15_part(doc, &mut pkg, p, dst, &look.scales);
                changed_r15 = true;
            }
        }
    }
    if changed_r15 {
        crate::rig::reseat_r15_joints(&mut doc.dom, model);
    }
    no_face
}

/// One R15 package MeshPart onto the character's part of the same name.
fn put_on_r15_part(
    doc: &mut doc::Document,
    pkg: &mut WeakDom,
    src: Ref,
    dst: Ref,
    scales: &crate::rig::Scales,
) {
    use glam::Vec3;
    let head = pkg.get_by_ref(src).is_some_and(|i| i.name == "Head");
    let scale = |v: rbx_dom_weak::types::Vector3| {
        let v = crate::rig::scale_vec(Vec3::new(v.x, v.y, v.z), scales, head);
        rbx_dom_weak::types::Vector3::new(v.x, v.y, v.z)
    };
    let Some(s) = pkg.get_by_ref(src) else { return };
    let props: Vec<(&str, Option<Variant>)> = [
        "MeshId",
        "MeshContent",
        "TextureID",
        "TextureContent",
        "InitialSize",
    ]
    .iter()
    .map(|k| (*k, s.properties.get(&ustr(k)).cloned()))
    .collect();
    let size = match s.properties.get(&ustr("Size")) {
        Some(Variant::Vector3(v)) => Some(scale(*v)),
        _ => None,
    };
    let kids: Vec<Ref> = s.children().to_vec();
    if let Some(d) = doc.dom.get_by_ref_mut(dst) {
        for (k, v) in props {
            match v {
                Some(v) => d.properties.insert(ustr(k), v),
                None => d.properties.remove(&ustr(k)),
            };
        }
        if let Some(size) = size {
            d.properties.insert(ustr("Size"), Variant::Vector3(size));
        }
    }
    if let (Some(size), Some(o)) = (size, child_named(doc, dst, "OriginalSize")) {
        if let Some(i) = doc.dom.get_by_ref_mut(o) {
            i.properties.insert(ustr("Value"), Variant::Vector3(size));
        }
    }
    for k in kids {
        let Some(ki) = pkg.get_by_ref(k) else { continue };
        match ki.class.as_str() {
            // The package's attachment frames (rig joints, accessories).
            "Attachment" => {
                let Some(Variant::CFrame(mut cf)) = ki.properties.get(&ustr("CFrame")).cloned()
                else {
                    continue;
                };
                cf.position = scale(cf.position);
                let Some(a) = child_named(doc, dst, &ki.name) else {
                    pkg.transfer(k, &mut doc.dom, dst);
                    if let Some(i) = doc.dom.get_by_ref_mut(k) {
                        i.properties.insert(ustr("CFrame"), Variant::CFrame(cf));
                    }
                    continue;
                };
                if let Some(i) = doc.dom.get_by_ref_mut(a) {
                    i.properties.insert(ustr("CFrame"), Variant::CFrame(cf));
                }
                if let Some(o) = child_named(doc, a, "OriginalPosition") {
                    if let Some(i) = doc.dom.get_by_ref_mut(o) {
                        i.properties.insert(ustr("Value"), Variant::Vector3(cf.position));
                    }
                }
            }
            // The cage layered clothing fits to.
            "WrapTarget" => {
                let old: Vec<Ref> = doc
                    .dom
                    .get_by_ref(dst)
                    .map(|i| i.children().to_vec())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|c| doc.dom.get_by_ref(*c).is_some_and(|i| i.class == "WrapTarget"))
                    .collect();
                for o in old {
                    doc.dom.destroy(o);
                }
                pkg.transfer(k, &mut doc.dom, dst);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_clothing_template_from_a_model() {
        let dom = WeakDom::new(
            InstanceBuilder::new("DataModel").with_child(
                InstanceBuilder::new("Shirt")
                    .with_name("Shirt")
                    .with_property(
                        "ShirtTemplate",
                        ContentId::from("http://www.roblox.com/asset/?id=123"),
                    ),
            ),
        );
        let mut bytes = Vec::new();
        rbx_binary::to_writer(&mut bytes, &dom, dom.root().children()).unwrap();
        let back = decode_model(&bytes).unwrap();
        assert_eq!(
            content_of(&back, "Shirt", "ShirtTemplate").as_deref(),
            Some("http://www.roblox.com/asset/?id=123")
        );
    }
}
