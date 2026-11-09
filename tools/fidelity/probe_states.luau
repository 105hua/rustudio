-- Round 19 probe (corpus "place-c-probe"): the client humanoid's states and
-- PlatformStanding events, and the Animator's tracks, once a second.
local plr = game:GetService("Players").LocalPlayer
local char = plr.Character or plr.CharacterAdded:Wait()
local hum = char:WaitForChild("Humanoid")
hum.PlatformStanding:Connect(function(on) print("FIDPROBE|platformstanding " .. tostring(on)) end)
hum.StateChanged:Connect(function(a, b) print("FIDPROBE|state " .. a.Name .. " -> " .. b.Name) end)
hum.Running:Connect(function(sp) print(string.format("FIDPROBE|running %.2f", sp)) end)
local animate = char:FindFirstChild("Animate")
print("FIDPROBE|animate " .. (animate and (animate.ClassName .. " children " .. #animate:GetChildren()) or "none"))
for i = 1, 8 do
	local names = {}
	local animator = hum:FindFirstChildOfClass("Animator")
	if animator then
		for _, tr in animator:GetPlayingAnimationTracks() do
			local a = tr.Animation
			table.insert(names, tr.Name .. ":" .. tostring(tr.IsPlaying) .. ":" .. (a and (a:GetFullName() .. "=" .. a.AnimationId) or "?") .. string.format(":w%.2f:len%.2f", tr.WeightCurrent, tr.Length))
		end
	end
	print(string.format("FIDPROBE|t%d state %s ps %s tracks %s", i, hum:GetState().Name, tostring(hum.PlatformStand), table.concat(names, ",")))
	task.wait(1)
end
