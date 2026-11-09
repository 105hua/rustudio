#!/usr/bin/env python3
"""Side-by-side pictures of Studio's chrome and rustudio's (checks/ui).

Studio's side is checks/ui/studio/ (captured from an offline Studio,
hidden, 1280x891, over its a Studio MCP bridge MCP: the window, each ribbon tab, each
menu popup, the test drop-down, the account menu, the Explorer). rustudio's
side comes from `rustudio PLACE --window 1280x891 --ui-shots DIR`.

    python3 tools/ui_compare.py RUSTUDIO_SHOTS_DIR [--props DIR] [--output DIR]

writes checks/ui/side-<name>.png (Studio left, rustudio right) and
checks/ui/index.md. --props takes `RUSTUDIO_UI_SHOTS=props rustudio PLACE
--window 1280x1500 --ui-shots DIR` (the Properties of Studio's reference
Part, Model, Script, ScreenGui and Lighting; Studio's pages are its panel
scrolled top to bottom), --output takes `RUSTUDIO_UI_SHOTS=output ...
--window 1280x891` (the Output after print / warn / error with timestamps on).
--editors takes `RUSTUDIO_UI_SHOTS=editors ... --window 1280x891` (the
BrickColor palette popup, the Select Color dialog and the Transparency
slider on Studio's RefPart).
"""
import json
import os
import sys

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
UI = os.path.join(os.path.dirname(os.path.dirname(HERE)), "checks", "ui")
STUDIO = os.path.join(UI, "studio")


def side(a, b, name, label_a="Studio", label_b="rustudio"):
    h = max(a.height, b.height) + 20
    out = Image.new("RGB", (a.width + b.width + 12, h), (60, 60, 60))
    out.paste(a, (0, 20))
    out.paste(b, (a.width + 12, 20))
    d = ImageDraw.Draw(out)
    d.text((4, 4), label_a, fill=(255, 255, 255))
    d.text((a.width + 16, 4), label_b, fill=(255, 255, 255))
    path = os.path.join(UI, f"side-{name}.png")
    out.save(path)
    return path


def props_sides(d):
    rects = json.load(open(os.path.join(d, "rects.json")))
    made = []
    for n in ["Part", "Model", "Script", "ScreenGui", "Lighting"]:
        pages = sorted(f for f in os.listdir(STUDIO) if f.startswith(f"props-{n}-"))
        ims = [Image.open(os.path.join(STUDIO, f)).convert("RGB") for f in pages]
        studio = Image.new("RGB", (sum(i.width for i in ims) + 6 * (len(ims) - 1), ims[0].height), (60, 60, 60))
        x = 0
        for i in ims:
            studio.paste(i, (x, 0))
            x += i.width + 6
        r = rects[f"props-{n}"]["props.panel"]
        ours = Image.open(os.path.join(d, f"props-{n}.png")).convert("RGB")
        ours = ours.crop((int(r[0]) - 8, int(r[1]) - 28, int(r[2]) + 4, int(r[3])))
        # Down to the last drawn row (the panel's empty rest is cut).
        px = ours.load()
        empty = px[ours.width // 2, ours.height - 1]
        bottom = ours.height
        while bottom > studio.height and all(px[x, bottom - 1] == empty for x in range(20, ours.width - 20, 7)):
            bottom -= 1
        many = "s" if len(ims) > 1 else ""
        made.append(side(studio, ours.crop((0, 0, ours.width, min(ours.height, bottom + 8))), f"props-{n}",
                         f"Studio ({len(ims)} page{many}, scrolled)", "rustudio"))
    return made


def output_side(d):
    rects = json.load(open(os.path.join(d, "rects.json")))
    r = rects["output-run-ts"]["output.panel"]
    ours = Image.open(os.path.join(d, "output-run-ts.png")).convert("RGB")
    ours = ours.crop((int(r[0]) - 4, int(r[1]) - 26, int(r[2]), int(r[3])))
    studio = Image.open(os.path.join(STUDIO, "output-run-ts.png")).convert("RGB").crop((0, 0, 613, 200))
    return [side(studio, ours.crop((0, 0, min(ours.width, 760), ours.height)), "output")]


def editor_sides(d):
    rects = json.load(open(os.path.join(d, "rects.json")))
    rs = lambda n: Image.open(os.path.join(d, f"{n}.png")).convert("RGB")
    st = lambda n: Image.open(os.path.join(STUDIO, f"{n}.png")).convert("RGB")
    made = []
    r = rects["edit-brickcolor"]["brick.popup"]
    made.append(side(st("brickcolor-popup"), rs("edit-brickcolor").crop(tuple(int(v) for v in r)), "brickcolor-popup"))
    r = rects["edit-color"]["color.dialog"]
    made.append(side(st("color-dialog"), rs("edit-color").crop(tuple(int(v) for v in r)), "color-dialog"))
    panel = rects["edit-slider"]["props.panel"]
    spin = rects["edit-slider"]["props.spin"]
    cy = (spin[1] + spin[3]) / 2
    ours = rs("edit-slider").crop((int(panel[0]) - 8, int(cy - 31), int(panel[2]) + 4, int(cy + 14)))
    made.append(side(st("props-slider"), ours, "props-slider", "Studio (Transparency row picked)", "rustudio (Transparency hovered)"))
    return made


def main():
    args = sys.argv[1:]
    extra = []
    for flag, fn in [("--props", props_sides), ("--output", output_side), ("--editors", editor_sides)]:
        if flag in args:
            i = args.index(flag)
            extra += fn(args[i + 1])
            del args[i:i + 2]
    shots = args[0]
    rects = json.load(open(os.path.join(shots, "rects.json")))
    rs = lambda n: Image.open(os.path.join(shots, f"{n}.png")).convert("RGB")
    st = lambda n: Image.open(os.path.join(STUDIO, f"{n}.png")).convert("RGB")
    made = []
    for tab in ["Home", "Model", "Avatar", "UI", "Script", "Plugins"]:
        made.append(side(st(f"ribbon-{tab}"), rs(f"ribbon-{tab}").crop((0, 0, 1280, 118)), f"ribbon-{tab}"))
    for menu in ["File", "Edit", "View", "Plugins", "Test", "Window", "Help"]:
        a = st(f"menu-{menu}")
        anchor = rects.get(f"menu-{menu}", {}).get("anchor", [0, 0, 40, 20])
        x0, y0 = int(anchor[0]) - 4, int(anchor[3])
        b = rs(f"menu-{menu}").crop((x0, y0, x0 + a.width + 90, y0 + a.height + 30))
        made.append(side(a, b, f"menu-{menu}"))
    a = st("explorer-hover")
    win = rs("explorer")
    search = rects.get("explorer", {}).get("explorer.search")
    x0 = int(search[0]) - 8 if search else win.width - a.width
    y0 = int(search[1]) - 40 if search else 115
    made.append(side(a, win.crop((x0, y0, x0 + a.width, y0 + a.height)), "explorer"))
    for n in ["test-dropdown", "account-menu"]:
        a = st(n)
        anchor = rects.get(n, {}).get("anchor", [0, 0, 100, 30])
        if n == "account-menu":
            x1 = int(anchor[2]) + 4
            b = rs(n).crop((x1 - a.width - 40, int(anchor[3]), x1, int(anchor[3]) + a.height + 60))
        else:
            b = rs(n).crop((int(anchor[0]) - 4, int(anchor[3]), int(anchor[0]) + a.width + 40, int(anchor[3]) + a.height + 20))
        made.append(side(a, b, n))
    made.append(side(st("window-output"), rs("explorer"), "window"))
    made += extra
    lines = ["# Studio | rustudio, side by side", "",
             "Left: an offline Studio (hidden, 1280x891, captured over a Studio MCP bridge). Right: rustudio "
             "(`--ui-shots`, same size, the control place). Made by `tools/ui_compare.py`; the checklist "
             "is app/UI-PARITY.md.", ""]
    for p in made:
        n = os.path.basename(p)
        lines += [f"## {n[5:-4]}", "", f"![{n}]({n})", ""]
    open(os.path.join(UI, "index.md"), "w").write("\n".join(lines))
    print("\n".join(made))


if __name__ == "__main__":
    main()
