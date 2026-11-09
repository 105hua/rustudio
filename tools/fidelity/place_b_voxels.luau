-- Place B's lake air pockets (round 26): ReadVoxelChannels around x -8396,
-- z 1230..1276, y 64..116 (the F4 camera's neighbourhood), one line per
-- voxel column: FIDVOX|cx|cy|cz0|material:solid:liquid,… (occupancies × 255).
local T = workspace.Terrain
local lo, hi = Vector3.new(-8420, 56, 1216), Vector3.new(-8368, 124, 1292)
local ch = T:ReadVoxelChannels(Region3.new(lo, hi), 4, {"SolidMaterial", "SolidOccupancy", "LiquidOccupancy"})
local size = ch.Size
for x = 1, size.X do
	for y = 1, size.Y do
		local row = {}
		for z = 1, size.Z do
			local m = ch.SolidMaterial[x][y][z]
			table.insert(row, m.Name .. ":" .. math.round(ch.SolidOccupancy[x][y][z] * 255) .. ":" .. math.round(ch.LiquidOccupancy[x][y][z] * 255))
		end
		print("FIDVOX|" .. (lo.X / 4 + x - 1) .. "|" .. (lo.Y / 4 + y - 1) .. "|" .. (lo.Z / 4) .. "|" .. table.concat(row, ","))
	end
end
print("FIDVOX|done|" .. tostring(size))
