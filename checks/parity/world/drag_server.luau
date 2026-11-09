-- DragDetector probe (server): the Studio measurement layout (checks/parity/world.md)
-- and the server's event log.
for _, c in workspace:GetChildren() do
	if c:IsA("BasePart") and c.Name ~= "Baseplate" and c.Name ~= "SpawnLocation" and not c:IsA("Terrain") then c:Destroy() end
end
if workspace:FindFirstChild("SpawnLocation") then workspace.SpawnLocation.CFrame = CFrame.new(0, 0.5, 20) end
local F = Instance.new("Folder") F.Name = "Drag"
local specs = {
	{ "TPlane", Vector3.new(-12, 2, -6), "TranslatePlane", "Geometric", true },
	{ "TLine", Vector3.new(-6, 2, -6), "TranslateLine", "Geometric", true },
	{ "TView", Vector3.new(0, 2, -6), "TranslateViewPlane", "Geometric", true },
	{ "RAxis", Vector3.new(6, 2, -6), "RotateAxis", "Geometric", true },
	{ "RTrack", Vector3.new(12, 2, -6), "RotateTrackball", "Geometric", true },
	{ "PhysPlane", Vector3.new(-12, 2, 2), "TranslatePlane", "Physical", false },
	{ "GeoLoose", Vector3.new(-6, 2, 2), "TranslatePlane", "Geometric", false },
	{ "Custom", Vector3.new(0, 2, 2), "TranslatePlane", "Custom", true },
	{ "PhysRot", Vector3.new(6, 2, 2), "RotateAxis", "Physical", false },
	{ "Script", Vector3.new(12, 2, 2), "Scriptable", "Geometric", true },
}
for _, s in specs do
	local p = Instance.new("Part") p.Name = s[1] p.Size = Vector3.new(3, 3, 3) p.Position = s[2] + Vector3.new(0, -0.5, 0) p.Anchored = s[5] p.Parent = F
	local d = Instance.new("DragDetector") d.DragStyle = Enum.DragDetectorDragStyle[s[3]] d.ResponseStyle = Enum.DragDetectorResponseStyle[s[4]] d.Parent = p
end
F.Parent = workspace
local function f(v)
	if typeof(v) == "CFrame" then local x, y, z = v:ToEulerAnglesXYZ() return string.format("CF(%.3f,%.3f,%.3f|%.1f,%.1f,%.1f)", v.X, v.Y, v.Z, math.deg(x), math.deg(y), math.deg(z))
	elseif typeof(v) == "Ray" then return string.format("Ray(%.2f,%.2f,%.2f>%.3f,%.3f,%.3f)", v.Origin.X, v.Origin.Y, v.Origin.Z, v.Direction.X, v.Direction.Y, v.Direction.Z)
	elseif typeof(v) == "Instance" then return v.Name end
	return tostring(v)
end
for _, p in F:GetChildren() do
	local d = p.DragDetector
	for _, ev in { "DragStart", "DragContinue", "DragEnd" } do
		d[ev]:Connect(function(...)
			local a = table.pack(...) local s = {} for i = 1, a.n do s[#s + 1] = f(a[i]) end
			print(string.format("DRAG S %s %s n=%d [%s] DF=%s P=%s A=%s", p.Name, ev, a.n, table.concat(s, "; "), f(d.DragFrame), f(p.CFrame), tostring(p.Anchored)))
		end)
	end
end
-- The script-driven cases (same as the Studio session's DragFns script).
local sd = F.Script.DragDetector
sd:SetDragStyleFunction(function(ray)
	print("DRAG STYLEFN", typeof(ray), string.format("%.2f", ray.Direction.Magnitude))
	return CFrame.new(12, 1.5, 2) * CFrame.new(ray.Direction.X / 200, 0, 0)
end)
local tp = F.TPlane.DragDetector
tp:AddConstraintFunction(1, function(cf) print("DRAG CONSTR1", cf.X) return CFrame.new(math.floor(cf.X + 0.5), cf.Y, 0) end)
tp:AddConstraintFunction(5, function(cf) print("DRAG CONSTR5", cf.X, cf.Z) return cf end)
local cd = F.Custom.DragDetector
cd.PermissionPolicy = Enum.DragDetectorPermissionPolicy.Scriptable
cd:SetPermissionPolicyFunction(function(player, part) print("DRAG PERM", player, part) return false end)
local tv = F.TView.DragDetector
tv.MaxDragTranslation = Vector3.new(1, 1, 1) tv.MinDragTranslation = Vector3.new(-1, -1, -1)
