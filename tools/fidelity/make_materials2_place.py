#!/usr/bin/env python3
"""Glass overlaps and ForceField primitive/mesh/texture calibration, Studio 0.746."""
from pathlib import Path

def v(n,x,y,z):
    return f'<Vector3 name="{n}"><X>{x}</X><Y>{y}</Y><Z>{z}</Z></Vector3>'
def cf(x,y,z):
    return f'<CoordinateFrame name="CFrame"><X>{x}</X><Y>{y}</Y><Z>{z}</Z><R00>1</R00><R01>0</R01><R02>0</R02><R10>0</R10><R11>1</R11><R12>0</R12><R20>0</R20><R21>0</R21><R22>1</R22></CoordinateFrame>'
def part(i,x,z,mat,t=0,mesh=False,texture=False):
    cls='MeshPart' if mesh else 'Part'
    more=('<Content name="MeshId"><url>rbxassetid://1527559</url></Content>'+v('InitialSize',1.9592,2.4825,1.9565)) if mesh else '<token name="shape">0</token>'
    if texture: more+='<Content name="TextureID"><url>rbxasset://textures/particles/forcefield_vortex_color.dds</url></Content>'
    return f'<Item class="{cls}" referent="p{i}"><Properties><string name="Name">{i}</string><bool name="Anchored">true</bool>{cf(x,6,z)}{v("size",9,12,9)}<token name="Material">{mat}</token><float name="Transparency">{t}</float><Color3uint8 name="Color3uint8">4283215615</Color3uint8>{more}</Properties></Item>'
parts=''.join(part(i,x,-35,mat,t,m,tex) for i,x,mat,t,m,tex in [(1,-24,1568,0,False,False),(2,-12,1568,.5,False,False),(3,0,1584,0,False,False),(4,12,1584,0,True,False),(5,24,1584,0,True,True)])
parts+=part(6,-12,-46,1568,.5)
xml=f'<roblox version="4"><Item class="Workspace" referent="w"><Properties><string name="Name">Workspace</string></Properties>{parts}</Item><Item class="Lighting" referent="l"><Properties><string name="Name">Lighting</string><float name="ClockTime">14</float><token name="Technology">3</token></Properties></Item><Item class="StarterPlayer" referent="sp"><Properties/><Item class="StarterPlayerScripts" referent="sps"><Properties/><Item class="LocalScript" referent="cam"><Properties><string name="Name">Camera</string><ProtectedString name="Source"><![CDATA[local r=game:GetService("RunService") local c=workspace.CurrentCamera r:BindToRenderStep("MaterialCam",Enum.RenderPriority.Last.Value,function() c.CameraType=Enum.CameraType.Scriptable c.FieldOfView=70 c.CFrame=CFrame.lookAt(Vector3.new(0,12,20),Vector3.new(0,6,-35)) end)]]></ProtectedString></Properties></Item></Item></Item></roblox>'
Path(__file__).with_name('corpus').joinpath('materials2.rbxlx').write_text(xml)
