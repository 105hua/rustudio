-- Place D from fixed cameras, on the client in both engines (round 25): two
-- of the maps that stand in Workspace from the start before the lobby UI
-- opens, the lobby with its vote board (the game's own spawn view, then
-- from the far end), and two more maps under the vote board. The vote ends
-- ~23 s in and the next map is random (with its own cutscene camera), so
-- every shot falls before it. Each spot is held from 3 s before its shot
-- (round 26: held 1 s, Studio's 10 s lobby shot came out dark: its local
-- lights hadn't caught up with the camera's jump from the Mall; held from
-- the start, place_d-lobby, the lobby is lit at every shot from 6 s).
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local look = CFrame.lookAt
local spots = {
	{6, look(Vector3.new(600, 50, -430), Vector3.new(600, 10, -600))},         -- D1 Carnival (Funland)
	{9, look(Vector3.new(0, 30, 700), Vector3.new(0, 10, 860))},               -- D2 Mall (the Place D House)
	{12, look(Vector3.new(238.5, 9.9, 3.1), Vector3.new(238.5, 6.8, -8.5))},   -- D3 lobby UI, the spawn view
	{15, look(Vector3.new(238.5, 16, 46), Vector3.new(238.5, 4, -12))},        -- D4 lobby from the far end
	{18, look(Vector3.new(-600, 45, 770), Vector3.new(-600, 15, 600))},        -- D5 Hospital
	{21, look(Vector3.new(1290, 170, 380), Vector3.new(1290, 30, -15))},       -- D6 Lodge
}
-- Timed by time() (game time since Play), as capture.luau reads the camera:
-- the offline Studio sometimes starts this script late (round 26 recapture:
-- every spot came out one spot behind with an os.clock origin).
local function apply()
	local t = time()
	local at = spots[1][2]
	for _, s in spots do
		if t >= s[1] - 3 then at = s[2] end
	end
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = at
end
RunService:BindToRenderStep("PlaceDCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
