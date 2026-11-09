//! Explorer: the instance tree, as Studio's (checks/ui/studio/explorer-*.png):
//! siblings in Studio's order (ReflectionMetadata ExplorerOrder, then name),
//! selection with Ctrl / Shift-click and the arrow keys, rename (F2), drag
//! and drop with an above / into / below indicator (several objects at once,
//! auto-scroll at the edges, auto-expand on hover), a "+" on the hovered row
//! that opens Insert Object for that object, and Studio's context menu.

use std::collections::HashSet;
use std::time::Instant;

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use rbx_dom_weak::types::Ref;

use crate::app::Editor;
use crate::doc::{DocKind, Document};
use crate::icons;
use crate::theme;

const ROW_H: f32 = 20.0;
const INDENT: f32 = 14.0;
/// Hovering a collapsed row this long while dragging expands it.
const AUTO_EXPAND_S: f32 = 0.6;
/// Dragging within this many points of the list's top or bottom scrolls it.
const AUTO_SCROLL_EDGE: f32 = 24.0;

#[derive(Default)]
pub struct ExplorerState {
    pub expanded: HashSet<Ref>,
    pub renaming: Option<(Ref, String)>,
    pub filter: String,
    pub scroll_to: Option<Ref>,
    anchor: Option<Ref>,
    /// Scroll offset and visible height from the last frame.
    offset: f32,
    view_h: f32,
    /// Rows fully inside the visible area last frame (the self-test reads it).
    pub visible: Vec<Ref>,
    /// Row rects on screen last frame and their "+" buttons (the self-test).
    pub row_rects: Vec<(Ref, Rect)>,
    pub plus_rect: Option<(Ref, Rect)>,
    /// The Explorer has the keyboard (last click was in it): arrows move the selection.
    pub focused: bool,
    /// Drag hover: the row and when the pointer arrived (auto-expand).
    drag_hover: Option<(Ref, Instant)>,
    /// Flattened rows and what they were built from (doc id, tree revision,
    /// expanded-set fingerprint, filter).
    rows_cache: Option<((u64, u64, u64, String), std::rc::Rc<Vec<(Ref, usize)>>)>,
    /// While searching: ancestors of a match, drawn open (caret down).
    forced_open: HashSet<Ref>,
    /// The search text last frame (clearing it reveals the selection).
    last_filter: String,
}

/// Order-independent fingerprint of the expanded set.
fn expanded_key(st: &ExplorerState) -> u64 {
    use std::hash::{Hash, Hasher};
    st.expanded.iter().fold(st.expanded.len() as u64, |acc, r| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        r.hash(&mut h);
        acc ^ h.finish()
    })
}

/// Drag payload: the instances being dragged.
#[derive(Clone)]
struct DragRefs(Vec<Ref>);

/// Where a drop lands relative to the row under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drop {
    Above,
    Into,
    Below,
}

impl ExplorerState {
    /// Studio opens a place with every service collapsed (Workspace too);
    /// a model file shows its contents.
    pub fn expand_defaults(&mut self, doc: &Document) {
        if doc.kind == DocKind::Model {
            self.expanded.insert(doc.dom.root_ref());
        }
    }

    /// Expands every collapsed ancestor of `r`.
    pub fn expand_ancestors(&mut self, doc: &Document, r: Ref) {
        let mut cur = doc.dom.get_by_ref(r).map(|i| i.parent());
        while let Some(p) = cur {
            if p.is_none() {
                break;
            }
            self.expanded.insert(p);
            cur = doc.dom.get_by_ref(p).map(|i| i.parent());
        }
    }
}

/// Studio's sibling order: ReflectionMetadata ExplorerOrder (an int on
/// ReflectionMetadataClass, category "Reflection", registered by
/// ), then the name A–Z (measured in an offline Studio).
fn sorted_children(doc: &Document, r: Ref) -> Vec<Ref> {
    let Some(inst) = doc.dom.get_by_ref(r) else {
        return Vec::new();
    };
    let mut kids: Vec<(i32, String, Ref)> = inst
        .children()
        .iter()
        .filter_map(|c| doc.dom.get_by_ref(*c))
        .map(|i| (crate::studio_panels::explorer_order(i.class.as_str()), i.name.to_lowercase(), i.referent()))
        .collect();
    kids.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    kids.into_iter().map(|(_, _, r)| r).collect()
}

/// Rows under `r` as the user expanded them.
fn flatten_open(doc: &Document, st: &ExplorerState, r: Ref, depth: usize, rows: &mut Vec<(Ref, usize)>) {
    let mut stack: Vec<(Ref, usize)> = sorted_children(doc, r).into_iter().rev().map(|r| (r, depth)).collect();
    while let Some((r, depth)) = stack.pop() {
        rows.push((r, depth));
        if st.expanded.contains(&r) {
            for c in sorted_children(doc, r).into_iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
}

/// The rows to draw, and (while searching) the ancestors drawn open.
/// Searching keeps the tree: every match shows under its whole parent
/// chain, like Studio's filtered Explorer.
fn flatten(doc: &Document, st: &ExplorerState) -> (Vec<(Ref, usize)>, HashSet<Ref>) {
    let mut rows = Vec::new();
    let root = doc.dom.root_ref();
    let filter = st.filter.trim().to_lowercase();
    if filter.is_empty() {
        flatten_open(doc, st, root, 0, &mut rows);
        return (rows, HashSet::new());
    }
    let mut show: HashSet<Ref> = HashSet::new();
    let mut ancestors: HashSet<Ref> = HashSet::new();
    for inst in doc.dom.descendants() {
        if inst.referent() == root
            || !(inst.name.to_lowercase().contains(&filter) || inst.class.to_lowercase().contains(&filter))
        {
            continue;
        }
        show.insert(inst.referent());
        let mut p = inst.parent();
        while p.is_some() && p != root && ancestors.insert(p) {
            show.insert(p);
            p = doc.dom.get_by_ref(p).map(|i| i.parent()).unwrap_or_else(Ref::none);
        }
    }
    fn walk(
        doc: &Document,
        st: &ExplorerState,
        show: &HashSet<Ref>,
        ancestors: &HashSet<Ref>,
        r: Ref,
        depth: usize,
        rows: &mut Vec<(Ref, usize)>,
    ) {
        for c in sorted_children(doc, r) {
            if !show.contains(&c) {
                continue;
            }
            rows.push((c, depth));
            if ancestors.contains(&c) {
                walk(doc, st, show, ancestors, c, depth + 1, rows);
            } else if st.expanded.contains(&c) {
                // A match the user opened shows all it holds.
                flatten_open(doc, st, c, depth + 1, &mut *rows);
            }
        }
    }
    walk(doc, st, &show, &ancestors, root, 0, &mut rows);
    (rows, ancestors)
}

impl Editor {
    pub fn explorer_ui(&mut self, ui: &mut egui::Ui) {
        // The keyboard belongs to the Explorer after a click in it.
        let panel = ui.max_rect();
        if let Some(p) = ui.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None }) {
            self.explorer.focused = panel.contains(p);
        }
        // During Play: which DataModel to inspect.
        if let Some(showing) = self
            .play
            .as_ref()
            .filter(|p| p.mode == crate::runtime::RunMode::Play)
            .map(|p| p.showing)
        {
            use crate::runtime::Side;
            let mut pick = showing;
            ui.horizontal(|ui| {
                ui.label(RichText::new("View").small().color(theme::TEXT_DIM));
                let r = ui
                    .selectable_value(&mut pick, Side::Server, "Server")
                    .on_hover_text("The server's DataModel: Scripts, ServerStorage, physics");
                self.ui_rects.insert("view.server", r.rect);
                let r = ui
                    .selectable_value(&mut pick, Side::Client, "Client")
                    .on_hover_text("The client's DataModel: what replicated, plus LocalScript changes");
                self.ui_rects.insert("view.client", r.rect);
            });
            if pick != showing {
                self.show_play_side(pick);
            }
        }
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.explorer.filter)
                    .hint_text("Search")
                    .desired_width(f32::INFINITY),
            );
            self.ui_rects.insert("explorer.search", r.rect);
            // An "x" inside the box's right end clears the search.
            if !self.explorer.filter.is_empty() {
                let x_rect = Rect::from_center_size(
                    Pos2::new(r.rect.max.x - 10.0, r.rect.center().y),
                    Vec2::splat(14.0),
                );
                let x = ui.interact(x_rect, egui::Id::new("explorer_search_clear"), Sense::click());
                let x = x.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Clear search");
                let col = if x.hovered() { Color32::WHITE } else { theme::TEXT_DIM };
                crate::ti::paint(ui.painter(), x_rect, "x", col);
                self.ui_rects.insert("explorer.search_clear", x_rect);
                if x.clicked() {
                    self.explorer.filter.clear();
                }
            }
        });
        // Clearing the search keeps what was picked in it in view.
        if self.explorer.filter.trim().is_empty() && !self.explorer.last_filter.trim().is_empty() {
            let sel = self.doc.selection.clone();
            for &s in &sel {
                self.explorer.expand_ancestors(&self.doc, s);
            }
            if let Some(&last) = sel.last() {
                self.explorer.scroll_to = Some(last);
            }
        }
        if self.explorer.last_filter != self.explorer.filter {
            self.explorer.last_filter = self.explorer.filter.clone();
        }
        ui.add_space(2.0);
        let key = (
            self.doc.id,
            self.doc.tree_revision,
            expanded_key(&self.explorer),
            self.explorer.filter.clone(),
        );
        let rows = match &self.explorer.rows_cache {
            Some((k, rows)) if *k == key => rows.clone(),
            _ => {
                let (rows, forced) = flatten(&self.doc, &self.explorer);
                let rows = std::rc::Rc::new(rows);
                self.explorer.forced_open = forced;
                self.explorer.rows_cache = Some((key, rows.clone()));
                rows
            }
        };
        if self.explorer.focused && self.explorer.renaming.is_none() && !ui.ctx().egui_wants_keyboard_input() {
            self.explorer_keys(ui, &rows);
        }
        // show_rows spaces rows by row height + item spacing: zero the
        // spacing so row i sits exactly at i * ROW_H.
        ui.spacing_mut().item_spacing.y = 0.0;
        let dragging = egui::DragAndDrop::has_payload_of_type::<DragRefs>(ui.ctx());
        let mut scroll = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("explorer_scroll");
        if let Some(target) = self.explorer.scroll_to.take() {
            if let Some(idx) = rows.iter().position(|(r, _)| *r == target) {
                let view_h = if self.explorer.view_h > 0.0 { self.explorer.view_h } else { ui.available_height() };
                let cur = self.explorer.offset;
                let y = idx as f32 * ROW_H;
                if y < cur || y + ROW_H > cur + view_h {
                    scroll = scroll.vertical_scroll_offset((y - view_h * 0.4).max(0.0));
                }
            }
        }
        let mut actions: Vec<RowAction> = Vec::new();
        let mut drawn: Vec<(Ref, Rect)> = Vec::new();
        self.explorer.plus_rect = None;
        let out = scroll.show_rows(ui, ROW_H, rows.len(), |ui, range| {
            // Dragging near the top or bottom edge scrolls the list.
            if dragging {
                let clip = ui.clip_rect();
                if let Some(p) = ui.ctx().pointer_hover_pos() {
                    if clip.x_range().contains(p.x) {
                        if p.y < clip.min.y + AUTO_SCROLL_EDGE && p.y > clip.min.y - AUTO_SCROLL_EDGE {
                            ui.scroll_with_delta(Vec2::new(0.0, 6.0));
                        } else if p.y > clip.max.y - AUTO_SCROLL_EDGE && p.y < clip.max.y + AUTO_SCROLL_EDGE {
                            ui.scroll_with_delta(Vec2::new(0.0, -6.0));
                        }
                    }
                }
            }
            for i in range {
                let (r, depth) = rows[i];
                let top = ui.cursor().min.y;
                self.explorer_row(ui, r, depth, &mut actions);
                drawn.push((r, Rect::from_min_max(egui::pos2(ui.min_rect().min.x, top), egui::pos2(ui.max_rect().max.x, top + ROW_H))));
                self.hud.explorer_rows += 1;
            }
            // Space below the rows accepts drops onto the default parent.
            let rest = ui.available_rect_before_wrap();
            if rest.height() > 4.0 {
                let resp = ui.allocate_rect(rest, Sense::click());
                if let Some(p) = resp.dnd_release_payload::<DragRefs>() {
                    actions.push(RowAction::Reparent(p.0.clone(), self.doc.default_parent()));
                }
                if resp.clicked() {
                    actions.push(RowAction::ClearSelection);
                }
            }
        });
        if dragging {
            ui.ctx().request_repaint();
        } else {
            self.explorer.drag_hover = None;
        }
        self.explorer.offset = out.state.offset.y;
        self.explorer.view_h = out.inner_rect.height();
        let (vis_top, vis_bottom) = (out.inner_rect.min.y, out.inner_rect.max.y);
        self.explorer.row_rects = drawn.clone();
        self.explorer.visible = drawn
            .into_iter()
            .filter(|(_, r)| r.min.y >= vis_top - 0.5 && r.max.y <= vis_bottom + 0.5)
            .map(|(r, _)| r)
            .collect();
        for a in actions {
            self.apply_row_action(a, &rows);
        }
    }

    /// Arrow keys: Up / Down move the selection (Shift extends it), Left
    /// collapses or goes to the parent, Right expands or goes to the first child.
    fn explorer_keys(&mut self, ui: &egui::Ui, rows: &[(Ref, usize)]) {
        use egui::Key;
        let (up, down, left, right, shift) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
                i.modifiers.shift,
            )
        });
        if !(up || down || left || right) {
            return;
        }
        let cur = self.doc.selection.last().copied();
        let idx = cur.and_then(|c| rows.iter().position(|(r, _)| *r == c));
        if up || down {
            let next = match idx {
                Some(i) if up => i.saturating_sub(1),
                Some(i) => (i + 1).min(rows.len().saturating_sub(1)),
                None => 0,
            };
            if let Some((r, _)) = rows.get(next) {
                let r = *r;
                if shift {
                    let anchor = self.explorer.anchor.unwrap_or(r);
                    if let (Some(a), Some(b)) = (rows.iter().position(|(x, _)| *x == anchor), Some(next)) {
                        let (lo, hi) = (a.min(b), a.max(b));
                        let mut sel: Vec<Ref> = rows[lo..=hi].iter().map(|(x, _)| *x).collect();
                        // (the moving end last: it is the one the next arrow moves)
                        sel.retain(|x| *x != r);
                        sel.push(r);
                        self.doc.selection = sel;
                    }
                } else {
                    self.doc.selection = vec![r];
                    self.explorer.anchor = Some(r);
                }
                self.explorer.scroll_to = Some(r);
            }
            return;
        }
        let Some(c) = cur else {
            return;
        };
        let has_kids = self.doc.dom.get_by_ref(c).is_some_and(|i| !i.children().is_empty());
        if left {
            if self.explorer.expanded.contains(&c) {
                self.explorer.expanded.remove(&c);
            } else if let Some(p) = self.doc.dom.get_by_ref(c).map(|i| i.parent()) {
                if p != self.doc.dom.root_ref() && !p.is_none() {
                    self.doc.selection = vec![p];
                    self.explorer.anchor = Some(p);
                    self.explorer.scroll_to = Some(p);
                }
            }
        } else if right && has_kids {
            if !self.explorer.expanded.contains(&c) {
                self.explorer.expanded.insert(c);
            } else if let Some(first) = sorted_children(&self.doc, c).first() {
                self.doc.selection = vec![*first];
                self.explorer.anchor = Some(*first);
                self.explorer.scroll_to = Some(*first);
            }
        }
    }

    fn explorer_row(&mut self, ui: &mut egui::Ui, r: Ref, depth: usize, actions: &mut Vec<RowAction>) {
        let Some(inst) = self.doc.dom.get_by_ref(r) else {
            return;
        };
        let name = inst.name.clone();
        let class = inst.class.to_string();
        let parent = inst.parent();
        let has_children = !inst.children().is_empty();
        let selected = self.doc.selection.contains(&r);
        let expanded = self.explorer.expanded.contains(&r) || self.explorer.forced_open.contains(&r);

        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        // Drag over this row: above / into / below by the pointer's height.
        let drop = resp.dnd_hover_payload::<DragRefs>().and_then(|_| {
            let y = ui.ctx().pointer_hover_pos()?.y;
            let t = (y - rect.min.y) / rect.height();
            Some(if t < 0.25 {
                Drop::Above
            } else if t > 0.75 && !expanded {
                Drop::Below
            } else {
                Drop::Into
            })
        });
        if selected {
            painter.rect_filled(rect, 0.0, theme::SELECT_BG);
        } else if resp.hovered() || drop.is_some() {
            painter.rect_filled(rect, 0.0, theme::HOVER_BG);
        }
        let x0 = rect.min.x + 4.0 + depth as f32 * INDENT;
        match drop {
            Some(Drop::Into) => {
                painter.rect_stroke(rect.shrink(1.0), 2.0, Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Inside);
            }
            Some(d) => {
                let y = if d == Drop::Above { rect.min.y + 1.0 } else { rect.max.y - 1.0 };
                painter.line_segment([Pos2::new(x0, y), Pos2::new(rect.max.x - 4.0, y)], Stroke::new(2.0, theme::ACCENT));
                painter.circle_filled(Pos2::new(x0, y), 3.0, theme::ACCENT);
            }
            None => {}
        }
        // Auto-expand a collapsed row the drag rests on.
        if drop.is_some() && has_children && !expanded {
            match self.explorer.drag_hover {
                Some((h, since)) if h == r => {
                    if since.elapsed().as_secs_f32() > AUTO_EXPAND_S {
                        self.explorer.expanded.insert(r);
                        self.explorer.drag_hover = None;
                    }
                }
                _ => self.explorer.drag_hover = Some((r, Instant::now())),
            }
        }
        let tri_rect = Rect::from_center_size(Pos2::new(x0 + 6.0, rect.center().y), Vec2::splat(12.0));
        if has_children {
            let glyph = if expanded { "caret-down-filled" } else { "caret-right-filled" };
            crate::ti::paint(&painter, tri_rect, glyph, theme::TEXT_DIM);
        }
        let icon_rect = Rect::from_center_size(Pos2::new(x0 + 22.0, rect.center().y), Vec2::splat(16.0));
        icons::paint_class_icon(&painter, icon_rect, &class);
        let text_pos = Pos2::new(x0 + 34.0, rect.center().y);
        // The "+" just right of the name while the row is hovered (Studio's quick insert).
        let label = if name.is_empty() { format!("({class})") } else { name.clone() };
        let name_w = ui.fonts_mut(|f| f.layout_no_wrap(label, egui::FontId::proportional(13.0), theme::TEXT).size().x);
        let plus_x = (text_pos.x + name_w + 6.0 + 8.0).min(rect.max.x - 10.0);
        let plus_rect = Rect::from_center_size(Pos2::new(plus_x, rect.center().y), Vec2::splat(16.0));
        let row_hot = ui.rect_contains_pointer(rect) && !egui::DragAndDrop::has_any_payload(ui.ctx());
        let plus = row_hot.then(|| ui.interact(plus_rect, egui::Id::new(("explorer_plus", r)), Sense::click()));
        if let Some(p) = &plus {
            if p.hovered() {
                painter.rect_filled(plus_rect.expand(1.0), 3.0, theme::ACTIVE_TAB);
            }
            crate::ti::paint(&painter, plus_rect, "plus", if p.hovered() { Color32::WHITE } else { theme::TEXT });
            self.explorer.plus_rect = Some((r, plus_rect));
        }

        let renaming = self.explorer.renaming.as_ref().is_some_and(|(rr, _)| *rr == r);
        if renaming {
            let edit_rect = Rect::from_min_max(Pos2::new(text_pos.x, rect.min.y + 1.0), Pos2::new(rect.max.x - 4.0, rect.max.y - 1.0));
            let mut text = self.explorer.renaming.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
            let edit = ui.put(edit_rect, egui::TextEdit::singleline(&mut text).margin(Vec2::new(2.0, 0.0)));
            edit.request_focus();
            let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if esc {
                self.explorer.renaming = None;
            } else if enter || edit.lost_focus() {
                actions.push(RowAction::Rename(r, text));
            } else {
                self.explorer.renaming = Some((r, text));
            }
        } else {
            let color = if selected { Color32::WHITE } else { theme::TEXT };
            let (label, col) = if name.is_empty() { (format!("({class})"), theme::TEXT_DIM) } else { (name.clone(), color) };
            painter.text(text_pos, egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), col);
        }

        // Interaction.
        if let Some(p) = plus.filter(|p| p.clicked()) {
            actions.push(RowAction::InsertDialog(r, p.rect.right_bottom()));
            return;
        }
        if resp.clicked() {
            let on_tri = has_children && resp.interact_pointer_pos().is_some_and(|p| tri_rect.expand(3.0).contains(p));
            if on_tri {
                actions.push(RowAction::Toggle(r));
            } else {
                let m = ui.input(|i| i.modifiers);
                actions.push(RowAction::Click(r, m.command, m.shift));
            }
        }
        if resp.double_clicked() {
            if matches!(class.as_str(), "Script" | "LocalScript" | "ModuleScript") {
                actions.push(RowAction::OpenScript(r));
            } else {
                actions.push(RowAction::StartRename(r));
            }
        }
        if resp.drag_started() {
            // Several selected objects drag together (Studio).
            let refs = if selected { self.doc.selection.clone() } else { vec![r] };
            resp.dnd_set_drag_payload(DragRefs(refs));
        }
        if let Some(p) = resp.dnd_release_payload::<DragRefs>() {
            let target = match drop.unwrap_or(Drop::Into) {
                Drop::Into => r,
                // Above / below: beside this row, in its parent (Studio then
                // orders siblings by ExplorerOrder and name).
                Drop::Above | Drop::Below => parent,
            };
            actions.push(RowAction::Reparent(p.0.clone(), target));
        }
        let script = matches!(class.as_str(), "Script" | "LocalScript" | "ModuleScript");
        let can_paste = self.clipboard.is_some();
        resp.context_menu(|ui| {
            ui.set_min_width(200.0);
            if !selected {
                actions.push(RowAction::Click(r, false, false));
            }
            let item = |ui: &mut egui::Ui, label: &str, enabled: bool| ui.add_enabled(enabled, egui::Button::new(label).frame(false)).clicked();
            if item(ui, "Cut", true) {
                actions.push(RowAction::Action("cut"));
                ui.close();
            }
            if item(ui, "Copy", true) {
                actions.push(RowAction::Action("copy"));
                ui.close();
            }
            if can_paste && item(ui, "Paste Into", true) {
                actions.push(RowAction::Action("paste_into"));
                ui.close();
            }
            ui.add_enabled_ui(false, |ui| ui.menu_button("Copy As", |_| {}));
            ui.separator();
            if item(ui, "Duplicate", true) {
                actions.push(RowAction::Action("duplicate"));
                ui.close();
            }
            if item(ui, "Delete", true) {
                actions.push(RowAction::Action("delete"));
                ui.close();
            }
            if item(ui, "Rename", true) {
                actions.push(RowAction::StartRename(r));
                ui.close();
            }
            ui.separator();
            if item(ui, "Group as Model", true) {
                actions.push(RowAction::Action("group"));
                ui.close();
            }
            if item(ui, "Group as Folder", true) {
                actions.push(RowAction::GroupFolder);
                ui.close();
            }
            if matches!(class.as_str(), "Model" | "Folder") && item(ui, "Ungroup", true) {
                actions.push(RowAction::Action("ungroup"));
                ui.close();
            }
            ui.add_enabled_ui(false, |ui| ui.menu_button("Solid Modeling", |_| {}));
            ui.menu_button("Insert", |ui| {
                if item(ui, "Insert Part", true) {
                    actions.push(RowAction::Insert("Part", r));
                    ui.close();
                }
                if item(ui, "Insert Object…", true) {
                    let at = ui.ctx().pointer_hover_pos().unwrap_or_default();
                    actions.push(RowAction::InsertDialog(r, at));
                    ui.close();
                }
                if item(ui, "Import Roblox Model", true) {
                    actions.push(RowAction::ImportModel(r));
                    ui.close();
                }
            });
            ui.menu_button("Hierarchy", |ui| {
                if item(ui, "Select Children", true) {
                    actions.push(RowAction::SelectChildren(r));
                    ui.close();
                }
            });
            ui.separator();
            if item(ui, "Zoom to", true) {
                actions.push(RowAction::Action("focus"));
                ui.close();
            }
            item(ui, "Show Orientation Indicator", false);
            ui.separator();
            item(ui, "Convert to Package", false);
            ui.menu_button("Save / Export", |ui| {
                if item(ui, "Save to File…", true) {
                    actions.push(RowAction::SaveSelection);
                    ui.close();
                }
            });
            if script {
                ui.separator();
                if item(ui, "Edit Source", true) {
                    actions.push(RowAction::OpenScript(r));
                    ui.close();
                }
            }
            ui.separator();
            if item(ui, "Help", true) {
                ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                    "https://create.roblox.com/docs/reference/engine/classes/{class}"
                )));
                ui.close();
            }
        });
        resp.on_hover_text_at_pointer(RichText::new(class).small());
    }

    fn apply_row_action(&mut self, a: RowAction, rows: &[(Ref, usize)]) {
        match a {
            RowAction::Toggle(r) => {
                if !self.explorer.expanded.remove(&r) {
                    self.explorer.expanded.insert(r);
                }
            }
            RowAction::Click(r, toggle, range) => {
                if range {
                    let anchor = self.explorer.anchor.unwrap_or(r);
                    let a = rows.iter().position(|(x, _)| *x == anchor);
                    let b = rows.iter().position(|(x, _)| *x == r);
                    if let (Some(a), Some(b)) = (a, b) {
                        let (lo, hi) = (a.min(b), a.max(b));
                        self.doc.selection = rows[lo..=hi].iter().map(|(x, _)| *x).collect();
                        return;
                    }
                }
                if toggle {
                    if let Some(pos) = self.doc.selection.iter().position(|s| *s == r) {
                        self.doc.selection.remove(pos);
                    } else {
                        self.doc.selection.push(r);
                    }
                } else {
                    self.doc.selection = vec![r];
                }
                self.explorer.anchor = Some(r);
            }
            RowAction::ClearSelection => self.doc.selection.clear(),
            RowAction::StartRename(r) => {
                let name = self.doc.dom.get_by_ref(r).map(|i| i.name.clone()).unwrap_or_default();
                self.explorer.renaming = Some((r, name));
            }
            RowAction::Rename(r, name) => {
                self.explorer.renaming = None;
                let name = name.trim();
                if !name.is_empty() {
                    self.doc.rename(r, name);
                }
            }
            RowAction::Reparent(refs, parent) => {
                if refs.iter().all(|r| self.doc.dom.get_by_ref(*r).is_some_and(|i| i.parent() == parent)) {
                    return; // dropped beside itself: nothing moves
                }
                let n = self.doc.reparent(&refs, parent);
                if n > 0 {
                    self.explorer.expanded.insert(parent);
                    let pname = self.doc.dom.get_by_ref(parent).map(|i| i.name.clone()).unwrap_or_default();
                    self.info(format!("Moved {n} object(s) into {pname}"));
                } else if !refs.contains(&parent) {
                    self.warn("Cannot move there (services, the root, or into its own descendant)");
                }
            }
            RowAction::Insert(class, parent) => {
                self.explorer.expanded.insert(parent);
                self.insert_class(class, parent);
            }
            RowAction::InsertDialog(parent, at) => {
                self.explorer.expanded.insert(parent);
                self.open_insert_popup(parent, at);
            }
            RowAction::ImportModel(parent) => {
                self.doc.selection = vec![parent];
                self.insert_from_file();
            }
            RowAction::SaveSelection => self.save_selection(),
            RowAction::GroupFolder => {
                let sel = self.doc.selection.clone();
                if let Some(f) = self.doc.group_as(&sel, "Folder") {
                    self.select(vec![f]);
                }
            }
            RowAction::Action(act) => self.run_action(act),
            RowAction::SelectChildren(r) => {
                let kids = sorted_children(&self.doc, r);
                self.explorer.expanded.insert(r);
                self.doc.selection = kids;
            }
            RowAction::OpenScript(r) => {
                self.open_script.push(r);
            }
        }
    }
}

enum RowAction {
    Toggle(Ref),
    Click(Ref, bool, bool),
    ClearSelection,
    StartRename(Ref),
    Rename(Ref, String),
    Reparent(Vec<Ref>, Ref),
    Insert(&'static str, Ref),
    InsertDialog(Ref, Pos2),
    ImportModel(Ref),
    GroupFolder,
    SaveSelection,
    Action(&'static str),
    SelectChildren(Ref),
    OpenScript(Ref),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::InstanceBuilder;

    #[test]
    fn search_keeps_the_parent_chain() {
        let mut doc = Document::new_place();
        let ws = doc.default_parent();
        let a = doc.insert(ws, InstanceBuilder::new("Folder").with_name("Outer"));
        let b = doc.insert(a, InstanceBuilder::new("Folder").with_name("Inner"));
        let hit = doc.insert(b, InstanceBuilder::new("TextButton").with_name("indexButton"));
        doc.insert(b, InstanceBuilder::new("Folder").with_name("Other"));
        let mut st = ExplorerState::default();
        st.filter = "indexbut".into();
        let (rows, open) = flatten(&doc, &st);
        let refs: Vec<Ref> = rows.iter().map(|r| r.0).collect();
        assert_eq!(refs, vec![ws, a, b, hit]);
        assert_eq!(rows.iter().map(|r| r.1).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert!(open.contains(&ws) && open.contains(&b) && !open.contains(&hit));
        // Cleared: the picked row stays reachable.
        st.filter.clear();
        st.expand_ancestors(&doc, hit);
        assert!(flatten(&doc, &st).0.iter().any(|r| r.0 == hit));
    }
}
