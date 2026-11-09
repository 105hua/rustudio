//! Self-test step for real character bodies and layered clothing.
//!
//! R15 in Play: the character is Studio's characterR15DynamicHeadV2.rbxm
//! (Roblox's body-part meshes, WrapTarget cages, RigAttachments, Motor6Ds)
//! with defaultDynamicHead.rbxm's head; two layered garments made from
//! Roblox's real UpperTorso cage (asset 7430071117) are added by a server
//! Script: "Shirt" (Order 1, Puffiness 1) and "Coat" (Order 2, Puffiness 0.5; Studio clamps Puffiness to [-1, 1]).
//! Each garment's inner cage is the template shrunk to 0.9, its outer cage
//! 0.97 and its surface 0.95, so fitted the Shirt sits at about 1.05 × the
//! body cage and the Coat, on the Shirt's fitted outer cage (1.07), at about
//! 1.07 + 0.5 × 0.05 = 1.095. R6 after it: character.rbxm with Roblox's six
//! classic joints and Studio's head mesh.

use glam::Vec3;
use rbx_dom_weak::types::{ContentId, Enum, Variant};
use rbx_dom_weak::InstanceBuilder;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;

const TORSO_CAGE: u64 = 7430071117;

const GIVE_SRC: &str = r#"local Players = game:GetService("Players")
local store = game:GetService("ServerStorage"):WaitForChild("Layers")
local function dress(c)
    local h = c:WaitForChild("Humanoid")
    for _, a in store:GetChildren() do
        h:AddAccessory(a:Clone())
    end
    print("dressed", #store:GetChildren())
end
Players.PlayerAdded:Connect(function(p)
    p.CharacterAdded:Connect(dress)
    if p.Character then dress(p.Character) end
end)
"#;

/// OBJ text for the cage's triangles scaled by `k` (OBJ stores v upward).
fn scaled_obj(m: &crate::mesh::MeshData, k: f32) -> String {
    let mut s = String::new();
    for (p, uv) in m.positions.iter().zip(&m.uvs) {
        let q = *p * k;
        s.push_str(&format!(
            "v {} {} {}\nvt {} {}\n",
            q.x,
            q.y,
            q.z,
            uv.x,
            1.0 - uv.y
        ));
    }
    for t in 0..m.positions.len() / 3 {
        let (a, b, c) = (t * 3 + 1, t * 3 + 2, t * 3 + 3);
        s.push_str(&format!("f {a}/{a} {b}/{b} {c}/{c}\n"));
    }
    s
}

fn garment(name: &str, order: i32, puffiness: f32, extent: Vec3) -> InstanceBuilder {
    let id = |what: &str| format!("rbxtemp://rustudio/selftest/{what}.obj");
    InstanceBuilder::new("Accessory")
        .with_name(name)
        .with_child(
            InstanceBuilder::new("MeshPart")
                .with_name("Handle")
                .with_property("MeshId", ContentId::from(id("garment")))
                .with_property(
                    "Size",
                    rbx_dom_weak::types::Vector3::new(extent.x, extent.y, extent.z),
                )
                .with_property("CanCollide", false)
                .with_property(
                    "Color",
                    rbx_dom_weak::types::Color3uint8::new(
                        if order == 1 { 220 } else { 40 },
                        60,
                        if order == 1 { 60 } else { 200 },
                    ),
                )
                .with_child(
                    InstanceBuilder::new("WrapLayer")
                        .with_name("WrapLayer")
                        .with_property("Enabled", true)
                        .with_property("Order", order)
                        .with_property("Puffiness", puffiness)
                        .with_property("ReferenceMeshId", ContentId::from(id("inner")))
                        .with_property("CageMeshId", ContentId::from(id("outer"))),
                )
                .with_child(InstanceBuilder::new("Attachment").with_name("BodyFrontAttachment")),
        )
}

impl SelfTest {
    /// Phases 80..=83: R15 with layered clothing (80, 81), R6 (82, 83).
    pub(super) fn tick_avatar(&mut self, f: u64, ed: &mut Editor) -> bool {
        let phase = self.r4.phase;
        if phase == 80 || phase == 82 {
            let r6 = phase == 82;
            let mut doc = Document::new_place();
            let root = doc.dom.root_ref();
            let sp = doc.find_child(root, "StarterPlayer").unwrap_or_else(|| {
                doc.dom.insert(
                    root,
                    InstanceBuilder::new("StarterPlayer").with_name("StarterPlayer"),
                )
            });
            if let Some(i) = doc.dom.get_by_ref_mut(sp) {
                i.properties.insert(
                    rbx_dom_weak::ustr("GameSettingsAvatar"),
                    Enum::from_u32(if r6 { 0 } else { 1 }).into(),
                );
            }
            let mut notes = Vec::new();
            if !r6 {
                // Garments from Roblox's UpperTorso cage.
                match crate::assets::public_cached(TORSO_CAGE)
                    .map_err(|e| e.to_string())
                    .and_then(|b| crate::mesh::parse(&b))
                {
                    Ok(cage) => {
                        for (what, k) in [("inner", 0.9), ("outer", 0.97), ("garment", 0.95)] {
                            crate::assets::register_generated(
                                &format!("rbxtemp://rustudio/selftest/{what}.obj"),
                                scaled_obj(&cage, k).into_bytes(),
                            );
                        }
                        let extent = (cage.max - cage.min) * 0.95;
                        let ss = doc.find_child(root, "ServerStorage").unwrap_or_else(|| {
                            doc.dom.insert(
                                root,
                                InstanceBuilder::new("ServerStorage").with_name("ServerStorage"),
                            )
                        });
                        let layers = doc
                            .dom
                            .insert(ss, InstanceBuilder::new("Folder").with_name("Layers"));
                        doc.insert(layers, garment("Shirt", 1, 1.0, extent));
                        doc.insert(layers, garment("Coat", 2, 0.5, extent));
                        let sss =
                            doc.find_child(root, "ServerScriptService")
                                .unwrap_or_else(|| {
                                    doc.dom.insert(
                                        root,
                                        InstanceBuilder::new("ServerScriptService")
                                            .with_name("ServerScriptService"),
                                    )
                                });
                        doc.insert(
                            sss,
                            InstanceBuilder::new("Script")
                                .with_name("Dress")
                                .with_property("Source", GIVE_SRC.to_string()),
                        );
                    }
                    Err(e) => notes.push(format!("UpperTorso cage {TORSO_CAGE}: {e}")),
                }
            }
            ed.set_document(doc);
            // The place's avatar setting decides (an earlier step picked R15).
            ed.play_rig = None;
            ed.start_play(crate::runtime::RunMode::Play);
            self.r6 = super::R6Test {
                started: f,
                notes,
                t0: Some(std::time::Instant::now()),
                ..Default::default()
            };
            self.r4.phase = phase + 1;
            return false;
        }
        let r6 = phase == 83;
        let waited = self.r6.t0.map_or(0, |t| t.elapsed().as_secs());
        let installed = crate::default_scripts::studio_dir().is_some();
        let doc = &ed.doc;
        let model = doc
            .workspace()
            .and_then(|ws| crate::default_scripts::child_named(doc, ws, &ed.player_name()));
        let fit = model.map(|m| crate::wrap::fitted(doc, m));
        // Waits for the character's own meshes (15 R15 parts, the R6 head)
        // and, on R15, the server Script's garments and their fitting.
        let meshes_drawn = model.map_or(0, |m| {
            doc.dom
                .get_by_ref(m)
                .map(|i| i.children().to_vec())
                .unwrap_or_default()
                .iter()
                .filter(|r| {
                    ed.renderer
                        .as_ref()
                        .and_then(|rd| rd.item_draws(**r))
                        .and_then(|d| d.first())
                        .is_some_and(|d| d.custom_mesh.is_some())
                })
                .count()
        });
        let pending = model.is_none()
            || meshes_drawn < if r6 { 1 } else { 15 }
            || (!r6
                && matches!(
                    fit,
                    Some(crate::wrap::Lookup::Pending | crate::wrap::Lookup::None)
                ));
        if (pending && waited < 60) || f.saturating_sub(self.r6.started) < 60 {
            return false;
        }
        let mut ok = true;
        let mut notes = std::mem::take(&mut self.r6.notes);
        let Some(m) = model else {
            self.steps.push(Step {
                step: if r6 {
                    "avatar-r6"
                } else {
                    "avatar-r15-layered"
                },
                passed: false,
                detail: "no character in Play".into(),
            });
            ed.stop_play();
            self.r4.phase = if r6 { 84 } else { 82 };
            return false;
        };
        let parts: Vec<_> = doc
            .dom
            .get_by_ref(m)
            .map(|i| i.children().to_vec())
            .unwrap_or_default();
        let named = |n: &str| crate::default_scripts::child_named(doc, m, n);
        // R15 Play characters get Studio 0.746's joint upgrade
        // (AnimationConstraints); R6 keeps Motor6Ds.
        let joint = if !r6 && crate::rig::joint_upgrade(doc) { "AnimationConstraint" } else { "Motor6D" };
        let motors = doc
            .dom
            .descendants_of(m)
            .filter(|i| i.class == joint)
            .count();
        let drawn_meshes = parts
            .iter()
            .filter(|r| {
                ed.renderer
                    .as_ref()
                    .and_then(|rd| rd.item_draws(**r))
                    .and_then(|d| d.first())
                    .is_some_and(|d| d.custom_mesh.is_some())
            })
            .count();
        if r6 {
            let classic = [
                "Neck",
                "Right Shoulder",
                "Left Shoulder",
                "Right Hip",
                "Left Hip",
                "RootJoint",
            ]
            .iter()
            .all(|n| doc.find_named(m, n).is_some());
            let shoulder_c0 =
                doc.find_named(m, "Right Shoulder")
                    .and_then(|j| match doc.prop(j, "C0") {
                        Some(Variant::CFrame(c)) => Some((c.position.x, c.orientation.x.z)),
                        _ => None,
                    });
            ok &= !installed
                || (classic && motors == 6 && shoulder_c0 == Some((1.0, 1.0)) && drawn_meshes >= 1);
            notes.push(format!(
                "R6 from Studio's character.rbxm: classic joints {classic} ({motors} Motor6Ds), Right Shoulder C0 (x, R02) {shoulder_c0:?} (Roblox: (1, 1)), head drawn from heads/head.mesh: {}",
                drawn_meshes >= 1
            ));
        } else {
            let mesh_of = |n: &str| {
                named(n).and_then(|p| {
                    crate::scene::content_string(doc.prop(p, "MeshContent"))
                        .or_else(|| crate::scene::content_string(doc.prop(p, "MeshId")))
                })
            };
            let hand = mesh_of("LeftHand").unwrap_or_default();
            let head = mesh_of("Head").unwrap_or_default();
            let targets = parts
                .iter()
                .filter(|p| {
                    doc.dom.get_by_ref(**p).is_some_and(|i| {
                        i.children()
                            .iter()
                            .any(|c| doc.class_of(*c).is_some_and(|k| k == "WrapTarget"))
                    })
                })
                .count();
            let body_ok = hand.ends_with("7430070991")
                && (head.ends_with("11445807850") || head.ends_with("123480233606534"))
                && motors == 15
                && targets >= 15
                && drawn_meshes >= 15;
            ok &= !installed || body_ok;
            notes.push(format!(
                "R15 from Studio's characterR15DynamicHeadV2.rbxm: LeftHand mesh {hand}, Head mesh {head}, {motors} {joint}s, {targets} WrapTargets, {drawn_meshes} parts drawn with their meshes"
            ));
            // Layered clothing.
            match fit {
                Some(crate::wrap::Lookup::Ready(fit)) => {
                    let torso = named("UpperTorso");
                    let cage_r = crate::assets::public_cached(TORSO_CAGE)
                        .ok()
                        .and_then(|b| crate::mesh::parse(&b).ok())
                        .map(|c| {
                            c.positions.iter().map(|p| p.length()).sum::<f32>()
                                / c.positions.len().max(1) as f32
                        })
                        .unwrap_or(1.0);
                    let ratio = |order: i32| -> Option<f32> {
                        let h = doc.dom.descendants_of(m).find(|i| {
                            i.name == "Handle"
                                && doc.dom.get_by_ref(i.parent()).is_some_and(|a| {
                                    a.name == if order == 1 { "Shirt" } else { "Coat" }
                                })
                        })?;
                        let pts: Vec<Vec3> = fit
                            .pieces
                            .iter()
                            .filter(|p| p.handle == h.referent() && Some(p.part) == torso)
                            .flat_map(|p| p.positions.iter().copied())
                            .collect();
                        (!pts.is_empty()).then(|| {
                            pts.iter().map(|p| p.length()).sum::<f32>() / pts.len() as f32 / cage_r
                        })
                    };
                    let (a, b) = (ratio(1), ratio(2));
                    let matched_all = fit.matched.iter().all(|(h, n)| *n > 0 && h == n);
                    let good = matched_all
                        && a.is_some_and(|a| (a - 1.05).abs() < 0.03)
                        && b.is_some_and(|b| (b - 1.095).abs() < 0.03);
                    ok &= good;
                    notes.push(format!(
                        "layered: inner cage points paired by UV {:?}; on UpperTorso the Shirt (Order 1, Puffiness 1) sits at {:.3} × the body cage (want 1.05), the Coat (Order 2, Puffiness 0.5) at {:.3} (want 1.095), {} pieces",
                        fit.matched,
                        a.unwrap_or(0.0),
                        b.unwrap_or(0.0),
                        fit.pieces.len()
                    ));
                }
                Some(crate::wrap::Lookup::Failed(e)) => {
                    ok = false;
                    notes.push(format!("layered clothing failed: {e}"));
                }
                _ => {
                    ok = false;
                    let acc: Vec<String> = doc
                        .dom
                        .descendants_of(m)
                        .filter(|i| i.class == "WrapLayer" || i.name == "Handle")
                        .map(|i| {
                            let mut k: Vec<String> = i
                                .properties
                                .iter()
                                .map(|(k, v)| format!("{k}={v:?}").chars().take(90).collect())
                                .collect();
                            k.sort();
                            format!("{} {} {}", i.class, i.name, k.join(" "))
                        })
                        .collect();
                    notes.push(format!(
                        "layered clothing not fitted (in the character: {acc:?})"
                    ));
                }
            }
        }
        if let Ok(dir) = std::env::var("RUSTUDIO_SELFTEST_SHOTS") {
            if let Some((w, h, px)) = ed
                .renderer
                .as_ref()
                .zip(ed.render_state.as_ref())
                .and_then(|(r, rs)| r.read_frame(rs))
            {
                if let Some(img) = image::RgbaImage::from_raw(w, h, px) {
                    let _ = img.save(std::path::Path::new(&dir).join(if r6 {
                        "avatar-r6.png"
                    } else {
                        "avatar-r15.png"
                    }));
                }
            }
        }
        self.steps.push(Step {
            step: if r6 {
                "avatar-r6"
            } else {
                "avatar-r15-layered"
            },
            passed: ok,
            detail: notes.join("; "),
        });
        ed.stop_play();
        self.r4.phase = if r6 { 84 } else { 82 };
        false
    }
}
