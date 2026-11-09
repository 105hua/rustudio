-- Light grid probe place (round 26): terrain stations, 1000 studs apart, lit
-- by OutdoorAmbient alone (Ambient 0, Brightness 0, no environment light),
-- so what each surface shows is its skylight. Everything is voxel aligned.
local T = workspace.Terrain
local M = Enum.Material
local function pad(cx, depth, width)
	-- Sand ground, top at y = 0.
	width = width or 320
	T:FillBlock(CFrame.new(cx, -depth / 2, 0), Vector3.new(width, depth, width), M.Sand)
end
-- S1 tall wall (x -4..4, 96 high, 128 long) and S2 short wall (12 high).
pad(0, 8)
T:FillBlock(CFrame.new(0, 48, 0), Vector3.new(8, 96, 128), M.Ground)
pad(1000, 8)
T:FillBlock(CFrame.new(1000, 6, 0), Vector3.new(8, 12, 128), M.Ground)
-- S3 square pits in an 80-deep pad: {centre x, width, depth}.
pad(2000, 80, 480)
for _, p in {{1900, 8, 32}, {1964, 16, 32}, {2028, 16, 16}, {2092, 16, 64}, {2130, 4, 32}} do
	T:FillBlock(CFrame.new(p[1], -p[3] / 2, 0), Vector3.new(p[2], p[3], p[2]), M.Air)
end
-- S4 floating roofs, 64 x 8 x 64, their undersides 16 / 32 / 64 over the ground.
for i, h in {16, 32, 64} do
	local cx = 2000 + i * 1000
	pad(cx, 8)
	T:FillBlock(CFrame.new(cx, h + 4, 0), Vector3.new(64, 8, 64), M.Ground)
end
-- S5 grid extent: a wall (x 5996..6004, 64 high, 64 long) seen from ever
-- farther away.
pad(6000, 8)
T:FillBlock(CFrame.new(6000, 32, 0), Vector3.new(8, 64, 64), M.Ground)
-- S6 a tunnel, 12 x 12, 96 deep, into a 128-stud block (open at z = -64).
pad(7000, 8)
T:FillBlock(CFrame.new(7000, 32, 0), Vector3.new(128, 64, 128), M.Ground)
T:FillBlock(CFrame.new(6998, 6, -16), Vector3.new(12, 12, 96), M.Air)
