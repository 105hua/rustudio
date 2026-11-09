#!/usr/bin/env python3
"""Writes corpus/lightgrid.rbxlx and lightgrid2.rbxlx (round 26's light
grid probes) from lightgrid[2]_build.luau (server: the voxels) and
lightgrid[2]_cams.luau (client: a fixed camera per shot), lit by
OutdoorAmbient alone: Ambient 0, Brightness 0 (no sun), no environment
diffuse / specular, ShadowMap. lightgrid-sun.rbxlx is lightgrid2 under a
low sun alone (Voxel, Brightness 2, ClockTime 16.5, no ambient): Voxel's
sun shadows."""
import os
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))


def rgb(name, v):
    return f'<Color3 name="{name}"><R>{v}</R><G>{v}</G><B>{v}</B></Color3>'


def place(name, sun=False):
    b = escape(open(os.path.join(HERE, name + "_build.luau")).read())
    c = escape(open(os.path.join(HERE, name + "_cams.luau")).read())
    return f'''<roblox version="4">
<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>
<Item class="Terrain" referent="9"><Properties><string name="Name">Terrain</string><bool name="Decoration">false</bool></Properties></Item>
</Item>
<Item class="Lighting" referent="2"><Properties><string name="Name">Lighting</string>
<float name="ClockTime">{16.5 if sun else 14}</float><bool name="GlobalShadows">true</bool><token name="Technology">{1 if sun else 3}</token>
{rgb("Ambient", 0)}{rgb("OutdoorAmbient", 0 if sun else 0.7843137)}<float name="Brightness">{2 if sun else 0}</float>
<float name="EnvironmentDiffuseScale">0</float><float name="EnvironmentSpecularScale">0</float>
<float name="ExposureCompensation">0</float>
</Properties></Item>
<Item class="ServerScriptService" referent="3"><Properties><string name="Name">ServerScriptService</string></Properties>
<Item class="Script" referent="4"><Properties><string name="Name">BuildTerrain</string><ProtectedString name="Source">{b}</ProtectedString></Properties></Item>
</Item>
<Item class="StarterPlayer" referent="5"><Properties><string name="Name">StarterPlayer</string></Properties>
<Item class="StarterPlayerScripts" referent="6"><Properties><string name="Name">StarterPlayerScripts</string></Properties>
<Item class="LocalScript" referent="7"><Properties><string name="Name">LightGridCams</string><ProtectedString name="Source">{c}</ProtectedString></Properties></Item>
</Item></Item>
<Item class="Players" referent="8"><Properties><string name="Name">Players</string><bool name="CharacterAutoLoads">false</bool></Properties></Item>
</roblox>
'''


for n in ("lightgrid", "lightgrid2"):
    open(os.path.join(HERE, "corpus", n + ".rbxlx"), "w").write(place(n))
open(os.path.join(HERE, "corpus", "lightgrid-sun.rbxlx"), "w").write(place("lightgrid2", sun=True))
