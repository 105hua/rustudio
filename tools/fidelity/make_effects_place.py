#!/usr/bin/env python3
"""Writes corpus/effects.rbxlx (round 25): ParticleEmitters, Beams and a
Trail laid out to measure Studio's conventions, in front of a dark wall.

Top row: still particles (Speed 0, each Emit(1) once by the client script)
on attachments: a SquareParticle of Size 4 (how wide is Size?), the arrow
cursor at Size 6 and Rotation 30 (texture way up, rotation sign), Squash 1
(which axis), red with Transparency 0.5 and LightEmission 1 (the blend).
Middle row: LightInfluence 1 vs LightInfluence 0 + Brightness 3 on grey;
a flat Beam (FaceCamera false) with the arrow texture between two
unrotated attachments (which way the width and the texture run).
Bottom row: an S-curved FaceCamera Beam (CurveSize 4 / 4 on attachments
whose Axis points up; Width 1 → 3, red → blue, Transparency 0 → 0.8, no
texture), a Trail on a part the client script swings, and an untextured
spreading emitter (does Texture "" draw?). Only built-in rbxasset textures,
so the offline Studio shows them too."""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
SQUARE = "rbxasset://textures/particles/SquareParticle.png"
ARROW = "rbxasset://textures/ArrowCursor.png"


def cf(x, y, z, r=(1, 0, 0, 0, 1, 0, 0, 0, 1), name="CFrame"):
    rr = "".join(f"<R{i // 3}{i % 3}>{v}</R{i // 3}{i % 3}>" for i, v in enumerate(r))
    return f'<CoordinateFrame name="{name}"><X>{x}</X><Y>{y}</Y><Z>{z}</Z>{rr}</CoordinateFrame>'


def part(ref, name, pos, size, color, children="", extra=""):
    return (f'<Item class="Part" referent="{ref}"><Properties><string name="Name">{name}</string><bool name="Anchored">true</bool>'
            f'<bool name="CanCollide">false</bool>{cf(*pos)}<Vector3 name="size"><X>{size[0]}</X><Y>{size[1]}</Y><Z>{size[2]}</Z></Vector3>'
            f'<Color3uint8 name="Color3uint8">{color}</Color3uint8>{extra}</Properties>{children}</Item>')


def attachment(ref, name, pos, children="", r=(1, 0, 0, 0, 1, 0, 0, 0, 1)):
    return (f'<Item class="Attachment" referent="{ref}"><Properties><string name="Name">{name}</string>{cf(*pos, r=r)}'
            f'</Properties>{children}</Item>')


def nseq(*kp):
    """NumberSequence from (time, value[, envelope]) keypoints."""
    return " ".join(f"{k[0]} {k[1]} {k[2] if len(k) > 2 else 0}" for k in kp) + " "


def cseq(*kp):
    return " ".join(f"{t} {r} {g} {b} 0" for t, r, g, b in kp) + " "


def content(name, url):
    return f'<Content name="{name}"><url>{url}</url></Content>' if url else f'<Content name="{name}"><null></null></Content>'


def emitter(ref, name, texture, size=4.0, rotation=0, squash=0, color=(1, 1, 1), transparency=0.0,
            emission=0.0, influence=0.0, brightness=1.0, enabled=False, rate=0.0, speed=0.0,
            lifetime=20.0, spread=(0, 0), extra=""):
    return (f'<Item class="ParticleEmitter" referent="{ref}"><Properties><string name="Name">{name}</string>{extra}'
            f'{content("Texture", texture)}<bool name="Enabled">{"true" if enabled else "false"}</bool>'
            f'<float name="Rate">{rate}</float><NumberRange name="Lifetime">{lifetime} {lifetime} </NumberRange>'
            f'<NumberRange name="Speed">{speed} {speed} </NumberRange>'
            f'<NumberRange name="Rotation">{rotation} {rotation} </NumberRange><NumberRange name="RotSpeed">0 0 </NumberRange>'
            f'<Vector2 name="SpreadAngle"><X>{spread[0]}</X><Y>{spread[1]}</Y></Vector2>'
            f'<NumberSequence name="Size">{nseq((0, size), (1, size))}</NumberSequence>'
            f'<NumberSequence name="Squash">{nseq((0, squash), (1, squash))}</NumberSequence>'
            f'<NumberSequence name="Transparency">{nseq((0, transparency), (1, transparency))}</NumberSequence>'
            f'<ColorSequence name="Color">{cseq((0, *color), (1, *color))}</ColorSequence>'
            f'<float name="LightEmission">{emission}</float><float name="LightInfluence">{influence}</float>'
            f'<float name="Brightness">{brightness}</float><float name="ZOffset">0</float>'
            f'</Properties></Item>')


def beam(ref, name, a0, a1, texture, width=(3, 3), curve=(0, 0), face=False, segments=10,
         color=((0, 1, 1, 1), (1, 1, 1, 1)), transparency=((0, 0), (1, 0)), tex_mode=0, tex_length=1, extra=""):
    return (f'<Item class="Beam" referent="{ref}"><Properties><string name="Name">{name}</string>{extra}'
            f'<Ref name="Attachment0">{a0}</Ref><Ref name="Attachment1">{a1}</Ref>{content("Texture", texture)}'
            f'<float name="Width0">{width[0]}</float><float name="Width1">{width[1]}</float>'
            f'<float name="CurveSize0">{curve[0]}</float><float name="CurveSize1">{curve[1]}</float>'
            f'<bool name="FaceCamera">{"true" if face else "false"}</bool><int name="Segments">{segments}</int>'
            f'<ColorSequence name="Color">{cseq(*color)}</ColorSequence>'
            f'<NumberSequence name="Transparency">{nseq(*transparency)}</NumberSequence>'
            f'<token name="TextureMode">{tex_mode}</token><float name="TextureLength">{tex_length}</float>'
            f'<float name="TextureSpeed">0</float><float name="LightEmission">0</float><float name="LightInfluence">0</float>'
            f'<float name="Brightness">1</float></Properties></Item>')


def trail(ref, a0, a1, lifetime=1, color=((0, 1, 1, 1), (1, 0.2, 1, 0.2)), transparency=((0, 0), (1, 1))):
    return (f'<Item class="Trail" referent="{ref}"><Properties><string name="Name">Trail</string>'
            f'<Ref name="Attachment0">{a0}</Ref><Ref name="Attachment1">{a1}</Ref>{content("Texture", "")}'
            f'<float name="Lifetime">{lifetime}</float><float name="MinLength">0.1</float><float name="MaxLength">0</float>'
            f'<ColorSequence name="Color">{cseq(*color)}</ColorSequence>'
            f'<NumberSequence name="Transparency">{nseq(*transparency)}</NumberSequence>'
            f'<NumberSequence name="WidthScale">{nseq((0, 1), (1, 1))}</NumberSequence>'
            f'<bool name="FaceCamera">false</bool><float name="LightEmission">0</float><float name="LightInfluence">0</float>'
            f'<float name="Brightness">1</float></Properties></Item>')


# Attachments with their X axis (Axis) pointing up: rotated 90° about Z.
UP_X = (0, -1, 0, 1, 0, 0, 0, 0, 1)

cam = '''local cam = workspace.CurrentCamera
local RunService = game:GetService("RunService")
RunService.RenderStepped:Connect(function()
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = CFrame.lookAt(Vector3.new(0, 4, 34), Vector3.new(0, 4, 0))
end)
-- The still particles: one each, once.
local holder = workspace:WaitForChild("Holder")
task.wait(1)
for _, a in holder:GetChildren() do
	local e = a:FindFirstChildOfClass("ParticleEmitter")
	if e then e:Emit(1) end
end
-- The trail's part swings left and right.
local swing = workspace:WaitForChild("Swing")
local t0 = os.clock()
RunService.RenderStepped:Connect(function()
	local t = os.clock() - t0
	swing.CFrame = CFrame.new(4 + math.sin(t * 2) * 4, -1, 2)
end)'''

holder_children = "".join([
    attachment("A1", "Size4", (-12, 10, 0), emitter("E1", "E", SQUARE, size=4)),
    attachment("A2", "Arrow", (-4, 10, 0), emitter("E2", "E", ARROW, size=6, rotation=30)),
    attachment("A3", "Squash", (4, 10, 0), emitter("E3", "E", SQUARE, size=4, squash=1)),
    attachment("A4", "Emission", (12, 10, 0), emitter("E4", "E", SQUARE, size=4, color=(1, 0.1, 0.1), transparency=0.5, emission=1)),
    attachment("A5", "Lit", (-12, 3, 0), emitter("E5", "E", SQUARE, size=4, color=(0.5, 0.5, 0.5), influence=1)),
    attachment("A6", "Bright", (-4, 3, 0), emitter("E6", "E", SQUARE, size=4, color=(0.5, 0.5, 0.5), brightness=3)),
    # The flat beam's ends (unrotated: Axis +X, SecondaryAxis +Y).
    attachment("B0", "FlatA", (1, 3, 0)),
    attachment("B1", "FlatB", (15, 3, 0), beam("BM1", "Flat", "RBXB0", "RBXB1", ARROW, width=(3, 3))),
    # The S curve's ends (Axis up).
    attachment("C0", "CurveA", (-15, -2, 0), r=UP_X),
    attachment("C1", "CurveB", (-3, -2, 0), beam("BM2", "Curve", "RBXC0", "RBXC1", "", width=(1, 3), curve=(4, 4), face=True,
                                                    segments=20, color=((0, 1, 0, 0), (1, 0, 0, 1)),
                                                    transparency=((0, 0), (1, 0.8))), r=UP_X),
])
swing_children = (attachment("T0", "Top", (0, 1, 0)) + attachment("T1", "Bottom", (0, -1, 0))
                  + trail("TR", "RBXT0", "RBXT1"))
items = [
    part(10, "Ground", (0, -5, 0), (200, 2, 200), 4288914085),
    part(11, "Wall", (0, 5, -8), (60, 30, 1), 4281545523),
    '<Item class="SpawnLocation" referent="30"><Properties><string name="Name">SpawnLocation</string><bool name="Anchored">true</bool>'
    + cf(0, -3.5, 80) + '<Vector3 name="size"><X>6</X><Y>1</Y><Z>6</Z></Vector3></Properties></Item>',
    # Attachments need a part: the holder sits at the origin, invisible.
    part("12", "Holder", (0, 0, 0), (1, 1, 1), 4294967295, holder_children, '<float name="Transparency">1</float>'),
    part("13", "Swing", (4, -1, 2), (0.5, 2, 0.5), 4294967295, swing_children),
    part("14", "Spray", (10, -3, 0), (2, 0.2, 2), 4294967295,
         emitter("E7", "Spray", "", size=0.4, enabled=True, rate=30, speed=6, lifetime=1.2, spread=(20, 0))),
]
xml = f'''<roblox version="4">
<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>
{"".join(items)}
</Item>
<Item class="Lighting" referent="2"><Properties><string name="Name">Lighting</string><float name="ClockTime">14</float><token name="Technology">4</token></Properties></Item>
<Item class="StarterPlayer" referent="5"><Properties><string name="Name">StarterPlayer</string></Properties>
<Item class="StarterPlayerScripts" referent="6"><Properties><string name="Name">StarterPlayerScripts</string></Properties>
<Item class="LocalScript" referent="7"><Properties><string name="Name">Cam</string><ProtectedString name="Source">{cam}</ProtectedString></Properties></Item>
</Item></Item>
<Item class="Players" referent="8"><Properties><string name="Name">Players</string></Properties></Item>
</roblox>
'''
# Referents: the XML ones are written as RBX<name> where they are referenced.
for r in ["A1", "A2", "A3", "A4", "A5", "A6", "B0", "B1", "C0", "C1", "T0", "T1", "E1", "E2", "E3", "E4", "E5", "E6", "E7",
          "BM1", "BM2", "TR"]:
    xml = xml.replace(f'referent="{r}"', f'referent="RBX{r}"')
open(os.path.join(HERE, "corpus", "effects.rbxlx"), "w").write(xml)
print("ok")

# ---------------------------------------------------------------- lanes
# corpus/effects-lanes.rbxlx (round 26): what effects.rbxlx left to the docs.
# Top: two screens of still particles, Rate 1000 and Rate 100 (the
# per-emitter particle limit: how much of each screen is covered at 6 s
# and 12 s). Middle: the Cylinder and Disc shapes (where particles start,
# which way they move). Left: four flat beams with the arrow texture,
# TextureLength 4 over 15 studs (Stretch, Wrap, Static at Width 2 and 4).
# Right: three trails on parts moving at a steady 6 studs/s (colour only,
# transparency only, both), so distance from the head is age.
SHAPE_SIZE = (6, 4, 2)


def shaped(ref, name, shape, direction, style=0, partial=1.0, speed=0.0, rate=0.0, lifetime=30.0, color=(1, 1, 1),
           enabled=False):
    extra = (f'<token name="Shape">{shape}</token><token name="EmissionDirection">{direction}</token>'
             f'<token name="ShapeStyle">{style}</token><float name="ShapePartial">{partial}</float>')
    return emitter(ref, name, SQUARE, size=0.15, color=color, speed=speed, rate=rate, lifetime=lifetime,
                   enabled=enabled, extra=extra)


lanes_cam = '''local cam = workspace.CurrentCamera
local RunService = game:GetService("RunService")
RunService.RenderStepped:Connect(function()
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = CFrame.lookAt(Vector3.new(0, 8, 40), Vector3.new(0, 8, 0))
end)
-- The trails' parts: 6 studs/s from x 2 to 26 (4 s), then 2 s still (the
-- old points expire), then back; time() + 2 puts both shots mid-sweep.
local movers = {}
for _, p in workspace:GetChildren() do
	if p.Name == "Mover" then table.insert(movers, p) end
end
RunService.RenderStepped:Connect(function()
	local ph = (time() + 2) % 6
	local x = 2 + 6 * math.min(ph, 4)
	for _, p in movers do
		p.CFrame = CFrame.new(x, p.Position.Y, 0)
	end
end)
-- The still shape particles: 300 each, once.
task.wait(1)
for _, e in workspace:GetDescendants() do
	if e:IsA("ParticleEmitter") and e.Name == "Still" then e:Emit(300) end
end'''

lane_items = [
    part(10, "Ground", (0, -5, 0), (200, 2, 200), 4288914085),
    part(11, "Wall", (0, 12, -8), (90, 56, 1), 4279900698),
    '<Item class="SpawnLocation" referent="30"><Properties><string name="Name">SpawnLocation</string><bool name="Anchored">true</bool>'
    + cf(0, -3.5, 80) + '<Vector3 name="size"><X>6</X><Y>1</Y><Z>6</Z></Vector3></Properties></Item>',
    # The particle-limit screens (thin boxes facing the camera).
    part(40, "Cap1000", (-15, 28, 0), (26, 8, 0.05), 4294967295,
         emitter("RBXL1", "Cap", SQUARE, size=0.1, enabled=True, rate=1000, lifetime=60),
         '<float name="Transparency">1</float>'),
    part(41, "Cap100", (15, 28, 0), (26, 8, 0.05), 4294967295,
         emitter("RBXL2", "Cap", SQUARE, size=0.1, enabled=True, rate=100, lifetime=60),
         '<float name="Transparency">1</float>'),
    # Shapes (Box 0, Sphere 1, Cylinder 2, Disc 3; NormalId Right 0, Top 1,
    # Front 5).
    part(42, "CylTop", (-24, 15, 0), SHAPE_SIZE, 4294967295, shaped("RBXS1", "Still", 2, 1),
         '<float name="Transparency">1</float>'),
    part(43, "CylFront", (-12, 15, 0), SHAPE_SIZE, 4294967295, shaped("RBXS2", "Still", 2, 5, style=1),
         '<float name="Transparency">1</float>'),
    part(44, "DiscFront", (0, 15, 0), SHAPE_SIZE, 4294967295, shaped("RBXS3", "Still", 3, 5),
         '<float name="Transparency">1</float>'),
    part(45, "DiscRing", (12, 15, 0), SHAPE_SIZE, 4294967295, shaped("RBXS4", "Still", 3, 5, partial=0.5),
         '<float name="Transparency">1</float>'),
    part(46, "CylMoving", (24, 15, 0), SHAPE_SIZE, 4294967295,
         shaped("RBXS5", "Moving", 2, 1, style=1, speed=2, rate=60, lifetime=1.2, enabled=True, color=(0.3, 1, 0.3))
         + shaped("RBXS6", "Moving", 3, 0, speed=2, rate=60, lifetime=1.2, enabled=True, color=(1, 0.3, 0.3)),
         '<float name="Transparency">1</float>'),
]
beam_lanes = [("Stretch", 0, 2, 8), ("Wrap", 1, 2, 5.5), ("Static", 2, 2, 3), ("StaticWide", 2, 4, -0.5)]
holder = ""
for i, (nm, mode, w, y) in enumerate(beam_lanes):
    holder += attachment(f"RBXBA{i}", nm + "A", (-27, y, 0))
    holder += attachment(f"RBXBB{i}", nm + "B", (-12, y, 0),
                         beam(f"RBXBM{i}", nm, f"RBXBA{i}", f"RBXBB{i}", ARROW, width=(w, w), tex_mode=mode, tex_length=4))
lane_items.append(part("47", "Holder", (0, 0, 0), (1, 1, 1), 4294967295, holder, '<float name="Transparency">1</float>'))
trail_lanes = [("Colour", ((0, 1, 1, 1), (1, 0.2, 1, 0.2)), ((0, 0), (1, 0)), 7),
               ("Fade", ((0, 1, 1, 1), (1, 1, 1, 1)), ((0, 0), (1, 1)), 3.5),
               ("Both", ((0, 1, 1, 1), (1, 0.2, 1, 0.2)), ((0, 0), (1, 1)), 0)]
for i, (nm, col, tr, y) in enumerate(trail_lanes):
    kids = (attachment(f"RBXTA{i}", "Top", (0, 0.6, 0)) + attachment(f"RBXTB{i}", "Bottom", (0, -0.6, 0))
            + trail(f"RBXTR{i}", f"RBXTA{i}", f"RBXTB{i}", color=col, transparency=tr))
    lane_items.append(part(f"5{i}", "Mover", (2, y, 0), (0.3, 1.2, 0.3), 4294967295, kids))

lanes_xml = f'''<roblox version="4">
<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>
{"".join(lane_items)}
</Item>
<Item class="Lighting" referent="2"><Properties><string name="Name">Lighting</string><float name="ClockTime">14</float><token name="Technology">4</token></Properties></Item>
<Item class="StarterPlayer" referent="5"><Properties><string name="Name">StarterPlayer</string></Properties>
<Item class="StarterPlayerScripts" referent="6"><Properties><string name="Name">StarterPlayerScripts</string></Properties>
<Item class="LocalScript" referent="7"><Properties><string name="Name">Cam</string><ProtectedString name="Source">{lanes_cam}</ProtectedString></Properties></Item>
</Item></Item>
<Item class="Players" referent="8"><Properties><string name="Name">Players</string></Properties></Item>
</roblox>
'''
open(os.path.join(HERE, "corpus", "effects-lanes.rbxlx"), "w").write(lanes_xml)
print("ok lanes")
