//! Self-test step for Sound behaviour: a non-looped sound
//! ends once, never wraps and then reads IsPaused true (Studio's IsPaused is
//! `not Playing`, from Studio); a looped one fires DidLoop per wrap and keeps
//! playing; `Playing = true` twice starts it once, `Play()` twice restarts
//! it; a missing asset is warned once and not fetched again; a clip picked
//! with PlaybackRegion ends at the region's end, not the file's (Place A's
//! SoundBank keeps many clips in one asset); a destroyed Sound stops.

use rbx_dom_weak::InstanceBuilder;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;

const SRC: &str = r#"local function mk(name, id, looped)
	local s = Instance.new("Sound")
	s.Name = name
	s.SoundId = id
	s.Looped = looped
	s.Parent = workspace
	return s
end
local once = mk("Once", "rbxasset://sounds/snd-once.wav", false)
local ended, wrapped, last = 0, false, 0
once.Ended:Connect(function() ended += 1 end)
local loop = mk("Loop", "rbxasset://sounds/snd-loop.wav", true)
local loops = 0
loop.DidLoop:Connect(function() loops += 1 end)
local region = mk("Region", "rbxasset://sounds/snd-sheet.wav", false)
region.PlaybackRegionsEnabled = true
region.PlaybackRegion = NumberRange.new(0.5, 0.6)
local regionEnded
local t0 = os.clock()
region.Ended:Connect(function() regionEnded = os.clock() - t0 end)
local gone = mk("Gone", "rbxasset://sounds/snd-gone.wav", true)
once:Play()
loop:Play()
region:Play()
gone:Play()
local idem = mk("Idem", "rbxasset://sounds/snd-idem.wav", false)
idem.Playing = true
task.wait(0.05)
idem.Playing = true
local twice = mk("Twice", "rbxasset://sounds/snd-twice.wav", false)
twice:Play()
task.wait(0.05)
twice:Play()
local missing = mk("Missing", "rbxasset://sounds/snd-missing.ogg", false)
missing:Play()
task.wait(0.1)
missing:Play()
task.wait(0.1)
gone:Destroy()
local t = os.clock()
while os.clock() - t < 1.2 do
	local p = once.TimePosition
	if p < last - 1e-6 then wrapped = true end
	last = p
	task.wait()
end
print(string.format(
	"SNDTEST once ended=%d wrapped=%s playing=%s paused=%s | loop loops=%d playing=%s paused=%s | region ended=%s | idem paused=%s",
	ended, tostring(wrapped), tostring(once.IsPlaying), tostring(once.IsPaused),
	loops, tostring(loop.IsPlaying), tostring(loop.IsPaused),
	regionEnded and string.format("%.2f", regionEnded) or "never", tostring(idem.IsPaused)))
"#;

/// A silent 16-bit mono WAV of `seconds`.
fn wav(seconds: f32) -> Vec<u8> {
    super::wav(seconds)
}

impl SelfTest {
    /// Phases 94..=95.
    pub(super) fn tick_sound(&mut self, f: u64, ed: &mut Editor) -> bool {
        if self.r4.phase == 94 {
            let dir = crate::assets::content_dir().join("sounds");
            let _ = std::fs::create_dir_all(&dir);
            for (name, secs) in [
                ("snd-once", 0.2),
                ("snd-loop", 0.2),
                ("snd-sheet", 1.5),
                ("snd-idem", 1.0),
                ("snd-twice", 1.0),
                ("snd-gone", 0.3),
            ] {
                let _ = std::fs::write(dir.join(format!("{name}.wav")), wav(secs));
            }
            let mut doc = Document::new_place();
            let root = doc.dom.root_ref();
            let sss = doc
                .find_child(root, "ServerScriptService")
                .expect("ServerScriptService");
            doc.insert(
                sss,
                InstanceBuilder::new("Script")
                    .with_name("SoundTest")
                    .with_property("Source", SRC.to_string()),
            );
            ed.set_document(doc);
            self.r6 = super::R6Test {
                started: f,
                mark: ed.log.len(),
                t0: Some(std::time::Instant::now()),
                ..Default::default()
            };
            ed.start_play(crate::runtime::RunMode::Run);
            self.r4.phase = 95;
            return false;
        }
        let line = ed.log[self.r6.mark.min(ed.log.len())..]
            .iter()
            .find(|l| l.text.contains("SNDTEST"))
            .map(|l| l.text.clone());
        let waited = self.r6.t0.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
        if line.is_none() && waited < 15.0 {
            return false;
        }
        let line = line.unwrap_or_else(|| "no SNDTEST line in 15 s".into());
        let id = |n: &str| format!("rbxasset://sounds/{n}");
        let (starts, fetches, still_gone) = match ed.play.as_ref() {
            Some(s) => {
                let g = &s.game;
                let st = |n: &str| g.audio_starts_by_id.get(&id(n)).copied().unwrap_or(0);
                let fe = |n: &str| g.sound_fetches.get(&id(n)).copied().unwrap_or(0);
                (
                    [st("snd-idem.wav"), st("snd-twice.wav"), st("snd-once.wav")],
                    fe("snd-missing.ogg"),
                    g.is_sound_live(&id("snd-gone.wav")),
                )
            }
            None => ([0; 3], 0, true),
        };
        let warnings = ed.log[self.r6.mark.min(ed.log.len())..]
            .iter()
            .filter(|l| l.text.contains("snd-missing.ogg"))
            .count();
        ed.stop_play();
        let region_end = line
            .split("region ended=")
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .and_then(|v| v.parse::<f32>().ok());

        let ok = line.contains("once ended=1 wrapped=false playing=false paused=true")
            && line.contains("playing=true paused=false | region")
            && line
                .split("loops=")
                .nth(1)
                .and_then(|r| r.split_whitespace().next())
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|n| n >= 3)
            && region_end.is_some_and(|t| t < 0.4)
            && line.contains("idem paused=true") // (its 1 s clip has ended: not Playing)
            && starts == [1, 2, 1]
            && fetches == 1
            && warnings == 1
            && !still_gone;
        self.steps.push(Step {
            step: "sound",
            passed: ok,
            detail: format!(
                "{line}; starts: Playing = true twice {} (want 1), Play() twice {} (want 2), once {} (want 1); missing asset fetched {fetches} time(s) and warned {warnings} (want 1, 1); destroyed looped sound still playing: {still_gone}; want: Ended once, no wrap, IsPaused true after (Studio), ≥ 3 DidLoop in 1.2 s of a 0.2 s loop, the 0.1 s region ending well before its 1.5 s file",
                starts[0], starts[1], starts[2]
            ),
        });
        self.r4.phase = 240;
        false
    }
}
