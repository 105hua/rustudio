-- Fidelity: Place A compared during a match, run on the client in both engines
-- a few seconds after Play. It readies the player's party through Place A's own
-- "Party" remote (what the menu's START GAME sends: Menu -> Party.send
-- ("ready", true)), waits for the character and a enemy, then keeps the
-- view on the nearest enemy in sight so the captures have enemies in view.
-- Prints FIDMATCH|{json} once the match is on screen.
task.spawn(function()
	local H = game:GetService("HttpService")
	local RS = game:GetService("ReplicatedStorage")
	local me = game:GetService("Players").LocalPlayer
	local party = RS:WaitForChild("Party", 30)
	if not party then
		print("FIDMATCH|" .. H:JSONEncode({ started = false, why = "no Party remote" }))
		return
	end
	party:FireServer("ready", true)
	local t0 = time()
	local root, folder
	while time() - t0 < 150 do
		local c = me.Character
		root = c and c:FindFirstChild("HumanoidRootPart")
		folder = workspace:FindFirstChild("ZombiesClient")
		if root and me:GetAttribute("Stance") and folder and #folder:GetChildren() > 0 then break end
		task.wait(0.25)
	end
	if not (root and folder) then
		print("FIDMATCH|" .. H:JSONEncode({ started = false, waited = time() - t0 }))
		return
	end
	print("FIDMATCH|" .. H:JSONEncode({ started = true, inMatchAt = time(), enemies = #folder:GetChildren() }))
	game:GetService("RunService"):BindToRenderStep("FidAim", Enum.RenderPriority.Camera.Value + 1, function()
		local cam = workspace.CurrentCamera
		if not (cam and root.Parent) then return end
		local eye = cam.CFrame.Position
		local params = RaycastParams.new()
		params.FilterType = Enum.RaycastFilterType.Exclude
		params.FilterDescendantsInstances = { me.Character }
		local best, bestD
		for _, rig in folder:GetChildren() do
			local part = rig:IsA("Model") and (rig:FindFirstChild("UpperTorso") or rig:FindFirstChild("Head"))
			if part then
				local d = (part.Position - eye).Magnitude
				local hit = d < 120 and workspace:Raycast(eye, (part.Position - eye) * 1.05, params)
				if hit and hit.Instance:IsDescendantOf(rig) and (not bestD or d < bestD) then
					best, bestD = part, d
				end
			end
		end
		if best then
			cam.CFrame = CFrame.lookAt(eye, best.Position)
		end
	end)
end)
