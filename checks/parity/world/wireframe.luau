-- WireframeHandleAdornment scene (same code in Studio's command bar and as
-- a rustudio client --luau): lines, a path, text, AlwaysOnTop on/off.
for _, c in workspace:GetChildren() do
	if (c:IsA("BasePart") and c.Name ~= "Baseplate" and not c:IsA("Terrain")) or c.Name == "Drag" or c.Name == "WF" then c:Destroy() end
end
local F = Instance.new("Folder") F.Name = "WF" F.Parent = workspace
local wall = Instance.new("Part") wall.Name = "Wall" wall.Anchored = true wall.Size = Vector3.new(6, 6, 1) wall.CFrame = CFrame.new(4, 3, 0) wall.Color = Color3.fromRGB(90, 90, 90) wall.Parent = F
local host = Instance.new("Part") host.Name = "Host" host.Anchored = true host.Size = Vector3.new(2, 2, 2) host.CFrame = CFrame.new(-6, 1, -2) * CFrame.Angles(0, math.rad(30), 0) host.Parent = F
-- 1: on the host part, a box outline in its frame (yellow, AlwaysOnTop false).
local a = Instance.new("WireframeHandleAdornment") a.Adornee = host a.Color3 = Color3.new(1, 1, 0) a.AlwaysOnTop = false a.Parent = F
local s = 1.5
local c = { Vector3.new(-s,-s,-s), Vector3.new(s,-s,-s), Vector3.new(s,-s,s), Vector3.new(-s,-s,s), Vector3.new(-s,s,-s), Vector3.new(s,s,-s), Vector3.new(s,s,s), Vector3.new(-s,s,s) }
for _, e in { {1,2},{2,3},{3,4},{4,1},{5,6},{6,7},{7,8},{8,5},{1,5},{2,6},{3,7},{4,8} } do a:AddLine(c[e[1]], c[e[2]]) end
a:AddText(Vector3.new(0, 2.5, 0), "Host", 18)
-- 2: no adornee: world space; a path behind the wall, depth-tested (red).
local b = Instance.new("WireframeHandleAdornment") b.Color3 = Color3.new(1, 0, 0) b.AlwaysOnTop = false b.Parent = F
b:AddPath({ Vector3.new(1, 1, -3), Vector3.new(4, 5, -3), Vector3.new(7, 1, -3) }, true)
-- 3: the same behind the wall, AlwaysOnTop (cyan), with AddLines pairs.
local t = Instance.new("WireframeHandleAdornment") t.Color3 = Color3.new(0, 1, 1) t.AlwaysOnTop = true t.Parent = F
t:AddLines({ Vector3.new(1, 3, -2), Vector3.new(7, 3, -2), Vector3.new(4, 0.5, -2), Vector3.new(4, 5.5, -2) })
t:AddText(Vector3.new(4, 6.5, -2), "Top", 14)
-- 4: cleared (nothing drawn).
local x = Instance.new("WireframeHandleAdornment") x.Color3 = Color3.new(1, 0, 1) x.Parent = F
x:AddLine(Vector3.new(-8, 4, 0), Vector3.new(8, 4, 0)) x:Clear()
local cam = workspace.CurrentCamera
cam.CameraType = Enum.CameraType.Scriptable
cam.CFrame = CFrame.lookAt(Vector3.new(0, 6, 14), Vector3.new(0, 2, -1))
cam.FieldOfView = 60
print("WF ready")
