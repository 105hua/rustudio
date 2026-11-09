//! Self-test step `gc`: instances a game makes and lets go of are freed, as
//! Roblox's garbage collector frees them. A script makes thousands of parts,
//! each with two Attachments, a Beam, a ParticleEmitter, a Sound and a
//! Touched connection (the power-up weapon's tracers, impacts and shot sounds
//! on a live place grew RSS past 24 GB), and gets rid of them three ways:
//! Destroy, Debris:AddItem and Parent = nil. Once they're gone the DataModel,
//! the collector's unparented set and the Lua value cache must be back at
//! their baseline, and every one of them freed. A part kept in a script
//! variable while unparented must survive (Roblox keeps it, and it can come
//! back).

use rbx_dom_weak::InstanceBuilder;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;

/// Parts per wave, waves.
const PER_WAVE: usize = 1000;
const WAVES: usize = 3;
/// Instances made per part (Part, 2 Attachments, Beam, ParticleEmitter, Sound).
const PER_PART: usize = 6;

const SRC: &str = r#"local N, WAVES = ...
local Debris = game:GetService("Debris")
task.wait(1)
print(string.format("GCTEST base kb=%.0f", gcinfo()))
task.wait(1)
local kept = Instance.new("Part")
kept.Name = "KeptOut"
kept.Parent = workspace
kept.Parent = nil
for w = 1, WAVES do
	for i = 1, N do
		local p = Instance.new("Part")
		p.Anchored = true
		p.Size = Vector3.new(1, 1, 1)
		p.Position = Vector3.new(i % 40, 30 + w, i // 40)
		local a0 = Instance.new("Attachment", p)
		local a1 = Instance.new("Attachment", p)
		a1.Position = Vector3.new(0, 3, 0)
		local b = Instance.new("Beam")
		b.Attachment0, b.Attachment1 = a0, a1
		b.Parent = p
		local e = Instance.new("ParticleEmitter")
		e.Rate = 0
		e.Parent = p
		local s = Instance.new("Sound")
		s.SoundId = "rbxasset://sounds/snd-once.wav"
		s.Parent = p
		p.Touched:Connect(function() end)
		p.Parent = workspace
		e:Emit(1)
		if i % 4 == 0 then s:Play() end
		local k = i % 3
		if k == 0 then
			p:Destroy()
		elseif k == 1 then
			Debris:AddItem(p, 0.2)
		else
			task.delay(0.1, function() p.Parent = nil end)
		end
	end
	task.wait(0.3)
end
task.wait(3)
print(string.format("GCTEST done kb=%.0f kept=%s", gcinfo(), tostring(kept.Name)))
task.wait(1.5)
kept.Parent = workspace
task.wait(0.2)
print("GCTEST back=" .. tostring(workspace:FindFirstChild("KeptOut") ~= nil))
"#;

#[derive(Default)]
pub(super) struct GcTest {
    t0: Option<std::time::Instant>,
    mark: usize,
    /// DataModel instances, Lua slots and Lua KB at the baseline line.
    base: Option<(usize, usize, f64)>,
    done: Option<(usize, usize, usize, u64, f64)>,
}

fn kb(line: &str) -> f64 {
    line.split("kb=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}

impl SelfTest {
    /// Phases 240..=241.
    pub(super) fn tick_gc(&mut self, _f: u64, ed: &mut Editor) -> bool {
        if self.r4.phase == 240 {
            let mut doc = Document::new_place();
            let root = doc.dom.root_ref();
            let sss = doc
                .find_child(root, "ServerScriptService")
                .expect("ServerScriptService");
            let src = format!(
                "return (function(...)\n{SRC}\nend)({PER_WAVE}, {WAVES})"
            );
            doc.insert(
                sss,
                InstanceBuilder::new("Script")
                    .with_name("GcTest")
                    .with_property("Source", src),
            );
            ed.set_document(doc);
            self.gc = GcTest {
                t0: Some(std::time::Instant::now()),
                mark: ed.log.len(),
                ..Default::default()
            };
            ed.start_play(crate::runtime::RunMode::Run);
            self.r4.phase = 241;
            return false;
        }
        let lines: Vec<String> = ed.log[self.gc.mark.min(ed.log.len())..]
            .iter()
            .filter(|l| l.text.contains("GCTEST"))
            .map(|l| l.text.clone())
            .collect();
        let tree = ed.doc.dom.descendants().count();
        let counts = ed.play.as_ref().map(|s| s.gc_counts());
        if self.gc.base.is_none() {
            if let (Some(l), Some((_, _, slots))) = (lines.iter().find(|l| l.contains("base")), counts) {
                self.gc.base = Some((tree, slots, kb(l)));
            }
        }
        if self.gc.done.is_none() {
            if let (Some(l), Some((orphans, freed, slots))) = (lines.iter().find(|l| l.contains("done")), counts) {
                self.gc.done = Some((tree, slots, orphans, freed, kb(l)));
            }
        }
        let back = lines.iter().find(|l| l.contains("back=")).cloned();
        let waited = self.gc.t0.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
        if back.is_none() && waited < 60.0 {
            return false;
        }
        ed.stop_play();
        let made = PER_WAVE * WAVES * PER_PART;
        let (ok, detail) = match (self.gc.base, self.gc.done) {
            (Some((t0, s0, k0)), Some((t1, s1, orphans, freed, k1))) => {
                // The kept part is the one unparented instance left; the
                // script's own few values may hold a slot or two.
                let ok = t1 == t0
                    && s1 <= s0 + 16
                    && orphans <= 2
                    && freed as usize >= made
                    && back.as_deref().is_some_and(|b| b.contains("back=true"));
                (
                    ok,
                    format!(
                        "made {made} instances ({WAVES} waves of {PER_WAVE} parts with 2 Attachments, a Beam, a ParticleEmitter, a Sound, a Touched connection; Destroy / Debris / Parent = nil); DataModel {t0} → {t1} (want equal), Lua slots {s0} → {s1} (want ≤ +16), unparented still watched {orphans} (want ≤ 2: the kept part), freed {freed} (want ≥ {made}), Lua heap {k0:.0} → {k1:.0} KB; kept unparented part came back: {}",
                        back.as_deref().unwrap_or("no line")
                    ),
                )
            }
            _ => (false, format!("no GCTEST base/done lines in 60 s: {lines:?}")),
        };
        self.steps.push(Step { step: "gc", passed: ok, detail });
        self.r4.phase = 97;
        false
    }
}
