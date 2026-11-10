-- Place B's water and shadows from fixed cameras, on the client in both
-- engines: under the water at the Water Defender, its surface from above,
-- the far lake (distant water), a tree's shadow on grass, and the spawn.
-- Each spot is held from 2 s before its shot (shots 4 s apart from 10 s).
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local look = CFrame.lookAt
local spots = {
	{10, look(Vector3.new(644, 48, -400), Vector3.new(644, 38, -440))},       -- W1 under water, Water Defender
	{14, look(Vector3.new(600, 110, -330), Vector3.new(650, 40, -440))},      -- W2 the surface from above
	{18, look(Vector3.new(-7500, 330, 2500), Vector3.new(-8300, 100, 1300))}, -- W3 distant lake
	{22, look(Vector3.new(-15, 50, -465), Vector3.new(-16, 22, -501))},       -- S1 tree shadow
	{26, look(Vector3.new(100, 40, 740), Vector3.new(100, 8, 782))},          -- S2 palm tree shadow
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
RunService:BindToRenderStep("PlaceBWaterCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
