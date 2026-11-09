#!/usr/bin/env python3
"""Writes corpus/terraincolor.rbxlx (round 26): flat terrain slabs of the
grey materials (Rock bluer in rustudio than in Studio, terrain T4 / T5) and
a few others, seen from straight above while a client script changes the
light: Ambient alone, OutdoorAmbient alone, the sun alone, then the terrain
place's own light (Ambient = OutdoorAmbient = 128, the sun at 14:00)."""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
build = """local T = workspace.Terrain
local M = Enum.Material
-- 5 x 2 slabs, 40 studs a side, 48 apart, tops at y = 0.
local mats = {M.Rock, M.Concrete, M.Slate, M.Basalt, M.Mud, M.Sand, M.Snow, M.Asphalt, M.Pavement, M.Ground}
for i, m in mats do
	local col, row = (i - 1) % 5, (i - 1) // 5
	T:FillBlock(CFrame.new(col * 48 - 96, -8, row * 48 - 24), Vector3.new(40, 16, 40), m)
end
"""
cams = """local RunService = game:GetService("RunService")
local L = game:GetService("Lighting")
local cam = workspace.CurrentCamera
local g = function(v) return Color3.fromRGB(v, v, v) end
-- {Ambient, OutdoorAmbient, Brightness}
local spots = {{128, 0, 0}, {0, 128, 0}, {0, 0, 2}, {128, 128, 2}}
local t0 = os.clock()
RunService:BindToRenderStep("ColorCams", Enum.RenderPriority.Camera.Value + 1, function()
	local t = os.clock() - t0
	local at = spots[1]
	for i, s in spots do
		if t >= 8 + (i - 1) * 5 - 3 then at = s end
	end
	L.Ambient, L.OutdoorAmbient, L.Brightness = g(at[1]), g(at[2]), at[3]
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 60
	cam.CFrame = CFrame.new(0, 200, 0) * CFrame.Angles(-math.pi / 2, 0, 0)
end)
"""
xml = f'''<roblox version="4">
<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>
<Item class="Terrain" referent="9"><Properties><string name="Name">Terrain</string><bool name="Decoration">false</bool></Properties></Item>
</Item>
<Item class="Lighting" referent="2"><Properties><string name="Name">Lighting</string>
<float name="ClockTime">14</float><bool name="GlobalShadows">true</bool><token name="Technology">3</token></Properties></Item>
<Item class="ServerScriptService" referent="3"><Properties><string name="Name">ServerScriptService</string></Properties>
<Item class="Script" referent="4"><Properties><string name="Name">BuildTerrain</string><ProtectedString name="Source">{build}</ProtectedString></Properties></Item>
</Item>
<Item class="StarterPlayer" referent="5"><Properties><string name="Name">StarterPlayer</string></Properties>
<Item class="StarterPlayerScripts" referent="6"><Properties><string name="Name">StarterPlayerScripts</string></Properties>
<Item class="LocalScript" referent="7"><Properties><string name="Name">ColorCams</string><ProtectedString name="Source">{cams}</ProtectedString></Properties></Item>
</Item></Item>
<Item class="Players" referent="8"><Properties><string name="Name">Players</string><bool name="CharacterAutoLoads">false</bool></Properties></Item>
</roblox>
'''
open(os.path.join(HERE, "corpus", "terraincolor.rbxlx"), "w").write(xml)
