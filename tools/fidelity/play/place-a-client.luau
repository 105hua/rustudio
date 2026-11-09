-- Place A match probe (client): records where the character is and the HUD's clip
-- count when it appears, and again WATCH seconds later (the harness walks and fires in
-- between, the probe aiming at the nearest enemy). Prints PLAYER|{json}.
local WATCH = 25 -- seconds between the two readings
task.spawn(function()
	local H = game:GetService("HttpService")
	local me = game.Players.LocalPlayer
	local t0 = os.clock()
	local root
	while os.clock() - t0 < 120 do
		local c = me.Character
		root = c and c:FindFirstChild("HumanoidRootPart")
		if root and me:GetAttribute("Stance") then break end
		task.wait(0.25)
	end
	if not root then
		print("PLAYER|" .. H:JSONEncode({ spawned = false }))
		return
	end
	-- The HUD's clip count: the labels showing the start pistol's 12.
	local labels = {}
	for _, d in me.PlayerGui:GetDescendants() do
		if d:IsA("TextLabel") and d.Text == "12" then table.insert(labels, d) end
	end
	local function clip()
		local n
		for _, l in labels do
			local v = tonumber(l.Text)
			if v then n = math.min(n or v, v) end
		end
		return n
	end
	local p0, c0 = root.Position, clip()
	local spawnedAt = os.clock() - t0
	-- The player aims: right after Place A's own camera step (same priority, bound
	-- later) and before its weapons step fires, the view turns to the
	-- nearest enemy in sight (what the mouse would do), so the harness's
	-- clicks fire at enemies rather than at a wall.
	local aimed, seen = 0, 0
	local RS = game:GetService("RunService")
	RS:BindToRenderStep("PlaceAProbeAim", Enum.RenderPriority.Camera.Value + 1, function()
		local folder = workspace:FindFirstChild("ZombiesClient")
		local cam = workspace.CurrentCamera
		if not (folder and cam and root.Parent) then return end
		local eye = cam.CFrame.Position
		local params = RaycastParams.new()
		params.FilterType = Enum.RaycastFilterType.Exclude
		params.FilterDescendantsInstances = { me.Character }
		params.CollisionGroup = "WeaponRay"
		local best, bestD
		for _, rig in folder:GetChildren() do
			if rig:IsA("Model") then
				-- a part a shot can hit (the root part isn't queryable), in sight
				local part = rig:FindFirstChild("UpperTorso") or rig:FindFirstChild("Torso") or rig:FindFirstChild("Head")
				if part and part.CanQuery then
					local d = (part.Position - eye).Magnitude
					local hit = d < 90 and workspace:Raycast(eye, (part.Position - eye) * 1.05, params)
					if hit and hit.Instance:IsDescendantOf(rig) and (not bestD or d < bestD) then
						best, bestD = part, d
					end
				end
			end
		end
		seen = math.max(seen, #folder:GetChildren())
		if best then
			aimed += 1
			cam.CFrame = CFrame.lookAt(eye, best.Position)
		end
	end)
	task.wait(WATCH)
	local p1 = root.Parent and root.Position or p0
	print("PLAYER|" .. H:JSONEncode({
		spawned = true, spawnedAfter = spawnedAt, moved = (Vector3.new(p1.X - p0.X, 0, p1.Z - p0.Z)).Magnitude,
		clipBefore = c0, clipAfter = clip(), aimedFrames = aimed, zombiesSeen = seen,
	}))
end)
