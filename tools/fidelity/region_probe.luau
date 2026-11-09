-- Terrain region semantics (round 26): is a Region3int16's Max inclusive in
-- CopyRegion, where PasteRegion puts the cells, and what a TerrainRegion's
-- SmoothGrid holds (serialized as rbxm, printed as hex): FIDREG lines.
local T = workspace.Terrain
local M = Enum.Material
local V, C = Vector3.new, Vector3int16.new
T:Clear()
-- Rock in cells x 10..13, y 1..2, z 20..23; Snow at (10, 1, 20), Grass at (13, 2, 23).
T:FillBlock(CFrame.new(48, 8, 88), V(16, 8, 16), M.Rock)
T:WriteVoxels(Region3.new(V(40, 4, 80), V(44, 8, 84)), 4, {{{M.Snow}}}, {{{1}}})
T:WriteVoxels(Region3.new(V(52, 8, 92), V(56, 12, 96)), 4, {{{M.Grass}}}, {{{1}}})
print("FIDREG|MaxExtents|" .. tostring(T.MaxExtents.Min) .. "|" .. tostring(T.MaxExtents.Max))
local function show(tag, r)
	-- (ExtentsMin / ExtentsMax are saved, not scriptable: Studio says
	-- "not a valid member")
	local ok, e = pcall(function() return r.ExtentsMin end)
	print("FIDREG|" .. tag .. "|" .. tostring(r.SizeInCells) .. "|" .. tostring(ok) .. "|" .. tostring(e))
end
local r1 = T:CopyRegion(Region3int16.new(C(10, 1, 20), C(13, 2, 23)))
show("r1", r1)
show("r2", T:CopyRegion(Region3int16.new(C(10, 1, 20), C(10, 1, 20))))
show("r3", T:CopyRegion(Region3int16.new(C(11, 1, 21), C(12, 2, 22))))
-- What lands where: paste r1 at cell (100, 1, 100), read cells 98..106.
T:PasteRegion(r1, C(100, 1, 100), true)
local function dump(tag, lo, hi)
	local mats, occ = T:ReadVoxels(Region3.new(lo * 4, hi * 4), 4)
	local s = mats.Size
	for x = 1, s.X do for y = 1, s.Y do for z = 1, s.Z do
		if mats[x][y][z] ~= M.Air then
			print("FIDREG|" .. tag .. "|" .. (lo.X + x - 1) .. "," .. (lo.Y + y - 1) .. "," .. (lo.Z + z - 1) .. "|" .. mats[x][y][z].Name .. "|" .. math.round(occ[x][y][z] * 255))
		end
	end end end
end
dump("paste", V(98, 0, 98), V(107, 5, 107))
-- PasteRegion with empty cells: does it clear only the region's own box?
T:FillBlock(CFrame.new(808, 8, 808), V(32, 8, 32), M.Sand) -- cells 198..205, 1..2
T:PasteRegion(r1, C(200, 1, 200), true)
dump("pasteover", V(197, 0, 197), V(207, 4, 207))
-- Water occupancy read back exactly (WriteVoxels quantizes to a byte).
local occs = {0.1, 0.25, 0.3, 0.5, 0.7, 0.9}
for i, o in occs do
	T:WriteVoxels(Region3.new(V(400 + i * 8, 400, 0), V(404 + i * 8, 404, 4)), 4, {{{M.Water}}}, {{{o}}})
	T:WriteVoxels(Region3.new(V(400 + i * 8, 408, 0), V(404 + i * 8, 412, 4)), 4, {{{M.Rock}}}, {{{o}}})
end
for i, o in occs do
	local r1v = Region3.new(V(400 + i * 8, 400, 0), V(404 + i * 8, 404, 4))
	local r2v = Region3.new(V(400 + i * 8, 408, 0), V(404 + i * 8, 412, 4))
	local _, wo = T:ReadVoxels(r1v, 4)
	local _, so = T:ReadVoxels(r2v, 4)
	local wc = T:ReadVoxelChannels(r1v, 4, {"LiquidOccupancy", "SolidOccupancy"})
	print(string.format("FIDREG|occ|%.2f|water %.6f|rock %.6f|liquid %.6f|solid %.6f", o, wo[1][1][1], so[1][1][1], wc.LiquidOccupancy[1][1][1], wc.SolidOccupancy[1][1][1]))
end
-- The saved form.
local ok, buf = pcall(function()
	return game:GetService("SerializationService"):SerializeInstancesAsync({r1})
end)
if ok then
	local str = buffer.tostring(buf)
	local hex = string.gsub(str, ".", function(c) return string.format("%02x", string.byte(c)) end)
	local n = math.ceil(#hex / 300)
	for i = 1, n do
		print("FIDREG|ser|" .. i .. "|" .. n .. "|" .. string.sub(hex, (i - 1) * 300 + 1, i * 300))
	end
else
	print("FIDREG|ser-error|" .. tostring(buf))
end
print("FIDREG|done")
