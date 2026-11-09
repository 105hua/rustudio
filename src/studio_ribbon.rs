//! Studio's top row (run type, Play / Pause / Stop, the ribbon tabs, the
//! account) and the ribbon tabs' buttons, as in an offline Studio
//! (checks/ui/studio/ribbon-*.png).

use egui::{Color32, Rect, RichText, Sense, Vec2};

use crate::app::{Editor, Tool};
use crate::studio_ui::{run_type_key, tab_rect_key, RibbonTab, RunType};
use crate::theme;

fn divider(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 46.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, theme::DIVIDER);
    ui.add_space(4.0);
}

/// Studio's ribbon button: a 24-px icon over an 11-px label.
pub fn ribbon_button(ui: &mut egui::Ui, icon: &str, label: &str, active: bool, enabled: bool) -> egui::Response {
    let font = egui::FontId::proportional(11.5);
    let tw = ui.fonts_mut(|f| f.layout_no_wrap(label.into(), font.clone(), theme::TEXT).size().x);
    let w = (tw + 10.0).max(48.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 50.0), sense);
    let p = ui.painter();
    let icon_r = Rect::from_center_size(r.center() - Vec2::new(0.0, 8.0), Vec2::splat(24.0));
    if active {
        p.rect_filled(icon_r.expand(5.0), 4.0, theme::ACTIVE_TAB.gamma_multiply(1.5));
    } else if enabled && resp.hovered() {
        p.rect_filled(r, 4.0, theme::HOVER_BG);
    }
    let col = if !enabled {
        theme::TEXT_DIM.gamma_multiply(0.8)
    } else if active {
        Color32::WHITE
    } else {
        theme::TEXT
    };
    crate::ti::paint(p, icon_r, icon, col);
    p.text(egui::pos2(r.center().x, r.max.y - 7.0), egui::Align2::CENTER_CENTER, label, font, col);
    resp
}

/// A top-row icon button (Play, Pause, Stop, Notifications…).
fn top_icon(ui: &mut egui::Ui, icon: &str, color: Color32, enabled: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(28.0, 24.0), sense);
    if enabled && resp.hovered() {
        ui.painter().rect_filled(r, 3.0, theme::HOVER_BG);
    }
    let col = if enabled { color } else { color.gamma_multiply(0.35) };
    crate::ti::paint(ui.painter(), r.shrink2(Vec2::new(6.0, 4.0)), icon, col);
    resp
}

fn tool_key(t: Tool) -> &'static str {
    match t {
        Tool::Select => "tool.select",
        Tool::Move => "tool.move",
        Tool::Scale => "tool.scale",
        Tool::Rotate => "tool.rotate",
    }
}

impl Editor {
    /// Under the menus: run type, Play / Pause / Stop, the ribbon tabs in
    /// the middle, the account at the right (Studio's layout).
    pub fn studio_top_row(&mut self, ui: &mut egui::Ui) {
        let mode = self.play.as_ref().map(|p| p.mode);
        let full = ui.max_rect();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
            let mut picked = None;
            let resp = egui::ComboBox::from_id_salt("run_type")
                .width(80.0)
                .selected_text(self.ui.run_type.label())
                .show_ui(ui, |ui| {
                    for t in RunType::ALL {
                        let b = egui::Button::selectable(self.ui.run_type == t, t.label());
                        let r = ui.add_enabled(t.available(), b);
                        self.ui_rects.insert(run_type_key(t), r.rect);
                        if r.clicked() {
                            picked = Some(t);
                        }
                    }
                })
                .response;
            self.ui_rects.insert("test.type", resp.rect);
            if let Some(t) = picked {
                self.ui.run_type = t;
            }
            let key = match self.ui.run_type {
                RunType::Test => " (F5)",
                RunType::Run => " (F8)",
                _ => "",
            };
            let tip = format!("{}{key}", self.ui.run_type.label());
            let resp = top_icon(ui, "player-play-filled", theme::ACCENT, mode.is_none()).on_hover_text(tip);
            self.ui_rects.insert("test.play", resp.rect);
            if mode.is_none() && resp.clicked() {
                self.start_run(self.ui.run_type);
            }
            let resp = top_icon(ui, "player-pause-filled", theme::TEXT, false).on_hover_text("Pause: not in rustudio yet");
            self.ui_rects.insert("test.pause", resp.rect);
            let resp = top_icon(ui, "player-stop-filled", Color32::from_rgb(200, 60, 60), mode.is_some())
                .on_hover_text("Stop (Shift+F5)");
            self.ui_rects.insert("test.stop", resp.rect);
            if mode.is_some() && resp.clicked() {
                self.stop_play();
            }
            top_icon(ui, "device-mobile", theme::TEXT, false).on_hover_text("Device Simulator: not in rustudio yet");

            // The tabs, centred like Studio's.
            let tab_w = 80.0;
            let x0 = full.center().x - tab_w * 7.0 / 2.0;
            let y = ui.min_rect().min.y;
            for (i, t) in RibbonTab::ALL.into_iter().enumerate() {
                let r = Rect::from_min_size(egui::pos2(x0 + i as f32 * tab_w, y), egui::vec2(tab_w - 4.0, 24.0));
                let resp = ui.interact(r, egui::Id::new(("ribbon_tab", i)), Sense::click());
                let active = self.ui.tab == t;
                let bg = if active {
                    theme::ACTIVE_TAB
                } else if resp.hovered() {
                    theme::HOVER_BG
                } else {
                    Color32::TRANSPARENT
                };
                ui.painter().rect_filled(r, 4.0, bg);
                let col = if active { theme::TEXT } else { theme::TEXT.gamma_multiply(0.75) };
                let font = egui::FontId::proportional(13.0);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, t.label(), font, col);
                self.ui_rects.insert(tab_rect_key(t), r);
                if resp.clicked() {
                    self.ui.tab = t;
                }
            }
            let plus = Rect::from_min_size(egui::pos2(x0 + 6.0 * tab_w + 26.0, y), egui::vec2(24.0, 24.0));
            let font = egui::FontId::proportional(15.0);
            ui.painter().text(plus.center(), egui::Align2::CENTER_CENTER, "+", font, theme::TEXT_DIM);
            ui.interact(plus, egui::Id::new("ribbon_tab_add"), Sense::hover())
                .on_hover_text("Custom tabs: not in rustudio yet");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
                // The account: a round button and its menu (Add Account, Log Out).
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
                let signed = self.account.signed_in();
                let fill = if signed { theme::ACCENT.gamma_multiply(0.8) } else { theme::PANEL_ALT };
                ui.painter().circle_filled(r.center(), 12.0, fill);
                crate::ti::paint(ui.painter(), r.shrink(5.0), "user", theme::TEXT);
                self.ui_rects.insert("account", r);
                let resp = resp.on_hover_text(self.account.status_short());
                if resp.clicked() {
                    self.account.open = !self.account.open;
                    self.account.adding = false;
                }
                top_icon(ui, "bell", theme::TEXT, false).on_hover_text("Notifications: not in rustudio yet");
                let tc = match (&self.doc.cloud, &self.cloud.status) {
                    (Some(o), Some((u, s))) if *u == o.universe_id => s.describe(),
                    _ => "Open a Team Create place to see its live collaborators".into(),
                };
                if top_icon(ui, "users-group", theme::TEXT, false).on_hover_text(tc).clicked() {self.cloud.live_collaborators_open=true;}
                top_icon(ui, "share", theme::TEXT, false).on_hover_text("Share: not in rustudio yet");
                if ui.button("Collaborate").clicked() {self.cloud.live_collaborators_open=true;}
            });
        });
    }

    fn ribbon_tools(&mut self, ui: &mut egui::Ui, act: &mut Option<String>) {
        for (icon, label, a, tool) in [
            ("pointer", "Select", "tool_select", Tool::Select),
            ("arrows-move", "Move", "tool_move", Tool::Move),
            ("resize", "Scale", "tool_scale", Tool::Scale),
            ("rotate-clockwise-2", "Rotate", "tool_rotate", Tool::Rotate),
        ] {
            let on = self.tool == tool;
            let r = self.rb(ui, icon, label, a, on, act);
            self.ui_rects.insert(tool_key(tool), r);
            self.hud.toolbar_buttons += 1;
        }
        self.rb(ui, "transform", "Transform", "transform", false, act);
        divider(ui);
        self.rb(ui, "geometry", "Geometric", "geometric", false, act);
        self.snap_widget(ui);
        divider(ui);
    }

    /// The selected ribbon tab's buttons, grouped with dividers like Studio's.
    pub fn studio_ribbon(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut act: Option<String> = None;
        let a = &mut act;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
            match self.ui.tab {
                RibbonTab::Home => {
                    self.ribbon_tools(ui, a);
                    self.part_button(ui, a);
                    self.rb(ui, "mountain", "Terrain", "terrain_editor", false, a);
                    self.rb(ui, "user-plus", "Character", "character", false, a);
                    self.rb(ui, "app-window", "GUI", "gui", false, a);
                    self.rb(ui, "file-code", "Script", "script", false, a);
                    self.rb(ui, "file-import", "Import", "import_model", false, a);
                    divider(ui);
                    self.material_quick_button(ui);
                    self.color_quick_button(ui);
                    self.rb(ui, "box-multiple", "Group", "group", false, a);
                    let lock = self.action_state("lock").1 == Some(true);
                    self.rb(ui, "lock", "Lock", "lock", lock, a);
                    let anchored = self.action_state("anchor").1 == Some(true);
                    let r = self.rb(ui, "anchor", "Anchor", "anchor", anchored, a);
                    self.ui_rects.insert("home.anchor", r);
                    divider(ui);
                    let e = self.ui.panels_open.contains("Explorer");
                    self.rb(ui, "list-tree", "Explorer", "panel_explorer", e, a);
                    let p = self.ui.panels_open.contains("Properties");
                    self.rb(ui, "list-details", "Properties", "panel_properties", p, a);
                    self.rb(ui, "tools", "Toolbox", "toolbox", false, a);
                    self.rb(ui, "folders", "Assets", "asset_manager", false, a);
                }
                RibbonTab::Model => {
                    self.ribbon_tools(ui, a);
                    self.rb(ui, "focus-centered", "Pivot", "pivot", false, a);
                    self.rb(ui, "restore", "Reset", "pivot_reset", false, a);
                    divider(ui);
                    self.part_button(ui, a);
                    let fx = ["ParticleEmitter", "Beam", "Trail", "Fire", "Smoke", "Sparkles", "Highlight"];
                    self.class_menu_button(ui, "sparkles", "Effect", "effect", &fx, a);
                    divider(ui);
                    self.rb(ui, "circle-dot", "Attachment", "attachment", false, a);
                    self.rb(ui, "link", "Constraint", "constraint", false, a);
                    self.rb(ui, "link-plus", "Weld", "weld", false, a);
                    divider(ui);
                    self.material_quick_button(ui);
                    self.color_quick_button(ui);
                    self.rb(ui, "photo", "Texture", "texture_tool", false, a);
                    self.rb(ui, "box-multiple", "Group", "group", false, a);
                    let lock = self.action_state("lock").1 == Some(true);
                    self.rb(ui, "lock", "Lock", "lock", lock, a);
                    let anchored = self.action_state("anchor").1 == Some(true);
                    self.rb(ui, "anchor", "Anchor", "anchor", anchored, a);
                    divider(ui);
                    self.rb(ui, "layout-align-center", "Align", "align", false, a);
                }
                RibbonTab::Avatar => {
                    self.ribbon_tools(ui, a);
                    let open = self.ui.panels_open.contains("Avatar Settings");
                    let r = self.rb(ui, "user-cog", "Settings", "avatar_settings", open, a);
                    self.ui_rects.insert("avatar.settings", r);
                    divider(ui);
                    self.rb(ui, "user-plus", "Character", "character", false, a);
                    self.rb(ui, "accessible", "Setup", "avatar_setup", false, a);
                    self.rb(ui, "hanger", "Accessory", "accessory_editor", false, a);
                    self.rb(ui, "shirt", "Adaptive…", "adaptive", false, a);
                    self.rb(ui, "checklist", "UGC Validati…", "ugc_validation", false, a);
                    divider(ui);
                    self.rb(ui, "movie", "Clip Editor", "clip_editor", false, a);
                    self.rb(ui, "chart-line", "Graph Editor", "graph_editor", false, a);
                }
                RibbonTab::Ui => {
                    self.rb(ui, "app-window", "GUI", "gui", false, a);
                    self.rb_class(ui, "square", "Frame", "ui_frame", "Frame", a);
                    self.rb_class(ui, "typography", "Label", "ui_label", "TextLabel", a);
                    self.rb_class(ui, "forms", "Input", "ui_input", "TextBox", a);
                    let ap = ["UICorner", "UIStroke", "UIGradient", "UIPadding", "UIScale"];
                    self.class_menu_button(ui, "border-corner-rounded", "Appearance", "ui_appearance", &ap, a);
                    let la = ["UIListLayout", "UIGridLayout", "UIPageLayout", "UITableLayout", "UIFlexItem"];
                    self.class_menu_button(ui, "layout-list", "Layout", "ui_layout", &la, a);
                    let co = ["UIAspectRatioConstraint", "UISizeConstraint", "UITextSizeConstraint"];
                    self.class_menu_button(ui, "aspect-ratio", "Constraint", "ui_constraint", &co, a);
                    divider(ui);
                    self.rb(ui, "brush", "Style Editor", "style_editor", false, a);
                }
                RibbonTab::Script => {
                    self.rb(ui, "chevron-left", "Back", "script_back", false, a);
                    self.rb(ui, "chevron-right", "Fwd", "script_fwd", false, a);
                    divider(ui);
                    self.rb(ui, "file-code", "Script", "script", false, a);
                    divider(ui);
                    self.rb(ui, "indent-increase", "Format", "format_document", false, a);
                    self.rb(ui, "search", "Find", "find_in_place", false, a);
                    self.rb(ui, "list-numbers", "Go to Line", "goto_line", false, a);
                    divider(ui);
                    self.rb(ui, "terminal-2", "Command", "command_bar", false, a);
                    let o = self.ui.panels_open.contains("Output");
                    self.rb(ui, "terminal", "Output", "panel_output", o, a);
                    divider(ui);
                    self.rb(ui, "point", "Breakpoints", "breakpoints", false, a);
                    self.rb(ui, "stack-2", "Call Stack", "call_stack", false, a);
                    self.rb(ui, "eye", "Watch", "watch", false, a);
                    divider(ui);
                    self.rb(ui, "chart-bar", "Analysis", "script_analysis", false, a);
                    self.rb(ui, "activity", "Script Activity", "script_activity", false, a);
                }
                RibbonTab::Plugins => {
                    self.rb(ui, "plug", "Manage Plugins", "manage_plugins", false, a);
                    self.rb(ui, "folder", "Plugins Folder", "plugins_folder", false, a);
                    ui.label(RichText::new("rustudio doesn't run Studio plugins yet").color(theme::TEXT_DIM).small());
                }
            }
        });
        self.ribbon_color_popup(&ctx);
        if let Some(a) = act {
            self.ui_action(&ctx, &a);
        }
    }

    /// A ribbon button for an action; disabled ones say so. Returns its rect.
    fn rb(&mut self, ui: &mut egui::Ui, icon: &str, label: &str, action: &str, active: bool, act: &mut Option<String>) -> Rect {
        let (enabled, _) = self.action_state(action);
        let resp = ribbon_button(ui, icon, label, active, enabled);
        let rect = resp.rect;
        if enabled {
            if resp.clicked() {
                *act = Some(action.to_string());
            }
        } else {
            resp.on_hover_text(format!("{label}: not in rustudio yet"));
        }
        rect
    }

    fn rb_class(&mut self, ui: &mut egui::Ui, icon: &str, label: &str, action: &str, class: &str, act: &mut Option<String>) {
        let (enabled, _) = self.action_state(action);
        let resp = ribbon_button(ui, icon, label, false, enabled);
        if enabled {
            if resp.clicked() {
                *act = Some(format!("class:{class}"));
            }
        } else {
            resp.on_hover_text(format!("Select a GUI object to add a {class} to it"));
        }
    }

    /// A ribbon button whose drop-down inserts one of `classes` into the selection.
    fn class_menu_button(&mut self, ui: &mut egui::Ui, icon: &str, label: &str, action: &str, classes: &[&str], act: &mut Option<String>) {
        let (enabled, _) = self.action_state(action);
        let resp = ribbon_button(ui, icon, label, false, enabled);
        if !enabled {
            resp.on_hover_text(format!("{label}: select where it goes first"));
            return;
        }
        egui::Popup::menu(&resp).show(|ui| {
            for c in classes {
                ui.horizontal(|ui| {
                    crate::icons::class_icon(ui, c, 16.0);
                    if ui.selectable_label(false, *c).clicked() {
                        *act = Some(format!("class:{c}"));
                        ui.close();
                    }
                });
            }
        });
    }

    /// Part: inserts a block; the arrow beside it lists the shapes (Studio).
    fn part_button(&mut self, ui: &mut egui::Ui, act: &mut Option<String>) {
        let enabled = self.action_state("part").0;
        let resp = ribbon_button(ui, "cube", "Part", false, enabled);
        self.ui_rects.insert("insert.part", resp.rect);
        self.hud.toolbar_buttons += 1;
        let arrow = Rect::from_min_size(resp.rect.right_top() + Vec2::new(-13.0, 18.0), Vec2::splat(12.0));
        crate::ti::paint(ui.painter(), arrow, "chevron-down", theme::TEXT_DIM);
        let ar = ui.interact(arrow, egui::Id::new("part_arrow"), Sense::click());
        if enabled && resp.clicked() && !ar.hovered() {
            *act = Some("part".into());
        }
        egui::Popup::menu(&ar).show(|ui| {
            for (label, a) in [
                ("Block", "part_block"),
                ("Sphere", "part_sphere"),
                ("Wedge", "part_wedge"),
                ("Corner Wedge", "part_corner"),
                ("Cylinder", "part_cylinder"),
            ] {
                if ui.button(label).clicked() {
                    *act = Some(a.into());
                    ui.close();
                }
            }
        });
    }

    /// Studio's snap controls: a check box and an increment for moves, then
    /// for rotations. Studio turns a snap on when its increment is changed to
    /// more than zero.
    fn snap_widget(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.snap_move, "").on_hover_text("Snap moves to the grid");
                let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                crate::ti::paint(ui.painter(), r, "arrows-move", theme::TEXT_DIM);
                let dv = egui::DragValue::new(&mut self.move_inc).range(0.01..=512.0).speed(0.1).suffix(" studs");
                if ui.add(dv).changed() && self.move_inc > 0.0 {
                    self.snap_move = true;
                }
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.snap_rotate, "").on_hover_text("Snap rotation");
                let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                crate::ti::paint(ui.painter(), r, "rotate", theme::TEXT_DIM);
                let dv = egui::DragValue::new(&mut self.rotate_inc).range(0.1..=180.0).speed(0.5).suffix("°");
                if ui.add(dv).changed() && self.rotate_inc > 0.0 {
                    self.snap_rotate = true;
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    /// Every icon the top row and the ribbon name exists in the bundled set.
    #[test]
    fn icons_exist() {
        let all = include_str!("studio_ribbon.rs");
        // (the code above this test module, not the test's own strings)
        let src = &all[..all.find("#[cfg(test)]").unwrap_or(all.len())];
        let quoted: Vec<&str> = src.split('"').collect();
        let mut missing = Vec::new();
        for w in quoted.windows(2) {
            let calls = ["rb(ui, ", "top_icon(ui, ", "class_menu_button(ui, ", "rb_class(ui, ", "ribbon_button(ui, "];
            if calls.iter().any(|p| w[0].ends_with(p)) && !crate::ti::has(w[1]) {
                missing.push(w[1]);
            }
        }
        for i in ["pointer", "arrows-move", "resize", "rotate-clockwise-2", "user", "chevron-down", "rotate", "check"] {
            if !crate::ti::has(i) {
                missing.push(i);
            }
        }
        assert!(missing.is_empty(), "icons not in Tabler: {missing:?}");
    }
}
