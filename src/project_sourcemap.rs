//! `sourcemap.json` in Rojo's format (what luau-lsp and other tools read):
//! the synced tree, each node `{name, className, filePaths?, children?}`
//! with paths relative to the project folder. As `rojo sourcemap` does by
//! default, only scripts and their ancestors are listed unless
//! `include_non_scripts` is set.

use std::path::{Path, PathBuf};

use rbx_dom_weak::types::Ref;
use serde_json::{json, Value as J};

use super::ProjectState;
use crate::doc::Document;
use crate::project::Origin;

fn is_script(class: &str) -> bool {
    matches!(class, "Script" | "LocalScript" | "ModuleScript")
}

impl ProjectState {
    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().replace('\\', "/")
    }

    fn file_paths(&self, o: &Origin) -> Vec<String> {
        let mut v: Vec<PathBuf> = Vec::new();
        match o {
            Origin::File { file, data, .. } | Origin::Model { file, data, .. } => {
                v.push(file.clone());
                v.push(data.clone());
            }
            Origin::Dir { init, data, .. } => {
                if let Some((f, _)) = init {
                    v.push(f.clone());
                }
                v.push(data.clone());
            }
            Origin::Node { .. } | Origin::InModel { .. } => {}
        }
        v.into_iter().filter(|p| p.is_file()).map(|p| self.rel(&p)).collect()
    }

    /// A project-file node that sets properties (Argon lists those).
    fn node_has_props(&self, o: &Origin) -> bool {
        let Origin::Node { project, json_path } = o else { return false };
        let Some(mut v) = self.disk.get(project).and_then(|b| serde_json::from_slice::<J>(b).ok()) else { return false };
        v = match v.get("tree") { Some(t) => t.clone(), None => return false };
        for k in json_path {
            v = match v.get(k) { Some(x) => x.clone(), None => return false };
        }
        ["$properties", "$attributes", "$tags"].iter().any(|k| v.get(*k).is_some())
    }

    fn node(&self, doc: &Document, r: Ref, all: bool, in_map: bool) -> Option<J> {
        let i = doc.dom.get_by_ref(r)?;
        let origin = self.map.get(&r);
        // Inside a model file: the model's (listed without paths, as Argon does).
        let in_model = in_map || matches!(origin, Some(Origin::Model { .. } | Origin::InModel { .. }));
        let kids: Vec<J> = i
            .children()
            .iter()
            .filter_map(|c| self.node(doc, *c, all, in_model))
            .collect();
        let class = i.class.as_str();
        let paths = origin.map(|o| self.file_paths(o)).unwrap_or_default();
        // Place-file instances and empty project nodes aren't listed.
        let synced = !paths.is_empty()
            || in_map
            || matches!(origin, Some(Origin::Dir { .. }))
            || (all && origin.is_some_and(|o| self.node_has_props(o)));
        let keep = !kids.is_empty() || (synced && (all || is_script(class)));
        if !keep {
            return None;
        }
        let mut o = serde_json::Map::new();
        o.insert("name".into(), json!(i.name));
        o.insert("className".into(), json!(class));
        if !paths.is_empty() {
            o.insert("filePaths".into(), json!(paths));
        }
        if !kids.is_empty() {
            o.insert("children".into(), J::Array(kids));
        }
        Some(J::Object(o))
    }

    /// The sourcemap of the open place.
    pub fn sourcemap_json(&self, doc: &Document, include_non_scripts: bool) -> J {
        let dm = doc.dom.root_ref();
        let mut root = self.node(doc, dm, include_non_scripts, false).unwrap_or_else(|| json!({}));
        if let Some(o) = root.as_object_mut() {
            o.insert("name".into(), json!(self.name));
            o.insert("className".into(), json!("DataModel"));
            o.insert("filePaths".into(), json!([self.rel(&self.file)]));
        }
        root
    }

    /// Writes the sourcemap when the project asks for one and it changed.
    pub fn update_sourcemap(&mut self, doc: &Document) {
        self.sourcemap_dirty = false;
        if !self.config.sourcemap {
            return;
        }
        let out = self.root.join(&self.config.sourcemap_path);
        let bytes = self.sourcemap_bytes(doc, self.config.sourcemap_non_scripts);
        if std::fs::read(&out).ok().as_deref() == Some(bytes.as_slice()) {
            return;
        }
        match std::fs::write(&out, &bytes) {
            Ok(()) => {
                // Our own write comes back from the watcher: known bytes are dropped.
                self.disk.insert(out, bytes);
            }
            Err(e) => self.warnings.push(format!("sourcemap {}: {e}", out.display())),
        }
    }

    pub fn sourcemap_bytes(&self, doc: &Document, include_non_scripts: bool) -> Vec<u8> {
        let mut s = serde_json::to_string(&self.sourcemap_json(doc, include_non_scripts)).unwrap_or_default();
        s.push('\n');
        s.into_bytes()
    }
}
