//! What each Studio menu and ribbon item does in rustudio, and whether it
//! is available (src/studio_ui.rs lists them). Anything rustudio can't do
//! yet is disabled rather than faked.


use egui::{Rect, RichText, Sense, Vec2};
use rbx_dom_weak::types::{Ref, Variant};

use crate::app::{Editor, PendingAction, Tab, Tool};
use crate::render::MeshKind;
use crate::studio_ui::{menu_rect_key, menu_shortcuts, RunType, MENUS, M};
use crate::theme;

/// Studio's File menu is 233 px wide (checks/ui/studio/menu-File.png);
/// the account menu uses this width.
pub const MENU_W: f32 = 228.0;

/// One menu item as Studio draws it: a check column, the label, and the
/// shortcut left-aligned in a column after the menu's longest label.
pub fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, checked: Option<bool>) -> egui::Response {
    menu_item_at(ui, label, shortcut, enabled, checked, MENU_W - 70.0, MENU_W)
}

fn menu_item_at(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, checked: Option<bool>, col: f32, w: f32) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), sense);
    let p = ui.painter();
    if enabled && resp.hovered() {
        p.rect_filled(r, 2.0, theme::HOVER_BG);
    }
    let c = if enabled { theme::TEXT } else { theme::TEXT_DIM };
    if checked == Some(true) {
        let cr = Rect::from_center_size(egui::pos2(r.min.x + 11.0, r.center().y), Vec2::splat(14.0));
        crate::ti::paint(p, cr, "check", c);
    }
    let font = egui::FontId::proportional(13.0);
    p.text(egui::pos2(r.min.x + 24.0, r.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), c);
    if !shortcut.is_empty() {
        p.text(egui::pos2(r.min.x + col, r.center().y), egui::Align2::LEFT_CENTER, shortcut, font, c);
    }
    resp
}

/// A menu's shortcut column and width, from its longest label and shortcut.
fn menu_metrics(ui: &egui::Ui, items: &[M]) -> (f32, f32) {
    let font = egui::FontId::proportional(13.0);
    let width = |t: &str| ui.fonts_mut(|f| f.layout_no_wrap(t.to_string(), font.clone(), theme::TEXT).size().x);
    let (mut label, mut sc) = (0.0f32, 0.0f32);
    for m in items {
        match m {
            M::I(l, k, _) => {
                label = label.max(width(l));
                sc = sc.max(width(k));
            }
            M::Sub(l, _) => label = label.max(width(l) + 16.0),
            M::S => {}
        }
    }
    let col = 24.0 + label + 28.0;
    (col, (col + sc + 16.0).max(150.0))
}

/// A submenu title, indented like the items beside it (their check column).
fn sub_label(label: &str) -> egui::RichText {
    RichText::new(format!("\u{2007}\u{2007}\u{2007}{label}")).size(13.0)
}

impl Editor {
    /// (enabled, checked) of a menu / ribbon action.
    pub fn action_state(&self, a: &str) -> (bool, Option<bool>) {
        let sel = !self.doc.selection.is_empty();
        let parts = !self.selected_parts().is_empty();
        let playing = self.play.is_some();
        let edit = !playing;
        let open = |n: &str| Some(self.ui.panels_open.contains(n));
        match a {
            "new" | "open" | "open_roblox" | "open_folder" | "close_place" | "import_model" => (edit, None),
            "save" | "save_as" | "save_roblox" | "publish" => (edit, None),
            "export_project" => (edit, None),
            "project_settings" => (edit && self.project.is_some(), None),
            "experience_settings" | "account" | "add_account" => (true, None),
            "live_collaborators" | "invite_collaborators" => (true,None),
            "avatar_settings" => (true, open("Avatar Settings")),
            "terrain_editor" => (true, open("Terrain Editor")),
            "about" | "exit" => (true, None),
            "undo" => (edit && self.doc.can_undo().is_some(), None),
            "redo" => (edit && self.doc.can_redo().is_some(), None),
            "cut" | "copy" | "duplicate" | "delete" => (edit && sel, None),
            "paste" | "paste_original" => (edit && self.clipboard.is_some(), None),
            "paste_into" | "paste_into_original" => (edit && self.clipboard.is_some() && sel, None),
            "group" | "group_folder" => (edit && sel, None),
            "ungroup" => (
                edit && self
                    .doc
                    .selection
                    .iter()
                    .any(|r| matches!(self.doc.class_of(*r).as_deref(), Some("Model" | "Folder"))),
                None,
            ),
            "lock" => (edit && parts, Some(parts && self.all_parts("Locked"))),
            "anchor" => (edit && parts, Some(parts && self.all_parts("Anchored"))),
            "unlock_all" | "select_all" => (edit, None),
            "zoom_in" | "zoom_out" => (true, None),
            "full_screen" => (true, Some(self.ui.fullscreen)),
            "show_grid" => (true, Some(self.show_grid)),
            "grid_2" => (self.show_grid, Some(self.ui.grid_size == 2.0)),
            "grid_4" => (self.show_grid, Some(self.ui.grid_size == 4.0)),
            "grid_16" => (self.show_grid, Some(self.ui.grid_size == 16.0)),
            "show_ui" => (true, Some(self.ui.show_gui)),
            "screenshot" => (true, None),
            "mute" => (true, Some(crate::audio::muted())),
            "test" | "test_here" | "run" => (!playing, None),
            "break_never" => (false, Some(true)),
            "toggle_client_server" => (
                self.play.as_ref().is_some_and(|p| p.mode == crate::runtime::RunMode::Play),
                None,
            ),
            "stop" => (playing, None),
            "panel_properties" => (true, open("Properties")),
            "panel_explorer" => (true, open("Explorer")),
            "panel_output" => (true, open("Output")),
            "insert_object" => (edit, None),
            "reset_layout" => (true, None),
            "documentation" | "tutorials" | "developer_forum" => (true, None),
            "command_bar" => (false, Some(false)),
            // Ribbon-only actions.
            "tool_select" | "tool_move" | "tool_scale" | "tool_rotate" => (true, None),
            "part" | "part_block" | "part_sphere" | "part_wedge" | "part_corner" | "part_cylinder" => (edit, None),
            "gui" | "script" => (edit, None),
            "effect" | "attachment" => (edit && parts, None),
            "material" | "color" => (edit && parts, None),
            "weld" => (edit && self.selected_parts().len() >= 2, None),
            "ui_frame" | "ui_label" | "ui_input" | "ui_appearance" | "ui_layout" | "ui_constraint" => {
                (edit && self.gui_target().is_some(), None)
            }
            _ => (false, None),
        }
    }

    pub fn all_parts(&self, prop: &str) -> bool {
        self.selected_parts()
            .iter()
            .all(|r| matches!(self.doc.prop(*r, prop), Some(Variant::Bool(true))))
    }

    /// Where a UI-tab insert goes: the first selected LayerCollector or GuiObject.
    pub fn gui_target(&self) -> Option<Ref> {
        self.doc.selection.iter().copied().find(|r| {
            self.doc
                .class_of(*r)
                .is_some_and(|c| crate::doc::is_a(&c, "GuiObject") || crate::doc::is_a(&c, "LayerCollector"))
        })
    }

    /// Inserts go into the first selected object, else Workspace (Studio).
    pub fn insert_parent(&self) -> Ref {
        self.doc.selection.first().copied().unwrap_or(self.doc.default_parent())
    }

    fn service(&self, name: &str) -> Ref {
        let root = self.doc.dom.root_ref();
        self.doc.find_child(root, name).unwrap_or(self.doc.default_parent())
    }

    fn set_on_parts(&mut self, label: &str, prop: &str, v: bool) {
        let edits = self
            .selected_parts()
            .into_iter()
            .map(|r| (r, rbx_dom_weak::ustr(prop), Variant::Bool(v)))
            .collect();
        self.doc.set_props(label, edits, None);
    }

    /// Runs a menu or ribbon action (disabled ones do nothing).
    pub fn ui_action(&mut self, ctx: &egui::Context, a: &str) {
        if !a.starts_with("class:") && !self.action_state(a).0 {
            return;
        }
        match a {
            "live_collaborators" | "invite_collaborators" => self.cloud.live_collaborators_open=true,
            "new" => self.request(PendingAction::New),
            "close_place" => self.request(PendingAction::Close),
            "open" => self.request(PendingAction::Open(None)),
            "open_folder" => self.request(PendingAction::OpenFolder),
            "export_project" => self.open_export_dialog(),
            "project_settings" => self.open_settings_dialog(),
            "open_roblox" => {
                self.cloud.open = true;
                let client = self.account.shared_client();
                let tab = self.cloud.tab;
                self.cloud.refresh(client, tab);
            }
            "import_model" => self.insert_from_file(),
            // "Save to File": a place from Roblox goes to a file too (Ctrl+S
            // saves it to Roblox).
            "save" if self.doc.cloud.is_some() => match self.doc.path.clone() {
                Some(p) => self.save_to(p),
                None => self.save_as(),
            },
            "save" => self.save(),
            "save_as" => self.save_as(),
            "save_roblox" | "publish" if self.doc.cloud.is_some() => {
                let vt = if a == "publish" {
                    rustudio_account::VersionType::Published
                } else {
                    rustudio_account::VersionType::Saved
                };
                if self.doc.cloud.as_ref().is_some_and(|o| o.local_copy) {
                    self.cloud.confirm_copy_save = Some(vt);
                } else {
                    self.save_cloud(vt, false);
                }
            }
            "save_roblox" | "publish" => {
                self.account.show_publish = true;
            }
            "account" => self.account.open = true,
            "add_account" => {
                self.account.open = true;
                self.account.adding = true;
            }
            "experience_settings" => self.cloud.settings_open = true,
            "avatar_settings" => self.ui.panel_requests.push((Tab::AvatarSettings, None)),
            "terrain_editor" => self.ui.panel_requests.push((Tab::TerrainEditor, None)),
            "about" => self.ui.about = true,
            "restart" => self.request(PendingAction::Restart),
            "exit" => self.request(PendingAction::Quit),
            "undo" | "redo" | "cut" | "copy" | "paste" | "paste_into" | "duplicate" | "delete" | "group"
            | "ungroup" => {
                self.run_action(a);
                if matches!(a, "cut" | "copy") {
                    self.note_os_clipboard(ctx);
                }
            }
            "paste_original" => self.run_action("paste"),
            "paste_into_original" => self.run_action("paste_into"),
            "group_folder" => {
                let sel = self.doc.selection.clone();
                if let Some(f) = self.doc.group_as(&sel, "Folder") {
                    self.select(vec![f]);
                }
            }
            "lock" => {
                let on = !self.all_parts("Locked");
                self.set_on_parts("Lock", "Locked", on);
            }
            "anchor" => {
                let on = !self.all_parts("Anchored");
                self.set_on_parts("Anchor", "Anchored", on);
            }
            "unlock_all" => {
                let edits: Vec<_> = self
                    .doc
                    .dom
                    .descendants()
                    .filter(|i| crate::doc::is_a(i.class.as_str(), "BasePart"))
                    .map(|i| (i.referent(), rbx_dom_weak::ustr("Locked"), Variant::Bool(false)))
                    .collect();
                self.doc.set_props("Unlock All", edits, None);
            }
            "select_all" => {
                // Studio: everything directly in Workspace but the camera and terrain.
                if let Some(ws) = self.doc.workspace() {
                    let kids: Vec<Ref> = self
                        .doc
                        .dom
                        .get_by_ref(ws)
                        .map(|i| i.children().to_vec())
                        .unwrap_or_default();
                    let all = kids
                        .into_iter()
                        .filter(|r| !matches!(self.doc.class_of(*r).as_deref(), Some("Camera" | "Terrain")))
                        .collect();
                    self.select(all);
                }
            }
            "zoom_in" => self.ui.zoom = (self.ui.zoom + 0.1).min(2.0),
            "zoom_out" => self.ui.zoom = (self.ui.zoom - 0.1).max(0.6),
            "full_screen" => {
                self.ui.fullscreen = !self.ui.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.ui.fullscreen));
            }
            "show_grid" => self.show_grid = !self.show_grid,
            "grid_2" => self.ui.grid_size = 2.0,
            "grid_4" => self.ui.grid_size = 4.0,
            "grid_16" => self.ui.grid_size = 16.0,
            "show_ui" => self.ui.show_gui = !self.ui.show_gui,
            "screenshot" => {
                let dir = crate::paths::pictures();
                let _ = std::fs::create_dir_all(&dir);
                let secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                self.pending_user_shot = Some(dir.join(format!("rustudio-{secs}.png")));
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            "mute" => {
                let on = !crate::audio::muted();
                crate::audio::set_muted_global(on);
                if let Some(s) = self.play.as_mut() {
                    s.game.set_muted(on);
                }
            }
            "test" => self.start_run(RunType::Test),
            "test_here" => self.start_run(RunType::TestHere),
            "run" => self.start_run(RunType::Run),
            "toggle_client_server" => self.play_shows_client = !self.play_shows_client,
            "stop" => self.stop_play(),
            "panel_properties" => self.ui.panel_requests.push((Tab::Properties, None)),
            "panel_explorer" => self.ui.panel_requests.push((Tab::Explorer, None)),
            "panel_output" => self.ui.panel_requests.push((Tab::Output, None)),
            "insert_object" => {
                let parent = self.insert_parent();
                let at = ctx.content_rect().center() - Vec2::new(120.0, 200.0);
                self.open_insert_popup(parent, at);
            }
            "reset_layout" => {
                ctx.data_mut(|d| d.insert_temp(egui::Id::new("reset_dock"), true));
            }
            "documentation" => ctx.open_url(egui::OpenUrl::new_tab("https://create.roblox.com/docs")),
            "tutorials" => ctx.open_url(egui::OpenUrl::new_tab("https://create.roblox.com/docs/tutorials")),
            "developer_forum" => ctx.open_url(egui::OpenUrl::new_tab("https://devforum.roblox.com")),
            "tool_select" => self.tool = Tool::Select,
            "tool_move" => self.tool = Tool::Move,
            "tool_scale" => self.tool = Tool::Scale,
            "tool_rotate" => self.tool = Tool::Rotate,
            "part" | "part_block" => self.insert_part(MeshKind::Block),
            "part_sphere" => self.insert_part(MeshKind::Ball),
            "part_wedge" => self.insert_part(MeshKind::Wedge),
            "part_corner" => self.insert_part(MeshKind::CornerWedge),
            "part_cylinder" => self.insert_part(MeshKind::Cylinder),
            "gui" => {
                let sg = self.service("StarterGui");
                self.insert_class("ScreenGui", sg);
            }
            "script" => {
                // Into the selection, else ServerScriptService (Studio's Script button).
                let p = match self.doc.selection.first() {
                    Some(r) => *r,
                    None => self.service("ServerScriptService"),
                };
                self.insert_class("Script", p);
            }
            "attachment" => {
                for p in self.selected_parts() {
                    self.insert_class("Attachment", p);
                }
            }
            "weld" => {
                let parts = self.selected_parts();
                let (p0, p1) = (parts[0], parts[1]);
                let builder = rbx_dom_weak::InstanceBuilder::new("WeldConstraint")
                    .with_name("WeldConstraint")
                    .with_property("Part0", Variant::Ref(p0))
                    .with_property("Part1", Variant::Ref(p1));
                let w = self.doc.insert(p0, builder);
                self.select(vec![w]);
            }
            other => {
                if let Some(class) = other.strip_prefix("class:") {
                    let class = class.to_string();
                    let gui_class = class.starts_with("UI") || crate::doc::is_a(&class, "GuiObject");
                    match self.gui_target().filter(|_| gui_class) {
                        Some(t) => self.insert_class(&class, t),
                        None => {
                            for p in self.selected_parts() {
                                self.insert_class(&class, p);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Starts the selected kind of test session (Studio's Play button).
    pub fn start_run(&mut self, t: RunType) {
        use crate::runtime::RunMode;
        if self.play.is_some() || !t.available() {
            return;
        }
        self.ui.run_type = t;
        match t {
            RunType::Test => {
                self.play_here = None;
                self.start_play(RunMode::Play);
            }
            RunType::TestHere => {
                // The character starts where the camera is (Studio's Test Here).
                self.play_here = Some(self.cam.pos);
                self.start_play(RunMode::Play);
            }
            RunType::Run => {
                self.play_here = None;
                self.start_play(RunMode::Run);
            }
            RunType::ServerClients => {}
        }

        // switch window to main viewport when playtesting
        // (the dock lives on RustudioApp, which consumes this flag in ui())
        self.switch_to_viewport = true;
    }

    /// The menu bar: Studio's menus in Studio's order.
    pub fn studio_menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut chosen: Option<String> = None;
        egui::MenuBar::new().ui(ui, |ui| {
            for (title, items) in MENUS {
                let r = ui.menu_button(*title, |ui| {
                    self.menu_items(ui, items, &mut chosen);
                });
                self.ui_rects.insert(menu_rect_key(title), r.response.rect);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = if self.doc.dirty() { " •" } else { "" };
                // A place from Roblox: its experience and creator, and
                // whether Team Create is on (File > Experience Settings).
                let cloud = match &self.doc.cloud {
                    Some(o) => {
                        let tc = match self.cloud.status.as_ref().filter(|(u, _)| *u == o.universe_id) {
                            Some((_, s)) if s.enabled == Some(true) => " · Team Create on",
                            _ => "",
                        };
                        format!(" @ {} ({}) v{}{tc}", o.experience, o.creator_label(), o.version)
                    }
                    None => String::new(),
                };
                let r = ui.label(RichText::new(format!("{}{dirty}{cloud} - rustudio", self.doc.title)).color(theme::TEXT_DIM));
                if let Some((_, s)) = self.cloud.status.as_ref().filter(|_| self.doc.cloud.is_some()) {
                    r.on_hover_text(s.describe());
                }
            });
        });
        if let Some(a) = chosen {
            self.ui_action(&ctx, &a);
        }
    }

    fn menu_items(&mut self, ui: &mut egui::Ui, items: &'static [M], chosen: &mut Option<String>) {
        let (col, w) = menu_metrics(ui, items);
        ui.set_width(w);
        for m in items {
            match m {
                M::S => {
                    ui.separator();
                }
                M::I(label, sc, a) => {
                    let (en, ck) = self.action_state(a);
                    if menu_item_at(ui, label, sc, en, ck, col, w).clicked() {
                        *chosen = Some(a.to_string());
                        ui.close();
                    }
                }
                M::Sub("Recent", _) => {
                    let recent = self.ui.recent.clone();
                    ui.add_enabled_ui(!recent.is_empty(), |ui| {
                        ui.menu_button(sub_label("Recent"), |ui| {
                            for p in recent {
                                if ui.button(p.display().to_string()).clicked() {
                                    self.request(PendingAction::Open(Some(p)));
                                    ui.close();
                                }
                            }
                        });
                    });
                }
                M::Sub(label, sub) => {
                    ui.menu_button(sub_label(label), |ui| {
                        self.menu_items(ui, sub, chosen);
                    });
                }
            }
        }
    }

    /// The menus' own accelerators (Ctrl+Shift+V, Alt+A, F11…), as actions.
    pub fn menu_shortcut_actions(&mut self, ctx: &egui::Context) -> Vec<&'static str> {
        let mut out = Vec::new();
        ctx.input_mut(|i| {
            for (k, a) in menu_shortcuts() {
                // Clipboard keys come as egui events; Del, F5, F8 and
                // Shift+F5 go through the editor's own paths.
                if matches!(a, "cut" | "copy" | "paste" | "test" | "run" | "stop" | "delete") {
                    continue;
                }
                if i.consume_shortcut(&k) {
                    out.push(a);
                }
            }
        });
        out
    }

    /// File > About rustudio.
    pub fn about_ui(&mut self, ctx: &egui::Context) {
        if !self.ui.about {
            return;
        }
        let mut open = true;
        egui::Window::new("About rustudio")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                crate::icons::logo(ui, 32.0);
                ui.label(RichText::new(format!("rustudio {}", env!("CARGO_PKG_VERSION"))).strong());
                ui.label("A native Linux editor for Roblox places. Not made by Roblox.");
                ui.label("Icons: Tabler Icons (MIT). See CREDITS.md.");
            });
        self.ui.about = open;
    }
}
