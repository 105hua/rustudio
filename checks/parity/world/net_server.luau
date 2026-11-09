-- Network ownership probe (server): loose parts at distances from the
-- character; owners printed when they change (Studio session in world.md).
task.spawn(function()
	local F = Instance.new("Folder", workspace) F.Name = "Own"
	local dists = { 30, 50, 60, 70, 80, 100, 120 }
	local parts = {}
	for i, d in dists do local p = Instance.new("Part") p.Name = "D" .. d p.Size = Vector3.new(1, 1, 1) p.Position = Vector3.new(d, 0.5, 0) p.Parent = F parts[i] = p end
	local spin = Instance.new("Part") spin.Name = "Spin" spin.Size = Vector3.new(2, 2, 2) spin.Position = Vector3.new(0, 300, -40) spin.Parent = F
	local plr = game.Players:GetPlayers()[1] or game.Players.PlayerAdded:Wait()
	local ch = plr.Character or plr.CharacterAdded:Wait()
	local hrp = ch:WaitForChild("HumanoidRootPart")
	hrp.CFrame = CFrame.new(0, 3, 0)
	spin:ApplyAngularImpulse(Vector3.new(0, 50, 0))
	local t0, last = os.clock(), ""
	while os.clock() - t0 < 40 do
		task.wait(0.1)
		local s = {}
		for _, p in parts do s[#s + 1] = p.Name .. (p:GetNetworkOwner() and "P" or "s") end
		local line = table.concat(s, " ")
		if line ~= last then print(string.format("NET S t=%.1f %s char=%s", os.clock() - t0, line, tostring(ch.HumanoidRootPart:GetNetworkOwner()))) last = line end
	end
end)
