-- Light grid probe, second set (round 26): what decides the skylight in a
-- shaft, under a roof and down a tunnel. Lit as lightgrid_build.luau (by
-- OutdoorAmbient alone); stations 1000 studs apart, voxel aligned.
local T = workspace.Terrain
local M = Enum.Material
local function pad(cx, depth, width)
	T:FillBlock(CFrame.new(cx, -depth / 2, 0), Vector3.new(width or 320, depth, width or 320), M.Sand)
end
-- Square pits {centre x, width, depth} in a 96-deep pad, 64 apart.
pad(0, 96, 1200)
for _, p in {
	{-512, 16, 8}, {-448, 16, 24}, {-384, 16, 40}, {-320, 16, 48}, {-256, 16, 56},
	{-192, 8, 8}, {-128, 8, 16}, {-64, 24, 32}, {0, 32, 32}, {64, 16, 16}, {128, 16, 32}, {192, 16, 64},
} do
	T:FillBlock(CFrame.new(p[1], -p[3] / 2, 0), Vector3.new(p[2], p[3], p[2]), M.Air)
end
-- Floating roofs 64 x 8 x 64, undersides 16 / 32 / 64 over the ground.
for i, h in {16, 32, 64} do
	local cx = 1000 * i
	pad(cx, 8)
	T:FillBlock(CFrame.new(cx, h + 4, 0), Vector3.new(64, 8, 64), M.Ground)
end
-- A tunnel, 12 x 12 (x 3992..4004, y 0..12), 96 deep into a 128-stud block
-- whose front face is at z = -64.
pad(4000, 8)
T:FillBlock(CFrame.new(4000, 32, 0), Vector3.new(128, 64, 128), M.Ground)
T:FillBlock(CFrame.new(3998, 6, -16), Vector3.new(12, 12, 96), M.Air)
-- Walls 8 thick, 128 long, of heights 4 / 8 / 24 / 48, 1000 apart.
for i, h in {4, 8, 24, 48} do
	local cx = 4000 + 1000 * i
	pad(cx, 8)
	T:FillBlock(CFrame.new(cx, h / 2, 0), Vector3.new(8, h, 128), M.Ground)
end
