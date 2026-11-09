//! Save as Project Folder: the open place (a file, or a place opened from
//! Roblox) written out as a Rojo / Argon project folder, the way `argon
//! init` + syncback lay one out:
//!
//! - `default.project.json` mapping each chosen container to
//!   `src/<Service>[/<Child>]`;
//! - scripts as `.server.luau` / `.client.luau` / `.luau` files (a folder
//!   with `init.*` when the script has children), Folders as folders;
//! - other instances: one `.rbxm` each when nothing inside them is a script
//!   (lossless), else a folder whose `init.meta.json` holds the class and
//!   its non-default properties, with the scripts inside as files;
//! - everything else stays in `<name>.rbxl` beside the project file (the
//!   exported children are taken out of it, so nothing is doubled);
//! - `rustudio.toml` with the chosen settings.
//!
//! Opening the folder gives back the same tree (checked by the test below).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rbx_dom_weak::types::{Ref, Variant};
use serde_json::{json, Value as J};

use crate::doc::Document;
use crate::project::{self, FileKind};
use crate::project_values::{resolve, to_json};

/// The containers offered (path under the DataModel, on by default).
pub const CONTAINERS: &[(&str, bool)] = &[
    ("ReplicatedFirst", true),
    ("ReplicatedStorage", true),
    ("ServerScriptService", true),
    ("ServerStorage", true),
    ("StarterGui", true),
    ("StarterPack", true),
    ("StarterPlayer/StarterPlayerScripts", true),
    ("StarterPlayer/StarterCharacterScripts", true),
    ("Workspace", false),
    ("Lighting", false),
    ("SoundService", false),
    ("Teams", false),
    ("TextChatService", false),
    ("Chat", false),
];

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub name: String,
    pub containers: Vec<String>,
    /// .lua instead of .luau.
    pub lua: bool,
    pub sourcemap: bool,
    /// The Roblox place / universe it came from (Rojo's placeId / gameId).
    pub place_id: Option<u64>,
    pub game_id: Option<u64>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            name: "game".into(),
            containers: CONTAINERS.iter().filter(|c| c.1).map(|c| c.0.to_string()).collect(),
            lua: false,
            sourcemap: true,
            place_id: None,
            game_id: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct ExportReport {
    pub project_file: PathBuf,
    pub scripts: usize,
    pub models: usize,
    pub folders: usize,
    pub warnings: Vec<String>,
}

/// Properties Studio fills in itself (ids, caches): not worth keeping.
const IGNORABLE: &[&str] = &[
    "SourceAssetId", "UniqueId", "HistoryId", "Capabilities", "DefinesCapabilities", "ScriptGuid",
    "LinkedSource", "ModelMeshCFrame", "ModelMeshSize", "WorldPivotData", "NeedsPivotMigration",
];

fn is_script(class: &str) -> bool {
    matches!(class, "Script" | "LocalScript" | "ModuleScript")
}

fn file_safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if "/\\:*?\"<>|\0".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let s = s.trim_end_matches(['.', ' ']).to_string();
    if s.trim().is_empty() || s == "." || s == ".." {
        "_".into()
    } else {
        s
    }
}

/// The reflection default of a property (superclasses followed).
fn default_of(class: &str, prop: &str) -> Option<&'static Variant> {
    let db = crate::doc::db();
    let mut cur = db.classes.get(class)?;
    loop {
        if let Some(v) = cur.default_properties.get(prop) {
            return Some(v);
        }
        cur = db.classes.get(cur.superclass.as_deref()?)?;
    }
}

struct Ctx<'a> {
    doc: &'a Document,
    lua: bool,
    report: ExportReport,
    /// Instances written to files (taken out of the place file).
    root: PathBuf,
}

impl Ctx<'_> {
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).display().to_string()
    }

    fn write(&mut self, p: &Path, bytes: &[u8]) {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(p, bytes) {
            self.report.warnings.push(format!("{}: {e}", p.display()));
        }
    }

    fn has_scripts(&self, r: Ref) -> bool {
        self.doc.dom.descendants_of(r).any(|i| is_script(i.class.as_str()))
    }

    /// A data file's JSON for `r`: className (when `class`), non-default
    /// properties (but `skip`), attributes, tags, originalName.
    fn meta(&mut self, r: Ref, class: bool, skip: &[&str], original: Option<&str>) -> Option<J> {
        let i = self.doc.dom.get_by_ref(r)?;
        let cls = i.class.to_string();
        let mut o = serde_json::Map::new();
        if class {
            o.insert("className".into(), json!(cls));
        }
        let mut props: Vec<(String, J)> = Vec::new();
        let mut lost: Vec<String> = Vec::new();
        for (k, v) in &i.properties {
            let k = k.as_str();
            if matches!(k, "Name" | "Attributes" | "Tags") || skip.contains(&k) {
                continue;
            }
            if default_of(&cls, k).is_some_and(|d| d == v) {
                continue;
            }
            match to_json(&cls, k, v) {
                // Only what reads back the same.
                Some(j) if resolve(&cls, k, &j).is_ok() => props.push((k.to_string(), j)),
                _ => {
                    if !IGNORABLE.contains(&k) && !matches!(v, Variant::Ref(x) if x.is_none()) {
                        lost.push(k.to_string());
                    }
                }
            }
        }
        if !lost.is_empty() {
            lost.sort();
            self.report.warnings.push(format!("{} ({cls}): not kept: {}", i.name, lost.join(", ")));
        }
        props.sort_by(|a, b| a.0.cmp(&b.0));
        if !props.is_empty() {
            o.insert("properties".into(), J::Object(props.into_iter().collect()));
        }
        if let Some(Variant::Attributes(a)) = i.properties.get(&rbx_dom_weak::ustr("Attributes")) {
            let mut m = serde_json::Map::new();
            for (k, x) in a.iter() {
                let j = match x {
                    Variant::Bool(b) => json!(b),
                    Variant::Float64(f) => json!(f),
                    Variant::String(s) => json!(s),
                    other => match (to_json("", "", other), other.ty()) {
                        (Some(j), ty) => json!({ format!("{ty:?}"): j }),
                        _ => continue,
                    },
                };
                m.insert(k.clone(), j);
            }
            if !m.is_empty() {
                o.insert("attributes".into(), J::Object(m));
            }
        }
        if let Some(Variant::Tags(t)) = i.properties.get(&rbx_dom_weak::ustr("Tags")) {
            let t: Vec<&str> = t.iter().collect();
            if !t.is_empty() {
                o.insert("tags".into(), json!(t));
            }
        }
        if let Some(n) = original {
            o.insert("originalName".into(), json!(n));
        }
        (!o.is_empty()).then_some(J::Object(o))
    }

    /// Properties of `r` a data file can't hold (Refs, binary data, ...).
    fn lossy(&self, r: Ref) -> Vec<String> {
        let Some(i) = self.doc.dom.get_by_ref(r) else { return Vec::new() };
        let cls = i.class.as_str();
        let mut v: Vec<String> = i
            .properties
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "Name" | "Attributes" | "Tags" | "Source") && !IGNORABLE.contains(&k.as_str()))
            .filter(|(k, v)| default_of(cls, k.as_str()) != Some(*v))
            .filter(|(_, v)| !matches!(v, Variant::Ref(x) if x.is_none()))
            .filter(|(k, v)| !to_json(cls, k, v).is_some_and(|j| resolve(cls, k, &j).is_ok()))
            .map(|(k, _)| k.to_string())
            .collect();
        v.sort();
        v
    }

    fn write_meta(&mut self, p: &Path, j: Option<J>) {
        if let Some(j) = j {
            let s = serde_json::to_string_pretty(&j).unwrap_or_default() + "\n";
            self.write(p, s.as_bytes());
        }
    }

    fn children(&mut self, parent: Ref, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        let kids = self.doc.dom.get_by_ref(parent).map(|i| i.children().to_vec()).unwrap_or_default();
        let mut used: HashSet<String> = HashSet::new();
        for c in kids {
            self.child(c, dir, &mut used);
        }
    }

    fn child(&mut self, r: Ref, dir: &Path, used: &mut HashSet<String>) {
        let Some(i) = self.doc.dom.get_by_ref(r) else { return };
        let name = i.name.clone();
        let class = i.class.to_string();
        let has_kids = !i.children().is_empty();
        let kind = match class.as_str() {
            "Script" => Some(FileKind::Server),
            "LocalScript" => Some(FileKind::Client),
            "ModuleScript" => Some(FileKind::Module),
            _ => None,
        };
        // A stem the folder doesn't have yet, that reads back as this
        // name and kind (else a safe one plus originalName).
        // An instance with scripts inside becomes a folder (the scripts as
        // files) only when its data file can hold all of it; else one .rbxm.
        let whole = kind.is_none() && class != "Folder" && {
            let lost = if self.has_scripts(r) { self.lossy(r) } else { vec!["-".into()] };
            if lost.first().is_some_and(|l| l != "-") {
                self.report.warnings.push(format!(
                    "{name} ({class}) kept whole as .rbxm, scripts inside it too ({} can't go in a data file)",
                    lost.join(", ")
                ));
            }
            !lost.is_empty()
        };
        let suffix = match (kind, has_kids) {
            (Some(k), false) => suffix_of(k, self.lua),
            _ if whole => ".rbxm",
            _ => "",
        };
        let mut stem = file_safe(&name);
        if stem.eq_ignore_ascii_case("init") || stem.starts_with('.') {
            stem = format!("{stem}_");
        }
        if !suffix.is_empty() {
            let fname = format!("{stem}{suffix}");
            let back = project::classify(&fname).map(|x| x.0);
            if back.as_deref() != Some(stem.as_str()) || project::is_data_file(&fname) {
                stem = stem.replace('.', "_");
            }
        }
        let base = stem.clone();
        let mut n = 2;
        while used.contains(&stem.to_lowercase()) {
            stem = format!("{base}_{n}");
            n += 1;
        }
        used.insert(stem.to_lowercase());
        let original = (stem != name).then_some(name.as_str());

        if let Some(k) = kind {
            self.report.scripts += 1;
            let src = match i.properties.get(&rbx_dom_weak::ustr("Source")) {
                Some(Variant::String(s)) => s.clone().into_bytes(),
                Some(Variant::BinaryString(b)) => AsRef::<[u8]>::as_ref(b).to_vec(),
                _ => Vec::new(),
            };
            if has_kids {
                let d = dir.join(&stem);
                self.write(&d.join(format!("init{}", suffix_of(k, self.lua))), &src);
                let m = self.meta(r, false, &["Source"], original);
                self.write_meta(&d.join("init.meta.json"), m);
                self.children(r, &d);
            } else {
                self.write(&dir.join(format!("{stem}{suffix}")), &src);
                let m = self.meta(r, false, &["Source"], original);
                self.write_meta(&dir.join(format!("{stem}.meta.json")), m);
            }
        } else if suffix == ".rbxm" {
            self.report.models += 1;
            let p = dir.join(format!("{stem}.rbxm"));
            match self.doc.encode(&[r], false) {
                Ok(b) => self.write(&p, &b),
                Err(e) => self.report.warnings.push(format!("{}: {e}", self.rel(&p))),
            }
            if let Some(o) = original {
                self.write_meta(&dir.join(format!("{stem}.meta.json")), Some(json!({ "originalName": o })));
            }
        } else {
            // A Folder, or an instance with scripts inside: a folder.
            self.report.folders += 1;
            let d = dir.join(&stem);
            let m = self.meta(r, class != "Folder", &[], original);
            self.write_meta(&d.join("init.meta.json"), m);
            self.children(r, &d);
        }
    }
}

fn suffix_of(k: FileKind, lua: bool) -> &'static str {
    match (k, lua) {
        (FileKind::Server, false) => ".server.luau",
        (FileKind::Client, false) => ".client.luau",
        (FileKind::Server, true) => ".server.lua",
        (FileKind::Client, true) => ".client.lua",
        (_, true) => ".lua",
        _ => ".luau",
    }
}

fn find_path(doc: &Document, path: &str) -> Option<Ref> {
    let mut r = doc.dom.root_ref();
    for seg in path.split('/') {
        r = doc.dom.get_by_ref(r)?.children().iter().copied().find(|c| {
            doc.dom.get_by_ref(*c).is_some_and(|i| i.name == seg)
        })?;
    }
    Some(r)
}

/// Workspace children that stay in the place file (the place's own).
fn stays(doc: &Document, container: &str, r: Ref) -> bool {
    container == "Workspace"
        && doc.dom.get_by_ref(r).is_some_and(|i| matches!(i.class.as_str(), "Terrain" | "Camera"))
}

/// Writes `doc` as a project folder at `dir` (new or empty).
pub fn export(doc: &Document, dir: &Path, o: &ExportOptions) -> Result<ExportReport, String> {
    if dir.exists() && std::fs::read_dir(dir).map(|mut r| r.next().is_some()).unwrap_or(true) {
        return Err(format!("{} isn't empty", dir.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dir = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
    let name = if o.name.trim().is_empty() { "game".to_string() } else { o.name.trim().to_string() };
    let mut ctx = Ctx { doc, lua: o.lua, report: ExportReport::default(), root: dir.clone() };

    // The project tree, and each container's children as files.
    let mut tree = serde_json::Map::new();
    tree.insert("$className".into(), json!("DataModel"));
    let mut exported: Vec<(String, Ref)> = Vec::new();
    for c in &o.containers {
        let Some(r) = find_path(doc, c) else { continue };
        let src = format!("src/{c}");
        let out = dir.join(&src);
        // Workspace keeps its Terrain and Camera in the place.
        let kids = doc.dom.get_by_ref(r).map(|i| i.children().to_vec()).unwrap_or_default();
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        let mut used = HashSet::new();
        for k in kids {
            if !stays(doc, c, k) {
                ctx.child(k, &out, &mut used);
            }
        }
        exported.push((c.clone(), r));
        // Nested nodes for the path (StarterPlayer > StarterPlayerScripts).
        let mut node = &mut tree;
        let segs: Vec<&str> = c.split('/').collect();
        let mut cur = doc.dom.root_ref();
        for (n, seg) in segs.iter().enumerate() {
            cur = find_path(doc, &segs[..=n].join("/")).unwrap_or(cur);
            let class = doc.dom.get_by_ref(cur).map(|i| i.class.to_string()).unwrap_or_else(|| seg.to_string());
            let entry = node
                .entry(seg.to_string())
                .or_insert_with(|| json!({ "$className": class }));
            node = entry.as_object_mut().expect("object");
        }
        node.insert("$path".into(), json!(src));
        if c == "Workspace" {
            node.insert("$ignoreUnknownInstances".into(), json!(true));
        }
    }
    let mut pj = json!({ "name": name });
    if let Some(p) = o.place_id {
        pj["placeId"] = json!(p);
        pj["servePlaceIds"] = json!([p]);
    }
    if let Some(g) = o.game_id {
        pj["gameId"] = json!(g);
    }
    pj["tree"] = J::Object(tree);
    let project_file = dir.join("default.project.json");
    ctx.write(&project_file, (serde_json::to_string_pretty(&pj).unwrap_or_default() + "\n").as_bytes());

    // The place: everything but what went to files.
    let mut place = doc.clone_for_play(name.clone());
    for (c, _) in &exported {
        if let Some(r) = find_path(&place, c) {
            let kids = place.dom.get_by_ref(r).map(|i| i.children().to_vec()).unwrap_or_default();
            for k in kids {
                if !stays(&place, c, k) {
                    place.dom.destroy(k);
                }
            }
        }
    }
    place.save_to(&dir.join(format!("{name}.rbxl")))?;

    let cfg = crate::project_config::ProjectConfig {
        root: dir.clone(),
        lua_ext: if o.lua { crate::project_config::LuaExt::Lua } else { crate::project_config::LuaExt::Luau },
        sourcemap: o.sourcemap,
        ..Default::default()
    };
    cfg.save(&project_file)?;
    let mut report = ctx.report;
    report.project_file = project_file;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::InstanceBuilder;

    fn tree(doc: &Document, r: Ref, out: &mut Vec<String>, depth: usize) {
        let i = doc.dom.get_by_ref(r).unwrap();
        let src = match i.properties.get(&rbx_dom_weak::ustr("Source")) {
            Some(Variant::String(s)) => s.clone(),
            _ => String::new(),
        };
        out.push(format!("{}{} {} {:?}", " ".repeat(depth), i.class, i.name, src));
        let mut kids: Vec<Ref> = i.children().to_vec();
        kids.sort_by_key(|k| {
            let i = doc.dom.get_by_ref(*k).unwrap();
            (i.name.clone(), i.class.to_string())
        });
        for k in kids {
            tree(doc, k, out, depth + 1);
        }
    }

    #[test]
    fn export_then_open_gives_the_same_tree() {
        let mut doc = Document::new_place();
        let root = doc.dom.root_ref();
        let rs = doc.find_child(root, "ReplicatedStorage").unwrap();
        let sss = doc.find_child(root, "ServerScriptService").unwrap();
        let sg = doc.find_child(root, "StarterGui").unwrap();
        let src = |s: &str| Variant::String(s.into());
        let m = doc.dom.insert(rs, InstanceBuilder::new("ModuleScript").with_name("Util").with_property("Source", src("return {}\n")));
        doc.dom.insert(m, InstanceBuilder::new("ModuleScript").with_name("Inner").with_property("Source", src("return 1\n")));
        doc.dom.insert(rs, InstanceBuilder::new("ModuleScript").with_name("Util").with_property("Source", src("return 2\n")));
        doc.dom.insert(rs, InstanceBuilder::new("ModuleScript").with_name("a.server").with_property("Source", src("return 3\n")));
        doc.dom.insert(rs, InstanceBuilder::new("Part").with_name("Block"));
        let f = doc.dom.insert(sss, InstanceBuilder::new("Folder").with_name("Systems"));
        doc.dom.insert(f, InstanceBuilder::new("Script").with_name("Main").with_property("Source", src("print('hi')\n")));
        let gui = doc.dom.insert(sg, InstanceBuilder::new("ScreenGui").with_name("Hud").with_property("ResetOnSpawn", Variant::Bool(false)));
        let btn = doc.dom.insert(gui, InstanceBuilder::new("TextButton").with_name("Play").with_property("Text", src("Play!")));
        doc.dom.insert(btn, InstanceBuilder::new("LocalScript").with_name("Click").with_property("Source", src("-- click\n")));

        let dir = std::env::temp_dir().join(format!("rustudio-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rep = export(&doc, &dir, &ExportOptions { name: "t".into(), ..Default::default() }).unwrap();
        assert_eq!(rep.scripts, 6, "{rep:?}");
        assert!(dir.join("src/ReplicatedStorage/Util/init.luau").is_file());
        assert!(dir.join("src/ServerScriptService/Systems/Main.server.luau").is_file());
        assert!(dir.join("src/StarterGui/Hud/Play/Click.client.luau").is_file());
        assert!(dir.join("src/ReplicatedStorage/Block.rbxm").is_file());

        let (back, st) = crate::project_sync::open(&rep.project_file).unwrap();
        crate::project_config::set_active(None);
        for svc in ["ReplicatedStorage", "ServerScriptService", "StarterGui"] {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            tree(&doc, doc.find_child(doc.dom.root_ref(), svc).unwrap(), &mut a, 0);
            tree(&back, back.find_child(back.dom.root_ref(), svc).unwrap(), &mut b, 0);
            assert_eq!(a, b, "{svc}");
        }
        let hud = back.dom.descendants().find(|i| i.name == "Hud").unwrap();
        assert_eq!(hud.properties.get(&rbx_dom_weak::ustr("ResetOnSpawn")), Some(&Variant::Bool(false)));
        let play = back.dom.descendants().find(|i| i.name == "Play").unwrap();
        assert_eq!(play.properties.get(&rbx_dom_weak::ustr("Text")), Some(&src("Play!")));
        // The sourcemap lists the scripts with their files.
        let sm = st.sourcemap_json(&back, false).to_string();
        assert!(sm.contains("src/ReplicatedStorage/Util/init.luau"), "{sm}");
        assert!(!sm.contains("Block"), "{sm}");
        assert!(dir.join("sourcemap.json").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod real {
    use super::*;

    /// Floats rounded to 5 decimals (rbx_binary snaps near-axis rotations).
    fn fuzz(s: &str) -> String {
        let mut out = String::new();
        let mut num = String::new();
        let flush = |num: &mut String, out: &mut String| {
            if !num.is_empty() {
                match num.parse::<f64>() {
                    Ok(f) if num.contains('.') || num.contains('e') => {
                        let r = (f * 1e5).round() / 1e5;
                        out.push_str(&format!("{}", if r == 0.0 { 0.0 } else { r }));
                    }
                    _ => out.push_str(num),
                }
                num.clear();
            }
        };
        for ch in s.chars() {
            if ch.is_ascii_digit() || ch == '.' || ch == 'e' && !num.is_empty() || ch == '-' && (num.is_empty() || num.ends_with('e')) {
                num.push(ch);
            } else {
                flush(&mut num, &mut out);
                out.push(ch);
            }
        }
        flush(&mut num, &mut out);
        out
    }

    /// path → (class, properties) for every instance, sibling duplicates numbered.
    fn flat(doc: &Document) -> std::collections::BTreeMap<String, (String, Vec<(String, String)>)> {
        let mut out = std::collections::BTreeMap::new();
        fn walk(doc: &Document, r: Ref, path: String, out: &mut std::collections::BTreeMap<String, (String, Vec<(String, String)>)>) {
            let i = doc.dom.get_by_ref(r).unwrap();
            let mut props: Vec<(String, String)> = i
                .properties
                .iter()
                .filter(|(k, v)| !matches!(v, Variant::Ref(_) | Variant::UniqueId(_) | Variant::SharedString(_)) && k.as_str() != "HistoryId")
                .filter(|(k, v)| default_of(i.class.as_str(), k.as_str()) != Some(*v))
                .map(|(k, v)| match v {
                    Variant::Font(f) => (k.to_string(), format!("{} {:?} {:?}", f.family, f.weight, f.style)),
                    v => (k.to_string(), fuzz(&format!("{v:?}"))),
                })
                .collect();
            props.sort();
            out.insert(path.clone(), (i.class.to_string(), props));
            let mut seen: std::collections::HashMap<String, usize> = Default::default();
            let mut kids = i.children().to_vec();
            kids.sort_by_cached_key(|k| {
                let c = doc.dom.get_by_ref(*k).unwrap();
                let mut p: Vec<String> = c.properties.iter()
                    .filter(|(_, v)| !matches!(v, Variant::Ref(_) | Variant::UniqueId(_) | Variant::SharedString(_)))
                    .filter(|(n, v)| default_of(c.class.as_str(), n.as_str()) != Some(*v))
                    .map(|(n, v)| match v {
                        Variant::Font(f) => format!("{n}={} {:?} {:?}", f.family, f.weight, f.style),
                        v => format!("{n}={}", fuzz(&format!("{v:?}"))),
                    }).collect();
                p.sort();
                (c.name.clone(), c.class.to_string(), p.join(","), doc.dom.descendants_of(*k).count())
            });
            for k in kids {
                let c = doc.dom.get_by_ref(k).unwrap();
                let n = seen.entry(c.name.clone()).or_default();
                *n += 1;
                walk(doc, k, format!("{path}/{}#{n}", c.name), out);
            }
        }
        walk(doc, doc.dom.root_ref(), String::new(), &mut out);
        out
    }

    /// `RUSTUDIO_EXPORT_PLACE=x.rbxl cargo test --release export_real_place -- --ignored`
    #[test]
    #[ignore]
    fn export_real_place() {
        let place = std::env::var("RUSTUDIO_EXPORT_PLACE").expect("RUSTUDIO_EXPORT_PLACE");
        let doc = Document::open(Path::new(&place)).unwrap();
        let dir = std::env::temp_dir().join(format!("rustudio-export-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rep = export(&doc, &dir, &ExportOptions { name: "g".into(), ..Default::default() }).unwrap();
        let (back, _) = crate::project_sync::open(&rep.project_file).unwrap();
        crate::project_config::set_active(None);
        let (a, b) = (flat(&doc), flat(&back));
        let mut diffs = Vec::new();
        for (k, va) in &a {
            match b.get(k) {
                None => diffs.push(format!("missing {k}")),
                Some(vb) if vb != va => {
                    let pa: std::collections::BTreeMap<_, _> = va.1.iter().cloned().collect();
                    let pb: std::collections::BTreeMap<_, _> = vb.1.iter().cloned().collect();
                    let mut d: Vec<String> = pa.iter().filter(|(p, v)| pb.get(*p) != Some(v)).map(|(p, v)| format!("{p}: {v:.400} -> {:.400}", pb.get(p).map(|s| s.as_str()).unwrap_or("<none>"))).collect();
                    d.extend(pb.keys().filter(|p| !pa.contains_key(*p)).map(|p| format!("{p}: <none> -> {:.60}", pb[p])));
                    if va.0 != vb.0 { d.push(format!("class {} -> {}", va.0, vb.0)); }
                    diffs.push(format!("{k}: {}", d.join("; ")));
                }
                _ => {}
            }
        }
        for k in b.keys().filter(|k| !a.contains_key(*k)) {
            diffs.push(format!("extra {k}"));
        }
        eprintln!("{} instances, {} differ; warnings {:?}", a.len(), diffs.len(), rep.warnings.iter().filter(|w| w.contains("kept whole")).take(6).collect::<Vec<_>>());
        for d in diffs.iter().take(40) {
            eprintln!("  {d}");
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(diffs.is_empty());
    }
}
