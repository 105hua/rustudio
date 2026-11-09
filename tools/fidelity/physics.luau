-- Fidelity physics facts, run on the client in both engines: the local
-- character's resting height, walk speed, jump apex and fall time,
-- measured the same way. Prints one line: FIDPHYS|<json>
-- (or FIDPHYS|{"skipped": reason} when the place has no character).
local HttpService = game:GetService("HttpService")
local RunService = game:GetService("RunService")
local Players = game:GetService("Players")

task.spawn(function()
    local me = Players.LocalPlayer
    local char = me and me.Character
    local t0 = os.clock()
    while not char and os.clock() - t0 < 5 do
        task.wait(0.1)
        char = me and me.Character
    end
    if not char then
        print("FIDPHYS|" .. HttpService:JSONEncode({ skipped = "no character" }))
        return
    end
    local hum = char:FindFirstChildOfClass("Humanoid")
    local root = char:FindFirstChild("HumanoidRootPart")
    if not hum or not root then
        print("FIDPHYS|" .. HttpService:JSONEncode({ skipped = "no Humanoid / HumanoidRootPart" }))
        return
    end
    -- The PlayerModule's controls would overwrite Humanoid:Move each frame.
    local controls
    pcall(function()
        controls = require(me:WaitForChild("PlayerScripts"):WaitForChild("PlayerModule", 2)):GetControls()
        controls:Disable()
    end)
    local function settle(seconds)
        local t = os.clock()
        while os.clock() - t < seconds do RunService.Heartbeat:Wait() end
    end
    settle(1)
    local facts = { walkSpeed = hum.WalkSpeed, jumpPower = hum.JumpPower, jumpHeight = hum.JumpHeight,
        useJumpPower = hum.UseJumpPower, hipHeight = hum.HipHeight, rigType = hum.RigType.Name,
        gravity = workspace.Gravity }

    local params = RaycastParams.new()
    params.FilterDescendantsInstances = { char }
    params.FilterType = Enum.RaycastFilterType.Exclude
    local function floorBelow()
        local hit = workspace:Raycast(root.Position, Vector3.new(0, -50, 0), params)
        return hit and hit.Position.Y
    end
    local fy = floorBelow()
    facts.restingHeight = fy and (root.Position.Y - fy) or nil

    -- Walk: MoveTo 60 studs toward +Z, the open side of the corpus places
    -- (Studio's plugin context doesn't drive Humanoid:Move, MoveTo works in
    -- both); speed from 0.5 s to 1.5 s.
    local a0 = root.Position
    hum:MoveTo(a0 + Vector3.new(0, 0, 60))
    local t = os.clock()
    local p0
    while os.clock() - t < 1.5 do
        RunService.Heartbeat:Wait()
        if not p0 and os.clock() - t >= 0.5 then p0 = root.Position; t0 = os.clock() end
    end
    local p1 = root.Position
    facts.walkMeasured = p0 and (Vector3.new(p1.X - p0.X, 0, p1.Z - p0.Z).Magnitude / (os.clock() - t0)) or nil
    hum:MoveTo(root.Position)
    settle(1)

    -- Jump apex above the resting root (ChangeState Jumping).
    local y0 = root.Position.Y
    local top = y0
    hum:ChangeState(Enum.HumanoidStateType.Jumping)
    t = os.clock()
    while os.clock() - t < 1.5 do
        RunService.Heartbeat:Wait()
        top = math.max(top, root.Position.Y)
    end
    facts.jumpApex = top - y0
    settle(0.75)

    -- Fall: lifted 30 studs, time until standing again.
    local start = root.Position.Y
    char:PivotTo(char:GetPivot() + Vector3.new(0, 30, 0))
    RunService.Heartbeat:Wait()
    local t = os.clock()
    local landed
    while os.clock() - t < 5 do
        RunService.Heartbeat:Wait()
        if root.Position.Y <= start + 0.05 or hum.FloorMaterial ~= Enum.Material.Air and os.clock() - t > 0.1 then
            landed = os.clock() - t
            break
        end
    end
    facts.fallTime30 = landed
    facts.fallTheory30 = math.sqrt(2 * 30 / workspace.Gravity)
    if controls then pcall(function() controls:Enable() end) end
    print("FIDPHYS|" .. HttpService:JSONEncode(facts))
end)
