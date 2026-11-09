#!/usr/bin/env python3
"""Built-in-only glass foreground rejection and ForceField phase calibration."""
from pathlib import Path

def vector(name, x, y, z):
    return f'<Vector3 name="{name}"><X>{x}</X><Y>{y}</Y><Z>{z}</Z></Vector3>'

def part(name, x, y, z, size, color, material=256, transparency=0, shape=1, mesh=False):
    cf = f'<CoordinateFrame name="CFrame"><X>{x}</X><Y>{y}</Y><Z>{z}</Z><R00>1</R00><R01>0</R01><R02>0</R02><R10>0</R10><R11>1</R11><R12>0</R12><R20>0</R20><R21>0</R21><R22>1</R22></CoordinateFrame>'
    cls = 'MeshPart' if mesh else 'Part'
    geometry = ('<Content name="MeshId"><url>rbxasset://avatar/heads/head.mesh</url></Content>' + vector('InitialSize', 2, 1, 1) + '<Content name="TextureID"><url>rbxasset://textures/particles/forcefield_vortex_color.dds</url></Content>') if mesh else f'<token name="shape">{shape}</token>'
    return f'<Item class="{cls}" referent="{name}"><Properties><string name="Name">{name}</string><bool name="Anchored">true</bool>{cf}{vector("size", *size)}<Color3uint8 name="Color3uint8">{0xff000000 | color}</Color3uint8><token name="Material">{material}</token><float name="Transparency">{transparency}</float>{geometry}</Properties></Item>'

def main():
    parts = []
    for x in range(-4, 5):
        for y in range(4):
            parts.append(part(f'checker{x}_{y}', x*7, y*7-3, -56, (7,7,1), 0xc55a32 if (x+y)%2 else 0x409e69))
    for i, (x, transparency) in enumerate([(-22, .25), (-11, .5), (0, .75)]):
        parts.append(part(f'glass{i}', x, 7, -35, (10,14,10), 0x7fcbf0, 1568, transparency, 0))
    parts.append(part('overlap', -11, 7, -45, (10,14,10), 0xeeeeff, 1568, .5, 0))
    parts.append(part('foreground', -6, 7, -26, (1,18,1), 0xf0ca20))
    parts.append(part('forcefield', 12, 7, -35, (10,14,10), 0x7fcbf0, 1584, 0, 0))
    parts.append(part('forcefieldTexture', 24, 7, -35, (10,14,10), 0x7fcbf0, 1584, 0, mesh=True))
    camera = 'local r=game:GetService("RunService") local c=workspace.CurrentCamera r:BindToRenderStep("MaterialCam",Enum.RenderPriority.Last.Value,function() c.CameraType=Enum.CameraType.Scriptable c.FieldOfView=70 c.CFrame=CFrame.lookAt(Vector3.new(0,12,20),Vector3.new(0,7,-35)) end)'
    xml = '<roblox version="4"><Item class="Workspace" referent="w"><Properties><string name="Name">Workspace</string></Properties>' + ''.join(parts) + '</Item><Item class="Lighting" referent="l"><Properties><string name="Name">Lighting</string><float name="ClockTime">14</float><token name="Technology">3</token></Properties></Item><Item class="StarterPlayer" referent="sp"><Properties/><Item class="StarterPlayerScripts" referent="sps"><Properties/><Item class="LocalScript" referent="cam"><Properties><string name="Name">Camera</string><ProtectedString name="Source"><![CDATA[' + camera + ']]></ProtectedString></Properties></Item></Item></Item></roblox>'
    Path(__file__).with_name('corpus').joinpath('materials3.rbxlx').write_text(xml)

if __name__ == '__main__':
    main()
