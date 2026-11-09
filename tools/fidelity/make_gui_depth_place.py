#!/usr/bin/env python3
"""Writes corpus/gui-depth.rbxlx (round 23): world GUIs against the world.

A sign (SurfaceGui, a big red TextLabel on its front) with a pillar in front
of its left half; a BillboardGui behind a wall; an AlwaysOnTop BillboardGui
behind the same wall; a SurfaceGui with LightInfluence 1 and one with 0 and
Brightness 2. A client script holds the camera still (both engines)."""
import os

HERE = os.path.dirname(os.path.abspath(__file__))


def cf(x, y, z):
    return (f'<CoordinateFrame name="CFrame"><X>{x}</X><Y>{y}</Y><Z>{z}</Z><R00>1</R00><R01>0</R01><R02>0</R02>'
            f'<R10>0</R10><R11>1</R11><R12>0</R12><R20>0</R20><R21>0</R21><R22>1</R22></CoordinateFrame>')


def part(ref, name, pos, size, color, children=""):
    return (f'<Item class="Part" referent="{ref}"><Properties><string name="Name">{name}</string><bool name="Anchored">true</bool>'
            f'{cf(*pos)}<Vector3 name="size"><X>{size[0]}</X><Y>{size[1]}</Y><Z>{size[2]}</Z></Vector3>'
            f'<Color3uint8 name="Color3uint8">{color}</Color3uint8></Properties>{children}</Item>')


def label(ref, text, rgb, size="<XS>1</XS><XO>0</XO><YS>1</YS><YO>0</YO>"):
    return (f'<Item class="TextLabel" referent="{ref}"><Properties><string name="Name">L</string>'
            f'<UDim2 name="Size">{size}</UDim2><string name="Text">{text}</string><bool name="TextScaled">true</bool>'
            f'<Color3 name="BackgroundColor3"><R>{rgb[0]}</R><G>{rgb[1]}</G><B>{rgb[2]}</B></Color3></Properties></Item>')


def surface(ref, li, bright, lab):
    return (f'<Item class="SurfaceGui" referent="{ref}"><Properties><string name="Name">Sign</string><token name="Face">5</token>'
            f'<float name="LightInfluence">{li}</float><float name="Brightness">{bright}</float></Properties>{lab}</Item>')


def billboard(ref, on_top, lab):
    return (f'<Item class="BillboardGui" referent="{ref}"><Properties><string name="Name">BB</string>'
            f'<bool name="AlwaysOnTop">{"true" if on_top else "false"}</bool><float name="LightInfluence">0</float>'
            f'<UDim2 name="Size"><XS>6</XS><XO>0</XO><YS>3</YS><YO>0</YO></UDim2></Properties>{lab}</Item>')


cam = '''local cam = workspace.CurrentCamera
game:GetService("RunService").RenderStepped:Connect(function()
	cam.CameraType = Enum.CameraType.Scriptable
	cam.FieldOfView = 70
	cam.CFrame = CFrame.lookAt(Vector3.new(0, 6, 24), Vector3.new(0, 6, 0))
end)'''
items = [
    part(10, "Ground", (0, -1, 0), (200, 2, 200), 4288914085),
    # (the character spawns behind the camera)
    '<Item class="SpawnLocation" referent="30"><Properties><string name="Name">SpawnLocation</string><bool name="Anchored">true</bool>' + cf(0, 0.5, 70) + '<Vector3 name="size"><X>6</X><Y>1</Y><Z>6</Z></Vector3></Properties></Item>',
    # The sign at z = 0 facing the camera (Front = -Z... the camera is at +Z: face Back = 2).
    part(11, "Sign", (-6, 6, 0), (8, 6, 1), 4294967295,
         surface(12, 0, 1, label(13, "BEHIND", (1, 0.2, 0.2))).replace('<token name="Face">5</token>', '<token name="Face">2</token>')),
    part(14, "Pillar", (-8, 6, 4), (2, 12, 2), 4281545523),
    # A wall with two billboards behind it.
    part(15, "Wall", (6, 5, 4), (6, 10, 1), 4285098345),
    part(16, "Anchor", (5, 5, -2), (1, 1, 1), 4294967295, billboard(17, False, label(18, "HIDDEN", (0.2, 0.8, 0.2)))),
    part(19, "Anchor2", (7, 8, -2), (1, 1, 1), 4294967295, billboard(20, True, label(21, "ON TOP", (0.2, 0.4, 1)))),
    # LightInfluence 1 vs 0 + Brightness 2, side by side.
    part(22, "Lit", (-3, 1.5, 6), (4, 3, 1), 4294967295,
         surface(23, 1, 1, label(24, "LI 1", (0.8, 0.8, 0.8))).replace('<token name="Face">5</token>', '<token name="Face">2</token>')),
    part(25, "Bright", (2, 1.5, 6), (4, 3, 1), 4294967295,
         surface(26, 0, 2, label(27, "B 2", (0.4, 0.4, 0.4))).replace('<token name="Face">5</token>', '<token name="Face">2</token>')),
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
open(os.path.join(HERE, "corpus", "gui-depth.rbxlx"), "w").write(xml)
print("ok")
