-- Atmosphere calibration (round 26, corpus/atmos.rbxlx): steps through
-- Atmosphere and Lighting settings, each held 6 s from this script's start
-- (sent 2 s after Play, so a step's window is 2 + 6k .. 8 + 6k s after
-- Play and its shot at 6.5 + 6k s: Studio may run the script up to 4 s
-- late). View A looks level down the row of walls, view B 50° up.
local RunService = game:GetService("RunService")
local Lighting = game:GetService("Lighting")
local cam = workspace.CurrentCamera
local atmos = Lighting:FindFirstChildOfClass("Atmosphere")

local A = CFrame.lookAt(Vector3.new(0, 10, 0), Vector3.new(0, 10, -100))
local B = CFrame.new(0, 10, 0) * CFrame.Angles(math.rad(50), 0, 0)

local NIGHT = {
	ClockTime = 0.5, GeographicLatitude = 18.897, Brightness = 2.6,
	Ambient = Color3.fromRGB(38, 36, 52), OutdoorAmbient = Color3.fromRGB(108, 120, 165),
	ColorShift_Top = Color3.fromRGB(210, 119, 213), EnvironmentDiffuseScale = 0.4,
	EnvironmentSpecularScale = 0.5, ExposureCompensation = 0.15,
}
local DAY = {
	ClockTime = 14, GeographicLatitude = 41.733, Brightness = 2,
	Ambient = Color3.fromRGB(70, 70, 70), OutdoorAmbient = Color3.fromRGB(128, 128, 128),
	ColorShift_Top = Color3.fromRGB(0, 0, 0), EnvironmentDiffuseScale = 0,
	EnvironmentSpecularScale = 0, ExposureCompensation = 0,
}
local PLACE_D = {
	Density = 0.3, Offset = 0.2, Color = Color3.fromRGB(92, 102, 138),
	Decay = Color3.fromRGB(46, 42, 74), Glare = 0, Haze = 1.2,
}
local DEFAULT = {
	Density = 0.395, Offset = 0, Color = Color3.fromRGB(199, 170, 107),
	Decay = Color3.fromRGB(92, 60, 13), Glare = 0, Haze = 0,
}
local function with(base, over)
	local t = table.clone(base)
	for k, v in over do t[k] = v end
	return t
end
local RED = Color3.fromRGB(200, 60, 60)

-- {label, lighting, atmosphere (nil = none), view}
local steps = {
	{"n0 none", NIGHT, nil, A},
	{"n1 place_d", NIGHT, PLACE_D, A},
	{"n1 place_d up", NIGHT, PLACE_D, B},
	{"n2 density 0.6", NIGHT, with(PLACE_D, {Density = 0.6}), A},
	{"n3 density 0.15", NIGHT, with(PLACE_D, {Density = 0.15}), A},
	{"n4 offset 0", NIGHT, with(PLACE_D, {Offset = 0}), A},
	{"n4 offset 0 up", NIGHT, with(PLACE_D, {Offset = 0}), B},
	{"n5 offset 0.6", NIGHT, with(PLACE_D, {Offset = 0.6}), A},
	{"n6 haze 0", NIGHT, with(PLACE_D, {Haze = 0}), A},
	{"n6 haze 0 up", NIGHT, with(PLACE_D, {Haze = 0}), B},
	{"n7 haze 3", NIGHT, with(PLACE_D, {Haze = 3}), A},
	{"n8 color red", NIGHT, with(PLACE_D, {Color = RED}), A},
	{"n8 color red up", NIGHT, with(PLACE_D, {Color = RED}), B},
	{"n9 decay red", NIGHT, with(PLACE_D, {Decay = RED}), A},
	{"n9 decay red up", NIGHT, with(PLACE_D, {Decay = RED}), B},
	{"d0 none", DAY, nil, A},
	{"d1 default", DAY, DEFAULT, A},
	{"d1 default up", DAY, DEFAULT, B},
	{"d2 place_d", DAY, PLACE_D, A},
	{"d2 place_d up", DAY, PLACE_D, B},
	{"d3 density 0.6 offset 0.5", DAY, with(DEFAULT, {Density = 0.6, Offset = 0.5}), A},
}

local HOLD = 6
local t0 = os.clock()
local shown = 0
local function apply()
	local k = math.clamp(math.floor((os.clock() - t0) / HOLD) + 1, 1, #steps)
	local s = steps[k]
	if k ~= shown then
		shown = k
		for p, v in s[2] do
			Lighting[p] = v
		end
		if s[3] then
			for p, v in s[3] do
				atmos[p] = v
			end
			atmos.Parent = Lighting
		else
			atmos.Parent = nil
		end
		print("FIDATMOS", k, s[1], time())
	end
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = s[4]
end
RunService:BindToRenderStep("AtmosCam", Enum.RenderPriority.Last.Value, apply)
RunService.RenderStepped:Connect(apply)
