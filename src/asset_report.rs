//! The asset report (round 23): every image and mesh a place's properties
//! name, and whether rustudio shows it. `RUSTUDIO_ASSET_REPORT=FILE` writes
//! it as JSON at each harness screenshot; tools/fidelity puts it in the
//! place's report so a texture that stops loading shows up.
//!
//! Per id: `loaded`; `failed` with why (`not fetched: …` — the download's
//! error, e.g. Roblox asking for a signed-in session — or `not decoded: …`);
//! `downloading`; or `not requested` (the property is never read by the
//! renderer: a rustudio gap).

use std::collections::BTreeMap;

use rbx_dom_weak::types::Variant;
use serde_json::{json, Value};

use crate::app::Editor;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Image,
    Mesh,
}

/// The content properties the report follows: (property, kind).
fn kind_of(prop: &str) -> Option<Kind> {
    Some(match prop {
        "MeshId" | "MeshID" => Kind::Mesh,
        "TextureID" | "TextureId" | "Texture" | "Image" | "ColorMap" | "NormalMap" | "MetalnessMap"
        | "RoughnessMap" | "ShirtTemplate" | "PantsTemplate" | "Graphic" | "Face" | "SkyboxBk" | "SkyboxDn"
        | "SkyboxFt" | "SkyboxLf" | "SkyboxRt" | "SkyboxUp" | "SunTextureId" | "MoonTextureId"
        | "HoverImage" | "PressedImage" | "ColorMapContent" | "TextureContent" | "ImageContent"
        | "MeshContent" => {
            if prop == "MeshContent" {
                Kind::Mesh
            } else {
                Kind::Image
            }
        }
        _ => return None,
    })
}

/// Workspace.Model.Part, as GetFullName.
fn full_name(doc: &crate::doc::Document, r: rbx_dom_weak::types::Ref) -> String {
    let root = doc.dom.root_ref();
    let mut names = Vec::new();
    let mut at = r;
    while at != root {
        let Some(i) = doc.dom.get_by_ref(at) else { break };
        names.push(i.name.clone());
        at = i.parent();
    }
    names.reverse();
    names.join(".")
}

fn content_str(v: &Variant) -> Option<String> {
    let s = match v {
        Variant::Content(c) => c.as_uri()?.to_string(),
        Variant::ContentId(c) => c.as_str().to_string(),
        Variant::String(s) => s.clone(),
        _ => return None,
    };
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

impl Editor {
    /// The report as JSON (the open DataModel; in Play the server's, whose
    /// parts are what both sides draw).
    pub fn asset_report(&self) -> Value {
        // id -> (kind, users "Class.Prop" -> count, an example path)
        let mut ids: BTreeMap<String, (Kind, BTreeMap<String, u32>, String)> = BTreeMap::new();
        for inst in self.doc.dom.descendants() {
            for (name, v) in &inst.properties {
                let Some(kind) = kind_of(name.as_str()) else { continue };
                let Some(id) = content_str(v) else { continue };
                let e = ids.entry(id).or_insert_with(|| (kind, BTreeMap::new(), full_name(&self.doc, inst.referent())));
                *e.1.entry(format!("{}.{}", inst.class, name)).or_default() += 1;
            }
        }
        let mut rows = Vec::new();
        let mut summary: BTreeMap<&str, BTreeMap<String, u32>> = BTreeMap::new();
        for (id, (kind, users, example)) in &ids {
            // (images: the 3D view's textures, else the GUI's, which
            // ParticleEmitter / Beam / Trail textures load through: round 26)
            let state = match kind {
                Kind::Mesh => self.meshes.state(id),
                Kind::Image => self.textures.state(id).or_else(|| self.gui.image_state(id)),
            };
            let (status, why) = match state {
                Some(Ok(())) => ("loaded", None),
                Some(Err(w)) => ("failed", Some(w)),
                None => match self.assets.why_not(id) {
                    Some(w) if w == "still downloading" => ("downloading", None),
                    Some(w) => ("failed", Some(format!("not fetched: {w}"))),
                    None => ("not requested", None),
                },
            };
            let k = if *kind == Kind::Mesh { "mesh" } else { "image" };
            *summary.entry(k).or_default().entry(status.to_string()).or_default() += 1;
            rows.push(json!({
                "id": id,
                "kind": k,
                "status": status,
                "why": why,
                "users": users,
                "example": example,
            }));
        }
        json!({ "summary": summary, "ids": rows })
    }

    /// RUSTUDIO_ASSET_REPORT: write the report (called at each harness shot).
    pub fn write_asset_report(&self) {
        let Some(path) = std::env::var_os("RUSTUDIO_ASSET_REPORT") else { return };
        let r = self.asset_report();
        let _ = std::fs::write(path, serde_json::to_string_pretty(&r).unwrap_or_default());
    }
}
