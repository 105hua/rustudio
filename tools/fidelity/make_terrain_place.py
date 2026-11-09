#!/usr/bin/env python3
"""Writes corpus/terrain.rbxlx (round 21's terrain comparison place) from
terrain_build.luau (server: the voxels) and terrain_cams.luau (client: a
fixed camera per shot). Terrain.Decoration is NotScriptable, so it is set
on the place's Terrain here."""
import os
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))
b = escape(open(os.path.join(HERE, "terrain_build.luau")).read())
c = escape(open(os.path.join(HERE, "terrain_cams.luau")).read())
xml = f'''<roblox version="4">
<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>
<Item class="Terrain" referent="9"><Properties><string name="Name">Terrain</string><bool name="Decoration">true</bool></Properties></Item>
</Item>
<Item class="Lighting" referent="2"><Properties><string name="Name">Lighting</string>
<float name="ClockTime">14</float><bool name="GlobalShadows">true</bool><token name="Technology">3</token>
</Properties></Item>
<Item class="ServerScriptService" referent="3"><Properties><string name="Name">ServerScriptService</string></Properties>
<Item class="Script" referent="4"><Properties><string name="Name">BuildTerrain</string><ProtectedString name="Source">{b}</ProtectedString></Properties></Item>
</Item>
<Item class="StarterPlayer" referent="5"><Properties><string name="Name">StarterPlayer</string></Properties>
<Item class="StarterPlayerScripts" referent="6"><Properties><string name="Name">StarterPlayerScripts</string></Properties>
<Item class="LocalScript" referent="7"><Properties><string name="Name">TerrainCams</string><ProtectedString name="Source">{c}</ProtectedString></Properties></Item>
</Item></Item>
<Item class="Players" referent="8"><Properties><string name="Name">Players</string><bool name="CharacterAutoLoads">false</bool></Properties></Item>
</roblox>
'''
open(os.path.join(HERE, "corpus", "terrain.rbxlx"), "w").write(xml)
