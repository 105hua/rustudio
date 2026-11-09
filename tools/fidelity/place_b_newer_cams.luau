-- Place B Newer (the person's newer Place B place file) from fixed cameras,
-- on the client in both engines (round 25): the spawn, the Aquarius gate,
-- the Dome, the Witches Conservatory and the hill houses.
--
-- Timed from this script's start, each spot held for 16 s: on this big
-- place Studio runs the harness's scripts 8-14 s late (and its time() starts
-- late too), so each shot falls 15 s into its window (shots at 17, 33, 49,
-- 65, 81 s; the script is sent 2 s after Play): any lag up to 15 s sees the
-- right spot.
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local look = CFrame.lookAt
local spots = {
	{0, look(Vector3.new(-207, 30, 175), Vector3.new(-207, 2, 119))},            -- N1 spawn
	{16, look(Vector3.new(-160, 25, 70), Vector3.new(-110, 5, 16))},             -- N2 Aquarius gate
	{32, look(Vector3.new(2645, 200, 1400), Vector3.new(2645, 150, 1276))},      -- N3 Dome
	{48, look(Vector3.new(24440, 90, 4700), Vector3.new(24440, 0, 4459))},       -- N4 Witches Conservatory
	{64, look(Vector3.new(25500, 260, 330), Vector3.new(25600, 170, 100))},      -- N5 hill houses
}
print("FIDCAMS start", time())
local t0 = os.clock()
local function apply()
	local t = os.clock() - t0
	local at = spots[1][2]
	for _, s in spots do
		if t >= s[1] then at = s[2] end
	end
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = at
end
RunService:BindToRenderStep("NewerCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
