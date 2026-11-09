//! Self-test step for the in-game menu (Esc): a game holding LockCenter has
//! its cursor freed while the menu is open (no grab, the arrow shows, no
//! mouse-look), GuiService.MenuIsOpen / MenuOpened / MenuClosed follow it,
//! the game's MouseBehavior stays LockCenter, and Resume takes the lock back.

use egui::{Event, Key, Modifiers, PointerButton, Vec2};

use super::{key, SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;
use crate::runtime::Side;

#[derive(Default)]
pub(super) struct PauseMenuTest {
    started: u64,
    notes: Vec<String>,
    ok: bool,
}

impl SelfTest {
    /// Phases 254..=255.
    pub(super) fn tick_pause_menu(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.pmenu.started);
        let vp = ed.viewport_rect;
        match self.r4.phase {
            254 if self.pmenu.started == 0 => {
                ed.set_document(Document::new_place());
                ed.start_play(crate::runtime::RunMode::Play);
                self.pmenu = PauseMenuTest { started: f, ok: true, ..Default::default() };
            }
            // A client script holds LockCenter every frame (Place A does in a round)
            // and counts GuiService's menu signals.
            254 if t == 60 => {
                let err = ed.run_luau(
                    Side::Client,
                    r#"
                    local UIS = game:GetService("UserInputService")
                    local GS = game:GetService("GuiService")
                    _G.__menuOpened, _G.__menuClosed = 0, 0
                    GS.MenuOpened:Connect(function() _G.__menuOpened += 1 end)
                    GS.MenuClosed:Connect(function() _G.__menuClosed += 1 end)
                    game:GetService("RunService"):BindToRenderStep("SelfTestMenuLock", Enum.RenderPriority.Last.Value, function()
                        UIS.MouseBehavior = Enum.MouseBehavior.LockCenter
                    end)
                    "#,
                );
                if let Some(e) = err {
                    self.pmenu.ok = false;
                    self.pmenu.notes.push(format!("lock script: {e}"));
                }
                self.queue.push(Event::PointerMoved(vp.center()));
            }
            254 if t == 80 => {
                let held = ed.game_grab.is_some();
                self.pmenu.ok &= held;
                self.pmenu.notes.push(format!("LockCenter before the menu: grab held {held}"));
                ed.cursor_log.clear();
                self.queue.push(key(Key::Escape, true, Modifiers::NONE));
                self.queue.push(key(Key::Escape, false, Modifiers::NONE));
            }
            // Mouse motion while open must not turn the camera.
            254 if (90..=100).contains(&t) => {
                self.queue.push(Event::MouseMoved(Vec2::new(15.0, 0.0)));
            }
            254 if t == 110 => {
                let open = ed.play.as_ref().is_some_and(|s| s.game.menu_open);
                let released = ed.game_grab.is_none();
                let shown = !ed.game_cursor_hidden;
                let mb = ed.play.as_ref().map(|s| s.mouse_behavior).unwrap_or(9);
                let log = ed.cursor_log.clone();
                let err = ed.run_luau(
                    Side::Client,
                    "assert(game:GetService('GuiService').MenuIsOpen, 'MenuIsOpen false'); assert(_G.__menuOpened == 1, 'MenuOpened ' .. _G.__menuOpened)",
                );
                let ok = open && released && shown && mb == 1 && log.iter().any(|l| l == "game-grab:menu") && err.is_none();
                self.pmenu.ok &= ok;
                self.pmenu.notes.push(format!(
                    "Esc: menu open {open}, grab released {released}, cursor shown {shown}, MouseBehavior still {mb} (LockCenter 1), cursor commands {log:?}, GuiService check {}",
                    err.as_deref().unwrap_or("ok")
                ));
                ed.cursor_log.clear();
                self.r4.phase = 255;
                self.pmenu.started = f;
            }
            // Resume, clicked where the menu drew it.
            255 if t == 10 => {
                match ed.play.as_ref().and_then(|s| s.game.menu.rects.get("resume").copied()) {
                    Some(r) => {
                        let p = r.center();
                        self.queue.push(Event::PointerMoved(p));
                        for pressed in [true, false] {
                            self.queue.push(Event::PointerButton {
                                pos: p,
                                button: PointerButton::Primary,
                                pressed,
                                modifiers: Modifiers::NONE,
                            });
                        }
                    }
                    None => {
                        self.pmenu.ok = false;
                        self.pmenu.notes.push("no Resume button drawn".into());
                    }
                }
            }
            255 if t == 40 => {
                let open = ed.play.as_ref().is_some_and(|s| s.game.menu_open);
                let held = ed.game_grab.is_some();
                let log = ed.cursor_log.clone();
                let err = ed.run_luau(
                    Side::Client,
                    "assert(not game:GetService('GuiService').MenuIsOpen, 'MenuIsOpen true'); assert(_G.__menuClosed == 1, 'MenuClosed ' .. _G.__menuClosed)",
                );
                let ok = !open && held && log.iter().any(|l| l == "game-grab") && err.is_none();
                self.pmenu.ok &= ok;
                self.pmenu.notes.push(format!(
                    "Resume: menu open {open}, grab held again {held}, cursor commands {log:?}, GuiService check {}",
                    err.as_deref().unwrap_or("ok")
                ));
                ed.stop_play();
                self.steps.push(Step {
                    step: "pause-menu-cursor",
                    passed: self.pmenu.ok,
                    detail: self.pmenu.notes.join("; "),
                });
                self.r4.phase = 242;
            }
            _ => {}
        }
        false
    }
}
