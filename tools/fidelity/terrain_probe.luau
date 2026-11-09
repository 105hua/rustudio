-- Terrain shape probe (round 21), the same code in Studio and rustudio.
-- Writes exact cells far from everything with WriteVoxels, raycasts down on a
-- 0.5-stud grid over each and prints the hit heights and materials; then
-- ReadVoxels after odd FillBlock / FillBall calls. Output: FIDTER|i|n|json.
local HttpService = game:GetService("HttpService")
local T = workspace.Terrain
local M = Enum.Material
local X0 = 5000

local function write(origin, cells)
	-- cells: {{dx, dy, dz, material, occupancy}, ...} in voxels from origin (voxel coords)
	for _, c in cells do
		local v = origin + Vector3.new(c[1], c[2], c[3]) * 4
		T:WriteVoxels(Region3.new(v, v + Vector3.new(4, 4, 4)), 4, {{{c[4]}}}, {{{c[5]}}})
	end
end

local probes = {}
local function cell(name, ox, cells, span)
	local origin = Vector3.new(X0 + ox, 0, 0)
	write(origin, cells)
	table.insert(probes, {name = name, origin = origin, span = span or 6})
end
-- Soft materials (no shift / quantize / cubify in Studio's table): Grass, Sand.
cell("lone1", 0, {{0, 0, 0, M.Grass, 1}})
cell("lone075", 64, {{0, 0, 0, M.Grass, 0.75}})
cell("lone05", 128, {{0, 0, 0, M.Grass, 0.5}})
cell("lone025", 192, {{0, 0, 0, M.Grass, 0.25}})
cell("lone01", 256, {{0, 0, 0, M.Grass, 0.1}})
do
	local c = {}
	for x = 0, 8 do for z = 0, 2 do
		table.insert(c, {x, 0, z, M.Grass, 1})
		table.insert(c, {x, 1, z, M.Grass, x / 8})
	end end
	cell("slab_eighths", 320, c, 20)
end
do
	local c = {}
	for i = 0, 7 do for y = 0, i do for z = 0, 2 do table.insert(c, {i, y, z, M.Sand, 1}) end end end
	cell("stairs", 448, c, 20)
end
do
	-- A 5 x 2 x 5 Grass slab: its edge and corner profile.
	local c = {}
	for x = 0, 4 do for y = 0, 1 do for z = 0, 4 do table.insert(c, {x, y, z, M.Grass, 1}) end end end
	cell("slab_soft", 720, c, 12)
end
do
	local c = {}
	for y = 0, 3 do for z = 0, 4 do table.insert(c, {0, y, z, M.Brick, 1}) end end
	cell("wall", 576, c, 12)
end
do
	local c = {}
	for x = 0, 3 do for z = 0, 3 do table.insert(c, {x, 0, z, x < 2 and M.Grass or M.Sand, 1}) end end
	cell("two_materials", 640, c, 12)
end

-- Both engines build terrain collision after an edit: let it settle.
task.wait(3)
local out = {rays = {}, voxels = {}}
for _, p in probes do
	local hs, ms = {}, {}
	local c = p.origin + Vector3.new(2, 0, 2)
	for iz = -p.span * 2, p.span * 2 + (p.span > 6 and 8 or 0) do
		for ix = -p.span * 2, p.span * 2 + (p.span > 6 and 16 or 0) do
			local o = c + Vector3.new(ix * 0.5, 40, iz * 0.5)
			local r = workspace:Raycast(o, Vector3.new(0, -80, 0))
			table.insert(hs, r and math.floor(r.Position.Y * 100 + 0.5) or false)
			table.insert(ms, r and r.Material.Name or false)
		end
	end
	-- (materials only where two meet: the rest is one material)
	out.rays[p.name] = {h = hs, m = p.name == "two_materials" and ms or nil, span = p.span}
end

-- Fill semantics: occupancies after odd FillBlock / FillBall.
local function read(name, lo, hi)
	local mats, occs = T:ReadVoxels(Region3.new(lo, hi), 4)
	local list = {}
	local s = mats.Size
	for x = 1, s.X do for y = 1, s.Y do for z = 1, s.Z do
		if occs[x][y][z] > 0 then
			table.insert(list, {x - 1, y - 1, z - 1, math.floor(occs[x][y][z] * 1000 + 0.5), mats[x][y][z].Name})
		end
	end end end
	out.voxels[name] = list
end
T:FillBlock(CFrame.new(X0 + 1000.5, 6.5, 0.25), Vector3.new(5, 5, 5), M.Rock)
read("block5_off", Vector3.new(X0 + 988, -8, -12), Vector3.new(X0 + 1012, 20, 12))
T:FillBlock(CFrame.new(X0 + 1100, 6, 0), Vector3.new(6, 6, 6), M.Rock)
read("block6", Vector3.new(X0 + 1088, -8, -12), Vector3.new(X0 + 1112, 20, 12))
T:FillBlock(CFrame.new(X0 + 1200, 6, 0), Vector3.new(2, 2, 2), M.Rock)
read("block2", Vector3.new(X0 + 1188, -4, -8), Vector3.new(X0 + 1212, 16, 8))
T:FillBall(Vector3.new(X0 + 1300.7, 7.3, 0), 5.5, M.Rock)
read("ball55", Vector3.new(X0 + 1284, -8, -16), Vector3.new(X0 + 1316, 24, 16))
T:FillBall(Vector3.new(X0 + 1400, 8, 0), 3, M.Rock)
read("ball3", Vector3.new(X0 + 1388, -4, -12), Vector3.new(X0 + 1412, 20, 12))

local text = HttpService:JSONEncode(out)
local size = 700
local n = math.ceil(#text / size)
for i = 1, n do
	print(string.format("FIDTER|%d|%d|%s", i, n, string.sub(text, (i - 1) * size + 1, i * size)))
end
