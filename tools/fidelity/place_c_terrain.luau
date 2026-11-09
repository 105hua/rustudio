-- Place C's planet ground in every biome (round 21's terrain set), on the
-- client in both engines: the ground part of Server/Surface.luau generate()
-- (one FillBlock, then 16-stud patches picked by math.noise) with the game's
-- own Biomes data and Main.server.luau's terrain colours, far from the
-- game's own surfaces; then the game's top-down surface camera (FOV 50,
-- height 66, Client/Camera.luau) over each biome, and a patch border close up.
-- (the clock starts here: building the ground takes each engine its own time)
local t0 = os.clock() - 4 -- this script starts 4 s after Play
local RunService = game:GetService("RunService")
local RS = game:GetService("ReplicatedStorage")
local Biomes = require(RS.Shared.Biomes)
local Config = require(RS.Shared.Config)
local T = workspace.Terrain
local half = Config.SURFACE_HALF
local origins = {}
-- Random's sequence and math.noise, for the engines to compare (FIDRNG).
do
	local out = {}
	for _, seed in { 0, 1, 1001, 1005, 123456789 } do
		local r = Random.new(seed)
		table.insert(out, string.format("%d:%.17g,%.17g,%d,%d", seed, r:NextNumber(), r:NextNumber(0, 1000), r:NextInteger(1, 1000000), r:NextInteger(-5, 5)))
	end
	table.insert(out, string.format("noise:%.17g,%.17g", math.noise(0.13 + 1.5, 0.26, 3), math.noise(12.34, 5.6, 7.8)))
	print("FIDRNG " .. table.concat(out, ";"))
end
for slot, b in ipairs(Biomes.list) do
	for mat, color in b.terrainColors do
		T:SetMaterialColor(mat, color)
	end
	local o = Vector3.new(20000 + slot * 1000, 0, 0)
	local seed = 1000 + slot
	local rng = Random.new(seed)
	local seedX = rng:NextNumber(0, 1000)
	local extent = half * 2 + 120
	T:FillBlock(CFrame.new(o + Vector3.new(0, -4, 0)), Vector3.new(extent, 8, extent), b.terrain[1])
	local CELL = 16
	local n = math.ceil(extent / CELL / 2)
	for ix = -n, n do
		for iz = -n, n do
			local v = math.noise(ix * 0.13 + seedX, iz * 0.13, seed % 97)
			local idx = math.clamp(math.floor((v + 0.5) * #b.terrain) + 1, 1, #b.terrain)
			local mat = b.terrain[idx]
			if mat ~= b.terrain[1] then
				T:FillBlock(CFrame.new(o + Vector3.new(ix * CELL, -4, iz * CELL)), Vector3.new(CELL, 8, CELL), mat)
			end
		end
	end
	table.insert(origins, o)
end
local cam = workspace.CurrentCamera
local function view(focus, h)
	return CFrame.lookAt(focus + Vector3.new(0, h, h * 0.3), focus)
end
-- One spot per biome 4 s apart from 10 s (held 2 s either side), then a
-- patch border close up.
local spots = {}
for i, o in origins do
	table.insert(spots, {6 + i * 4, view(o + Vector3.new(24, 0, 24), 66)})
end
table.insert(spots, {6 + (#origins + 1) * 4, view(origins[4] + Vector3.new(8, 0, 8), 40)})
local function apply()
	local t = os.clock() - t0
	local at = spots[1][2]
	for _, s in spots do
		if t >= s[1] - 2 then at = s[2] end
	end
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 50
	cam.CFrame = at
end
RunService:BindToRenderStep("PlaceCTerrainCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
