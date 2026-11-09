-- Round 26 probe: Place D's fixed views with the player's ScreenGuis hidden
-- from 15 s (the vote board covers the Hospital and the Lodge); before
-- that, at 14 s, every wide GUI object that draws something (what covers
-- the Hospital's lights and the Lodge's aurora with the board shown).
local pg = game:GetService("Players").LocalPlayer:WaitForChild("PlayerGui")
local t0 = os.clock()
game:GetService("RunService").RenderStepped:Connect(function()
	if os.clock() - t0 < 13 then return end
	for _, g in pg:GetChildren() do
		if g:IsA("ScreenGui") then g.Enabled = false end
	end
end)
local function shown(g)
	local p = g
	while p and p ~= pg do
		if p:IsA("GuiObject") and not p.Visible then return false end
		if p:IsA("ScreenGui") and not p.Enabled then return false end
		p = p.Parent
	end
	return true
end
task.delay(12, function()
	for _, g in pg:GetDescendants() do
		if g:IsA("GuiObject") and g.AbsoluteSize.X > 500 and shown(g) and g:FindFirstAncestorOfClass("ScreenGui")
			and (g.BackgroundTransparency < 0.98 or (g:IsA("ImageLabel") and g.ImageTransparency < 0.98)) then
			local grad = g:FindFirstChildOfClass("UIGradient")
			print("PROBEGUI", g:GetFullName(), g.ClassName, string.format("BT%.2f", g.BackgroundTransparency), tostring(g.BackgroundColor3),
				tostring(g.AbsolutePosition), tostring(g.AbsoluteSize), g.ZIndex,
				grad and ("grad " .. tostring(grad.Transparency) .. " rot " .. grad.Rotation .. " " .. tostring(grad.Enabled)) or "",
				g:IsA("ImageLabel") and (g.Image .. string.format(" IT%.2f", g.ImageTransparency)) or "")
		end
	end
end)
