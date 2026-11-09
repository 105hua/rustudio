-- Place B Newer's sky domes (round 26): HallowsVille.HallowsvilleSky is a
-- 2 048-stud sphere MeshPart (DoubleSided) around the hill houses with a
-- SurfaceAppearance (cloud ColorMap, Color tint (85, 39, 19), Overlay).
-- Studio draws it plain orange, rustudio drew the clouds. The same mesh and
-- SurfaceAppearance do show in Studio at 100-1200 studs and from inside a
-- 300-stud copy (sa-mesh, sa-far), so this checks the size: from the N5
-- spot, each step held 16 s (Studio runs this place's scripts 8-14 s
-- late): as saved; shrunk to 300 studs, 500 studs ahead of the camera;
-- 1 200 studs there (the camera inside); 2 048 there.
local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local at = CFrame.lookAt(Vector3.new(25500, 260, 330), Vector3.new(25600, 170, 100))
local dome = workspace:WaitForChild("HallowsVille"):WaitForChild("HallowsvilleSky")
local ahead = at * CFrame.new(0, 0, -500)
local t0 = os.clock()
local step = 0
local function apply()
	local k = math.floor((os.clock() - t0) / 16) + 1
	if k ~= step then
		step = k
		if k == 2 then
			dome.Size = Vector3.new(300, 300, 300)
			dome.CFrame = ahead
		elseif k == 3 then
			dome.Size = Vector3.new(1200, 1200, 1200)
		elseif k == 4 then
			dome.Size = Vector3.new(2048, 2048, 2048)
		end
		print("FIDDOME", k, time())
	end
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = at
end
RunService:BindToRenderStep("DomeCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
