-- Terrain comparison place (round 21): the same voxels in Studio and rustudio.
local T = workspace.Terrain
local M = Enum.Material
local mats = {M.Grass, M.Slate, M.Concrete, M.Brick, M.Sand, M.WoodPlanks, M.Rock, M.Glacier, M.Snow,
	M.Sandstone, M.Mud, M.Basalt, M.Ground, M.CrackedLava, M.Asphalt, M.Cobblestone, M.Ice, M.LeafyGrass,
	M.Salt, M.Limestone, M.Pavement}
-- Pads: 7 x 3, 24 x 8 x 24, 32 apart; tops at y = 4.
for i, m in mats do
	local col, row = (i - 1) % 7, (i - 1) // 7
	T:FillBlock(CFrame.new(col * 32, 0, row * 32), Vector3.new(24, 8, 24), m)
end
-- Transition checker: 8-stud cells of four materials on one 32 x 8 x 32 slab.
local four = {M.Grass, M.Rock, M.Sand, M.Snow}
for x = 0, 3 do for z = 0, 3 do
	T:FillBlock(CFrame.new(300 + x * 8, 0, z * 8), Vector3.new(8, 8, 8), four[(x + z) % 4 + 1])
end end
-- Slopes 15 / 30 / 45 / 60 degrees (8 high), 20 wide, 24 apart.
for i, deg in {15, 30, 45, 60} do
	local h = 16
	local len = h / math.tan(math.rad(deg))
	T:FillWedge(CFrame.new(400 + i * 24, h / 2, 0), Vector3.new(20, h, len), M.Rock)
end
-- Voxel-by-voxel staircase: 8 steps of one voxel (4 studs) up and along.
for i = 0, 7 do
	T:FillBlock(CFrame.new(400 + i * 4 + 2, (i + 1) * 2, 60), Vector3.new(4, (i + 1) * 4, 16), M.Concrete)
end
-- Thin wall (one voxel thick), an overhang, a cave.
T:FillBlock(CFrame.new(460, 8, 60), Vector3.new(4, 16, 24), M.Brick)
T:FillBlock(CFrame.new(490, 8, 60), Vector3.new(8, 16, 8), M.Slate)
T:FillBlock(CFrame.new(498, 14, 60), Vector3.new(16, 4, 8), M.Slate)
T:FillBlock(CFrame.new(560, 12, 0), Vector3.new(40, 24, 40), M.Rock)
T:FillBall(Vector3.new(560, 12, 0), 10, M.Air)
-- Partial voxels: one voxel each at occupancy 1, .75, .5, .25, .1 on a slab, 12 apart.
T:FillBlock(CFrame.new(640, -2, 0), Vector3.new(72, 4, 16), M.Basalt)
for i, occ in {1, 0.75, 0.5, 0.25, 0.1} do
	local x = 612 + i * 12
	local region = Region3.new(Vector3.new(x, 0, -4), Vector3.new(x + 4, 4, 0))
	T:WriteVoxels(region, 4, {{{M.Snow}}}, {{{occ}}})
end
-- Water: a sand basin with a sloped shore, 20 studs deep.
T:FillBlock(CFrame.new(800, -12, 0), Vector3.new(96, 32, 96), M.Sand)
T:FillBlock(CFrame.new(800, -4, 0), Vector3.new(64, 24, 64), M.Air)
T:FillWedge(CFrame.new(800, -8, 36) * CFrame.Angles(0, math.pi, 0), Vector3.new(64, 16, 24), M.Sand)
T:FillBlock(CFrame.new(800, -6, 0), Vector3.new(64, 20, 64), M.Water)
-- Grass hill with decoration.
T:FillBall(Vector3.new(1000, -12, 0), 28, M.Grass)
-- (Terrain.Decoration is NotScriptable: set in the place file)
-- MaterialColors pass (for the later shots).
task.delay(46, function()
	T:SetMaterialColor(M.Grass, Color3.fromRGB(200, 60, 60))
	T:SetMaterialColor(M.Rock, Color3.fromRGB(60, 60, 200))
	T:SetMaterialColor(M.Sand, Color3.fromRGB(240, 240, 240))
	T:SetMaterialColor(M.Concrete, Color3.fromRGB(30, 140, 30))
end)
