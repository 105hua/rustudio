//! File > Save as Project Folder… and File > Project Settings…: the windows
//! for src/project_export.rs and src/project_config.rs.

use std::path::PathBuf;

use crate::app::Editor;
use crate::project::FileKind;
use crate::project_config::{kind_label, LineEndings, LuaExt, ProjectConfig, SyncRule, KINDS};
use crate::project_export::{ExportOptions, CONTAINERS};

/// Save as Project Folder: where, and what goes to files.
pub struct ExportDialog {
    pub opts: ExportOptions,
    /// The folder the project folder is made in.
    pub parent: String,
    pub open_after: bool,
    pub error: Option<String>,
}

/// Project Settings: an editable copy of the open project's settings.
pub struct SettingsDialog {
    pub cfg: ProjectConfig,
    pub ignore_text: String,
    pub error: Option<String>,
}

fn safe_name(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_alphanumeric() || "-_ ".contains(c) { c } else { '_' }).collect();
    let s = s.trim().replace(' ', "-");
    if s.is_empty() { "game".into() } else { s }
}

impl Editor {
    pub fn open_export_dialog(&mut self) {
        let name = self
            .doc
            .cloud
            .as_ref()
            .map(|c| if c.experience.is_empty() { c.name.clone() } else { c.experience.clone() })
            .or_else(|| self.doc.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "game".into());
        let parent = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
            .unwrap_or_else(crate::paths::home);
        self.project_export = Some(ExportDialog {
            opts: ExportOptions { name: safe_name(&name), ..Default::default() },
            parent: parent.display().to_string(),
            open_after: true,
            error: None,
        });
    }

    pub fn open_settings_dialog(&mut self) {
        let Some(st) = self.project.as_ref() else { return };
        let cfg = (*st.config).clone();
        self.project_settings = Some(SettingsDialog { ignore_text: cfg.ignore_globs.join("\n"), cfg, error: None });
    }

    /// Draws the open project windows (once a frame).
    pub fn project_windows(&mut self, ctx: &egui::Context) {
        self.export_window(ctx);
        self.settings_window(ctx);
    }

    fn export_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.project_export.take() else { return };
        let mut open = true;
        let mut go = false;
        let root = self.doc.dom.root_ref();
        let present: Vec<(&str, bool)> = CONTAINERS
            .iter()
            .copied()
            .filter(|(c, _)| {
                let mut r = root;
                c.split('/').all(|seg| {
                    let next = self.doc.dom.get_by_ref(r).and_then(|i| {
                        i.children().iter().copied().find(|k| self.doc.dom.get_by_ref(*k).is_some_and(|x| x.name == seg))
                    });
                    next.map(|n| r = n).is_some()
                })
            })
            .collect();
        egui::Window::new("Save as Project Folder")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Writes this place as a Rojo / Argon project folder: scripts become files, the rest stays in a place file beside the project.");
                ui.add_space(6.0);
                egui::Grid::new("export_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Project name");
                    ui.text_edit_singleline(&mut d.opts.name);
                    ui.end_row();
                    ui.label("Create in");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut d.parent).desired_width(260.0));
                        if ui.button("Browse…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().set_directory(&d.parent).pick_folder() {
                                d.parent = p.display().to_string();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Folder");
                    ui.label(PathBuf::from(&d.parent).join(safe_name(&d.opts.name)).display().to_string());
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.label("Synced to files:");
                ui.columns(2, |cols| {
                    for (n, (c, _)) in present.iter().enumerate() {
                        let mut on = d.opts.containers.iter().any(|x| x == c);
                        if cols[n % 2].checkbox(&mut on, *c).changed() {
                            d.opts.containers.retain(|x| x != c);
                            if on {
                                d.opts.containers.push(c.to_string());
                            }
                        }
                    }
                });
                ui.add_space(6.0);
                ui.checkbox(&mut d.opts.lua, "Use .lua instead of .luau");
                ui.checkbox(&mut d.opts.sourcemap, "Keep a sourcemap.json up to date (for luau-lsp)");
                ui.checkbox(&mut d.open_after, "Open the project folder afterwards");
                if let Some(e) = &d.error {
                    ui.colored_label(egui::Color32::from_rgb(230, 90, 80), e);
                }
                ui.add_space(4.0);
                if ui.button("Save as Project Folder").clicked() {
                    go = true;
                }
            });
        if go {
            let dir = PathBuf::from(&d.parent).join(safe_name(&d.opts.name));
            let mut opts = d.opts.clone();
            opts.name = safe_name(&opts.name);
            if let Some(c) = &self.doc.cloud {
                opts.place_id = Some(c.place_id);
                opts.game_id = Some(c.universe_id);
            }
            match crate::project_export::export(&self.doc, &dir, &opts) {
                Ok(rep) => {
                    self.info(format!(
                        "Saved as project folder {}: {} scripts, {} folders, {} model files",
                        dir.display(),
                        rep.scripts,
                        rep.folders,
                        rep.models
                    ));
                    for w in rep.warnings.iter().take(20) {
                        self.warn(format!("Project export: {w}"));
                    }
                    if d.open_after {
                        self.open_project(&rep.project_file);
                    }
                    return;
                }
                Err(e) => d.error = Some(e),
            }
        }
        if open {
            self.project_export = Some(d);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.project_settings.take() else { return };
        if self.project.is_none() {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut write_map = false;
        egui::Window::new("Project Settings")
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .open(&mut open)
            .show(ctx, |ui| {
                let c = &mut d.cfg;
                ui.label(format!("{}", c.root.display()));
                ui.add_space(4.0);
                egui::CollapsingHeader::new("Files").default_open(true).show(ui, |ui| {
                    egui::Grid::new("ps_files").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label("New script files");
                        egui::ComboBox::from_id_salt("ps_lua")
                            .selected_text(match c.lua_ext { LuaExt::Auto => "Match the folder", LuaExt::Luau => ".luau", LuaExt::Lua => ".lua" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.lua_ext, LuaExt::Auto, "Match the folder");
                                ui.selectable_value(&mut c.lua_ext, LuaExt::Luau, ".luau");
                                ui.selectable_value(&mut c.lua_ext, LuaExt::Lua, ".lua");
                            });
                        ui.end_row();
                        ui.label("Line endings");
                        egui::ComboBox::from_id_salt("ps_le")
                            .selected_text(match c.line_endings { LineEndings::Keep => "Keep each file's", LineEndings::Lf => "LF", LineEndings::Crlf => "CRLF" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.line_endings, LineEndings::Keep, "Keep each file's");
                                ui.selectable_value(&mut c.line_endings, LineEndings::Lf, "LF");
                                ui.selectable_value(&mut c.line_endings, LineEndings::Crlf, "CRLF");
                            });
                        ui.end_row();
                        ui.label("Deleted instances");
                        egui::ComboBox::from_id_salt("ps_trash")
                            .selected_text(if c.trash { "Files go to the trash" } else { "Files are deleted" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut c.trash, true, "Files go to the trash");
                                ui.selectable_value(&mut c.trash, false, "Files are deleted");
                            });
                        ui.end_row();
                    });
                    ui.checkbox(&mut c.emit_legacy_scripts, "Legacy scripts (.client files are LocalScripts; off: Scripts with RunContext)");
                });
                egui::CollapsingHeader::new("Sourcemap").default_open(true).show(ui, |ui| {
                    ui.checkbox(&mut c.sourcemap, "Keep a sourcemap up to date (luau-lsp reads it)");
                    ui.horizontal(|ui| {
                        ui.label("File");
                        ui.text_edit_singleline(&mut c.sourcemap_path);
                    });
                    ui.checkbox(&mut c.sourcemap_non_scripts, "Include instances that aren't scripts");
                    if ui.button("Write it now").clicked() {
                        write_map = true;
                    }
                });
                egui::CollapsingHeader::new("Ignored files").default_open(true).show(ui, |ui| {
                    ui.label("Glob patterns, one per line (relative to the project folder):");
                    ui.add(egui::TextEdit::multiline(&mut d.ignore_text).desired_rows(3).desired_width(f32::INFINITY).hint_text("**/*.spec.luau"));
                });
                egui::CollapsingHeader::new("Sync rules").default_open(true).show(ui, |ui| {
                    ui.label("Tried before the defaults, top first. Pattern matches the file name or its path in the project.");
                    let mut remove = None;
                    let mut swap = None;
                    egui::Grid::new("ps_rules").num_columns(6).spacing([6.0, 4.0]).striped(true).show(ui, |ui| {
                        ui.strong("Pattern");
                        ui.strong("Becomes");
                        ui.strong("Name suffix");
                        ui.strong("Folder init file");
                        ui.strong("Except");
                        ui.label("");
                        ui.end_row();
                        let n = c.rules.len();
                        for (i, r) in c.rules.iter_mut().enumerate() {
                            ui.add(egui::TextEdit::singleline(&mut r.pattern).desired_width(120.0).hint_text("*.server.lua"));
                            egui::ComboBox::from_id_salt(("ps_kind", i)).selected_text(kind_label(r.kind)).show_ui(ui, |ui| {
                                for (_, label, k) in KINDS {
                                    ui.selectable_value(&mut r.kind, *k, *label);
                                }
                            });
                            let mut suffix = r.suffix.clone().unwrap_or_default();
                            if ui.add(egui::TextEdit::singleline(&mut suffix).desired_width(90.0).hint_text("auto")).changed() {
                                r.suffix = (!suffix.is_empty()).then_some(suffix);
                            }
                            let mut child = r.child_pattern.clone().unwrap_or_default();
                            if ui.add(egui::TextEdit::singleline(&mut child).desired_width(110.0).hint_text("init.server.lua")).changed() {
                                r.child_pattern = (!child.is_empty()).then_some(child);
                            }
                            let mut ex = r.exclude.join(", ");
                            if ui.add(egui::TextEdit::singleline(&mut ex).desired_width(100.0)).changed() {
                                r.exclude = ex.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                            }
                            ui.horizontal(|ui| {
                                if ui.add_enabled(i > 0, egui::Button::new("↑").small()).clicked() {
                                    swap = Some((i, i - 1));
                                }
                                if ui.add_enabled(i + 1 < n, egui::Button::new("↓").small()).clicked() {
                                    swap = Some((i, i + 1));
                                }
                                if ui.small_button("✕").clicked() {
                                    remove = Some(i);
                                }
                            });
                            ui.end_row();
                        }
                    });
                    if let Some((a, b)) = swap {
                        c.rules.swap(a, b);
                    }
                    if let Some(i) = remove {
                        c.rules.remove(i);
                    }
                    if ui.button("Add rule").clicked() {
                        c.rules.push(SyncRule {
                            pattern: String::new(),
                            kind: Some(FileKind::Module),
                            child_pattern: None,
                            exclude: Vec::new(),
                            suffix: None,
                        });
                    }
                });
                ui.add_space(6.0);
                ui.label("Saved to rustudio.toml, and the shared keys (syncRules, ignore globs, emitLegacyScripts) into the project file so Rojo / Argon use them too.");
                if let Some(e) = &d.error {
                    ui.colored_label(egui::Color32::from_rgb(230, 90, 80), e);
                }
                if ui.button("Save and reload project").clicked() {
                    save = true;
                }
            });
        if write_map {
            let doc = &self.doc;
            if let Some(st) = self.project.as_mut() {
                let mut tmp = (*st.config).clone();
                tmp.sourcemap = true;
                tmp.sourcemap_path = d.cfg.sourcemap_path.clone();
                tmp.sourcemap_non_scripts = d.cfg.sourcemap_non_scripts;
                let keep = std::mem::replace(&mut st.config, std::sync::Arc::new(tmp));
                st.update_sourcemap(doc);
                st.config = keep;
                let p = st.root.join(&d.cfg.sourcemap_path);
                self.info(format!("Project: wrote {}", p.display()));
            }
        }
        if save {
            d.cfg.ignore_globs = d.ignore_text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            d.cfg.rules.retain(|r| !r.pattern.trim().is_empty());
            let bad: Vec<String> = d
                .cfg
                .ignore_globs
                .iter()
                .chain(d.cfg.rules.iter().map(|r| &r.pattern))
                .filter(|g| glob::Pattern::new(g).is_err())
                .cloned()
                .collect();
            if !bad.is_empty() {
                d.error = Some(format!("Not a valid glob pattern: {}", bad.join(", ")));
            } else if let Some(file) = self.project.as_ref().map(|p| p.file.clone()) {
                // Edits outside the synced files go to the place first.
                if self.project.as_ref().is_some_and(|p| p.place_dirty) {
                    self.save_project();
                }
                match d.cfg.save(&file) {
                    Ok(()) => {
                        self.open_project(&file);
                        self.info("Project settings saved");
                        return;
                    }
                    Err(e) => d.error = Some(e),
                }
            }
        }
        if open {
            self.project_settings = Some(d);
        }
    }
}
