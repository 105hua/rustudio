//! Self-test step for what client scripts read off moving parts. The client
//! simulates the parts its own scripts made (physics_client.rs); Studio's
//! client reports their velocities from that simulation, and a replicated
//! part's from the server. rustudio read 0 for every client-made part
//! (AssemblyLinearVelocity / Velocity / AssemblyAngularVelocity): the
//! client's velocity table held only the server's parts.
//!
//! In Play: a client script makes three parts high above the baseplate, one
//! left to fall, one given AssemblyLinearVelocity (30, 0, 0), one given
//! AssemblyAngularVelocity (0, 8, 0); the server drops a part of its own.
//! After half a second the client must read each one moving.

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;
use crate::runtime::Side;

#[derive(Default)]
pub(super) struct ClientVelocityTest {
    started: u64,
    notes: Vec<String>,
    ok: bool,
}

const SERVER: &str = r#"
local p = Instance.new("Part")
p.Name = "SrvFall"
p.Size = Vector3.new(2, 2, 2)
p.Position = Vector3.new(40, 400, 0)
p.Parent = workspace
"#;

const CLIENT: &str = r#"
local function part(name, x)
    local p = Instance.new("Part")
    p.Name = name
    p.Size = Vector3.new(2, 2, 2)
    p.Position = Vector3.new(x, 300, 30)
    p.Parent = workspace
    return p
end
_G.__cvFall = part("CliFall", 0)
_G.__cvPush = part("CliPush", 20)
_G.__cvSpin = part("CliSpin", -20)
_G.__cvPush.AssemblyLinearVelocity = Vector3.new(30, 0, 0)
_G.__cvSpin.AssemblyAngularVelocity = Vector3.new(0, 8, 0)
"#;

/// Prints what the client reads; errors name the first wrong one.
const CHECK: &str = r#"
local fall, push, spin = _G.__cvFall, _G.__cvPush, _G.__cvSpin
local srv = workspace:FindFirstChild("SrvFall")
local out = string.format("fall %.1f / %.1f, push x %.1f, spin y %.2f, server fall %s",
    fall.AssemblyLinearVelocity.Y, fall.Velocity.Y, push.AssemblyLinearVelocity.X,
    spin.AssemblyAngularVelocity.Y, if srv then string.format("%.1f", srv.AssemblyLinearVelocity.Y) else "missing")
assert(fall.AssemblyLinearVelocity.Y < -5, "client part falling reads " .. out)
assert(fall.Velocity.Y < -5, "Velocity " .. out)
assert(push.AssemblyLinearVelocity.X > 20, "pushed part " .. out)
assert(spin.AssemblyAngularVelocity.Y > 4, "spun part " .. out)
assert(srv and srv.AssemblyLinearVelocity.Y < -5, "server part " .. out)
error("ok: " .. out)
"#;

impl SelfTest {
    /// Phases 244..=245.
    pub(super) fn tick_client_velocity(&mut self, f: u64, ed: &mut Editor) -> bool {
        let t = f.saturating_sub(self.cvel.started);
        match self.r4.phase {
            244 if self.cvel.started == 0 => {
                ed.set_document(Document::new_place());
                ed.start_play(crate::runtime::RunMode::Play);
                self.cvel = ClientVelocityTest { started: f, ok: true, ..Default::default() };
            }
            244 if t == 60 => {
                for (side, src) in [(Side::Server, SERVER), (Side::Client, CLIENT)] {
                    if let Some(e) = ed.run_luau(side, src) {
                        self.cvel.ok = false;
                        self.cvel.notes.push(format!("setup ({side:?}): {e}"));
                    }
                }
                self.r4.phase = 245;
            }
            // (about half a second of falling at the self-test's frame rate)
            245 if t == 95 => {
                let res = ed.run_luau(Side::Client, CHECK).unwrap_or_default();
                let ok = res.contains("ok: ");
                self.cvel.ok &= ok;
                self.cvel.notes.push(res);
                ed.stop_play();
                self.steps.push(Step {
                    step: "client-velocity",
                    passed: self.cvel.ok,
                    detail: self.cvel.notes.join("; "),
                });
                self.r4.phase = 246;
            }
            _ => {}
        }
        false
    }
}
