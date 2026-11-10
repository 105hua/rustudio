//! The Properties panel, drawn like Studio's (checks/ui/studio/props-*.png):
//! a filter field, then category bands (25 px, (32, 34, 39), bold name and
//! a ▼ / ▶ to fold) over two-column rows (21 px with a 1-px (32, 34, 39)
//! separator; names indented 25 px; the divider at 48 % of the width).
//! Text (188, 190, 200), read-only rows (73, 77, 90). Values as Studio shows
//! them: check boxes, colour swatches with "[r, g, b]" (and the BrickColor
//! name when the colour is one), enum and BrickColor drop-downs, vectors as
//! "x, y, z" that unfold into X / Y / Z, CFrames as Position / Orientation.
//! Which rows, in which category and order: src/props_layout.rs.

use std::collections::{HashMap, HashSet};

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use glam::Vec3;
use rbx_dom_weak::types::{BrickColor, Color3, Color3uint8, CustomPhysicalProperties, Enum, PhysicalProperties, Ref, Tags, Variant, Vector2, Vector3};
use rbx_dom_weak::ustr;

use crate::app::Editor;
use crate::doc;
use crate::math::Xform;
use crate::props_layout::{Category, Frame, Row};

const ROW_H: f32 = 21.0;
const CAT_H: f32 = 25.0;
const INDENT: f32 = 25.0;
const DIVIDER: f32 = 0.48;
pub const TEXT: Color32 = Color32::from_rgb(188, 190, 200);
pub const READ_ONLY: Color32 = Color32::from_rgb(73, 77, 90);
pub const BAND: Color32 = Color32::from_rgb(32, 34, 39);
const BOX_OFF: Color32 = Color32::from_rgb(40, 41, 49);
const FONT: f32 = 12.5;

#[derive(Default)]
pub struct PropertiesState {
    pub filter: String,
    /// Folded categories (Surface starts folded, as in Studio).
    folded: HashSet<String>,
    unfolded: HashSet<String>,
    /// Unfolded vector rows ("Size", "CFrame.Position"…).
    open_rows: HashSet<String>,
    /// The cell being typed into: (row key, text).
    editing: Option<(String, String)>,
    /// The word a double-click selected, held until the button is released
    /// (egui would otherwise select on release and flash a caret on press).
    /// Quick presses counted toward double/triple click: frame, count, time, place.
    press_frame: u64,
    press_count: u8,
    press_time: f64,
    press_pos: Option<Pos2>,
    /// A drag selecting text: (click count, anchor start, anchor end).
    drag: Option<(u8, usize, usize)>,
    /// The edit box as last laid out: (row key, galley, text origin, rect).
    edit_layout: Option<(String, std::sync::Arc<egui::Galley>, Pos2, Rect)>,
    cache: HashMap<(String, bool), Vec<Category>>,
    /// Row labels drawn last frame, in order (the self-test reads them).
    pub drawn: Vec<String>,
    /// Studio's BrickColor palette popup, when open.
    pub brick_popup: Option<crate::color_editors::BrickPopup>,
    /// The ribbon Color button's BrickColor popup (crate::ribbon_quick).
    pub ribbon_brick: Option<crate::color_editors::BrickPopup>,
    /// Studio's Select Color dialog, when open.
    pub color_dialog: Option<crate::color_editors::ColorDialog>,
    /// The dialog's 16 custom colours (kept for the session, as Qt does).
    pub custom_colors: [Option<[u8; 3]>; 16],
    /// A slider being dragged: the property and the values before.
    pub slider_drag: Option<SliderDrag>,
    /// The ranged row last clicked: its editor stays shown (Studio shows it
    /// on the selected row; rustudio also on hover).
    pub active_row: Option<String>,
    active_for: Option<Ref>,
    /// Each drawn row's rect, by property name (the self-test reads them).
    pub row_rects: HashMap<String, Rect>,
}

pub struct SliderDrag {
    pub prop: String,
    pub originals: Vec<(Ref, Variant)>,
}

/// A number property's value as f64.
fn number_of(v: &Variant) -> Option<f64> {
    match v {
        Variant::Float32(f) => Some(*f as f64),
        Variant::Float64(f) => Some(*f),
        Variant::Int32(i) => Some(*i as f64),
        Variant::Int64(i) => Some(*i as f64),
        _ => None,
    }
}

/// `x` in the same number type as `like`.
fn number_like(like: &Variant, x: f64) -> Variant {
    match like {
        Variant::Float64(_) => Variant::Float64(x),
        Variant::Int32(_) => Variant::Int32(x.round() as i32),
        Variant::Int64(_) => Variant::Int64(x.round() as i64),
        _ => Variant::Float32(x as f32),
    }
}

/// `rgb` in the same colour type as `like` (Color3uint8 for parts).
fn color_variant(like: &Variant, c: [u8; 3]) -> Variant {
    match like {
        Variant::Color3(_) => Variant::Color3(Color3::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0)),
        _ => Variant::Color3uint8(Color3uint8::new(c[0], c[1], c[2])),
    }
}

fn has_prop(class: &str, name: &str) -> bool {
    let db = doc::db();
    let Some(desc) = db.classes.get(class) else {
        return false;
    };
    db.superclasses_iter(desc).any(|c| c.properties.contains_key(name))
}

/// The filter field's id (Ctrl+Shift+P focuses it, as in Studio).
pub const PROPS_FILTER_ID: &str = "properties_filter";

/// The Properties panel's title: Studio's "Properties - Class "Name"".
pub fn panel_title(ed: &Editor) -> String {
    let sel = &ed.doc.selection;
    match sel.first().and_then(|r| ed.doc.dom.get_by_ref(*r)) {
        Some(i) if sel.len() > 1 => format!("Properties - {} \"{}\" (+{} more)", i.class, i.name, sel.len() - 1),
        Some(i) => format!("Properties - {} \"{}\"", i.class, i.name),
        None => "Properties".into(),
    }
}

/// Studio's number text: up to 3 decimals, no trailing zeros.
pub fn fmt_num(f: f64) -> String {
    let s = format!("{f:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn fmt_v3(v: Vec3) -> String {
    format!("{}, {}, {}", fmt_num(v.x as f64), fmt_num(v.y as f64), fmt_num(v.z as f64))
}

/// Parses `n` comma-separated numbers; a single number fills every
/// component (typing "0" into a Vector3 gives 0, 0, 0).
fn parse_nums(s: &str, n: usize) -> Option<Vec<f32>> {
    let p: Vec<f32> = s.split(',').map(|x| x.trim().trim_matches(['[', ']'])).filter(|x| !x.is_empty()).map(|x| x.parse().ok()).collect::<Option<_>>()?;
    match p.len() {
        1 => Some(vec![p[0]; n]),
        l if l == n => Some(p),
        _ => None,
    }
}

/// The run of non-separator characters around char index `c` (a whole
/// number like "-2840.53" in "1, -2840.53, 3").
fn word_bounds(text: &str, c: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let sep = |ch: char| ch == ',' || ch.is_whitespace();
    let c = c.min(chars.len());
    let mut a = c;
    while a > 0 && !sep(chars[a - 1]) {
        a -= 1;
    }
    let mut b = c;
    while b < chars.len() && !sep(chars[b]) {
        b += 1;
    }
    (a, b)
}

fn parse_v3(s: &str) -> Option<Vec3> {
    parse_nums(s, 3).map(|p| Vec3::new(p[0], p[1], p[2]))
}

/// The palette BrickColor closest to a colour.
pub(crate) fn nearest_brick(rgb: [u8; 3]) -> BrickColor {
    (1..=1032u16)
        .filter_map(BrickColor::from_number)
        .min_by_key(|b| {
            let c = crate::scene::brick_rgb(*b);
            (0..3).map(|i| (c[i] as i32 - rgb[i] as i32).pow(2)).sum::<i32>()
        })
        .unwrap_or(BrickColor::MediumStoneGrey)
}

/// The BrickColor whose colour this exactly is (Studio adds its name).
fn brick_name(rgb: [u8; 3]) -> Option<String> {
    (1..=1032u16)
        .filter_map(BrickColor::from_number)
        .find(|b| crate::scene::brick_rgb(*b) == rgb)
        .map(|b| b.to_string())
}

fn mul(a: &Xform, b: &Xform) -> Xform {
    Xform { pos: a.pos + a.rot * b.pos, rot: a.rot * b.rot }
}

fn cf_of(v: Option<Variant>) -> Option<Xform> {
    match v? {
        Variant::CFrame(cf) => Some(Xform::from_cframe(&cf)),
        Variant::OptionalCFrame(Some(cf)) => Some(Xform::from_cframe(&cf)),
        _ => None,
    }
}

/// A value's text as Studio shows it (None: drawn by its own widget).
fn value_text(doc: &doc::Document, v: &Variant) -> String {
    match v {
        Variant::String(s) => s.lines().next().unwrap_or("").to_string(),
        Variant::Int32(i) => i.to_string(),
        Variant::Int64(i) => i.to_string(),
        Variant::Float32(f) => fmt_num(*f as f64),
        Variant::Float64(f) => fmt_num(*f),
        Variant::Vector3(v) => fmt_v3(crate::math::v3(*v)),
        Variant::Vector2(v) => format!("{}, {}", fmt_num(v.x as f64), fmt_num(v.y as f64)),
        Variant::Vector3int16(v) => format!("{}, {}, {}", v.x, v.y, v.z),
        Variant::Vector2int16(v) => format!("{}, {}", v.x, v.y),
        Variant::UDim(u) => format!("{}, {}", fmt_num(u.scale as f64), u.offset),
        Variant::UDim2(u) => format!(
            "{{{}, {}}}, {{{}, {}}}",
            fmt_num(u.x.scale as f64),
            u.x.offset,
            fmt_num(u.y.scale as f64),
            u.y.offset
        ),
        Variant::NumberRange(r) => format!("{} {}", fmt_num(r.min as f64), fmt_num(r.max as f64)),
        Variant::Ref(r) => doc.dom.get_by_ref(*r).map(|i| i.name.clone()).unwrap_or_default(),
        Variant::ContentId(c) => c.as_str().to_string(),
        Variant::Content(c) => c.as_uri().unwrap_or_default().to_string(),
        Variant::Font(f) => f.family.rsplit('/').next().unwrap_or("").trim_end_matches(".json").to_string(),
        Variant::Faces(f) => format!("{f:?}"),
        Variant::Axes(a) => format!("{a:?}"),
        Variant::UniqueId(u) => u.to_string(),
        Variant::PhysicalProperties(_) => String::new(),
        Variant::CFrame(cf) => fmt_v3(crate::math::v3(cf.position)),
        other => format!("{:?}", other.ty()),
    }
}

/// What the panel did this frame.
enum Edit {
    Prop(String, Variant),
    Frame(Frame, Option<Vec3>, Option<Vec3>),
    AddTag(String),
    RemoveTag(String),
}

struct Painter<'a> {
    ui: &'a mut egui::Ui,
    width: f32,
    x0: f32,
}

impl Painter<'_> {
    fn row(&mut self, key: &str, h: f32) -> (Rect, egui::Response) {
        let (r, resp) = self.ui.allocate_exact_size(Vec2::new(self.width, h), Sense::click());
        let _ = key;
        (r, resp)
    }
}

impl Editor {
    pub fn properties_ui(&mut self, ui: &mut egui::Ui) {
        // Studio: the filter field always, its shortcut in the hint; the
        // selection's class and name are in the panel's title.
        ui.add(
            egui::TextEdit::singleline(&mut self.props.filter)
                .id(egui::Id::new(PROPS_FILTER_ID))
                .hint_text(format!("Filter Properties ({})", crate::studio_ui::shortcut_label("Ctrl+Shift+P")))
                .desired_width(f32::INFINITY),
        );
        ui.add_space(2.0);
        self.props.drawn.clear();
        self.ui_rects.insert("props.panel", ui.max_rect());
        let sel = self.doc.selection.clone();
        let Some(&first) = sel.first() else {
            self.hud.property_rows = 0;
            return;
        };
        let Some(class) = self.doc.class_of(first).map(|c| c.to_string()) else {
            return;
        };
        if self.props.active_for != Some(first) {
            self.props.active_for = Some(first);
            self.props.active_row = None;
        }
        let can_collide = !matches!(self.doc.prop(first, "CanCollide"), Some(Variant::Bool(false)));
        let cats = self
            .props
            .cache
            .entry((class.clone(), can_collide))
            .or_insert_with(|| crate::props_layout::rows(&class, can_collide))
            .clone();
        let filter = self.props.filter.trim().to_lowercase();
        let mut edits: Vec<Edit> = Vec::new();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("props_scroll")
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let width = ui.available_width();
                let x0 = ui.cursor().min.x;
                let mut p = Painter { ui, width, x0 };
                for cat in &cats {
                    let rows: Vec<&Row> = cat
                        .rows
                        .iter()
                        .filter(|r| {
                            filter.is_empty()
                                || match r {
                                    Row::Prop(n, _) => n.to_lowercase().contains(&filter),
                                    Row::Frame(l, _) => l.to_lowercase().contains(&filter),
                                }
                        })
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    let folded = filter.is_empty() && self.category_folded(cat);
                    if self.band(&mut p, &cat.name, folded, None) {
                        self.toggle_category(cat);
                    }
                    if folded {
                        continue;
                    }
                    for r in rows {
                        match r {
                            Row::Prop(name, ro) => self.prop_row(&mut p, first, &class, name, *ro, &mut edits),
                            Row::Frame(label, f) => self.frame_rows(&mut p, first, label, *f, &mut edits),
                        }
                    }
                }
                self.tags_section(&mut p, first, &mut edits);
                self.attributes_section(&mut p, first);
            });
        self.hud.property_rows = self.props.drawn.len();
        let ctx = ui.ctx().clone();
        self.color_editors_ui(&ctx, &mut edits);
        self.apply_prop_edits(&sel, first, edits);
    }

    fn category_folded(&self, cat: &Category) -> bool {
        if cat.collapsed {
            !self.props.unfolded.contains(&cat.name)
        } else {
            self.props.folded.contains(&cat.name)
        }
    }

    fn toggle_category(&mut self, cat: &Category) {
        let set = if cat.collapsed { &mut self.props.unfolded } else { &mut self.props.folded };
        if !set.remove(&cat.name) {
            set.insert(cat.name.clone());
        }
    }

    /// A category band; returns true when it was clicked (fold / unfold).
    /// `plus`: draw Studio's "+" at the right (Tags, Attributes) and report
    /// its click through the returned flag's second meaning via `plus_hit`.
    fn band(&mut self, p: &mut Painter, name: &str, folded: bool, plus_hit: Option<&mut bool>) -> bool {
        let (r, resp) = p.row(name, CAT_H);
        let painter = p.ui.painter();
        painter.rect_filled(r, 0.0, BAND);
        let tri = Rect::from_center_size(Pos2::new(r.min.x + 10.0, r.center().y), Vec2::splat(12.0));
        crate::ti::paint(painter, tri, if folded { "caret-right-filled" } else { "caret-down-filled" }, TEXT);
        painter.text(
            Pos2::new(r.min.x + INDENT, r.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            FontId::new(FONT, egui::FontFamily::Name("bold".into())),
            TEXT,
        );
        let mut clicked = resp.clicked();
        if let Some(hit) = plus_hit {
            let pr = Rect::from_center_size(Pos2::new(r.max.x - 14.0, r.center().y), Vec2::splat(14.0));
            crate::ti::paint(p.ui.painter(), pr, "plus", TEXT);
            if clicked && resp.interact_pointer_pos().is_some_and(|q| pr.expand(3.0).contains(q)) {
                *hit = true;
                clicked = false;
            }
        }
        let _ = p.x0;
        clicked
    }

    /// The two-column cell frame of a row: (row rect, name right edge, value rect).
    fn cells(p: &mut Painter, key: &str) -> (Rect, Rect, egui::Response) {
        let (r, resp) = p.row(key, ROW_H + 1.0);
        let painter = p.ui.painter();
        painter.line_segment([Pos2::new(r.min.x, r.max.y - 0.5), Pos2::new(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, BAND));
        let div = r.min.x + (r.width() * DIVIDER).round();
        painter.line_segment([Pos2::new(div, r.min.y), Pos2::new(div, r.max.y)], Stroke::new(1.0, BAND));
        let value = Rect::from_min_max(Pos2::new(div + 5.0, r.min.y), Pos2::new(r.max.x - 4.0, r.max.y - 1.0));
        (r, value, resp)
    }

    fn name_text(p: &mut Painter, r: Rect, depth: f32, name: &str, color: Color32, arrow: Option<bool>) {
        // Clipped at the divider, as Studio's name column is.
        let div = r.min.x + (r.width() * DIVIDER).round();
        let clip = if depth < 0.0 { r } else { Rect::from_min_max(r.min, Pos2::new(div - 2.0, r.max.y)) };
        let painter = p.ui.painter_at(clip);
        let x = r.min.x + INDENT + depth * 18.0;
        if let Some(open) = arrow {
            let tri = Rect::from_center_size(Pos2::new(x - 15.0, r.center().y), Vec2::splat(11.0));
            crate::ti::paint(&painter, tri, if open { "caret-down-filled" } else { "caret-right-filled" }, color);
        }
        painter.text(Pos2::new(x, r.center().y), egui::Align2::LEFT_CENTER, name, FontId::proportional(FONT), color);
    }

    fn value_label(p: &mut Painter, v: Rect, text: &str, color: Color32) {
        let painter = p.ui.painter_at(v);
        painter.text(Pos2::new(v.min.x, v.center().y), egui::Align2::LEFT_CENTER, text, FontId::proportional(FONT), color);
    }

    /// An editable text cell: typing replaces the shown text, Enter or
    /// leaving commits; returns the committed text.
    ///
    /// The mouse selects as Studio's edit box does, on the press rather than
    /// egui's release: a click opens the cell with everything selected, a
    /// second click picks the number under the pointer, a third the whole
    /// text, and dragging after a double-click extends word by word.
    fn text_cell(&mut self, p: &mut Painter, key: &str, v: Rect, shown: &str, ro: bool, clicked: bool) -> Option<String> {
        use egui::text::{CCursor, CCursorRange};
        let eid = egui::Id::new(("prop_edit", key));
        let ctx = p.ui.ctx().clone();
        let (pressed, down, pos, now, shift) =
            p.ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos(), i.time, i.modifiers.shift));
        // Count quick presses once a frame (1 = caret, 2 = word, 3 = all).
        if pressed && self.props.press_frame != self.frame_no {
            self.props.press_frame = self.frame_no;
            let near = pos.zip(self.props.press_pos).is_some_and(|(a, b)| a.distance(b) < 6.0);
            self.props.press_count = if near && now - self.props.press_time < 0.4 { self.props.press_count % 3 + 1 } else { 1 };
            self.props.press_time = now;
            self.props.press_pos = pos;
        }
        let editing = self.props.editing.as_ref().is_some_and(|(k, _)| k == key);
        if !editing {
            Self::value_label(p, v, shown, if ro { READ_ONLY } else { TEXT });
            if clicked && !ro {
                self.props.editing = Some((key.to_string(), shown.to_string()));
                let mut st = egui::text_edit::TextEditState::default();
                st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(shown.chars().count()))));
                st.store(&ctx, eid);
                self.props.drag = None;
                self.props.edit_layout = None;
                ctx.request_repaint();
            }
            return None;
        }
        let mut text = self.props.editing.as_ref().map(|(_, t)| t.clone()).unwrap_or_default();
        let n = text.chars().count();
        // Selection from the pointer, worked out against last frame's layout
        // so this frame already paints it.
        let mut sel: Option<(usize, usize)> = None;
        if let Some((_, galley, gpos, rect)) = self.props.edit_layout.clone().filter(|l| l.0 == key) {
            let char_at = |q: Pos2| usize::from(galley.cursor_from_pos(q - gpos).index).min(n);
            match (pos, self.props.drag) {
                (Some(q), _) if pressed && rect.contains(q) => {
                    let c = char_at(q);
                    let (a, b) = match self.props.press_count {
                        1 if shift => {
                            let anchor = egui::text_edit::TextEditState::load(&ctx, eid)
                                .and_then(|s| s.cursor.char_range())
                                .map_or(c, |r| usize::from(r.secondary.index));
                            (anchor, c)
                        }
                        1 => (c, c),
                        2 => word_bounds(&text, c),
                        _ => (0, n),
                    };
                    self.props.drag = Some((self.props.press_count, a, b));
                    sel = Some((a, b));
                }
                (Some(q), Some((mode, a, b))) if down && !pressed => {
                    let c = char_at(q);
                    sel = Some(match mode {
                        1 => (a, c),
                        2 => {
                            let (wa, wb) = word_bounds(&text, c);
                            if wa < a { (b, wa) } else { (a, wb.max(b)) }
                        }
                        _ => (0, n),
                    });
                }
                _ => {}
            }
        }
        if !down {
            self.props.drag = None;
        }
        if let Some((a, b)) = sel {
            let mut st = egui::text_edit::TextEditState::load(&ctx, eid).unwrap_or_default();
            st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
            st.store(&ctx, eid);
        }
        let out = p
            .ui
            .scope_builder(egui::UiBuilder::new().max_rect(v).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)), |ui| {
                egui::TextEdit::singleline(&mut text).id(eid).frame(egui::Frame::NONE).font(FontId::proportional(FONT)).show(ui)
            })
            .inner;
        let resp = out.response.response.clone();
        // Sits over the edit box so egui's own click handling never runs.
        p.ui.interact(resp.rect, eid.with("mouse"), egui::Sense::CLICK | egui::Sense::DRAG).on_hover_cursor(egui::CursorIcon::Text);
        self.props.edit_layout = Some((key.to_string(), out.galley.clone(), out.galley_pos, resp.rect));
        resp.request_focus();
        let (enter, esc) = p.ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        if esc {
            self.props.editing = None;
            return None;
        }
        if enter || resp.lost_focus() {
            self.props.editing = None;
            return Some(text);
        }
        self.props.editing = Some((key.to_string(), text));
        None
    }

    fn prop_row(&mut self, p: &mut Painter, first: Ref, class: &str, name: &str, ro_api: bool, edits: &mut Vec<Edit>) {
        // ClassName / Parent aren't stored properties.
        let parent_is_root = self.doc.dom.get_by_ref(first).is_some_and(|i| i.parent() == self.doc.dom.root_ref());
        let value = match name {
            "ClassName" => Some(Variant::String(class.to_string())),
            "Name" => self.doc.dom.get_by_ref(first).map(|i| Variant::String(i.name.clone())),
            "Parent" if parent_is_root => Some(Variant::String(self.doc.title.clone())),
            "Parent" => self.doc.dom.get_by_ref(first).map(|i| Variant::Ref(i.parent())),
            // A Part resizes on every face by whole studs (Studio: "Right, Top,
            // Back, Left, Bottom, Front", 1).
            "ResizeableFaces" if self.doc.prop(first, name).is_none() => {
                Some(Variant::String("Right, Top, Back, Left, Bottom, Front".into()))
            }
            "ResizeIncrement" if self.doc.prop(first, name).is_none() => Some(Variant::Int32(1)),
            // A part keeps only Color; its BrickColor is the nearest palette
            // colour (Studio shows both).
            "BrickColor" => self.doc.prop(first, name).or_else(|| match self.doc.prop(first, "Color") {
                Some(Variant::Color3uint8(c)) => Some(Variant::BrickColor(nearest_brick([c.r, c.g, c.b]))),
                _ => None,
            }),
            // A part's mass facts, read-only and worked out as Studio shows
            // them (captured RefPart 4×1.2×2 Plastic: Mass 6.72, AssemblyMass
            // inf when anchored, root itself, centre at its position).
            "Mass" | "AssemblyMass" | "AssemblyRootPart" | "AssemblyCenterOfMass" if doc::is_a(class, "BasePart") => {
                let size = match self.doc.prop(first, "Size") {
                    Some(Variant::Vector3(s)) => s,
                    _ => Vector3::new(4.0, 1.2, 2.0),
                };
                let density = match self.doc.prop(first, "CustomPhysicalProperties") {
                    Some(Variant::PhysicalProperties(PhysicalProperties::Custom(c))) => c.density(),
                    _ => crate::mech::PART_DENSITY,
                };
                let mass = size.x * size.y * size.z * density;
                let anchored = matches!(self.doc.prop(first, "Anchored"), Some(Variant::Bool(true)));
                Some(match name {
                    "Mass" => Variant::Float32(mass),
                    "AssemblyMass" if anchored => Variant::String("inf".into()),
                    "AssemblyMass" => Variant::Float32(mass),
                    "AssemblyRootPart" => Variant::Ref(first),
                    _ => Variant::Vector3(crate::math::rv3(crate::scene::xform_of(&self.doc, first).map(|x| x.pos).unwrap_or_default())),
                })
            }
            // Studio shows PhysicalProperties as a checkbox: Current is always
            // ticked (greyed), Custom is ticked when set.
            "CurrentPhysicalProperties" => Some(Variant::Bool(true)),
            // A script runs unless Disabled or Enabled = false (as Play reads it).
            "Enabled" if doc::is_a(class, "BaseScript") => Some(Variant::Bool(
                !matches!(self.doc.prop(first, "Disabled"), Some(Variant::Bool(true)))
                    && !matches!(self.doc.prop(first, "Enabled"), Some(Variant::Bool(false))),
            )),
            // A ScreenGui's AbsoluteSize is the view's size (Studio: 863, 675
            // for its 863×675 viewport).
            "AbsoluteSize" if doc::is_a(class, "LayerCollector") => {
                let s = self.viewport_rect.size();
                Some(Variant::Vector2(Vector2::new(s.x.round(), s.y.round())))
            }
            // Every instance has a UniqueId in Studio; one not saved yet shows
            // its session id.
            "UniqueId"
                if match self.doc.prop(first, name) {
                    Some(Variant::UniqueId(u)) => u.to_string().bytes().all(|c| c == b'0'),
                    other => other.is_none(),
                } =>
            {
                Some(Variant::String(format!("{first}")))
            }
            "CustomPhysicalProperties" => Some(Variant::Bool(matches!(
                self.doc.prop(first, name),
                Some(Variant::PhysicalProperties(PhysicalProperties::Custom(_)))
            ))),
            _ => self.doc.prop(first, name).or_else(|| typed_empty(class, name)),
        };
        let ro = ro_api
            || matches!(name, "ClassName" | "UniqueId" | "IsInSandbox")
            || (name == "Parent" && parent_is_root);
        let color = if ro { READ_ONLY } else { TEXT };
        let vec_like = matches!(value, Some(Variant::Vector3(_)) | Some(Variant::Vector2(_)));
        let open = self.props.open_rows.contains(name);
        let (r, v, resp) = Self::cells(p, name);
        self.props.row_rects.insert(name.to_string(), r);
        if self.props.active_row.as_deref() == Some(name) {
            // Studio's selected row: the name cell in the selection blue.
            let div = r.min.x + (r.width() * DIVIDER).round();
            p.ui.painter().rect_filled(Rect::from_min_max(r.min, Pos2::new(div, r.max.y - 1.0)), 0.0, Color32::from_rgb(19, 35, 93));
        }
        let custom_set = name == "CustomPhysicalProperties" && matches!(value, Some(Variant::Bool(true)));
        let arrow = vec_like
            .then_some(open)
            .or((matches!(value, Some(Variant::Faces(_))) || matches!(name, "ResizeableFaces" | "CurrentPhysicalProperties") || custom_set).then_some(false));
        Self::name_text(p, r, 0.0, name, color, arrow);
        self.props.drawn.push(name.to_string());
        let on_name = resp.interact_pointer_pos().is_some_and(|q| q.x < v.min.x);
        if resp.clicked() && on_name && vec_like {
            if !self.props.open_rows.remove(name) {
                self.props.open_rows.insert(name.to_string());
            }
        }
        let clicked_value = resp.clicked() && !on_name;
        let Some(value) = value else {
            return;
        };
        match &value {
            Variant::Bool(b) => {
                let br = Rect::from_min_size(Pos2::new(v.min.x + 2.0, v.center().y - 7.0), Vec2::splat(14.0));
                let painter = p.ui.painter();
                if *b {
                    painter.rect_filled(br, 2.0, if ro { READ_ONLY } else { Color32::from_rgb(230, 231, 233) });
                    crate::ti::paint(painter, br.shrink(1.0), "check", Color32::from_rgb(25, 26, 31));
                } else {
                    painter.rect_filled(br, 2.0, BOX_OFF);
                }
                if clicked_value && !ro {
                    edits.push(Edit::Prop(name.to_string(), Variant::Bool(!b)));
                }
            }
            Variant::Color3(_) | Variant::Color3uint8(_) | Variant::BrickColor(_) => {
                let (rgb, label) = match &value {
                    Variant::Color3(c) => {
                        let rgb = [(c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8];
                        (rgb, None)
                    }
                    Variant::Color3uint8(c) => ([c.r, c.g, c.b], None),
                    Variant::BrickColor(b) => (crate::scene::brick_rgb(*b), Some(b.to_string())),
                    _ => unreachable!(),
                };
                let sw = Rect::from_min_size(Pos2::new(v.min.x + 2.0, v.center().y - 6.0), Vec2::splat(12.0));
                p.ui.painter().rect_filled(sw, 1.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                p.ui.painter().rect_stroke(sw, 1.0, Stroke::new(1.0, BAND), egui::StrokeKind::Inside);
                let text = match label {
                    Some(n) => n,
                    None => {
                        let base = format!("[{}, {}, {}]", rgb[0], rgb[1], rgb[2]);
                        // A part's Color names its BrickColor when it is one (captured).
                        match brick_name(rgb).filter(|_| name == "Color") {
                            Some(n) => format!("{base} ({n})"),
                            None => base,
                        }
                    }
                };
                let tv = Rect::from_min_max(Pos2::new(sw.max.x + 6.0, v.min.y), v.max);
                self.ui_rects.insert(if matches!(value, Variant::BrickColor(_)) { "props.brickcolor" } else { "props.color" }, v);
                if ro {
                    Self::value_label(p, tv, &text, color);
                } else if matches!(value, Variant::BrickColor(_)) {
                    // Studio's palette popup (crate::color_editors).
                    Self::value_label(p, tv, &text, color);
                    if clicked_value {
                        self.props.brick_popup = Some(crate::color_editors::BrickPopup {
                            prop: name.to_string(),
                            origin: v.left_bottom() + Vec2::new(0.0, 1.0),
                            opened_frame: self.frame_no,
                            hovered: None,
                        });
                    }
                } else {
                    // The swatch opens Studio's Select Color dialog; the text
                    // edits "r, g, b" (as Studio's row editor does).
                    let on_swatch = resp.interact_pointer_pos().is_some_and(|q| q.x < tv.min.x);
                    if clicked_value && on_swatch {
                        self.open_color_dialog(name, rgb);
                    }
                    let plain = format!("{}, {}, {}", rgb[0], rgb[1], rgb[2]);
                    let editing = self.props.editing.as_ref().is_some_and(|(k, _)| k == name);
                    if !editing {
                        Self::value_label(p, tv, &text, color);
                    }
                    if let Some(t) = self.text_cell(p, name, if editing { tv } else { Rect::NOTHING }, &plain, false, clicked_value && !on_swatch) {
                        if let Some(n) = parse_nums(&t, 3) {
                            let c = [n[0].clamp(0.0, 255.0) as u8, n[1].clamp(0.0, 255.0) as u8, n[2].clamp(0.0, 255.0) as u8];
                            edits.push(Edit::Prop(name.to_string(), color_variant(&value, c)));
                        }
                    }
                }
            }
            Variant::Enum(_) | Variant::EnumItem(_) => {
                let (ty, cur) = match &value {
                    Variant::Enum(e) => (enum_type_of(class, name), e.to_u32()),
                    Variant::EnumItem(e) => (Some(e.ty.clone()), e.value),
                    _ => unreachable!(),
                };
                let items: Vec<(String, u32)> = ty
                    .as_deref()
                    .and_then(|t| doc::db().enums.get(t))
                    .map(|e| {
                        let mut v: Vec<(String, u32)> = e.items.iter().map(|(k, v)| (k.to_string(), *v)).collect();
                        v.sort_by_key(|(_, n)| *n);
                        v
                    })
                    .unwrap_or_default();
                let cur_name = items.iter().find(|(_, n)| *n == cur).map(|(k, _)| k.clone()).unwrap_or_else(|| cur.to_string());
                Self::value_label(p, v, &cur_name, color);
                if !ro {
                    let id = egui::Id::new(("enum_popup", name));
                    if clicked_value {
                        egui::Popup::open_id(p.ui.ctx(), id);
                    }
                    let mut picked = None;
                    let just_opened = clicked_value;
                    // Studio's enum list: as wide as the value cell, compact
                    // rows, scrollbar on the right edge, ~12 rows tall.
                    const ITEM_H: f32 = 19.0;
                    let width = v.width().max(120.0);
                    egui::Popup::new(id, p.ui.ctx().clone(), egui::PopupAnchor::Position(v.left_bottom()), p.ui.layer_id())
                        .open_memory(None)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                        .width(width)
                        .frame(egui::Frame::popup(p.ui.style()).inner_margin(egui::Margin::same(2)).corner_radius(0))
                        .show(|ui| {
                            ui.set_width(width - 4.0);
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                            egui::ScrollArea::vertical()
                                .max_height(ITEM_H * 12.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    for (k, n) in &items {
                                        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ITEM_H), Sense::click());
                                        let sel = *n == cur;
                                        if sel || resp.hovered() {
                                            let bg = if sel { Color32::from_rgb(19, 35, 93) } else { Color32::from_rgb(45, 48, 58) };
                                            ui.painter().rect_filled(r, 0.0, bg);
                                        }
                                        ui.painter().text(Pos2::new(r.min.x + 6.0, r.center().y), egui::Align2::LEFT_CENTER, k, FontId::proportional(FONT), TEXT);
                                        if sel && just_opened {
                                            resp.scroll_to_me(Some(egui::Align::Center));
                                        }
                                        if resp.clicked() {
                                            picked = Some(*n);
                                        }
                                    }
                                });
                        });
                    if let Some(n) = picked.filter(|n| *n != cur) {
                        edits.push(Edit::Prop(name.to_string(), Variant::Enum(Enum::from_u32(n))));
                    }
                }
            }
            Variant::Vector3(v3) => {
                let g = crate::math::v3(*v3);
                if let Some(t) = self.text_cell(p, name, v, &fmt_v3(g), ro, clicked_value) {
                    if let Some(n) = parse_v3(&t) {
                        edits.push(Edit::Prop(name.to_string(), Variant::Vector3(crate::math::rv3(n))));
                    }
                }
                if open {
                    for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                        let key = format!("{name}.{axis}");
                        let (r, vv, resp) = Self::cells(p, &key);
                        Self::name_text(p, r, 1.0, axis, color, None);
                        if let Some(t) = self.text_cell(p, &key, vv, &fmt_num(g[i] as f64), ro, resp.clicked()) {
                            if let Ok(x) = t.trim().parse::<f32>() {
                                let mut n = g;
                                n[i] = x;
                                edits.push(Edit::Prop(name.to_string(), Variant::Vector3(crate::math::rv3(n))));
                            }
                        }
                    }
                }
            }
            Variant::Tags(_) | Variant::Attributes(_) => {}
            other if !ro && number_of(other).is_some() && crate::prop_slider::spec_for(class, name).is_some() => {
                let spec = crate::prop_slider::spec_for(class, name).unwrap_or(crate::prop_slider::SliderSpec { min: 0.0, max: 1.0, ticks: 20.0, square: false });
                self.slider_cell(p, name, other, spec, v, &resp, clicked_value, edits);
            }
            other => {
                let shown = value_text(&self.doc, other);
                let editable = !ro
                    && matches!(other, Variant::String(_) | Variant::Int32(_) | Variant::Int64(_) | Variant::Float32(_) | Variant::Float64(_) | Variant::Vector2(_) | Variant::ContentId(_) | Variant::Content(_));
                if let Some(t) = self.text_cell(p, name, v, &shown, ro, clicked_value && editable) {
                    if let Some(nv) = parse_like(other, &t) {
                        if name == "Name" {
                            edits.push(Edit::Prop("Name".into(), nv));
                        } else {
                            edits.push(Edit::Prop(name.to_string(), nv));
                        }
                    }
                }
            }
        }
    }

    /// A ranged number (crate::prop_slider): the value as text, and, while
    /// the row is hovered or picked, Studio's spin box and slider. Dragging
    /// the slider updates the selection live and is one undo step.
    #[allow(clippy::too_many_arguments)]
    fn slider_cell(
        &mut self,
        p: &mut Painter,
        name: &str,
        value: &Variant,
        spec: crate::prop_slider::SliderSpec,
        v: Rect,
        resp: &egui::Response,
        clicked_value: bool,
        edits: &mut Vec<Edit>,
    ) {
        use crate::prop_slider as ps;
        let cur = number_of(value).unwrap_or(0.0);
        let dragging = self.props.slider_drag.as_ref().is_some_and(|d| d.prop == name);
        if clicked_value {
            self.props.active_row = Some(name.to_string());
        }
        let editing = self.props.editing.as_ref().is_some_and(|(k, _)| k == name);
        // The pointer over the row (a press on the slider takes the row's
        // own hover away, so ask the pointer), a drag, typing, or picked.
        let over = p.ui.ctx().pointer_latest_pos().is_some_and(|q| resp.rect.contains(q));
        let held = p.ui.ctx().is_being_dragged(p.ui.id().with(("slider", name)));
        let shown = over || held || dragging || editing || self.props.active_row.as_deref() == Some(name);
        if !shown {
            Self::value_label(p, v, &fmt_num(cur), TEXT);
            return;
        }
        // Studio's editor fills the value cell from the divider.
        let cell = Rect::from_min_max(Pos2::new(v.min.x - 4.0, v.min.y + 1.0), Pos2::new(v.max.x + 4.0, v.max.y));
        let spin = Rect::from_min_size(cell.min, Vec2::new(ps::SPIN_W.min(cell.width() * 0.5), cell.height()));
        let slider = Rect::from_min_max(Pos2::new(spin.max.x + 2.0, cell.min.y), cell.max);
        self.ui_rects.insert("props.slider", slider);
        self.ui_rects.insert("props.spin", spin);
        let painter = p.ui.painter().clone();
        painter.rect_filled(cell, 0.0, ps::SPIN_BG);
        let arrows = Rect::from_min_max(Pos2::new(spin.max.x - 12.0, spin.min.y), spin.max);
        let up = Rect::from_min_max(arrows.min, Pos2::new(arrows.max.x, arrows.center().y));
        let down = Rect::from_min_max(Pos2::new(arrows.min.x, arrows.center().y), arrows.max);
        crate::ti::paint(&painter, Rect::from_center_size(up.center() + Vec2::new(0.0, 1.5), Vec2::splat(9.0)), "chevron-up", TEXT);
        crate::ti::paint(&painter, Rect::from_center_size(down.center() - Vec2::new(0.0, 1.5), Vec2::splat(9.0)), "chevron-down", TEXT);
        let text_rect = Rect::from_min_max(spin.min + Vec2::new(4.0, 0.0), Pos2::new(arrows.min.x - 2.0, spin.max.y));
        let on_text = resp.interact_pointer_pos().is_some_and(|q| text_rect.contains(q));
        if let Some(t) = self.text_cell(p, name, text_rect, &fmt_num(cur), false, clicked_value && on_text) {
            if let Some(nv) = parse_like(value, &t) {
                edits.push(Edit::Prop(name.to_string(), nv));
            }
        }
        // Arrows: one step (the slider's tick) up or down.
        for (rect, dir, key) in [(up, 1.0, "up"), (down, -1.0, "down")] {
            if p.ui.interact(rect, p.ui.id().with(("spin", name, key)), Sense::click()).clicked() {
                edits.push(Edit::Prop(name.to_string(), number_like(value, spec.nudge(cur, dir))));
                self.props.active_row = Some(name.to_string());
            }
        }
        ps::paint_slider(&painter, slider, spec.fraction(cur));
        let sresp = p.ui.interact(slider, p.ui.id().with(("slider", name)), Sense::click_and_drag());
        if sresp.drag_started() {
            let originals = self
                .doc
                .selection
                .iter()
                .filter_map(|r| self.doc.prop(*r, name).map(|v| (*r, v)))
                .collect();
            self.props.slider_drag = Some(SliderDrag { prop: name.to_string(), originals });
            self.props.active_row = Some(name.to_string());
        }
        if let Some(q) = sresp.interact_pointer_pos().filter(|_| sresp.dragged() || sresp.drag_started()) {
            let nv = spec.value_at(ps::fraction_at(slider, q.x));
            if (nv - cur).abs() > 1e-9 {
                let refs: Vec<Ref> = self.props.slider_drag.as_ref().map(|d| d.originals.iter().map(|(r, _)| *r).collect()).unwrap_or_default();
                for r in refs {
                    self.doc.set_prop_live(r, ustr(name), number_like(value, nv));
                }
            }
        }
        if sresp.drag_stopped() {
            if let Some(d) = self.props.slider_drag.take() {
                let changes: Vec<_> = d
                    .originals
                    .into_iter()
                    .filter_map(|(r, old)| {
                        let new = self.doc.dom.get_by_ref(r)?.properties.get(&ustr(name)).cloned()?;
                        (new != old).then(|| (r, ustr(name), Some(old), new))
                    })
                    .collect();
                if !changes.is_empty() {
                    self.doc.record_props(&format!("Set {name}"), changes);
                }
            }
        } else if sresp.clicked() {
            if let Some(q) = sresp.interact_pointer_pos() {
                edits.push(Edit::Prop(name.to_string(), number_like(value, spec.value_at(ps::fraction_at(slider, q.x)))));
            }
        }
    }

    /// Opens Studio's Select Color dialog for `prop` on the selection.
    pub fn open_color_dialog(&mut self, prop: &str, rgb: [u8; 3]) {
        let originals = self.doc.selection.iter().filter_map(|r| self.doc.prop(*r, prop).map(|v| (*r, v))).collect();
        self.props.color_dialog = Some(crate::color_editors::ColorDialog::new(prop, originals, rgb, self.frame_no));
    }

    /// The BrickColor popup and the Select Color dialog, when open.
    fn color_editors_ui(&mut self, ctx: &egui::Context, edits: &mut Vec<Edit>) {
        use crate::color_editors::{brick_popup_ui, color_dialog_ui, BrickPick, DialogOut};
        if let Some(mut pop) = self.props.brick_popup.take() {
            let first = self.doc.selection.first().copied();
            let current = first.and_then(|r| match self.doc.prop(r, &pop.prop) {
                Some(Variant::BrickColor(b)) => Some(b),
                _ => match self.doc.prop(r, "Color") {
                    Some(Variant::Color3uint8(c)) => Some(nearest_brick([c.r, c.g, c.b])),
                    _ => None,
                },
            });
            self.ui_rects.insert("brick.popup", Rect::from_min_size(crate::color_editors::popup_origin(ctx, &pop), crate::color_editors::POPUP_SIZE));
            match brick_popup_ui(ctx, &mut pop, current, self.frame_no) {
                BrickPick::None => self.props.brick_popup = Some(pop),
                BrickPick::Close => {}
                BrickPick::Picked(b) => edits.push(Edit::Prop(pop.prop.clone(), Variant::BrickColor(b))),
                BrickPick::MoreColors => {
                    // Studio's More Colors opens the colour dialog on Color.
                    let rgb = current.map(crate::scene::brick_rgb).unwrap_or([163, 162, 165]);
                    let prop = if first.and_then(|r| self.doc.class_of(r)).is_some_and(|c| doc::is_a(&c, "BasePart")) { "Color" } else { pop.prop.as_str() };
                    let prop = prop.to_string();
                    self.open_color_dialog(&prop, rgb);
                }
            }
        }
        if let Some(mut d) = self.props.color_dialog.take() {
            self.ui_rects.insert("color.dialog", Rect::from_min_size(crate::color_editors::dialog_origin(ctx), crate::color_editors::DIALOG_SIZE));
            let mut custom = self.props.custom_colors;
            let out = color_dialog_ui(ctx, &mut d, &mut custom, self.frame_no);
            self.props.custom_colors = custom;
            match out {
                DialogOut::None => self.props.color_dialog = Some(d),
                DialogOut::Changed(c) => {
                    // Live, as Studio: the part changes while the dialog is open.
                    for (r, old) in &d.originals {
                        self.doc.set_prop_live(*r, ustr(&d.prop), color_variant(old, c));
                    }
                    self.props.color_dialog = Some(d);
                }
                DialogOut::Ok => {
                    // Every live change of the dialog becomes one undo step.
                    let changes: Vec<_> = d
                        .originals
                        .iter()
                        .map(|(r, old)| (*r, ustr(&d.prop), Some(old.clone()), color_variant(old, d.rgb)))
                        .collect();
                    for (r, n, _, v) in &changes {
                        self.doc.set_prop_live(*r, *n, v.clone());
                    }
                    self.doc.record_props(&format!("Set {}", d.prop), changes);
                }
                DialogOut::Cancel => {
                    for (r, old) in &d.originals {
                        self.doc.set_prop_live(*r, ustr(&d.prop), old.clone());
                    }
                }
            }
        }
    }

    /// A CFrame row with Position and Orientation under it (Studio's Transform / Pivot rows).
    fn frame_rows(&mut self, p: &mut Painter, first: Ref, label: &str, f: Frame, edits: &mut Vec<Edit>) {
        let xf = self.frame_value(first, f).unwrap_or(Xform { pos: Vec3::ZERO, rot: glam::Mat3::IDENTITY });
        let (r, _, resp) = Self::cells(p, label);
        let folded = self.props.open_rows.contains(&format!("fold:{label}"));
        Self::name_text(p, r, 0.0, label, TEXT, Some(!folded));
        self.props.drawn.push(label.to_string());
        if resp.clicked() {
            let k = format!("fold:{label}");
            if !self.props.open_rows.remove(&k) {
                self.props.open_rows.insert(k);
            }
        }
        if folded {
            return;
        }
        for (part, value) in [("Position", xf.pos), ("Orientation", xf.orientation_deg())] {
            let key = format!("{label}.{part}");
            let (r, v, resp) = Self::cells(p, &key);
            Self::name_text(p, r, 1.0, part, TEXT, Some(false));
            let on_value = resp.interact_pointer_pos().is_some_and(|q| q.x >= v.min.x);
            if let Some(t) = self.text_cell(p, &key, v, &fmt_v3(value), false, resp.clicked() && on_value) {
                if let Some(n) = parse_v3(&t) {
                    let (pos, ori) = if part == "Position" { (Some(n), None) } else { (None, Some(n)) };
                    edits.push(Edit::Frame(f, pos, ori));
                }
            }
        }
    }

    fn frame_value(&self, r: Ref, f: Frame) -> Option<Xform> {
        let offset = cf_of(self.doc.prop(r, "PivotOffset")).unwrap_or(Xform { pos: Vec3::ZERO, rot: glam::Mat3::IDENTITY });
        match f {
            Frame::PartCFrame => cf_of(self.doc.prop(r, "CFrame")),
            Frame::PartOrigin => cf_of(self.doc.prop(r, "CFrame")).map(|c| mul(&c, &offset)),
            Frame::PivotOffset => Some(offset),
            Frame::ModelOrigin | Frame::WorldPivot => cf_of(self.doc.prop(r, "WorldPivotData")).or_else(|| {
                let pp = match self.doc.prop(r, "PrimaryPart") {
                    Some(Variant::Ref(p)) if p.is_some() => p,
                    _ => self.selected_parts().first().copied()?,
                };
                cf_of(self.doc.prop(pp, "CFrame"))
            }),
        }
    }

    fn tags_section(&mut self, p: &mut Painter, first: Ref, edits: &mut Vec<Edit>) {
        let mut plus = false;
        self.band(p, "Tags", false, Some(&mut plus));
        let tags: Vec<String> = match self.doc.prop(first, "Tags") {
            Some(Variant::Tags(t)) => t.iter().map(|s| s.to_string()).collect(),
            _ => Vec::new(),
        };
        if plus {
            self.props.editing = Some(("tag:new".into(), String::new()));
        }
        if self.props.editing.as_ref().is_some_and(|(k, _)| k == "tag:new") {
            let (_, v, _) = Self::cells(p, "tag:new");
            if let Some(t) = self.text_cell(p, "tag:new", v, "", false, false) {
                if !t.trim().is_empty() {
                    edits.push(Edit::AddTag(t.trim().to_string()));
                }
            }
        }
        if tags.is_empty() {
            let (r, _, _) = Self::cells(p, "tags:none");
            Self::name_text(p, r, -0.5, "No Tags have been added yet", READ_ONLY.gamma_multiply(1.6), None);
        }
        for t in tags {
            let (r, v, resp) = Self::cells(p, &format!("tag:{t}"));
            Self::name_text(p, r, 0.0, &t, TEXT, None);
            let x = Rect::from_center_size(Pos2::new(v.max.x - 8.0, v.center().y), Vec2::splat(12.0));
            crate::ti::paint(p.ui.painter(), x, "x", TEXT);
            if resp.clicked() && resp.interact_pointer_pos().is_some_and(|q| x.expand(3.0).contains(q)) {
                edits.push(Edit::RemoveTag(t));
            }
        }
    }

    fn attributes_section(&mut self, p: &mut Painter, first: Ref) {
        let mut plus = false;
        self.band(p, "Attributes", false, Some(&mut plus));
        let attrs: Vec<(String, String)> = match self.doc.prop(first, "Attributes") {
            Some(Variant::Attributes(a)) => {
                let mut v: Vec<(String, String)> = a
                    .iter()
                    .filter(|(k, _)| !k.starts_with("RBX_"))
                    .map(|(k, v)| (k.clone(), value_text(&self.doc, v)))
                    .collect();
                v.sort();
                v
            }
            _ => Vec::new(),
        };
        if attrs.is_empty() {
            let (r, _, _) = Self::cells(p, "attrs:none");
            Self::name_text(p, r, -0.5, "No Attribute has been added yet", READ_ONLY.gamma_multiply(1.6), None);
        }
        for (k, v) in attrs {
            let (r, vr, _) = Self::cells(p, &format!("attr:{k}"));
            Self::name_text(p, r, 0.0, &k, TEXT, None);
            Self::value_label(p, vr, &v, TEXT);
        }
    }

    fn apply_prop_edits(&mut self, sel: &[Ref], first: Ref, edits: Vec<Edit>) {
        for e in edits {
            match e {
                Edit::Prop(name, v) if name == "Name" => {
                    if let Variant::String(n) = v {
                        let n = n.trim().to_string();
                        if !n.is_empty() {
                            for r in sel {
                                self.doc.rename(*r, &n);
                            }
                        }
                    }
                }
                // A part's BrickColor sets its Color (the colour it is drawn with).
                Edit::Prop(name, Variant::BrickColor(b)) if name == "BrickColor" && self.doc.class_of(first).is_some_and(|c| doc::is_a(&c, "BasePart")) => {
                    let rgb = crate::scene::brick_rgb(b);
                    let changes = sel
                        .iter()
                        .copied()
                        .filter(|r| self.doc.class_of(*r).is_some_and(|c| doc::is_a(&c, "BasePart")))
                        .map(|r| (r, ustr("Color"), Variant::Color3uint8(Color3uint8::new(rgb[0], rgb[1], rgb[2]))))
                        .collect();
                    self.doc.set_props("Set BrickColor", changes, None);
                }
                // A script's Enabled box sets Enabled and clears an old Disabled.
                Edit::Prop(name, Variant::Bool(on)) if name == "Enabled" && self.doc.class_of(first).is_some_and(|c| doc::is_a(&c, "BaseScript")) => {
                    let changes = sel
                        .iter()
                        .copied()
                        .filter(|r| self.doc.class_of(*r).is_some_and(|c| doc::is_a(&c, "BaseScript")))
                        .flat_map(|r| [(r, ustr("Enabled"), Variant::Bool(on)), (r, ustr("Disabled"), Variant::Bool(!on))])
                        .collect();
                    self.doc.set_props("Set Enabled", changes, None);
                }
                // Ticking CustomPhysicalProperties starts from Plastic's values
                // (density 0.7, friction 0.3, elasticity 0.5, weights 1).
                Edit::Prop(name, Variant::Bool(on)) if name == "CustomPhysicalProperties" => {
                    let v = Variant::PhysicalProperties(if on {
                        PhysicalProperties::Custom(CustomPhysicalProperties::new(0.7, 0.3, 0.5, 1.0, 1.0, 1.0))
                    } else {
                        PhysicalProperties::Default
                    });
                    let changes = sel
                        .iter()
                        .copied()
                        .filter(|r| self.doc.class_of(*r).is_some_and(|c| doc::is_a(&c, "BasePart")))
                        .map(|r| (r, ustr("CustomPhysicalProperties"), v.clone()))
                        .collect();
                    self.doc.set_props("Set CustomPhysicalProperties", changes, None);
                }
                Edit::Prop(name, v) => {
                    let targets: Vec<Ref> = sel
                        .iter()
                        .copied()
                        .filter(|r| self.doc.class_of(*r).is_some_and(|c| has_prop(&c, &name)))
                        .collect();
                    let changes = targets.iter().map(|r| (*r, ustr(&name), v.clone())).collect();
                    self.doc.set_props(&format!("Set {name}"), changes, Some(format!("prop:{name}:{targets:?}")));
                }
                Edit::Frame(f, pos, ori) => self.set_frame(first, f, pos, ori),
                Edit::AddTag(t) => self.edit_tags(sel, |tags| {
                    if !tags.contains(&t) {
                        tags.push(t.clone());
                    }
                }),
                Edit::RemoveTag(t) => self.edit_tags(sel, |tags| tags.retain(|x| *x != t)),
            }
        }
    }

    fn edit_tags(&mut self, sel: &[Ref], f: impl Fn(&mut Vec<String>)) {
        let changes = sel
            .iter()
            .map(|r| {
                let mut tags: Vec<String> = match self.doc.prop(*r, "Tags") {
                    Some(Variant::Tags(t)) => t.iter().map(|s| s.to_string()).collect(),
                    _ => Vec::new(),
                };
                f(&mut tags);
                (*r, ustr("Tags"), Variant::Tags(Tags::from(tags)))
            })
            .collect();
        self.doc.set_props("Set Tags", changes, None);
    }

    /// Writes a Transform / Pivot row: CFrame directly; Origin by moving
    /// the part (or model) so its pivot lands there; the pivot offsets alone.
    fn set_frame(&mut self, r: Ref, f: Frame, pos: Option<Vec3>, ori: Option<Vec3>) {
        let Some(cur) = self.frame_value(r, f) else {
            return;
        };
        let new = Xform::from_orientation_deg(pos.unwrap_or(cur.pos), ori.unwrap_or_else(|| cur.orientation_deg()));
        let cf = |x: &Xform| Variant::CFrame(x.to_cframe());
        match f {
            Frame::PartCFrame => self.doc.set_props("Set CFrame", vec![(r, ustr("CFrame"), cf(&new))], None),
            Frame::PivotOffset => self.doc.set_props("Set PivotOffset", vec![(r, ustr("PivotOffset"), cf(&new))], None),
            Frame::PartOrigin => {
                let offset = cf_of(self.doc.prop(r, "PivotOffset")).unwrap_or(Xform { pos: Vec3::ZERO, rot: glam::Mat3::IDENTITY });
                let part = mul(&new, &offset.inverse());
                self.doc.set_props("Set Origin", vec![(r, ustr("CFrame"), cf(&part))], None);
            }
            Frame::WorldPivot => self.doc.set_props(
                "Set WorldPivot",
                vec![(r, ustr("WorldPivotData"), Variant::OptionalCFrame(Some(new.to_cframe())))],
                None,
            ),
            Frame::ModelOrigin => {
                // Move every part of the model by new × old⁻¹.
                let delta = mul(&new, &cur.inverse());
                let mut changes: Vec<(Ref, rbx_dom_weak::Ustr, Variant)> = self
                    .doc
                    .dom
                    .descendants_of(r)
                    .map(|i| i.referent())
                    .filter_map(|p| cf_of(self.doc.prop(p, "CFrame")).filter(|_| self.scene.get(p).is_some()).map(|c| (p, ustr("CFrame"), cf(&mul(&delta, &c)))))
                    .collect();
                changes.push((r, ustr("WorldPivotData"), Variant::OptionalCFrame(Some(new.to_cframe()))));
                self.doc.set_props("Set Origin", changes, None);
            }
        }
    }
}

/// A property with no stored value and no default in the reflection
/// database (IgnoreGuiInset, AbsoluteSize…): the empty value of its type.
fn typed_empty(class: &str, prop: &str) -> Option<Variant> {
    use rbx_dom_weak::types::VariantType as T;
    let db = doc::db();
    let desc = db.classes.get(class)?;
    let ty = db.superclasses_iter(desc).find_map(|c| match &c.properties.get(prop)?.data_type {
        rbx_reflection::DataType::Value(t) => Some(*t),
        _ => None,
    })?;
    Some(match ty {
        T::Bool => Variant::Bool(false),
        T::Float32 => Variant::Float32(0.0),
        T::Float64 => Variant::Float64(0.0),
        T::Int32 => Variant::Int32(0),
        T::Int64 => Variant::Int64(0),
        T::String => Variant::String(String::new()),
        T::Vector2 => Variant::Vector2(Vector2::new(0.0, 0.0)),
        T::Vector3 => Variant::Vector3(rbx_dom_weak::types::Vector3::new(0.0, 0.0, 0.0)),
        T::Ref => Variant::Ref(Ref::none()),
        _ => return None,
    })
}

/// The enum type of `class.prop` from the reflection database.
fn enum_type_of(class: &str, prop: &str) -> Option<String> {
    let db = doc::db();
    let desc = db.classes.get(class)?;
    db.superclasses_iter(desc).find_map(|c| match &c.properties.get(prop)?.data_type {
        rbx_reflection::DataType::Enum(e) => Some(e.to_string()),
        _ => None,
    })
}

/// Parses typed text back into a value of the same kind.
fn parse_like(old: &Variant, t: &str) -> Option<Variant> {
    let t = t.trim();
    Some(match old {
        Variant::String(_) => Variant::String(t.to_string()),
        Variant::Int32(_) => Variant::Int32(t.parse::<f64>().ok()?.round() as i32),
        Variant::Int64(_) => Variant::Int64(t.parse::<f64>().ok()?.round() as i64),
        Variant::Float32(_) => Variant::Float32(t.parse().ok()?),
        Variant::Float64(_) => Variant::Float64(t.parse().ok()?),
        Variant::Vector2(_) => {
            let p = parse_nums(t, 2)?;
            Variant::Vector2(Vector2::new(p[0], p[1]))
        }
        Variant::ContentId(_) => Variant::ContentId(t.to_string().into()),
        Variant::Content(_) => Variant::Content(rbx_dom_weak::types::Content::from(t.to_string())),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn single_number_fills_vector() {
        assert_eq!(super::parse_nums("0", 3), Some(vec![0.0; 3]));
        assert_eq!(super::parse_nums(" 2.5 ", 2), Some(vec![2.5; 2]));
        assert_eq!(super::parse_nums("1, 2, 3", 3), Some(vec![1.0, 2.0, 3.0]));
        assert_eq!(super::parse_nums("1, 2", 3), None);
        assert_eq!(super::parse_nums("1, x, 3", 3), None);
    }

    #[test]
    fn numbers_like_studio() {
        assert_eq!(super::fmt_num(0.5), "0.5");
        assert_eq!(super::fmt_num(41.7330001), "41.733");
        assert_eq!(super::fmt_num(100000.0), "100000");
        assert_eq!(super::fmt_num(-0.0001), "0");
        assert_eq!(super::fmt_v3(glam::Vec3::new(4.0, 1.2, 2.0)), "4, 1.2, 2");
    }
}
