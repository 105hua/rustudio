-- Fixed cameras for the terrain set: each spot is held from 2 s before its
-- shot to 2 s after (shots 4 s apart: Studio's Play start, which the
-- harness counts from, is only known to a second or two).
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local look = CFrame.lookAt
local spots = {
	{8, look(Vector3.new(96, 70, -70), Vector3.new(96, 0, 32))},      -- T1 pads overview
	{12, look(Vector3.new(-16, 8, 40), Vector3.new(80, 2, 40))},      -- T2 pads, grazing
	{16, look(Vector3.new(312, 50, 13), Vector3.new(312, 0, 12))},    -- T3 checker from above
	{20, look(Vector3.new(460, 20, -50), Vector3.new(460, 8, 30))},   -- T4 slopes / stairs / wall
	{24, look(Vector3.new(552, 10, 4), Vector3.new(570, 10, -4))},    -- T5 cave interior
	{28, look(Vector3.new(800, 6, 56), Vector3.new(800, -2, 10))},    -- T6 shore at water level
	{32, look(Vector3.new(800, -8, 0), Vector3.new(820, -12, -20))},  -- Place A underwater
	{36, look(Vector3.new(1000, 26, -40), Vector3.new(1000, 10, 0))}, -- T8 grass hill close
	{40, look(Vector3.new(1000, 60, -150), Vector3.new(1000, 0, 0))}, -- T8b grass hill far
	{44, look(Vector3.new(500, 400, -1500), Vector3.new(500, 0, 0))}, -- T9 far view
	{48, look(Vector3.new(96, 70, -70), Vector3.new(96, 0, 32))},     -- T1 with MaterialColors
	{52, look(Vector3.new(640, 10, -24), Vector3.new(640, 1, -2))},   -- T10 partial voxels
}
-- Timed from this script's start (the client joining, close to Play start
-- in both engines); time() starts at different moments in each.
local t0 = os.clock()
RunService:BindToRenderStep("TerrainCams", Enum.RenderPriority.Camera.Value + 1, function()
	local t = os.clock() - t0
	local at = spots[1][2]
	for _, s in spots do
		if t >= s[1] - 2 then at = s[2] end
	end
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = at
end)
