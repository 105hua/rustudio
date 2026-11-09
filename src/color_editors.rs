//! Studio's two colour editors in the Properties panel, as the Studio
//! an offline Studio runs show them (checks/ui/studio/brickcolor-popup.png,
//! color-dialog.png):
//!
//! - BrickColor: the `ColorPickerPopup` (368 × 400): Studio's palette as a
//!   honeycomb of pointy-top hexagons (26 × 29 px, 27.75 px apart, rows
//!   24 px apart, every row half a cell in from the one above), rows of 7, 8,
//!   … 13 … 8, 7 cells holding `BrickColor.palette(0 … 126)` in order, then
//!   a row of 12 greys, then a "More Colors" button. The current colour has a
//!   2-px red ring outside a 2-px yellow one. The hovered colour's name shows
//!   beside the pointer (Studio's names, Studio.App.RobloxCustomWidgets.
//!   BrickColor<n>).
//! - Color3: Qt's `QColorDialog` "Select Color" (522 × 399): 48 basic colours
//!   (Qt's fixed set), Pick Screen Color, 16 custom colours, Add to Custom
//!   Colors, the hue × saturation field (220 × 200, drawn at value 200, hue
//!   falling left to right, saturation falling downwards), the value strip
//!   with its arrow, the current colour's swatch, Hue / Sat / Val and Red /
//!   Green / Blue spin boxes, the Hex field, OK and Cancel. Studio applies
//!   the colour to the part live while the dialog is open; OK keeps it as
//!   one undo step, Cancel puts the old colour back (measured).

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use rbx_dom_weak::types::BrickColor;

use crate::properties::TEXT;

const BG: Color32 = Color32::from_rgb(25, 26, 31);
const BUTTON: Color32 = Color32::from_rgb(53, 55, 65);
const FIELD: Color32 = Color32::from_rgb(40, 41, 49);
const EDGE_DARK: Color32 = Color32::from_rgb(32, 34, 39);
const EDGE_LIGHT: Color32 = Color32::from_rgb(53, 55, 65);
const CELL_FRAME: Color32 = Color32::from_rgb(38, 39, 46);
const UI_FONT: f32 = 12.5;

/// `BrickColor.palette(0 … 127)`, read from Studio.
pub const PALETTE: [u16; 128] = [
    141, 301, 107, 26, 1012, 303, 1011, 304, 28, 1018, 302, 305, 306, 307, 308, 1021, 309, 310, 1019, 135, 102, 23,
    1010, 312, 313, 37, 1022, 1020, 1027, 311, 315, 1023, 1031, 316, 151, 317, 318, 319, 1024, 314, 1013, 1006, 321,
    322, 104, 1008, 119, 323, 324, 325, 320, 11, 1026, 1016, 1032, 1015, 327, 1005, 1009, 29, 328, 1028, 208, 45, 329,
    330, 331, 1004, 21, 332, 333, 24, 334, 226, 1029, 335, 336, 342, 343, 338, 1007, 339, 133, 106, 340, 341, 1001, 1,
    9, 1025, 337, 344, 345, 1014, 105, 346, 347, 348, 349, 1030, 125, 101, 350, 192, 351, 352, 353, 354, 1002, 5, 18,
    217, 355, 356, 153, 357, 358, 359, 360, 38, 361, 362, 199, 194, 363, 364, 365, 1003,
];
/// The grey row under the honeycomb (captured, left to right).
pub const GREY_ROW: [u16; 12] = [1003, 199, 194, 1002, 325, 348, 26, 302, 311, 320, 335, 1001];

pub const POPUP_SIZE: Vec2 = Vec2::new(368.0, 400.0);
const HEX_R: f32 = 14.5;

/// Every cell of the popup: (BrickColor number, centre from the popup's top
/// left). The honeycomb first (palette order), then the grey row.
pub fn popup_cells() -> Vec<(u16, Vec2)> {
    let mut out = Vec::with_capacity(139);
    let mut i = 0;
    for row in 0..13 {
        let k = row.min(12 - row) as f32;
        let n = 7 + row.min(12 - row);
        for c in 0..n {
            out.push((PALETTE[i], Vec2::new(100.5 - 13.875 * k + 27.75 * c as f32, 22.9 + 24.0 * row as f32)));
            i += 1;
        }
    }
    for (c, n) in GREY_ROW.iter().enumerate() {
        out.push((*n, Vec2::new(30.8 + 27.75 * c as f32, 339.5)));
    }
    out
}

fn more_colors_rect(origin: Pos2) -> Rect {
    Rect::from_min_size(origin + Vec2::new(10.0, 365.0), Vec2::new(348.0, 29.0))
}

fn hexagon(c: Pos2, r: f32) -> Vec<Pos2> {
    (0..6)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_3 * i as f32 - std::f32::consts::FRAC_PI_2;
            Pos2::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

fn rgb_of(n: u16) -> [u8; 3] {
    BrickColor::from_number(n).map(crate::scene::brick_rgb).unwrap_or([163, 162, 165])
}

pub struct BrickPopup {
    /// The property it edits ("BrickColor", or a BrickColorValue's "Value").
    pub prop: String,
    pub origin: Pos2,
    pub opened_frame: u64,
    pub hovered: Option<u16>,
}

/// Where the popup is drawn: under its cell, kept on screen.
pub fn popup_origin(ctx: &egui::Context, pop: &BrickPopup) -> Pos2 {
    let screen = ctx.content_rect();
    Pos2::new(
        pop.origin.x.min(screen.max.x - POPUP_SIZE.x).max(screen.min.x),
        pop.origin.y.min(screen.max.y - POPUP_SIZE.y).max(screen.min.y),
    )
}

pub enum BrickPick {
    None,
    Picked(BrickColor),
    MoreColors,
    Close,
}

/// Draws the BrickColor popup; what the user did with it.
pub fn brick_popup_ui(ctx: &egui::Context, pop: &mut BrickPopup, current: Option<BrickColor>, frame: u64) -> BrickPick {
    let mut out = BrickPick::None;
    let origin = popup_origin(ctx, pop);
    let area = egui::Area::new(egui::Id::new("brickcolor_popup"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(POPUP_SIZE, Sense::click());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, BG);
            let hover = resp.hover_pos();
            pop.hovered = None;
            let cur = current.map(|b| b as u16);
            let mut ring_drawn = false;
            for (n, off) in popup_cells() {
                let c = rect.min + off;
                let rgb = rgb_of(n);
                let fill = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                let edge = Color32::from_rgb((rgb[0] as f32 * 0.64) as u8, (rgb[1] as f32 * 0.64) as u8, (rgb[2] as f32 * 0.64) as u8);
                painter.add(egui::Shape::convex_polygon(hexagon(c, HEX_R), fill, Stroke::new(1.5, edge)));
                let inside = hover.is_some_and(|h| (h - c).length() < HEX_R * 0.9);
                if inside {
                    pop.hovered = Some(n);
                }
                if Some(n) == cur && !ring_drawn {
                    ring_drawn = true;
                    painter.add(egui::Shape::closed_line(hexagon(c, HEX_R + 0.5), Stroke::new(2.0, Color32::from_rgb(255, 255, 0))));
                    painter.add(egui::Shape::closed_line(hexagon(c, HEX_R + 2.5), Stroke::new(2.0, Color32::from_rgb(255, 0, 0))));
                } else if inside {
                    painter.add(egui::Shape::closed_line(hexagon(c, HEX_R + 0.5), Stroke::new(2.0, Color32::WHITE)));
                }
            }
            let mr = more_colors_rect(rect.min);
            painter.rect_filled(mr, 2.0, BUTTON);
            painter.text(mr.center(), egui::Align2::CENTER_CENTER, "More Colors", FontId::proportional(UI_FONT), TEXT);
            if resp.clicked() {
                if let Some(n) = pop.hovered {
                    if let Some(b) = BrickColor::from_number(n) {
                        out = BrickPick::Picked(b);
                    }
                } else if resp.interact_pointer_pos().is_some_and(|p| mr.contains(p)) {
                    out = BrickPick::MoreColors;
                }
            }
            if let (Some(n), Some(h)) = (pop.hovered, hover) {
                let name = BrickColor::from_number(n).map(|b| b.to_string()).unwrap_or_default();
                let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("brickcolor_name"));
                let p = ctx.layer_painter(layer);
                let g = p.layout_no_wrap(name, FontId::proportional(UI_FONT), TEXT);
                let r = Rect::from_min_size(h + Vec2::new(14.0, 18.0), g.size() + Vec2::new(10.0, 6.0));
                p.rect_filled(r, 2.0, Color32::from_rgb(40, 41, 49));
                p.rect_stroke(r, 2.0, Stroke::new(1.0, EDGE_DARK), egui::StrokeKind::Inside);
                p.galley(r.min + Vec2::new(5.0, 3.0), g, TEXT);
            }
            rect
        });
    let rect = area.inner;
    let (pressed, esc) = ctx.input(|i| (i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten(), i.key_pressed(egui::Key::Escape)));
    if esc || (frame > pop.opened_frame && pressed.is_some_and(|p| !rect.contains(p))) {
        if matches!(out, BrickPick::None) {
            out = BrickPick::Close;
        }
    }
    out
}

// ------------------------------------------------------------ Select Color

pub const DIALOG_SIZE: Vec2 = Vec2::new(522.0, 399.0);
const TITLE_H: f32 = 26.0;
const FIELD_RECT: (f32, f32, f32, f32) = (248.0, 21.0, 220.0, 200.0);
const STRIP_RECT: (f32, f32, f32, f32) = (486.0, 21.0, 13.0, 200.0);
const SWATCH_RECT: (f32, f32, f32, f32) = (248.0, 244.0, 58.0, 106.0);

fn r(o: Pos2, (x, y, w, h): (f32, f32, f32, f32)) -> Rect {
    Rect::from_min_size(o + Vec2::new(x, y), Vec2::new(w, h))
}

/// Qt's basic colours (QColorDialog's standard set) at row, column.
pub fn basic_color(row: usize, col: usize) -> [u8; 3] {
    let red = if row < 3 { [0, 170] } else { [85, 255] }[col % 2];
    let green = [0, 85, 170, 255][col / 2];
    let blue = [0, 127, 255][row % 3];
    [red, green, blue]
}

/// QColor::getHsv: hue 0–359 (−1 for greys, shown as 0), saturation and value 0–255.
pub fn rgb_to_hsv(c: [u8; 3]) -> (i32, i32, i32) {
    let (r, g, b) = (c[0] as f64, c[1] as f64, c[2] as f64);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max > 0.0 { d * 255.0 / max } else { 0.0 };
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    ((h.round() as i32) % 360, s.round() as i32, v.round() as i32)
}

pub fn hsv_to_rgb(h: i32, s: i32, v: i32) -> [u8; 3] {
    let (h, s, v) = ((h.rem_euclid(360)) as f64, s.clamp(0, 255) as f64 / 255.0, v.clamp(0, 255) as f64 / 255.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

pub struct ColorDialog {
    pub prop: String,
    /// The objects it edits, with their colour before the dialog opened.
    pub originals: Vec<(rbx_dom_weak::types::Ref, rbx_dom_weak::types::Variant)>,
    pub rgb: [u8; 3],
    pub hsv: (i32, i32, i32),
    /// The spin box or hex field being typed into: (index, text).
    edit: Option<(usize, String)>,
    pub picking: bool,
    pick_at: Option<Pos2>,
    texture: Option<egui::TextureHandle>,
    pub opened_frame: u64,
}

pub enum DialogOut {
    None,
    Changed([u8; 3]),
    Ok,
    Cancel,
}

impl ColorDialog {
    pub fn new(prop: &str, originals: Vec<(rbx_dom_weak::types::Ref, rbx_dom_weak::types::Variant)>, rgb: [u8; 3], frame: u64) -> Self {
        ColorDialog {
            prop: prop.to_string(),
            originals,
            rgb,
            hsv: rgb_to_hsv(rgb),
            edit: None,
            picking: false,
            pick_at: None,
            texture: None,
            opened_frame: frame,
        }
    }

    fn set_rgb(&mut self, c: [u8; 3]) {
        self.rgb = c;
        let (h, s, v) = rgb_to_hsv(c);
        // Greys keep the hue the user had (Qt does the same).
        self.hsv = (if s == 0 { self.hsv.0 } else { h }, s, v);
    }

    fn set_hsv(&mut self, h: i32, s: i32, v: i32) {
        self.hsv = (h.rem_euclid(360), s.clamp(0, 255), v.clamp(0, 255));
        self.rgb = hsv_to_rgb(self.hsv.0, self.hsv.1, self.hsv.2);
    }

    /// Qt's field image: hue across, saturation down, value 200.
    fn field_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.texture.is_none() {
            let (w, h) = (FIELD_RECT.2 as usize, FIELD_RECT.3 as usize);
            let mut img = egui::ColorImage::new([w, h], vec![Color32::BLACK; w * h]);
            for y in 0..h {
                for x in 0..w {
                    let hue = 360 - (x as i32) * 360 / (w as i32 - 1);
                    let sat = 255 - (y as i32) * 255 / (h as i32 - 1);
                    let c = hsv_to_rgb(hue, sat, 200);
                    img.pixels[y * w + x] = Color32::from_rgb(c[0], c[1], c[2]);
                }
            }
            self.texture = Some(ctx.load_texture("qt_color_field", img, egui::TextureOptions::NEAREST));
        }
        self.texture.as_ref().map(|t| t.id()).unwrap_or_default()
    }
}

fn button(ui: &mut egui::Ui, rect: Rect, label: &str) -> bool {
    let resp = ui.interact(rect, ui.id().with(("qtbtn", label)), Sense::click());
    let fill = if resp.hovered() { Color32::from_rgb(63, 65, 77) } else { BUTTON };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, FontId::proportional(UI_FONT), TEXT);
    resp.clicked()
}

fn sunken(p: &egui::Painter, rect: Rect) {
    // Qt's sunken frame: dark top / left, light bottom / right.
    p.line_segment([Pos2::new(rect.min.x - 0.5, rect.min.y - 1.0), Pos2::new(rect.min.x - 0.5, rect.max.y)], Stroke::new(1.0, EDGE_DARK));
    p.line_segment([Pos2::new(rect.min.x - 1.0, rect.min.y - 0.5), Pos2::new(rect.max.x, rect.min.y - 0.5)], Stroke::new(1.0, EDGE_DARK));
    p.line_segment([Pos2::new(rect.max.x + 0.5, rect.min.y), Pos2::new(rect.max.x + 0.5, rect.max.y + 1.0)], Stroke::new(1.0, EDGE_LIGHT));
    p.line_segment([Pos2::new(rect.min.x, rect.max.y + 0.5), Pos2::new(rect.max.x + 1.0, rect.max.y + 0.5)], Stroke::new(1.0, EDGE_LIGHT));
}

/// Where the dialog's content (the 522 × 399 Qt client area) starts.
pub fn dialog_origin(ctx: &egui::Context) -> Pos2 {
    let s = ctx.content_rect();
    Pos2::new((s.center().x - DIALOG_SIZE.x * 0.5).round(), (s.center().y - (DIALOG_SIZE.y + TITLE_H) * 0.5).round() + TITLE_H)
}

/// Draws the Select Color dialog (modal); what changed.
pub fn color_dialog_ui(ctx: &egui::Context, d: &mut ColorDialog, custom: &mut [Option<[u8; 3]>; 16], frame: u64) -> DialogOut {
    let mut out = DialogOut::None;
    let o = dialog_origin(ctx);
    let tex = d.field_texture(ctx);
    // A screen pick in flight: the screenshot arrives a frame later.
    if let Some(at) = d.pick_at {
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let ppp = ctx.pixels_per_point();
            let (x, y) = ((at.x * ppp) as usize, (at.y * ppp) as usize);
            if x < img.size[0] && y < img.size[1] {
                let c = img.pixels[y * img.size[0] + x];
                d.set_rgb([c.r(), c.g(), c.b()]);
                out = DialogOut::Changed(d.rgb);
            }
            d.pick_at = None;
        }
    }
    let backdrop = egui::Area::new(egui::Id::new("color_dialog_block"))
        .order(egui::Order::Foreground)
        .fixed_pos(ctx.content_rect().min)
        .show(ctx, |ui| ui.allocate_exact_size(ctx.content_rect().size(), Sense::click()).1);
    if d.picking && backdrop.inner.clicked() {
        if let Some(p) = backdrop.inner.interact_pointer_pos() {
            d.pick_at = Some(p);
            d.picking = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
    }
    egui::Area::new(egui::Id::new("color_dialog"))
        .order(egui::Order::Foreground)
        .fixed_pos(o - Vec2::new(0.0, TITLE_H))
        .show(ctx, |ui| {
            ui.ctx().move_to_top(ui.layer_id());
            let full = Rect::from_min_size(o - Vec2::new(0.0, TITLE_H), DIALOG_SIZE + Vec2::new(0.0, TITLE_H));
            let (_, _) = ui.allocate_exact_size(full.size(), Sense::click());
            let p = ui.painter().clone();
            p.rect_filled(full.expand(1.0), 0.0, EDGE_DARK);
            let title = Rect::from_min_size(full.min, Vec2::new(DIALOG_SIZE.x, TITLE_H));
            p.rect_filled(title, 0.0, Color32::from_rgb(32, 34, 39));
            p.text(title.center(), egui::Align2::CENTER_CENTER, "Select Color", FontId::proportional(UI_FONT), TEXT);
            let close = Rect::from_center_size(Pos2::new(title.max.x - 14.0, title.center().y), Vec2::splat(16.0));
            crate::ti::paint(&p, close, "x", TEXT);
            if ui.interact(close, ui.id().with("close"), Sense::click()).clicked() {
                out = DialogOut::Cancel;
            }
            p.rect_filled(Rect::from_min_size(o, DIALOG_SIZE), 0.0, BG);
            let font = FontId::proportional(UI_FONT);
            p.text(o + Vec2::new(11.0, 18.0), egui::Align2::LEFT_CENTER, "Basic colors", font.clone(), TEXT);
            p.text(o + Vec2::new(11.0, 258.0), egui::Align2::LEFT_CENTER, "Custom colors", font.clone(), TEXT);
            // Basic and custom colour cells (18 × 14 on a 28 × 24 pitch).
            let mut picked: Option<[u8; 3]> = None;
            for row in 0..8 {
                for col in 0..8 {
                    let (y0, c) = if row < 6 { (37.0 + 24.0 * row as f32, Some(basic_color(row, col))) } else { (277.0 + 24.0 * (row - 6) as f32, custom[(row - 6) * 8 + col].or(Some([255, 255, 255]))) };
                    let cell = Rect::from_min_size(o + Vec2::new(16.0 + 28.0 * col as f32, y0), Vec2::new(18.0, 14.0));
                    p.rect_stroke(cell.expand(2.0), 0.0, Stroke::new(1.0, CELL_FRAME), egui::StrokeKind::Inside);
                    let c = c.unwrap_or([255, 255, 255]);
                    p.rect_filled(cell, 0.0, Color32::from_rgb(c[0], c[1], c[2]));
                    if ui.interact(cell.expand(2.0), ui.id().with(("cell", row, col)), Sense::click()).clicked() {
                        picked = Some(c);
                    }
                }
            }
            if let Some(c) = picked {
                d.set_rgb(c);
                out = DialogOut::Changed(d.rgb);
            }
            if button(ui, Rect::from_min_size(o + Vec2::new(11.0, 182.0), Vec2::new(224.0, 25.0)), "Pick Screen Color") {
                d.picking = true;
            }
            if button(ui, Rect::from_min_size(o + Vec2::new(11.0, 326.0), Vec2::new(224.0, 25.0)), "Add to Custom Colors") {
                // Qt fills the next slot column by column (two rows).
                let n = custom.iter().position(|c| c.is_none()).unwrap_or(0);
                let slot = (n % 2) * 8 + n / 2;
                custom[slot.min(15)] = Some(d.rgb);
                if custom.iter().all(|c| c.is_some()) && n == 0 {
                    custom.iter_mut().for_each(|c| *c = None);
                    custom[0] = Some(d.rgb);
                }
            }
            // The hue × saturation field and its crosshair.
            let field = r(o, FIELD_RECT);
            sunken(&p, field);
            p.image(tex, field, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let fresp = ui.interact(field, ui.id().with("field"), Sense::click_and_drag());
            if let Some(q) = fresp.interact_pointer_pos().filter(|_| fresp.is_pointer_button_down_on() || fresp.clicked()) {
                let x = (q.x - field.min.x).clamp(0.0, field.width() - 1.0) as i32;
                let y = (q.y - field.min.y).clamp(0.0, field.height() - 1.0) as i32;
                let hue = 360 - x * 360 / (field.width() as i32 - 1);
                let sat = 255 - y * 255 / (field.height() as i32 - 1);
                d.set_hsv(hue, sat, d.hsv.2);
                out = DialogOut::Changed(d.rgb);
            }
            let cx = field.min.x + ((360 - d.hsv.0) * (field.width() as i32 - 1) / 360) as f32;
            let cy = field.min.y + ((255 - d.hsv.1) * (field.height() as i32 - 1) / 255) as f32;
            let fp = ui.painter_at(field);
            fp.rect_filled(Rect::from_min_size(Pos2::new(cx - 9.0, cy), Vec2::new(20.0, 2.0)), 0.0, Color32::BLACK);
            fp.rect_filled(Rect::from_min_size(Pos2::new(cx, cy - 9.0), Vec2::new(2.0, 20.0)), 0.0, Color32::BLACK);
            // The value strip: this hue and saturation from value 255 to 0.
            let strip = r(o, STRIP_RECT);
            sunken(&p, strip);
            let top = hsv_to_rgb(d.hsv.0, d.hsv.1, 255);
            let mut mesh = egui::Mesh::default();
            let tc = Color32::from_rgb(top[0], top[1], top[2]);
            mesh.colored_vertex(strip.left_top(), tc);
            mesh.colored_vertex(strip.right_top(), tc);
            mesh.colored_vertex(strip.right_bottom(), Color32::BLACK);
            mesh.colored_vertex(strip.left_bottom(), Color32::BLACK);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            p.add(mesh);
            let sresp = ui.interact(strip.expand2(Vec2::new(0.0, 0.0)).with_max_x(strip.max.x + 8.0), ui.id().with("strip"), Sense::click_and_drag());
            if let Some(q) = sresp.interact_pointer_pos().filter(|_| sresp.is_pointer_button_down_on() || sresp.clicked()) {
                let y = (q.y - strip.min.y).clamp(0.0, strip.height() - 1.0) as i32;
                let val = 255 - y * 255 / (strip.height() as i32 - 1);
                d.set_hsv(d.hsv.0, d.hsv.1, val);
                out = DialogOut::Changed(d.rgb);
            }
            let ay = strip.min.y + ((255 - d.hsv.2) * (strip.height() as i32 - 1) / 255) as f32;
            p.add(egui::Shape::convex_polygon(
                vec![Pos2::new(strip.max.x + 1.5, ay), Pos2::new(strip.max.x + 6.5, ay - 5.0), Pos2::new(strip.max.x + 6.5, ay + 5.0)],
                TEXT,
                Stroke::NONE,
            ));
            // The swatch.
            let sw = r(o, SWATCH_RECT);
            sunken(&p, sw);
            p.rect_filled(sw, 0.0, Color32::from_rgb(d.rgb[0], d.rgb[1], d.rgb[2]));
            // Spin boxes and the hex field.
            let spins: [(&str, f32, f32, i32, i32); 6] = [
                ("Hue:", 363.0, 243.0, d.hsv.0, 359),
                ("Sat:", 363.0, 270.0, d.hsv.1, 255),
                ("Val:", 363.0, 297.0, d.hsv.2, 255),
                ("Red:", 462.0, 243.0, d.rgb[0] as i32, 255),
                ("Green:", 462.0, 270.0, d.rgb[1] as i32, 255),
                ("Blue:", 462.0, 297.0, d.rgb[2] as i32, 255),
            ];
            let mut changed: Option<(usize, i32)> = None;
            for (i, (label, x, y, value, max)) in spins.iter().enumerate() {
                let rect = Rect::from_min_size(o + Vec2::new(*x, *y), Vec2::new(43.0, 21.0));
                p.text(Pos2::new(rect.min.x - 5.0, rect.center().y), egui::Align2::RIGHT_CENTER, *label, font.clone(), TEXT);
                if let Some(v) = spin_box(ui, rect, i, *value, *max, &mut d.edit) {
                    changed = Some((i, v));
                }
            }
            if let Some((i, v)) = changed {
                match i {
                    0 => d.set_hsv(v, d.hsv.1, d.hsv.2),
                    1 => d.set_hsv(d.hsv.0, v, d.hsv.2),
                    2 => d.set_hsv(d.hsv.0, d.hsv.1, v),
                    _ => {
                        let mut c = d.rgb;
                        c[i - 3] = v.clamp(0, 255) as u8;
                        d.set_rgb(c);
                    }
                }
                out = DialogOut::Changed(d.rgb);
            }
            let hex = Rect::from_min_size(o + Vec2::new(363.0, 329.0), Vec2::new(93.0, 21.0));
            p.text(Pos2::new(hex.min.x - 5.0, hex.center().y), egui::Align2::RIGHT_CENTER, "Hex:", font.clone(), TEXT);
            p.rect_filled(hex, 0.0, FIELD);
            let shown = format!("#{:02x}{:02x}{:02x}", d.rgb[0], d.rgb[1], d.rgb[2]);
            let mut text = match &d.edit {
                Some((6, t)) => t.clone(),
                _ => shown.clone(),
            };
            let resp = ui.put(hex.shrink2(Vec2::new(4.0, 2.0)), egui::TextEdit::singleline(&mut text).frame(egui::Frame::NONE).font(font.clone()).text_color(TEXT));
            if resp.has_focus() {
                d.edit = Some((6, text.clone()));
            }
            if resp.lost_focus() {
                if let Some(c) = parse_hex(&text) {
                    d.set_rgb(c);
                    out = DialogOut::Changed(d.rgb);
                }
                if matches!(d.edit, Some((6, _))) {
                    d.edit = None;
                }
            }
            if button(ui, Rect::from_min_size(o + Vec2::new(355.0, 363.0), Vec2::new(75.0, 25.0)), "OK") {
                out = DialogOut::Ok;
            }
            if button(ui, Rect::from_min_size(o + Vec2::new(436.0, 363.0), Vec2::new(75.0, 25.0)), "Cancel") {
                out = DialogOut::Cancel;
            }
            if d.picking {
                p.text(o + Vec2::new(123.0, 216.0), egui::Align2::CENTER_CENTER, "Click anywhere to pick", font.clone(), TEXT);
            }
        });
    let (esc, enter) = ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
    if frame > d.opened_frame && d.edit.is_none() {
        if esc {
            out = DialogOut::Cancel;
        } else if enter {
            out = DialogOut::Ok;
        }
    }
    out
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    (s.len() == 6).then_some(())?;
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// A Qt spin box: the number (typed into when clicked) and up / down arrows.
fn spin_box(ui: &mut egui::Ui, rect: Rect, idx: usize, value: i32, max: i32, edit: &mut Option<(usize, String)>) -> Option<i32> {
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, FIELD);
    let arrows = Rect::from_min_max(Pos2::new(rect.max.x - 12.0, rect.min.y), rect.max);
    let up = Rect::from_min_max(arrows.min, Pos2::new(arrows.max.x, arrows.center().y));
    let down = Rect::from_min_max(Pos2::new(arrows.min.x, arrows.center().y), arrows.max);
    crate::ti::paint(&p, Rect::from_center_size(up.center() + Vec2::new(0.0, 1.5), Vec2::splat(9.0)), "chevron-up", TEXT);
    crate::ti::paint(&p, Rect::from_center_size(down.center() - Vec2::new(0.0, 1.5), Vec2::splat(9.0)), "chevron-down", TEXT);
    let mut out = None;
    if ui.interact(up, ui.id().with(("spin_up", idx)), Sense::click()).clicked() {
        out = Some(if idx == 0 { (value + 1).rem_euclid(360) } else { (value + 1).min(max) });
    }
    if ui.interact(down, ui.id().with(("spin_down", idx)), Sense::click()).clicked() {
        out = Some(if idx == 0 { (value - 1).rem_euclid(360) } else { (value - 1).max(0) });
    }
    let mut text = match edit {
        Some((i, t)) if *i == idx => t.clone(),
        _ => value.to_string(),
    };
    let field = Rect::from_min_max(rect.min + Vec2::new(4.0, 2.0), Pos2::new(arrows.min.x - 1.0, rect.max.y - 2.0));
    let resp = ui.put(field, egui::TextEdit::singleline(&mut text).frame(egui::Frame::NONE).font(FontId::proportional(UI_FONT)).text_color(TEXT));
    if resp.has_focus() {
        *edit = Some((idx, text.clone()));
        let (wheel, enter) = ui.input(|i| (i.smooth_scroll_delta.y, i.key_pressed(egui::Key::Enter)));
        if wheel != 0.0 {
            out = Some((value + wheel.signum() as i32).clamp(0, max));
        }
        if enter {
            out = text.trim().parse::<i32>().ok().map(|v| v.clamp(0, max));
        }
    }
    if resp.lost_focus() {
        if out.is_none() {
            out = text.trim().parse::<i32>().ok().map(|v| v.clamp(0, max));
        }
        if matches!(edit, Some((i, _)) if *i == idx) {
            *edit = None;
        }
    }
    out.filter(|v| *v != value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Studio's palette fills the honeycomb in order; Qt's basic colours and
    /// HSV match the captured dialog (grey 163, 162, 165 is hue 260, sat 5).
    #[test]
    fn studio_layouts() {
        let cells = popup_cells();
        assert_eq!(cells.len(), 139);
        assert_eq!(cells[0].0, 141);
        assert!((cells[57].1.x - 17.0).abs() < 0.5 && (cells[57].1.y - 166.9).abs() < 0.5);
        assert_eq!(cells[126].0, 365);
        assert_eq!(basic_color(0, 1), [170, 0, 0]);
        assert_eq!(basic_color(4, 3), [255, 85, 127]);
        assert_eq!(rgb_to_hsv([163, 162, 165]), (260, 5, 165));
        assert_eq!(hsv_to_rgb(0, 255, 255), [255, 0, 0]);
        assert_eq!(parse_hex("#a3a2a5"), Some([163, 162, 165]));
    }
}
