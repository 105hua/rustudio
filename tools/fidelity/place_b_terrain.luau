-- Place B's terrain from fixed cameras (round 21's terrain set), on the client
-- in both engines: the Cobblestone massif and its lake, the mixed-material
-- patch by the spawn (Ice, Asphalt, Glacier, Grass, Rock, Sand), the Brick
-- area (cube mapping), under the lake's water, and the overview. Each spot
-- is held from 2 s before its shot to 2 s after (shots 4 s apart from 10 s).
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local look = CFrame.lookAt
local spots = {
	{10, look(Vector3.new(-7950, 560, 1950), Vector3.new(-8300, 260, 1450))}, -- F1 massif and lake
	{14, look(Vector3.new(576, 180, -100), Vector3.new(576, 40, -384))},      -- F2 mixed materials
	{18, look(Vector3.new(-8064, 150, 6000), Vector3.new(-8064, 64, 6208))},  -- F3 Brick
	{22, look(Vector3.new(-8400, 150, 1300), Vector3.new(-8300, 100, 1300))}, -- F4 under the lake
	{26, look(Vector3.new(0, 600, 600), Vector3.new(0, 180, 0))},             -- F5 overview
}
local t0 = os.clock() - 2 -- this script starts 2 s after Play
local function apply()
	local t = os.clock() - t0
	local at = spots[1][2]
	for _, s in spots do
		if t >= s[1] - 2 then at = s[2] end
	end
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = at
end
RunService:BindToRenderStep("PlaceBTerrainCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
