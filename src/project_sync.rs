//! An open project folder (src/project.rs reads it): the place it builds,
//! which instance came from which file, the watcher that brings changes on
//! disk in, and the write-back of edits made in rustudio.
//!
//! Rules kept here:
//! - only edits to synced instances are written, each to its own file, in
//!   place (same name, extension and line endings); nothing outside the
//!   project's mapped paths (and its project files) is ever written;
//! - a file is written only when its bytes would change, so opening a
//!   project and saving it untouched changes no file;
//! - changes on disk reach the open place within the watcher's debounce
//!   (mech::PROJECT_DEBOUNCE_MS) in edit mode; during Play they wait and are
//!   applied when Play stops (so the next Play has them);
//! - removed files go to the desktop trash (XDG), never deleted outright.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use rbx_dom_weak::types::Ref;
use rbx_dom_weak::ustr;

use crate::doc::{self, Document};
use crate::project::{self, Origin, Reader, Snap};

#[path = "project_write.rs"]
mod write;

pub struct ProjectState {
    /// The project file and its folder.
    pub file: PathBuf,
    pub root: PathBuf,
    pub name: String,
    /// Where instances outside the mapped paths are saved.
    pub place: Option<PathBuf>,
    ignore: Vec<glob::Pattern>,
    /// The project's mapped paths (writes never leave them).
    pub mapped: Vec<PathBuf>,
    /// Synced instance → where it came from.
    pub map: HashMap<Ref, Origin>,
    /// File bytes as last read or written (what's on disk, as far as we know).
    pub disk: HashMap<PathBuf, Vec<u8>>,
    /// Which document the map belongs to (Play swaps documents).
    pub doc_id: u64,
    watcher: Option<notify::RecommendedWatcher>,
    events: Option<mpsc::Receiver<notify::Result<notify::Event>>>,
    /// Changes seen on disk, applied once quiet for the debounce.
    pending: HashSet<PathBuf>,
    pending_since: Option<std::time::Instant>,
    /// A non-synced instance changed since load / save (Ctrl+S writes the place).
    pub place_dirty: bool,
    /// Files changed during Play: applied when Play stops.
    pub reapply_after_play: bool,
    pub warnings: Vec<String>,
    /// Files written / trashed (the self-test and Output read it).
    pub log: Vec<String>,
}

/// Opens a project file: the place plus the project's tree.
pub fn open(file: &Path) -> Result<(Document, ProjectState), String> {
    let file = std::fs::canonicalize(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let root = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let pj: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let name = pj.get("name").and_then(|n| n.as_str()).unwrap_or("project").to_string();
    // Ignore globs: Rojo's globIgnorePaths, Argon's ignoreGlobs.
    let ignore: Vec<glob::Pattern> = ["globIgnorePaths", "ignoreGlobs"]
        .iter()
        .filter_map(|k| pj.get(*k).and_then(|v| v.as_array()))
        .flatten()
        .filter_map(|g| g.as_str().and_then(|g| glob::Pattern::new(g).ok()))
        .collect();
    let place = find_place(&root, &name);
    let mut doc = match &place {
        Some(p) => Document::open(p)?,
        None => {
            let mut d = Document::empty();
            d.ensure_services();
            d
        }
    };
    doc.title = format!("{name} (project)");
    let mut reader = Reader::new(&root, &ignore);
    let tree = reader.project_file(&file).ok_or("the project file has no tree")?;
    let mut st = ProjectState {
        file: file.clone(),
        root: root.clone(),
        name,
        place: place.clone(),
        ignore: ignore.clone(),
        mapped: Vec::new(),
        map: HashMap::new(),
        disk: HashMap::new(),
        doc_id: doc.id,
        watcher: None,
        events: None,
        pending: HashSet::new(),
        pending_since: None,
        place_dirty: false,
        reapply_after_play: false,
        warnings: std::mem::take(&mut reader.warnings),
        log: Vec::new(),
    };
    st.disk = std::mem::take(&mut reader.read);
    st.disk.insert(file.clone(), text.into_bytes());
    collect_mapped(&tree, &mut st.mapped);
    // The tree's root is the DataModel: its children go over the place's.
    let dm = doc.dom.root_ref();
    for c in tree.children {
        st.apply(&mut doc, dm, c);
    }
    st.map.insert(dm, tree.origin.unwrap_or(Origin::Node { project: file, json_path: Vec::new() }));
    doc.mark_all();
    // The project reader starts from here: what loading did isn't an edit.
    let _ = doc.marks_for(doc::Reader::Project);
    st.watch();
    Ok((doc, st))
}

/// `<name>.rbxl(x)` beside the project file, else the only place file there.
fn find_place(root: &Path, name: &str) -> Option<PathBuf> {
    for ext in ["rbxl", "rbxlx"] {
        let p = root.join(format!("{name}.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    let places: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|x| x == "rbxl" || x == "rbxlx")
        })
        .collect();
    (places.len() == 1).then(|| places[0].clone())
}

fn collect_mapped(s: &Snap, out: &mut Vec<PathBuf>) {
    match &s.origin {
        Some(Origin::Dir { dir, .. }) => {
            out.push(dir.clone());
            return;
        }
        Some(Origin::File { file, data, .. }) | Some(Origin::Model { file, data, .. }) => {
            out.push(file.clone());
            out.push(data.clone());
            return;
        }
        _ => {}
    }
    for c in &s.children {
        collect_mapped(c, out);
    }
}

impl ProjectState {
    /// Whether a path is inside the project's mapped paths (or is one of its
    /// project files): the only places rustudio writes.
    pub fn writable(&self, p: &Path) -> bool {
        p == self.file || self.mapped.iter().any(|m| p.starts_with(m))
    }

    /// Puts a snapshot under `parent`, replacing a same-named child the place
    /// already has (files are the truth), and records where it came from.
    fn apply(&mut self, doc: &mut Document, parent: Ref, s: Snap) -> Ref {
        // Only a place-file instance can be taken over: one already loaded
        // from a file is a same-named sibling (Argon keeps those apart with
        // UUID-suffixed folders and originalName).
        let existing = doc.dom.get_by_ref(parent).and_then(|p| {
            p.children().iter().copied().find(|c| {
                !self.map.contains_key(c) && doc.dom.get_by_ref(*c).is_some_and(|i| i.name == s.name)
            })
        });
        // A project node without $path over an instance of the same class
        // (a service, or a folder the place has): kept, with its unknown
        // children when the node says so.
        let keep_existing = existing.filter(|e| {
            doc.dom.get_by_ref(*e).is_some_and(|i| i.class == s.class.as_str()) && s.dom.is_none()
        });
        let r = match keep_existing {
            Some(e) => {
                if let Some(i) = doc.dom.get_by_ref_mut(e) {
                    for (k, v) in &s.props {
                        i.properties.insert(ustr(k), v.clone());
                    }
                }
                if !s.keep_unknowns {
                    let kids: Vec<Ref> = doc.dom.get_by_ref(e).map(|i| i.children().to_vec()).unwrap_or_default();
                    for k in kids {
                        doc.dom.destroy(k);
                    }
                }
                e
            }
            None => {
                if let Some(e) = existing {
                    doc.dom.destroy(e);
                }
                // A new instance: built with its whole subtree.
                let r = self.build(doc, parent, &s);
                if let Some(o) = s.origin.clone() {
                    self.map.insert(r, o);
                }
                return r;
            }
        };
        if let Some(o) = s.origin.clone() {
            self.map.insert(r, o);
        }
        if s.dom.is_none() {
            for c in s.children {
                self.apply(doc, r, c);
            }
        }
        r
    }

    /// A new instance (and its subtree) for a snapshot.
    fn build(&mut self, doc: &mut Document, parent: Ref, s: &Snap) -> Ref {
        if let Some((dom, kids)) = &s.dom {
            // A model file: its instance (or a Folder of several), named after the file.
            let mut tmp = rbx_dom_weak::WeakDom::new(rbx_dom_weak::InstanceBuilder::new("Folder"));
            let copies = dom.clone_multiple_into_external(kids, &mut tmp);
            let (root, inner) = if copies.len() == 1 {
                (copies[0], Vec::new())
            } else {
                (tmp.root_ref(), copies)
            };
            let r = if inner.is_empty() {
                let c = dom_clone(&tmp, root, doc, parent);
                if let Some(i) = doc.dom.get_by_ref_mut(c) {
                    i.name = s.name.clone();
                }
                c
            } else {
                let f = doc.dom.insert(parent, rbx_dom_weak::InstanceBuilder::new("Folder").with_name(s.name.clone()));
                for k in inner {
                    dom_clone(&tmp, k, doc, f);
                }
                f
            };
            if let Some(i) = doc.dom.get_by_ref_mut(r) {
                for (k, v) in &s.props {
                    i.properties.insert(ustr(k), v.clone());
                }
            }
            let file = s.origin.as_ref().and_then(|o| o.key()).map(Path::to_path_buf);
            if let Some(file) = file {
                let inside: Vec<Ref> = doc.dom.descendants_of(r).map(|i| i.referent()).filter(|x| *x != r).collect();
                for d in inside {
                    self.map.insert(d, Origin::InModel { file: file.clone() });
                }
            }
            return r;
        }
        let r = doc.dom.insert(parent, project::builder(s));
        for c in &s.children {
            let cr = self.build(doc, r, c);
            if let Some(o) = c.origin.clone() {
                self.map.insert(cr, o);
            }
        }
        r
    }
}

/// Copies `r` (with its subtree) from `from` into the document under `parent`.
fn dom_clone(from: &rbx_dom_weak::WeakDom, r: Ref, doc: &mut Document, parent: Ref) -> Ref {
    let c = from.clone_into_external(r, &mut doc.dom);
    doc.dom.transfer_within(c, parent);
    c
}

impl ProjectState {
    /// Starts watching the project folder.
    fn watch(&mut self) {
        use notify::Watcher;
        let (tx, rx) = mpsc::channel();
        match notify::recommended_watcher(tx) {
            Ok(mut w) => {
                if let Err(e) = w.watch(&self.root, notify::RecursiveMode::Recursive) {
                    self.warnings.push(format!("can't watch {}: {e}", self.root.display()));
                    return;
                }
                self.watcher = Some(w);
                self.events = Some(rx);
            }
            Err(e) => self.warnings.push(format!("file watcher unavailable: {e}")),
        }
    }

    /// Paths changed on disk since the last call that are really different
    /// from what rustudio last read or wrote (its own writes come back as
    /// events and are dropped here).
    pub fn poll_disk(&mut self) -> Option<HashSet<PathBuf>> {
        if let Some(rx) = &self.events {
            while let Ok(ev) = rx.try_recv() {
                let Ok(ev) = ev else { continue };
                if matches!(ev.kind, notify::EventKind::Access(_)) {
                    continue;
                }
                for p in ev.paths {
                    if p.components().any(|c| c.as_os_str() == ".git") {
                        continue;
                    }
                    self.pending.insert(p);
                    self.pending_since = Some(std::time::Instant::now());
                }
            }
        }
        let since = self.pending_since?;
        if since.elapsed().as_millis() < crate::mech::PROJECT_DEBOUNCE_MS as u128 {
            return None;
        }
        self.pending_since = None;
        let changed: HashSet<PathBuf> = std::mem::take(&mut self.pending)
            .into_iter()
            .filter(|p| {
                let now = std::fs::read(p).ok();
                let known = self.disk.get(p);
                match (now, known) {
                    (Some(a), Some(b)) => a != *b,
                    (None, None) => p.is_dir() || !p.exists(),
                    _ => true,
                }
            })
            .collect();
        (!changed.is_empty()).then_some(changed)
    }
}

// ------------------------------------------------------------ disk → place

impl ProjectState {
    /// Re-reads the project and brings the place in line with the files,
    /// keeping the instances that still match their file (so selection, open
    /// script tabs and references survive). Returns how many instances
    /// changed.
    pub fn reconcile(&mut self, doc: &mut Document) -> usize {
        let ignore = self.ignore.clone();
        let root = self.root.clone();
        let mut reader = Reader::new(&root, &ignore);
        let Some(tree) = reader.project_file(&self.file) else {
            return 0;
        };
        let old_disk = std::mem::take(&mut self.disk);
        self.disk = std::mem::take(&mut reader.read);
        if let Ok(b) = std::fs::read(&self.file) {
            self.disk.insert(self.file.clone(), b);
        }
        self.warnings.extend(reader.warnings);
        self.mapped.clear();
        collect_mapped(&tree, &mut self.mapped);
        let mut n = 0;
        let dm = doc.dom.root_ref();
        for c in tree.children {
            let existing = doc.dom.get_by_ref(dm).and_then(|p| {
                p.children()
                    .iter()
                    .copied()
                    .find(|x| doc.dom.get_by_ref(*x).is_some_and(|i| i.name == c.name))
            });
            match existing {
                Some(e) => n += self.reconcile_one(doc, e, c, &old_disk),
                None => {
                    let r = self.build(doc, dm, &c);
                    if let Some(o) = c.origin.clone() {
                        self.map.insert(r, o);
                    }
                    doc.mark_tree(r);
                    n += 1;
                }
            }
        }
        // What reconciling did isn't an edit to write back.
        let _ = doc.marks_for(doc::Reader::Project);
        n
    }

    fn reconcile_one(
        &mut self,
        doc: &mut Document,
        r: Ref,
        s: Snap,
        old_disk: &HashMap<PathBuf, Vec<u8>>,
    ) -> usize {
        let parent = doc.dom.get_by_ref(r).map(|i| i.parent()).unwrap_or_else(Ref::none);
        let class_same = doc.dom.get_by_ref(r).is_some_and(|i| i.class == s.class.as_str());
        // A model file that changed (or a class change): rebuilt whole.
        let is_model = matches!(s.origin, Some(Origin::Model { .. }));
        let model_changed = is_model
            && s.origin
                .as_ref()
                .and_then(|o| o.key())
                .is_some_and(|k| old_disk.get(k) != self.disk.get(k));
        if !class_same || model_changed {
            self.unmap_subtree(doc, r);
            doc.dom.destroy(r);
            doc.mark_tree(parent);
            let nr = self.build(doc, parent, &s);
            if let Some(o) = s.origin.clone() {
                self.map.insert(nr, o);
            }
            doc.mark_tree(nr);
            return 1;
        }
        let mut n = 0;
        if let Some(o) = s.origin.clone() {
            self.map.insert(r, o);
        }
        if doc.dom.get_by_ref(r).is_some_and(|i| i.name != s.name) {
            if let Some(i) = doc.dom.get_by_ref_mut(r) {
                i.name = s.name.clone();
            }
            doc.mark_prop(r, ustr("Name"));
            n += 1;
        }
        for (k, v) in &s.props {
            let same = doc.dom.get_by_ref(r).and_then(|i| i.properties.get(&ustr(k))) == Some(v);
            if !same {
                if let Some(i) = doc.dom.get_by_ref_mut(r) {
                    i.properties.insert(ustr(k), v.clone());
                }
                doc.mark_prop(r, ustr(k));
                n += 1;
            }
        }
        // A model file's insides are rebuilt only when the file changed.
        if is_model {
            return n;
        }
        // Children: synced ones matched by their file (or project node path).
        let kids: Vec<Ref> = doc.dom.get_by_ref(r).map(|i| i.children().to_vec()).unwrap_or_default();
        let mut used: HashSet<Ref> = HashSet::new();
        for c in s.children {
            let found = kids.iter().copied().find(|k| {
                !used.contains(k)
                    && match (self.map.get(k), &c.origin) {
                        (
                            Some(Origin::Node { json_path: a, .. }),
                            Some(Origin::Node { json_path: b, .. }),
                        ) => a == b,
                        (Some(o), Some(co)) => o.key().is_some() && o.key() == co.key(),
                        _ => false,
                    }
            });
            match found {
                Some(k) => {
                    used.insert(k);
                    n += self.reconcile_one(doc, k, c, old_disk);
                }
                None => {
                    let nr = self.build(doc, r, &c);
                    if let Some(o) = c.origin.clone() {
                        self.map.insert(nr, o);
                    }
                    used.insert(nr);
                    doc.mark_tree(nr);
                    n += 1;
                }
            }
        }
        // Synced children whose file is gone.
        for k in kids {
            let synced_file = self.map.get(&k).is_some_and(|o| o.key().is_some());
            if synced_file && !used.contains(&k) {
                self.unmap_subtree(doc, k);
                doc.dom.destroy(k);
                doc.mark_tree(r);
                n += 1;
            }
        }
        n
    }

    fn unmap_subtree(&mut self, doc: &Document, r: Ref) {
        for d in doc.dom.descendants_of(r) {
            self.map.remove(&d.referent());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::types::Variant;
    use rbx_dom_weak::InstanceBuilder;

    fn named(doc: &Document, parent: Ref, name: &str) -> Option<Ref> {
        doc.dom.get_by_ref(parent)?.children().iter().copied().find(|c| {
            doc.dom.get_by_ref(*c).is_some_and(|i| i.name == name)
        })
    }

    /// RUSTUDIO_PROJECTS=dir1:dir2…: each opened, re-read from disk and
    /// written back without edits — every file must stay byte-identical
    /// (run on /tmp copies of real projects).
    #[test]
    #[ignore]
    fn untouched_round_trip_real_projects() {
        let Ok(list) = std::env::var("RUSTUDIO_PROJECTS") else { return };
        fn hash_tree(d: &Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    hash_tree(&p, out);
                } else if let Ok(b) = std::fs::read(&p) {
                    out.insert(p, b);
                }
            }
        }
        for dir in list.split(':') {
            let dir = Path::new(dir);
            let mut before = Default::default();
            hash_tree(dir, &mut before);
            let file = crate::project::project_files(dir)[0].clone();
            let (mut doc, mut st) = open(&file).unwrap();
            st.write_back(&mut doc);
            let n = st.reconcile(&mut doc);
            st.write_back(&mut doc);
            let mut after = Default::default();
            hash_tree(dir, &mut after);
            let changed: Vec<_> = after.iter().filter(|(p, b)| before.get(*p) != Some(*b)).map(|(p, _)| p.clone()).collect();
            println!(
                "{}: {} files, {} synced instances, reconcile changed {n}, files changed {}, log {:?}",
                dir.display(),
                before.len(),
                st.map.len(),
                changed.len(),
                st.log
            );
            assert!(changed.is_empty() && after.len() == before.len() && n == 0, "{changed:?}");
        }
    }

    /// Creating, renaming, editing a property of, moving and removing synced
    /// instances each change exactly the right files.
    #[test]
    fn write_back_structure() {
        let dir = std::env::temp_dir().join(format!("rustudio-test-proj-{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("src/Server/Sub"));
        std::fs::write(
            dir.join("default.project.json"),
            r#"{"name":"T","tree":{"$className":"DataModel","ServerScriptService":{"Server":{"$path":"src/Server"}}}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("src/Server/A.server.luau"), "print(1)\n").unwrap();
        std::fs::write(dir.join("src/Server/Sub/B.luau"), "return 1\n").unwrap();
        // The trash for this test stays in /tmp.
        std::env::set_var("XDG_DATA_HOME", dir.join("xdg"));
        let (mut doc, mut st) = open(&dir.join("default.project.json")).unwrap();
        let root = doc.dom.root_ref();
        let server = named(&doc, named(&doc, root, "ServerScriptService").unwrap(), "Server").unwrap();
        let a = named(&doc, server, "A").unwrap();
        let sub = named(&doc, server, "Sub").unwrap();

        // New script → new file.
        let c = doc.insert(server, InstanceBuilder::new("LocalScript").with_name("C").with_property("Source", "-- c\n".to_string()));
        st.write_back(&mut doc);
        assert_eq!(std::fs::read_to_string(dir.join("src/Server/C.client.luau")).unwrap(), "-- c\n");
        // Rename → the file follows.
        doc.rename(a, "A2");
        st.write_back(&mut doc);
        assert!(dir.join("src/Server/A2.server.luau").is_file() && !dir.join("src/Server/A.server.luau").exists());
        // A property → its .meta.json.
        doc.set_props("t", vec![(a, ustr("Disabled"), Variant::Bool(true))], None);
        st.write_back(&mut doc);
        let meta = std::fs::read_to_string(dir.join("src/Server/A2.meta.json")).unwrap();
        assert!(meta.contains("\"Disabled\": true"), "{meta}");
        // Reparent into Sub → moved.
        doc.reparent(&[c], sub);
        st.write_back(&mut doc);
        assert!(dir.join("src/Server/Sub/C.client.luau").is_file() && !dir.join("src/Server/C.client.luau").exists());
        // Remove → to the trash, not gone.
        doc.remove(&[c]);
        st.write_back(&mut doc);
        assert!(!dir.join("src/Server/Sub/C.client.luau").exists());
        assert!(dir.join("xdg/Trash/files/C.client.luau").is_file());
        // A new Part → a .model.json.
        let _p = doc.insert(sub, InstanceBuilder::new("Part").with_name("Box").with_property("Anchored", true));
        st.write_back(&mut doc);
        let m = std::fs::read_to_string(dir.join("src/Server/Sub/Box.model.json")).unwrap();
        assert!(m.contains("\"ClassName\": \"Part\"") && m.contains("\"Anchored\": true"), "{m}");
        // Nothing ever written outside the mapped paths.
        assert!(st.log.iter().all(|l| !l.contains("outside")), "{:?}", st.log);
        // Re-opening reads it all back.
        let (doc2, _) = open(&dir.join("default.project.json")).unwrap();
        let server2 = named(&doc2, named(&doc2, doc2.dom.root_ref(), "ServerScriptService").unwrap(), "Server").unwrap();
        let a2 = named(&doc2, server2, "A2").unwrap();
        assert_eq!(doc2.prop(a2, "Disabled"), Some(Variant::Bool(true)));
        assert!(named(&doc2, named(&doc2, server2, "Sub").unwrap(), "Box").is_some());
    }
}
