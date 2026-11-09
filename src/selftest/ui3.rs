//! Self-test steps for round 18's Properties editors (phases 111–113):
//!
//! - `brickcolor-palette`: clicking a part's BrickColor opens Studio's
//!   honeycomb palette (BrickColor.palette order); hovering a cell names it;
//!   clicking Bright red sets it, in one undo step;
//! - `color-dialog`: the Color swatch opens the Select Color dialog
//!   (522 × 399); a click in its field recolours the part live; OK keeps it
//!   as one undo step;
//! - `property-slider`: hovering Transparency shows Studio's slider; dragging
//!   it sets the value live on 0.05 steps, one undo step; typing still works.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Vec2};
use rbx_dom_weak::types::{CFrame, Matrix3, Ref, Variant, Vector3};
use rbx_dom_weak::InstanceBuilder;

use super::{SelfTest, Step};
use crate::app::Editor;

pub(super) struct Ui3Test {
    started: u64,
    part: Ref,
    undo_before: usize,
    notes: Vec<String>,
    ok: bool,
    at: Pos2,
}

impl Default for Ui3Test {
    fn default() -> Self {
        Ui3Test { started: 0, part: Ref::none(), undo_before: 0, notes: Vec::new(), ok: false, at: Pos2::ZERO }
    }
}

fn button(q: &mut Vec<Event>, p: Pos2, down: bool) {
    q.push(Event::PointerMoved(p));
    q.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
}

fn click(q: &mut Vec<Event>, p: Pos2) {
    button(q, p, true);
    button(q, p, false);
}

fn rgb_of(ed: &Editor, r: Ref) -> [u8; 3] {
    match ed.doc.prop(r, "Color") {
        Some(Variant::Color3uint8(c)) => [c.r, c.g, c.b],
        _ => [0, 0, 0],
    }
}

fn transparency(ed: &Editor, r: Ref) -> f64 {
    match ed.doc.prop(r, "Transparency") {
        Some(Variant::Float32(f)) => f as f64,
        _ => -1.0,
    }
}

impl SelfTest {
    pub(super) fn tick_ui3(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.ui3.started);
        match (self.r4.phase, t) {
            // ------------------------------------------------ BrickColor palette
            (111, _) if self.ui3.started == 0 => {
                if ed.play.is_some() {
                    ed.stop_play();
                }
                ed.new_place();
                let ws = ed.doc.default_parent();
                let part = ed.doc.insert(
                    ws,
                    InstanceBuilder::new("Part")
                        .with_name("EditMe")
                        .with_property("Anchored", true)
                        .with_property("CFrame", Variant::CFrame(CFrame::new(Vector3::new(0.0, 6.0, -10.0), Matrix3::identity()))),
                );
                ed.select(vec![part]);
                self.ui3 = Ui3Test { started: f, part, ..Default::default() };
            }
            (111, 4) => match ed.ui_rects.get("props.brickcolor") {
                Some(v) => click(&mut self.queue, v.center()),
                None => self.ui3.notes.push("no BrickColor row drawn".into()),
            },
            (111, 7) => {
                let open = ed.props.brick_popup.is_some();
                self.ui3.notes.push(format!("popup open: {open}"));
                let origin = ed.ui_rects.get("brick.popup").map(|r| r.min);
                let cell = crate::color_editors::popup_cells().into_iter().find(|(n, _)| *n == 21);
                match (origin, cell) {
                    (Some(o), Some((_, off))) => {
                        self.ui3.at = o + off;
                        self.queue.push(Event::PointerMoved(self.ui3.at));
                    }
                    _ => self.ui3.notes.push("no popup rect".into()),
                }
                self.ui3.undo_before = ed.doc.undo_len();
            }
            (111, 9) => {
                let hovered = ed.props.brick_popup.as_ref().and_then(|p| p.hovered);
                self.ui3.ok = hovered == Some(21);
                self.ui3.notes.push(format!("hovering the cell of Bright red names {:?}", hovered.and_then(rbx_dom_weak::types::BrickColor::from_number).map(|b| b.to_string())));
                click(&mut self.queue, self.ui3.at);
            }
            (111, 12) => {
                let c = rgb_of(ed, self.ui3.part);
                let closed = ed.props.brick_popup.is_none();
                let steps = ed.doc.undo_len() - self.ui3.undo_before;
                ed.undo();
                let back = rgb_of(ed, self.ui3.part);
                let pal = crate::color_editors::popup_cells();
                self.steps.push(Step {
                    step: "brickcolor-palette",
                    passed: self.ui3.ok && c == [196, 40, 28] && closed && steps == 1 && back == [163, 162, 165] && pal.len() == 139,
                    detail: format!(
                        "{}; clicked it: Color {c:?} (Bright red 196, 40, 28), popup closed: {closed}, undo steps {steps}, undo → {back:?}; {} cells (Studio: rows of 7…13…7 = BrickColor.palette 0–126, then 12 greys)",
                        self.ui3.notes.join("; "),
                        pal.len()
                    ),
                });
                self.ui3.notes.clear();
                self.r4.phase = 112;
                self.ui3.started = f;
            }
            // -------------------------------------------------- Select Color
            (112, 2) => match ed.ui_rects.get("props.color") {
                Some(v) => click(&mut self.queue, Pos2::new(v.min.x + 8.0, v.center().y)),
                None => self.ui3.notes.push("no Color row drawn".into()),
            },
            (112, 5) => {
                let size = ed.ui_rects.get("color.dialog").map(|r| r.size());
                self.ui3.ok = ed.props.color_dialog.is_some() && size == Some(Vec2::new(522.0, 399.0));
                self.ui3.notes.push(format!("dialog open: {}, size {size:?}", ed.props.color_dialog.is_some()));
                self.ui3.undo_before = ed.doc.undo_len();
                if let Some(d) = ed.ui_rects.get("color.dialog") {
                    // In the hue × saturation field (x 248–468, y 21–221).
                    click(&mut self.queue, d.min + Vec2::new(300.0, 60.0));
                }
            }
            (112, 8) => {
                let live = rgb_of(ed, self.ui3.part);
                self.ui3.notes.push(format!("clicked the field: part {live:?} while the dialog is open"));
                self.ui3.ok &= live != [163, 162, 165] && ed.props.color_dialog.is_some();
                if let Some(d) = ed.ui_rects.get("color.dialog") {
                    click(&mut self.queue, d.min + Vec2::new(392.0, 375.0));
                }
            }
            (112, 11) => {
                let kept = rgb_of(ed, self.ui3.part);
                let steps = ed.doc.undo_len() - self.ui3.undo_before;
                ed.undo();
                let back = rgb_of(ed, self.ui3.part);
                self.steps.push(Step {
                    step: "color-dialog",
                    passed: self.ui3.ok && ed.props.color_dialog.is_none() && steps == 1 && back == [163, 162, 165] && kept != back,
                    detail: format!("{}; OK kept {kept:?}, undo steps {steps}, one undo → {back:?} (Studio: live while open, OK is one undo step)", self.ui3.notes.join("; ")),
                });
                self.ui3.notes.clear();
                self.r4.phase = 113;
                self.ui3.started = f;
            }
            // ------------------------------------------------------- slider
            (113, 2) => match ed.props.row_rects.get("Transparency") {
                Some(r) => self.queue.push(Event::PointerMoved(Pos2::new(r.max.x - 30.0, r.center().y))),
                None => self.ui3.notes.push("no Transparency row".into()),
            },
            (113, 4) => {
                let row = ed.props.row_rects.get("Transparency").copied();
                let slider = ed.ui_rects.get("props.slider").copied();
                let shown = matches!((row, slider), (Some(r), Some(s)) if s.center().y > r.min.y && s.center().y < r.max.y);
                self.ui3.ok = shown;
                self.ui3.notes.push(format!("hover shows the slider: {shown}"));
                self.ui3.undo_before = ed.doc.undo_len();
                if let Some(s) = slider {
                    let (a, _) = crate::prop_slider::track(s);
                    self.ui3.at = Pos2::new(s.max.x - 4.5, s.center().y);
                    let start = Pos2::new(a, s.center().y);
                    button(&mut self.queue, start, true);
                }
            }
            (113, 5..=8) => {
                if let Some(s) = ed.ui_rects.get("props.slider").copied() {
                    let (a, b) = crate::prop_slider::track(s);
                    let k = (t - 4) as f32 / 4.0 * 0.55;
                    self.queue.push(Event::PointerMoved(Pos2::new(a + (b - a) * k, s.center().y)));
                }
            }
            (113, 9) => {
                let live = transparency(ed, self.ui3.part);
                self.ui3.notes.push(format!("while dragging to 55 %: Transparency {live:.3}"));
                self.ui3.ok &= live > 0.4 && live < 0.7 && ((live * 20.0) - (live * 20.0).round()).abs() < 1e-4;
                if let Some(s) = ed.ui_rects.get("props.slider").copied() {
                    let (a, b) = crate::prop_slider::track(s);
                    button(&mut self.queue, Pos2::new(a + (b - a) * 0.55, s.center().y), false);
                }
            }
            (113, 11) => {
                let v = transparency(ed, self.ui3.part);
                let steps = ed.doc.undo_len() - self.ui3.undo_before;
                ed.undo();
                let back = transparency(ed, self.ui3.part);
                self.ui3.notes.push(format!("released at {v:.3}, undo steps {steps}, undo → {back}"));
                self.ui3.ok &= steps == 1 && back == 0.0;
                // Typing still works: click the number, type, Enter.
                if let Some(sp) = ed.ui_rects.get("props.spin").copied() {
                    click(&mut self.queue, Pos2::new(sp.min.x + 15.0, sp.center().y));
                }
            }
            (113, 13) => {
                self.queue.push(Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
                self.queue.push(Event::Text("0.3".into()));
                self.queue.push(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            }
            (113, 16) => {
                let typed = transparency(ed, self.ui3.part);
                let n = crate::prop_slider::ranges().len();
                let spec = crate::prop_slider::spec_for("Part", "Transparency");
                self.steps.push(Step {
                    step: "property-slider",
                    passed: self.ui3.ok && (typed - 0.3).abs() < 1e-4 && n >= 250 && spec.is_some_and(|s| (s.step() - 0.05).abs() < 1e-9),
                    detail: format!(
                        "{}; typed 0.3 → {typed:.3}; {n} properties with Studio's UIMinimum / UIMaximum get a slider (Part.Transparency step {:?})",
                        self.ui3.notes.join("; "),
                        spec.map(|s| s.step())
                    ),
                });
                ed.new_place();
                self.r4.phase = 114;
            }
            _ => {}
        }
        false
    }
}
