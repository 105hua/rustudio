-- Round 22's movement course, built on the server in both engines: one lane
-- per probe segment (movement_probe.luau), 24 studs apart along X, each
-- starting at z = 0 and running toward -Z on a baseplate whose top is y = 0.
-- Sets workspace "CourseReady" when done.
local Course = Instance.new("Folder")
Course.Name = "Course"
Course.Parent = workspace

local LANE = 24
local function lane(i)
	return i * LANE
end

local function part(props)
	local p = Instance.new(props.class or "Part")
	p.Anchored = props.anchored ~= false
	p.Size = props.size
	p.CFrame = props.cf
	p.TopSurface = Enum.SurfaceType.Smooth
	p.BottomSurface = Enum.SurfaceType.Smooth
	p.Color = props.color or Color3.fromRGB(150, 150, 150)
	if props.name then
		p.Name = props.name
	end
	if props.physical then
		p.CustomPhysicalProperties = props.physical
	end
	if props.transparency then
		p.Transparency = props.transparency
	end
	p.Parent = props.parent or Course
	return p
end

-- Ground: one baseplate under every lane.
part({ name = "Ground", size = Vector3.new(60 * LANE, 2, 240), cf = CFrame.new(30 * LANE - LANE, -1, -60), color = Color3.fromRGB(90, 140, 90) })

-- The lanes, by name (movement_probe.luau's segment names).
local L = {}
local order = {
	"walk_stop", "sprint", "turn90", "turn180", "jump_stand", "jump_run", "jump_power", "jump_low",
	"ledge", "step_05", "step_10", "step_15", "step_20", "step_25", "step_30",
	"ramp_30", "ramp_45", "ramp_60", "ramp_75", "ramp_85",
	"wall_slide", "corner", "thin_wall", "gap_16", "gap_22", "gap_30", "ceiling_walk", "ceiling_jump",
	"truss", "seat", "vehicle_seat", "conveyor", "spinner", "tween_platform", "slippery", "grippy",
	"terrain_30", "terrain_55", "water", "push_block", "falling_down", "platform_stand", "edge_stand",
	"friction_001", "friction_005", "friction_02", "friction_w1",
	"wall_jump", "wall_jump_diag",
}
for i, name in order do
	L[name] = lane(i)
end
Course:SetAttribute("Lanes", table.concat(order, ","))
Course:SetAttribute("LaneWidth", LANE)

local function at(name, x, y, z)
	return CFrame.new(L[name] + x, y, z)
end

-- Ledge: start on a block 8 high whose edge is 4 studs ahead.
part({ size = Vector3.new(16, 8, 16), cf = at("ledge", 0, 4, 4) })

-- Steps across the lane from z = -6.
for _, s in { { "step_05", 0.5 }, { "step_10", 1 }, { "step_15", 1.5 }, { "step_20", 2 }, { "step_25", 2.5 }, { "step_30", 3 } } do
	part({ size = Vector3.new(16, s[2], 20), cf = at(s[1], 0, s[2] / 2, -16) })
end

-- Ramps rising toward -Z from z = -4 (slabs turned about X; top surface
-- meets the ground at z = -4).
for _, r in { { "ramp_30", 30 }, { "ramp_45", 45 }, { "ramp_60", 60 }, { "ramp_75", 75 }, { "ramp_85", 85 } } do
	local a = math.rad(r[2])
	local len = 40
	local dir = Vector3.new(0, math.sin(a), -math.cos(a))
	local nrm = Vector3.new(0, math.cos(a), math.sin(a))
	local base = Vector3.new(L[r[1]], 0, -4)
	local c = base + dir * (len / 2) - nrm * 0.5
	part({ size = Vector3.new(16, 1, len), cf = CFrame.new(c) * CFrame.Angles(a, 0, 0), color = Color3.fromRGB(170, 120, 80) })
end

-- Wall along the lane 3 studs to the right (+X) for sliding.
part({ size = Vector3.new(1, 10, 80), cf = at("wall_slide", 3.5, 5, -38) })
-- Inside corner ahead-right.
part({ size = Vector3.new(1, 10, 20), cf = at("corner", 4.5, 5, -10) })
part({ size = Vector3.new(20, 10, 1), cf = at("corner", -5, 5, -10.5) })
-- Thin wall across the lane.
part({ size = Vector3.new(16, 10, 0.2), cf = at("thin_wall", 0, 5, -6) })
-- Gaps of 1.6, 2.2 and 3 studs (two walls each side of the lane centre).
for _, g in { { "gap_16", 1.6 }, { "gap_22", 2.2 }, { "gap_30", 3 } } do
	for _, side in { -1, 1 } do
		part({ size = Vector3.new(8, 10, 4), cf = at(g[1], side * (g[2] / 2 + 4), 5, -8) })
	end
end
-- Low ceilings: 4.5 above the ground (walk under) and 8 (jump under).
part({ size = Vector3.new(16, 1, 20), cf = at("ceiling_walk", 0, 4.5 + 0.5, -16) })
part({ size = Vector3.new(16, 1, 20), cf = at("ceiling_jump", 0, 8 + 0.5, 0) })

-- Truss ahead.
part({ class = "TrussPart", size = Vector3.new(2, 40, 2), cf = at("truss", 0, 20, -5) })
-- Seats.
part({ class = "Seat", size = Vector3.new(2, 1, 2), cf = at("seat", 0, 0.5, -6), anchored = true })
part({ class = "VehicleSeat", size = Vector3.new(2, 1, 2), cf = at("vehicle_seat", 0, 0.5, -6), anchored = true })

-- Conveyor: anchored, its AssemblyLinearVelocity carries what stands on it.
local conv = part({ size = Vector3.new(16, 1, 40), cf = at("conveyor", 0, 0.5, -10), color = Color3.fromRGB(60, 60, 60) })
conv.AssemblyLinearVelocity = Vector3.new(0, 0, -10)
-- Spinner: anchored disc turning at 1 rad/s.
local spin = part({ class = "Part", size = Vector3.new(1, 24, 24), cf = at("spinner", 0, 0.5, 0) * CFrame.Angles(0, 0, math.rad(90)), color = Color3.fromRGB(200, 80, 80) })
spin.Shape = Enum.PartType.Cylinder
spin.AssemblyAngularVelocity = Vector3.new(0, 1, 0)
-- Tweened anchored platform (the probe moves it 20 studs along −Z over 3 s,
-- on the client).
local tp = part({ name = "TweenPlatform", size = Vector3.new(10, 1, 10), cf = at("tween_platform", 0, 3, 0), color = Color3.fromRGB(80, 80, 200) })
part({ size = Vector3.new(4, 3, 4), cf = at("tween_platform", 0, 1.5, 7) }) -- a step up onto it
-- Friction floors.
part({ size = Vector3.new(16, 1, 40), cf = at("slippery", 0, 0.5, -10), physical = PhysicalProperties.new(0.7, 0, 0.5, 100, 1), color = Color3.fromRGB(180, 220, 255) })
part({ size = Vector3.new(16, 1, 40), cf = at("grippy", 0, 0.5, -10), physical = PhysicalProperties.new(0.7, 2, 0.5, 100, 1), color = Color3.fromRGB(120, 60, 40) })
-- Friction steps for Studio's braking curve (weight 100: the floor's wins),
-- and Friction 0 at weight 1 (mixed with the character's).
for _, f in { { "friction_001", 0.01, 100 }, { "friction_005", 0.05, 100 }, { "friction_02", 0.2, 100 }, { "friction_w1", 0, 1 } } do
	part({ size = Vector3.new(16, 1, 40), cf = at(f[1], 0, 0.5, -10), physical = PhysicalProperties.new(0.7, f[2], 0.5, f[3], 1), color = Color3.fromRGB(200, 230, 255) })
end

-- Terrain: wedges rising toward -Z, and a water pool.
local T = workspace.Terrain
for _, r in { { "terrain_30", 30 }, { "terrain_55", 55 } } do
	local a = math.rad(r[2])
	local run = 24
	local h = run * math.tan(a)
	-- FillWedge: the slope rises toward the wedge's −Z side (a WedgePart's
	-- high edge; turned by π both engines drew a cliff).
	T:FillWedge(CFrame.new(L[r[1]], h / 2, -4 - run / 2), Vector3.new(16, h, run), Enum.Material.Grass)
end
-- Water standing on the ground, 8 deep, from z = -6.
T:FillBlock(CFrame.new(L.water, 4, -26), Vector3.new(16, 8, 40), Enum.Material.Water)

-- An unanchored block to push.
part({ name = "PushBlock", size = Vector3.new(2, 2, 2), cf = at("push_block", 0, 1, -5), anchored = false, color = Color3.fromRGB(230, 200, 60) })

-- Round 23: tall walls across the lane 4 studs ahead, to jump into.
part({ size = Vector3.new(16, 30, 1), cf = at("wall_jump", 0, 15, -4.5) })
part({ size = Vector3.new(16, 30, 1), cf = at("wall_jump_diag", 0, 15, -4.5) })

-- Edge: a block 4 high whose edge runs along the lane centre.
part({ size = Vector3.new(16, 4, 16), cf = at("edge_stand", -8, 2, 0) })

Course:SetAttribute("Ready", true)
