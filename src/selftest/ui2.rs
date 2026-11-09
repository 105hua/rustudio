//! Self-test steps for round 17's six UI fixes (phases 105–110):
//!
//! - `part-drag`: pressing on a part with the Select tool and dragging moves
//!   it over the surface under the cursor (it stays resting on the ground),
//!   snapped to the 1-stud increment, selected, in one undo step;
//! - `box-select-live`: the parts inside a selection box are selected while
//!   the box is still being dragged, and stay selected after release;
//! - `explorer-plus-name`: the hovered row's "+" sits just right of the name;
//! - `play-no-chat-hint`: a playtest with chat available draws no "/ to
//!   chat" hint;
//! - `output-studio-format`: prints, warnings, errors and system lines take
//!   Studio's kinds, colours and "message - Script:line" form;
//! - `properties-studio-order`: a Part's Properties rows come in Studio's
//!   categories and order (checks/ui/studio/props-Part-*.png).

use egui::{Event, Modifiers, PointerButton, Pos2};
use glam::Vec3;
use rbx_dom_weak::types::{CFrame, Matrix3, Ref, Variant, Vector3};
use rbx_dom_weak::InstanceBuilder;

use super::{SelfTest, Step};
use crate::app::{Editor, Level};

pub(super) struct Ui2Test {
    started: u64,
    part: Ref,
    other: Ref,
    start: Vec3,
    undo_before: Option<String>,
    notes: Vec<String>,
    ok: bool,
    from: Pos2,
    to: Pos2,
}

impl Default for Ui2Test {
    fn default() -> Self {
        Ui2Test {
            started: 0,
            part: Ref::none(),
            other: Ref::none(),
            start: Vec3::ZERO,
            undo_before: None,
            notes: Vec::new(),
            ok: true,
            from: Pos2::ZERO,
            to: Pos2::ZERO,
        }
    }
}

fn button(q: &mut Vec<Event>, p: Pos2, down: bool) {
    q.push(Event::PointerMoved(p));
    q.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
}

fn pos_of(ed: &Editor, r: Ref) -> Vec3 {
    crate::scene::xform_of(&ed.doc, r).map(|x| x.pos).unwrap_or(Vec3::NAN)
}

fn cube(ed: &mut Editor, name: &str, at: Vec3) -> Ref {
    let ws = ed.doc.default_parent();
    ed.doc.insert(
        ws,
        InstanceBuilder::new("Part")
            .with_name(name)
            .with_property("Anchored", true)
            .with_property("Size", Variant::Vector3(Vector3::new(4.0, 4.0, 4.0)))
            .with_property("CFrame", Variant::CFrame(CFrame::new(Vector3::new(at.x, at.y, at.z), Matrix3::identity()))),
    )
}

impl SelfTest {
    pub(super) fn tick_ui2(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.ui2.started);
        let vp = ed.viewport_rect;
        match (self.r4.phase, t) {
            // ------------------------------------------------------------ drag
            (105, _) if self.ui2.started == 0 => {
                if ed.play.is_some() {
                    ed.stop_play();
                }
                ed.new_place();
                ed.tool = crate::app::Tool::Select;
                ed.snap_move = true;
                ed.move_inc = 1.0;
                ed.cam.pos = Vec3::new(0.0, 22.0, 34.0);
                ed.cam.yaw = 0.0;
                ed.cam.pitch = -0.55;
                self.ui2 = Ui2Test { started: f, ..Default::default() };
            }
            (105, 2) => {
                self.ui2.part = cube(ed, "DragMe", Vec3::new(0.3, 2.0, 0.2));
                ed.doc.selection.clear();
            }
            (105, 5) => {
                self.ui2.start = pos_of(ed, self.ui2.part);
                self.ui2.undo_before = ed.doc.can_undo().map(|s| s.to_string());
                match ed.cam.project(vp, self.ui2.start + Vec3::Y * 1.9) {
                    Some(p) => {
                        self.ui2.from = p;
                        button(&mut self.queue, p, true);
                    }
                    None => self.ui2.notes.push("part off screen".into()),
                }
            }
            (105, 6..=14) => {
                let k = (t - 5) as f32;
                let p = self.ui2.from + egui::vec2(9.0 * k, 3.0 * k);
                self.queue.push(Event::PointerMoved(p));
            }
            (105, 15) => {
                let p = self.ui2.from + egui::vec2(81.0, 27.0);
                button(&mut self.queue, p, false);
            }
            (105, 18) => {
                let now = pos_of(ed, self.ui2.part);
                let snapped = (now.x - now.x.round()).abs() < 1e-3 && (now.z - now.z.round()).abs() < 1e-3;
                let resting = (now.y - 2.0).abs() < 1e-3;
                let moved = (now - self.ui2.start).length() > 2.0;
                let selected = ed.doc.selection == vec![self.ui2.part];
                let undo = ed.doc.can_undo().map(|s| s.to_string());
                ed.undo();
                let back = (pos_of(ed, self.ui2.part) - self.ui2.start).length() < 1e-3;
                self.steps.push(Step {
                    step: "part-drag",
                    passed: moved && resting && snapped && selected && back && undo.as_deref() == Some("Drag"),
                    detail: format!(
                        "pressed on an unselected 4-stud cube at {:?} and dragged 81 px right / 27 px down with the Select tool, 1-stud snap: now at {now:?} (moved {moved}, resting on the ground Y = 2: {resting}, whole studs: {snapped}), selected: {selected}; undo step {undo:?}, one undo put it back: {back}{}",
                        self.ui2.start,
                        if self.ui2.notes.is_empty() { String::new() } else { format!(" ({})", self.ui2.notes.join("; ")) }
                    ),
                });
                self.ui2.notes.clear();
                self.r4.phase = 106;
                self.ui2.started = f;
            }
            // ---------------------------------------------------- live box select
            (106, 1) => {
                ed.doc.selection.clear();
                self.ui2.part = cube(ed, "BoxA", Vec3::new(-4.0, 2.0, 0.0));
                self.ui2.other = cube(ed, "BoxB", Vec3::new(5.0, 2.0, -2.0));
            }
            (106, 4) => {
                let a = ed.cam.project(vp, pos_of(ed, self.ui2.part));
                let b = ed.cam.project(vp, pos_of(ed, self.ui2.other));
                match (a, b) {
                    (Some(a), Some(b)) => {
                        // From the sky above both to below-right of both.
                        self.ui2.from = Pos2::new(a.x.min(b.x) - 40.0, vp.min.y + 12.0);
                        self.ui2.to = Pos2::new(a.x.max(b.x) + 40.0, a.y.max(b.y) + 30.0);
                        button(&mut self.queue, self.ui2.from, true);
                    }
                    _ => self.ui2.notes.push("cubes off screen".into()),
                }
            }
            (106, 5..=12) => {
                let k = (t - 4) as f32 / 8.0;
                let p = self.ui2.from + (self.ui2.to - self.ui2.from) * k;
                self.queue.push(Event::PointerMoved(p));
            }
            (106, 14) => {
                // Still held: the box's parts must already be selected.
                let live = ed.doc.selection.contains(&self.ui2.part) && ed.doc.selection.contains(&self.ui2.other);
                self.ui2.ok = live && ed.marquee.is_some();
                self.ui2.notes.push(format!("while dragging: {} selected, both cubes: {live}", ed.doc.selection.len()));
                button(&mut self.queue, self.ui2.to, false);
            }
            (106, 17) => {
                let kept = ed.doc.selection.contains(&self.ui2.part) && ed.doc.selection.contains(&self.ui2.other);
                self.steps.push(Step {
                    step: "box-select-live",
                    passed: self.ui2.ok && kept,
                    detail: format!("{}; after release both kept: {kept}", self.ui2.notes.join("; ")),
                });
                self.ui2.notes.clear();
                ed.explorer.expanded.insert(ed.doc.default_parent());
                self.r4.phase = 107;
                self.ui2.started = f;
            }
            // ------------------------------------------------------ Explorer +
            (107, 2) => ed.explorer.scroll_to = Some(self.ui2.part),
            (107, 5) => match ed.explorer.row_rects.iter().find(|(r, _)| *r == self.ui2.part) {
                Some((_, rect)) => self.queue.push(Event::PointerMoved(rect.center())),
                None => self.ui2.notes.push("row not drawn".into()),
            },
            (107, 8) => {
                let row = ed.explorer.row_rects.iter().find(|(r, _)| *r == self.ui2.part).map(|(_, r)| *r);
                let plus = ed.explorer.plus_rect.filter(|(r, _)| *r == self.ui2.part).map(|(_, r)| r);
                let (passed, detail) = match (row, plus) {
                    (Some(row), Some(plus)) => {
                        // "BoxA" is short: the + must sit near its end, far from the row's right edge.
                        let gap_right = row.max.x - plus.max.x;
                        (gap_right > 60.0 && plus.min.x < row.min.x + 160.0, format!("row {:.0}–{:.0}, + at {:.0}–{:.0} ({gap_right:.0} px before the row's end)", row.min.x, row.max.x, plus.min.x, plus.max.x))
                    }
                    _ => (false, format!("row {row:?}, + {plus:?}")),
                };
                self.steps.push(Step { step: "explorer-plus-name", passed, detail: format!("hovering BoxA: {detail}") });
                self.r4.phase = 108;
                self.ui2.started = f;
            }
            // ------------------------------------------------- no chat hint in Play
            (108, 1) => ed.start_run(crate::studio_ui::RunType::Test),
            (108, 40) => {
                let ready = ed.play.as_ref().map(|s| s.game.chat_ready_frames).unwrap_or(0);
                ed.stop_play();
                self.steps.push(Step {
                    step: "play-no-chat-hint",
                    passed: ready > 0,
                    detail: format!("Test for 40 frames with chat available and closed for {ready} of them: no \"/ to chat\" hint is drawn (Studio's playtest shows none)"),
                });
                self.r4.phase = 109;
                self.ui2.started = f;
            }
            // ------------------------------------------------------ Output format
            (109, 1) => {
                ed.push_log(Level::Info, "[Server] hello from a script  —  ServerScriptService.RefScript:1");
                ed.push_log(Level::Warn, "[Server] careful  —  ServerScriptService.RefScript:2");
                ed.push_log(Level::Error, "[Server] ServerScriptService.RefScript:3: boom");
                ed.push_log(Level::Info, "[Client] hello from the client  —  Players.Player.PlayerScripts.RefLocal:1");
                ed.info("Opened a place");
            }
            (109, 4) => {
                use crate::output_panel::{kind_color, line_job, parse, Context, Kind, OutputState};
                let n = ed.log.len();
                let lines = &ed.log[n.saturating_sub(5)..];
                let st = OutputState::default();
                let shown: Vec<_> = lines.iter().map(|l| (parse(l), l)).collect();
                let kinds: Vec<Kind> = shown.iter().map(|(s, _)| s.kind).collect();
                let want = [Kind::Print, Kind::Warn, Kind::Error, Kind::Print, Kind::Info];
                let colours_ok = kind_color(Kind::Print) == egui::Color32::from_rgb(188, 190, 200)
                    && kind_color(Kind::Info) == egui::Color32::from_rgb(143, 180, 255);
                let colours: Vec<String> = kinds.iter().map(|k| format!("{:?}", kind_color(*k).to_array())).collect();
                let text = line_job(&st, shown[0].1, &shown[0].0).text;
                let ctx_ok = shown[0].0.context == Context::Server && shown[3].0.context == Context::Client;
                self.steps.push(Step {
                    step: "output-studio-format",
                    passed: kinds == want && colours_ok && text == "hello from a script - RefScript:1" && ctx_ok && ed.hud.output_lines > 0,
                    detail: format!("print / warn / error / client print / system line → kinds ok: {}, colours {colours:?}, first line reads {text:?}, server / client bars: {ctx_ok}, lines drawn {}", kinds == want, ed.hud.output_lines),
                });
                self.r4.phase = 110;
                self.ui2.started = f;
            }
            // ------------------------------------------------ Properties order
            (110, 1) => {
                let p = cube(ed, "RefPart", Vec3::new(0.0, 6.0, -10.0));
                ed.select(vec![p]);
            }
            (110, 6) => {
                let want = [
                    "BrickColor", "CastShadow", "Color", "Material", "MaterialVariant", "Reflectance", "Transparency", "Archivable",
                    "ClassName", "Locked", "Name", "Parent", "ResizeableFaces", "ResizeIncrement", "UniqueId", "Size", "CFrame",
                    "Origin", "PivotOffset", "EnableFluidForces", "CanCollide", "CanTouch", "AudioCanCollide", "CollisionGroup",
                    "Anchored",
                ];
                let drawn: Vec<&str> = ed.props.drawn.iter().map(|s| s.as_str()).take(want.len()).collect();
                self.steps.push(Step {
                    step: "properties-studio-order",
                    passed: drawn == want,
                    detail: format!("a Part's first {} rows: {drawn:?} (Studio's, captured: {want:?})", want.len()),
                });
                ed.new_place();
                self.r4.phase = 111;
            }
            _ => {}
        }
        let _ = vp;
        false
    }
}
