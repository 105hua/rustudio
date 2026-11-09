-- Fixed cameras for the light grid probe (round 26): a spot every 5 s from
-- 8 s, each held from 3 s before its shot (the grid needs a moment to fill
-- after the camera moves). Camera.Focus is set with the camera (to the
-- camera itself unless a spot names another), as Studio may centre its grid
-- on the Focus.
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local V = Vector3.new
local down = CFrame.Angles(-math.pi / 2, 0, 0) -- looking down, +X to the right
local function top(x, h, z, fov)
	return {CFrame.new(x, h, z) * down, fov}
end
local function side(from, at, fov, focus)
	return {CFrame.lookAt(from, at), fov, focus}
end
local spots = {
	top(50, 48, 0, 80),   -- 8   S1 tall wall from above
	top(1050, 48, 0, 80), -- 13  S2 short wall
	top(1900, 8, 0, 50),  -- 18  S3 pit 8 wide, 32 deep
	top(1964, 8, 0, 50),  -- 23  pit 16 / 32
	top(2028, 8, 0, 50),  -- 28  pit 16 / 16
	top(2092, 8, 0, 50),  -- 33  pit 16 / 64
	top(2130, 8, 0, 50),  -- 38  pit 4 / 32
	side(V(3000, 3, -72), V(3000, 8, 0), 70),  -- 43  S4 roof 16 over the ground
	side(V(4000, 3, -72), V(4000, 16, 0), 70), -- 48  roof 32
	side(V(5000, 3, -72), V(5000, 32, 0), 70), -- 53  roof 64
	side(V(6998, 6, -80), V(6998, 6, 0), 70),  -- 58  S6 tunnel
}
-- S5: the wall's -X face from D studs away (FOV keeping its size), then the
-- ground beside it from H studs up.
for _, d in {64, 128, 192, 256, 320, 448, 640, 900} do -- 63 … 98
	table.insert(spots, side(V(6000 - d, 32, 0), V(6000, 32, 0), 2 * math.deg(math.atan(40 / (d - 4)))))
end
for _, h in {96, 160, 224, 320, 448, 640} do -- 103 … 128
	table.insert(spots, top(6024, h, 0, 2 * math.deg(math.atan(40 / h))))
end
-- The Focus far from a near camera, and at the wall for a far one.
table.insert(spots, side(V(5936, 32, 0), V(6000, 32, 0), 64, CFrame.new(6000, 32, 2000))) -- 133
table.insert(spots, side(V(5100, 32, 0), V(6000, 32, 0), 2 * math.deg(math.atan(40 / 896)), CFrame.new(5990, 32, 0))) -- 138
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
	cam.Focus = at[3] or at[1]
end)
