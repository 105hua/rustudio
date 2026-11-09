//! Self-test step for the Animator and animation assets: in Play, Roblox's
//! own Animate script (from the Studio install) loads its default
//! animations (R15, then R6) through the character's Animator; the assets
//! come from public asset delivery and their KeyframeSequences drive the
//! Motor6Ds. A LocalScript reports what the client sees.

use egui::{Key, Modifiers};
use rbx_dom_weak::types::Enum;
use rbx_dom_weak::InstanceBuilder;

use super::{key, SelfTest, Step};
use crate::app::Editor;
use crate::doc::Document;

/// Reports the Animator's tracks once Animate's idle has loaded, then the
/// walk after Humanoid:Move, plus a layered Action track at half weight.
const REPORT_SRC: &str = r#"local Players = game:GetService("Players")
local RunService = game:GetService("RunService")
local player = Players.LocalPlayer
local char = player.Character or player.CharacterAdded:Wait()
local hum = char:WaitForChild("Humanoid")
local animator = hum:FindFirstChildOfClass("Animator")
local rig = hum.RigType.Name
local function playing()
    local out = {}
    for _, t in (animator or hum):GetPlayingAnimationTracks() do
        if t.Length > 0 then
            table.insert(out, string.format("%s(%s, %.2fs, looped %s, %s)", t.Name, t.Animation.AnimationId:match("%d+$") or "?", t.Length, tostring(t.Looped), t.Priority.Name))
        end
    end
    table.sort(out)
    return out
end
local t0 = os.clock()
repeat RunService.Heartbeat:Wait() until #playing() > 0 or os.clock() - t0 > 90
local idle = playing()
local joint = rig == "R15" and char:FindFirstChild("RightShoulder", true) or char:FindFirstChild("Right Shoulder", true)
local moved = 0
for _ = 1, 30 do
    RunService.Heartbeat:Wait()
    if joint then
        local _, a = joint.Transform:ToAxisAngle()
        moved = math.max(moved, math.abs(a), joint.Transform.Position.Magnitude)
    end
end
-- The test holds W now (the ControlModule moves the character).
print("anim-hold-w")
local t1 = os.clock()
local walk = {}
repeat
    RunService.Heartbeat:Wait()
    walk = playing()
until table.concat(walk, " "):lower():find("walk", 1, true) or os.clock() - t1 > 60
print("anim-release-w")
-- Layering: an Action track at weight 0.5 over the idle.
local action = Instance.new("Animation")
action.AnimationId = "rbxassetid://" .. (idle[1] and idle[1]:match("%((%d+)") or "0")
local tr = animator:LoadAnimation(action)
tr.Priority = Enum.AnimationPriority.Action
tr:Play(0, 0.5)
for _ = 1, 5 do RunService.Heartbeat:Wait() end
print(string.format("anim-report %s animator=%s idle=[%s] joint=%.3f walk=[%s] action=%s/%.2f",
    rig, tostring(animator ~= nil and animator.Parent == hum), table.concat(idle, " "), moved,
    table.concat(walk, " "), tr.Priority.Name, tr.WeightCurrent))
"#;

/// The idle's KeyframeSequence decoded from its asset, on the server
/// (InsertService:LoadAsset is server-only, as in Studio, so the client
/// report can't do it).
const KFS_SRC: &str = r#"local id = ...
local kfs = game:GetService("InsertService"):LoadAsset(id)
local seq = kfs and kfs:FindFirstChildWhichIsA("KeyframeSequence", true)
local frames, poses = 0, 0
if seq then
    for _, k in seq:GetChildren() do
        if k:IsA("Keyframe") then frames += 1; poses += #k:GetDescendants() end
    end
end
print(string.format("anim-kfs keyframes=%d poses=%d", frames, poses))
"#;

impl SelfTest {
    /// Phases 76..=79: R15 (76, 77), then R6 (78, 79).
    pub(super) fn tick_anim(&mut self, f: u64, ed: &mut Editor) -> bool {
        let phase = self.r4.phase;
        if phase == 76 || phase == 78 {
            let r6 = phase == 78;
            let mut doc = Document::new_place();
            let root = doc.dom.root_ref();
            let sp = doc.find_child(root, "StarterPlayer").unwrap_or_else(|| {
                doc.dom.insert(
                    root,
                    InstanceBuilder::new("StarterPlayer").with_name("StarterPlayer"),
                )
            });
            if let Some(i) = doc.dom.get_by_ref_mut(sp) {
                i.properties.insert(
                    rbx_dom_weak::ustr("GameSettingsAvatar"),
                    Enum::from_u32(if r6 { 0 } else { 1 }).into(),
                );
            }
            let sps = doc
                .find_child(sp, "StarterPlayerScripts")
                .unwrap_or_else(|| {
                    doc.dom.insert(
                        sp,
                        InstanceBuilder::new("StarterPlayerScripts")
                            .with_name("StarterPlayerScripts"),
                    )
                });
            doc.insert(
                sps,
                InstanceBuilder::new("LocalScript")
                    .with_name("AnimReport")
                    .with_property("Source", REPORT_SRC.to_string()),
            );
            let sss = doc
                .find_child(root, "ServerScriptService")
                .expect("ServerScriptService");
            // The first of Roblox's default idles for the rig.
            let idle = if r6 { 180435571 } else { 507766388 };
            doc.insert(
                sss,
                InstanceBuilder::new("Script")
                    .with_name("AnimKfs")
                    .with_property("Source", format!("return (function(...)\n{KFS_SRC}\nend)({idle})")),
            );
            ed.set_document(doc);
            // The place's avatar setting decides (an earlier step picked R15).
            ed.play_rig = None;
            ed.start_play(crate::runtime::RunMode::Play);
            self.r6 = super::R6Test {
                started: f,
                mark: ed.log.len(),
                t0: Some(std::time::Instant::now()),
                ..Default::default()
            };
            self.r4.phase = phase + 1;
            return false;
        }
        let r6 = phase == 79;
        let seen = |ed: &Editor, needle: &str| {
            ed.log[self.r6.mark.min(ed.log.len())..]
                .iter()
                .any(|l| l.text.contains(needle))
        };
        // Walk on request: W down until the walk animation shows up.
        if self.r6.ok == false && seen(ed, "anim-hold-w") {
            self.queue.push(key(Key::W, true, Modifiers::NONE));
            self.r6.ok = true;
        }
        if self.r6.ok && seen(ed, "anim-release-w") && self.r6.notes.is_empty() {
            self.queue.push(key(Key::W, false, Modifiers::NONE));
            self.r6.notes.push("released".into());
        }
        let find = |needle: &str| {
            ed.log[self.r6.mark.min(ed.log.len())..]
                .iter()
                .find(|l| l.text.contains(needle))
                .map(|l| l.text.clone())
        };
        let line = find("anim-report");
        let kfs = find("anim-kfs").unwrap_or_default();
        let waited = self.r6.t0.map_or(0, |t| t.elapsed().as_secs());
        if line.is_none() && waited < 200 {
            return false;
        }
        let installed = crate::default_scripts::studio_dir().is_some();
        let text = line.unwrap_or_else(|| "no report from the client".into());
        let field = |k: &str| {
            text.split_whitespace()
                .find_map(|w| w.strip_prefix(k))
                .unwrap_or("")
                .to_string()
        };
        let idle = text
            .split("idle=[")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .unwrap_or("")
            .to_string();
        let walk = text
            .split("walk=[")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .unwrap_or("")
            .to_string();
        let joint: f32 = field("joint=").parse().unwrap_or(0.0);
        let frames: u32 = kfs
            .split_whitespace()
            .find_map(|w| w.strip_prefix("keyframes="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // Roblox's default ids (the Animate scripts in the Studio install).
        let (idle_ids, walk_id) = if r6 {
            (&["180435571", "180435792"][..], "180426354")
        } else {
            (&["507766388", "507766666"][..], "913402848")
        };
        let ok = !installed
            || (field("animator=") == "true"
                && idle_ids.iter().any(|id| idle.contains(id))
                && idle.contains("looped true")
                && walk.contains(walk_id)
                && joint > 1e-3
                && frames > 1
                && field("action=") == "Action/0.50");
        let rig = if r6 { "R6" } else { "R15" };
        self.steps.push(Step {
            step: if r6 { "animation-r6" } else { "animation-r15" },
            passed: ok,
            detail: format!(
                "{rig} character in Play, Roblox's Animate: {}; server {} (want an Animator in the Humanoid, idle {idle_ids:?} looped, walk {walk_id} on Move, the shoulder posed, a decoded KeyframeSequence, an Action track layered at 0.5)",
                text.trim(),
                if kfs.is_empty() { "no anim-kfs line" } else { kfs.trim() }
            ),
        });
        ed.stop_play();
        self.r4.phase = if r6 { 80 } else { 78 };
        false
    }
}

#[cfg(test)]
mod semantics {
    use crate::doc::Document;
    use crate::runtime::{RunMode, Runtime, World};
    use rbx_dom_weak::InstanceBuilder;

    // Registered assets keep this regression independent of network/cache state.
    const PROBE: &str = r#"
local model = Instance.new('Model', workspace)
local ac = Instance.new('AnimationController', model)
local animator = Instance.new('Animator', ac)
local root = Instance.new('Part', model); root.Name = 'Root'
local limb = Instance.new('Part', model); limb.Name = 'Limb'
local motor = Instance.new('Motor6D', root); motor.Part0 = root; motor.Part1 = limb
local function track(x, style, dir)
    local seq = Instance.new('KeyframeSequence'); seq.Loop = true
    for i = 0, 1 do
        local k = Instance.new('Keyframe', seq); k.Time = i; k.Name = 'K' .. i
        local p = Instance.new('Pose', k); p.Name = 'Root'
        local q = Instance.new('Pose', p); q.Name = 'Limb'
        q.CFrame = CFrame.new(if x then x else i, 0, 0)
        q.EasingStyle = style or Enum.PoseEasingStyle.Linear
        q.EasingDirection = dir or Enum.PoseEasingDirection.In
    end
    local anim = Instance.new('Animation')
    anim.AnimationId = game:GetService('KeyframeSequenceProvider'):RegisterKeyframeSequence(seq)
    return animator:LoadAnimation(anim)
end
local function near(a,b) assert(math.abs(a-b) < .025, tostring(a)..' ~= '..tostring(b)) end
local t = track(10)
assert(t.WeightTarget == 0)
local stopped, ended = 0, 0
t.Stopped:Connect(function() stopped += 1 end)
t.Ended:Connect(function() ended += 1 end)
t:Play(0, .4); task.wait(.05)
t:AdjustWeight(.8, 1); task.wait(.5); near(t.WeightCurrent, .6)
task.wait(.55); near(t.WeightCurrent, .8)
t:Stop(1); local before = t.TimePosition
assert(not t.IsPlaying and #animator:GetPlayingAnimationTracks() == 1)
task.wait(.5); near(t.WeightCurrent,.4)
assert(t.TimePosition ~= before and stopped == 1 and ended == 0)
t:Destroy(); task.wait(.55)
assert(#animator:GetPlayingAnimationTracks() == 0)
local core, idle, action = track(10), track(20), track(30)
core.Priority = Enum.AnimationPriority.Core; idle.Priority = Enum.AnimationPriority.Idle
 action.Priority = Enum.AnimationPriority.Action
core:Play(0,1,0); idle:Play(0,.5,0); action:Play(0,.5,0)
task.wait(.05); near(motor.Transform.X,25)
for _,v in animator:GetPlayingAnimationTracks() do v:Stop(0) end; task.wait(.05)
local one, two = track(10), track(20)
one:Play(0,.2,0); two:Play(0,.3,0)
task.wait(.05); near(motor.Transform.X,8)
one:Stop(0); two:Stop(0); task.wait(.05)
for _,spec in {
    {Enum.PoseEasingStyle.Constant, Enum.PoseEasingDirection.In, 1},
    {Enum.PoseEasingStyle.Cubic, Enum.PoseEasingDirection.In, .578125},
    {Enum.PoseEasingStyle.CubicV2, Enum.PoseEasingDirection.In, .015625},
    {Enum.PoseEasingStyle.Bounce, Enum.PoseEasingDirection.Out, .47265625},
    {Enum.PoseEasingStyle.Elastic, Enum.PoseEasingDirection.InOut, .011969447},
} do
    local v = track(nil, spec[1], spec[2]); v:Play(0,1,0); v.TimePosition = .25
    task.wait(.05); near(motor.Transform.X,spec[3]); v:Stop(0); task.wait(.05)
end
local v = track(); v.TimePosition = -.3; near(v.TimePosition,.7)
v.TimePosition = 2.3; near(v.TimePosition,.3)
v.Looped = false; v.TimePosition = 2.3; near(v.TimePosition,1)
local listedAtEnd = false
v.Ended:Connect(function() listedAtEnd = #animator:GetPlayingAnimationTracks() == 1 end)
v:Play(.3); task.wait(1.05)
assert(not v.IsPlaying and v.WeightCurrent > .7 and #animator:GetPlayingAnimationTracks() == 1)
task.wait(.35); assert(listedAtEnd and #animator:GetPlayingAnimationTracks() == 0)
local events = {}
v = track(); v.KeyframeReached:Connect(function(name) table.insert(events,name) end)
v:Play(0); task.wait(1.05); v:Stop(0); task.wait(.05)
assert(table.concat(events, ',') == 'K0,K1,K0',table.concat(events, ','))
v = track(); v.Looped = false; v:Play(0,1,-1); v.TimePosition = .3
task.wait(.35); assert(not v.IsPlaying); near(v.TimePosition,0)
v:Stop(0); task.wait(.1)
print('animation-semantics-pass')
"#;

    #[test]
    fn studio_animation_semantics() {
        run(PROBE);
    }

    #[test]
    fn studio_animation_unloaded_limit() {
        run(r#"
local model = Instance.new('Model',workspace); model.Name = 'Player'
local animator = Instance.new('Animator',Instance.new('AnimationController',model))
local anim = Instance.new('Animation'); anim.AnimationId = 'rbxtemp://missing'
local held = {}
for i = 1,258 do held[i] = animator:LoadAnimation(anim); held[i]:Play(0) end
assert(#animator:GetPlayingAnimationTracks() == 257)
assert(held[258].IsPlaying)
for _,t in held do t:Stop(0) end
task.wait(.05); assert(#animator:GetPlayingAnimationTracks() == 0)
local t = animator:LoadAnimation(anim); t:Play(.1); task.wait(.05)
assert(t.Length == 0 and t.WeightCurrent == 0 and t.WeightTarget == 1)
t:Stop(.1); t:Destroy(); task.wait(.3)
assert(not t.IsPlaying and #animator:GetPlayingAnimationTracks() == 1)
print('animation-semantics-pass')
"#);
    }

    #[test]
    fn studio_animation_curves_rotation_and_reverse() {
        run(r#"
local model = Instance.new('Model',workspace)
local animator = Instance.new('Animator',Instance.new('AnimationController',model))
local root=Instance.new('Part',model);root.Name='Root'
local limb=Instance.new('Part',model);limb.Name='Limb'
local motor=Instance.new('Motor6D',root);motor.Part0=root;motor.Part1=limb
local function near(a,b) assert(math.abs(a-b)<.0002,tostring(a)..' ~= '..tostring(b)) end
local function load(clip)
 local a=Instance.new('Animation')
 a.AnimationId=__rs_lua_methods.RegisterAnimationClip(nil,clip)
 return animator:LoadAnimation(a)
end
local function pose(cf,w)
 local s=Instance.new('KeyframeSequence');s.Loop=true
 for i=0,1 do
  local k=Instance.new('Keyframe',s);k.Time=i;k.Name='K'..i
  local m=Instance.new('KeyframeMarker',k);m.Name='M';m.Value=k.Name
  local p=Instance.new('Pose',k);p.Name='Root'
  local q=Instance.new('Pose',p);q.Name='Limb';q.CFrame=cf;q.Weight=w
 end
 return load(s)
end
local a=pose(CFrame.new(10,0,0)*CFrame.Angles(0,0,math.rad(120)),.25)
a:Play(0,1,0);task.wait(.05);near(motor.Transform.X,10)
near(motor.Transform.RightVector.X,-.5);a:Stop(0);task.wait(.05)
local a,b=pose(CFrame.Angles(math.rad(120),0,0),1),pose(CFrame.Angles(0,math.rad(100),0),1)
a:Play(0,.2,0);b:Play(0,.3,0);task.wait(.05)
near(motor.Transform.RightVector.X,.81709647)
near(motor.Transform.UpVector.X,.13784724)
a:Stop(0);b:Stop(0);task.wait(.05)
local a,b,c=pose(CFrame.Angles(math.rad(120),0,0),1),pose(CFrame.Angles(0,math.rad(100),0),1),pose(CFrame.Angles(0,0,math.rad(80)),1)
a.Priority=Enum.AnimationPriority.Action;b.Priority=Enum.AnimationPriority.Action;c.Priority=Enum.AnimationPriority.Idle
a:Play(0,.2,0);b:Play(0,.3,0);c:Play(0,.5,0);task.wait(.05)
near(motor.Transform.RightVector.X,.51433307)
for _,t in animator:GetPlayingAnimationTracks() do t:Stop(0) end;task.wait(.05)
local seq=Instance.new('KeyframeSequence');seq.Loop=true
for i=0,1 do
 local k=Instance.new('Keyframe',seq);k.Time=i
 local p=Instance.new('Pose',k);p.Name='Root'
 local q=Instance.new('Pose',p);q.Name='Limb'
 q.CFrame=if i==0 then CFrame.Angles(math.rad(120),0,0) else CFrame.Angles(0,math.rad(100),0)
 q.EasingStyle=Enum.PoseEasingStyle.Linear
end
local t=load(seq);t:Play(0,1,0);t.TimePosition=.25;task.wait(.05)
near(motor.Transform.RightVector.X,.90160203);t:Stop(0);task.wait(.05)
local c=workspace:FindFirstChild('CurveFixture')
local t=load(c);near(t.Length,1);t:Play(0,1,0);t.TimePosition=.25;task.wait(.05)
near(motor.Transform.X,2.875);near(motor.Transform.Y,7);near(motor.Transform.RightVector.X,math.cos(.5))
near(motor.Transform.RightVector.Y,math.sin(.5)*math.cos(1))
t:Stop(0);task.wait(.05)
local hits={};t:GetMarkerReachedSignal('Events'):Connect(function(v) table.insert(hits,v) end)
t:Play(0);task.wait(.3);t:Stop(0);task.wait(.05)
assert(table.concat(hits,',')=='Hit',table.concat(hits,','))
c.Root.Limb.Rotation.RotationOrder=Enum.RotationOrder.ZYX
local t=load(c);t:Play(0,1,0);t.TimePosition=.25;task.wait(.05)
near(motor.Transform.RightVector.Y,math.sin(.5));t:Stop(0);task.wait(.05)
for _,case in {{.1,.15,'MK0,MK1,K0,K1,LOOP'},{1/60,.05,'MK0,K0,MK1,K1,LOOP'}} do
local t=pose(CFrame.new(),1);local ev={}
t.KeyframeReached:Connect(function(n) table.insert(ev,n) end)
t:GetMarkerReachedSignal('M'):Connect(function(n) table.insert(ev,'M'..n) end)
t.DidLoop:Connect(function() table.insert(ev,'LOOP') end)
t:Play(0,1,-1);t.TimePosition=case[1];task.wait(case[2]);t:Stop(0);task.wait(.05)
assert(table.concat(ev,',')==case[3],table.concat(ev,','))
end
print('animation-semantics-pass')
"#);
    }

    fn run(source: &str) {
        let mut d = Document::new_place();
        // Native serialized fixture: scripts cannot assign BinaryString properties.
        let ws = d.find_child(d.dom.root_ref(), "Workspace").unwrap();
        let clip = d.dom.insert(
            ws,
            InstanceBuilder::new("CurveAnimation")
                .with_name("CurveFixture")
                .with_property("Loop", true),
        );
        let root = d
            .dom
            .insert(clip, InstanceBuilder::new("Folder").with_name("Root"));
        let limb = d
            .dom
            .insert(root, InstanceBuilder::new("Folder").with_name("Limb"));
        let pos = d.dom.insert(
            limb,
            InstanceBuilder::new("Vector3Curve").with_name("Position"),
        );
        let rot = d.dom.insert(
            limb,
            InstanceBuilder::new("EulerRotationCurve").with_name("Rotation"),
        );
        fn wire(
            mode: u16,
            values: [f32; 2],
            tangents: [f32; 2],
        ) -> rbx_dom_weak::types::BinaryString {
            let mut b = Vec::new();
            b.extend(2u32.to_le_bytes());
            b.extend(2u32.to_le_bytes());
            for (i, v) in values.into_iter().enumerate() {
                b.extend(mode.to_le_bytes());
                b.extend(v.to_le_bytes());
                b.extend((if i == 1 { tangents[0] } else { 0.0 }).to_le_bytes());
                b.extend((if i == 0 { tangents[1] } else { 0.0 }).to_le_bytes());
            }
            for v in [1i32, 2, 0, 2400] {
                b.extend(v.to_le_bytes());
            }
            b.into()
        }
        d.dom.insert(
            pos,
            InstanceBuilder::new("FloatCurve")
                .with_name("X")
                .with_property("ValuesAndTimes", wire(2, [2., 4.], [0., 4.])),
        );
        d.dom.insert(
            pos,
            InstanceBuilder::new("FloatCurve")
                .with_name("Y")
                .with_property("ValuesAndTimes", wire(0, [7., 9.], [0., 0.])),
        );
        let mut markers = Vec::new();
        for v in [2u32, 1] {
            markers.extend(v.to_le_bytes());
        }
        markers.extend(b"Hit\0");
        for v in [1i32, 1, 600] {
            markers.extend(v.to_le_bytes());
        }
        d.dom.insert(
            clip,
            InstanceBuilder::new("MarkerCurve")
                .with_name("Events")
                .with_property(
                    "ValuesAndTimes",
                    rbx_dom_weak::types::BinaryString::from(markers),
                ),
        );
        d.dom.insert(
            rot,
            InstanceBuilder::new("FloatCurve")
                .with_name("X")
                .with_property("ValuesAndTimes", wire(2, [1., 1.], [0., 0.])),
        );
        d.dom.insert(
            rot,
            InstanceBuilder::new("FloatCurve")
                .with_name("Z")
                .with_property("ValuesAndTimes", wire(1, [0., 2.], [0., 0.])),
        );
        let parent = d
            .find_child(d.dom.root_ref(), "ServerScriptService")
            .unwrap();
        let s = d.dom.insert(
            parent,
            InstanceBuilder::new("Script")
                .with_name("AnimationSemantics")
                .with_property("Source", source.to_string()),
        );
        let rt = Runtime::new().unwrap();
        rt.enter(World::new(d, RunMode::Run));
        rt.bind_globals().unwrap();
        rt.run_script(s);
        let rs = {
            let w = rt.world_mut().unwrap();
            w.d()
                .find_child(w.d().dom.root_ref(), "RunService")
                .unwrap()
        };
        for _ in 0..540 {
            rt.fire(rs, "PreAnimation", vec![mlua::Value::Number(1.0 / 60.0)]);
            rt.step(1.0 / 60.0);
        }
        let output = rt.leave().output;
        assert!(
            output
                .iter()
                .any(|(_, s)| s.contains("animation-semantics-pass")),
            "{output:?}"
        );
    }
}
