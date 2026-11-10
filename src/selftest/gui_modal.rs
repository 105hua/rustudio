//! Self-test step for GuiButton.Modal (gui_modal.rs): a game holding
//! MouseBehavior LockCenter every frame lets go of the cursor while a
//! Modal button is shown in its PlayerGui, takes it back when the button is
//! hidden, keeps it when the shown button isn't Modal, and
//! UserInputService.MouseBehavior keeps reading LockCenter throughout.

use egui::Event;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;
use crate::runtime::Side;

#[derive(Default)]
pub(super) struct GuiModalTest {
    started: u64,
    notes: Vec<String>,
    ok: bool,
    /// The view side shown before this step (restored after).
    showed_client: bool,
}

const SETUP: &str = r#"
local UIS = game:GetService("UserInputService")
game:GetService("RunService"):BindToRenderStep("SelfTestModalLock", Enum.RenderPriority.Last.Value, function()
    UIS.MouseBehavior = Enum.MouseBehavior.LockCenter
end)
local gui = Instance.new("ScreenGui")
gui.Name = "ModalTest"
local b = Instance.new("TextButton")
b.Size = UDim2.fromOffset(160, 40)
b.Position = UDim2.fromOffset(40, 40)
b.Text = "Modal"
b.Visible = false
b.Parent = gui
gui.Parent = game.Players.LocalPlayer:WaitForChild("PlayerGui")
_G.__modalButton = b
"#;

impl SelfTest {
    fn modal_check(&mut self, ed: &Editor, want_grab: bool, what: &str) {
        let held = ed.game_grab.is_some();
        let behavior = ed.play.as_ref().map(|s| s.mouse_behavior).unwrap_or(9);
        let ok = held == want_grab && behavior == 1;
        self.gmodal.ok &= ok;
        self.gmodal.notes.push(format!(
            "{what}: grab held {held} (want {want_grab}), MouseBehavior {behavior} (want 1 LockCenter)"
        ));
    }

    /// Phases 246..=247.
    pub(super) fn tick_gui_modal(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.gmodal.started);
        let vp = ed.viewport_rect;
        let mut run = |this: &mut Self, ed: &mut Editor, src: &str| {
            if let Some(e) = ed.run_luau(Side::Client, src) {
                this.gmodal.ok = false;
                this.gmodal.notes.push(format!("script: {e}"));
            }
        };
        match self.r4.phase {
            246 if self.gmodal.started == 0 => {
                let showed_client = ed.play_shows_client;
                ed.play_shows_client = true;
                ed.set_document(Document::new_place());
                ed.start_play(crate::runtime::RunMode::Play);
                self.gmodal = GuiModalTest { started: f, ok: true, showed_client, ..Default::default() };
            }
            246 if t == 60 => {
                run(self, ed, SETUP);
                self.queue.push(Event::PointerMoved(vp.center()));
            }
            246 if t == 80 => {
                self.modal_check(ed, true, "LockCenter, button hidden");
                run(self, ed, "_G.__modalButton.Visible = true");
            }
            // (the button shows: drawn and laid out on the next frames)
            246 if t == 95 => {
                self.modal_check(ed, true, "a shown button that isn't Modal");
                run(self, ed, "_G.__modalButton.Modal = true");
                self.r4.phase = 247;
            }
            247 if t == 110 => {
                self.modal_check(ed, false, "Modal set on the shown button");
                run(self, ed, "_G.__modalButton.Visible = false");
            }
            247 if t == 125 => {
                self.modal_check(ed, true, "Modal button hidden again");
                run(self, ed, "_G.__modalButton.Visible = true; _G.__modalButton.Parent.Enabled = false");
            }
            247 if t == 140 => {
                self.modal_check(ed, true, "its ScreenGui disabled");
                ed.stop_play();
                ed.play_shows_client = self.gmodal.showed_client;
                self.steps.push(Step {
                    step: "gui-modal-cursor",
                    passed: self.gmodal.ok,
                    detail: self.gmodal.notes.join("; "),
                });
                self.r4.phase = 242;
            }
            _ => {}
        }
        false
    }
}
