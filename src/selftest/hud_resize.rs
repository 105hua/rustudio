//! Self-test step for screen-fitted GUIs across a resize (a live place Map 1,
//! 2026-11-06: Place A's HUD is a 720-unit canvas scaled by a UIScale that it
//! refits on ScreenGui.AbsoluteSize changes; rustudio never fired those, so
//! after the 3D view grew the points / round tally / perks sat mid-screen
//! instead of bottom-left). A client script builds the same canvas and a
//! bottom-left box, the UI zoom changes (the 3D view changes size),
//! and the canvas must refit with the box back at the bottom.

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;
use crate::runtime::Side;

#[derive(Default)]
pub(super) struct HudResizeTest {
    started: u64,
    notes: Vec<String>,
    ok: bool,
    zoom: f32,
}

/// HudStyle.canvas + HudScore's box, as Place A builds them.
const SETUP: &str = r#"
local me = game.Players.LocalPlayer
local gui = Instance.new("ScreenGui")
gui.Name = "HudResizeTest"
gui.IgnoreGuiInset = true
gui.Parent = me:WaitForChild("PlayerGui")
local canvas = Instance.new("Frame")
canvas.Name = "Canvas720"
canvas.BackgroundTransparency = 1
canvas.Parent = gui
local scale = Instance.new("UIScale")
scale.Parent = canvas
_G.__fits, _G.__vpChanged = 0, 0
local function fit()
    local vp = gui.AbsoluteSize
    if vp.Y <= 0 then return end
    local k = vp.Y / 720
    scale.Scale = k
    canvas.Size = UDim2.fromOffset(vp.X / k, 720)
    _G.__fits += 1
end
gui:GetPropertyChangedSignal("AbsoluteSize"):Connect(fit)
fit()
workspace.CurrentCamera:GetPropertyChangedSignal("ViewportSize"):Connect(function() _G.__vpChanged += 1 end)
-- Score: setLeftRight(true, false, 30, 164) setTopBottom(false, true, -256, -128)
local score = Instance.new("Frame")
score.Name = "Score"
score.AnchorPoint = Vector2.new(0.5, 0.5)
score.Position = UDim2.new(0, 97, 1, -192)
score.Size = UDim2.fromOffset(134, 128)
score.Parent = canvas
_G.__hudGui, _G.__hudCanvas, _G.__hudScore = gui, canvas, score
"#;

/// The canvas covers the screen and Score's bottom is 128 canvas units above it.
const CHECK: &str = r#"
local gui, canvas, score = _G.__hudGui, _G.__hudCanvas, _G.__hudScore
local g, c, s = gui.AbsoluteSize, canvas.AbsoluteSize, score.AbsolutePosition.Y + score.AbsoluteSize.Y
local k = g.Y / 720
local bottom = gui.AbsolutePosition.Y + g.Y - 128 * k
assert(math.abs(c.X - g.X) < 2 and math.abs(c.Y - g.Y) < 2, string.format("canvas %s, screen %s", tostring(c), tostring(g)))
assert(math.abs(s - bottom) < 2, string.format("Score bottom %.1f, want %.1f", s, bottom))
return string.format("%dx%d", g.X, g.Y)
"#;

impl SelfTest {
    /// Phases 242..=243.
    pub(super) fn tick_hud_resize(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.hud.started);
        match self.r4.phase {
            242 if self.hud.started == 0 => {
                ed.set_document(Document::new_place());
                ed.start_play(crate::runtime::RunMode::Play);
                self.hud = HudResizeTest { started: f, ok: true, ..Default::default() };
            }
            242 if t == 60 => {
                if let Some(e) = ed.run_luau(Side::Client, SETUP) {
                    self.hud.ok = false;
                    self.hud.notes.push(format!("setup: {e}"));
                }
            }
            242 if t == 90 => {
                let before = ed.run_luau(Side::Client, CHECK);
                self.hud.ok &= before.is_none();
                self.hud.notes.push(format!("before resize: {}", before.as_deref().unwrap_or("ok")));
                // The 3D view changes size (the UI zoom: the window holds
                // fewer points; put back after, so the dock is untouched).
                self.hud.zoom = ed.ui.zoom;
                ed.ui.zoom = (ed.ui.zoom + 0.25).min(2.0);
                self.r4.phase = 243;
                self.hud.started = f;
            }
            243 if t == 60 => {
                let after = ed.run_luau(Side::Client, CHECK);
                let signals = ed.run_luau(
                    Side::Client,
                    "assert(_G.__fits >= 2, 'AbsoluteSize never changed: fits ' .. _G.__fits); assert(_G.__vpChanged >= 1, 'ViewportSize never changed')",
                );
                let ok = after.is_none() && signals.is_none();
                self.hud.ok &= ok;
                self.hud.notes.push(format!(
                    "after resize: {}; signals: {}",
                    after.as_deref().unwrap_or("canvas refit, Score at the bottom"),
                    signals.as_deref().unwrap_or("AbsoluteSize and ViewportSize fired")
                ));
                ed.ui.zoom = self.hud.zoom;
                ed.stop_play();
                self.steps.push(Step {
                    step: "gui-resize-refit",
                    passed: self.hud.ok,
                    detail: self.hud.notes.join("; "),
                });
                self.r4.phase = 70;
            }
            _ => {}
        }
        false
    }
}
