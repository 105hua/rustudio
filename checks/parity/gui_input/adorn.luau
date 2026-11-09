-- HandleAdornment / Handles / ArcHandles mouse events (gui_input.md).
task.spawn(function()
local pg = game.Players.LocalPlayer:WaitForChild("PlayerGui")
local b = Instance.new("BoxHandleAdornment") b.Adornee = workspace:WaitForChild("Target") b.Size = Vector3.new(4,4,4) b.Transparency = 0.5 b.Parent = pg
for _, e in {"MouseEnter","MouseLeave","MouseButton1Down","MouseButton1Up"} do b[e]:Connect(function(...) print("ADORN box "..e.." n="..select("#", ...)) end) end
local h = Instance.new("Handles") h.Adornee = workspace:WaitForChild("Target2") h.Parent = pg
for _, e in {"MouseEnter","MouseLeave","MouseButton1Down","MouseButton1Up","MouseDrag"} do h[e]:Connect(function(...) local a = {...} for i,v in a do a[i] = tostring(v) end print("ADORN handles "..e.." "..table.concat(a, ",")) end) end
task.wait(0.5)
local cam = workspace.CurrentCamera
local function sp(v) local p = cam:WorldToViewportPoint(v) return math.floor(p.X)..","..math.floor(p.Y) end
print("ADORN at box "..sp(workspace.Target.Position).." top "..sp(workspace.Target2.Position + Vector3.new(0,4,0)).." vp "..tostring(cam.ViewportSize))
end)
