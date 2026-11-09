-- Round 26 probe: where Camera.Focus is while the harness's fixed cameras
-- (Scriptable, CFrame only) look at a spot: Studio's Soft lights go out
-- with the Focus 1500 studs away, not 200 or 400 (corpus lightreach-soft /
-- lightfocus-soft), and a Focus left alone sits 20 studs ahead of the camera.
local cam = workspace.CurrentCamera
local t0 = os.clock()
game:GetService("RunService").Heartbeat:Connect(function()
	local t = os.clock() - t0
	if t >= 1 then
		t0 = os.clock()
		print("PROBEFOCUS", time(), cam.CameraType, cam.Focus.Position, cam.CFrame.Position)
	end
end)
