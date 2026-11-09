//! Project folders: rustudio opens a game's Rojo / Argon project directly
//! (no server, no plugin). The `*.project.json` tree and the files it maps
//! are read the way Argon reads them (its default sync rules, in order —
//! argon-rbx/argon src/constants.rs — which are Rojo 7's conventions plus a
//! few of its own):
//!
//! | File | Instance |
//! |---|---|
//! | `X.server.luau` / `.server.lua` | Script X (RunContext Legacy) |
//! | `X.client.luau` / `.client.lua` | LocalScript X |
//! | `X.luau` / `.lua` | ModuleScript X |
//! | `X.txt` | StringValue (Value) |
//! | `X.csv` | LocalizationTable (Contents) |
//! | `X.json` (not model / meta / data / project) | ModuleScript returning the table |
//! | `X.model.json` | the instance tree it describes |
//! | `X.rbxm` / `X.rbxmx` | the model's instance |
//! | `X.project.json`, `dir/default.project.json` | a nested project |
//! | a directory | Folder, or the instance of its `init.*` file |
//! | `X.meta.json` / `X.data.json`, `dir/init.meta.json` / `dir/.data.json` | className, properties, attributes, tags, keepUnknowns (ignoreUnknownInstances), originalName for X / the directory |
//!
//! Instances outside the mapped paths come from the project's place file:
//! `<name>.rbxl(x)` beside the project file, else the only .rbxl / .rbxlx
//! there, else an empty DataModel.
//!
//! Files are the truth: a watcher (`notify`) applies changes on disk to the
//! open place (src/project_sync.rs), and edits in rustudio are written back
//! to the file each synced instance came from.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{ustr, InstanceBuilder, WeakDom};
use serde_json::Value as J;

use crate::doc::Document;
use crate::project_values::resolve;

/// Script, value and model file kinds (the middleware Argon picks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Server,
    Client,
    Module,
    Txt,
    Csv,
    JsonModule,
    JsonModel,
    Rbxm,
    Rbxmx,
    Project,
}

/// Where a synced instance comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    /// A node of a project file without `$path` (its `$properties` live in
    /// that project file).
    Node { project: PathBuf, json_path: Vec<String> },
    /// A file (`file`), its data file beside it (`data`, may not exist).
    File { file: PathBuf, kind: FileKind, data: PathBuf },
    /// A directory; `init` is its init file when the directory is a script
    /// or value; `data` its init.meta.json / .data.json.
    Dir { dir: PathBuf, init: Option<(PathBuf, FileKind)>, data: PathBuf },
    /// The root instance of a model file.
    Model { file: PathBuf, kind: FileKind, data: PathBuf },
    /// Inside a model file (the model root owns it).
    InModel { file: PathBuf },
}

impl Origin {
    /// The file or directory it is read from (its identity for reconciling).
    pub fn key(&self) -> Option<&Path> {
        match self {
            Origin::Node { .. } | Origin::InModel { .. } => None,
            Origin::File { file, .. } | Origin::Model { file, .. } => Some(file),
            Origin::Dir { dir, .. } => Some(dir),
        }
    }
}

/// An instance read from the project, before it joins the place.
pub struct Snap {
    pub name: String,
    pub class: String,
    pub props: Vec<(String, Variant)>,
    pub children: Vec<Snap>,
    pub origin: Option<Origin>,
    pub keep_unknowns: bool,
    /// Instances decoded from a .rbxm / .rbxmx (its root's children).
    pub dom: Option<(WeakDom, Vec<Ref>)>,
}

impl Snap {
    fn new(name: &str, class: &str) -> Self {
        Snap {
            name: name.to_string(),
            class: class.to_string(),
            props: Vec::new(),
            children: Vec::new(),
            origin: None,
            keep_unknowns: false,
            dom: None,
        }
    }
    fn set(&mut self, k: &str, v: Variant) {
        self.props.retain(|(n, _)| n != k);
        self.props.push((k.to_string(), v));
    }
}

/// Warnings while reading (Argon logs them and goes on).
pub type Warnings = Vec<String>;

/// A service by the reflection database, or one of the two player-script
/// containers Argon and Rojo also infer from the name (Argon util.rs).
pub fn is_service(name: &str) -> bool {
    crate::doc::db()
        .classes
        .get(name)
        .is_some_and(|c| c.tags.contains(&rbx_reflection::ClassTag::Service))
        || name == "StarterPlayerScripts"
        || name == "StarterCharacterScripts"
}

/// The project files in a folder (default.project.json first).
pub fn project_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && name_of(p).ends_with(".project.json"))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|p| (name_of(p) != "default.project.json", name_of(p).to_string()));
    v
}

pub fn name_of(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// The file rules, in Argon's order: (suffix, kind). `.meta.json` /
/// `.data.json` are data files, never instances.
const RULES: &[(&str, FileKind)] = &[
    (".project.json", FileKind::Project),
    (".server.luau", FileKind::Server),
    (".client.luau", FileKind::Client),
    (".luau", FileKind::Module),
    (".server.lua", FileKind::Server),
    (".client.lua", FileKind::Client),
    (".lua", FileKind::Module),
    (".txt", FileKind::Txt),
    (".csv", FileKind::Csv),
    (".model.json", FileKind::JsonModel),
    (".json", FileKind::JsonModule),
    (".rbxm", FileKind::Rbxm),
    (".rbxmx", FileKind::Rbxmx),
];

pub fn is_data_file(name: &str) -> bool {
    name.ends_with(".meta.json") || name.ends_with(".data.json")
}

/// (instance name, kind) of a file name, or None when no rule takes it.
pub fn classify(file_name: &str) -> Option<(String, FileKind)> {
    if is_data_file(file_name) {
        return None;
    }
    for (suffix, kind) in RULES {
        if let Some(stem) = file_name.strip_suffix(suffix) {
            if !stem.is_empty() {
                return Some((stem.to_string(), *kind));
            }
        }
    }
    None
}

/// The init file of a directory (its rule priority), if any.
fn init_file(dir: &Path) -> Option<(PathBuf, FileKind)> {
    const INITS: &[(&str, FileKind)] = &[
        ("default.project.json", FileKind::Project),
        ("init.server.luau", FileKind::Server),
        ("init.client.luau", FileKind::Client),
        ("init.luau", FileKind::Module),
        ("init.server.lua", FileKind::Server),
        ("init.client.lua", FileKind::Client),
        ("init.lua", FileKind::Module),
        ("init.txt", FileKind::Txt),
        ("init.csv", FileKind::Csv),
        ("init.json", FileKind::JsonModule),
        ("init.model.json", FileKind::JsonModel),
        ("init.rbxm", FileKind::Rbxm),
        ("init.rbxmx", FileKind::Rbxmx),
    ];
    INITS
        .iter()
        .map(|(n, k)| (dir.join(n), *k))
        .find(|(p, _)| p.is_file())
}

/// Reads one project folder tree.
pub struct Reader<'a> {
    pub ignore: &'a [glob::Pattern],
    pub root: &'a Path,
    pub warnings: Warnings,
    /// Bytes of every file read (the sync compares against them).
    pub read: HashMap<PathBuf, Vec<u8>>,
    /// Argon's defaults rather than Rojo's (see `argon_project`).
    pub argon: bool,
}

/// Whether a project is Argon's (an argon.toml beside it, or keys only Argon
/// writes): then a project node without `$path` keeps the place's unknown
/// children only when it is a service (Argon's project.rs), where Rojo keeps
/// them for every such node.
pub fn argon_project(file: &Path, json: &J) -> bool {
    let text = json.to_string();
    file.parent().is_some_and(|d| d.join("argon.toml").is_file())
        || text.contains("\"$keepUnknowns\"")
        || json.get("syncback").is_some()
        || json.get("ignoreGlobs").is_some()
}

impl<'a> Reader<'a> {
    pub fn new(root: &'a Path, ignore: &'a [glob::Pattern]) -> Self {
        Reader {
            ignore,
            root,
            warnings: Vec::new(),
            read: HashMap::new(),
            argon: false,
        }
    }

    fn ignored(&self, p: &Path) -> bool {
        let rel = p.strip_prefix(self.root).unwrap_or(p);
        let n = name_of(p);
        n == ".git" || n == ".DS_Store" || self.ignore.iter().any(|g| g.matches_path(rel))
    }

    fn bytes(&mut self, p: &Path) -> Option<Vec<u8>> {
        match std::fs::read(p) {
            Ok(b) => {
                self.read.insert(p.to_path_buf(), b.clone());
                Some(b)
            }
            Err(e) => {
                self.warnings.push(format!("{}: {e}", p.display()));
                None
            }
        }
    }

    fn text(&mut self, p: &Path) -> Option<String> {
        let b = self.bytes(p)?;
        match String::from_utf8(b) {
            Ok(s) => Some(s),
            Err(_) => {
                self.warnings.push(format!("{}: not UTF-8", p.display()));
                None
            }
        }
    }

    fn json(&mut self, p: &Path) -> Option<J> {
        let t = self.text(p)?;
        if t.trim().is_empty() {
            return Some(J::Object(Default::default()));
        }
        match serde_json::from_str(&t) {
            Ok(v) => Some(v),
            Err(e) => {
                self.warnings.push(format!("{}: {e}", p.display()));
                None
            }
        }
    }

    /// Anything at `path` (file or directory), named `name` unless the
    /// path gives the name.
    pub fn snapshot(&mut self, path: &Path) -> Option<Snap> {
        if self.ignored(path) || !path.exists() {
            return None;
        }
        if path.is_dir() {
            return Some(self.dir(path));
        }
        let (name, kind) = classify(name_of(path))?;
        let data = data_path_for_file(path, &name);
        let mut s = self.file(path, &name, kind)?;
        self.apply_data(&mut s, &data, false);
        if s.origin.is_none() {
            s.origin = Some(match kind {
                FileKind::JsonModel | FileKind::Rbxm | FileKind::Rbxmx => Origin::Model {
                    file: path.to_path_buf(),
                    kind,
                    data,
                },
                _ => Origin::File {
                    file: path.to_path_buf(),
                    kind,
                    data,
                },
            });
        }
        Some(s)
    }

    fn dir(&mut self, dir: &Path) -> Snap {
        let name = name_of(dir).to_string();
        let init = init_file(dir);
        let data = [dir.join("init.meta.json"), dir.join(".data.json")]
            .into_iter()
            .find(|p| p.is_file())
            .unwrap_or_else(|| dir.join("init.meta.json"));
        let mut s = match &init {
            Some((p, FileKind::Project)) => {
                // A nested project replaces the directory.
                let mut s = match self.project_file(p) {
                    Some(s) => s,
                    None => Snap::new(&name, "Folder"),
                };
                s.name = name;
                return s;
            }
            Some((p, kind)) => {
                let mut s = self.file(p, &name, *kind).unwrap_or_else(|| Snap::new(&name, "Folder"));
                s.name = name.clone();
                s
            }
            None => Snap::new(&name, "Folder"),
        };
        s.origin = Some(Origin::Dir {
            dir: dir.to_path_buf(),
            init: init.clone(),
            data: data.clone(),
        });
        // Children: every entry but the init file and data files.
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        entries.sort();
        for e in entries {
            if init.as_ref().is_some_and(|(p, _)| *p == e) || is_data_file(name_of(&e)) {
                continue;
            }
            if let Some(c) = self.snapshot(&e) {
                s.children.push(c);
            }
        }
        self.apply_data(&mut s, &data, init.is_none());
        s
    }

    fn file(&mut self, p: &Path, name: &str, kind: FileKind) -> Option<Snap> {
        Some(match kind {
            FileKind::Server | FileKind::Client | FileKind::Module => {
                let src = self.text(p)?;
                let class = match kind {
                    FileKind::Server => "Script",
                    FileKind::Client => "LocalScript",
                    _ => "ModuleScript",
                };
                let mut s = Snap::new(name, class);
                s.set("Source", Variant::String(src));
                s
            }
            FileKind::Txt => {
                let t = self.text(p)?;
                let mut s = Snap::new(name, "StringValue");
                s.set("Value", Variant::String(t));
                s
            }
            FileKind::Csv => {
                let t = self.text(p)?;
                let mut s = Snap::new(name, "LocalizationTable");
                s.set("Contents", Variant::String(csv_to_contents(&t)));
                s
            }
            FileKind::JsonModule => {
                let v = self.json(p)?;
                let mut s = Snap::new(name, "ModuleScript");
                s.set("Source", Variant::String(format!("return {}", json_to_luau(&v, 0))));
                s
            }
            FileKind::JsonModel => {
                let v = self.json(p)?;
                let mut s = self.json_model(&v, name, p);
                s.name = v
                    .get("Name")
                    .or_else(|| v.get("name"))
                    .and_then(|n| n.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| name.to_string());
                s
            }
            FileKind::Rbxm | FileKind::Rbxmx => {
                let b = self.bytes(p)?;
                let dom = match Document::decode(&b) {
                    Ok(d) => d,
                    Err(e) => {
                        self.warnings.push(format!("{}: {e}", p.display()));
                        return None;
                    }
                };
                let kids = dom.root().children().to_vec();
                let class = match kids.as_slice() {
                    [one] => dom.get_by_ref(*one).map(|i| i.class.to_string()),
                    _ => None,
                }
                .unwrap_or_else(|| "Folder".to_string());
                let mut s = Snap::new(name, &class);
                s.dom = Some((dom, kids));
                s
            }
            FileKind::Project => {
                let mut s = self.project_file(p)?;
                if s.name.is_empty() {
                    s.name = name.to_string();
                }
                s
            }
        })
    }

    fn json_model(&mut self, v: &J, name: &str, file: &Path) -> Snap {
        let get = |k: &str, alt: &str| v.get(k).or_else(|| v.get(alt));
        let class = get("ClassName", "className")
            .and_then(|c| c.as_str())
            .unwrap_or("Folder")
            .to_string();
        let mut s = Snap::new(name, &class);
        if let Some(props) = get("Properties", "properties").and_then(|p| p.as_object()) {
            for (k, x) in props {
                match resolve(&class, k, x) {
                    Ok((n, val)) => s.set(&n, val),
                    Err(e) => self.warnings.push(format!("{}: {e}", file.display())),
                }
            }
        }
        if let Some(a) = get("Attributes", "attributes") {
            if let Ok((n, val)) = resolve(&class, "Attributes", &serde_json::json!({ "Attributes": a })) {
                s.set(&n, val);
            }
        }
        if let Some(t) = get("Tags", "tags") {
            if let Ok((n, val)) = resolve(&class, "Tags", &serde_json::json!({ "Tags": t })) {
                s.set(&n, val);
            }
        }
        for c in get("Children", "children").and_then(|c| c.as_array()).into_iter().flatten() {
            let cname = c
                .get("Name")
                .or_else(|| c.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("Instance")
                .to_string();
            let mut cs = self.json_model(c, &cname, file);
            cs.origin = Some(Origin::InModel {
                file: file.to_path_buf(),
            });
            s.children.push(cs);
        }
        s
    }

    /// `.meta.json` / `.data.json`: className (directories only), properties,
    /// attributes, tags, keepUnknowns / ignoreUnknownInstances, originalName.
    fn apply_data(&mut self, s: &mut Snap, data: &Path, may_set_class: bool) {
        if !data.is_file() {
            return;
        }
        let Some(v) = self.json(data) else { return };
        if may_set_class {
            if let Some(c) = v.get("className").and_then(|c| c.as_str()) {
                s.class = c.to_string();
            } else if is_service(&s.name) {
                s.class = s.name.clone();
            }
        }
        if let Some(props) = v.get("properties").and_then(|p| p.as_object()) {
            for (k, x) in props {
                match resolve(&s.class, k, x) {
                    Ok((n, val)) => s.set(&n, val),
                    Err(e) => self.warnings.push(format!("{}: {e}", data.display())),
                }
            }
        }
        for (key, ty) in [("attributes", "Attributes"), ("tags", "Tags")] {
            if let Some(a) = v.get(key) {
                match resolve(&s.class, ty, &serde_json::json!({ ty: a })) {
                    Ok((n, val)) => s.set(&n, val),
                    Err(e) => self.warnings.push(format!("{}: {e}", data.display())),
                }
            }
        }
        if let Some(k) = v
            .get("keepUnknowns")
            .or_else(|| v.get("ignoreUnknownInstances"))
            .and_then(|k| k.as_bool())
        {
            s.keep_unknowns = k;
        }
        if let Some(n) = v.get("originalName").and_then(|n| n.as_str()) {
            s.name = n.to_string();
        }
    }

    /// A project file as one instance (its tree's root).
    pub fn project_file(&mut self, p: &Path) -> Option<Snap> {
        let v = self.json(p)?;
        let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let tree = v.get("tree")?;
        let outer = self.argon;
        self.argon = argon_project(p, &v);
        let s = self.node(&name, tree, p, &mut Vec::new());
        self.argon = outer || self.argon;
        Some(s)
    }

    /// A project node: `$className`, `$path`, `$properties`, `$attributes`,
    /// `$tags`, `$keepUnknowns` / `$ignoreUnknownInstances`, children.
    pub fn node(&mut self, name: &str, n: &J, project: &Path, json_path: &mut Vec<String>) -> Snap {
        let explicit = n.get("$className").and_then(|c| c.as_str());
        let class = explicit
            .map(String::from)
            .unwrap_or_else(|| if is_service(name) { name.to_string() } else { "Folder".to_string() });
        let mut s = Snap::new(name, &class);
        s.origin = Some(Origin::Node {
            project: project.to_path_buf(),
            json_path: json_path.clone(),
        });
        if let Some(path) = n.get("$path").and_then(|p| p.as_str().or_else(|| p.get("optional").and_then(|o| o.as_str()))) {
            let full = clean(&project.parent().unwrap_or(Path::new(".")).join(path));
            match self.snapshot(&full) {
                Some(mut ps) => {
                    ps.name = name.to_string();
                    if ps.class == "Folder" {
                        ps.class = class.clone();
                    }
                    s = ps;
                }
                None if n.get("$path").is_some_and(|p| p.is_string()) && !full.exists() => {
                    self.warnings
                        .push(format!("{}: $path {} doesn't exist", project.display(), full.display()));
                }
                None => {}
            }
        }
        if let Some(props) = n.get("$properties").and_then(|p| p.as_object()) {
            for (k, x) in props {
                match resolve(&s.class, k, x) {
                    Ok((nm, val)) => s.set(&nm, val),
                    Err(e) => self.warnings.push(format!("{}: {e}", project.display())),
                }
            }
        }
        for (key, ty) in [("$attributes", "Attributes"), ("$tags", "Tags")] {
            if let Some(a) = n.get(key) {
                if let Ok((nm, val)) = resolve(&s.class, ty, &serde_json::json!({ ty: a })) {
                    s.set(&nm, val);
                }
            }
        }
        let keep = n
            .get("$keepUnknowns")
            .or_else(|| n.get("$ignoreUnknownInstances"))
            .and_then(|k| k.as_bool());
        // Unknown instances (the place's own children): Argon keeps them for
        // services only, Rojo for every node without $path; with $path, as
        // the path's data file says (Argon also keeps a service's).
        s.keep_unknowns = keep.unwrap_or(if n.get("$path").is_some() {
            s.keep_unknowns || is_service(&s.class)
        } else if self.argon {
            is_service(&s.class)
        } else {
            true
        });
        if let Some(obj) = n.as_object() {
            for (k, child) in obj {
                if k.starts_with('$') || !child.is_object() {
                    continue;
                }
                json_path.push(k.clone());
                let c = self.node(k, child, project, json_path);
                json_path.pop();
                s.children.retain(|x| x.name != *k);
                s.children.push(c);
            }
        }
        s
    }
}

/// `dir/../x` → `x` without touching the disk.
pub fn clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The data file beside a file: `X.meta.json` (or an existing `X.data.json`).
pub fn data_path_for_file(file: &Path, name: &str) -> PathBuf {
    let data = file.with_file_name(format!("{name}.data.json"));
    if data.is_file() {
        return data;
    }
    file.with_file_name(format!("{name}.meta.json"))
}

/// A Luau table literal for a JSON value (Rojo / Argon JSON modules).
pub fn json_to_luau(v: &J, depth: usize) -> String {
    let pad = "\t".repeat(depth + 1);
    let end = "\t".repeat(depth);
    match v {
        J::Null => "nil".into(),
        J::Bool(b) => b.to_string(),
        J::Number(n) => n.to_string(),
        J::String(s) => format!("{s:?}"),
        J::Array(a) if a.is_empty() => format!("{{\n{end}}}"),
        J::Array(a) => {
            let items: Vec<String> = a.iter().map(|x| format!("{pad}{},", json_to_luau(x, depth + 1))).collect();
            format!("{{\n{}\n{end}}}", items.join("\n"))
        }
        J::Object(o) if o.is_empty() => format!("{{\n{end}}}"),
        J::Object(o) => {
            let items: Vec<String> = o
                .iter()
                // Argon's json2lua writes every key bracketed: ["key"] = value.
                .map(|(k, x)| format!("{pad}[{k:?}] = {},", json_to_luau(x, depth + 1)))
                .collect();
            format!("{{\n{}\n{end}}}", items.join("\n"))
        }
    }
}

/// Roblox's localization CSV (Key, Source, Context, Example, then a column
/// per locale) as LocalizationTable.Contents JSON.
pub fn csv_to_contents(text: &str) -> String {
    let rows = parse_csv(text);
    let Some(header) = rows.first() else {
        return "[]".into();
    };
    let col = |n: &str| header.iter().position(|h| h.eq_ignore_ascii_case(n));
    let (key, source, context, example) = (col("Key"), col("Source"), col("Context"), col("Example"));
    let mut out = Vec::new();
    for r in rows.iter().skip(1) {
        let get = |i: Option<usize>| i.and_then(|i| r.get(i)).cloned().unwrap_or_default();
        let mut values = BTreeMap::new();
        for (i, h) in header.iter().enumerate() {
            if Some(i) != key && Some(i) != source && Some(i) != context && Some(i) != example {
                if let Some(v) = r.get(i).filter(|v| !v.is_empty()) {
                    values.insert(h.clone(), v.clone());
                }
            }
        }
        let mut e = serde_json::Map::new();
        for (k, v) in [("key", get(key)), ("context", get(context)), ("examples", get(example)), ("source", get(source))] {
            if !v.is_empty() {
                e.insert(k.into(), J::String(v));
            }
        }
        e.insert("values".into(), serde_json::to_value(values).unwrap_or_default());
        out.push(J::Object(e));
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => cell.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// An instance builder for a snapshot (no children).
pub fn builder(s: &Snap) -> InstanceBuilder {
    let mut b = InstanceBuilder::new(s.class.as_str()).with_name(s.name.clone());
    for (k, v) in &s.props {
        b.add_property(ustr(k), v.clone());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_kinds() {
        assert_eq!(classify("Foo.server.luau"), Some(("Foo".into(), FileKind::Server)));
        assert_eq!(classify("Foo.client.lua"), Some(("Foo".into(), FileKind::Client)));
        assert_eq!(classify("Foo.spec.luau"), Some(("Foo.spec".into(), FileKind::Module)));
        assert_eq!(classify("Map.model.json"), Some(("Map".into(), FileKind::JsonModel)));
        assert_eq!(classify("data.json"), Some(("data".into(), FileKind::JsonModule)));
        assert_eq!(classify("Foo.meta.json"), None);
        assert_eq!(classify(".gitkeep"), None);
        assert_eq!(classify("Nested.project.json"), Some(("Nested".into(), FileKind::Project)));
    }

    #[test]
    fn json_module_and_csv() {
        let v: J = serde_json::json!({"a": 1, "b c": [true, "x"]});
        let l = json_to_luau(&v, 0);
        assert!(l.contains("[\"a\"] = 1") && l.contains("[\"b c\"] = {"), "{l}");
        let c = csv_to_contents("Key,Source,Context,Example,es\nhi,Hello,,,Hola\n");
        assert!(c.contains("\"key\":\"hi\"") && c.contains("\"es\":\"Hola\""), "{c}");
    }

    #[test]
    fn values() {
        let (_, v) = resolve("Part", "Size", &serde_json::json!([4, 1, 2])).unwrap();
        assert!(matches!(v, Variant::Vector3(_)));
        let (_, v) = resolve("Part", "Material", &serde_json::json!("Fabric")).unwrap();
        assert!(matches!(v, Variant::Enum(_)));
        let (_, v) = resolve("Part", "Color", &serde_json::json!([80, 22, 30])).unwrap();
        assert!(matches!(v, Variant::Color3uint8(c) if c.r == 80));
        let (_, v) = resolve("Part", "Whatever", &serde_json::json!({"Vector3": [1, 2, 3]})).unwrap();
        assert!(matches!(v, Variant::Vector3(_)));
        let (_, v) = resolve("Folder", "Attributes", &serde_json::json!({"Attributes": {"x": 1, "y": "s"}})).unwrap();
        assert!(matches!(v, Variant::Attributes(_)));
    }
}
