-- Place D's lobby from the game's spawn view (D3) from the start (round 26):
-- is Studio's dark lobby at 10 s (place_d_cams.luau, a second after the
-- camera jumped there from the Mall) how the lobby looks, or lighting that
-- hasn't caught up with the jump? Shots at 6-18 s with no jump.
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local at = CFrame.lookAt(Vector3.new(238.5, 9.9, 3.1), Vector3.new(238.5, 6.8, -8.5))
local function apply()
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = at
end
RunService:BindToRenderStep("PlaceDLobbyCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
