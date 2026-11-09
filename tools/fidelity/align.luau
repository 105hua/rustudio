-- Fidelity prelude, run on the client in both engines right after Play:
-- Studio spawns the character at a random point on the SpawnLocation's top,
-- rustudio at its centre. Once the character exists it is put on the centre,
-- facing the way it spawned, so both engines' frames show the same scene.
-- (A place without a character or a SpawnLocation is left alone.)
task.spawn(function()
	local me = game:GetService("Players").LocalPlayer
	local t0 = os.clock()
	local root
	while os.clock() - t0 < 15 do
		local c = me and me.Character
		root = c and c:FindFirstChild("HumanoidRootPart")
		if root then break end
		task.wait(0.05)
	end
	local spawn = workspace:FindFirstChildWhichIsA("SpawnLocation", true)
	if not (root and spawn) then
		print("FIDALIGN|skipped")
		return
	end
	local char = root.Parent
	local pivot = char:GetPivot()
	local top = spawn.Position + Vector3.new(0, spawn.Size.Y / 2, 0)
	local lift = root.Position.Y - (spawn.Position.Y + spawn.Size.Y / 2)
	local at = Vector3.new(top.X, top.Y + math.max(lift, 0), top.Z)
	char:PivotTo(CFrame.new(at) * pivot.Rotation)
	print(string.format("FIDALIGN|moved %.2f studs", (Vector3.new(pivot.X, 0, pivot.Z) - Vector3.new(at.X, 0, at.Z)).Magnitude))
end)
