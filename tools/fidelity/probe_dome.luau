-- Round 26 probe: what Studio's client holds for Place B Newer's Dome (N3,
-- translucent pink in Studio, opaque red in rustudio): per Dome model and
-- part group, the count, transparency, local transparency modifier, size,
-- distance from the dome's centre; and the streaming settings.
local c = Vector3.new(2645.0889, 134, 1276.1915)
print("PROBEDOME streaming", workspace.StreamingEnabled)
for _, d in workspace:GetChildren() do
	if d.Name == "Dome" then
		local g = {}
		for _, p in d:GetDescendants() do
			if p:IsA("BasePart") then
				local k = string.format("%s:%s:T%.2f:LT%.2f:%s:%s", p.ClassName, p.Material.Name, p.Transparency,
					p.LocalTransparencyModifier, tostring(p.BrickColor), tostring(p.Size))
				local e = g[k] or { n = 0, rmin = 1e9, rmax = 0 }
				local r = (p.Position - c).Magnitude
				e.n += 1
				e.rmin = math.min(e.rmin, r)
				e.rmax = math.max(e.rmax, r)
				g[k] = e
			elseif not p:IsA("Model") then
				print("PROBEDOME other", p:GetFullName(), p.ClassName)
			end
		end
		for k, e in g do
			print("PROBEDOME", d:GetPivot().Position.Y, k, e.n, string.format("r %.3f..%.3f", e.rmin, e.rmax))
		end
	end
end
