//! Self-test steps for round 16's Studio UI behaviours:
//!
//! - `camera-qe-relative`: E moves the camera along its own up axis (Studio,
//!   measured: E went along CurrentCamera.CFrame.UpVector), not world up;
//! - `camera-keys-anywhere`: W flies the camera with the mouse over the
//!   Explorer, and does nothing while a text field has the keyboard;
//! - `paste-into-shortcut`: Ctrl+Shift+V pastes into the selection, both as
//!   egui-winit delivers it from a real keyboard (a Paste event with Shift
//!   held) and as the key itself;
//! - `explorer-plus-insert`: the "+" on a hovered Explorer row opens Insert
//!   Object for that object; typing a class and Enter inserts it there;
//! - `run-type-dropdown`: picking Run in the test drop-down makes the Play
//!   button start Run, Stop stops it, and Test Here starts Play at the camera.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use rbx_dom_weak::types::Ref;
use rbx_dom_weak::InstanceBuilder;

use super::{key, SelfTest, Step};
use crate::app::Editor;
use crate::runtime::RunMode;
use crate::studio_ui::{run_type_key, RunType};

pub(super) struct UiTest {
    started: u64,
    cam0: glam::Vec3,
    up0: glam::Vec3,
    notes: Vec<String>,
    ok: bool,
    folder: Ref,
    part: Ref,
    moved_free: f32,
}

impl Default for UiTest {
    fn default() -> Self {
        UiTest {
            started: 0,
            cam0: glam::Vec3::ZERO,
            up0: glam::Vec3::ZERO,
            notes: Vec::new(),
            ok: false,
            folder: Ref::none(),
            part: Ref::none(),
            moved_free: 0.0,
        }
    }
}

fn press(q: &mut Vec<Event>, p: Pos2, down: bool) {
    q.push(Event::PointerMoved(p));
    q.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
}

fn children_named(ed: &Editor, parent: Ref, name: &str) -> usize {
    ed.doc
        .dom
        .get_by_ref(parent)
        .map(|i| i.children().iter().filter(|c| ed.doc.dom.get_by_ref(**c).is_some_and(|k| k.name == name)).count())
        .unwrap_or(0)
}

impl SelfTest {
    /// A point over the Explorer panel (off the viewport).
    fn off_viewport(ed: &Editor) -> Pos2 {
        ed.ui_rects
            .get("explorer.search")
            .map(|r| r.center() + egui::vec2(0.0, 80.0))
            .unwrap_or(Pos2::new(ed.viewport_rect.max.x + 60.0, ed.viewport_rect.center().y))
    }

    pub(super) fn tick_ui(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.ui.started);
        let phase = self.r4.phase;
        match (phase, t) {
            // ------------------------------------------------ Q / E camera-relative
            (100, _) if self.ui.started == 0 => {
                if ed.play.is_some() {
                    ed.stop_play();
                }
                ed.new_place();
                self.ui = UiTest { started: f, ok: true, ..Default::default() };
            }
            (100, 3) => {
                // Looking down at 40°: camera up is far from world up.
                ed.cam.yaw = 0.3;
                ed.cam.pitch = -0.7;
                ed.cam.focus = ed.cam.pos + ed.cam.forward() * 20.0;
                self.ui.cam0 = ed.cam.pos;
                self.ui.up0 = ed.cam.up();
                let p = Self::off_viewport(ed);
                self.queue.push(Event::PointerMoved(p));
                self.queue.push(key(Key::E, true, Modifiers::NONE));
            }
            (100, 13) => self.queue.push(key(Key::E, false, Modifiers::NONE)),
            (100, 16) => {
                let d = ed.cam.pos - self.ui.cam0;
                let dir = d.normalize_or_zero();
                let along = dir.dot(self.ui.up0);
                let world = dir.dot(glam::Vec3::Y);
                let passed = d.length() > 0.3 && along > 0.99 && world < 0.95;
                self.steps.push(Step {
                    step: "camera-qe-relative",
                    passed,
                    detail: format!(
                        "E held 10 frames with the mouse over the Explorer, camera pitched -40°: moved {:.2} studs, along the camera's up {along:.3}, along world up {world:.3} (Studio: along CFrame.UpVector)",
                        d.length()
                    ),
                });
                self.r4.phase = 101;
                self.ui.started = f;
            }
            // --------------------------------- WASD off the viewport, not while typing
            (101, 1) => {
                self.ui.cam0 = ed.cam.pos;
                let p = Self::off_viewport(ed);
                self.queue.push(Event::PointerMoved(p));
                self.queue.push(key(Key::W, true, Modifiers::NONE));
            }
            (101, 9) => self.queue.push(key(Key::W, false, Modifiers::NONE)),
            (101, 11) => {
                self.ui.moved_free = (ed.cam.pos - self.ui.cam0).length();
                // Focus the Explorer's search field.
                match ed.ui_rects.get("explorer.search").map(|r| r.center()) {
                    Some(c) => press(&mut self.queue, c, true),
                    None => self.ui.notes.push("no Explorer search field".into()),
                }
            }
            (101, 12) => {
                let c = ed.ui_rects.get("explorer.search").map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, false);
            }
            (101, 15) => {
                self.ui.cam0 = ed.cam.pos;
                self.queue.push(key(Key::W, true, Modifiers::NONE));
                self.queue.push(Event::Text("w".into()));
            }
            (101, 23) => self.queue.push(key(Key::W, false, Modifiers::NONE)),
            (101, 25) => {
                let typing_moved = (ed.cam.pos - self.ui.cam0).length();
                let typed = ed.explorer.filter.contains('w');
                let passed = self.ui.moved_free > 0.3 && typing_moved < 1e-4 && typed;
                self.steps.push(Step {
                    step: "camera-keys-anywhere",
                    passed,
                    detail: format!(
                        "W with the mouse over the Explorer flew {:.2} studs; W while the search field had the keyboard: camera moved {typing_moved:.4}, the field got the text: {typed}{}",
                        self.ui.moved_free,
                        if self.ui.notes.is_empty() { String::new() } else { format!(" ({})", self.ui.notes.join("; ")) }
                    ),
                });
                self.queue.push(key(Key::Escape, true, Modifiers::NONE));
                self.queue.push(key(Key::Escape, false, Modifiers::NONE));
                ed.explorer.filter.clear();
                self.ui.notes.clear();
                self.r4.phase = 102;
                self.ui.started = f;
            }
            // ------------------------------------------------------- Ctrl+Shift+V
            (102, 1) => {
                let ws = ed.doc.default_parent();
                self.ui.folder = ed.doc.insert(ws, InstanceBuilder::new("Folder").with_name("PasteTarget"));
                self.ui.part = ed.doc.insert(ws, InstanceBuilder::new("Part").with_name("PasteMe"));
                ed.select(vec![self.ui.part]);
                ed.run_action("copy");
                ed.select(vec![self.ui.folder]);
            }
            (102, 3) => {
                // A real keyboard: egui-winit sends Ctrl+Shift+V as a Paste event.
                let m = Modifiers::COMMAND | Modifiers::SHIFT;
                self.modifiers = m;
                self.queue.push(Event::ModifiersChanged(m));
                self.queue.push(Event::Paste("rustudio".into()));
            }
            (102, 5) => {
                self.modifiers = Modifiers::NONE;
                self.queue.push(Event::ModifiersChanged(Modifiers::NONE));
            }
            (102, 7) => {
                self.ui.notes.push(format!("paste event: {} inside", children_named(ed, self.ui.folder, "PasteMe")));
                ed.select(vec![self.ui.folder]);
                self.press_combo(Modifiers::COMMAND | Modifiers::SHIFT, Key::V);
            }
            (102, 10) => {
                let n = children_named(ed, self.ui.folder, "PasteMe");
                self.steps.push(Step {
                    step: "paste-into-shortcut",
                    passed: n == 2,
                    detail: format!(
                        "a part copied, the folder selected, Ctrl+Shift+V twice (as a Paste event with Shift, and as the key): {}; {n} copies inside the folder (want 2)",
                        self.ui.notes.join(", ")
                    ),
                });
                self.ui.notes.clear();
                ed.explorer.expanded.insert(ed.doc.default_parent());
                ed.doc.selection.clear();
                self.r4.phase = 103;
                self.ui.started = f;
            }
            // ------------------------------------------------- Explorer row "+"
            (103, 2) => {
                ed.explorer.scroll_to = Some(self.ui.folder);
            }
            (103, 5) => match ed.explorer.row_rects.iter().find(|(r, _)| *r == self.ui.folder) {
                Some((_, rect)) => self.queue.push(Event::PointerMoved(rect.center())),
                None => self.ui.notes.push("folder row not drawn".into()),
            },
            (103, 7) => match ed.explorer.plus_rect.filter(|(r, _)| *r == self.ui.folder) {
                Some((_, p)) => press(&mut self.queue, p.center(), true),
                None => self.ui.notes.push("no + on the hovered row".into()),
            },
            (103, 8) => {
                let p = ed.explorer.plus_rect.map(|(_, r)| r.center()).unwrap_or_default();
                press(&mut self.queue, p, false);
            }
            (103, 12) => {
                let open = ed.ui.insert_popup.as_ref().map(|p| p.parent) == Some(self.ui.folder);
                self.ui.notes.push(format!("popup for the folder: {open}"));
                self.queue.push(Event::Text("PointLight".into()));
            }
            (103, 16) => {
                self.queue.push(key(Key::Enter, true, Modifiers::NONE));
                self.queue.push(key(Key::Enter, false, Modifiers::NONE));
            }
            (103, 20) => {
                let n = ed
                    .doc
                    .dom
                    .get_by_ref(self.ui.folder)
                    .map(|i| i.children().iter().filter(|c| ed.doc.class_of(**c).as_deref() == Some("PointLight")).count())
                    .unwrap_or(0);
                self.steps.push(Step {
                    step: "explorer-plus-insert",
                    passed: n == 1 && ed.ui.insert_popup.is_none(),
                    detail: format!("hovered the folder's row, clicked its +, typed PointLight, Enter: {}; PointLights in the folder: {n}", self.ui.notes.join(", ")),
                });
                self.ui.notes.clear();
                self.r4.phase = 104;
                self.ui.started = f;
            }
            // ------------------------------------------------- run-type drop-down
            (104, 1) => match ed.ui_rects.get("test.type").map(|r| r.center()) {
                Some(c) => press(&mut self.queue, c, true),
                None => self.ui.notes.push("no run-type drop-down".into()),
            },
            (104, 2) => {
                let c = ed.ui_rects.get("test.type").map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, false);
            }
            (104, 6) => match ed.ui_rects.get(run_type_key(RunType::Run)).map(|r| r.center()) {
                Some(c) => press(&mut self.queue, c, true),
                None => self.ui.notes.push("Run not listed".into()),
            },
            (104, 7) => {
                let c = ed.ui_rects.get(run_type_key(RunType::Run)).map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, false);
            }
            (104, 10) => {
                self.ui.notes.push(format!("picked: {}", ed.ui.run_type.label()));
                let c = ed.ui_rects.get("test.play").map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, true);
            }
            (104, 11) => {
                let c = ed.ui_rects.get("test.play").map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, false);
            }
            (104, 16) => {
                let mode = ed.play.as_ref().map(|p| p.mode);
                self.ui.ok = mode == Some(RunMode::Run);
                self.ui.notes.push(format!("Play button started: {:?}", mode));
                let c = ed.ui_rects.get("test.stop").map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, true);
            }
            (104, 17) => {
                let c = ed.ui_rects.get("test.stop").map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, false);
            }
            (104, 21) => {
                let stopped = ed.play.is_none();
                self.ui.ok &= stopped;
                self.ui.notes.push(format!("Stop stopped it: {stopped}"));
                // Test Here: Play with the character at the camera.
                ed.ui.run_type = RunType::TestHere;
                self.ui.cam0 = ed.cam.pos;
                let c = ed.ui_rects.get("test.play").map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, true);
            }
            (104, 22) => {
                let c = ed.ui_rects.get("test.play").map(|r| r.center()).unwrap_or_default();
                press(&mut self.queue, c, false);
            }
            (104, 26) => {
                let mode = ed.play.as_ref().map(|p| p.mode);
                let here = ed.play_here == Some(self.ui.cam0);
                self.ui.ok &= mode == Some(RunMode::Play) && here;
                self.ui.notes.push(format!("Test Here started {:?} with the spawn at the camera: {here}", mode));
                ed.stop_play();
                ed.ui.run_type = RunType::Test;
                self.steps.push(Step {
                    step: "run-type-dropdown",
                    passed: self.ui.ok,
                    detail: self.ui.notes.join("; "),
                });
                ed.new_place();
                self.r4.phase = 105;
            }
            _ => {}
        }
        false
    }
}
