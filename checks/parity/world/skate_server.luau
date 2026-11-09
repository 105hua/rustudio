-- SkateboardPlatform probe (server): a free board under the character
-- (Studio: the mount error, PlatformStand, no events), then one inside the
-- character model (Equipped with a SkateboardController).
task.spawn(function()
	local plr = game.Players:GetPlayers()[1] or game.Players.PlayerAdded:Wait()
	local ch = plr.Character or plr.CharacterAdded:Wait()
	local hrp = ch:WaitForChild("HumanoidRootPart") local hum = ch:WaitForChild("Humanoid")
	task.wait(1)
	local free = Instance.new("SkateboardPlatform") free.Name = "Free" free.Anchored = true
	free.Position = hrp.Position - Vector3.new(0, hrp.Size.Y / 2 + hum.HipHeight + 0.6, 0) free.Parent = workspace
	free.Equipped:Connect(function(h, c) print("SKATE S Free Equipped", h, c) end)
	task.wait(1)
	print("SKATE S free", free.ControllingHumanoid, free.Controller, hum.PlatformStand)
	free:Destroy() hum.PlatformStand = false task.wait(0.5)
	local own = Instance.new("SkateboardPlatform") own.Name = "Own" own.Anchored = true
	own.Equipped:Connect(function(h, c) print("SKATE S Own Equipped", h, c and c.ClassName, c and c.Steer, c and c.Throttle) end)
	own.Position = hrp.Position - Vector3.new(0, hrp.Size.Y / 2 + hum.HipHeight + 0.6, 0) own.Parent = ch
	task.wait(1)
	print("SKATE S own", own.ControllingHumanoid, own.Controller, hum.PlatformStand)
end)
