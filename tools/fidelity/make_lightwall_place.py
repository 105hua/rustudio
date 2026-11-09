#!/usr/bin/env python3
"""Writes corpus/lightwall-soft.rbxlx and lightwall-real.rbxlx (round 26):
local lights behind parts, and see-through Neon.

At night (Ambient 100, no sun) over a grey floor, from a fixed camera above:
- two closed rooms (1-stud walls and roof) with a PointLight inside,
  Shadows on and off;
- four walls with a PointLight 3 studs behind them: 1, 0.2 and 4 studs
  thick with Shadows on, 1 stud with Shadows off;
- a SpotLight aimed at a wall, and a closed room lit by a SurfaceLight
  on its ceiling (Shadows on);
- cyan Neon panels at Transparency 0.5, 0.86 and three stacked at 0.86
  (Place D's aurora), and an opaque one.
lightfocus-soft and -real: the same with Camera.Focus moved away (FOCUS).
Soft is Technology ShadowMap with LightingStyle Soft (Place B Newer, Place D);
Real is Technology Future, Realistic."""
import os
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))
ref = [100]
# lightfocus-*: the same scene with Camera.Focus moved away over time (at
# the scene until 8 s, 400 studs off until 14 s, 1500 off until 20 s, then
# left alone): does Studio light only near the Focus?
FOCUS = '''local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local at = CFrame.lookAt(Vector3.new(0, 90, 45), Vector3.new(0, 0, -8))
local t0 = os.clock()
local phase = -1
RunService:BindToRenderStep("CalCam", Enum.RenderPriority.Camera.Value + 1, function()
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = at
	local t = os.clock() - t0
	local p = if t < 8 then 0 elseif t < 14 then 1 elseif t < 20 then 2 else 3
	if p == 0 then cam.Focus = CFrame.new(0, 0, -8)
	elseif p == 1 then cam.Focus = CFrame.new(400, 0, -8)
	elseif p == 2 then cam.Focus = CFrame.new(1500, 0, -8) end
	if p ~= phase then
		phase = p
		print("FOCUS", p, time(), cam.Focus.Position)
	end
end)
'''


def nref():
    ref[0] += 1
    return ref[0]


def cf(x, y, z):
    return (f'<CoordinateFrame name="CFrame"><X>{x}</X><Y>{y}</Y><Z>{z}</Z><R00>1</R00><R01>0</R01><R02>0</R02>'
            f'<R10>0</R10><R11>1</R11><R12>0</R12><R20>0</R20><R21>0</R21><R22>1</R22></CoordinateFrame>')


def rgb(r, g, b):
    return (255 << 24) | (r << 16) | (g << 8) | b


def part(name, pos, size, color=(128, 128, 128), material=256, transparency=0.0, children=""):
    return (f'<Item class="Part" referent="{nref()}"><Properties><string name="Name">{name}</string><bool name="Anchored">true</bool>'
            f'{cf(*pos)}<Vector3 name="size"><X>{size[0]}</X><Y>{size[1]}</Y><Z>{size[2]}</Z></Vector3>'
            f'<Color3uint8 name="Color3uint8">{rgb(*color)}</Color3uint8><token name="Material">{material}</token>'
            f'<float name="Transparency">{transparency}</float></Properties>{children}</Item>')


def light(cls, brightness, rng, shadows, extra=""):
    return (f'<Item class="{cls}" referent="{nref()}"><Properties><string name="Name">{cls}</string>'
            f'<float name="Brightness">{brightness}</float><float name="Range">{rng}</float>'
            f'<bool name="Shadows">{"true" if shadows else "false"}</bool>{extra}</Properties></Item>')


def holder(name, pos, child):
    """An invisible 0.2-stud part carrying a light."""
    return part(name, pos, (0.2, 0.2, 0.2), transparency=1.0, children=child)


def room(name, x, z, inside):
    """A closed 16 x 10 x 16 room (1-stud walls and roof) on the floor."""
    out = [
        part(name + "Roof", (x, 10.5, z), (18, 1, 18), (160, 160, 160)),
        part(name + "N", (x, 5, z - 8.5), (18, 10, 1), (160, 160, 160)),
        part(name + "S", (x, 5, z + 8.5), (18, 10, 1), (160, 160, 160)),
        part(name + "E", (x + 8.5, 5, z), (1, 10, 16), (160, 160, 160)),
        part(name + "W", (x - 8.5, 5, z), (1, 10, 16), (160, 160, 160)),
    ]
    return out + inside


def wall(name, x, z, thick, shadows):
    """A 16 x 8 wall with a PointLight 3 studs behind it (away from the camera)."""
    return [
        part(name, (x, 4, z), (16, 8, thick), (160, 160, 160)),
        holder(name + "Light", (x, 3, z - thick / 2 - 3), light("PointLight", 2, 16, shadows)),
    ]


def make(kind, cam=None):
    ref[0] = 100
    items = [part("Floor", (0, -0.5, -5), (200, 1, 100), (128, 128, 128))]
    # (the character spawns out of view)
    items.append('<Item class="SpawnLocation" referent="99"><Properties><string name="Name">SpawnLocation</string>'
                 '<bool name="Anchored">true</bool>' + cf(0, 0.5, 200) +
                 '<Vector3 name="size"><X>6</X><Y>1</Y><Z>6</Z></Vector3></Properties></Item>')
    items += room("RoomS", -62, -25, [holder("RoomSLight", (-62, 5, -25), light("PointLight", 2, 20, True))])
    items += room("RoomN", -38, -25, [holder("RoomNLight", (-38, 5, -25), light("PointLight", 2, 20, False))])
    items += wall("Wall1", -12, -25, 1, True)
    items += wall("Wall02", 12, -25, 0.2, True)
    items += wall("Wall4", 36, -25, 4, True)
    items += wall("Wall1N", 60, -25, 1, False)
    # Second row: Neon panels, a spot at a wall, a SurfaceLight room.
    cyan = (60, 220, 255)
    items.append(part("Neon0", (-66, 5, 15), (10, 8, 0.5), cyan, 288, 0))
    items.append(part("Neon50", (-52, 5, 15), (10, 8, 0.5), cyan, 288, 0.5))
    items.append(part("Neon86", (-38, 5, 15), (10, 8, 0.5), cyan, 288, 0.86))
    for i in range(3):
        items.append(part(f"Neon86x3_{i}", (-24, 5, 15 + i), (10, 8, 0.5), cyan, 288, 0.86))
    items.append(part("SpotWall", (0, 4, 15), (16, 8, 1), (160, 160, 160)))
    items.append(holder("Spot", (0, 4, 11), light("SpotLight", 2, 20, True,
                                                   '<float name="Angle">90</float><token name="Face">2</token>')))
    items += room("RoomL", 30, 15, [part("Panel", (30, 9.5, 15), (4, 1, 4), (200, 200, 200),
                                         children=light("SurfaceLight", 2, 16, True,
                                                        '<float name="Angle">120</float><token name="Face">4</token>'))])
    tech, style = (3, 1) if kind == "soft" else (4, 0)
    name = "lightwall" if cam is None else "lightfocus" if cam is FOCUS else "lightreach"
    cam = cam or '''local RunService = game:GetService("RunService")
local cam = workspace.CurrentCamera
local at = CFrame.lookAt(Vector3.new(0, 90, 45), Vector3.new(0, 0, -8))
RunService:BindToRenderStep("CalCam", Enum.RenderPriority.Camera.Value + 1, function()
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = at
end)
'''
    xml = f'''<roblox version="4">
<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>
{"".join(items)}
</Item>
<Item class="Lighting" referent="2"><Properties><string name="Name">Lighting</string>
<Color3 name="Ambient"><R>0.392</R><G>0.392</G><B>0.392</B></Color3><float name="Brightness">0</float><float name="ClockTime">0</float>
<float name="EnvironmentDiffuseScale">0</float><float name="EnvironmentSpecularScale">0</float><bool name="GlobalShadows">true</bool>
<Color3 name="OutdoorAmbient"><R>0</R><G>0</G><B>0</B></Color3><token name="Technology">{tech}</token><token name="LightingStyle">{style}</token>
</Properties>
<Item class="Sky" referent="3"><Properties><string name="Name">Sky</string><int name="StarCount">0</int><bool name="CelestialBodiesShown">false</bool></Properties></Item>
</Item>
<Item class="StarterPlayer" referent="5"><Properties><string name="Name">StarterPlayer</string></Properties>
<Item class="StarterPlayerScripts" referent="6"><Properties><string name="Name">StarterPlayerScripts</string></Properties>
<Item class="LocalScript" referent="7"><Properties><string name="Name">CalCamera</string><ProtectedString name="Source">{escape(cam)}</ProtectedString></Properties></Item>
</Item></Item>
<Item class="Players" referent="8"><Properties><string name="Name">Players</string></Properties></Item>
</roblox>
'''
    open(os.path.join(HERE, "corpus", f"{name}-{kind}.rbxlx"), "w").write(xml)


# lightreach-*: Camera.Focus 60, 120 and 200 studs off along +X (6 s each):
# the lights along the first row are then 0..260 studs from it.
REACH = FOCUS.replace("t < 8 then 0 elseif t < 14 then 1 elseif t < 20 then 2", "t < 6 then 0 elseif t < 12 then 1 elseif t < 18 then 2")
REACH = REACH.replace("CFrame.new(0, 0, -8)\n", "CFrame.new(60, 0, -25)\n").replace("CFrame.new(400, 0, -8)", "CFrame.new(120, 0, -25)").replace("CFrame.new(1500, 0, -8)", "CFrame.new(200, 0, -25)")

for k in ("soft", "real"):
    make(k)
    make(k, FOCUS)
    make(k, REACH)
print("ok")
