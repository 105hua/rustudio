//! `--self-test <file.json>`: drives the editor with injected keyboard and
//! mouse events (the same events a person produces), then checks the
//! editor's own state after each step.

use std::path::PathBuf;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Vec2};
use glam::Vec3;
use rbx_dom_weak::types::Ref;
use rbx_dom_weak::InstanceBuilder;
use serde_json::json;

use crate::app::{Editor, Tool};
use crate::doc::Document;
use crate::viewport::Handle;

mod movement_r6;
mod account;
mod cloud;
mod anim;
mod avatar;
mod bloom;
mod collision;
mod fidelity;
mod light;
mod material;
mod union_density;
mod mesh_density;
mod packs;
mod player;
mod client_velocity;
mod gui_modal;
mod pause_menu;
mod hud_resize;
mod project;
mod ui;
mod ui2;
mod ui3;
mod terrain_ed;
mod sound;
mod gc;

struct Step {
    step: &'static str,
    passed: bool,
    detail: String,
}

pub struct SelfTest {
    out: PathBuf,
    /// Seconds each step took (shown with the results), and the clock.
    step_secs: Vec<f32>,
    clock: Option<(std::time::Instant, f32)>,
    frame: u64,
    queue: Vec<Event>,
    modifiers: Modifiers,
    steps: Vec<Step>,
    cam_start: Vec3,
    cam_moved: f32,
    part: Option<Ref>,
    part_start: Vec3,
    part_moved: Vec3,
    parts_before: usize,
    drag_from: Pos2,
    drag_dir: Vec2,
    notes: Vec<String>,
    act_detail: String,
    clicked: bool,
    orient_before: f32,
    ring_center: Pos2,
    ring_radius: Vec2,
    turned: f32,
    size_before: Vec3,
    look: LookState,
    reveal: RevealRefs,
    reveal_ok: bool,
    reveal_notes: Vec<String>,
    finished: bool,
    probe: Ref,
    edit_parts: usize,
    log_mark: usize,
    luau_ok: bool,
    luau_notes: Vec<String>,
    char_start: Option<Vec3>,
    walked: f32,
    ground_y: f32,
    max_y: f32,
    play_ok: bool,
    play_notes: Vec<String>,
    cam: CamTest,
    m3: M3Test,
    r4: R4Test,
    r6: R6Test,
    gc: gc::GcTest,
    pm: player::PlayerTest,
    pmenu: pause_menu::PauseMenuTest,
    cvel: client_velocity::ClientVelocityTest,
    gmodal: gui_modal::GuiModalTest,
    hud: hud_resize::HudResizeTest,
    pj: project::ProjectTest,
    ui: ui::UiTest,
    ui2: ui2::Ui2Test,
    ui3: ui3::Ui3Test,
    ted: terrain_ed::TerrainEdTest,
    fid: fidelity::FidTest,
    fid_started: bool,
    cloud: cloud::CloudTest,
}

/// Ctrl as egui-winit reports it on Linux: both `ctrl` and `command` set.
const CTRL: Modifiers = Modifiers {
    alt: false,
    ctrl: true,
    shift: false,
    mac_cmd: false,
    command: true,
};

fn key(k: Key, pressed: bool, m: Modifiers) -> Event {
    Event::Key {
        key: k,
        physical_key: Some(k),
        pressed,
        repeat: false,
        modifiers: m,
    }
}

impl SelfTest {
    pub fn new(out: PathBuf) -> Self {
        // The asset check fills a throwaway cache, never the real one; the
        // content folder is our own generated files; sounds stay silent.
        std::env::set_var(
            "RUSTUDIO_CACHE_DIR",
            std::env::temp_dir().join("rustudio-selftest-cache"),
        );
        std::env::set_var(
            "RUSTUDIO_CONTENT_DIR",
            std::env::temp_dir().join("rustudio-selftest-content"),
        );
        std::env::set_var("RUSTUDIO_NO_AUDIO", "1");
        SelfTest {
            out,
            step_secs: Vec::new(),
            clock: None,
            frame: 0,
            queue: Vec::new(),
            modifiers: Modifiers::NONE,
            steps: Vec::new(),
            cam_start: Vec3::ZERO,
            cam_moved: 0.0,
            part: None,
            part_start: Vec3::ZERO,
            part_moved: Vec3::ZERO,
            parts_before: 0,
            drag_from: Pos2::ZERO,
            drag_dir: Vec2::X,
            notes: Vec::new(),
            act_detail: String::new(),
            clicked: false,
            orient_before: 0.0,
            ring_center: Pos2::ZERO,
            ring_radius: Vec2::ZERO,
            turned: 0.0,
            size_before: Vec3::ZERO,
            look: LookState::default(),
            reveal: RevealRefs::default(),
            reveal_ok: true,
            reveal_notes: Vec::new(),
            finished: false,
            probe: Ref::none(),
            edit_parts: 0,
            log_mark: 0,
            luau_ok: false,
            luau_notes: Vec::new(),
            char_start: None,
            walked: 0.0,
            ground_y: 0.0,
            max_y: 0.0,
            play_ok: false,
            play_notes: Vec::new(),
            cam: CamTest::default(),
            m3: M3Test {
                leg_min: f32::MAX,
                leg_max: f32::MIN,
                ..Default::default()
            },
            r4: R4Test::default(),
            r6: R6Test::default(),
            gc: gc::GcTest::default(),
            pm: player::PlayerTest::default(),
            pmenu: pause_menu::PauseMenuTest::default(),
            cvel: client_velocity::ClientVelocityTest::default(),
            gmodal: gui_modal::GuiModalTest::default(),
            hud: hud_resize::HudResizeTest::default(),
            pj: project::ProjectTest::default(),
            ui: ui::UiTest::default(),
            ui2: ui2::Ui2Test::default(),
            ui3: ui3::Ui3Test::default(),
            ted: Default::default(),
            fid: fidelity::FidTest::default(),
            fid_started: false,
            cloud: cloud::CloudTest::default(),
        }
    }

    pub fn inject(&mut self, raw: &mut RawInput) {
        // Only scripted input: drop real mouse/keyboard events so a hand on
        // the mouse can't disturb the run.
        raw.events.retain(|e| {
            !matches!(
                e,
                Event::PointerMoved(_)
                    | Event::MouseMoved(_)
                    | Event::PointerButton { .. }
                    | Event::PointerGone
                    | Event::Key { .. }
                    | Event::Text(_)
                    | Event::MouseWheel { .. }
                    | Event::ModifiersChanged(_)
                    | Event::Copy
                    | Event::Cut
                    | Event::Paste(_)
            )
        });
        raw.events.extend(self.queue.drain(..));
        // The scripted person has the window focused (the hidden test window
        // never gets focus; game cursor grabs need it, viewport.rs).
        raw.focused = true;
    }

    fn press_combo(&mut self, m: Modifiers, k: Key) {
        self.modifiers = m;
        self.queue.push(key(k, true, m));
        self.queue.push(key(k, false, m));
    }

    fn part_pos(ed: &Editor, r: Ref) -> Option<Vec3> {
        crate::scene::xform_of(&ed.doc, r).map(|x| x.pos)
    }

    /// Advances the script one frame; returns true when finished.
    pub fn tick(&mut self, ed: &mut Editor, _ctx: &egui::Context) -> bool {
        self.frame += 1;
        let (t0, last) = *self.clock.get_or_insert((std::time::Instant::now(), 0.0));
        let now = t0.elapsed().as_secs_f32();
        while self.step_secs.len() < self.steps.len() {
            self.step_secs.push(now - last);
            self.clock = Some((t0, now));
        }
        // RUSTUDIO_SELFTEST_FROM=<phase>: start at a later step (development).
        if self.frame == 1 {
            if let Some(p) = std::env::var("RUSTUDIO_SELFTEST_FROM")
                .ok()
                .and_then(|v| v.parse().ok())
            {
                self.frame = 813;
                self.r4.phase = p;
            }
        }
        // The Place A playthrough runs in its own process alongside every other
        // step (the self-test has 90 s in all).
        if !self.fid_started {
            self.fid_started = true;
            self.fid_spawn();
        }
        let f = self.frame;
        let vp = ed.viewport_rect;
        if f > 2 && self.modifiers != Modifiers::NONE && self.queue.is_empty() {
            self.modifiers = Modifiers::NONE;
        }
        if f == 1 {
            // The older steps watch the server side while playing.
            ed.play_shows_client = false;
        }
        match f {
            // 1. Fly the camera forward with W.
            10 => {
                // Studio opens with snapping off; tick both boxes like a user would.
                ed.snap_move = true;
                ed.snap_rotate = true;
                self.cam_start = ed.cam.pos;
                self.queue.push(Event::PointerMoved(vp.center()));
                self.queue.push(key(Key::W, true, Modifiers::NONE));
            }
            // Studio-speed flying (107 studs/s): 10 frames ≈ 18 studs.
            20 => self.queue.push(key(Key::W, false, Modifiers::NONE)),
            45 => self.cam_moved = (ed.cam.pos - self.cam_start).length(),
            // 2. Click the Part button in the toolbar.
            50 => {
                self.parts_before = ed.doc.count_class("BasePart");
                match ed.ui_rects.get("insert.part") {
                    Some(r) => {
                        let c = r.center();
                        self.queue.push(Event::PointerMoved(c));
                        self.queue.push(Event::PointerButton {
                            pos: c,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        });
                    }
                    None => self.notes.push("Part button not found".into()),
                }
            }
            51 => {
                let c = ed
                    .ui_rects
                    .get("insert.part")
                    .map(|r| r.center())
                    .unwrap_or_default();
                self.queue.push(Event::PointerButton {
                    pos: c,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                });
            }
            55 => {
                let sel = ed.doc.selection.first().copied();
                if ed.doc.count_class("BasePart") == self.parts_before + 1 {
                    self.part = sel;
                    self.part_start = sel.and_then(|r| Self::part_pos(ed, r)).unwrap_or_default();
                } else {
                    self.notes
                        .push("clicking Part did not insert a part".into());
                }
                // 3. Ctrl+2 switches to the Move tool.
                self.press_combo(CTRL, Key::Num2);
            }
            60 => {
                if ed.tool != Tool::Move {
                    self.notes
                        .push("Ctrl+2 did not select the Move tool".into());
                }
                let handle = ed
                    .gizmo_points
                    .iter()
                    .find(|(h, _)| {
                        *h == Handle {
                            tool: Tool::Move,
                            axis: 0,
                            sign: 1.0,
                        }
                    })
                    .map(|(_, p)| *p);
                match (handle, self.part) {
                    (Some(p), Some(_)) => {
                        let a = ed.cam.project(vp, self.part_start);
                        let b = ed.cam.project(vp, self.part_start + Vec3::X * 4.0);
                        self.drag_dir = match (a, b) {
                            (Some(a), Some(b)) => (b - a).normalized(),
                            _ => Vec2::X,
                        };
                        self.drag_from = p;
                        self.queue.push(Event::PointerMoved(p));
                        self.queue.push(Event::PointerButton {
                            pos: p,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        });
                    }
                    _ => self.notes.push("move gizmo handle not visible".into()),
                }
            }
            61..=75 => {
                let p = self.drag_from + self.drag_dir * ((f - 60) as f32 * 10.0);
                self.queue.push(Event::PointerMoved(p));
            }
            76 => {
                let p = self.drag_from + self.drag_dir * 150.0;
                self.queue.push(Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                });
            }
            80 => {
                let now = self
                    .part
                    .and_then(|r| Self::part_pos(ed, r))
                    .unwrap_or(self.part_start);
                self.part_moved = now;
                let d = now - self.part_start;
                let snapped = (d.x / ed.move_inc - (d.x / ed.move_inc).round()).abs() < 1e-3;
                let ok_part = d.x >= 1.0 && d.y.abs() < 1e-3 && d.z.abs() < 1e-3 && snapped;
                let ok_cam = self.cam_moved > 1.0;
                self.steps.push(Step {
                    step: "move",
                    passed: ok_part && ok_cam,
                    detail: format!(
                        "W flew the camera {:.1} studs; dragging the Move tool's X arrow 150 px moved the new part {:+.2} studs on X (snapped to {} studs: {snapped})",
                        self.cam_moved, d.x, ed.move_inc
                    ),
                });
                // 4. Duplicate with Ctrl+D.
                self.parts_before = ed.doc.count_class("BasePart");
                self.press_combo(CTRL, Key::D);
            }
            86 => {
                let n = ed.doc.count_class("BasePart");
                if n != self.parts_before + 1 {
                    self.notes.push(format!(
                        "Ctrl+D: expected {} parts, found {n}",
                        self.parts_before + 1
                    ));
                }
                self.press_combo(CTRL, Key::Z); // undo duplicate
            }
            92 => {
                if ed.doc.count_class("BasePart") != self.parts_before {
                    self.notes.push("Ctrl+Z did not undo the duplicate".into());
                }
                self.press_combo(CTRL, Key::Z); // undo move
            }
            98 => {
                let p = self
                    .part
                    .and_then(|r| Self::part_pos(ed, r))
                    .unwrap_or_default();
                if (p - self.part_start).length() > 1e-3 {
                    self.notes.push("Ctrl+Z did not undo the move".into());
                }
                self.press_combo(CTRL, Key::Y); // redo move
            }
            104 => {
                let p = self
                    .part
                    .and_then(|r| Self::part_pos(ed, r))
                    .unwrap_or_default();
                if (p - self.part_moved).length() > 1e-3 {
                    self.notes.push("Ctrl+Y did not redo the move".into());
                }
                // Save and reload through the real file format.
                let path = std::env::temp_dir().join("rustudio-selftest.rbxl");
                let before = ed.doc.count_class("BasePart");
                let roundtrip = ed
                    .doc
                    .encode_all(false)
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()))
                    .and_then(|_| Document::open(&path))
                    .map(|d| d.count_class("BasePart"));
                match roundtrip {
                    Ok(n) if n == before => {}
                    Ok(n) => self
                        .notes
                        .push(format!("saved {before} parts, reloaded {n}")),
                    Err(e) => self.notes.push(format!("save/load failed: {e}")),
                }
                let _ = std::fs::remove_file(&path);
                self.act_detail = format!(
                    "clicked Part to insert, Ctrl+D duplicated, Ctrl+Z undid the duplicate and the move, Ctrl+Y redid it, the place saved and reloaded as .rbxl with {before} parts"
                );
                // Undoing the duplicate cleared the selection: click the part to select it again.
                match ed.cam.project(vp, self.part_moved) {
                    Some(c) => {
                        self.drag_from = c;
                        self.queue.push(Event::PointerMoved(c));
                        self.queue.push(Event::PointerButton {
                            pos: c,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        });
                    }
                    None => self.notes.push("part is off screen".into()),
                }
            }
            105 => {
                let c = self.drag_from;
                self.queue.push(Event::PointerButton {
                    pos: c,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                });
            }
            108 => {
                self.clicked = self.part.is_some_and(|p| ed.doc.selection == vec![p]);
                self.press_combo(CTRL, Key::Num4); // Rotate tool
            }
            // 5. Drag the Y ring 60 degrees around the gizmo centre.
            112 => {
                let ring = ed
                    .gizmo_points
                    .iter()
                    .find(|(h, _)| {
                        *h == Handle {
                            tool: Tool::Rotate,
                            axis: 1,
                            sign: 1.0,
                        }
                    })
                    .map(|(_, p)| *p);
                let center = ed.cam.project(vp, self.part_moved);
                match (ring, center) {
                    (Some(p), Some(c)) => {
                        self.orient_before = self
                            .part
                            .and_then(|r| crate::scene::xform_of(&ed.doc, r))
                            .map(|x| x.orientation_deg().y)
                            .unwrap_or(0.0);
                        self.ring_center = c;
                        self.ring_radius = p - c;
                        self.queue.push(Event::PointerMoved(p));
                        self.queue.push(Event::PointerButton {
                            pos: p,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        });
                    }
                    _ => self.notes.push("rotate ring not visible".into()),
                }
            }
            113..=122 => {
                let a = ((f - 112) as f32 * 6.0).to_radians();
                let r = self.ring_radius;
                let p = self.ring_center
                    + Vec2::new(r.x * a.cos() - r.y * a.sin(), r.x * a.sin() + r.y * a.cos());
                self.queue.push(Event::PointerMoved(p));
                self.drag_from = p;
            }
            123 => {
                let p = self.drag_from;
                self.queue.push(Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                });
            }
            126 => {
                let after = self
                    .part
                    .and_then(|r| crate::scene::xform_of(&ed.doc, r))
                    .map(|x| x.orientation_deg().y)
                    .unwrap_or(0.0);
                self.turned = ((after - self.orient_before + 540.0) % 360.0) - 180.0;
                let on_step = (self.turned / ed.rotate_inc - (self.turned / ed.rotate_inc).round())
                    .abs()
                    < 1e-2;
                if self.turned.abs() < 1.0 || !on_step {
                    self.notes
                        .push(format!("rotate drag turned the part {:.1}°", self.turned));
                }
                self.size_before = self
                    .part
                    .and_then(|r| crate::scene::vec3_of(ed.doc.prop(r, "Size")))
                    .unwrap_or_default();
                self.press_combo(CTRL, Key::Num3); // Scale tool
            }
            // 6. Drag the +X scale handle 100 px.
            130 => {
                let handle = ed
                    .gizmo_points
                    .iter()
                    .find(|(h, _)| {
                        *h == Handle {
                            tool: Tool::Scale,
                            axis: 0,
                            sign: 1.0,
                        }
                    })
                    .map(|(_, p)| *p);
                let xf = self.part.and_then(|r| crate::scene::xform_of(&ed.doc, r));
                match (handle, xf) {
                    (Some(p), Some(xf)) => {
                        let a = ed.cam.project(vp, xf.pos);
                        let b = ed.cam.project(vp, xf.pos + xf.rot.x_axis * 4.0);
                        self.drag_dir = match (a, b) {
                            (Some(a), Some(b)) => (b - a).normalized(),
                            _ => Vec2::X,
                        };
                        self.drag_from = p;
                        self.queue.push(Event::PointerMoved(p));
                        self.queue.push(Event::PointerButton {
                            pos: p,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers: Modifiers::NONE,
                        });
                    }
                    _ => self.notes.push("scale handle not visible".into()),
                }
            }
            131..=140 => {
                let p = self.drag_from + self.drag_dir * ((f - 130) as f32 * 10.0);
                self.queue.push(Event::PointerMoved(p));
            }
            141 => {
                let p = self.drag_from + self.drag_dir * 100.0;
                self.queue.push(Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                });
            }
            144 => {
                let size = self
                    .part
                    .and_then(|r| crate::scene::vec3_of(ed.doc.prop(r, "Size")))
                    .unwrap_or_default();
                let grow = size.x - self.size_before.x;
                if grow < 1.0 || (grow - grow.round()).abs() > 1e-3 {
                    self.notes
                        .push(format!("scale drag changed Size.X by {grow:.2}"));
                }
                let ok = self.notes.is_empty();
                self.steps.push(Step {
                    step: "act",
                    passed: ok,
                    detail: if ok {
                        format!(
                            "{}; Rotate tool ring drag turned it {:+.0}° ({}° snap); Scale tool handle drag grew Size.X by {grow:.0} studs",
                            self.act_detail, self.turned, ed.rotate_inc
                        )
                    } else {
                        self.notes.join("; ")
                    },
                });
            }
            150 => {
                let clicked = self.clicked;
                let h = &ed.hud;
                let ok = clicked
                    && h.explorer_rows > 0
                    && h.property_rows > 0
                    && h.output_lines > 0
                    && h.viewport_drawn
                    && h.toolbar_buttons >= 5
                    && h.status_drawn;
                self.steps.push(Step {
                    step: "hud",
                    passed: ok,
                    detail: format!(
                        "clicking the part in the viewport selected it: {clicked}; viewport drawn: {}, Explorer rows: {}, Properties rows: {}, Output lines: {}, toolbar buttons: {}, status bar: {}",
                        h.viewport_drawn, h.explorer_rows, h.property_rows, h.output_lines, h.toolbar_buttons, h.status_drawn
                    ),
                });
            }
            151.. => return self.tick_more(f, ed),
            _ => {}
        }
        false
    }

    /// Writes the results file; returns true (the run is over).
    fn finish(&mut self) -> bool {
        if self.finished {
            return true;
        }
        self.finished = true;
        let movement_dir = self.out.with_extension("movement-r6");
        let movement = movement_r6::run(&movement_dir);
        self.steps.push(Step { step: "movement-r6-traces", passed: movement.is_ok(), detail: format!("{}; {}", movement_dir.display(), movement.unwrap_or_else(|e|e)) });
        let body = json!({
            "version": 1,
            "steps": self.steps.iter().map(|s| json!({"step": s.step, "passed": s.passed, "detail": s.detail})).collect::<Vec<_>>(),
        });
        if let Some(dir) = self.out.parent() {
            if !dir.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(dir);
            }
        }
        match std::fs::write(
            &self.out,
            serde_json::to_string_pretty(&body).unwrap_or_default(),
        ) {
            Ok(()) => println!("self-test results written to {}", self.out.display()),
            Err(e) => eprintln!("could not write {}: {e}", self.out.display()),
        }
        for s in &self.steps {
            println!(
                "[{}] {}: {}",
                if s.passed { "PASS" } else { "FAIL" },
                s.step,
                s.detail
            );
        }
        for (s, t) in self.steps.iter().zip(&self.step_secs) {
            println!("  {:>6.1} s  {}", t, s.step);
        }
        true
    }

    fn press(&mut self, p: Pos2, button: PointerButton, m: Modifiers) {
        self.queue.push(Event::PointerMoved(p));
        self.queue.push(Event::PointerButton {
            pos: p,
            button,
            pressed: true,
            modifiers: m,
        });
    }

    fn release(&mut self, p: Pos2, button: PointerButton, m: Modifiers) {
        self.queue.push(Event::PointerButton {
            pos: p,
            button,
            pressed: false,
            modifiers: m,
        });
    }

    /// Regression checks for the playtest notes, then the play-mode checks.
    fn tick_more(&mut self, f: u64, ed: &mut Editor) -> bool {
        let vp = ed.viewport_rect;
        match f {
            // ---- camera look direction + cursor grab (right-drag) ----
            160 => {
                ed.doc.selection.clear();
                ed.tool = Tool::Select;
                ed.cam = crate::camera::Camera::default();
                self.look.fwd0 = ed.cam.forward();
                self.look.right0 = ed.cam.right();
                self.look.pitch0 = ed.cam.pitch;
                self.look.center = vp.center();
                self.press(vp.center(), PointerButton::Secondary, Modifiers::NONE);
            }
            // Move the pointer a little so the drag registers, as a hand would.
            161 => self
                .queue
                .push(Event::PointerMoved(self.look.center + Vec2::new(10.0, 0.0))),
            // Then raw motion only: a locked cursor stays put while the mouse moves.
            163..=172 => self.queue.push(Event::MouseMoved(Vec2::new(15.0, 0.0))),
            173 => {
                self.look.mid_grabbed = ed.look_grab.is_some();
                self.look.mid_log = ed.cursor_log.clone();
                let fwd = ed.cam.forward();
                self.look.turn_right = fwd.dot(self.look.right0);
                self.look.yaw_deg = self.look.fwd0.angle_between(fwd).to_degrees();
            }
            174..=178 => self.queue.push(Event::MouseMoved(Vec2::new(0.0, 15.0))),
            179 => {
                self.look.pitch_drop = (self.look.pitch0 - ed.cam.pitch).to_degrees();
                let p = self.look.center + Vec2::new(10.0, 0.0);
                self.release(p, PointerButton::Secondary, Modifiers::NONE);
            }
            183 => {
                let l = &self.look;
                let look_ok = l.turn_right > 0.2 && l.pitch_drop > 5.0;
                self.steps.push(Step {
                    step: "camera-look",
                    passed: look_ok,
                    detail: format!(
                        "right-drag 150 px to the right turned the view {:.1}° toward the right (forward·old right = {:+.2}); dragging 75 px down tilted it {:.1}° down",
                        l.yaw_deg, l.turn_right, l.pitch_drop
                    ),
                });
                let tail: Vec<&str> = ed
                    .cursor_log
                    .iter()
                    .rev()
                    .take(3)
                    .rev()
                    .map(|s| s.as_str())
                    .collect();
                let grabbed = l
                    .mid_log
                    .iter()
                    .any(|s| s.starts_with("grab:locked") || s.starts_with("grab:confined"))
                    && l.mid_log.iter().any(|s| s == "hide");
                let restored =
                    ed.look_grab.is_none() && tail == ["restore-position", "grab:none", "show"];
                self.steps.push(Step {
                    step: "cursor-grab",
                    passed: l.mid_grabbed && grabbed && restored,
                    detail: format!(
                        "during the drag the cursor was grabbed+hidden: {} ({}); raw mouse deltas drove the look; on release: {}",
                        l.mid_grabbed && grabbed,
                        l.mid_log.join(", "),
                        tail.join(", ")
                    ),
                });
            }
            // ---- Explorer reveal on viewport selection ----
            190 => {
                let ws = ed.doc.default_parent();
                for i in 0..40 {
                    ed.doc.insert(
                        ws,
                        InstanceBuilder::new("Folder").with_name(format!("Filler{i:02}")),
                    );
                }
                let fa = ed
                    .doc
                    .insert(ws, InstanceBuilder::new("Folder").with_name("FolderA"));
                let ma = ed
                    .doc
                    .insert(fa, InstanceBuilder::new("Model").with_name("ModelA"));
                let pa = ed.doc.insert(
                    ma,
                    crate::doc::new_part(
                        "PartA",
                        1,
                        Vec3::new(-5.0, 3.0, -20.0),
                        Vec3::splat(4.0),
                        [200, 80, 80],
                    ),
                );
                let fb = ed
                    .doc
                    .insert(ws, InstanceBuilder::new("Folder").with_name("FolderB"));
                let pb = ed.doc.insert(
                    fb,
                    crate::doc::new_part(
                        "PartB",
                        1,
                        Vec3::new(5.0, 3.0, -20.0),
                        Vec3::splat(4.0),
                        [80, 80, 200],
                    ),
                );
                self.reveal = RevealRefs {
                    ws,
                    fa,
                    ma,
                    pa,
                    fb,
                    pb,
                };
                ed.cam = crate::camera::Camera::default();
                ed.cam.pos = Vec3::new(0.0, 10.0, 2.0);
                ed.cam.pitch = -0.3;
                ed.doc.selection.clear();
                ed.explorer.expanded.clear();
            }
            193 => {
                let p = ed
                    .cam
                    .project(vp, Vec3::new(-5.0, 3.0, -20.0))
                    .unwrap_or(vp.center());
                self.look.center = p;
                self.press(p, PointerButton::Primary, Modifiers::NONE);
            }
            194 => self.release(self.look.center, PointerButton::Primary, Modifiers::NONE),
            199 => {
                let r = self.reveal;
                let e = &ed.explorer;
                self.reveal_notes.push(format!(
                    "click PartA → selected ModelA: {}, Workspace+FolderA expanded: {}, ModelA row on screen: {}",
                    ed.doc.selection == vec![r.ma],
                    e.expanded.contains(&r.ws) && e.expanded.contains(&r.fa),
                    e.visible.contains(&r.ma)
                ));
                self.reveal_ok &= ed.doc.selection == vec![r.ma]
                    && e.expanded.contains(&r.ws)
                    && e.expanded.contains(&r.fa)
                    && e.visible.contains(&r.ma);
                let p = ed
                    .cam
                    .project(vp, Vec3::new(5.0, 3.0, -20.0))
                    .unwrap_or(vp.center());
                self.look.center = p;
                self.queue.push(Event::ModifiersChanged(CTRL));
                self.press(p, PointerButton::Primary, CTRL);
            }
            200 => self.release(self.look.center, PointerButton::Primary, CTRL),
            201 => self.queue.push(Event::ModifiersChanged(Modifiers::NONE)),
            206 => {
                let r = self.reveal;
                let e = &ed.explorer;
                let ok = ed.doc.selection == vec![r.ma, r.pb]
                    && e.expanded.contains(&r.fb)
                    && e.visible.contains(&r.pb);
                self.reveal_notes.push(format!(
                    "Ctrl+click PartB → multi-select [ModelA, PartB]: {}, FolderB expanded: {}, PartB row on screen: {}",
                    ed.doc.selection == vec![r.ma, r.pb],
                    e.expanded.contains(&r.fb),
                    e.visible.contains(&r.pb)
                ));
                self.reveal_ok &= ok;
                self.queue.push(key(Key::Delete, true, Modifiers::NONE));
                self.queue.push(key(Key::Delete, false, Modifiers::NONE));
            }
            209 => {
                // Collapse everything and scroll to the top, then undo the delete.
                ed.explorer.expanded.clear();
                self.press_combo(CTRL, Key::Z);
            }
            216 => {
                let r = self.reveal;
                let e = &ed.explorer;
                let restored = ed.doc.exists(r.pa) && ed.doc.exists(r.pb);
                let ok = restored
                    && ed.doc.selection == vec![r.ma, r.pb]
                    && e.expanded.contains(&r.fa)
                    && e.expanded.contains(&r.fb)
                    && e.visible.contains(&r.pb);
                self.reveal_notes.push(format!(
                    "Delete then (all collapsed) Ctrl+Z → objects back: {restored}, selection restored: {}, both folders re-expanded: {}, PartB row on screen: {}",
                    ed.doc.selection == vec![r.ma, r.pb],
                    e.expanded.contains(&r.fa) && e.expanded.contains(&r.fb),
                    e.visible.contains(&r.pb)
                ));
                self.reveal_ok &= ok;
                self.steps.push(Step {
                    step: "explorer-reveal",
                    passed: self.reveal_ok,
                    detail: self.reveal_notes.join("; "),
                });
            }
            230.. => return self.tick_play(f, ed),
            _ => {}
        }
        false
    }
}

/// Camera-look regression state.
#[derive(Default)]
struct LookState {
    fwd0: Vec3,
    right0: Vec3,
    pitch0: f32,
    center: Pos2,
    mid_grabbed: bool,
    mid_log: Vec<String>,
    turn_right: f32,
    yaw_deg: f32,
    pitch_drop: f32,
}

#[derive(Clone, Copy)]
struct RevealRefs {
    ws: Ref,
    fa: Ref,
    ma: Ref,
    pa: Ref,
    fb: Ref,
    pb: Ref,
}

impl Default for RevealRefs {
    fn default() -> Self {
        RevealRefs {
            ws: Ref::none(),
            fa: Ref::none(),
            ma: Ref::none(),
            pa: Ref::none(),
            fb: Ref::none(),
            pb: Ref::none(),
        }
    }
}

impl SelfTest {
    /// Script runtime and play-mode checks.
    fn tick_play(&mut self, f: u64, ed: &mut Editor) -> bool {
        let tap = |q: &mut Vec<Event>, k: Key, m: Modifiers| {
            q.push(key(k, true, m));
            q.push(key(k, false, m));
        };
        match f {
            // ---- Luau runtime under Run (F8) ----
            230 => {
                let root = ed.doc.dom.root_ref();
                let rs = ed.doc.find_child(root, "ReplicatedStorage").unwrap_or(root);
                let sss = ed
                    .doc
                    .find_child(root, "ServerScriptService")
                    .unwrap_or(root);
                ed.doc.insert(
                    rs,
                    InstanceBuilder::new("ModuleScript")
                        .with_name("Lib")
                        .with_property("Source", LIB_SRC.to_string()),
                );
                self.probe = ed.doc.insert(
                    sss,
                    InstanceBuilder::new("Script")
                        .with_name("Probe")
                        .with_property("Source", PROBE_SRC.to_string()),
                );
                let ws = ed.doc.default_parent();
                let lava = ed.doc.insert(
                    ws,
                    crate::doc::new_part(
                        "Lava",
                        1,
                        Vec3::new(0.0, 0.6, -10.0),
                        Vec3::new(6.0, 0.2, 6.0),
                        [255, 90, 40],
                    )
                    .with_property("CanCollide", false)
                    .with_child(
                        InstanceBuilder::new("Script")
                            .with_name("LavaScript")
                            .with_property("Source", LAVA_SRC.to_string()),
                    ),
                );
                let _ = lava;
                if let Some(sps) = ed
                    .doc
                    .find_child(root, "StarterPlayer")
                    .and_then(|sp| ed.doc.find_child(sp, "StarterPlayerScripts"))
                {
                    ed.doc.insert(
                        sps,
                        InstanceBuilder::new("LocalScript")
                            .with_name("Hello")
                            .with_property("Source", LOCAL_SRC.to_string()),
                    );
                }
                self.edit_parts = ed.doc.count_class("BasePart");
                self.log_mark = ed.log.len();
                tap(&mut self.queue, Key::F8, Modifiers::NONE);
            }
            300 => {
                let out: Vec<String> = ed.log[self.log_mark.min(ed.log.len())..]
                    .iter()
                    .map(|l| l.text.clone())
                    .collect();
                let has = |needle: &str| out.iter().any(|l| l.contains(needle));
                let faller_y = ed
                    .doc
                    .workspace()
                    .and_then(|ws| ed.doc.find_named(ws, "Faller"))
                    .and_then(|r| crate::scene::xform_of(&ed.doc, r))
                    .map(|x| x.pos.y);
                let script_part = ed
                    .doc
                    .workspace()
                    .and_then(|ws| ed.doc.find_named(ws, "ScriptPart"))
                    .is_some();
                let checks = [
                    ("Run started", ed.is_playing()),
                    (
                        "require + print with script:line",
                        has("sum 5  —  ServerScriptService.Probe:2"),
                    ),
                    ("task.spawn", has("spawned")),
                    ("task.delay", has("delayed")),
                    ("bad member raises", has("bad member errors true")),
                    (
                        "task.wait + Changed + typeof",
                        has("waited true changes 2 CFrame true"),
                    ),
                    (
                        "error routed with line",
                        has("ServerScriptService.Probe:26: intentional test error"),
                    ),
                    ("Instance.new parented to workspace", script_part),
                    (
                        "physics: unanchored ball fell and rests on the baseplate",
                        faller_y.is_some_and(|y| y > 0.5 && y < 3.0),
                    ),
                    ("Touched fired", has("faller touched Baseplate")),
                ];
                self.luau_notes = checks.iter().map(|(n, ok)| format!("{n}: {ok}")).collect();
                self.luau_ok = checks.iter().all(|(_, ok)| *ok);
                if let Some(y) = faller_y {
                    self.luau_notes.push(format!("ball y = {y:.2}"));
                }
                tap(&mut self.queue, Key::F5, Modifiers::SHIFT);
            }
            304 => {
                let restored = !ed.is_playing()
                    && ed.doc.exists(self.probe)
                    && ed
                        .doc
                        .workspace()
                        .and_then(|ws| ed.doc.find_named(ws, "ScriptPart"))
                        .is_none()
                    && ed.doc.count_class("BasePart") == self.edit_parts;
                self.luau_notes
                    .push(format!("Shift+F5 restored the edit DataModel: {restored}"));
                self.steps.push(Step {
                    step: "luau",
                    passed: self.luau_ok && restored,
                    detail: self.luau_notes.join("; "),
                });
                self.log_mark = ed.log.len();
                tap(&mut self.queue, Key::F5, Modifiers::NONE);
            }
            // ---- Play (F5): character, walking, jumping, touch scripts ----
            310 => {
                self.char_start = ed
                    .play
                    .as_ref()
                    .and_then(|p| p.character.as_ref())
                    .map(|c| c.body.pos);
                self.queue.push(key(Key::W, true, Modifiers::NONE));
            }
            370 => self.queue.push(key(Key::W, false, Modifiers::NONE)),
            374 => {
                let pos = ed
                    .play
                    .as_ref()
                    .and_then(|p| p.character.as_ref())
                    .map(|c| c.body.pos);
                self.walked = match (self.char_start, pos) {
                    (Some(a), Some(b)) => Vec3::new(b.x - a.x, 0.0, b.z - a.z).length(),
                    _ => 0.0,
                };
                self.ground_y = pos.map(|p| p.y).unwrap_or(0.0);
                self.max_y = self.ground_y;
                self.queue.push(key(Key::Space, true, Modifiers::NONE));
            }
            378 => self.queue.push(key(Key::Space, false, Modifiers::NONE)),
            375..=400 => {
                if let Some(p) = ed.play.as_ref().and_then(|p| p.character.as_ref()) {
                    self.max_y = self.max_y.max(p.body.pos.y);
                }
            }
            _ => {}
        }
        match f {
            402 => {
                let out: Vec<String> = ed.log[self.log_mark.min(ed.log.len())..]
                    .iter()
                    .map(|l| l.text.clone())
                    .collect();
                let has = |needle: &str| out.iter().any(|l| l.contains(needle));
                let s = ed.play.as_ref();
                let character = s.and_then(|p| p.character.as_ref());
                let player_ok = s.is_some_and(|p| {
                    p.player.is_some() && matches!(ed.doc.prop(p.player, "Character"), Some(rbx_dom_weak::types::Variant::Ref(c)) if Some(c) == character.map(|c| c.model))
                });
                // The view is the client's CurrentCamera, which Roblox's
                // CameraModule keeps on the character (Focus at the head).
                let client = s.and_then(|p| {
                    if p.showing == crate::runtime::Side::Client {
                        Some(&ed.doc)
                    } else {
                        ed.play_other_doc()
                    }
                });
                let cam_follows = character.is_some_and(|c| {
                    client.is_some_and(|d| {
                        let cam = d.workspace().and_then(|ws| match d.prop(ws, "CurrentCamera") {
                            Some(rbx_dom_weak::types::Variant::Ref(r)) => Some(r),
                            _ => None,
                        });
                        let focus = cam.and_then(|cam| match d.prop(cam, "Focus") {
                            Some(rbx_dom_weak::types::Variant::CFrame(f)) => Some(Vec3::new(f.position.x, f.position.y, f.position.z)),
                            _ => None,
                        });
                        let at = cam.and_then(|cam| crate::scene::xform_of(d, cam)).map(|x| x.pos);
                        // (Watching the server side leaves the editor camera free.)
                        let shown = s.is_some_and(|p| p.showing == crate::runtime::Side::Server) || at.is_some_and(|p| (p - ed.cam.pos).length() < 0.01);
                        matches!((focus, at), (Some(f), Some(p)) if (f - c.body.pos).length() < 4.0 && (p - f).length() > 5.0) && shown
                    })
                });
                let jump = self.max_y - self.ground_y;
                let checks = [
                    (
                        "character spawned with Humanoid and Player.Character set",
                        player_ok,
                    ),
                    ("W walked the character", self.walked > 8.0),
                    ("Space jumped", jump > 2.0),
                    ("follow camera on the character", cam_follows),
                    (
                        "Touched from the character reached the part's Script",
                        has("lava touched Player"),
                    ),
                    (
                        "LocalScript in StarterPlayerScripts ran",
                        has("local script runs for Player"),
                    ),
                ];
                let mut notes: Vec<String> =
                    checks.iter().map(|(n, ok)| format!("{n}: {ok}")).collect();
                notes.push(format!(
                    "walked {:.1} studs in 1 s, jumped {:.1} studs",
                    self.walked, jump
                ));
                self.play_ok = checks.iter().all(|(_, ok)| *ok);
                self.play_notes = notes;
                tap(&mut self.queue, Key::F5, Modifiers::SHIFT);
            }
            406 => {
                let restored = !ed.is_playing()
                    && ed.doc.count_class("BasePart") == self.edit_parts
                    && ed.doc.count_class("Humanoid") == 0;
                self.play_notes
                    .push(format!("Shift+F5 restored the edit DataModel: {restored}"));
                self.steps.push(Step {
                    step: "play",
                    passed: self.play_ok && restored,
                    detail: self.play_notes.join("; "),
                });
            }
            407.. => return self.tick_cam(f, ed),
            _ => {}
        }
        false
    }
}

const LIB_SRC: &str = "local M = {}\nfunction M.add(a, b)\n\treturn a + b\nend\nreturn M\n";

/// Line numbers matter: the test checks `Probe:2` and `Probe:26`.
const PROBE_SRC: &str = r#"local Lib = require(game:GetService("ReplicatedStorage"):WaitForChild("Lib"))
print("sum", Lib.add(2, 3))
local p = Instance.new("Part")
p.Name = "ScriptPart"
p.Anchored = true
p.Size = Vector3.new(2, 2, 2)
p.CFrame = CFrame.new(0, 40, 0) * CFrame.Angles(0, math.rad(90), 0)
p.Color = Color3.fromRGB(255, 0, 0)
local changes = 0
p.Changed:Connect(function() changes += 1 end)
p.Parent = workspace
p.Transparency = 0.5
local ball = Instance.new("Part")
ball.Name = "Faller"
ball.Shape = Enum.PartType.Ball
ball.Size = Vector3.new(2, 2, 2)
ball.Position = Vector3.new(30, 20, 30)
ball.Touched:Connect(function(hit) if hit.Name == "Baseplate" then print("faller touched", hit.Name) end end)
ball.Parent = workspace
task.spawn(function() print("spawned") end)
task.delay(0.2, function() print("delayed") end)
local ok = pcall(function() return p.NotAProperty end)
print("bad member errors", not ok)
local t = task.wait(0.25)
print("waited", t >= 0.24, "changes", changes, typeof(p.CFrame), p:IsA("BasePart"))
error("intentional test error")
"#;

const LAVA_SRC: &str = r#"script.Parent.Touched:Connect(function(hit)
	local h = hit.Parent:FindFirstChildOfClass("Humanoid")
	if h then
		print("lava touched", hit.Parent.Name)
	end
end)
"#;

const LOCAL_SRC: &str =
    "print(\"local script runs for\", game:GetService(\"Players\").LocalPlayer.Name)\n";

/// Wheel-zoom and fly-acceleration regression state.
#[derive(Default)]
struct CamTest {
    pos0: Vec3,
    fwd0: Vec3,
    notes: Vec<String>,
    ok: bool,
    prev: Vec3,
    early: f32,
    late: f32,
    after_reset: f32,
}

/// A mouse-wheel notch toward the screen (zoom in).
fn wheel_notch() -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: Vec2::new(0.0, 1.0),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    }
}

impl SelfTest {
    /// Playtest round 3: wheel zoom toward the cursor, scaled by distance;
    /// fly speed ramps up while held and resets after a pause.
    fn tick_cam(&mut self, f: u64, ed: &mut Editor) -> bool {
        let vp = ed.viewport_rect;
        let reset_cam = |ed: &mut Editor| {
            ed.cam = crate::camera::Camera::default();
            ed.fly_held = 0.0;
            ed.fly_idle = 10.0;
            ed.last_zoom = None;
        };
        match f {
            410 => {
                reset_cam(ed);
                self.cam.ok = true;
                self.cam.pos0 = ed.cam.pos;
                self.cam.fwd0 = ed.cam.forward();
                // Lower right of the view: the baseplate, well off-centre.
                let p = vp.min + egui::vec2(vp.width() * 0.8, vp.height() * 0.8);
                self.queue.push(Event::PointerMoved(p));
                self.queue.push(wheel_notch());
            }
            413 => {
                let moved = ed.cam.pos - self.cam.pos0;
                let (ok, note) = match ed.last_zoom {
                    Some((dir, Some(hit))) => {
                        let along = moved.normalize_or_zero().dot(dir);
                        let off_centre = moved.normalize_or_zero().dot(self.cam.fwd0);
                        // Studio: a fixed "Camera Mouse Wheel Speed" step toward the mouse.
                        let ok = along > 0.999
                            && (moved.length() - crate::mech::WHEEL_STEP).abs() < 0.01
                            && off_centre < 0.995;
                        (ok, format!(
                            "one notch over the baseplate {hit:.1} studs away moved the camera {:.2} studs along the cursor ray (Studio: {}; alignment {along:.4}; vs screen centre {off_centre:.3})",
                            moved.length(),
                            crate::mech::WHEEL_STEP
                        ))
                    }
                    other => (
                        false,
                        format!("wheel over the baseplate found no hit: {other:?}"),
                    ),
                };
                self.cam.ok &= ok;
                self.cam.notes.push(note);
                reset_cam(ed);
                self.cam.pos0 = ed.cam.pos;
                // Near the top: open sky, nothing under the cursor.
                let p = vp.min + egui::vec2(vp.width() * 0.5, vp.height() * 0.06);
                self.queue.push(Event::PointerMoved(p));
                self.queue.push(wheel_notch());
            }
            416 => {
                let moved = ed.cam.pos - self.cam.pos0;
                let (ok, note) = match ed.last_zoom {
                    Some((dir, None)) => {
                        let along = moved.normalize_or_zero().dot(dir);
                        (along > 0.999 && (moved.length() - crate::mech::WHEEL_STEP).abs() < 0.01, format!("one notch into empty sky moved {:.1} studs along the cursor ray (alignment {along:.4})", moved.length()))
                    }
                    other => (false, format!("sky zoom: unexpected {other:?}")),
                };
                self.cam.ok &= ok;
                self.cam.notes.push(note);
                self.steps.push(Step {
                    step: "wheel-zoom",
                    passed: self.cam.ok,
                    detail: self.cam.notes.join("; "),
                });
                // Fly acceleration.
                reset_cam(ed);
                self.queue.push(Event::PointerMoved(vp.center()));
                self.queue.push(key(Key::W, true, Modifiers::NONE));
            }
            417..=570 => {
                let step = (ed.cam.pos - self.cam.prev).length();
                if f == 419 {
                    self.cam.early = step;
                }
                if f == 569 {
                    self.cam.late = step;
                }
                if f == 570 {
                    self.queue.push(key(Key::W, false, Modifiers::NONE));
                }
            }
            // Keys released (Studio resets its hold counter at once), then move again.
            671 => self.queue.push(key(Key::W, true, Modifiers::NONE)),
            674 => {
                self.cam.after_reset = (ed.cam.pos - self.cam.prev).length();
                self.queue.push(key(Key::W, false, Modifiers::NONE));
            }
            676 => {
                let base = crate::mech::FLY_SPEED / 60.0;
                let c = &self.cam;
                let ramp = c.late / c.early.max(1e-6);
                // Studio's rule: base step for 120 ticks, then
                // ticks / 240; frame 569 is about 2.5 s into the hold.
                let held = (569.0 - 417.0) / 60.0;
                let expect = crate::viewport::fly_boost_at(held);
                let ok = (c.early - base).abs() < base * 0.15
                    && (ramp - expect).abs() < expect * 0.1
                    && expect > 2.5
                    && (c.after_reset - base).abs() < base * 0.2;
                self.steps.push(Step {
                    step: "fly-acceleration",
                    passed: ok,
                    detail: format!(
                        "holding W: {:.2} studs/frame at first, {:.2} after 2.5 s (×{ramp:.2}, Studio's rule expects ×{expect:.2}); after letting go and pressing again {:.2} (base {base:.2})",
                        c.early, c.late, c.after_reset
                    ),
                });
            }
            813.. => return self.tick_r4(f, ed),
            680.. => return self.tick_m3(f, ed),
            _ => {}
        }
        self.cam.prev = ed.cam.pos;
        false
    }

    /// Milestone 3: client/server Play, R15 character, asset pipeline.
    fn tick_m3(&mut self, f: u64, ed: &mut Editor) -> bool {
        use crate::runtime::Side;
        let tap = |q: &mut Vec<Event>, k: Key, m: Modifiers| {
            q.push(key(k, true, m));
            q.push(key(k, false, m));
        };
        let has = |ed: &Editor, mark: usize, needle: &str| {
            ed.log[mark.min(ed.log.len())..]
                .iter()
                .any(|l| l.text.contains(needle))
        };
        match f {
            680 => {
                ed.cam = crate::camera::Camera::default();
                let root = ed.doc.dom.root_ref();
                let child = |ed: &Editor, n: &str| ed.doc.find_child(root, n).unwrap_or(root);
                let (ss, rs, sss) = (
                    child(ed, "ServerStorage"),
                    child(ed, "ReplicatedStorage"),
                    child(ed, "ServerScriptService"),
                );
                ed.doc
                    .insert(ss, InstanceBuilder::new("Folder").with_name("Secret"));
                ed.doc
                    .insert(rs, InstanceBuilder::new("RemoteEvent").with_name("Ping"));
                ed.doc
                    .insert(rs, InstanceBuilder::new("RemoteFunction").with_name("Ask"));
                ed.doc.insert(
                    sss,
                    InstanceBuilder::new("Script")
                        .with_name("Srv")
                        .with_property("Source", SRV_SRC.to_string()),
                );
                if let Some(sps) = ed
                    .doc
                    .find_child(root, "StarterPlayer")
                    .and_then(|sp| ed.doc.find_child(sp, "StarterPlayerScripts"))
                {
                    ed.doc.insert(
                        sps,
                        InstanceBuilder::new("LocalScript")
                            .with_name("Cli")
                            .with_property("Source", CLI_SRC.to_string()),
                    );
                }
                if let Some(scs) = ed
                    .doc
                    .find_child(root, "StarterPlayer")
                    .and_then(|sp| ed.doc.find_child(sp, "StarterCharacterScripts"))
                {
                    ed.doc.insert(
                        scs,
                        InstanceBuilder::new("LocalScript")
                            .with_name("Swing")
                            .with_property("Source", SWING_SRC.to_string()),
                    );
                }
                ed.play_rig = Some(crate::rig::RigType::R15);
                self.m3.mark = ed.log.len();
                self.m3.edit_parts = ed.doc.count_class("BasePart");
                tap(&mut self.queue, Key::F5, Modifiers::NONE);
            }
            // Walk, then jump, while the remotes do their work.
            690 => self.queue.push(key(Key::W, true, Modifiers::NONE)),
            691..=730 => {
                if let Some(ch) = ed.play.as_ref().and_then(|p| p.character.as_ref()) {
                    let leg = ed.doc.find_named(ch.model, "LeftUpperLeg");
                    let root = crate::scene::xform_of(&ed.doc, ch.rig.root);
                    if let (Some(leg), Some(root)) =
                        (leg.and_then(|l| crate::scene::xform_of(&ed.doc, l)), root)
                    {
                        let rel = root.rot.transpose() * leg.rot;
                        let angle = rel.y_axis.angle_between(Vec3::Y);
                        self.m3.leg_min = self.m3.leg_min.min(angle);
                        self.m3.leg_max = self.m3.leg_max.max(angle);
                    }
                }
            }
            731 => {
                self.queue.push(key(Key::W, false, Modifiers::NONE));
                self.queue.push(key(Key::Space, true, Modifiers::NONE));
            }
            734 => self.queue.push(key(Key::Space, false, Modifiers::NONE)),
            790 => {
                let mark = self.m3.mark;
                let s = ed.play.as_ref();
                let other = ed.play_other_doc();
                let server = &ed.doc;
                let ws_named = |d: &crate::doc::Document, n: &str| {
                    d.workspace().and_then(|w| d.find_named(w, n)).is_some()
                };
                let kids = |d: &crate::doc::Document, n: &str| {
                    d.find_child(d.dom.root_ref(), n)
                        .and_then(|x| d.dom.get_by_ref(x))
                        .map(|i| i.children().len())
                        .unwrap_or(99)
                };
                let client_ok = other.is_some_and(|c| {
                    ws_named(c, "ServerMade")
                        && ws_named(c, "ClientOnly")
                        && kids(c, "ServerStorage") == 0
                        && kids(c, "ServerScriptService") == 0
                });
                let server_ok = ws_named(server, "ServerMade")
                    && !ws_named(server, "ClientOnly")
                    && kids(server, "ServerStorage") > 0;
                let checks = [
                    ("server and client both running", s.is_some_and(|p| p.showing == Side::Server) && other.is_some()),
                    ("RunService sides", has(ed, mark, "[Server] server sees client? false server? true") && has(ed, mark, "[Client] client: true local player Player storage empty true")),
                    ("FireServer → OnServerEvent", has(ed, mark, "[Server] server got ping 21 from Player")),
                    ("FireClient → OnClientEvent", has(ed, mark, "[Client] client got pong 42")),
                    ("InvokeServer → OnServerInvoke reply", has(ed, mark, "[Client] invoke returned 42 sum")),
                    ("server property change replicated (Changed on client)", has(ed, mark, "[Client] client saw color 200")),
                    ("server DataModel: ServerMade, no ClientOnly, ServerStorage kept", server_ok),
                    ("client DataModel: ServerMade replicated, ClientOnly local, server services empty", client_ok),
                ];
                self.m3.cs_notes = checks.iter().map(|(n, ok)| format!("{n}: {ok}")).collect();
                self.m3.cs_ok = checks.iter().all(|(_, ok)| *ok);
                // Character.
                let ch = s.and_then(|p| p.character.as_ref());
                let motors = ch
                    .map(|c| {
                        server
                            .dom
                            .descendants_of(c.model)
                            // Studio 0.746's joint upgrade: AnimationConstraints
                            // (Motor6Ds with StarterPlayer.AvatarJointUpgrade off).
                            .filter(|i| {
                                i.class
                                    == if crate::rig::joint_upgrade(server) {
                                        "AnimationConstraint"
                                    } else {
                                        "Motor6D"
                                    }
                            })
                            .count()
                    })
                    .unwrap_or(0);
                let r15 = ch.is_some_and(|c| c.rig_type == crate::rig::RigType::R15);
                let upright = ch
                    .and_then(|c| crate::scene::xform_of(server, c.rig.root))
                    .is_some_and(|x| x.rot.y_axis.y > 0.999);
                let owned = ch.zip(other).zip(s).is_some_and(|((c, cd), p)| {
                    let sc = crate::scene::xform_of(server, c.rig.root);
                    let cc = p
                        .map
                        .s2c
                        .get(&c.rig.root)
                        .and_then(|r| crate::scene::xform_of(cd, *r));
                    matches!((sc, cc), (Some(a), Some(b)) if (a.pos - b.pos).length() < 1e-3)
                });
                let log = s.map(|p| p.state_log.join(" → ")).unwrap_or_default();
                let seq = ["Jumping", "Freefall", "Landed", "Running"];
                let mut it = s
                    .map(|p| p.state_log.clone())
                    .unwrap_or_default()
                    .into_iter();
                let states_ok = seq.iter().all(|want| it.any(|got| got == *want));
                let swing = self.m3.leg_max - self.m3.leg_min;
                let checks = [
                    ("R15 rig with 15 joints (AnimationConstraints, Studio's joint upgrade)", r15 && motors == 15),
                    ("a KeyframeSequence played through the Humanoid swings the leg (Motor6D.Transform)", swing > 0.4 && has(ed, mark, "swing track length 0.5")),
                    ("states Jumping → Freefall → Landed → Running", states_ok),
                    ("stays upright", upright),
                    ("client-owned character matches on the server", owned),
                ];
                self.m3.ch_notes = checks.iter().map(|(n, ok)| format!("{n}: {ok}")).collect();
                self.m3
                    .ch_notes
                    .push(format!("leg swing {swing:.2} rad; states {log}"));
                self.m3.ch_ok = checks.iter().all(|(_, ok)| *ok);
                // Switch the Explorer to the client view with a click.
                if let Some(r) = ed.ui_rects.get("view.client") {
                    let c = r.center();
                    self.press(c, PointerButton::Primary, Modifiers::NONE);
                    self.m3.click = c;
                }
            }
            791 => self.release(self.m3.click, PointerButton::Primary, Modifiers::NONE),
            795 => {
                let viewing_client = ed.play.as_ref().is_some_and(|p| p.showing == Side::Client)
                    && ed
                        .doc
                        .workspace()
                        .and_then(|w| ed.doc.find_named(w, "ClientOnly"))
                        .is_some();
                self.m3.cs_notes.push(format!(
                    "Explorer switched to the client view: {viewing_client}"
                ));
                self.m3.cs_ok &= viewing_client;
                tap(&mut self.queue, Key::F5, Modifiers::SHIFT);
            }
            800 => {
                let restored = !ed.is_playing()
                    && ed.doc.count_class("BasePart") == self.m3.edit_parts
                    && ed.doc.count_class("Humanoid") == 0;
                self.m3
                    .cs_notes
                    .push(format!("Stop restored the edit DataModel: {restored}"));
                self.steps.push(Step {
                    step: "client-server",
                    passed: self.m3.cs_ok && restored,
                    detail: self.m3.cs_notes.join("; "),
                });
                self.steps.push(Step {
                    step: "character",
                    passed: self.m3.ch_ok,
                    detail: self.m3.ch_notes.join("; "),
                });
                // Asset pipeline, offline: a cache filled as a download would.
                let ok = (|| -> Result<(), String> {
                    let mesh = crate::assets::cache_path(crate::assets::AssetRef {
                        id: 900001,
                        version: None,
                    });
                    std::fs::create_dir_all(mesh.parent().ok_or("cache dir")?)
                        .map_err(|e| e.to_string())?;
                    std::fs::write(&mesh, PYRAMID_OBJ).map_err(|e| e.to_string())?;
                    let img = image::RgbaImage::from_fn(8, 8, |x, y| {
                        image::Rgba([200, (x * 30) as u8, (y * 30) as u8, 255])
                    });
                    let mut png = Vec::new();
                    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                        .map_err(|e| e.to_string())?;
                    std::fs::write(
                        crate::assets::cache_path(crate::assets::AssetRef {
                            id: 900002,
                            version: None,
                        }),
                        png,
                    )
                    .map_err(|e| e.to_string())
                })();
                if let Err(e) = ok {
                    self.m3.asset_notes.push(format!("cache setup failed: {e}"));
                }
                let ws = ed.doc.default_parent();
                use rbx_dom_weak::types::{CFrame, ContentId, Enum, Vector3};
                let at = |x: f32| {
                    CFrame::new(Vector3::new(x, 3.0, -8.0), CFrame::identity().orientation)
                };
                ed.doc.insert(
                    ws,
                    InstanceBuilder::new("MeshPart")
                        .with_name("CachedMesh")
                        .with_property("Anchored", true)
                        .with_property("Size", Vector3::new(4.0, 4.0, 4.0))
                        .with_property("CFrame", at(-6.0))
                        .with_property("MeshId", ContentId::from("rbxassetid://900001"))
                        .with_property("TextureID", ContentId::from("rbxassetid://900002")),
                );
                ed.doc.insert(
                    ws,
                    crate::doc::new_part(
                        "Sign",
                        1,
                        Vec3::new(0.0, 3.0, -8.0),
                        Vec3::new(4.0, 4.0, 1.0),
                        [90, 90, 90],
                    )
                    .with_child(
                        InstanceBuilder::new("Decal")
                            .with_name("Decal")
                            .with_property("Texture", ContentId::from("rbxassetid://900002"))
                            .with_property("Face", Enum::from_u32(5)),
                    ),
                );
                ed.doc.insert(
                    ws,
                    InstanceBuilder::new("Model")
                        .with_name("Mannequin")
                        .with_child(crate::doc::new_part(
                            "Torso",
                            1,
                            Vec3::new(6.0, 3.0, -8.0),
                            Vec3::new(2.0, 2.0, 1.0),
                            [200, 200, 200],
                        ))
                        .with_child(
                            InstanceBuilder::new("Shirt")
                                .with_name("Shirt")
                                .with_property(
                                    "ShirtTemplate",
                                    ContentId::from("rbxassetid://900002"),
                                ),
                        ),
                );
            }
            812 => {
                let [meshes, textured, decals, clothed] = ed.draw_stats;
                let ok = meshes >= 1 && textured >= 2 && decals >= 1 && clothed >= 1;
                self.m3.asset_notes.push(format!(
                    "from the asset cache (rbxassetid → ~/.cache/rustudio layout): {meshes} mesh drawn, {textured} textured parts (MeshPart TextureID + Shirt on a Torso), {decals} decal quad, {clothed} clothed body part"
                ));
                self.steps.push(Step {
                    step: "assets",
                    passed: ok && !self.m3.asset_notes.iter().any(|n| n.contains("failed")),
                    detail: self.m3.asset_notes.join("; "),
                });
            }
            _ => {}
        }
        false
    }
}

/// Milestone 3 state.
#[derive(Default)]
struct M3Test {
    mark: usize,
    edit_parts: usize,
    leg_min: f32,
    leg_max: f32,
    cs_ok: bool,
    cs_notes: Vec<String>,
    ch_ok: bool,
    ch_notes: Vec<String>,
    asset_notes: Vec<String>,
    click: Pos2,
}

const SRV_SRC: &str = r#"local RS = game:GetService("ReplicatedStorage")
local RunService = game:GetService("RunService")
local ping, ask = RS:WaitForChild("Ping"), RS:WaitForChild("Ask")
print("server sees client?", RunService:IsClient(), "server?", RunService:IsServer())
ping.OnServerEvent:Connect(function(player, word, n)
	print("server got", word, n, "from", player.Name)
	ping:FireClient(player, "pong", n * 2)
end)
ask.OnServerInvoke = function(player, a, b)
	return a + b, "sum"
end
local made = Instance.new("Part")
made.Name = "ServerMade"
made.Anchored = true
made.Position = Vector3.new(0, 30, -40)
made.Parent = workspace
task.delay(0.3, function()
	made.Color = Color3.fromRGB(10, 200, 30)
end)
"#;

/// A walk-like KeyframeSequence built in a LocalScript, registered with
/// KeyframeSequenceProvider and played through the Humanoid: the engine
/// poses the leg from the Motor6D.Transform the track writes.
const SWING_SRC: &str = r#"local function pose(name, cf, kids)
	local p = Instance.new("Pose")
	p.Name = name
	p.CFrame = cf
	for _, k in kids or {} do k.Parent = p end
	return p
end
local seq = Instance.new("KeyframeSequence")
seq.Loop = true
seq.Priority = Enum.AnimationPriority.Movement
for i, angle in { 0.6, -0.6, 0.6 } do
	local k = Instance.new("Keyframe")
	k.Time = (i - 1) * 0.25
	pose("HumanoidRootPart", CFrame.new(), { pose("LowerTorso", CFrame.new(), { pose("LeftUpperLeg", CFrame.Angles(angle, 0, 0)) }) }).Parent = k
	k.Parent = seq
end
local anim = Instance.new("Animation")
anim.AnimationId = game:GetService("KeyframeSequenceProvider"):RegisterKeyframeSequence(seq)
local track = script.Parent:WaitForChild("Humanoid"):LoadAnimation(anim)
track:Play()
print("swing track length", track.Length)
"#;

const CLI_SRC: &str = r#"local RS = game:GetService("ReplicatedStorage")
local Players = game:GetService("Players")
print("client:", game:GetService("RunService"):IsClient(), "local player", Players.LocalPlayer.Name, "storage empty", #game:GetService("ServerStorage"):GetChildren() == 0)
local ping = RS:WaitForChild("Ping")
ping.OnClientEvent:Connect(function(word, n)
	print("client got", word, n)
end)
ping:FireServer("ping", 21)
local a, b = RS:WaitForChild("Ask"):InvokeServer(40, 2)
print("invoke returned", a, b)
local mine = Instance.new("Part")
mine.Name = "ClientOnly"
mine.Parent = workspace
local made = workspace:WaitForChild("ServerMade")
made:GetPropertyChangedSignal("Color"):Connect(function()
	print("client saw color", math.floor(made.Color.G * 255 + 0.5))
end)
"#;

const PYRAMID_OBJ: &str = "v -1 0 -1\nv 1 0 -1\nv 1 0 1\nv -1 0 1\nv 0 2 0\nvt 0 1\nvt 1 1\nvt 0.5 0\nf 1/1 2/2 5/3\nf 2/1 3/2 5/3\nf 3/1 4/2 5/3\nf 4/1 1/2 5/3\nf 1 3 2\nf 1 4 3\n";

// ------------------------------------------------------------------ round 4

/// Content folder, in-game APIs and the og.rbxl performance check.
#[derive(Default)]
struct R4Test {
    phase: u8,
    started: u64,
    mark: usize,
    bench: Option<crate::bench::Bench>,
    perf_place: Option<PathBuf>,
}

/// A Roblox mesh in the text "version 1.00" format (community-documented):
/// a tetrahedron, our own geometry. Positions in 1.00 files are doubled.
fn tetra_mesh_v1() -> String {
    let p = [
        [0.0f32, 2.0, 0.0],
        [-2.0, -2.0, -2.0],
        [2.0, -2.0, -2.0],
        [0.0, -2.0, 2.0],
    ];
    let faces = [[0usize, 2, 1], [0, 3, 2], [0, 1, 3], [1, 2, 3]];
    let mut s = format!("version 1.00\n{}\n", faces.len());
    for f in faces {
        let (a, b, c) = (
            glam::Vec3::from(p[f[0]]),
            glam::Vec3::from(p[f[1]]),
            glam::Vec3::from(p[f[2]]),
        );
        let n = (b - a).cross(c - a).normalize();
        for (i, v) in [a, b, c].iter().enumerate() {
            s.push_str(&format!(
                "[{},{},{}][{},{},{}][{},{},0]",
                v.x,
                v.y,
                v.z,
                n.x,
                n.y,
                n.z,
                i as f32 * 0.5,
                1.0 - i as f32 * 0.5
            ));
        }
    }
    s.push('\n');
    s
}

/// A short silent WAV (16-bit mono) for the Sound test.
fn wav(seconds: f32) -> Vec<u8> {
    let rate = 8000u32;
    let n = (rate as f32 * seconds) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + n * 2).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(n * 2).to_le_bytes());
    b.resize(b.len() + (n * 2) as usize, 0);
    b
}

const GAME_SERVER_SRC: &str = r#"local Players = game:GetService("Players")
local button = workspace:WaitForChild("Button")
button.ClickDetector.MouseClick:Connect(function(player) print("clicked by " .. player.Name) end)
button.ClickDetector.MouseHoverEnter:Connect(function(player) print("hover " .. player.Name) end)
Players.PlayerAdded:Connect(function(player)
	player.Chatted:Connect(function(msg) print("chat " .. msg) end)
end)
for _, p in ipairs(Players:GetPlayers()) do
	p.Chatted:Connect(function(msg) print("chat " .. msg) end)
end
local sound = workspace:WaitForChild("Beep")
sound.Ended:Connect(function() print("sound ended") end)
sound:Play()
delay(0.5, function()
	local loose = Instance.new("Part")
	loose.Position = Vector3.new(30, 2, 30)
	loose.Parent = workspace
	local e = Instance.new("Explosion")
	e.Position = Vector3.new(30, 1, 30)
	e.BlastRadius = 6
	local hits = 0
	e.Hit:Connect(function() hits += 1 end)
	e.Parent = workspace
	delay(0.2, function() print("explosion hits " .. hits) end)
end)
"#;

const TOOL_SRC: &str = r#"local tool = script.Parent
tool.Equipped:Connect(function() print("equipped " .. tool.Name) end)
tool.Activated:Connect(function() print("activated " .. tool.Name) end)
"#;

impl SelfTest {
    fn tick_r4(&mut self, f: u64, ed: &mut Editor) -> bool {
        use crate::doc::new_part;
        use rbx_dom_weak::types::{ContentId, Enum};
        let has = |ed: &Editor, mark: usize, needle: &str| {
            ed.log[mark.min(ed.log.len())..]
                .iter()
                .any(|l| l.text.contains(needle))
        };
        match self.r4.phase {
            // Content folder: rbxasset:// meshes and textures from our own files.
            0 => {
                let dir = crate::assets::content_dir();
                let _ = std::fs::create_dir_all(dir.join("fonts"));
                let _ = std::fs::create_dir_all(dir.join("textures"));
                let _ = std::fs::create_dir_all(dir.join("sounds"));
                let _ = std::fs::write(dir.join("fonts/selftest.mesh"), tetra_mesh_v1());
                let img = image::RgbaImage::from_fn(8, 8, |x, y| {
                    image::Rgba([(x * 32) as u8, (y * 32) as u8, 160, 255])
                });
                let _ = img.save(dir.join("textures/selftest.png"));
                let _ = std::fs::write(dir.join("sounds/selftest.wav"), wav(0.2));
                ed.set_document(Document::new_place());
                let ws = ed.doc.workspace().expect("workspace");
                ed.doc.insert(
                    ws,
                    new_part(
                        "Gem",
                        1,
                        Vec3::new(0.0, 4.0, -10.0),
                        Vec3::new(2.0, 2.0, 2.0),
                        [200, 120, 40],
                    )
                    .with_child(
                        InstanceBuilder::new("SpecialMesh")
                            .with_property("MeshType", Enum::from_u32(5))
                            .with_property(
                                "MeshId",
                                ContentId::from("rbxasset://fonts/selftest.mesh"),
                            )
                            .with_property(
                                "TextureId",
                                ContentId::from("rbxasset://textures/selftest.png"),
                            ),
                    ),
                );
                ed.doc.insert(
                    ws,
                    new_part(
                        "Face",
                        1,
                        Vec3::new(4.0, 3.0, -10.0),
                        Vec3::new(2.0, 2.0, 2.0),
                        [240, 220, 180],
                    )
                    .with_child(
                        InstanceBuilder::new("Decal")
                            .with_property(
                                "Texture",
                                ContentId::from("rbxasset://textures/selftest.png"),
                            )
                            .with_property("Face", Enum::from_u32(5)),
                    ),
                );
                self.r4.phase = 1;
                self.r4.started = f;
            }
            1 if f >= self.r4.started + 10 => {
                let [meshes, textured, decals, _] = ed.draw_stats;
                let ok = meshes >= 1 && textured >= 1 && decals >= 1;
                self.steps.push(Step {
                    step: "content-folder",
                    passed: ok,
                    detail: format!(
                        "rbxasset:// from the content folder (a temp folder of generated files here, never the Studio install): {meshes} mesh (text v1.00 .mesh), {textured} textured, {decals} decal (PNG)"
                    ),
                });
                self.r4.phase = 2;
            }
            // In-game APIs during Play.
            2 => {
                ed.set_document(Document::new_place());
                let root = ed.doc.dom.root_ref();
                let ws = ed.doc.workspace().expect("workspace");
                let sss = ed.doc.find_child(root, "ServerScriptService").expect("sss");
                let pack = ed
                    .doc
                    .find_child(root, "StarterPack")
                    .expect("starter pack");
                ed.doc.insert(
                    ws,
                    new_part(
                        "Button",
                        1,
                        Vec3::new(6.0, 1.5, 0.0),
                        Vec3::new(2.0, 1.0, 2.0),
                        [200, 40, 40],
                    )
                    .with_child(InstanceBuilder::new("ClickDetector")),
                );
                ed.doc.insert(
                    ws,
                    InstanceBuilder::new("Sound")
                        .with_name("Beep")
                        .with_property(
                            "SoundId",
                            ContentId::from("rbxasset://sounds/selftest.wav"),
                        ),
                );
                ed.doc.insert(
                    sss,
                    InstanceBuilder::new("Script")
                        .with_name("Game")
                        .with_property("Source", GAME_SERVER_SRC.to_string()),
                );
                ed.doc.insert(
                    pack,
                    InstanceBuilder::new("Tool")
                        .with_name("Wand")
                        .with_child(
                            new_part(
                                "Handle",
                                1,
                                Vec3::new(0.0, 5.0, 0.0),
                                Vec3::new(0.4, 2.0, 0.4),
                                [120, 80, 200],
                            )
                            .with_property("Anchored", false),
                        )
                        .with_child(
                            InstanceBuilder::new("Script")
                                .with_name("WandScript")
                                .with_property("Source", TOOL_SRC.to_string()),
                        ),
                );
                self.r4.mark = ed.log.len();
                ed.start_play(crate::runtime::RunMode::Play);
                self.r4.phase = 3;
                self.r4.started = f;
            }
            3 => {
                let t = f - self.r4.started;
                let button = ed
                    .doc
                    .workspace()
                    .and_then(|ws| ed.doc.find_named(ws, "Button"));
                if let (Some(s), Some(b)) = (ed.play.as_mut(), button) {
                    use crate::play::game::Pointer;
                    let side = s.showing;
                    match t {
                        20 => {
                            s.game.input.pointer = Some(Pointer {
                                origin: Vec3::new(6.0, 10.0, 0.0),
                                dir: -Vec3::Y,
                                hit: Some((b, Vec3::new(6.0, 2.0, 0.0))),
                                xy: [0.0, 0.0],
                                view: [800.0, 600.0],
                                side,
                            });
                            s.game.input.clicks.push((b, 1));
                        }
                        24 => s.game.input.chat.push("hello rustudio".into()),
                        28 => s.game.input.slots.push(0),
                        40 => {
                            s.game.input.buttons.push((1, true));
                            s.game.input.buttons.push((1, false));
                        }
                        _ => {}
                    }
                }
                let want = [
                    "clicked by Player",
                    "hover Player",
                    "chat hello rustudio",
                    "equipped Wand",
                    "activated Wand",
                    "sound ended",
                    "explosion hits",
                ];
                let found: Vec<&'static str> = want
                    .iter()
                    .copied()
                    .filter(|w| has(ed, self.r4.mark, w))
                    .collect();
                let hits_ok = ed.log[self.r4.mark.min(ed.log.len())..].iter().any(|l| {
                    l.text.contains("explosion hits") && !l.text.contains("explosion hits 0")
                });
                if (found.len() == want.len() && t > 40) || t > 1200 {
                    let missing: Vec<&str> = want
                        .iter()
                        .copied()
                        .filter(|w| !found.contains(w))
                        .collect();
                    let slots = ed.play.as_ref().map(|s| s.game.slots.len()).unwrap_or(0);
                    ed.stop_play();
                    self.steps.push(Step {
                        step: "game-api",
                        passed: missing.is_empty() && hits_ok,
                        detail: format!(
                            "in Play: ClickDetector MouseClick + MouseHoverEnter with the player, Player.Chatted from the chat box, Tool equipped from hotbar slot 1 ({slots} tool) and Activated by a click, Sound.Ended (silent WAV from the content folder), Explosion.Hit; missing: {missing:?}, explosion hit something: {hits_ok}"
                        ),
                    });
                    self.r4.phase = 10;
                }
            }
            // Round 6: GUI drawing and clicking, an FBX pack, the lobby camera.
            10..=39 => return self.tick_r6(f, ed),
            // Lighting: lit colours against Studio's, measured in an offline Studio.
            40..=59 => return self.tick_light(f, ed),
            // Roblox's default player scripts drive walking, jumping and the camera.
            60..=69 => return self.tick_player(f, ed),
            // The Esc menu frees a locked cursor and Resume takes it back.
            254..=255 => return self.tick_pause_menu(f, ed),
            // Client scripts read the client's own parts' motion.
            244..=245 => return self.tick_client_velocity(f, ed),
            // A shown GuiButton.Modal frees a locked cursor.
            246..=247 => return self.tick_gui_modal(f, ed),
            // Screen-fitted GUIs refit when the 3D view resizes (Place A HUD).
            242..=243 => return self.tick_hud_resize(f, ed),
            // Legacy surfaces, terrain colours and MaterialVariants against Studio.
            70..=71 => return self.tick_material(f, ed),
            250..=251 => return self.tick_union_density(f, ed),
            252..=253 => return self.tick_mesh_density(f, ed),
            // MeshPart collision by CollisionFidelity.
            72..=73 => return self.tick_collision(f, ed),
            // Material texture packs from public asset delivery.
            74..=75 => return self.tick_packs(f, ed),
            // The Animator playing Animate's default animations, R15 and R6.
            76..=79 => return self.tick_anim(f, ed),
            // Studio's own R15 / R6 bodies and layered clothing.
            80..=83 => return self.tick_avatar(f, ed),
            // The session cookie, the rig picker, login-only catalog assets.
            84..=91 => return self.tick_account(f, ed),
            // Place A played in its own process, judged against Studio's capture.
            92..=93 => return self.tick_fidelity(f, ed),
            // Sound: Ended / DidLoop, Playing vs Play(), regions, missing assets.
            94..=95 => return self.tick_sound(f, ed),
            // Instances a script lets go of are freed (a live place power-up weapon leak).
            240..=241 => return self.tick_gc(f, ed),
            // Project folders: untouched round trip, disk ↔ place sync.
            97..=99 => return self.tick_project(f, ed),
            // Studio UI: Q/E camera-relative, keys off the viewport, Ctrl+Shift+V,
            // the Explorer row "+", the run-type drop-down.
            100..=104 => return self.tick_ui(f, ed),
            // Round 17: part drag, live box select, Explorer +, no chat hint,
            // Output format, Properties order.
            105..=110 => return self.tick_ui2(f, ed),
            // Round 18: BrickColor palette, Select Color, property sliders.
            111..=113 => return self.tick_ui3(f, ed),
            // Round 19: Neon bloom falls off smoothly, as Studio's.
            114..=115 => return self.tick_bloom(f, ed),
            // Round 21: the Terrain Editor (Generate, a Draw stroke, Clear, undo).
            116 => return self.tick_terrain_ed(f, ed),
            // Everything has run: the report.
            96 => return self.finish(),
            // og.rbxl (or RUSTUDIO_PERF_PLACE): 10 s of Play, frame times.
            4 => {
                let place = std::env::var_os("RUSTUDIO_PERF_PLACE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        [crate::mech::PERF_PLACE, crate::mech::PERF_PLACE_FALLBACK]
                            .iter()
                            .map(PathBuf::from)
                            .find(|p| p.exists())
                            .unwrap_or_else(|| PathBuf::from(crate::mech::PERF_PLACE))
                    });
                if !place.exists() {
                    self.steps.push(Step {
                        step: "perf",
                        passed: true,
                        detail: format!(
                            "skipped: {} not found (set RUSTUDIO_PERF_PLACE)",
                            place.display()
                        ),
                    });
                    self.r4.phase = 92;
                    return false;
                }
                ed.open_path(place.clone());
                ed.fixed_dt = None;
                ed.start_play(crate::runtime::RunMode::Play);
                self.r4.perf_place = Some(place);
                self.r4.bench = Some(crate::bench::Bench::new(
                    None,
                    "perf",
                    true,
                    crate::mech::SELFTEST_PERF_SECONDS,
                ));
                self.r4.phase = 5;
            }
            5 => {
                let work = ed.last_work_ms;
                let done = self.r4.bench.as_mut().is_some_and(|b| b.frame(work));
                if done {
                    let report = self
                        .r4
                        .bench
                        .as_ref()
                        .map(|b| b.report())
                        .unwrap_or_default();
                    let avg = report["avg_fps"].as_f64().unwrap_or(0.0);
                    let low = report["low1_fps"].as_f64().unwrap_or(0.0);
                    let avg_ms = report["avg_frame_ms"].as_f64().unwrap_or(0.0);
                    let low_ms = report["low1_frame_ms"].as_f64().unwrap_or(0.0);
                    let parts = ed.scene.parts.len();
                    ed.stop_play();
                    ed.fixed_dt = Some(1.0 / 60.0);
                    self.steps.push(Step {
                        step: "perf",
                        passed: avg >= crate::mech::PERF_MIN_FPS as f64,
                        detail: format!(
                            "{}: {parts} parts, Play for {} s at the window size: average {avg_ms:.2} ms/frame ({avg:.0} fps), 1% low {low_ms:.2} ms ({low:.0} fps); target ≥ {} fps; machine load average {} on {} cores",
                            self.r4.perf_place.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                            crate::mech::SELFTEST_PERF_SECONDS,
                            crate::mech::PERF_MIN_FPS,
                            std::fs::read_to_string("/proc/loadavg").ok().and_then(|l| l.split_whitespace().next().map(str::to_string)).unwrap_or_else(|| "?".into()),
                            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
                        ),
                    });
                    // Then the Place A playthrough's result (it ran meanwhile).
                    self.r4.phase = 92;
                    return false;
                }
            }
            _ => {}
        }
        false
    }
}

// ------------------------------------------------------------------ round 6

#[derive(Default)]
struct R6Test {
    started: u64,
    mark: usize,
    notes: Vec<String>,
    ok: bool,
    assets_dir: Option<PathBuf>,
    /// When a step began waiting on background work (downloads).
    t0: Option<std::time::Instant>,
}

/// The GUI test's LocalScript: hooks the buttons and the text box.
const GUI_CLIENT_SRC: &str = r#"local pg = game:GetService("Players").LocalPlayer:WaitForChild("PlayerGui")
local hud = pg:WaitForChild("Hud")
local panel = hud:WaitForChild("Panel")
panel:WaitForChild("Go").Activated:Connect(function() print("gui go clicked") end)
local box = panel:WaitForChild("NameBox")
box.FocusLost:Connect(function(enter) print("gui typed " .. box.Text .. " " .. tostring(enter)) end)
local board = pg:WaitForChild("BoardGui")
board.Adornee = workspace:WaitForChild("Board")
board:WaitForChild("Press").MouseButton1Click:Connect(function() print("gui surface clicked") end)
"#;

/// The lobby-camera test: two camera bindings (the later priority wins) and
/// one that yields, with no character (CharacterAutoLoads off).
const CAMERA_CLIENT_SRC: &str = r#"local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
RunService:BindToRenderStep("Late", Enum.RenderPriority.Camera.Value + 5, function()
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 50
	cam.CFrame = CFrame.lookAt(Vector3.new(0, 10, 50), Vector3.new(0, 0, 0))
end)
RunService:BindToRenderStep("Early", Enum.RenderPriority.Camera.Value + 1, function()
	cam.CameraType = Enum.CameraType.Scriptable
	cam.CFrame = CFrame.new(100, 100, 100)
end)
RunService:BindToRenderStep("Slow", Enum.RenderPriority.Camera.Value, function()
	task.wait(5)
end)
"#;

const FBX_ASSET_ID: u64 = 9_000_000_000_000_001;

impl SelfTest {
    fn tick_r6(&mut self, f: u64, ed: &mut Editor) -> bool {
        use crate::doc::new_part;
        use rbx_dom_weak::types::{Color3, ContentId, Enum, UDim, UDim2, Variant, Vector2};
        let has = |ed: &Editor, mark: usize, needle: &str| {
            ed.log[mark.min(ed.log.len())..]
                .iter()
                .any(|l| l.text.contains(needle))
        };
        let t = f.saturating_sub(self.r6.started);
        // The client's GUI objects by path below PlayerGui.
        let gui_obj = |ed: &Editor, path: &[&str]| -> Option<Ref> {
            let d = &ed.doc;
            let players = d.find_child(d.dom.root_ref(), "Players")?;
            let lp = match d.prop(players, "LocalPlayer") {
                Some(Variant::Ref(r)) => r,
                _ => return None,
            };
            let mut cur = d.find_child(lp, "PlayerGui")?;
            for name in path {
                cur = d
                    .dom
                    .get_by_ref(cur)?
                    .children()
                    .iter()
                    .copied()
                    .find(|c| d.dom.get_by_ref(*c).is_some_and(|i| i.name == *name))?;
            }
            Some(cur)
        };
        let click = |q: &mut Vec<Event>, p: Pos2, pressed: bool| {
            q.push(Event::PointerMoved(p));
            q.push(Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        };
        match self.r4.phase {
            // GUI: a ScreenGui panel (list layout, padding, corner, scaled
            // text, image, button, text box) and a SurfaceGui on a part.
            10 => {
                // From here Play opens on the client's view, as in Studio.
                ed.play_shows_client = true;
                ed.set_document(Document::new_place());
                let root = ed.doc.dom.root_ref();
                let ws = ed.doc.workspace().expect("workspace");
                let sg = ed.doc.find_child(root, "StarterGui").expect("starter gui");
                let sp = ed
                    .doc
                    .find_child(root, "StarterPlayer")
                    .expect("starter player");
                let sps = ed
                    .doc
                    .find_child(sp, "StarterPlayerScripts")
                    .unwrap_or_else(|| {
                        ed.doc.dom.insert(
                            sp,
                            InstanceBuilder::new("StarterPlayerScripts")
                                .with_name("StarterPlayerScripts"),
                        )
                    });
                let u2 = |xs: f32, xo: i32, ys: f32, yo: i32| {
                    UDim2::new(UDim::new(xs, xo), UDim::new(ys, yo))
                };
                ed.doc.insert(
                    ws,
                    new_part(
                        "Board",
                        1,
                        Vec3::new(0.0, 8.0, -30.0),
                        Vec3::new(12.0, 6.0, 0.5),
                        [40, 40, 60],
                    ),
                );
                ed.doc.insert(
                    sg,
                    InstanceBuilder::new("ScreenGui")
                        .with_name("Hud")
                        .with_property("IgnoreGuiInset", true)
                        .with_child(
                            InstanceBuilder::new("Frame")
                                .with_name("Panel")
                                .with_property("Size", u2(0.0, 300, 0.0, 220))
                                .with_property("Position", u2(0.0, 20, 0.0, 80))
                                .with_property("BackgroundColor3", Color3::new(0.15, 0.15, 0.2))
                                .with_child(
                                    InstanceBuilder::new("UIListLayout")
                                        .with_property("Padding", UDim::new(0.0, 8))
                                        .with_property("SortOrder", Enum::from_u32(2)),
                                )
                                .with_child(
                                    InstanceBuilder::new("UIPadding")
                                        .with_property("PaddingLeft", UDim::new(0.0, 10))
                                        .with_property("PaddingTop", UDim::new(0.0, 10)),
                                )
                                .with_child(
                                    InstanceBuilder::new("UICorner")
                                        .with_property("CornerRadius", UDim::new(0.0, 8)),
                                )
                                .with_child(
                                    InstanceBuilder::new("TextLabel")
                                        .with_name("Title")
                                        .with_property("LayoutOrder", 1i32)
                                        .with_property("Size", u2(1.0, -20, 0.0, 30))
                                        .with_property("Text", "Hello GUI".to_string())
                                        .with_property("TextScaled", true)
                                        .with_property("BackgroundTransparency", 1.0f32),
                                )
                                .with_child(
                                    InstanceBuilder::new("TextButton")
                                        .with_name("Go")
                                        .with_property("LayoutOrder", 2i32)
                                        .with_property("Size", u2(0.0, 120, 0.0, 40))
                                        .with_property("Text", "Go".to_string())
                                        .with_property("TextSize", 20.0f32),
                                )
                                .with_child(
                                    InstanceBuilder::new("TextBox")
                                        .with_name("NameBox")
                                        .with_property("LayoutOrder", 3i32)
                                        .with_property("Size", u2(1.0, -20, 0.0, 30))
                                        .with_property("Text", String::new())
                                        .with_property("PlaceholderText", "type here".to_string()),
                                )
                                .with_child(
                                    InstanceBuilder::new("ImageLabel")
                                        .with_name("Pic")
                                        .with_property("LayoutOrder", 4i32)
                                        .with_property("Size", u2(0.0, 48, 0.0, 48))
                                        .with_property(
                                            "Image",
                                            ContentId::from("rbxasset://textures/selftest.png"),
                                        ),
                                ),
                        ),
                );
                ed.doc.insert(
                    sg,
                    InstanceBuilder::new("SurfaceGui")
                        .with_name("BoardGui")
                        .with_property("Face", Enum::from_u32(2))
                        .with_property("CanvasSize", Vector2::new(400.0, 200.0))
                        .with_property("AlwaysOnTop", true)
                        .with_child(
                            InstanceBuilder::new("TextButton")
                                .with_name("Press")
                                .with_property("Size", u2(1.0, 0, 1.0, 0))
                                .with_property("Text", "Press me".to_string())
                                .with_property("TextScaled", true),
                        ),
                );
                ed.doc.insert(
                    sps,
                    InstanceBuilder::new("LocalScript")
                        .with_name("GuiTest")
                        .with_property("Source", GUI_CLIENT_SRC.to_string()),
                );
                // (GUI events are logged too, for the step's detail if it fails.)
                std::env::set_var("RUSTUDIO_GUI_DEBUG", "1");
                self.r6 = R6Test {
                    started: f,
                    mark: ed.log.len(),
                    ..Default::default()
                };
                ed.start_play(crate::runtime::RunMode::Play);
                self.r4.phase = 11;
            }
            11 => match t {
                60 => {
                    // Layout as Roblox computes it: inside the padding, stacked by the list.
                    let (panel, title, go) = (
                        gui_obj(ed, &["Hud", "Panel"]),
                        gui_obj(ed, &["Hud", "Panel", "Title"]),
                        gui_obj(ed, &["Hud", "Panel", "Go"]),
                    );
                    let lay = |r: Option<Ref>| r.and_then(|r| ed.gui.layout.get(&r).copied());
                    let inset = crate::mech::GUI_TOP_INSET;
                    let want_go = [30.0, 80.0 - inset + 10.0 + 30.0 + 8.0];
                    match (lay(panel), lay(title), lay(go)) {
                        (Some(p), Some(ti), Some(g)) => {
                            let ok = (p.pos[0] - 20.0).abs() < 0.5
                                && (p.pos[1] - (80.0 - inset)).abs() < 0.5
                                && (g.pos[0] - want_go[0]).abs() < 0.5
                                && (g.pos[1] - want_go[1]).abs() < 0.5
                                && (g.size[0] - 120.0).abs() < 0.5
                                && ti.text.is_some_and(|tb| tb[0] > 10.0);
                            self.r6.ok = ok;
                            self.r6.notes.push(format!(
                                "panel at ({:.0}, {:.0}) {:.0}×{:.0}; Go button at ({:.0}, {:.0}) {:.0}×{:.0} (expected ({}, {}) 120×40 from the padding and list layout); title TextScaled bounds {:?}; {} shapes drawn",
                                p.pos[0], p.pos[1], p.size[0], p.size[1], g.pos[0], g.pos[1], g.size[0], g.size[1], want_go[0], want_go[1], ti.text.map(|t| [t[0].round(), t[1].round()]), ed.gui.drawn
                            ));
                        }
                        other => {
                            self.r6.ok = false;
                            self.r6.notes.push(format!(
                                "GUI not laid out: {:?}",
                                (other.0.is_some(), other.1.is_some(), other.2.is_some())
                            ));
                        }
                    }
                    if let Some(p) = go.and_then(|g| ed.gui.screen_center(g)) {
                        click(&mut self.queue, p, true);
                    }
                }
                62 => {
                    if let Some(p) =
                        gui_obj(ed, &["Hud", "Panel", "Go"]).and_then(|g| ed.gui.screen_center(g))
                    {
                        click(&mut self.queue, p, false);
                    }
                }
                70 | 72 => {
                    if let Some(p) = gui_obj(ed, &["Hud", "Panel", "NameBox"])
                        .and_then(|g| ed.gui.screen_center(g))
                    {
                        click(&mut self.queue, p, t == 70);
                    }
                }
                74 => self.queue.push(Event::Text("hi".into())),
                76 => {
                    self.queue.push(key(Key::Enter, true, Modifiers::NONE));
                    self.queue.push(key(Key::Enter, false, Modifiers::NONE));
                }
                80 | 82 => {
                    if let Some(p) =
                        gui_obj(ed, &["BoardGui", "Press"]).and_then(|g| ed.gui.screen_center(g))
                    {
                        click(&mut self.queue, p, t == 80);
                    } else if t == 80 {
                        self.r6
                            .notes
                            .push("the SurfaceGui button wasn't drawn (board out of view?)".into());
                    }
                }
                120 => {
                    let want = ["gui go clicked", "gui typed hi true", "gui surface clicked"];
                    let missing: Vec<&str> = want
                        .iter()
                        .copied()
                        .filter(|w| !has(ed, self.r6.mark, w))
                        .collect();
                    if !missing.is_empty() {
                        let lines: Vec<String> = ed.log[self.r6.mark.min(ed.log.len())..]
                            .iter()
                            .filter(|l| {
                                (l.text.contains("[gui]") && !l.text.contains("Input"))
                                    || l.text.contains("gui typed")
                                    || l.level == crate::app::Level::Error
                            })
                            .map(|l| l.text.clone())
                            .take(30)
                            .collect();
                        self.r6.notes.push(format!("log: {lines:?}"));
                    }
                    std::env::remove_var("RUSTUDIO_GUI_DEBUG");
                    ed.stop_play();
                    self.steps.push(Step {
                        step: "gui",
                        passed: self.r6.ok && missing.is_empty(),
                        detail: format!("{}; clicked the Go button, typed into the TextBox and pressed Enter, clicked a SurfaceGui button on a part; missing: {missing:?}", self.r6.notes.join("; ")),
                    });
                    self.r4.phase = 20;
                }
                _ => {}
            },
            // FBX pack: InsertService:LoadAsset on an FBX in the local asset store.
            20 => {
                let dir = std::env::temp_dir().join("rustudio-selftest-assets");
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(
                    dir.join(format!("{FBX_ASSET_ID}.fbx")),
                    crate::fbx::sample([200.0, 100.0, 300.0], [500.0, 0.0, 0.0]),
                );
                std::env::set_var("RUSTUDIO_LOCAL_ASSETS", &dir);
                ed.set_document(Document::new_place());
                let root = ed.doc.dom.root_ref();
                let sss = ed.doc.find_child(root, "ServerScriptService").expect("sss");
                let src = format!(
                    r#"local m = game:GetService("InsertService"):LoadAsset({FBX_ASSET_ID})
m.Parent = workspace
for _, d in m:GetDescendants() do
	if d:IsA("MeshPart") then
		d.Position += Vector3.new(0, 4, -12)
		print(string.format("fbx part %s in %s %.2f %.2f %.2f", d.Name, d.Parent.Name, d.Size.X, d.Size.Y, d.Size.Z))
	end
end"#
                );
                ed.doc.insert(
                    sss,
                    InstanceBuilder::new("Script")
                        .with_name("LoadPack")
                        .with_property("Source", src),
                );
                self.r6 = R6Test {
                    started: f,
                    mark: ed.log.len(),
                    assets_dir: Some(dir),
                    ..Default::default()
                };
                ed.start_play(crate::runtime::RunMode::Run);
                self.r4.phase = 21;
            }
            21 if t >= 30 => {
                let [meshes, ..] = ed.draw_stats;
                let line = ed.log[self.r6.mark.min(ed.log.len())..]
                    .iter()
                    .find(|l| l.text.contains("fbx part"))
                    .map(|l| l.text.clone());
                let ok = line
                    .as_deref()
                    .is_some_and(|l| l.contains("fbx part Crate in Pack 2.00 1.00 3.00"))
                    && meshes >= 1;
                ed.stop_play();
                std::env::remove_var("RUSTUDIO_LOCAL_ASSETS");
                self.steps.push(Step {
                    step: "fbx-pack",
                    passed: ok,
                    detail: format!(
                        "InsertService:LoadAsset on a binary FBX in the local asset store ({}): {}; {meshes} mesh drawn",
                        self.r6.assets_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default(),
                        line.unwrap_or_else(|| "no MeshPart came out".into())
                    ),
                });
                self.r4.phase = 30;
            }
            // Lobby camera: no character, the render-step bindings in priority order.
            30 => {
                ed.set_document(Document::new_place());
                let root = ed.doc.dom.root_ref();
                if let Some(players) = ed.doc.find_child(root, "Players") {
                    if let Some(i) = ed.doc.dom.get_by_ref_mut(players) {
                        i.properties.insert(
                            rbx_dom_weak::ustr("CharacterAutoLoads"),
                            Variant::Bool(false),
                        );
                    }
                }
                let sp = ed
                    .doc
                    .find_child(root, "StarterPlayer")
                    .expect("starter player");
                let sps = ed
                    .doc
                    .find_child(sp, "StarterPlayerScripts")
                    .unwrap_or_else(|| {
                        ed.doc.dom.insert(
                            sp,
                            InstanceBuilder::new("StarterPlayerScripts")
                                .with_name("StarterPlayerScripts"),
                        )
                    });
                ed.doc.insert(
                    sps,
                    InstanceBuilder::new("LocalScript")
                        .with_name("LobbyCamera")
                        .with_property("Source", CAMERA_CLIENT_SRC.to_string()),
                );
                self.r6 = R6Test {
                    started: f,
                    mark: ed.log.len(),
                    ..Default::default()
                };
                ed.start_play(crate::runtime::RunMode::Play);
                self.r4.phase = 31;
            }
            31 if t >= 40 => {
                let character = ed.play.as_ref().is_some_and(|s| s.character.is_some());
                let pos = ed.cam.pos;
                let look = ed.cam.forward();
                let want_look = (Vec3::ZERO - Vec3::new(0.0, 10.0, 50.0)).normalize();
                let fov = ed.cam.fov_deg;
                let ok = !character
                    && (pos - Vec3::new(0.0, 10.0, 50.0)).length() < 0.01
                    && look.dot(want_look) > 0.999
                    && (fov - 50.0).abs() < 0.01;
                let errors = ed.log[self.r6.mark.min(ed.log.len())..]
                    .iter()
                    .filter(|l| l.level == crate::app::Level::Error)
                    .count();
                let first_error = ed.log[self.r6.mark.min(ed.log.len())..]
                    .iter()
                    .find(|l| l.level == crate::app::Level::Error)
                    .map(|l| l.text.clone())
                    .unwrap_or_default();
                ed.stop_play();
                self.steps.push(Step {
                    step: "lobby-camera",
                    passed: ok && errors == 0,
                    detail: format!(
                        "CharacterAutoLoads off → character: {character}; camera at ({:.1}, {:.1}, {:.1}) looking ({:.2}, {:.2}, {:.2}), FOV {fov:.0} (the Camera+5 binding wins over Camera+1; a yielding binding doesn't hold the others up); {errors} errors {first_error}",
                        pos.x, pos.y, pos.z, look.x, look.y, look.z
                    ),
                });
                self.r4.phase = 40;
            }
            _ => {}
        }
        false
    }
}
