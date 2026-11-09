-- Fixed cameras for the second light grid probe (round 26): a spot every
-- 5 s from 8 s, held from 3 s before its shot; Focus with the camera.
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local down = CFrame.Angles(-math.pi / 2, 0, 0) -- looking down, +X to the right
local spots = {}
local function top(x, h, z, fov)
	table.insert(spots, {CFrame.new(x, h, z) * down, fov})
end
-- Pit floors, from 8 over each pit's mouth (8 … 63).
for _, x in {-512, -448, -384, -320, -256, -192, -128, -64, 0, 64, 128, 192} do
	top(x, 8, 0, 50)
end
-- The ground under each roof, from half way up (68 … 78).
for i, h in {16, 32, 64} do
	top(1000 * i, h / 2, 0, 90)
end
-- The tunnel floor 4 / 12 / 24 / 40 / 64 studs in (83 … 103).
for _, z in {-60, -52, -40, -24, 0} do
	top(3998, 9, z, 60)
end
-- The ground beside walls 4 / 8 / 24 / 48 high (108 … 123).
for i = 1, 4 do
	top(4000 + 1000 * i + 46, 48, 0, 80)
end
local t0 = os.clock()
RunService:BindToRenderStep("LightGridCams", Enum.RenderPriority.Camera.Value + 1, function()
	local t = os.clock() - t0
	local at = spots[1]
	for i, s in spots do
		if t >= 8 + (i - 1) * 5 - 3 then at = s end
	end
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = at[2]
	cam.CFrame = at[1]
	cam.Focus = at[1]
end)
