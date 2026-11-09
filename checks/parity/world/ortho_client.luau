-- Orthographic camera probe (client): the scene of the flagged Studio
-- session (checks/parity/world.md "Orthographic"), projected points printed.
task.spawn(function()
	local lp = game.Players.LocalPlayer
	local ch = lp.Character or lp.CharacterAdded:Wait()
	ch:WaitForChild("HumanoidRootPart").CFrame = CFrame.new(60, 3, 60) ch.HumanoidRootPart.Anchored = true
	for _, c in workspace:GetChildren() do if c:IsA("BasePart") and c.Name ~= "Baseplate" and not c:IsA("Terrain") then c:Destroy() end end
	local F = Instance.new("Folder", workspace) F.Name = "WF"
	for i = 0, 4 do local p = Instance.new("Part") p.Anchored = true p.Size = Vector3.new(2, 2, 2) p.Position = Vector3.new(-8 + i * 4, 1, -i * 6) p.Color = Color3.fromHSV(i / 5, 0.8, 0.9) p.Parent = F end
	local g = Instance.new("Part") g.Anchored = true g.Size = Vector3.new(1, 10, 1) g.Position = Vector3.new(0, 5, 0) g.Color = Color3.new(1, 1, 1) g.Parent = F
	local cam = workspace.CurrentCamera
	game:GetService("RunService"):BindToRenderStep("pin", Enum.RenderPriority.Last.Value, function()
		cam.CameraType = Enum.CameraType.Scriptable
		cam.CFrame = CFrame.lookAt(Vector3.new(0, 6, 14), Vector3.new(0, 2, -1))
		cam.FieldOfView = 60
	end)
	cam.ProjectionType = Enum.ProjectionType.Orthographic
	cam.OrthographicSize = 20
	task.wait(0.5)
	local function p(v) local s = cam:WorldToViewportPoint(v) return string.format("(%.1f,%.1f,%.3f)", s.X, s.Y, s.Z) end
	local vs = cam.ViewportSize
	print("ORTHO", cam.ProjectionType, cam.OrthographicSize, vs)
	local a = cam:WorldToViewportPoint(Vector3.zero) local b = cam:WorldToViewportPoint(Vector3.new(5, 0, 0))
	print(string.format("ORTHO pxPerStud %.4f extent v %.3f", (b.X - a.X) / 5, vs.Y / ((b.X - a.X) / 5)))
	print("ORTHO pole top", p(Vector3.new(0, 10, 0)), "base", p(Vector3.zero))
	local r = cam:ViewportPointToRay(100, 100) print("ORTHO ray", r.Origin, r.Direction)
	cam.OrthographicSize = -5 print("ORTHO clamp", cam.OrthographicSize) cam.OrthographicSize = 20
end)
