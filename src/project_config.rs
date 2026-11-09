//! Project settings, the way Rojo and Argon let a project choose them:
//!
//! - the project file's own keys: `syncRules` (Rojo's `{pattern, use,
//!   exclude, suffix}` or Argon's `{type, pattern, childPattern, exclude,
//!   suffix}`), `emitLegacyScripts`, `globIgnorePaths` / `ignoreGlobs`;
//! - `argon.toml` beside it (`lua_extension`, `line_endings`, `move_to_bin`);
//! - `rustudio.toml` beside it (read last, so it wins): the same choices plus
//!   the sourcemap.
//!
//! Custom sync rules are tried before the default ones (src/project.rs
//! RULES), as in both tools.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde_json::Value as J;

use crate::project::FileKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LuaExt {
    /// .lua where the folder already uses it, else .luau.
    #[default]
    Auto,
    Luau,
    Lua,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineEndings {
    /// A file keeps its own; new files get LF.
    #[default]
    Keep,
    Lf,
    Crlf,
}

/// A sync rule: files matching `pattern` (relative to the project folder, or
/// the file name alone) become `kind`; `None` ignores them.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncRule {
    pub pattern: String,
    pub kind: Option<FileKind>,
    /// Argon's childPattern: the init file name that makes a folder this kind.
    pub child_pattern: Option<String>,
    pub exclude: Vec<String>,
    /// Stripped from the file name for the instance name (else what follows
    /// the pattern's last `*`).
    pub suffix: Option<String>,
}

/// Every kind a rule can name: (Rojo `use`, Argon `type`, kind).
pub const KINDS: &[(&str, &str, Option<FileKind>)] = &[
    ("serverScript", "ServerScript", Some(FileKind::Server)),
    ("clientScript", "ClientScript", Some(FileKind::Client)),
    ("moduleScript", "ModuleScript", Some(FileKind::Module)),
    ("text", "StringValue", Some(FileKind::Txt)),
    ("csv", "LocalizationTable", Some(FileKind::Csv)),
    ("json", "JsonModule", Some(FileKind::JsonModule)),
    ("jsonModel", "JsonModel", Some(FileKind::JsonModel)),
    ("rbxm", "RbxmModel", Some(FileKind::Rbxm)),
    ("rbxmx", "RbxmxModel", Some(FileKind::Rbxmx)),
    ("project", "Project", Some(FileKind::Project)),
    ("ignore", "Ignore", None),
];

fn kind_named(name: &str) -> Option<Option<FileKind>> {
    let n: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    let k = match n.as_str() {
        "serverscript" | "script" | "server" => Some(FileKind::Server),
        "clientscript" | "localscript" | "client" => Some(FileKind::Client),
        "modulescript" | "module" | "lua" | "luau" => Some(FileKind::Module),
        "text" | "txt" | "stringvalue" => Some(FileKind::Txt),
        "csv" | "localizationtable" => Some(FileKind::Csv),
        "json" | "jsonmodule" => Some(FileKind::JsonModule),
        "jsonmodel" | "model" => Some(FileKind::JsonModel),
        "rbxm" | "rbxmmodel" => Some(FileKind::Rbxm),
        "rbxmx" | "rbxmxmodel" => Some(FileKind::Rbxmx),
        "project" => Some(FileKind::Project),
        "ignore" | "none" => None,
        _ => return None,
    };
    Some(k)
}

pub fn kind_label(k: Option<FileKind>) -> &'static str {
    KINDS.iter().find(|x| x.2 == k).map(|x| x.1).unwrap_or("Ignore")
}

#[derive(Clone, Debug)]
pub struct ProjectConfig {
    pub root: PathBuf,
    pub lua_ext: LuaExt,
    pub line_endings: LineEndings,
    /// Deleted instances' files go to the desktop trash (else deleted).
    pub trash: bool,
    /// Rojo's emitLegacyScripts (default true): false makes `.server` /
    /// `.client` files Scripts with RunContext Server / Client.
    pub emit_legacy_scripts: bool,
    pub ignore_globs: Vec<String>,
    pub rules: Vec<SyncRule>,
    pub sourcemap: bool,
    pub sourcemap_path: String,
    pub sourcemap_non_scripts: bool,
    /// Argon-style project (rules written with `type`, ignores as ignoreGlobs).
    pub argon: bool,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        ProjectConfig {
            root: PathBuf::new(),
            lua_ext: LuaExt::Auto,
            line_endings: LineEndings::Keep,
            trash: true,
            emit_legacy_scripts: true,
            ignore_globs: Vec::new(),
            rules: Vec::new(),
            sourcemap: false,
            sourcemap_path: "sourcemap.json".into(),
            sourcemap_non_scripts: false,
            argon: false,
        }
    }
}

fn strs(v: Option<&J>) -> Vec<String> {
    match v {
        Some(J::String(s)) => vec![s.clone()],
        Some(J::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    }
}

fn parse_rule(v: &J) -> Option<SyncRule> {
    let pattern = v.get("pattern")?.as_str()?.to_string();
    let kind = kind_named(v.get("use").or_else(|| v.get("type"))?.as_str()?)?;
    Some(SyncRule {
        pattern,
        kind,
        child_pattern: v.get("childPattern").and_then(|x| x.as_str()).map(String::from),
        exclude: strs(v.get("exclude")),
        suffix: v.get("suffix").and_then(|x| x.as_str()).map(String::from),
    })
}

impl ProjectConfig {
    /// The settings of the project file `file` (its JSON `pj`).
    pub fn load(file: &Path, pj: &J) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let root = file.parent().unwrap_or(Path::new("/")).to_path_buf();
        let mut c = ProjectConfig { root: root.clone(), ..Default::default() };
        c.argon = crate::project::argon_project(file, pj);
        c.ignore_globs = ["globIgnorePaths", "ignoreGlobs"].iter().flat_map(|k| strs(pj.get(*k))).collect();
        if let Some(b) = pj.get("emitLegacyScripts").and_then(|x| x.as_bool()) {
            c.emit_legacy_scripts = b;
        }
        for r in pj.get("syncRules").and_then(|x| x.as_array()).into_iter().flatten() {
            match parse_rule(r) {
                Some(rule) => c.rules.push(rule),
                None => warnings.push(format!("syncRules: can't read {r}")),
            }
        }
        for (name, ours) in [("argon.toml", false), ("rustudio.toml", true)] {
            let p = root.join(name);
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            match text.parse::<toml::Table>() {
                Ok(t) => c.apply_toml(&t, ours),
                Err(e) => warnings.push(format!("{}: {e}", p.display())),
            }
        }
        (c, warnings)
    }

    fn apply_toml(&mut self, t: &toml::Table, ours: bool) {
        match t.get("lua_extension") {
            Some(toml::Value::Boolean(b)) => self.lua_ext = if *b { LuaExt::Lua } else { LuaExt::Luau },
            Some(toml::Value::String(s)) => {
                self.lua_ext = match s.to_ascii_lowercase().as_str() {
                    "lua" => LuaExt::Lua,
                    "luau" => LuaExt::Luau,
                    _ => LuaExt::Auto,
                }
            }
            _ => {}
        }
        if let Some(s) = t.get("line_endings").and_then(|v| v.as_str()) {
            self.line_endings = match s.to_ascii_lowercase().as_str() {
                "lf" => LineEndings::Lf,
                "crlf" => LineEndings::Crlf,
                _ => LineEndings::Keep,
            };
        }
        if let Some(b) = t.get("move_to_bin").or_else(|| t.get("trash")).and_then(|v| v.as_bool()) {
            self.trash = b;
        }
        if !ours {
            return;
        }
        if let Some(sm) = t.get("sourcemap").and_then(|v| v.as_table()) {
            if let Some(b) = sm.get("enabled").and_then(|v| v.as_bool()) {
                self.sourcemap = b;
            }
            if let Some(s) = sm.get("path").and_then(|v| v.as_str()) {
                self.sourcemap_path = s.to_string();
            }
            if let Some(b) = sm.get("include_non_scripts").and_then(|v| v.as_bool()) {
                self.sourcemap_non_scripts = b;
            }
        }
    }

    /// Saves the settings: rustudio.toml, and the shared keys into the
    /// project file (keeping its other keys and its formatting style).
    pub fn save(&self, project_file: &Path) -> Result<(), String> {
        let lua = match self.lua_ext {
            LuaExt::Auto => "auto",
            LuaExt::Luau => "luau",
            LuaExt::Lua => "lua",
        };
        let le = match self.line_endings {
            LineEndings::Keep => "keep",
            LineEndings::Lf => "lf",
            LineEndings::Crlf => "crlf",
        };
        let toml = format!(
            "# rustudio project settings (Project Settings… writes this file)\n\
             lua_extension = \"{lua}\"   # auto | luau | lua\n\
             line_endings = \"{le}\"     # keep | lf | crlf\n\
             trash = {}                # deleted instances' files go to the trash (false: deleted)\n\n\
             [sourcemap]\n\
             enabled = {}\n\
             path = {}\n\
             include_non_scripts = {}\n",
            self.trash,
            self.sourcemap,
            toml::Value::String(self.sourcemap_path.clone()),
            self.sourcemap_non_scripts
        );
        let rt = self.root.join("rustudio.toml");
        std::fs::write(&rt, toml).map_err(|e| format!("{}: {e}", rt.display()))?;

        let old = std::fs::read(project_file).map_err(|e| format!("{}: {e}", project_file.display()))?;
        let mut pj: J = serde_json::from_slice(&old).map_err(|e| format!("{}: {e}", project_file.display()))?;
        let o = pj.as_object_mut().ok_or("the project file isn't a JSON object")?;
        let ig_key = if self.argon { "ignoreGlobs" } else { "globIgnorePaths" };
        o.remove("globIgnorePaths");
        o.remove("ignoreGlobs");
        if !self.ignore_globs.is_empty() {
            o.insert(ig_key.into(), serde_json::json!(self.ignore_globs));
        }
        if self.emit_legacy_scripts {
            o.remove("emitLegacyScripts");
        } else {
            o.insert("emitLegacyScripts".into(), J::Bool(false));
        }
        o.remove("syncRules");
        if !self.rules.is_empty() {
            let rules: Vec<J> = self
                .rules
                .iter()
                .map(|r| {
                    let mut m = serde_json::Map::new();
                    m.insert("pattern".into(), J::String(r.pattern.clone()));
                    let k = KINDS.iter().find(|x| x.2 == r.kind).unwrap_or(&KINDS[KINDS.len() - 1]);
                    if self.argon {
                        m.insert("type".into(), J::String(k.1.into()));
                    } else {
                        m.insert("use".into(), J::String(k.0.into()));
                    }
                    if let Some(c) = &r.child_pattern {
                        m.insert("childPattern".into(), J::String(c.clone()));
                    }
                    if !r.exclude.is_empty() {
                        m.insert("exclude".into(), if self.argon { serde_json::json!(r.exclude) } else { J::String(r.exclude[0].clone()) });
                    }
                    if let Some(s) = &r.suffix {
                        m.insert("suffix".into(), J::String(s.clone()));
                    }
                    J::Object(m)
                })
                .collect();
            o.insert("syncRules".into(), J::Array(rules));
        }
        let crlf = old.windows(2).any(|w| w == b"\r\n");
        let mut s = serde_json::to_string_pretty(&pj).unwrap_or_default();
        if old.ends_with(b"\n") {
            s.push('\n');
        }
        if crlf {
            s = s.replace('\n', "\r\n");
        }
        if s.as_bytes() != old.as_slice() {
            std::fs::write(project_file, s).map_err(|e| format!("{}: {e}", project_file.display()))?;
        }
        Ok(())
    }

    /// The instance name and kind a custom rule gives a file (`Some(None)`:
    /// a rule ignores it), or None when no custom rule takes it.
    pub fn classify(&self, path: Option<&Path>, file_name: &str) -> Option<Option<(String, FileKind)>> {
        let rel = path.map(|p| p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().replace('\\', "/"));
        let hit = |pat: &str| {
            glob::Pattern::new(pat)
                .map(|g| g.matches(file_name) || rel.as_deref().is_some_and(|r| g.matches(r)))
                .unwrap_or(false)
        };
        for r in &self.rules {
            if !hit(&r.pattern) || r.exclude.iter().any(|e| hit(e)) {
                continue;
            }
            let Some(kind) = r.kind else { return Some(None) };
            let suffix = r.suffix.clone().unwrap_or_else(|| {
                r.pattern.rsplit('*').next().unwrap_or("").rsplit('/').next().unwrap_or("").to_string()
            });
            let stem = match file_name.strip_suffix(suffix.as_str()) {
                Some(s) if !suffix.is_empty() && !s.is_empty() => s.to_string(),
                _ => file_name.split('.').next().unwrap_or(file_name).to_string(),
            };
            if stem.is_empty() {
                continue;
            }
            return Some(Some((stem, kind)));
        }
        None
    }

    /// A folder's init file from a custom rule's childPattern.
    pub fn init_file(&self, dir: &Path) -> Option<(PathBuf, FileKind)> {
        self.rules.iter().find_map(|r| {
            let k = r.kind?;
            let p = dir.join(r.child_pattern.as_ref()?);
            p.is_file().then_some((p, k))
        })
    }
}

static ACTIVE: RwLock<Option<Arc<ProjectConfig>>> = RwLock::new(None);

/// The open project's settings (one project is open at a time).
pub fn active() -> Option<Arc<ProjectConfig>> {
    ACTIVE.read().ok().and_then(|g| g.clone())
}

pub fn set_active(c: Option<Arc<ProjectConfig>>) {
    if let Ok(mut g) = ACTIVE.write() {
        *g = c;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_from_both_tools() {
        let pj = serde_json::json!({
            "name": "x", "tree": {},
            "syncRules": [
                {"pattern": "*.data.lua", "use": "json", "suffix": ".data.lua"},
                {"type": "ServerScript", "pattern": "*.srv.luau", "childPattern": "init.srv.luau", "exclude": ["skip/*"]},
                {"pattern": "*.bak", "use": "ignore"}
            ],
            "emitLegacyScripts": false
        });
        let (c, w) = ProjectConfig::load(Path::new("/p/default.project.json"), &pj);
        assert!(w.is_empty(), "{w:?}");
        assert!(!c.emit_legacy_scripts);
        assert_eq!(c.classify(None, "A.data.lua"), Some(Some(("A".into(), FileKind::JsonModule))));
        assert_eq!(c.classify(Some(Path::new("/p/src/B.srv.luau")), "B.srv.luau"), Some(Some(("B".into(), FileKind::Server))));
        assert_eq!(c.classify(Some(Path::new("/p/skip/B.srv.luau")), "B.srv.luau"), None);
        assert_eq!(c.classify(None, "x.bak"), Some(None));
        assert_eq!(c.classify(None, "x.luau"), None);
    }
}
