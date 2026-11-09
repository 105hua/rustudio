-- Fidelity capture: the same code runs on the client in real Studio
-- (offline Studio, run_luau) and in rustudio (--luau client). It prints the place's
-- state as JSON in short lines (Studio's log cuts lines at ~950 chars):
--   FIDCAP|<i>|<n>|<chunk>
-- tools/fidelity/fidelity.py reassembles and compares them.
local HttpService = game:GetService("HttpService")
local Lighting = game:GetService("Lighting")

local function r3(x) return math.floor(x * 1000 + 0.5) / 1000 end
local function v3(v) return { r3(v.X), r3(v.Y), r3(v.Z) } end
local function c3(c) return { math.floor(c.R * 255 + 0.5), math.floor(c.G * 255 + 0.5), math.floor(c.B * 255 + 0.5) } end

local out = { services = {}, parts = {}, classes = {}, lighting = {}, camera = {} }

for _, s in game:GetChildren() do
    table.insert(out.services, s.ClassName)
end
table.sort(out.services)

local players = game:GetService("Players")
local me = players.LocalPlayer
-- Games name things after the player's UserId; the two engines sign in as
-- different accounts, so the compare swaps it for a placeholder.
out.userId = me and me.UserId or 0
local count = 0
for _, d in workspace:GetDescendants() do
    out.classes[d.ClassName] = (out.classes[d.ClassName] or 0) + 1
    if d:IsA("BasePart") and count < 2500 then
        -- The local character moves; it's compared through the physics facts.
        local char = me and me.Character
        if not (char and d:IsDescendantOf(char)) then
            count += 1
            local rx, ry, rz = d.CFrame:ToOrientation()
            table.insert(out.parts, {
                d:GetFullName(), d.ClassName, v3(d.Position), v3(d.Size),
                { r3(math.deg(rx)), r3(math.deg(ry)), r3(math.deg(rz)) },
                c3(d.Color), d.Material.Name, r3(d.Transparency),
            })
        end
    end
end
table.sort(out.parts, function(a, b) return a[1] < b[1] end)

local L = out.lighting
for _, k in { "ClockTime", "Brightness", "ExposureCompensation", "EnvironmentDiffuseScale",
    "EnvironmentSpecularScale", "FogStart", "FogEnd", "GeographicLatitude", "ShadowSoftness" } do
    local ok, v = pcall(function() return Lighting[k] end)
    if ok then L[k] = r3(v) end
end
for _, k in { "Ambient", "OutdoorAmbient", "FogColor", "ColorShift_Top", "ColorShift_Bottom" } do
    local ok, v = pcall(function() return Lighting[k] end)
    if ok then L[k] = c3(v) end
end
local okT, tech = pcall(function() return Lighting.Technology.Name end)
L.Technology = okT and tech or "?"
L.GlobalShadows = Lighting.GlobalShadows
-- What Studio draws with (Technology itself isn't readable from a plugin).
local okS, style = pcall(function() return Lighting.LightingStyle.Name end)
if okS then L.LightingStyle = style end
local okQ, quality = pcall(function() return Lighting.PrioritizeLightingQuality end)
if okQ then L.PrioritizeLightingQuality = quality end

-- The local character: its parts (avatar) and what its Animator plays.
out.character = { parts = {}, tracks = {} }
local char = me and me.Character
if char then
    local hum = char:FindFirstChildOfClass("Humanoid")
    out.character.rig = hum and hum.RigType.Name or ""
    for _, d in char:GetDescendants() do
        if d:IsA("BasePart") then
            local mesh = ""
            pcall(function() mesh = d.MeshId end)
            table.insert(out.character.parts, { d.Name, d.ClassName, v3(d.Size), c3(d.Color), (string.match(mesh, "%d+$") or "") })
        end
    end
    table.sort(out.character.parts, function(a, b) return a[1] < b[1] end)
    local animator = hum and hum:FindFirstChildOfClass("Animator")
    if animator then
        for _, tr in animator:GetPlayingAnimationTracks() do
            local id = tr.Animation and string.match(tr.Animation.AnimationId, "%d+$") or "?"
            -- The Animate slot it came from ("idle", "walk"...): Animate picks
            -- among a slot's animations at random, so slots are compared.
            local slot = tr.Animation and tr.Animation.Parent and tr.Animation.Parent.Name or "?"
            table.insert(out.character.tracks, { id, tr.Priority.Name, tr.Looped, slot })
        end
        table.sort(out.character.tracks, function(a, b) return a[1] < b[1] end)
    end
end

-- Frame time over one second (RenderStepped deltas).
local deltas = {}
local conn = game:GetService("RunService").RenderStepped:Connect(function(dt) table.insert(deltas, dt) end)
local tw = os.clock()
while os.clock() - tw < 1 do game:GetService("RunService").Heartbeat:Wait() end
conn:Disconnect()
local sum = 0
for _, d in deltas do sum += d end
out.frame = { frames = #deltas, mean_ms = #deltas > 0 and r3(sum / #deltas * 1000) or 0 }

-- Both engines read the camera at the same game time (time() since Play):
-- a camera driven by the clock (Place A's menu sways on os.clock, put in phase
-- by RUSTUDIO_CLOCK_BASE) is then compared at the same moment. Round 13's
-- camera_controls drop (look 2.18° apart) was the two captures reading it
-- whenever they happened to run.
-- (A corpus place can move it: corpus.json "camera_at".)
local CAMERA_AT = FID_CAMERA_AT or 12.5
while time() < CAMERA_AT do game:GetService("RunService").Heartbeat:Wait() end

-- The game clocks at this moment: os.clock (each engine's own origin) and
-- time() (since Play), so the harness can put rustudio's os.clock in phase.
out.clock = os.clock()
out.gameTime = time()
local cam = workspace.CurrentCamera
if cam then
    out.camera = {
        position = v3(cam.CFrame.Position),
        look = v3(cam.CFrame.LookVector),
        fov = r3(cam.FieldOfView),
        viewport = { r3(cam.ViewportSize.X), r3(cam.ViewportSize.Y) },
        type = cam.CameraType.Name,
        subject = cam.CameraSubject and cam.CameraSubject:GetFullName() or "",
    }
    -- Where the camera sits from its subject's root (spawn spots differ
    -- run to run: Studio picks a random point on the SpawnLocation).
    local subj = cam.CameraSubject
    local model = subj and (subj:IsA("Humanoid") and subj.Parent or subj)
    local root = model and model:FindFirstChild("HumanoidRootPart")
    if root then
        out.camera.offset = v3(cam.CFrame.Position - root.Position)
    end
end

local text = HttpService:JSONEncode(out)
local size = 700
local n = math.ceil(#text / size)
for i = 1, n do
    print(string.format("FIDCAP|%d|%d|%s", i, n, string.sub(text, (i - 1) * size + 1, i * size)))
end
