-- DragDetector probe (client): the Studio camera, the character in reach,
-- screen positions of the parts, and the client's event log.
task.spawn(function()
	local F = workspace:WaitForChild("Drag")
	local cam = workspace.CurrentCamera
	local lp = game.Players.LocalPlayer
	local ch = lp.Character or lp.CharacterAdded:Wait()
	ch:WaitForChild("HumanoidRootPart").CFrame = CFrame.new(0, 3, 21)
	ch.HumanoidRootPart.Anchored = true
	game:GetService("RunService"):BindToRenderStep("pin", Enum.RenderPriority.Last.Value, function()
		cam.CameraType = Enum.CameraType.Scriptable
		cam.CFrame = CFrame.lookAt(Vector3.new(0, 14, 16), Vector3.new(0, 1, -2))
	end)
	task.wait(0.5)
	local function f(v)
		if typeof(v) == "CFrame" then local x, y, z = v:ToEulerAnglesXYZ() return string.format("CF(%.3f,%.3f,%.3f|%.1f,%.1f,%.1f)", v.X, v.Y, v.Z, math.deg(x), math.deg(y), math.deg(z))
		elseif typeof(v) == "Ray" then return string.format("Ray(%.2f,%.2f,%.2f>%.3f,%.3f,%.3f)", v.Origin.X, v.Origin.Y, v.Origin.Z, v.Direction.X, v.Direction.Y, v.Direction.Z)
		elseif typeof(v) == "Instance" then return v.Name end
		return tostring(v)
	end
	local s = {}
	for _, p in F:GetChildren() do
		local v = cam:WorldToViewportPoint(p.Position)
		s[#s + 1] = string.format("%s=%d,%d(%.1f,%.1f,%.1f)", p.Name, v.X, v.Y, p.Position.X, p.Position.Y, p.Position.Z)
		local d = p:WaitForChild("DragDetector")
		for _, ev in { "DragStart", "DragContinue", "DragEnd" } do
			d[ev]:Connect(function(...)
				local a = table.pack(...) local t = {} for i = 1, a.n do t[#t + 1] = f(a[i]) end
				print(string.format("DRAG C %s %s n=%d [%s] DF=%s P=%s", p.Name, ev, a.n, table.concat(t, "; "), f(d.DragFrame), f(p.CFrame)))
			end)
		end
	end
	print("DRAG SCREEN " .. table.concat(s, " ") .. " vp=" .. tostring(cam.ViewportSize))
end)
