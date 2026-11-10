//! Write-back: edits made in rustudio to synced instances go to the file
//! each one came from (src/project_sync.rs keeps the map). Only changed bytes
//! are written, in place, with the file's own line endings; new instances
//! get new files in the folder their parent maps to; removed ones move to
//! the desktop trash. Nothing outside the project's mapped paths (and its
//! project files) is ever written.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rbx_dom_weak::types::{Ref, Variant};

use super::ProjectState;
use crate::doc::{self, Document, Marks};
use crate::project::{self, data_path_for_file, FileKind, Origin};
use crate::project_values::to_json;

/// The line endings to write over a file's bytes: the project's setting,
/// else the file's own (CRLF when it has any).
fn crlf(bytes: &[u8]) -> bool {
    use crate::project_config::LineEndings;
    match crate::project_config::active().map(|c| c.line_endings) {
        Some(LineEndings::Lf) => false,
        Some(LineEndings::Crlf) => true,
        _ => bytes.windows(2).any(|w| w == b"\r\n"),
    }
}

/// `text` in the file's line endings (the editor writes LF).
fn with_endings(text: &str, crlf: bool) -> Vec<u8> {
    if !crlf {
        return text.as_bytes().to_vec();
    }
    let mut out = String::with_capacity(text.len() + 64);
    let mut prev_cr = false;
    for ch in text.chars() {
        if ch == '\n' && !prev_cr {
            out.push('\r');
        }
        prev_cr = ch == '\r';
        out.push(ch);
    }
    out.into_bytes()
}

/// A file name for an instance name (characters file systems refuse → _).
fn file_safe(name: &str) -> (String, bool) {
    let s: String = name
        .chars()
        .map(|c| if "/\\:*?\"<>|\0".contains(c) { '_' } else { c })
        .collect();
    let s = if s.trim().is_empty() || s == "." || s == ".." {
        "_".to_string()
    } else {
        s
    };
    let changed = s != name;
    (s, changed)
}

fn suffix_of(kind: FileKind, lua: bool) -> &'static str {
    match (kind, lua) {
        (FileKind::Server, false) => ".server.luau",
        (FileKind::Client, false) => ".client.luau",
        (FileKind::Module, false) => ".luau",
        (FileKind::Server, true) => ".server.lua",
        (FileKind::Client, true) => ".client.lua",
        (FileKind::Module, true) => ".lua",
        (FileKind::Txt, _) => ".txt",
        (FileKind::Csv, _) => ".csv",
        (FileKind::JsonModule, _) => ".json",
        (FileKind::JsonModel, _) => ".model.json",
        (FileKind::Rbxm, _) => ".rbxm",
        (FileKind::Rbxmx, _) => ".rbxmx",
        (FileKind::Project, _) => ".project.json",
    }
}

fn script_kind(class: &str) -> Option<FileKind> {
    match class {
        "Script" => Some(FileKind::Server),
        "LocalScript" => Some(FileKind::Client),
        "ModuleScript" => Some(FileKind::Module),
        _ => None,
    }
}

/// Properties a file's own contents already hold (not written to data files).
fn held_by_file(kind: FileKind, prop: &str) -> bool {
    match kind {
        FileKind::Server | FileKind::Client | FileKind::Module | FileKind::JsonModule => {
            prop == "Source"
        }
        FileKind::Txt => prop == "Value",
        FileKind::Csv => prop == "Contents",
        _ => false,
    }
}

/// ISO-8601 time for .trashinfo (UTC, seconds).
fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    let (h, m, s) = (secs % 86400 / 3600, secs % 3600 / 60, secs % 60);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if mo <= 2 { 1 } else { 0 };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

/// Copies a file or folder (used when the trash is on another file system).
fn copy_tree(a: &Path, b: &Path) -> std::io::Result<()> {
    if a.is_dir() {
        std::fs::create_dir_all(b)?;
        for e in std::fs::read_dir(a)? {
            let e = e?;
            copy_tree(&e.path(), &b.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(a, b).map(|_| ())
    }
}

impl ProjectState {
    /// Writes `bytes` to `path` when they differ from what's there; refuses
    /// anything outside the project's mapped paths.
    pub(super) fn write(&mut self, path: &Path, bytes: Vec<u8>) {
        if !self.writable(path) {
            self.log
                .push(format!("not written (outside the mapped paths): {}", path.display()));
            return;
        }
        if std::fs::read(path).ok().as_deref() == Some(bytes.as_slice()) {
            self.disk.insert(path.to_path_buf(), bytes);
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(path, &bytes) {
            Ok(()) => {
                self.log.push(format!("wrote {}", path.display()));
                self.disk.insert(path.to_path_buf(), bytes);
            }
            Err(e) => self.warnings.push(format!("{}: {e}", path.display())),
        }
    }

    /// Moves a file or folder to the desktop trash (macOS: the Finder Trash;
    /// Windows: the Recycle Bin; elsewhere the XDG spec's), never deleting it
    /// outright.
    fn to_trash(&mut self, path: &Path) {
        if !path.exists() || !self.writable(path) {
            return;
        }
        if !self.config.trash {
            let gone = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            match gone {
                Ok(()) => {
                    self.log.push(format!("deleted {}", path.display()));
                    let p = path.to_path_buf();
                    self.disk.retain(|k, _| !k.starts_with(&p));
                }
                Err(e) => self.warnings.push(format!("couldn't delete {}: {e}", path.display())),
            }
            return;
        }
        // RUSTUDIO_TRASH_DIR: an XDG-layout trash folder used on every
        // system instead of the desktop's own (tests).
        let own = std::env::var_os("RUSTUDIO_TRASH_DIR").filter(|d| !d.is_empty()).map(PathBuf::from);
        if let Some(r) = own.is_none().then(|| crate::platform::native_trash(path)).flatten() {
            match r {
                Ok(()) => {
                    self.log.push(format!("moved to trash: {}", path.display()));
                    let p = path.to_path_buf();
                    self.disk.retain(|k, _| !k.starts_with(&p));
                }
                Err(e) => self.warnings.push(format!("couldn't move {} to the trash: {e}", path.display())),
            }
            return;
        }
        let base = own.unwrap_or_else(|| {
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(crate::paths::data).join("Trash")
        });
        let (files, info) = (base.join("files"), base.join("info"));
        let _ = std::fs::create_dir_all(&files);
        let _ = std::fs::create_dir_all(&info);
        let stem = project::name_of(path).to_string();
        let mut name = stem.clone();
        let mut i = 1;
        while files.join(&name).exists() || info.join(format!("{name}.trashinfo")).exists() {
            i += 1;
            name = format!("{stem}.{i}");
        }
        let _ = std::fs::write(
            info.join(format!("{name}.trashinfo")),
            format!("[Trash Info]\nPath={}\nDeletionDate={}\n", path.display(), now_iso()),
        );
        let dest = files.join(&name);
        let moved = std::fs::rename(path, &dest).is_ok()
            || (copy_tree(path, &dest).is_ok() && {
                // Copied into the trash (another file system): now the original goes.
                if path.is_dir() {
                    std::fs::remove_dir_all(path).is_ok()
                } else {
                    std::fs::remove_file(path).is_ok()
                }
            });
        if moved {
            self.log.push(format!("moved to trash: {}", path.display()));
            let p = path.to_path_buf();
            self.disk.retain(|k, _| !k.starts_with(&p));
        } else {
            self.warnings
                .push(format!("couldn't move {} to the trash", path.display()));
        }
    }

    /// Edits since the last call, written to their files. Call in edit mode
    /// with the project's document.
    pub fn write_back(&mut self, doc: &mut Document) {
        if doc.id != self.doc_id {
            return;
        }
        let marks = match doc.marks_for(doc::Reader::Project) {
            Marks::Some(m) => m,
            // A rebuild (load, a whole-document change): nothing to write.
            Marks::All => return,
        };
        if marks.is_empty() {
            return;
        }
        let mut models: HashSet<PathBuf> = HashSet::new();
        let mut props: Vec<(Ref, String)> = Vec::new();
        let mut trees: Vec<Ref> = Vec::new();
        for m in marks {
            match m {
                doc::Mark::Prop(r, n) => props.push((r, n.to_string())),
                doc::Mark::Tree(r) => trees.push(r),
            }
        }
        // Structure first (creations / moves / removals), then properties.
        let mut seen = HashSet::new();
        for r in trees {
            if seen.insert(r) {
                self.tree_changed(doc, r, &mut models);
            }
        }
        let mut done = HashSet::new();
        for (r, n) in props {
            if done.insert((r, n.clone())) {
                self.prop_changed(doc, r, &n, &mut models);
            }
        }
        for m in models {
            self.write_model(doc, &m);
        }
    }

    fn prop_changed(&mut self, doc: &Document, r: Ref, prop: &str, models: &mut HashSet<PathBuf>) {
        let Some(origin) = self.map.get(&r).cloned() else {
            if doc.dom.get_by_ref(r).is_some() {
                self.place_dirty = true;
            }
            return;
        };
        let Some(inst) = doc.dom.get_by_ref(r) else { return };
        let class = inst.class.to_string();
        match &origin {
            Origin::InModel { file } | Origin::Model { file, .. } => {
                if prop == "Name" && matches!(origin, Origin::Model { .. }) {
                    self.rename(doc, r, &origin);
                }
                models.insert(file.clone());
            }
            Origin::File { file, kind, data } => {
                if prop == "Name" {
                    self.rename(doc, r, &origin);
                } else if held_by_file(*kind, prop) {
                    self.write_contents(doc, r, file, *kind);
                } else {
                    self.write_data_prop(doc, r, &class, prop, data);
                }
            }
            Origin::Dir { dir, init, data } => {
                if prop == "Name" {
                    self.rename(doc, r, &origin);
                } else if let Some((f, k)) = init.as_ref().filter(|(_, k)| held_by_file(*k, prop)) {
                    self.write_contents(doc, r, &f.clone(), *k);
                } else {
                    let _ = dir;
                    self.write_data_prop(doc, r, &class, prop, &data.clone());
                }
            }
            Origin::Node { project, json_path } => {
                self.write_node_prop(doc, r, &class, prop, project, json_path);
            }
        }
    }

    /// A script's Source, a StringValue's Value…: the file's own contents.
    fn write_contents(&mut self, doc: &Document, r: Ref, file: &Path, kind: FileKind) {
        let prop = match kind {
            FileKind::Txt => "Value",
            FileKind::Csv => return, // (LocalizationTable edits stay in the place)
            FileKind::JsonModule => return, // (a JSON module's source is generated)
            _ => "Source",
        };
        let text = match doc.prop(r, prop) {
            Some(Variant::String(s)) => s,
            _ => return,
        };
        let old = std::fs::read(file).unwrap_or_default();
        let bytes = with_endings(&text, crlf(&old));
        self.write(file, bytes);
    }

    /// One property into a data file (.meta.json / init.meta.json /
    /// .data.json): merged into what's there, keys kept in their order.
    fn write_data_prop(&mut self, doc: &Document, r: Ref, class: &str, prop: &str, data: &Path) {
        let value = doc.prop(r, prop);
        let mut json: serde_json::Value = std::fs::read(data)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let obj = match json.as_object_mut() {
            Some(o) => o,
            None => return,
        };
        let (key, val) = match prop {
            "Attributes" | "Tags" => (prop.to_ascii_lowercase(), value.as_ref().and_then(|v| attr_or_tags(v))),
            _ => ("properties".to_string(), None),
        };
        if key != "properties" {
            match val {
                Some(v) => {
                    obj.insert(key, v);
                }
                None => {
                    obj.remove(&key);
                }
            }
        } else {
            let props = obj
                .entry("properties")
                .or_insert_with(|| serde_json::json!({}));
            let Some(p) = props.as_object_mut() else { return };
            match value.as_ref().and_then(|v| to_json(class, prop, v)) {
                Some(j) => {
                    p.insert(prop.to_string(), j);
                }
                None => {
                    p.remove(prop);
                }
            }
        }
        let text = serde_json::to_string_pretty(&json).unwrap_or_default() + "\n";
        let old = std::fs::read(data).unwrap_or_default();
        self.write(data, with_endings(&text, crlf(&old)));
    }

    /// A project-file node's `$properties` (Place B keeps whole scripts there).
    fn write_node_prop(
        &mut self,
        doc: &Document,
        r: Ref,
        class: &str,
        prop: &str,
        project: &Path,
        json_path: &[String],
    ) {
        if prop == "Name" || json_path.is_empty() {
            return; // (renaming a project node would rewrite the tree's keys)
        }
        let Ok(bytes) = std::fs::read(project) else { return };
        let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return };
        let mut node = match json.get_mut("tree") {
            Some(n) => n,
            None => return,
        };
        for k in json_path {
            node = match node.get_mut(k) {
                Some(n) => n,
                None => return,
            };
        }
        let Some(obj) = node.as_object_mut() else { return };
        let props = obj
            .entry("$properties")
            .or_insert_with(|| serde_json::json!({}));
        let Some(p) = props.as_object_mut() else { return };
        match doc.prop(r, prop).and_then(|v| to_json(class, prop, &v)) {
            Some(j) => {
                p.insert(prop.to_string(), j);
            }
            None => {
                p.remove(prop);
            }
        }
        let text = to_json_like(&json, &bytes);
        self.write(project, text);
    }

    /// A model file rewritten from its instance (.rbxm / .rbxmx / .model.json).
    fn write_model(&mut self, doc: &Document, file: &Path) {
        let Some(root) = self
            .map
            .iter()
            .find(|(_, o)| matches!(o, Origin::Model { file: f, .. } if f == file))
            .map(|(r, _)| *r)
        else {
            return;
        };
        if doc.dom.get_by_ref(root).is_none() {
            return;
        }
        let name = project::name_of(file);
        let bytes = if name.ends_with(".model.json") {
            let j = json_model(doc, root, false);
            let old = std::fs::read(file).unwrap_or_default();
            with_endings(&(serde_json::to_string_pretty(&j).unwrap_or_default() + "\n"), crlf(&old))
        } else {
            let mut out = Vec::new();
            let ok = if name.ends_with(".rbxmx") {
                rbx_xml::to_writer_default(&mut out, &doc.dom, &[root]).is_ok()
            } else {
                rbx_binary::to_writer(&mut out, &doc.dom, &[root]).is_ok()
            };
            if !ok {
                return;
            }
            out
        };
        self.write(file, bytes);
    }

    /// The instance moved, was created or removed.
    fn tree_changed(&mut self, doc: &Document, r: Ref, models: &mut HashSet<PathBuf>) {
        let alive = doc.dom.get_by_ref(r).is_some();
        let origin = self.map.get(&r).cloned();
        if !alive {
            // Removed: its files go to the trash (a model's inside: rewrite).
            match origin {
                Some(Origin::InModel { file }) => {
                    models.insert(file);
                }
                Some(o) => {
                    self.remove_files(&o);
                    self.map.remove(&r);
                }
                None => self.place_dirty = true,
            }
            return;
        }
        let parent = doc.dom.get_by_ref(r).map(|i| i.parent()).unwrap_or_else(Ref::none);
        let container = self.container_dir(parent);
        // Inside a model file: that file changes.
        if let Some(Origin::InModel { file }) | Some(Origin::Model { file, .. }) = self.map.get(&parent).cloned() {
            if !matches!(origin, Some(Origin::Model { .. }) | Some(Origin::File { .. }) | Some(Origin::Dir { .. })) {
                self.map_into_model(doc, r, &file);
                models.insert(file);
                return;
            }
        }
        match (origin, container) {
            // Synced, still somewhere synced: moved?
            (Some(o), Some(dir)) => {
                if let Some(cur) = o.key().and_then(|k| k.parent()) {
                    if cur != dir && !matches!(o, Origin::Node { .. } | Origin::InModel { .. }) {
                        self.move_files(doc, r, &o, &dir);
                    }
                }
            }
            // Synced, moved out of the mapped paths: files to the trash.
            (Some(o), None) => {
                if !matches!(o, Origin::Node { .. }) {
                    self.remove_files(&o);
                    self.unmap(doc, r);
                    self.place_dirty = true;
                }
            }
            // New under a synced folder: new files.
            (None, Some(dir)) => self.create_files(doc, r, &dir),
            (None, None) => self.place_dirty = true,
        }
    }

    /// The folder an instance's children live in, when it maps to one.
    fn container_dir(&self, r: Ref) -> Option<PathBuf> {
        match self.map.get(&r)? {
            Origin::Dir { dir, .. } => Some(dir.clone()),
            _ => None,
        }
    }

    fn unmap(&mut self, doc: &Document, r: Ref) {
        for d in doc.dom.descendants_of(r) {
            self.map.remove(&d.referent());
        }
        self.map.remove(&r);
    }

    fn map_into_model(&mut self, doc: &Document, r: Ref, file: &Path) {
        for d in doc.dom.descendants_of(r) {
            self.map.insert(d.referent(), Origin::InModel { file: file.to_path_buf() });
        }
    }

    fn remove_files(&mut self, o: &Origin) {
        match o {
            Origin::File { file, data, .. } | Origin::Model { file, data, .. } => {
                self.to_trash(&file.clone());
                if data.is_file() {
                    self.to_trash(&data.clone());
                }
            }
            Origin::Dir { dir, .. } => self.to_trash(&dir.clone()),
            _ => {}
        }
    }

    /// New files for a new instance under a mapped folder: a script file
    /// (or a folder with init.* when it has children), a folder for a
    /// Folder, else a .model.json holding it and its descendants.
    fn create_files(&mut self, doc: &Document, r: Ref, dir: &Path) {
        let Some(inst) = doc.dom.get_by_ref(r) else { return };
        let class = inst.class.to_string();
        let (safe, renamed) = file_safe(&inst.name);
        let has_kids = !inst.children().is_empty();
        let lua = self.uses_lua_extension(dir);
        let free = |base: String, ext: &str| {
            let mut name = format!("{base}{ext}");
            let mut i = 2;
            while dir.join(&name).exists() {
                name = format!("{base}_{i}{ext}");
                i += 1;
            }
            dir.join(name)
        };
        // A Script with RunContext Client is a .client file (Rojo's
        // emitLegacyScripts = false reads it back that way).
        let kind = match script_kind(&class) {
            Some(FileKind::Server)
                if !self.config.emit_legacy_scripts
                    && matches!(inst.properties.get(&rbx_dom_weak::ustr("RunContext")), Some(Variant::Enum(e)) if e.to_u32() == 2) =>
            {
                Some(FileKind::Client)
            }
            k => k,
        };
        if let Some(kind) = kind {
            if has_kids {
                let d = free(safe.clone(), "");
                let init = d.join(format!("init{}", suffix_of(kind, lua)));
                let data = d.join("init.meta.json");
                self.mapped.push(d.clone());
                self.map.insert(r, Origin::Dir { dir: d.clone(), init: Some((init.clone(), kind)), data: data.clone() });
                self.write_contents(doc, r, &init, kind);
                if renamed {
                    self.write_original_name(&data, &inst.name);
                }
                for c in inst.children().to_vec() {
                    self.create_files(doc, c, &d);
                }
            } else {
                let f = free(safe.clone(), suffix_of(kind, lua));
                let data = data_path_for_file(&f, &project::classify(project::name_of(&f)).map(|x| x.0).unwrap_or(safe));
                self.mapped.push(f.clone());
                self.mapped.push(data.clone());
                self.map.insert(r, Origin::File { file: f.clone(), kind, data: data.clone() });
                self.write_contents(doc, r, &f, kind);
                if renamed {
                    self.write_original_name(&data, &inst.name);
                }
            }
        } else if class == "Folder" {
            let d = free(safe, "");
            let data = d.join("init.meta.json");
            self.mapped.push(d.clone());
            let _ = std::fs::create_dir_all(&d);
            self.map.insert(r, Origin::Dir { dir: d.clone(), init: None, data: data.clone() });
            self.log.push(format!("created {}", d.display()));
            if renamed {
                self.write_original_name(&data, &inst.name);
            }
            for c in inst.children().to_vec() {
                self.create_files(doc, c, &d);
            }
        } else {
            let f = free(safe.clone(), ".model.json");
            let data = data_path_for_file(&f, &safe);
            self.mapped.push(f.clone());
            self.map.insert(r, Origin::Model { file: f.clone(), kind: FileKind::JsonModel, data });
            self.map_into_model(doc, r, &f);
            self.map.insert(r, Origin::Model { file: f.clone(), kind: FileKind::JsonModel, data: f.with_extension("meta.json") });
            let j = json_model(doc, r, renamed);
            self.write(&f, (serde_json::to_string_pretty(&j).unwrap_or_default() + "\n").into_bytes());
        }
    }

    fn write_original_name(&mut self, data: &Path, name: &str) {
        let mut json: serde_json::Value = std::fs::read(data)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(o) = json.as_object_mut() {
            o.insert("originalName".into(), serde_json::json!(name));
        }
        self.mapped.push(data.to_path_buf());
        self.write(data, (serde_json::to_string_pretty(&json).unwrap_or_default() + "\n").into_bytes());
    }

    /// The project writes .lua when its folder already does (else .luau).
    fn uses_lua_extension(&self, dir: &Path) -> bool {
        use crate::project_config::LuaExt;
        match self.config.lua_ext {
            LuaExt::Lua => return true,
            LuaExt::Luau => return false,
            LuaExt::Auto => {}
        }
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten().any(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.ends_with(".lua")
                })
            })
            .unwrap_or(false)
    }

    /// Renames an instance's file or folder (keeping its extension).
    fn rename(&mut self, doc: &Document, r: Ref, o: &Origin) {
        let Some(name) = doc.dom.get_by_ref(r).map(|i| i.name.clone()) else { return };
        let (safe, _) = file_safe(&name);
        let (from, to) = match o {
            Origin::File { file, .. } | Origin::Model { file, .. } => {
                let old = project::name_of(file).to_string();
                let Some((stem, _)) = project::classify(&old) else { return };
                let ext = old[stem.len()..].to_string();
                (file.clone(), file.with_file_name(format!("{safe}{ext}")))
            }
            Origin::Dir { dir, .. } => (dir.clone(), dir.with_file_name(&safe)),
            _ => return,
        };
        if from == to || to.exists() || !self.writable(&from) || !self.writable(&to.parent().unwrap_or(Path::new("/")).join("")) {
            return;
        }
        if std::fs::rename(&from, &to).is_err() {
            self.warnings.push(format!("couldn't rename {} to {}", from.display(), to.display()));
            return;
        }
        self.log.push(format!("renamed {} → {}", from.display(), to.display()));
        // Its data file follows (X.meta.json → Y.meta.json).
        if let Origin::File { data, .. } | Origin::Model { data, .. } = o {
            if data.is_file() {
                let dn = project::name_of(data).to_string();
                let tail = if dn.ends_with(".data.json") { ".data.json" } else { ".meta.json" };
                let nd = data.with_file_name(format!("{safe}{tail}"));
                let _ = std::fs::rename(data, &nd);
            }
        }
        self.repath(&from, &to);
    }

    /// Moves an instance's files into another mapped folder.
    fn move_files(&mut self, _doc: &Document, _r: Ref, o: &Origin, dir: &Path) {
        let Some(from) = o.key().map(Path::to_path_buf) else { return };
        let to = dir.join(project::name_of(&from));
        if to.exists() || !self.writable(&from) || !self.writable(&to) {
            return;
        }
        if std::fs::rename(&from, &to).is_ok() {
            self.log.push(format!("moved {} → {}", from.display(), to.display()));
            if let Origin::File { data, .. } | Origin::Model { data, .. } = o {
                if data.is_file() {
                    let _ = std::fs::rename(data, dir.join(project::name_of(data)));
                }
            }
            self.repath(&from, &to);
        }
    }

    /// After a rename / move: every origin under `from` now lives under `to`.
    fn repath(&mut self, from: &Path, to: &Path) {
        let swap = |p: &PathBuf| -> PathBuf {
            match p.strip_prefix(from) {
                Ok(rest) if rest.as_os_str().is_empty() => to.to_path_buf(),
                Ok(rest) => to.join(rest),
                Err(_) => {
                    // The data file beside a renamed file.
                    if p.parent() == from.parent() {
                        let fname = project::name_of(from);
                        let stem = project::classify(fname).map(|x| x.0).unwrap_or_default();
                        let pn = project::name_of(p);
                        if !stem.is_empty() && pn.starts_with(&format!("{stem}.")) && project::is_data_file(pn) {
                            let tname = project::name_of(to);
                            let tstem = project::classify(tname).map(|x| x.0).unwrap_or_default();
                            return p.with_file_name(format!("{tstem}{}", &pn[stem.len()..]));
                        }
                    }
                    p.clone()
                }
            }
        };
        for o in self.map.values_mut() {
            match o {
                Origin::File { file, data, .. } | Origin::Model { file, data, .. } => {
                    *file = swap(file);
                    *data = swap(data);
                }
                Origin::Dir { dir, init, data } => {
                    *dir = swap(dir);
                    *data = swap(data);
                    if let Some((p, _)) = init {
                        *p = swap(p);
                    }
                }
                Origin::InModel { file } => *file = swap(file),
                Origin::Node { .. } => {}
            }
        }
        for m in self.mapped.iter_mut() {
            *m = swap(m);
        }
        let moved: Vec<(PathBuf, Vec<u8>)> = self
            .disk
            .iter()
            .filter(|(p, _)| p.starts_with(from))
            .map(|(p, b)| (swap(p), b.clone()))
            .collect();
        self.disk.retain(|p, _| !p.starts_with(from));
        self.disk.extend(moved);
    }
}

/// Attributes / Tags as data-file JSON.
fn attr_or_tags(v: &Variant) -> Option<serde_json::Value> {
    match v {
        Variant::Tags(t) => Some(serde_json::json!(t.iter().collect::<Vec<_>>())),
        Variant::Attributes(a) => {
            let mut m = serde_json::Map::new();
            for (k, x) in a.iter() {
                let j = match x {
                    Variant::Bool(b) => serde_json::json!(b),
                    Variant::Float64(f) => serde_json::json!(f),
                    Variant::String(s) => serde_json::json!(s),
                    other => match to_json("", "", other) {
                        Some(j) => j,
                        None => continue,
                    },
                };
                m.insert(k.clone(), j);
            }
            (!m.is_empty()).then_some(serde_json::Value::Object(m))
        }
        _ => None,
    }
}

/// A Rojo JSON model of an instance and its descendants.
fn json_model(doc: &Document, r: Ref, with_name: bool) -> serde_json::Value {
    let Some(i) = doc.dom.get_by_ref(r) else {
        return serde_json::json!({});
    };
    let class = i.class.to_string();
    let mut o = serde_json::Map::new();
    if with_name {
        o.insert("Name".into(), serde_json::json!(i.name));
    }
    o.insert("ClassName".into(), serde_json::json!(class));
    let mut props: Vec<(&str, serde_json::Value)> = i
        .properties
        .iter()
        .filter(|(k, _)| k.as_str() != "Attributes" && k.as_str() != "Tags")
        .filter_map(|(k, v)| to_json(&class, k, v).map(|j| (k.as_str(), j)))
        .collect();
    props.sort_by(|a, b| a.0.cmp(b.0));
    if !props.is_empty() {
        let mut p = serde_json::Map::new();
        for (k, v) in props {
            p.insert(k.to_string(), v);
        }
        o.insert("Properties".into(), serde_json::Value::Object(p));
    }
    for key in ["Attributes", "Tags"] {
        if let Some(j) = i.properties.get(&rbx_dom_weak::ustr(key)).and_then(attr_or_tags) {
            o.insert(key.into(), j);
        }
    }
    let kids: Vec<serde_json::Value> = i
        .children()
        .iter()
        .map(|c| json_model(doc, *c, true))
        .collect();
    if !kids.is_empty() {
        o.insert("Children".into(), serde_json::Value::Array(kids));
    }
    serde_json::Value::Object(o)
}

/// JSON text in the style of the file it replaces (indent width and
/// tabs vs spaces, trailing newline, line endings).
fn to_json_like(v: &serde_json::Value, old: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(old);
    let indent = text
        .lines()
        .skip(1)
        .find_map(|l| {
            let lead: String = l.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            (!lead.is_empty()).then_some(lead)
        })
        .unwrap_or_else(|| "  ".to_string());
    let mut out = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
    use serde::Serialize;
    let _ = v.serialize(&mut ser);
    let mut s = String::from_utf8(out).unwrap_or_default();
    if text.ends_with('\n') {
        s.push('\n');
    }
    with_endings(&s, crlf(old))
}
