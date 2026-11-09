//! Roblox's default player and character scripts, inserted at Play the way
//! Studio does it (checked in an offline Studio: the client's PlayerScripts and the
//! character after Play start, source lengths matched file by file).
//!
//! Nothing is copied into rustudio: the scripts are read at Play start from
//! the Studio install (read only), found as `RUSTUDIO_STUDIO_DIR`, else the
//! folder the default content link (`~/.local/share/rustudio/content`) points
//! into, else the copy rustudio downloaded from Roblox's CDN
//! (`~/.local/share/rustudio/studio`, see studio_content.rs), else
//! `~/.local/share/rustudio/studio`.
//!
//! - StarterPlayerScripts gets `PlayerScriptsLoader` (LocalScript),
//!   `PlayerModule` (ModuleScript tree: CameraModule, ControlModule,
//!   CommonUtils) and `RbxCharacterSounds` (LocalScript + AtomicBinding),
//!   from `ExtraContent/scripts/PlayerScripts/StarterPlayerScripts` and
//!   `StarterPlayerScriptsCommon`.
//! - Each new character gets `Health` (Script,
//!   `content/avatar/scripts/humanoidHealthRegenScript.rbxmx`) and `Animate`
//!   (LocalScript with its animation values: R15
//!   `content/avatar/scripts/humanoidAnimateR15_DEPRECATED_20260813.rbxm`,
//!   R6 `content/avatar/scripts/humanoidAnimatePlayEmote.rbxm`).
//!
//! A place's own script of the same name (in StarterPlayerScripts or
//! StarterCharacterScripts) wins, as in Studio.

use std::path::{Path, PathBuf};

use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{InstanceBuilder, WeakDom};

use crate::doc::Document;
use crate::rig::RigType;

/// The Studio install the default scripts are read from.
pub fn studio_dir() -> Option<PathBuf> {
    let has = |p: &Path| p.join("ExtraContent/scripts/PlayerScripts").is_dir();
    if let Ok(p) = std::env::var("RUSTUDIO_STUDIO_DIR") {
        let p = PathBuf::from(p);
        return has(&p).then_some(p);
    }
    // The person's content link usually points at Studio's own content/.
    if let Ok(target) = std::fs::canonicalize(crate::paths::data().join("rustudio/content")) {
        if let Some(root) = target.parent() {
            if has(root) {
                return Some(root.to_path_buf());
            }
        }
    }
    // Studio's content downloaded by rustudio itself (studio_content.rs).
    if let Some(d) = crate::studio_content::installed() {
        if has(&d) {
            return Some(d);
        }
    }
    let fallback = crate::paths::data().join("rustudio/studio");
    has(&fallback).then_some(fallback)
}

const PLAYER_SCRIPTS: &str = "ExtraContent/scripts/PlayerScripts";

/// A child by name (Document::find_child matches the class).
pub fn child_named(doc: &Document, parent: Ref, name: &str) -> Option<Ref> {
    let p = doc.dom.get_by_ref(parent)?;
    p.children()
        .iter()
        .copied()
        .find(|c| doc.dom.get_by_ref(*c).is_some_and(|i| i.name == name))
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn script(class: &str, name: &str, source: String) -> InstanceBuilder {
    InstanceBuilder::new(class)
        .with_name(name)
        .with_property("Source", Variant::String(source))
}

/// A Rojo-style folder of `.lua` files: `X.lua` next to a folder `X` is a
/// ModuleScript with the folder's contents as children; a folder alone is a
/// Folder. `*.spec.lua` test files are left out (Studio leaves them out too).
fn module_children(dir: &Path) -> Vec<InstanceBuilder> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    let mut out = Vec::new();
    for p in &entries {
        let Some(file) = p.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if p.is_file() {
            let Some(stem) = file.strip_suffix(".lua") else {
                continue;
            };
            if stem.ends_with(".spec") {
                continue;
            }
            let Some(src) = read(p) else { continue };
            let mut b = script("ModuleScript", stem, src);
            let sub = dir.join(stem);
            if sub.is_dir() {
                b = b.with_children(module_children(&sub));
            }
            out.push(b);
        } else if p.is_dir() && !dir.join(format!("{file}.lua")).is_file() {
            out.push(
                InstanceBuilder::new("Folder")
                    .with_name(file)
                    .with_children(module_children(p)),
            );
        }
    }
    out
}

/// The PlayerModule tree (StarterPlayerScripts/PlayerModule.module*).
fn player_module(root: &Path) -> Option<InstanceBuilder> {
    let base = root.join(PLAYER_SCRIPTS).join("StarterPlayerScripts");
    let src = read(&base.join("PlayerModule.module.lua"))?;
    Some(
        script("ModuleScript", "PlayerModule", src)
            .with_children(module_children(&base.join("PlayerModule.module"))),
    )
}

/// Puts the default player scripts into StarterPlayer.StarterPlayerScripts
/// (made if missing) unless the place has its own of the same name. Returns
/// the names inserted.
pub fn insert_player_scripts(doc: &mut Document) -> Vec<&'static str> {
    let Some(root_dir) = studio_dir() else {
        return Vec::new();
    };
    let root = doc.dom.root_ref();
    let sp = doc.find_child(root, "StarterPlayer").unwrap_or_else(|| {
        doc.dom.insert(
            root,
            InstanceBuilder::new("StarterPlayer").with_name("StarterPlayer"),
        )
    });
    let sps = doc
        .find_child(sp, "StarterPlayerScripts")
        .unwrap_or_else(|| {
            doc.dom.insert(
                sp,
                InstanceBuilder::new("StarterPlayerScripts").with_name("StarterPlayerScripts"),
            )
        });
    let base = root_dir.join(PLAYER_SCRIPTS);
    let mut added = Vec::new();
    let mut add = |doc: &mut Document, name: &'static str, b: Option<InstanceBuilder>| {
        if child_named(doc, sps, name).is_some() {
            return;
        }
        if let Some(b) = b {
            let r = doc.dom.insert(sps, b);
            doc.mark_tree(r);
            added.push(name);
        }
    };
    add(
        doc,
        "PlayerScriptsLoader",
        read(&base.join("StarterPlayerScripts/PlayerScriptsLoader.lua"))
            .map(|s| script("LocalScript", "PlayerScriptsLoader", s)),
    );
    add(doc, "PlayerModule", player_module(&root_dir));
    let sounds = read(&base.join("StarterPlayerScriptsCommon/RbxCharacterSounds.lua")).map(|s| {
        let mut b = script("LocalScript", "RbxCharacterSounds", s);
        if let Some(ab) =
            read(&base.join("StarterPlayerScriptsCommon/RbxCharacterSounds/AtomicBinding.lua"))
        {
            b = b.with_child(script("ModuleScript", "AtomicBinding", ab));
        }
        b
    });
    add(doc, "RbxCharacterSounds", sounds);
    added
}

/// Reads a model file (binary or XML) into its own DOM.
fn read_model(path: &Path) -> Option<WeakDom> {
    Document::decode(&std::fs::read(path).ok()?).ok()
}

/// Moves the model file's top-level instance named `name` into `parent`.
fn insert_model(doc: &mut Document, path: &Path, name: &str, parent: Ref) -> Option<Ref> {
    let mut tmp = read_model(path)?;
    let top = tmp
        .root()
        .children()
        .iter()
        .copied()
        .find(|r| tmp.get_by_ref(*r).is_some_and(|i| i.name == name))?;
    tmp.transfer(top, &mut doc.dom, parent);
    doc.mark_tree(top);
    Some(top)
}

/// Adds Health and Animate to a new character (after StarterCharacterScripts
/// were copied in, so a place's own Health / Animate stays). Returns the names added.
pub fn insert_character_scripts(
    doc: &mut Document,
    character: Ref,
    rig: RigType,
) -> Vec<&'static str> {
    let Some(root_dir) = studio_dir() else {
        return Vec::new();
    };
    let avatar = root_dir.join("content/avatar/scripts");
    let mut added = Vec::new();
    if child_named(doc, character, "Health").is_none()
        && insert_model(
            doc,
            &avatar.join("humanoidHealthRegenScript.rbxmx"),
            "Health",
            character,
        )
        .is_some()
    {
        added.push("Health");
    }
    let animate = match rig {
        RigType::R15 => "humanoidAnimateR15_DEPRECATED_20260813.rbxm",
        RigType::R6 => "humanoidAnimatePlayEmote.rbxm",
    };
    if child_named(doc, character, "Animate").is_none()
        && insert_model(doc, &avatar.join(animate), "Animate", character).is_some()
    {
        added.push("Animate");
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scripts_from_studio_folder() {
        let Some(_) = studio_dir() else { return }; // no Studio install on this machine
        let mut d = Document::empty();
        let added = insert_player_scripts(&mut d);
        assert_eq!(
            added,
            vec!["PlayerScriptsLoader", "PlayerModule", "RbxCharacterSounds"]
        );
        let root = d.dom.root_ref();
        let sps = d
            .find_child(
                d.find_child(root, "StarterPlayer").unwrap(),
                "StarterPlayerScripts",
            )
            .unwrap();
        let n = |p: Ref, name: &str| child_named(&d, p, name);
        let pm = n(sps, "PlayerModule").unwrap();
        let cam = n(pm, "CameraModule").unwrap();
        assert!(n(cam, "ZoomController")
            .and_then(|z| n(z, "Popper"))
            .is_some());
        assert!(n(cam, "VRCameraTeleportDetector.spec").is_none());
        assert_eq!(
            d.class_of(n(pm, "CommonUtils").unwrap())
                .map(|c| c.to_string())
                .as_deref(),
            Some("Folder")
        );
        // Studio's own lengths (offline Studio, client PlayerScripts during Play).
        fn len(d: &Document, r: Ref) -> usize {
            match d.prop(r, "Source") {
                Some(Variant::String(s)) => s.len(),
                _ => 0,
            }
        }
        assert_eq!(len(&d, pm), 793);
        assert_eq!(len(&d, cam), 25338);
        // A second call keeps what is there.
        assert!(insert_player_scripts(&mut d).is_empty());
        let ch = d
            .dom
            .insert(root, InstanceBuilder::new("Model").with_name("Player"));
        assert_eq!(
            insert_character_scripts(&mut d, ch, RigType::R15),
            vec!["Health", "Animate"]
        );
        let anim = child_named(&d, ch, "Animate").unwrap();
        assert_eq!(len(&d, anim), 28802);
        assert!(child_named(&d, anim, "walk").is_some());
    }
}
