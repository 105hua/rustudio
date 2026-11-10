-- Place B's particles, beams and trails from fixed cameras (round 25), on the
-- client in both engines: the waterfall (two curved beams, mist and splash
-- emitters), the fire, the coffee steam, the rain / lightning clouds, the
-- game-show stage lights (FaceCamera beams, LightEmission 1) and the
-- conveyor beams, the snowfall (Rate 5000). Each spot is held from 2 s
-- before its shot (shots 4 s
-- apart from 10 s).
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local look = CFrame.lookAt
local spots = {
	{10, look(Vector3.new(-600, 22, 772), Vector3.new(-606, 28, 840))},        -- F1 waterfall
	{14, look(Vector3.new(760, 95, 70), Vector3.new(811, 74, -4))},            -- F2 fire
	{18, look(Vector3.new(259, 26, -291), Vector3.new(265, 24, -297))},        -- F3 coffee steam
	{22, look(Vector3.new(-545, 20, 120), Vector3.new(-587, 8, 70))},          -- F4 rain / lightning clouds
	{26, look(Vector3.new(770, 25, 84), Vector3.new(833, 30, 84))},            -- F5 stage lights
	{30, look(Vector3.new(201, 250, -1690), Vector3.new(201, 228, -1745))},    -- F6 conveyor
	{34, look(Vector3.new(245, 25, -240), Vector3.new(245, 25, -400))},        -- F7 snowfall
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
RunService:BindToRenderStep("PlaceBFxCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
