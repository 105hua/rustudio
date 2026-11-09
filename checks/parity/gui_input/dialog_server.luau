-- Dialog check (gui_input.md): an NPC part with a Dialog, server listeners.
local npc = Instance.new("Part") npc.Name = "NPC" npc.Anchored = true npc.Size = Vector3.new(2,4,2) npc.CFrame = CFrame.new(0, 3, -8)
local d = Instance.new("Dialog") d.InitialPrompt = "Hello traveler" d.Parent = npc
local c1 = Instance.new("DialogChoice") c1.Name = "C1" c1.UserDialog = "Who are you?" c1.ResponseDialog = "A part." c1.Parent = d
local c2 = Instance.new("DialogChoice") c2.Name = "C2" c2.UserDialog = "Bye" c2.ResponseDialog = "Ok." c2.Parent = d
d.DialogChoiceSelected:Connect(function(p, c) print("DLG server choice", p.Name, c.Name, d.InUse, #d:GetCurrentPlayers()) end)
d:GetPropertyChangedSignal("InUse"):Connect(function() print("DLG server InUse", d.InUse, #d:GetCurrentPlayers()) end)
npc.Parent = workspace
