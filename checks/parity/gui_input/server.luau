task.spawn(function()
-- Server side of the prompt check: a prompt by the player; the client's hold
-- must reach the server's Triggered / PromptTriggered.
local plr = game.Players:GetPlayers()[1] or game.Players.PlayerAdded:Wait()
local char = plr.Character or plr.CharacterAdded:Wait()
local hrp = char:WaitForChild("HumanoidRootPart")
local part = Instance.new("Part") part.Name = "PromptPart" part.Anchored = true part.CFrame = hrp.CFrame * CFrame.new(0,0,-3)
local pp = Instance.new("ProximityPrompt") pp.Parent = part
pp.Triggered:Connect(function(p) print("GI server Triggered " .. p.Name) end)
game:GetService("ProximityPromptService").PromptTriggered:Connect(function(x, p) print("GI server PPS.PromptTriggered " .. x.ClassName .. " " .. p.Name) end)
part.Parent = workspace
end)
