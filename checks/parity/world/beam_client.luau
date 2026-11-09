-- Beam:SetTextureOffset probe (client): three beams, offsets nil / 0.25 / 0.5,
-- the Studio session's layout (checks/parity/world.md).
task.spawn(function()
	local lp = game.Players.LocalPlayer
	local ch = lp.Character or lp.CharacterAdded:Wait()
	ch:WaitForChild("HumanoidRootPart").CFrame = CFrame.new(60, 3, 60) ch.HumanoidRootPart.Anchored = true
	for _, c in workspace:GetChildren() do if c:IsA("BasePart") and c.Name ~= "Baseplate" and not c:IsA("Terrain") then c:Destroy() end end
	local F = Instance.new("Folder", workspace) F.Name = "WF"
	local cam = workspace.CurrentCamera
	game:GetService("RunService"):BindToRenderStep("pin", Enum.RenderPriority.Last.Value, function()
		cam.CameraType = Enum.CameraType.Scriptable
		cam.CFrame = CFrame.lookAt(Vector3.new(0, 6, 14), Vector3.new(0, 2, -1))
		cam.FieldOfView = 60
	end)
	local function beam(y, off, name)
		local p = Instance.new("Part") p.Name = name p.Anchored = true p.Transparency = 1 p.CanCollide = false p.Position = Vector3.new(0, y, 0) p.Parent = F
		local a0 = Instance.new("Attachment", p) a0.Position = Vector3.new(-6, 0, 0)
		local a1 = Instance.new("Attachment", p) a1.Position = Vector3.new(6, 0, 0)
		local b = Instance.new("Beam") b.Attachment0 = a0 b.Attachment1 = a1 b.Texture = "rbxasset://textures/explosion.png" b.TextureSpeed = 0 b.TextureLength = 1
		b.TextureMode = Enum.TextureMode.Stretch b.Width0 = 2 b.Width1 = 2 b.FaceCamera = true b.LightEmission = 1 b.LightInfluence = 0 b.Parent = p
		if off then b:SetTextureOffset(off) end
	end
	beam(7, nil, "B0") beam(4.5, 0.25, "B25") beam(2, 0.5, "B50")
	print("BEAM ready")
end)
