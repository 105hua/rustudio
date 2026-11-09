//! Self-test steps for project folders (src/project.rs, project_sync.rs):
//! a small Rojo / Argon project written to /tmp (a script with CRLF line
//! endings and a .meta.json, a module, a .txt, a .model.json, a .json),
//!
//! - `project-roundtrip`: opened and saved (Ctrl+S) untouched, every file
//!   stays byte-identical and no file appears;
//! - `project-sync`: a script edited on disk shows in the open place within
//!   1 s, and an edit made in rustudio (the script editor's path) writes
//!   exactly that change to the file, in its own line endings, touching no
//!   other file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rbx_dom_weak::types::{Ref, Variant};

use super::{SelfTest, Step};
use crate::app::Editor;

#[derive(Default)]
pub(super) struct ProjectTest {
    started: u64,
    dir: PathBuf,
    before: BTreeMap<PathBuf, Vec<u8>>,
    notes: Vec<String>,
    ok: bool,
    disk_edit_at: Option<Instant>,
    seen_ms: Option<u128>,
}

const HELLO: &str = "print(\"hi\")\r\nlocal x = 1\r\n";
const HELLO_DISK: &str = "print(\"hi from disk\")\r\nlocal x = 1\r\n";

fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(d: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if let Ok(b) = std::fs::read(&p) {
                out.insert(p, b);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, &mut out);
    out
}

fn write_fixture(dir: &Path) -> std::io::Result<()> {
    let w = |rel: &str, text: &str| -> std::io::Result<()> {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap_or(dir))?;
        std::fs::write(p, text)
    };
    w(
        "default.project.json",
        r#"{
  "name": "Fixture",
  "tree": {
    "$className": "DataModel",
    "ServerScriptService": {
      "Server": { "$path": "src/Server" }
    },
    "ReplicatedStorage": {
      "Shared": { "$path": "src/Shared" }
    }
  }
}
"#,
    )?;
    w("src/Server/Hello.server.luau", HELLO)?;
    w("src/Server/Hello.meta.json", "{\n  \"properties\": {\n    \"Disabled\": false\n  }\n}\n")?;
    w("src/Shared/Mod.luau", "return {}\n")?;
    w("src/Shared/Note.txt", "hello")?;
    w(
        "src/Shared/Box.model.json",
        "{\n  \"ClassName\": \"Part\",\n  \"Properties\": {\n    \"Anchored\": true,\n    \"Size\": [4, 1, 2]\n  }\n}\n",
    )?;
    w("src/Shared/Data.json", "{\"a\": 1, \"list\": [1, 2]}\n")
}

/// A child by name.
fn named(ed: &Editor, parent: Ref, name: &str) -> Option<Ref> {
    let p = ed.doc.dom.get_by_ref(parent)?;
    p.children()
        .iter()
        .copied()
        .find(|c| ed.doc.dom.get_by_ref(*c).is_some_and(|i| i.name == name))
}

/// The fixture's Hello script in the open place.
fn hello(ed: &Editor) -> Option<Ref> {
    let root = ed.doc.dom.root_ref();
    let sss = named(ed, root, "ServerScriptService")?;
    let server = named(ed, sss, "Server")?;
    named(ed, server, "Hello")
}

fn source(ed: &Editor, r: Ref) -> String {
    match ed.doc.prop(r, "Source") {
        Some(Variant::String(s)) => s,
        _ => String::new(),
    }
}

impl SelfTest {
    /// Phases 97..=99.
    pub(super) fn tick_project(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.pj.started);
        match self.r4.phase {
            97 if self.pj.dir.as_os_str().is_empty() => {
                // Edit mode (the step before may still be playing).
                if ed.play.is_some() {
                    ed.stop_play();
                }
                let dir = std::env::temp_dir().join(format!("rustudio-selftest-project-{}", std::process::id()));
                let _ = std::fs::create_dir_all(&dir);
                self.pj = ProjectTest { started: f, dir: dir.clone(), ok: true, ..Default::default() };
                if let Err(e) = write_fixture(&dir) {
                    self.pj.ok = false;
                    self.pj.notes.push(format!("fixture: {e}"));
                }
                self.pj.before = files(&dir);
                ed.open_path(dir);
            }
            // Untouched: Ctrl+S, then nothing may have changed on disk.
            97 if t == 3 => ed.save(),
            97 if t == 20 => {
                let after = files(&self.pj.dir);
                let same = after == self.pj.before;
                let changed: Vec<String> = after
                    .iter()
                    .filter(|(p, b)| self.pj.before.get(*p) != Some(*b))
                    .map(|(p, _)| p.display().to_string())
                    .chain(self.pj.before.keys().filter(|p| !after.contains_key(*p)).map(|p| format!("{} (gone)", p.display())))
                    .collect();
                let root = ed.doc.dom.root_ref();
                let shared = named(ed, root, "ReplicatedStorage").and_then(|rs| named(ed, rs, "Shared"));
                let class = |n: &str| {
                    shared
                        .and_then(|s| named(ed, s, n))
                        .and_then(|r| ed.doc.class_of(r))
                        .unwrap_or_default()
                };
                let loaded = hello(ed).is_some_and(|h| {
                    ed.doc.class_of(h).as_deref() == Some("Script") && source(ed, h) == HELLO
                }) && class("Mod") == "ModuleScript"
                    && class("Note") == "StringValue"
                    && class("Box") == "Part"
                    && class("Data") == "ModuleScript";
                if !loaded {
                    self.pj.notes.push(format!(
                        "loaded state: project open {}, document \"{}\", playing {}, Hello {:?} source {:?}, Shared: {:?}",
                        ed.project.is_some(),
                        ed.doc.title,
                        ed.play.is_some(),
                        hello(ed).and_then(|h| ed.doc.class_of(h)),
                        hello(ed).map(|h| source(ed, h)),
                        ["Mod", "Note", "Box", "Data"].map(class)
                    ));
                }
                self.steps.push(Step {
                    step: "project-roundtrip",
                    passed: same && loaded && ed.project.is_some(),
                    detail: format!(
                        "fixture project opened ({} files: script with CRLF + .meta.json, module, .txt, .model.json, .json): loaded as Script / ModuleScript / StringValue / Part / ModuleScript: {loaded}; Ctrl+S untouched → every file byte-identical: {same}{}",
                        self.pj.before.len(),
                        if changed.is_empty() { String::new() } else { format!(" (changed: {changed:?})") }
                    ) + &if self.pj.notes.is_empty() { String::new() } else { format!("; {}", self.pj.notes.join("; ")) },
                });
                // (c) on disk → in rustudio.
                let p = self.pj.dir.join("src/Server/Hello.server.luau");
                let _ = std::fs::write(&p, HELLO_DISK);
                self.pj.disk_edit_at = Some(Instant::now());
                self.r4.phase = 98;
                self.pj.started = f;
            }
            98 => {
                if self.pj.seen_ms.is_none() && hello(ed).is_some_and(|h| source(ed, h) == HELLO_DISK) {
                    self.pj.seen_ms = self.pj.disk_edit_at.map(|i| i.elapsed().as_millis());
                }
                let waited = self.pj.disk_edit_at.map(|i| i.elapsed().as_millis()).unwrap_or(0);
                if self.pj.seen_ms.is_some() || waited > 3000 {
                    let ok = self.pj.seen_ms.is_some_and(|ms| ms <= 1000);
                    self.pj.ok &= ok;
                    self.pj.notes.push(match self.pj.seen_ms {
                        Some(ms) => format!("Hello.server.luau edited on disk: in the open place after {ms} ms"),
                        None => "Hello.server.luau edited on disk: not in the open place after 3 s".into(),
                    });
                    // (c) in rustudio → the file: what the script editor does.
                    self.pj.before = files(&self.pj.dir);
                    if let Some(h) = hello(ed) {
                        let text = format!("{}print(2)\n", source(ed, h).replace("\r\n", "\n"));
                        ed.doc.set_props(
                            "Edit script",
                            vec![(h, rbx_dom_weak::ustr("Source"), Variant::String(text))],
                            Some(format!("source:{h:?}")),
                        );
                    }
                    self.r4.phase = 99;
                    self.pj.started = f;
                }
            }
            99 if t == 10 => {
                let p = self.pj.dir.join("src/Server/Hello.server.luau");
                let expect = "print(\"hi from disk\")\r\nlocal x = 1\r\nprint(2)\r\n";
                let got = std::fs::read_to_string(&p).unwrap_or_default();
                let exact = got == expect;
                let after = files(&self.pj.dir);
                let others: Vec<String> = after
                    .iter()
                    .filter(|(q, b)| **q != p && self.pj.before.get(*q) != Some(*b))
                    .map(|(q, _)| q.display().to_string())
                    .collect();
                self.pj.ok &= exact && others.is_empty();
                self.pj.notes.push(format!(
                    "edited in rustudio (a line added, LF from the editor): file now exactly the edit in CRLF: {exact}{}; other files touched: {}",
                    if exact { String::new() } else { format!(" (file: {got:?})") },
                    if others.is_empty() { "none".to_string() } else { format!("{others:?}") }
                ));
                self.steps.push(Step {
                    step: "project-sync",
                    passed: self.pj.ok,
                    detail: self.pj.notes.join("; "),
                });
                // Close the project (later steps open other places).
                ed.new_place();
                self.r4.phase = 100;
            }
            _ => {}
        }
        false
    }
}
