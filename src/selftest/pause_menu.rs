//! Self-test step for the in-game menu (Esc): a game holding LockCenter has
//! its cursor freed while the menu is open (no grab, the arrow shows, no
//! mouse-look), GuiService.MenuIsOpen / MenuOpened / MenuClosed follow it,
//! the game's MouseBehavior stays LockCenter, hovering, moving, scrolling and
//! right-dragging over every page (People, Settings, Captures, Report, Help
//! and both confirmations) never grabs the cursor again, and Resume takes
//! the lock back.

use egui::{Event, Key, Modifiers, PointerButton, Vec2};

use super::{key, SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;
use crate::play::game::game_menu::MenuPage;
use crate::runtime::Side;

/// Every page the hover sweep visits, in order.
const PAGES: [MenuPage; 7] = [
    MenuPage::People,
    MenuPage::Settings,
    MenuPage::Captures,
    MenuPage::Report,
    MenuPage::Help,
    MenuPage::ConfirmReset,
    MenuPage::ConfirmLeave,
];
/// Frames per page in the sweep.
const PAGE_FRAMES: u64 = 40;

#[derive(Default)]
pub(super) struct PauseMenuTest {
    started: u64,
    notes: Vec<String>,
    ok: bool,
    /// The hover sweep over every page is done (phase 255 then resumes).
    swept: bool,
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
                    -- Right-button releases while the menu is open still end (processed).
                    _G.__rmbEnded, _G.__rmbBegan = 0, 0
                    UIS.InputBegan:Connect(function(i, gpe) if i.UserInputType == Enum.UserInputType.MouseButton2 then _G.__rmbBegan += 1 end end)
                    UIS.InputEnded:Connect(function(i, gpe) if i.UserInputType == Enum.UserInputType.MouseButton2 and gpe then _G.__rmbEnded += 1 end end)
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
            // Hover sweep: each page in turn, the pointer moved across the
            // page (or a confirmation's panel) with raw motion, a wheel notch
            // and a right-drag; nothing may grab or hide the cursor.
            255 if !self.pmenu.swept => {
                let k = (t / PAGE_FRAMES) as usize;
                let at = t % PAGE_FRAMES;
                if k >= PAGES.len() {
                    let err = ed.run_luau(
                        Side::Client,
                        &format!("assert(_G.__rmbEnded == {n} and _G.__rmbBegan == 0, ('right button in the menu: %d began, %d ended processed'):format(_G.__rmbBegan, _G.__rmbEnded))", n = PAGES.len()),
                    );
                    self.pmenu.ok &= err.is_none();
                    self.pmenu.notes.push(format!(
                        "right-drags in the menu: presses unheard, releases end processed: {}",
                        err.as_deref().unwrap_or("ok")
                    ));
                    if let Some(s) = ed.play.as_mut() {
                        s.game.menu.page = MenuPage::People;
                    }
                    ed.cursor_log.clear();
                    self.pmenu.swept = true;
                    self.pmenu.started = f;
                    return false;
                }
                let page = PAGES[k];
                if at == 0 {
                    if let Some(s) = ed.play.as_mut() {
                        s.game.menu.page = page;
                    }
                }
                let area = ed.play.as_ref().and_then(|s| {
                    let r = &s.game.menu.rects;
                    if matches!(page, MenuPage::ConfirmReset | MenuPage::ConfirmLeave) {
                        r.get("container").copied()
                    } else {
                        r.get("page").copied()
                    }
                });
                match (at, area) {
                    // (a frame to draw the page first)
                    (0..=2, _) => {}
                    (3..=26, Some(r)) => {
                        // A zig-zag over the page: rows top to bottom.
                        let i = (at - 3) as f32;
                        let x = if (at / 4) % 2 == 0 { r.left() + 8.0 + i * 37.0 % (r.width() - 16.0) } else { r.right() - 8.0 - i * 37.0 % (r.width() - 16.0) };
                        let y = r.top() + 6.0 + (i / 23.0) * (r.height() - 12.0);
                        self.queue.push(Event::PointerMoved(egui::pos2(x, y)));
                        self.queue.push(Event::MouseMoved(Vec2::new(12.0, 7.0)));
                        if at == 14 {
                            self.queue.push(Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Line,
                                delta: Vec2::new(0.0, -1.0),
                                modifiers: Modifiers::NONE,
                                phase: egui::TouchPhase::Move,
                            });
                        }
                        if at == 18 || at == 24 {
                            self.queue.push(Event::PointerButton {
                                pos: egui::pos2(x, y),
                                button: PointerButton::Secondary,
                                pressed: at == 18,
                                modifiers: Modifiers::NONE,
                            });
                        }
                    }
                    (3..=26, None) => {
                        self.pmenu.ok = false;
                        self.pmenu.notes.push(format!("{page:?}: no page drawn"));
                        if let Some(s) = ed.play.as_mut() {
                            s.game.menu.page = PAGES[(k + 1).min(PAGES.len() - 1)];
                        }
                        self.pmenu.started = f - (k as u64 + 1) * PAGE_FRAMES;
                    }
                    (36, _) => {
                        let open = ed.play.as_ref().is_some_and(|s| s.game.menu_open);
                        let shown = ed.play.as_ref().map(|s| s.game.menu.page);
                        let free = ed.game_grab.is_none() && ed.look_grab.is_none();
                        let visible = !ed.game_cursor_hidden;
                        let mb = ed.play.as_ref().map(|s| s.mouse_behavior).unwrap_or(9);
                        let log = std::mem::take(&mut ed.cursor_log);
                        let grabbed = log.iter().any(|l| l.contains("grab") && !l.ends_with(":none") && !l.ends_with(":menu"));
                        let ok = open && shown == Some(page) && free && visible && !grabbed && mb == 1;
                        self.pmenu.ok &= ok;
                        self.pmenu.notes.push(format!(
                            "hover {page:?}: menu open {open}, page {shown:?}, cursor free {free}, shown {visible}, MouseBehavior {mb}, cursor commands {log:?}"
                        ));
                    }
                    _ => {}
                }
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
                self.r4.phase = 244;
            }
            _ => {}
        }
        false
    }
}
