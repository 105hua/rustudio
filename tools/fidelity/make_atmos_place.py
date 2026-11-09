#!/usr/bin/env python3
"""Writes corpus/atmos.rbxlx (round 26): Atmosphere and the night sky.

A grey floor running 2 000 studs away from a fixed camera, white walls at
25, 50, 100, 200, 400, 800 and 1600 studs (each 0.1 × its distance wide, so
they look the same size), and Studio's default Sky. Lighting starts as
Place D's (ClockTime 0.5 at latitude 18.9, Ambient (38, 36, 52),
OutdoorAmbient (108, 120, 165), ShadowMap, Soft) with Place D's Atmosphere;
atmos_cams.luau then steps through Atmosphere / Lighting settings, each
shot level (A) and some 50° up (B), to measure the sky colour and the fog
over distance."""
import os

HERE = os.path.dirname(os.path.abspath(__file__))


def cf(x, y, z):
    return (f'<CoordinateFrame name="CFrame"><X>{x}</X><Y>{y}</Y><Z>{z}</Z><R00>1</R00><R01>0</R01><R02>0</R02>'
            f'<R10>0</R10><R11>1</R11><R12>0</R12><R20>0</R20><R21>0</R21><R22>1</R22></CoordinateFrame>')


def rgb(r, g, b):
    return str(0xFF000000 | (r << 16) | (g << 8) | b)


def part(ref, name, pos, size, color, mat=256):
    return (f'<Item class="Part" referent="{ref}"><Properties><string name="Name">{name}</string><bool name="Anchored">true</bool>'
            f'{cf(*pos)}<Color3uint8 name="Color3uint8">{rgb(*color)}</Color3uint8><token name="Material">{mat}</token>'
            f'<Vector3 name="size"><X>{size[0]}</X><Y>{size[1]}</Y><Z>{size[2]}</Z></Vector3>'
            f'<bool name="CastShadow">false</bool></Properties></Item>')


def c3(name, r, g, b):
    return f'<Color3 name="{name}"><R>{r / 255}</R><G>{g / 255}</G><B>{b / 255}</B></Color3>'


items = []
ref = 100
# The floor: SmoothPlastic grey, from just behind the camera to 2 040 studs out.
items.append(part(ref, "Floor", (0, -0.5, -1000), (2048, 1, 2048), (128, 128, 128))); ref += 1
dists = [25, 50, 100, 200, 400, 800, 1600]
for i, d in enumerate(dists):
    # Spread across the view, -27° to +27°, so none hides another.
    import math
    ang = math.radians(-27 + 54 * i / (len(dists) - 1))
    w = 0.1 * d
    items.append(part(ref, f"Wall{d}", (round(d * math.tan(ang), 2), w * 0.75, -d), (w, w * 1.5, 1), (255, 255, 255))); ref += 1
items.append(f'<Item class="SpawnLocation" referent="{ref}"><Properties><string name="Name">SpawnLocation</string><bool name="Anchored">true</bool>'
             f'{cf(0, 0.5, 30)}<Vector3 name="size"><X>6</X><Y>1</Y><Z>6</Z></Vector3><bool name="Neutral">true</bool></Properties></Item>'); ref += 1

xml = ('<roblox version="4">\n<Item class="Workspace" referent="1"><Properties><string name="Name">Workspace</string></Properties>\n'
       + "\n".join(items) + "\n</Item>\n"
       '<Item class="Lighting" referent="2"><Properties><string name="Name">Lighting</string>'
       + c3("Ambient", 38, 36, 52) + c3("OutdoorAmbient", 108, 120, 165) + c3("ColorShift_Top", 210, 119, 213)
       + '<float name="Brightness">2.6</float><float name="ClockTime">0.5</float><float name="GeographicLatitude">18.897</float>'
       '<float name="EnvironmentDiffuseScale">0.4</float><float name="EnvironmentSpecularScale">0.5</float>'
       '<float name="ExposureCompensation">0.15</float><bool name="GlobalShadows">true</bool>'
       '<token name="Technology">3</token><token name="LightingStyle">1</token></Properties>\n'
       '<Item class="Sky" referent="3"><Properties><string name="Name">Sky</string></Properties></Item>\n'
       '<Item class="Atmosphere" referent="4"><Properties><string name="Name">Atmosphere</string>'
       '<float name="Density">0.3</float><float name="Offset">0.2</float>' + c3("Color", 92, 102, 138) + c3("Decay", 46, 42, 74)
       + '<float name="Glare">0</float><float name="Haze">1.2</float></Properties></Item>\n'
       '</Item>\n'
       '<Item class="Players" referent="5"><Properties><string name="Name">Players</string></Properties></Item>\n'
       '</roblox>\n')
open(os.path.join(HERE, "corpus", "atmos.rbxlx"), "w").write(xml)
print("wrote corpus/atmos.rbxlx")
