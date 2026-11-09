#!/usr/bin/env python3
"""Writes corpus/sa-mesh.rbxlx (round 26): which MeshParts Studio draws a
SurfaceAppearance on. Place B Newer's sky domes (mesh 1527559, a version
1.00 mesh, with a SurfaceAppearance cloud ColorMap) draw plain Color in
Studio, even with the SurfaceAppearance's Color set white (place-b-dome).
Five orange MeshParts in a row under the day sky, each from the offline Studio's local
assets: the v1.00 mesh with the dome's SurfaceAppearance; a v2.00 and a
v4.01 mesh with the same; the v1.00 mesh with the cloud image as its
TextureID instead; the v4.01 one with the SurfaceAppearance and
DoubleSided. Lighting holds two Skys: SkyA (empty: the default sky) first,
SkyB (every face the cloud image) second; which one draws? Also writes
corpus/sa-far.rbxlx (see below)."""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
CLOUD = "rbxassetid://18252487167"


def v3(name, x, y, z, kind="Vector3"):
    return f'<{kind} name="{name}"><X>{x}</X><Y>{y}</Y><Z>{z}</Z></{kind}>'


def cf(x, y, z):
    return (f'<CoordinateFrame name="CFrame"><X>{x}</X><Y>{y}</Y><Z>{z}</Z><R00>1</R00><R01>0</R01><R02>0</R02>'
            f'<R10>0</R10><R11>1</R11><R12>0</R12><R20>0</R20><R21>0</R21><R22>1</R22></CoordinateFrame>')


def meshpart(ref, name, x, mesh, initial, size, sa=True, texture=None, double=False, at=None):
    pos = at or (x, size[1] / 2, -40)
    props = (f'<string name="Name">{name}</string><bool name="Anchored">true</bool>{cf(*pos)}'
             f'{v3("size", *size)}{v3("InitialSize", *initial)}'
             f'<Content name="MeshId"><url>rbxassetid://{mesh}</url></Content>'
             f'<Content name="TextureID">{"<url>" + texture + "</url>" if texture else "<null></null>"}</Content>'
             f'<Color3uint8 name="Color3uint8">{0xFF000000 | (213 << 16) | (115 << 8) | 61}</Color3uint8>'
             f'<bool name="DoubleSided">{"true" if double else "false"}</bool>')
    kids = ""
    if sa:
        kids = (f'<Item class="SurfaceAppearance" referent="{ref}sa"><Properties><string name="Name">SurfaceAppearance</string>'
                f'<Content name="ColorMap"><url>{CLOUD}</url></Content><token name="AlphaMode">0</token></Properties></Item>')
    return f'<Item class="MeshPart" referent="{ref}"><Properties>{props}</Properties>{kids}</Item>'


parts = [
    meshpart("m1", "V1SA", -28, 1527559, (1.9592, 2.4825, 1.9565), (10, 12, 10)),
    meshpart("m2", "V2SA", -14, 12221505, (1, 2, 1), (6, 12, 6)),
    meshpart("m3", "V4SA", 0, 6572715905, (2, 1.185, 2), (10, 6, 10)),
    meshpart("m4", "V1Texture", 14, 1527559, (1.9592, 2.4825, 1.9565), (10, 12, 10), sa=False, texture=CLOUD),
    meshpart("m5", "V4SADouble", 28, 6572715905, (2, 1.185, 2), (10, 6, 10), double=True),
]
xml = ('<roblox version="4">\n<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>\n'
       '<Item class="Part" referent="2"><Properties><string name="Name">Floor</string><bool name="Anchored">true</bool>'
       + cf(0, -0.5, 0) + v3("size", 200, 1, 200) + '<Color3uint8 name="Color3uint8">4284506208</Color3uint8></Properties></Item>\n'
       + "\n".join(parts) + "\n</Item>\n"
       '<Item class="Lighting" referent="3"><Properties><string name="Name">Lighting</string><float name="ClockTime">14</float>'
       '<token name="Technology">3</token></Properties>'
       '<Item class="Sky" referent="3a"><Properties><string name="Name">SkyA</string></Properties></Item>'
       '<Item class="Sky" referent="3b"><Properties><string name="Name">SkyB</string>'
       + "".join(f'<Content name="Skybox{f}"><url>{CLOUD}</url></Content>' for f in ("Bk", "Dn", "Ft", "Lf", "Rt", "Up"))
       + '</Properties></Item></Item>\n'
       '<Item class="StarterPlayer" referent="4"><Properties><string name="Name">StarterPlayer</string></Properties>\n'
       '<Item class="StarterPlayerScripts" referent="5"><Properties><string name="Name">StarterPlayerScripts</string></Properties>\n'
       '<Item class="LocalScript" referent="6"><Properties><string name="Name">SACamera</string><ProtectedString name="Source">'
       'local RunService = game:GetService("RunService")\nlocal cam = workspace.CurrentCamera\n'
       'local at = CFrame.lookAt(Vector3.new(0, 10, 10), Vector3.new(0, 5, -40))\n'
       'RunService:BindToRenderStep("SACam", Enum.RenderPriority.Last.Value, function()\n'
       '\tcam.CameraType = Enum.CameraType.Scriptable\n\tcam.FieldOfView = 70\n\tcam.CFrame = at\nend)\n'
       '</ProtectedString></Properties></Item>\n</Item></Item>\n'
       '<Item class="Players" referent="7"><Properties><string name="Name">Players</string></Properties></Item>\n'
       '</roblox>\n')
open(os.path.join(HERE, "corpus", "sa-mesh.rbxlx"), "w").write(xml)
print("wrote corpus/sa-mesh.rbxlx")

# corpus/sa-far.rbxlx: the same v1.00 sphere with the SurfaceAppearance at
# 100, 300, 600 and 1200 studs (sized 0.15 × the distance, so alike on
# screen), then (from 10 s) the view from inside a 300-stud DoubleSided one,
# as Place B's domes are seen.
V1 = (1.9592, 2.4825, 1.9565)
far = [meshpart(f"f{d}", f"Far{d}", 0, 1527559, V1, (0.15 * d,) * 3, at=(d * x, 0.15 * d * 0.6, -d))
       for d, x in ((100, -0.45), (300, -0.15), (600, 0.15), (1200, 0.45))]
far.append(meshpart("dome", "Dome", 0, 1527559, V1, (300, 300, 300), double=True, at=(3000, 0, 0)))
cam = ('local RunService = game:GetService("RunService")\nlocal cam = workspace.CurrentCamera\n'
       'local A = CFrame.lookAt(Vector3.new(0, 20, 20), Vector3.new(0, 20, -100))\n'
       'local B = CFrame.lookAt(Vector3.new(3000, 20, 0), Vector3.new(3000, 60, -100))\n'
       'local t0 = os.clock()\n'
       'RunService:BindToRenderStep("SAFarCam", Enum.RenderPriority.Last.Value, function()\n'
       '\tcam.CameraType = Enum.CameraType.Scriptable\n\tcam.FieldOfView = 70\n'
       '\tcam.CFrame = if os.clock() - t0 >= 10 then B else A\nend)\n')
xml = ('<roblox version="4">\n<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>\n'
       '<Item class="Part" referent="2"><Properties><string name="Name">Floor</string><bool name="Anchored">true</bool>'
       + cf(0, -0.5, 0) + v3("size", 200, 1, 200) + '<Color3uint8 name="Color3uint8">4284506208</Color3uint8></Properties></Item>\n'
       + "\n".join(far) + "\n</Item>\n"
       '<Item class="Lighting" referent="3"><Properties><string name="Name">Lighting</string><float name="ClockTime">14</float>'
       '<token name="Technology">3</token></Properties></Item>\n'
       '<Item class="StarterPlayer" referent="4"><Properties><string name="Name">StarterPlayer</string></Properties>\n'
       '<Item class="StarterPlayerScripts" referent="5"><Properties><string name="Name">StarterPlayerScripts</string></Properties>\n'
       '<Item class="LocalScript" referent="6"><Properties><string name="Name">SAFarCamera</string><ProtectedString name="Source">'
       + cam + '</ProtectedString></Properties></Item>\n</Item></Item>\n'
       '<Item class="Players" referent="7"><Properties><string name="Name">Players</string></Properties></Item>\n'
       '</roblox>\n')
open(os.path.join(HERE, "corpus", "sa-far.rbxlx"), "w").write(xml)
print("wrote corpus/sa-far.rbxlx")
