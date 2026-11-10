//! The Start Page: what Studio shows when no place is open. One scrolling
//! page holds everything: template cards for a new place, the experiences
//! recently opened from Roblox, My Experiences, Shared with Me, one section
//! per group, and the files opened from disk. The search box filters all of
//! it; the sidebar jumps to a section; headers collapse. Plus an open-by-ID
//! box. Picking
//! anything opens it and the editor takes over; File > Close Place comes
//! back here.
//!
//! Lists load on their own threads (as cloud_ui.rs does) through the same
//! CloudApi; opening goes through `Editor::open_cloud` / `PendingAction`, so
//! Team Create prompts, the download cache and offline reopen behave the
//! same as File > Open from Roblox.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use rustudio_account::cloud::GroupInfo;
use rustudio_account::{parse_place_ref, CloudApi, CreatorKind, Experience, ExperienceSource, PublishKeyStore};

use crate::app::{Editor, PendingAction};
use crate::cloud_ui::Paged;
use crate::doc::Document;
use crate::theme;

/// A new-place template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    Baseplate,
    FlatTerrain,
}

impl Template {
    const ALL: [Template; 2] = [Template::Baseplate, Template::FlatTerrain];

    fn label(self) -> &'static str {
        match self {
            Template::Baseplate => "Baseplate",
            Template::FlatTerrain => "Flat Terrain",
        }
    }

    /// The template's place, unsaved and clean.
    pub fn build(self) -> Document {
        let mut doc = Document::new_place();
        if self == Template::FlatTerrain {
            flat_terrain(&mut doc);
        }
        doc.title = self.label().to_string();
        doc
    }
}

/// Flat Terrain: no Baseplate, a 1024×1024 grass slab whose top is y = 0.
fn flat_terrain(doc: &mut Document) {
    use rbx_dom_weak::types::{BinaryString, Variant};
    let Some(ws) = doc.workspace() else { return };
    let base: Vec<_> = doc
        .dom
        .get_by_ref(ws)
        .map(|w| w.children().to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter(|r| doc.dom.get_by_ref(*r).is_some_and(|i| i.name == "Baseplate"))
        .collect();
    for r in base {
        doc.dom.destroy(r);
    }
    let Some(t) = doc.find_child(ws, "Terrain") else { return };
    let Some(grass) = crate::terrain::material_index("Grass") else { return };
    let (half, depth) = (512.0f32, 16.0f32);
    let mut v = crate::terrain::Voxels::empty();
    let (lo, hi) = ([-half, -depth, -half], [half, 0.0, half]);
    v.fill(lo, hi, grass, |p| {
        let c = [0.0, -depth / 2.0, 0.0];
        let h = [half, depth / 2.0, half];
        let q = [0, 1, 2].map(|k| (p[k] - c[k]).abs() - h[k]);
        let out = q.map(|x| x.max(0.0));
        (out[0] * out[0] + out[1] * out[1] + out[2] * out[2]).sqrt() + q[0].max(q[1]).max(q[2]).min(0.0)
    });
    if let Some(i) = doc.dom.get_by_ref_mut(t) {
        i.properties
            .insert(rbx_dom_weak::ustr("SmoothGrid"), Variant::BinaryString(BinaryString::from(v.encode())));
    }
}

type Listing = Option<Result<Vec<Experience>, String>>;

enum Msg {
    Page(ExperienceSource, u32, Result<(Vec<Experience>, bool), String>),
    Groups(Result<Vec<GroupInfo>, String>),
    Icon(u64, egui::ColorImage),
    Done,
}

pub struct StartPage {
    pub visible: bool,
    /// The document that was open when the page came up: another one means
    /// something was opened, so the page goes away.
    pub doc_id: u64,
    /// Sections the person folded away (by key: recent, mine, shared,
    /// group:<id>, files).
    collapsed: HashSet<String>,
    /// Card sections opened past their first two rows ("Show all").
    expanded: HashSet<String>,
    /// A sidebar click: scroll that section's header into view.
    jump: Option<String>,
    search: String,
    url: String,
    url_error: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    jobs: usize,
    lists: HashMap<ExperienceSource, Paged>,
    groups: Option<Result<Vec<GroupInfo>, String>>,
    icons: HashMap<u64, egui::TextureHandle>,
    icons_asked: HashSet<u64>,
    /// Whether the lists were loaded for a signed-in session yet.
    loaded_signed_in: bool,
}

impl StartPage {
    /// Drops the lists of the previous account (account switch); they load
    /// again for the new one.
    pub fn forget_lists(&mut self) {
        self.lists.clear();
        self.groups = None;
        self.loaded_signed_in = false;
    }
}

impl Default for StartPage {
    fn default() -> Self {
        let (tx, rx) = channel();
        StartPage {
            visible: false,
            doc_id: 0,
            collapsed: HashSet::new(),
            expanded: HashSet::new(),
            jump: None,
            search: String::new(),
            url: String::new(),
            url_error: None,
            tx,
            rx,
            jobs: 0,
            lists: HashMap::new(),
            groups: None,
            icons: HashMap::new(),
            icons_asked: HashSet::new(),
            loaded_signed_in: false,
        }
    }
}

/// What a click on the page asks for (applied after drawing).
enum Do {
    Jump(String),
    Toggle(String),
    ShowAll(String),
    Template(Template),
    OpenFile(Option<PathBuf>),
    OpenFolder,
    Cloud(u64, bool),
    Forget(u64),
    ForgetFile(PathBuf),
    Refresh,
    More(ExperienceSource),
    Account,
    Copy(String),
}

const CARD_W: f32 = 176.0;
const THUMB_H: f32 = 112.0;
const CARD_H: f32 = THUMB_H + 50.0;
const GAP: f32 = 16.0;

impl StartPage {
    fn api(ed_client: Option<crate::assets::SharedClient>, ep: rustudio_account::Endpoints) -> Option<CloudApi> {
        let session = {
            let g = ed_client.as_ref()?.lock().ok()?;
            g.as_ref()?.session.downloader()
        };
        let key = PublishKeyStore::load().ok().flatten();
        Some(CloudApi::new(session, ep, key))
    }

    fn spawn(
        &mut self,
        client: Option<crate::assets::SharedClient>,
        ep: rustudio_account::Endpoints,
        f: impl FnOnce(Option<CloudApi>, &Sender<Msg>) + Send + 'static,
    ) {
        self.jobs += 1;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            f(Self::api(client, ep), &tx);
            let _ = tx.send(Msg::Done);
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Page(src, page, r) => {
                    if let Some(p) = self.lists.get_mut(&src) {
                        p.arrived(page, r);
                    }
                }
                Msg::Groups(r) => self.groups = Some(r),
                Msg::Icon(u, img) => {
                    let t = ctx.load_texture(format!("start_icon_{u}"), img, egui::TextureOptions::LINEAR);
                    self.icons.insert(u, t);
                }
                Msg::Done => self.jobs = self.jobs.saturating_sub(1),
            }
        }
    }

    fn busy(&self) -> bool {
        self.jobs > 0
    }

    /// RUSTUDIO_START_PAGE=recent|mine|shared|files: start scrolled there.
    pub fn open_page(&mut self, name: &str) {
        if matches!(name, "recent" | "mine" | "shared" | "files") {
            self.jump = Some(name.to_string());
        }
    }
}

fn signed_out() -> String {
    "Sign in to Roblox to list your experiences.".into()
}

impl Editor {
    /// Shows the Start Page (startup with nothing to open, File > Close Place).
    pub fn show_start_page(&mut self) {
        self.start.visible = true;
        self.start.doc_id = self.doc.id;
        self.start.url_error = None;
    }

    fn start_client(&self) -> (Option<crate::assets::SharedClient>, rustudio_account::Endpoints) {
        (self.account.shared_client(), self.cloud.endpoints.clone())
    }

    /// (Re)loads every list: My Experiences, Shared with Me, the groups
    /// (each group's experiences follow once the groups arrive).
    fn start_load_all(&mut self) {
        let (client, ep) = self.start_client();
        let sp = &mut self.start;
        sp.icons_asked.clear();
        sp.lists.clear();
        sp.groups = Some(Ok(Vec::new()));
        sp.spawn(client, ep, |a, tx| {
            let r = a.ok_or_else(signed_out).and_then(|a| a.groups().map_err(|e| e.0));
            let _ = tx.send(Msg::Groups(r));
        });
        self.start_reset_list(ExperienceSource::Mine);
        self.start_reset_list(ExperienceSource::Shared);
    }

    fn start_reset_list(&mut self, src: ExperienceSource) {
        self.start.lists.insert(src, Paged::fresh());
        self.start_load_page(src);
    }

    fn start_load_page(&mut self, src: ExperienceSource) {
        let Some(page) = self.start.lists.get_mut(&src).and_then(Paged::next_page) else { return };
        let (client, ep) = self.start_client();
        self.start.spawn(client, ep, move |a, tx| {
            let r = a.ok_or_else(signed_out).and_then(|a| a.experiences_page(src, page, 25).map_err(|e| e.0));
            let _ = tx.send(Msg::Page(src, page, r));
        });
    }

    fn start_icons(&mut self, ids: Vec<u64>) {
        let ids: Vec<u64> = ids.into_iter().filter(|i| self.start.icons_asked.insert(*i)).collect();
        if ids.is_empty() {
            return;
        }
        let (client, ep) = self.start_client();
        self.start.spawn(client, ep, move |a, tx| {
            let Some(a) = a else { return };
            let Ok(urls) = a.icon_urls(&ids) else { return };
            for (id, url) in urls {
                let Ok(bytes) = a.image(&url) else { continue };
                let Ok(img) = image::load_from_memory(&bytes) else { continue };
                let img = img.thumbnail(150, 150).to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let _ = tx.send(Msg::Icon(id, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw())));
            }
        });
    }

    /// Draws the Start Page over the whole window. False once it has gone
    /// (something was opened): the editor draws this frame instead.
    pub fn start_page_ui(&mut self, ui: &mut egui::Ui) -> bool {
        if !self.start.visible {
            return false;
        }
        if self.doc.id != self.start.doc_id {
            self.start.visible = false;
            return false;
        }
        let ctx = ui.ctx().clone();
        self.account.restore();
        self.start.poll(&ctx);
        // Signing in (or the session check finishing) fills the lists.
        let signed = self.account.signed_in();
        if signed && !self.start.loaded_signed_in {
            self.start.loaded_signed_in = true;
            self.start_load_all();
        }
        // Each group's experiences, as soon as the groups are known.
        if let Some(Ok(groups)) = &self.start.groups {
            let missing: Vec<ExperienceSource> = groups
                .iter()
                .map(|g| ExperienceSource::Group(g.id))
                .filter(|s| !self.start.lists.contains_key(s))
                .collect();
            for src in missing {
                self.start_reset_list(src);
            }
        }
        let mut todo: Vec<Do> = Vec::new();
        // Ctrl+N / Ctrl+O / Ctrl+Shift+O, as in Studio's start page.
        if !ctx.egui_wants_keyboard_input() {
            ctx.input_mut(|i| {
                use egui::{Key, KeyboardShortcut, Modifiers};
                if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::N)) {
                    todo.push(Do::Template(Template::Baseplate));
                }
                if i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::O)) {
                    todo.push(Do::OpenFile(None));
                }
            });
        }

        egui::Panel::top("start_top")
            .exact_size(48.0)
            .frame(egui::Frame::new().fill(theme::MENU_BAR).inner_margin(egui::Margin::symmetric(16, 0)))
            .show(ui, |ui| self.start_top(ui, &mut todo));
        egui::Panel::left("start_nav")
            .exact_size(220.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(egui::Margin::symmetric(10, 14)))
            .show(ui, |ui| self.start_nav(ui, &mut todo));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(egui::Margin { left: 28, right: 20, top: 20, bottom: 12 }))
            .show(ui, |ui| self.start_body(ui, &mut todo));

        for d in todo {
            match d {
                Do::Jump(k) => {
                    self.start.collapsed.remove(&k);
                    self.start.jump = Some(k);
                }
                Do::Toggle(k) => {
                    if !self.start.collapsed.remove(&k) {
                        self.start.collapsed.insert(k);
                    }
                }
                Do::ShowAll(k) => {
                    if !self.start.expanded.remove(&k) {
                        self.start.expanded.insert(k);
                    }
                }
                Do::Refresh => self.start_load_all(),
                Do::More(src) => self.start_load_page(src),
                Do::Template(t) => {
                    let doc = t.build();
                    self.set_document(doc);
                    self.cam = crate::camera::Camera::default();
                    self.info(format!("New place from the {} template", t.label()));
                }
                Do::OpenFile(p) => self.request(PendingAction::Open(p)),
                Do::OpenFolder => self.request(PendingAction::OpenFolder),
                Do::Cloud(p, copy) => {
                    self.start.url_error = None;
                    self.open_cloud(p, None, copy);
                }
                Do::Forget(p) => {
                    let _ = self.cloud.cache.forget(p);
                }
                Do::ForgetFile(p) => {
                    self.ui.recent.retain(|r| *r != p);
                    crate::studio_ui::save_recent(&self.ui);
                }
                Do::Account => self.account.open = true,
                Do::Copy(s) => ctx.copy_text(s),
            }
        }
        if self.start.busy() || self.cloud.busy() || self.account.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        true
    }

    fn start_top(&mut self, ui: &mut egui::Ui, todo: &mut Vec<Do>) {
        ui.horizontal_centered(|ui| {
            crate::icons::logo(ui, 26.0);
            ui.add_space(4.0);
            ui.label(RichText::new("rustudio").size(16.0).strong().color(theme::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if self.account.signed_in() {
                    self.account.status_short().trim_start_matches("Roblox: ").to_string()
                } else {
                    "Sign in".to_string()
                };
                if ui
                    .add(egui::Button::new(RichText::new(label).color(theme::TEXT)).fill(theme::PANEL_ALT))
                    .on_hover_text("Roblox account")
                    .clicked()
                {
                    todo.push(Do::Account);
                }
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.start.search)
                        .hint_text("🔍  Search")
                        .desired_width(240.0),
                );
            });
        });
    }

    fn start_nav(&mut self, ui: &mut egui::Ui, todo: &mut Vec<Do>) {
        ui.spacing_mut().item_spacing.y = 2.0;
        let w = ui.available_width();
        let big = |ui: &mut egui::Ui, text: &str, fill: Color32| {
            ui.add_sized(
                [w, 34.0],
                egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
                    .fill(fill)
                    .corner_radius(CornerRadius::same(4)),
            )
        };
        if big(ui, "+  New Experience", theme::ACCENT).on_hover_text("A new Baseplate place (Ctrl+N)").clicked() {
            todo.push(Do::Template(Template::Baseplate));
        }
        ui.add_space(6.0);
        if big(ui, "Open from File…", theme::PANEL_ALT).on_hover_text("Ctrl+O").clicked() {
            todo.push(Do::OpenFile(None));
        }
        if big(ui, "Open Folder…", theme::PANEL_ALT)
            .on_hover_text("A *.project.json folder (files are the truth)")
            .clicked()
        {
            todo.push(Do::OpenFolder);
        }
        ui.add_space(14.0);
        ui.label(RichText::new("JUMP TO").small().color(theme::TEXT_DIM));
        ui.add_space(4.0);
        let mut entries: Vec<(String, String, bool)> = vec![
            ("recent".into(), "Recent".into(), false),
            ("mine".into(), "My Experiences".into(), false),
            ("shared".into(), "Shared with Me".into(), false),
        ];
        if let Some(Ok(groups)) = &self.start.groups {
            for g in groups {
                let has = self
                    .start
                    .lists
                    .get(&ExperienceSource::Group(g.id))
                    .map(|p| &p.list)
                    .is_none_or(|l| matches!(l, None | Some(Ok(_))) && !matches!(l, Some(Ok(v)) if v.is_empty()));
                if has && g.can_edit != Some(false) {
                    entries.push((format!("group:{}", g.id), g.name.clone(), true));
                }
            }
        }
        entries.push(("files".into(), "Recent Files".into(), false));
        egui::ScrollArea::vertical().id_salt("start_nav_scroll").max_height(ui.available_height() - 24.0).show(ui, |ui| {
            let mut groups_said = false;
            for (key, label, indent) in entries {
                if indent && !groups_said {
                    groups_said = true;
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new("Groups").color(theme::TEXT));
                    });
                }
                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(4), theme::HOVER_BG);
                }
                let x = r.min.x + if indent { 26.0 } else { 12.0 };
                let font = egui::FontId::proportional(if indent { 13.0 } else { 14.0 });
                let text = elide(ui.painter(), &label, &font, r.max.x - x - 6.0);
                ui.painter().text(
                    Pos2::new(x, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font,
                    if indent { theme::TEXT_DIM } else { theme::TEXT },
                );
                if resp.clicked() {
                    todo.push(Do::Jump(key));
                }
            }
        });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(
                RichText::new(format!("rustudio {}", env!("CARGO_PKG_VERSION")))
                    .small()
                    .color(theme::TEXT_DIM),
            );
        });
    }

    fn start_body(&mut self, ui: &mut egui::Ui, todo: &mut Vec<Do>) {
        // The open-by-ID row sits at the bottom, as in File > Open from Roblox.
        egui::Panel::bottom("start_url")
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 0 }))
            .show_separator_line(true)
            .show(ui, |ui| self.start_url_row(ui, todo));

        ui.horizontal(|ui| {
            ui.label(RichText::new("Home").size(22.0).strong().color(theme::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!self.start.busy(), egui::Button::new("Refresh")).clicked() {
                    todo.push(Do::Refresh);
                }
                if self.start.busy() || self.cloud.busy() {
                    ui.spinner();
                    if self.cloud.busy() {
                        ui.label(RichText::new("Opening…").color(theme::TEXT_DIM));
                    }
                }
            });
        });
        if let Some(e) = self.cloud.open_error.clone().or(self.start.url_error.clone()) {
            ui.label(RichText::new(e).color(theme::WARN));
        }
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .id_salt("start_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| self.start_home(ui, todo));
    }

    fn start_url_row(&mut self, ui: &mut egui::Ui, todo: &mut Vec<Do>) {
        ui.horizontal(|ui| {
            ui.label("Open by place ID or URL:");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.start.url)
                    .hint_text("roblox.com/games/… or a place ID")
                    .desired_width(320.0),
            );
            let go = ui.add_enabled(!self.start.url.trim().is_empty(), egui::Button::new("Open")).clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go {
                match parse_place_ref(&self.start.url) {
                    Ok(p) => todo.push(Do::Cloud(p.place_id, false)),
                    Err(e) => self.start.url_error = Some(e),
                }
            }
        });
    }

    fn section(ui: &mut egui::Ui, title: &str) {
        ui.label(RichText::new(title).size(16.0).strong().color(theme::TEXT));
        ui.add_space(8.0);
    }

    /// A foldable section header (click to fold). Returns whether it's open.
    fn fold_header(&mut self, ui: &mut egui::Ui, key: &str, title: &str, count: Option<String>, todo: &mut Vec<Do>) -> bool {
        let open = !self.start.collapsed.contains(key);
        let w = ui.available_width();
        let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::click());
        if self.start.jump.as_deref() == Some(key) {
            self.start.jump = None;
            ui.scroll_to_rect(r, Some(egui::Align::TOP));
        }
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(r, CornerRadius::same(4), theme::HOVER_BG);
        }
        // The fold arrow.
        let c = Pos2::new(r.min.x + 10.0, r.center().y);
        let tri = if open {
            vec![c + Vec2::new(-5.0, -3.0), c + Vec2::new(5.0, -3.0), c + Vec2::new(0.0, 4.0)]
        } else {
            vec![c + Vec2::new(-3.0, -5.0), c + Vec2::new(4.0, 0.0), c + Vec2::new(-3.0, 5.0)]
        };
        p.add(egui::Shape::convex_polygon(tri, theme::TEXT_DIM, Stroke::NONE));
        let font = egui::FontId::proportional(16.0);
        let tr = p.text(Pos2::new(r.min.x + 24.0, r.center().y), egui::Align2::LEFT_CENTER, title, font, theme::TEXT);
        if let Some(n) = count {
            p.text(
                Pos2::new(tr.max.x + 8.0, r.center().y + 1.0),
                egui::Align2::LEFT_CENTER,
                n,
                egui::FontId::proportional(12.5),
                theme::TEXT_DIM,
            );
        }
        // A thin rule to the right edge.
        p.line_segment(
            [Pos2::new(tr.max.x + 34.0, r.center().y), Pos2::new(r.max.x - 4.0, r.center().y)],
            Stroke::new(1.0, theme::DIVIDER),
        );
        if resp.clicked() {
            todo.push(Do::Toggle(key.to_string()));
        }
        ui.add_space(6.0);
        open
    }

    /// How many of a listing's experiences pass the search (None while loading / failed).
    fn listing_count(&self, src: ExperienceSource) -> Option<usize> {
        match self.start.lists.get(&src).and_then(|p| p.list.as_ref()) {
            Some(Ok(v)) if !v.is_empty() => Some(v.iter().filter(|e| self.matches(&e.name)).count()),
            _ => None,
        }
    }

    fn matches(&self, text: &str) -> bool {
        let q = self.start.search.trim().to_lowercase();
        q.is_empty() || text.to_lowercase().contains(&q)
    }

    fn start_home(&mut self, ui: &mut egui::Ui, todo: &mut Vec<Do>) {
        let searching = !self.start.search.trim().is_empty();
        let templates: Vec<Template> = Template::ALL.into_iter().filter(|t| self.matches(t.label())).collect();
        if !templates.is_empty() {
            Self::section(ui, "New Experience");
            grid(ui, templates.len(), |ui, i| {
                let t = templates[i];
                let r = card(ui, t.label(), "Template", |p, rect| template_thumb(p, rect, t));
                if r.clicked() {
                    todo.push(Do::Template(t));
                }
            });
            ui.add_space(14.0);
        }

        // Recently opened from Roblox (the download cache's list).
        let recent: Vec<_> = self
            .cloud
            .cache
            .recent()
            .into_iter()
            .filter(|r| self.matches(&r.origin.name) || self.matches(&r.origin.experience))
            .collect();
        if !(searching && recent.is_empty()) && self.fold_header(ui, "recent", "Recent", Some(recent.len().to_string()), todo) {
            if recent.is_empty() {
                ui.label(
                    RichText::new("Places you open from Roblox show up here.").color(theme::TEXT_DIM),
                );
            } else {
                let missing: Vec<u64> = recent
                    .iter()
                    .map(|r| r.origin.universe_id)
                    .filter(|u| !self.start.icons_asked.contains(u))
                    .collect();
                if !missing.is_empty() && self.account.signed_in() {
                    self.start_icons(missing);
                }
                let now = unix_now();
                let shown = self.capped(ui, "recent", recent.len());
                grid(ui, shown, |ui, i| {
                    let o = &recent[i].origin;
                    let sub = format!("Opened {}", ago(now, recent[i].opened_at));
                    let icon = self.start.icons.get(&o.universe_id).map(|t| t.id());
                    let r = card(ui, &o.name, &sub, |p, rect| icon_thumb(p, rect, icon, &o.name));
                    let r = r.on_hover_text(format!(
                        "{} by {}\nplace {} · universe {}",
                        o.experience,
                        o.creator_label(),
                        o.place_id,
                        o.universe_id
                    ));
                    if r.clicked() {
                        todo.push(Do::Cloud(o.place_id, false));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Open").clicked() {
                            todo.push(Do::Cloud(o.place_id, false));
                            ui.close();
                        }
                        if ui.button("Open a local copy").clicked() {
                            todo.push(Do::Cloud(o.place_id, true));
                            ui.close();
                        }
                        if ui.button("Copy place ID").clicked() {
                            todo.push(Do::Copy(o.place_id.to_string()));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Remove from Recent").clicked() {
                            todo.push(Do::Forget(o.place_id));
                            ui.close();
                        }
                    });
                });
                self.show_all_button(ui, "recent", recent.len(), shown, todo);
            }
            ui.add_space(8.0);
        }

        // My Experiences, Shared with Me, then each group.
        let mut lists: Vec<(String, String, ExperienceSource, String)> = vec![
            ("mine".into(), "My Experiences".into(), ExperienceSource::Mine, "This account owns no experiences.".into()),
            ("shared".into(), "Shared with Me".into(), ExperienceSource::Shared, "No experiences are shared with this account.".into()),
        ];
        let groups = match self.start.groups.clone() {
            Some(Ok(g)) => g,
            _ => Vec::new(),
        };
        for g in &groups {
            let src = ExperienceSource::Group(g.id);
            let l = self.start.lists.get(&src).and_then(|p| p.list.as_ref());
            // Groups with nothing in them, or that this account can't
            // edit / list, stay out of the way.
            if matches!(l, Some(Ok(v)) if v.is_empty()) && !self.start.busy() {
                continue;
            }
            if g.can_edit == Some(false) || matches!(l, Some(Err(_))) {
                continue;
            }
            lists.push((format!("group:{}", g.id), g.name.clone(), src, "This group has no experiences.".into()));
        }
        for (key, title, src, empty) in lists {
            let more = self.start.lists.get(&src).is_some_and(|p| p.more);
            let count = self.listing_count(src);
            // A search only sees the pages loaded so far, so a list with
            // more stays, with its Show more.
            if searching && count.unwrap_or(0) == 0 && !more {
                continue;
            }
            let count = count.map(|n| if more { format!("{n}+") } else { n.to_string() });
            if self.fold_header(ui, &key, &title, count, todo) {
                self.experience_grid(ui, &key, src, &empty, todo);
                ui.add_space(8.0);
            }
        }
        if let Some(Err(e)) = &self.start.groups {
            if self.account.signed_in() {
                ui.label(RichText::new(format!("Groups: {e}")).small().color(theme::WARN));
            }
        }

        let files = self.ui.recent.iter().filter(|p| self.matches(&p.display().to_string())).count();
        if !(searching && files == 0) && self.fold_header(ui, "files", "Recent Files", Some(files.to_string()), todo) {
            self.start_files(ui, usize::MAX, todo);
        }
        ui.add_space(24.0);
    }

    fn start_files(&mut self, ui: &mut egui::Ui, max: usize, todo: &mut Vec<Do>) {
        let files: Vec<PathBuf> = self
            .ui
            .recent
            .iter()
            .filter(|p| self.matches(&p.display().to_string()))
            // Home lists what still exists; Recent Files shows everything.
            .filter(|p| max == usize::MAX || p.exists())
            .take(max)
            .cloned()
            .collect();
        if files.is_empty() {
            ui.label(RichText::new("Files and folders you open show up here.").color(theme::TEXT_DIM));
            return;
        }
        let w = ui.available_width().min(900.0);
        for p in files {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
            let exists = p.exists();
            let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(4), theme::HOVER_BG);
            }
            let kind = if p.is_dir() {
                "Folder"
            } else {
                p.extension().and_then(|e| e.to_str()).unwrap_or("")
            };
            let badge = Rect::from_min_size(r.min + Vec2::new(8.0, 6.0), Vec2::new(28.0, 28.0));
            ui.painter().rect_filled(badge, CornerRadius::same(4), theme::PANEL_ALT);
            let tag = if p.is_dir() { "DIR".to_string() } else { kind.to_uppercase().chars().take(5).collect() };
            ui.painter().text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                tag,
                egui::FontId::proportional(8.5),
                theme::TEXT,
            );
            ui.painter().text(
                Pos2::new(badge.max.x + 12.0, r.min.y + 13.0),
                egui::Align2::LEFT_CENTER,
                &name,
                egui::FontId::proportional(14.0),
                if exists { theme::TEXT } else { theme::TEXT_DIM },
            );
            let sub = if exists {
                format!("{}  ·  {}", kind, p.parent().map(|d| d.display().to_string()).unwrap_or_default())
            } else {
                format!("missing  ·  {}", p.display())
            };
            ui.painter().text(
                Pos2::new(badge.max.x + 12.0, r.min.y + 29.0),
                egui::Align2::LEFT_CENTER,
                sub,
                egui::FontId::proportional(11.5),
                theme::TEXT_DIM,
            );
            if resp.clicked() && exists {
                if p.is_dir() {
                    self.open_folder(p.clone(), true);
                } else {
                    todo.push(Do::OpenFile(Some(p.clone())));
                }
            }
            resp.context_menu(|ui| {
                if ui.add_enabled(exists, egui::Button::new("Open")).clicked() {
                    todo.push(Do::OpenFile(Some(p.clone())));
                    ui.close();
                }
                if ui.button("Copy path").clicked() {
                    todo.push(Do::Copy(p.display().to_string()));
                    ui.close();
                }
                ui.separator();
                if ui.button("Remove from Recent").clicked() {
                    todo.push(Do::ForgetFile(p.clone()));
                    ui.close();
                }
            });
        }
    }

    fn start_signed_out_or_loading(&self, ui: &mut egui::Ui, todo: &mut Vec<Do>) {
        if self.account.signed_in() {
            ui.label(RichText::new("Loading…").color(theme::TEXT_DIM));
        } else {
            ui.label(RichText::new(signed_out()).color(theme::TEXT_DIM));
            if ui.button("Sign in…").clicked() {
                todo.push(Do::Account);
            }
        }
    }

    /// How many cards a section shows: two rows, unless opened with
    /// "Show all" or a search is narrowing it.
    fn capped(&self, ui: &egui::Ui, key: &str, n: usize) -> usize {
        if self.start.expanded.contains(key) || !self.start.search.trim().is_empty() {
            return n;
        }
        n.min(per_row(ui) * 2)
    }

    /// "Show all (N)" under a capped section, "Show less" once opened.
    fn show_all_button(&self, ui: &mut egui::Ui, key: &str, n: usize, shown: usize, todo: &mut Vec<Do>) {
        let open = self.start.expanded.contains(key);
        if shown >= n && !open {
            return;
        }
        if open && n <= per_row(ui) * 2 {
            return;
        }
        let label = if open { "Show less".to_string() } else { format!("Show {} more", n - shown) };
        if pill_button(ui, &label, true) {
            todo.push(Do::ShowAll(key.to_string()));
        }
        ui.add_space(10.0);
    }

    /// Under an experience section: "Show N more" opens it past its first
    /// two rows; once everything loaded is shown, "Show more" asks Roblox for
    /// the next page (a click, not scrolling, so passing a long section on
    /// the way to the next one doesn't keep loading it).
    fn experience_buttons(&self, ui: &mut egui::Ui, key: &str, src: ExperienceSource, n: usize, shown: usize, todo: &mut Vec<Do>) {
        let (more, loading) = self.start.lists.get(&src).map_or((false, false), |p| (p.more, p.loading));
        let open = self.start.expanded.contains(key);
        let less = open && (n > per_row(ui) * 2 || more);
        if shown >= n && !more && !less {
            return;
        }
        ui.horizontal(|ui| {
            if shown < n {
                if pill_button(ui, &format!("Show {} more", n - shown), true) {
                    todo.push(Do::ShowAll(key.to_string()));
                }
            } else if more {
                let label = if loading { "Loading…" } else { "Show more" };
                if pill_button(ui, label, !loading) {
                    todo.push(Do::More(src));
                }
            }
            if less && pill_button(ui, "Show less", true) {
                todo.push(Do::ShowAll(key.to_string()));
            }
        });
        ui.add_space(10.0);
    }

    fn experience_grid(&mut self, ui: &mut egui::Ui, key: &str, src: ExperienceSource, empty: &str, todo: &mut Vec<Do>) {
        let list: Listing = self.start.lists.get(&src).and_then(|p| p.list.clone());
        let exps = match &list {
            None => {
                self.start_signed_out_or_loading(ui, todo);
                return;
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(theme::WARN));
                if !self.account.signed_in() && ui.button("Sign in…").clicked() {
                    todo.push(Do::Account);
                }
                return;
            }
            Some(Ok(v)) => v,
        };
        if exps.is_empty() {
            let msg = if self.start.busy() { "Loading…" } else { empty };
            ui.label(RichText::new(msg).color(theme::TEXT_DIM));
            return;
        }
        let exps: Vec<&Experience> = exps.iter().filter(|e| self.matches(&e.name)).collect();
        let missing: Vec<u64> = exps.iter().map(|e| e.id).filter(|u| !self.start.icons_asked.contains(u)).collect();
        let shown = self.capped(ui, key, exps.len());
        let missing: Vec<u64> = missing.into_iter().filter(|u| exps[..shown].iter().any(|e| e.id == *u)).collect();
        if !missing.is_empty() {
            self.start_icons(missing);
        }
        grid(ui, shown, |ui, i| {
            let e = exps[i];
            let sub = if e.updated.is_empty() {
                creator(e)
            } else {
                format!("Updated {}", e.updated.get(..10).unwrap_or(&e.updated))
            };
            let icon = self.start.icons.get(&e.id).map(|t| t.id());
            let r = card(ui, &e.name, &sub, |p, rect| {
                icon_thumb(p, rect, icon, &e.name);
                if e.team_create == Some(true) {
                    let b = Rect::from_min_size(rect.min + Vec2::new(6.0, 6.0), Vec2::new(86.0, 18.0));
                    p.rect_filled(b, CornerRadius::same(3), Color32::from_black_alpha(180));
                    p.text(b.center(), egui::Align2::CENTER_CENTER, "Team Create", egui::FontId::proportional(11.0), Color32::WHITE);
                }
            });
            let r = r.on_hover_text(format!("{}\nby {} · universe {}", e.name, creator(e), e.id));
            if r.clicked() {
                if let Some(p) = e.root_place_id {
                    todo.push(Do::Cloud(p, false));
                }
            }
            r.context_menu(|ui| {
                if let Some(p) = e.root_place_id {
                    if ui.button("Open").clicked() {
                        todo.push(Do::Cloud(p, false));
                        ui.close();
                    }
                    if ui.button("Open a local copy").clicked() {
                        todo.push(Do::Cloud(p, true));
                        ui.close();
                    }
                    if ui.button("Copy start place ID").clicked() {
                        todo.push(Do::Copy(p.to_string()));
                        ui.close();
                    }
                }
                if ui.button("Copy universe ID").clicked() {
                    todo.push(Do::Copy(e.id.to_string()));
                    ui.close();
                }
            });
        });
        self.experience_buttons(ui, key, src, exps.len(), shown, todo);
    }
}

fn creator(e: &Experience) -> String {
    format!("{}{}", if e.creator_type == CreatorKind::User { "@" } else { "" }, e.creator_name)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn ago(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then);
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "yesterday".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}

/// Lays `n` cards out in rows that fill the width.
/// Cards per row at the current width.
fn pill_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    let b = egui::Button::new(RichText::new(label).color(theme::TEXT))
        .fill(theme::PANEL_DARK)
        .corner_radius(CornerRadius::same(4));
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = Vec2::new(14.0, 6.0);
        ui.add_enabled(enabled, b).clicked()
    })
    .inner
}

fn per_row(ui: &egui::Ui) -> usize {
    (((ui.available_width() + GAP) / (CARD_W + GAP)).floor() as usize).max(1)
}

fn grid(ui: &mut egui::Ui, n: usize, mut each: impl FnMut(&mut egui::Ui, usize)) {
    let per_row = per_row(ui);
    let mut i = 0;
    while i < n {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for _ in 0..per_row {
                if i >= n {
                    break;
                }
                each(ui, i);
                i += 1;
            }
        });
        ui.add_space(GAP);
    }
}

/// A card: thumbnail (drawn by `thumb`), title and one dim line.
fn card(ui: &mut egui::Ui, title: &str, sub: &str, thumb: impl FnOnce(&egui::Painter, Rect)) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(CARD_W, CARD_H), Sense::click());
    let p = ui.painter().with_clip_rect(r);
    let hov = resp.hovered();
    p.rect_filled(r, CornerRadius::same(6), if hov { theme::ACTIVE_TAB } else { theme::PANEL_DARK });
    let tr = Rect::from_min_size(r.min, Vec2::new(CARD_W, THUMB_H));
    thumb(&p.with_clip_rect(tr), tr);
    if hov {
        p.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.5, theme::SELECT), egui::StrokeKind::Inside);
    }
    let font = egui::FontId::proportional(13.5);
    let title = elide(&p, title, &font, CARD_W - 20.0);
    p.text(Pos2::new(r.min.x + 10.0, tr.max.y + 16.0), egui::Align2::LEFT_CENTER, title, font, theme::TEXT);
    let small = egui::FontId::proportional(11.5);
    let sub = elide(&p, sub, &small, CARD_W - 20.0);
    p.text(Pos2::new(r.min.x + 10.0, tr.max.y + 35.0), egui::Align2::LEFT_CENTER, sub, small, theme::TEXT_DIM);
    if hov {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

fn elide(p: &egui::Painter, s: &str, font: &egui::FontId, max: f32) -> String {
    let width = |t: &str| p.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE).size().x;
    if width(s) <= max {
        return s.to_string();
    }
    let mut out: String = s.to_string();
    while !out.is_empty() && width(&format!("{out}…")) > max {
        out.pop();
    }
    format!("{out}…")
}

/// The experience's icon, centred on a blurred-looking fill; a letter tile
/// until it loads.
fn icon_thumb(p: &egui::Painter, r: Rect, icon: Option<egui::TextureId>, name: &str) {
    p.rect_filled(r, CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, theme::PANEL_ALT);
    let side = r.height() - 16.0;
    let ir = Rect::from_center_size(r.center(), Vec2::splat(side));
    match icon {
        Some(t) => {
            p.image(t, ir, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
            let c = Color32::from_rgb(60 + (h % 90) as u8, 70 + ((h >> 8) % 80) as u8, 110 + ((h >> 16) % 100) as u8);
            p.rect_filled(ir, CornerRadius::same(6), c);
            let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            p.text(ir.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(36.0), Color32::WHITE);
        }
    }
}

/// A small painting of the template: sky, ground and a spawn pad.
fn template_thumb(p: &egui::Painter, r: Rect, t: Template) {
    let top = CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 };
    let horizon = r.min.y + r.height() * 0.48;
    let sky = Rect::from_min_max(r.min, Pos2::new(r.max.x, horizon));
    let mut mesh = egui::Mesh::default();
    let (a, b) = (Color32::from_rgb(92, 150, 214), Color32::from_rgb(178, 210, 240));
    mesh.colored_vertex(sky.left_top(), a);
    mesh.colored_vertex(sky.right_top(), a);
    mesh.colored_vertex(sky.right_bottom(), b);
    mesh.colored_vertex(sky.left_bottom(), b);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.rect_filled(sky, top, a);
    p.add(mesh);
    let (ground, line) = match t {
        Template::Baseplate => (Color32::from_rgb(99, 95, 98), Color32::from_rgb(86, 83, 86)),
        Template::FlatTerrain => (Color32::from_rgb(88, 132, 62), Color32::from_rgb(76, 116, 52)),
    };
    // The plane in perspective: a trapezoid with grid lines.
    let gl = Rect::from_min_max(Pos2::new(r.min.x, horizon), r.max);
    p.rect_filled(gl, CornerRadius::ZERO, ground);
    let vp = Pos2::new(r.center().x, horizon - r.height() * 0.6);
    if t == Template::Baseplate {
        for k in 0..=12 {
            let x = r.min.x - r.width() * 0.5 + r.width() * 2.0 * k as f32 / 12.0;
            let bottom = Pos2::new(x, r.max.y);
            let tt = (horizon - vp.y) / (bottom.y - vp.y);
            let at = Pos2::new(vp.x + (bottom.x - vp.x) * tt, horizon);
            p.line_segment([at, bottom], Stroke::new(1.0, line));
        }
        for k in 1..6 {
            let y = horizon + (r.max.y - horizon) * (k as f32 / 6.0).powf(1.6);
            p.line_segment([Pos2::new(r.min.x, y), Pos2::new(r.max.x, y)], Stroke::new(1.0, line));
        }
    } else {
        for k in 0..40 {
            let h = (k * 2654435761u32 as usize) as u32;
            let x = r.min.x + (h % 1000) as f32 / 1000.0 * r.width();
            let y = horizon + 4.0 + ((h >> 10) % 1000) as f32 / 1000.0 * (r.max.y - horizon - 6.0);
            p.line_segment([Pos2::new(x, y), Pos2::new(x + 1.0, y - 3.0)], Stroke::new(1.0, line));
        }
    }
    // The SpawnLocation.
    let c = Pos2::new(r.center().x, horizon + (r.max.y - horizon) * 0.45);
    let pad = [
        Pos2::new(c.x - 22.0, c.y),
        Pos2::new(c.x, c.y - 7.0),
        Pos2::new(c.x + 22.0, c.y),
        Pos2::new(c.x, c.y + 7.0),
    ];
    p.add(egui::Shape::convex_polygon(pad.to_vec(), Color32::from_rgb(163, 162, 165), Stroke::new(1.0, Color32::from_rgb(120, 120, 124))));
    let side = [pad[0], pad[3], pad[3] + Vec2::new(0.0, 3.0), pad[0] + Vec2::new(0.0, 3.0)];
    p.add(egui::Shape::convex_polygon(side.to_vec(), Color32::from_rgb(130, 129, 132), Stroke::NONE));
    let side2 = [pad[3], pad[2], pad[2] + Vec2::new(0.0, 3.0), pad[3] + Vec2::new(0.0, 3.0)];
    p.add(egui::Shape::convex_polygon(side2.to_vec(), Color32::from_rgb(112, 111, 114), Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_build_clean_places() {
        let bp = Template::Baseplate.build();
        assert!(!bp.dirty());
        let ws = bp.workspace().unwrap();
        assert!(bp.dom.get_by_ref(ws).unwrap().children().iter().any(|r| bp.dom.get_by_ref(*r).unwrap().name == "Baseplate"));

        let ft = Template::FlatTerrain.build();
        assert!(!ft.dirty());
        let ws = ft.workspace().unwrap();
        assert!(!ft.dom.get_by_ref(ws).unwrap().children().iter().any(|r| ft.dom.get_by_ref(*r).unwrap().name == "Baseplate"));
        let v = crate::terrain::Voxels::from_dom(&ft.dom).unwrap().unwrap();
        let grass = crate::terrain::material_index("Grass").unwrap();
        assert_eq!(v.get(0, -1, 0).0, grass);
        assert_eq!(v.get(100, -2, -100).0, grass);
        assert_eq!(v.get(0, 1, 0).0, 0);
    }
}
