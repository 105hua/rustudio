//! Self-test step for Roblox's default player scripts in Play: the
//! PlayerModule (ControlModule + CameraModule), Animate and Health come from
//! the Studio install and drive the character and camera. Expected numbers
//! were measured in offline Studio (docs/MECHANICS.md, "Player controller").

use egui::{Event, Key, Modifiers, PointerButton, Vec2};
use glam::Vec3;
use rbx_dom_weak::types::{Ref, Variant};

use super::{key, SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;
use crate::runtime::Side;

/// What the step has seen so far.
#[derive(Default)]
pub(super) struct PlayerTest {
    started: u64,
    notes: Vec<String>,
    ok: bool,
    start: Option<Vec3>,
    ground: f32,
    top: f32,
    yaw0: f32,
    /// Output lines before Play started.
    mark: usize,
}

/// The client's DataModel (shown, or waiting in other_doc).
fn client_doc(ed: &Editor) -> Option<&Document> {
    let s = ed.play.as_ref()?;
    if s.showing == Side::Client {
        Some(&ed.doc)
    } else {
        ed.play_other_doc()
    }
}

fn child(d: &Document, parent: Ref, name: &str) -> Option<Ref> {
    crate::default_scripts::child_named(d, parent, name)
}

/// (camera-to-focus distance, yaw in degrees) of the client's CurrentCamera.
fn client_camera(ed: &Editor) -> Option<(f32, f32)> {
    let d = client_doc(ed)?;
    let ws = d.workspace()?;
    let cam = match d.prop(ws, "CurrentCamera") {
        Some(Variant::Ref(c)) => c,
        _ => return None,
    };
    let cf = crate::scene::xform_of(d, cam)?;
    let focus = match d.prop(cam, "Focus") {
        Some(Variant::CFrame(f)) => Vec3::new(f.position.x, f.position.y, f.position.z),
        _ => return None,
    };
    let look = -cf.rot.z_axis;
    Some((
        (cf.pos - focus).length(),
        (-look.x).atan2(-look.z).to_degrees(),
    ))
}

fn character_pos(ed: &Editor) -> Option<Vec3> {
    ed.play.as_ref()?.character.as_ref().map(|c| c.body.pos)
}

impl SelfTest {
    /// Phases 60..=69 (68–69: the game-lock step).
    pub(super) fn tick_player(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.pm.started);
        let vp = ed.viewport_rect;
        match self.r4.phase {
            60 => {
                ed.set_document(Document::new_place());
                ed.start_play(crate::runtime::RunMode::Play);
                self.pm = PlayerTest {
                    started: f,
                    ok: true,
                    mark: ed.log.len(),
                    ..Default::default()
                };
                self.r4.phase = 61;
            }
            // Scripts in place; the camera starts where Studio's does.
            61 if t >= 40 => {
                let (scripts, char_scripts) = match client_doc(ed) {
                    Some(d) => {
                        let root = d.dom.root_ref();
                        let ps = d
                            .find_child(root, "Players")
                            .and_then(|p| {
                                d.dom
                                    .get_by_ref(p)
                                    .and_then(|i| i.children().first().copied())
                            })
                            .and_then(|pl| child(d, pl, "PlayerScripts"));
                        let names = |r: Option<Ref>, want: &[&str]| {
                            want.iter()
                                .all(|n| r.is_some_and(|r| child(d, r, n).is_some()))
                        };
                        let ch = d
                            .workspace()
                            .and_then(|ws| child(d, ws, &ed.player_name()));
                        (
                            names(
                                ps,
                                &["PlayerModule", "PlayerScriptsLoader", "RbxCharacterSounds"],
                            ),
                            names(ch, &["Animate", "Health"]),
                        )
                    }
                    None => (false, false),
                };
                let installed = crate::default_scripts::studio_dir().is_some();
                let cam = client_camera(ed);
                let errors = ed.log[self.pm.mark.min(ed.log.len())..]
                    .iter()
                    .filter(|l| matches!(l.level, crate::app::Level::Error))
                    .count();
                let start_ok = cam.is_some_and(|(d, _)| (d - 12.5).abs() < 0.1);
                self.pm.ok &= (!installed || (scripts && char_scripts)) && start_ok;
                self.pm.notes.push(format!(
                    "PlayerModule, PlayerScriptsLoader, RbxCharacterSounds in PlayerScripts: {scripts}; Animate, Health in the character: {char_scripts} (Studio install found: {installed}); camera starts {:.2} studs from its focus (Studio: 12.5); {errors} errors",
                    cam.map(|c| c.0).unwrap_or(0.0)
                ));
                self.pm.start = character_pos(ed);
                self.queue.push(key(Key::W, true, Modifiers::NONE));
                self.r4.phase = 62;
                self.pm.started = f;
            }
            // Walking: W held 1 s.
            62 if t >= 60 => {
                self.queue.push(key(Key::W, false, Modifiers::NONE));
                let walked = match (self.pm.start, character_pos(ed)) {
                    (Some(a), Some(b)) => Vec3::new(b.x - a.x, 0.0, b.z - a.z).length(),
                    _ => 0.0,
                };
                self.pm.ok &= (walked - 16.0).abs() < 1.5;
                self.pm.notes.push(format!("W for 1 s through the ControlModule walked {walked:.1} studs (Studio: 16 studs/s)"));
                self.r4.phase = 63;
                self.pm.started = f;
            }
            63 if t == 10 => {
                let y = character_pos(ed).map(|p| p.y).unwrap_or(0.0);
                self.pm.ground = y;
                self.pm.top = y;
                self.queue.push(key(Key::Space, true, Modifiers::NONE));
            }
            63 if t == 16 => self.queue.push(key(Key::Space, false, Modifiers::NONE)),
            63 if t < 60 => {
                if let Some(p) = character_pos(ed) {
                    self.pm.top = self.pm.top.max(p.y);
                }
            }
            // Jump done: wheel 3 notches out.
            63 => {
                let rise = self.pm.top - self.pm.ground;
                self.pm.ok &= (rise - 7.27).abs() < 0.5;
                self.pm.notes.push(format!(
                    "Space jumped {rise:.2} studs (Studio: 7.27 at JumpPower 50)"
                ));
                self.queue.push(Event::PointerMoved(vp.center()));
                self.queue.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: Vec2::new(0.0, -3.0),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                });
                self.r4.phase = 64;
                self.pm.started = f;
            }
            // Zoomed: then right-drag 200 px to the right.
            64 if t >= 90 => {
                let (dist, yaw) = client_camera(ed).unwrap_or((0.0, 0.0));
                self.pm.ok &= (dist - 23.375).abs() < 0.1;
                self.pm.notes.push(format!(
                    "3 wheel notches out: {dist:.3} studs (Studio: 23.375)"
                ));
                self.pm.yaw0 = yaw;
                self.queue.push(Event::PointerMoved(vp.center()));
                self.queue.push(Event::PointerButton {
                    pos: vp.center(),
                    button: PointerButton::Secondary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                });
                self.r4.phase = 65;
                self.pm.started = f;
            }
            65 if (4..14).contains(&t) => self.queue.push(Event::MouseMoved(Vec2::new(20.0, 0.0))),
            65 if t == 16 => {
                let grabbed = ed.play.as_ref().is_some_and(|s| s.mouse_behavior == 2);
                self.pm.notes.push(format!(
                    "right button held: MouseBehavior LockCurrentPosition {grabbed}"
                ));
                self.pm.ok &= grabbed;
                self.queue.push(Event::PointerButton {
                    pos: vp.center(),
                    button: PointerButton::Secondary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                });
            }
            65 if t >= 30 => {
                let (_, yaw) = client_camera(ed).unwrap_or((0.0, 0.0));
                let turned = (yaw - self.pm.yaw0 + 540.0).rem_euclid(360.0) - 180.0;
                // CameraInput: 0.5° per pixel of mouse motion.
                self.pm.ok &= (turned + 100.0).abs() < 2.0;
                self.pm.notes.push(format!(
                    "right-drag 200 px turned the camera {turned:.1}° (CameraInput: −100°)"
                ));
                self.queue.push(Event::ModifiersChanged(Modifiers::SHIFT));
                self.r4.phase = 66;
                self.pm.started = f;
            }
            // Shift-lock on.
            66 if t == 4 => self.queue.push(Event::ModifiersChanged(Modifiers::NONE)),
            66 if t >= 20 => {
                let (mb, rel) = ed
                    .play
                    .as_ref()
                    .map(|s| (s.mouse_behavior, s.camera_relative))
                    .unwrap_or((0, false));
                let on = mb == 1 && rel;
                self.pm.ok &= on;
                self.pm.notes.push(format!("Shift: shift-lock on (MouseBehavior LockCenter, RotationType CameraRelative): {on}"));
                self.queue.push(Event::ModifiersChanged(Modifiers::SHIFT));
                self.r4.phase = 67;
                self.pm.started = f;
            }
            // Shift-lock off, then zoom all the way in.
            67 if t == 4 => self.queue.push(Event::ModifiersChanged(Modifiers::NONE)),
            67 if t == 20 => {
                let off = ed
                    .play
                    .as_ref()
                    .is_some_and(|s| s.mouse_behavior == 0 && !s.camera_relative);
                self.pm.ok &= off;
                self.pm.notes.push(format!("Shift again: off {off}"));
                self.queue.push(Event::PointerMoved(vp.center()));
                self.queue.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: Vec2::new(0.0, 40.0),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                });
            }
            67 if t >= 110 => {
                let (dist, _) = client_camera(ed).unwrap_or((99.0, 0.0));
                let (mb, rel) = ed
                    .play
                    .as_ref()
                    .map(|s| (s.mouse_behavior, s.camera_relative))
                    .unwrap_or((0, false));
                let head_hidden = client_doc(ed)
                    .and_then(|d| {
                        d.workspace()
                            .and_then(|ws| child(d, ws, &ed.player_name()))
                            .and_then(|c| child(d, c, "Head"))
                            .map(|h| {
                                crate::scene::f32_of(d.dom.get_by_ref(h).and_then(|i| {
                                    i.properties
                                        .get(&rbx_dom_weak::ustr("LocalTransparencyModifier"))
                                        .cloned()
                                }))
                            })
                    })
                    .flatten()
                    .unwrap_or(0.0)
                    > 0.5;
                let fp = dist < 1.0 && mb == 1 && rel && head_hidden;
                self.pm.ok &= fp;
                self.pm.notes.push(format!(
                    "wheel all the way in: first person {fp} (distance {dist:.2}, LockCenter {}, CameraRelative {rel}, head hidden {head_hidden})",
                    mb == 1
                ));
                let errors: Vec<String> = ed.log[self.pm.mark.min(ed.log.len())..]
                    .iter()
                    .filter(|l| matches!(l.level, crate::app::Level::Error))
                    .map(|l| l.text.chars().take(160).collect())
                    .take(3)
                    .collect();
                self.pm.ok &= errors.is_empty();
                if !errors.is_empty() {
                    self.pm.notes.push(format!("errors: {errors:?}"));
                }
                self.steps.push(Step {
                    step: "player-module",
                    passed: self.pm.ok,
                    detail: self.pm.notes.join("; "),
                });
                // Same session: the game-lock step. Out of
                // first person first (there the camera module locks by itself).
                self.queue.push(Event::PointerMoved(vp.center()));
                self.queue.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: Vec2::new(0.0, -40.0),
                    modifiers: Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                });
                self.pm = PlayerTest {
                    started: f,
                    ok: true,
                    mark: ed.log.len(),
                    ..Default::default()
                };
                self.r4.phase = 68;
            }
            // A client script holds LockCenter every frame, as Place A does in a
            // round; the pointer then leaves the game view while the mouse
            // keeps moving.
            68 if t == 60 => {
                let err = ed.run_luau(
                    Side::Client,
                    r#"
                    local UIS = game:GetService("UserInputService")
                    _G.__lockMode = Enum.MouseBehavior.LockCenter
                    game:GetService("RunService"):BindToRenderStep("SelfTestLock", Enum.RenderPriority.Last.Value, function()
                        UIS.MouseBehavior = _G.__lockMode
                    end)
                    "#,
                );
                if let Some(e) = err {
                    self.pm.ok = false;
                    self.pm.notes.push(format!("lock script: {e}"));
                }
                self.queue.push(Event::PointerMoved(vp.center()));
            }
            68 if t == 75 => {
                self.pm.yaw0 = client_camera(ed).map(|c| c.1).unwrap_or(0.0);
                // Off the view (past its bottom-right corner).
                self.queue
                    .push(Event::PointerMoved(vp.max + Vec2::new(40.0, 40.0)));
            }
            68 if (76..=95).contains(&t) => {
                self.queue.push(Event::MouseMoved(Vec2::new(12.0, 0.0)));
            }
            68 if t == 100 => {
                let held = ed.game_grab.is_some();
                let mb = ed.play.as_ref().map(|s| s.mouse_behavior).unwrap_or(0);
                let yaw = client_camera(ed).map(|c| c.1).unwrap_or(self.pm.yaw0);
                let turned = (yaw - self.pm.yaw0 + 540.0).rem_euclid(360.0) - 180.0;
                let ok = held && mb == 1 && turned.abs() > 2.0;
                self.pm.ok &= ok;
                self.pm.notes.push(format!(
                    "LockCenter from a client script, pointer off the view: grab held {held} (MouseBehavior {mb}), camera turned {turned:.1}°"
                ));
                ed.cursor_log.clear();
                let err = ed.run_luau(
                    Side::Client,
                    "_G.__lockMode = Enum.MouseBehavior.Default",
                );
                if let Some(e) = err {
                    self.pm.ok = false;
                    self.pm.notes.push(format!("default script: {e}"));
                }
                self.r4.phase = 69;
                self.pm.started = f;
            }
            69 if t >= 20 => {
                let released = ed.game_grab.is_none();
                let mb = ed.play.as_ref().map(|s| s.mouse_behavior).unwrap_or(9);
                let log = ed.cursor_log.clone();
                let ok = released
                    && mb == 0
                    && log.iter().any(|l| l == "game-grab:none")
                    && log.iter().any(|l| l == "game-grab:restore");
                self.pm.ok &= ok;
                self.pm.notes.push(format!(
                    "Default from the script: grab released {released} (MouseBehavior {mb}), cursor commands {log:?}"
                ));
                ed.stop_play();
                self.steps.push(Step {
                    step: "game-lock-center",
                    passed: self.pm.ok,
                    detail: self.pm.notes.join("; "),
                });
                self.r4.phase = 254;
            }
            _ => {}
        }
        false
    }
}
