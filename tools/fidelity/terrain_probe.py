#!/usr/bin/env python3
"""Compares terrain_probe.luau's output (FIDTER chunks) from Studio and
rustudio: ray heightmaps per cell (mean |dh| where both hit, hit/miss
disagreement) and the voxels ReadVoxels gives back after odd Fill calls.

    terrain_probe.py [checks/compare/terrain-probe] [--dump CELL]
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import fidelity  # noqa: E402


def load(base, who):
    return fidelity.assemble(fidelity.parse_log(os.path.join(base, who, "output.log")), "FIDTER")


def heights(cell):
    return [None if h is False else h / 100 for h in cell["h"]]


def main():
    base = os.path.join(fidelity.CHECKS, "terrain-probe")
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if args:
        base = args[0]
    s, r = load(base, "studio"), load(base, "rustudio")
    if not s or not r:
        print("missing probe output:", "studio" if not s else "rustudio")
        return 1
    out = {"cells": {}, "voxels": {}}
    for name, sc in s["rays"].items():
        rc = r["rays"].get(name)
        hs, hr = heights(sc), heights(rc)
        both = [(a, b) for a, b in zip(hs, hr) if a is not None and b is not None]
        disagree = sum(1 for a, b in zip(hs, hr) if (a is None) != (b is None))
        row = {"rays": len(hs), "studio_hits": sum(a is not None for a in hs),
               "rustudio_hits": sum(b is not None for b in hr), "hit_disagree": disagree,
               "mean_abs_dh": round(sum(abs(a - b) for a, b in both) / len(both), 3) if both else None,
               "studio_max_h": max((a for a in hs if a is not None), default=None),
               "rustudio_max_h": max((b for b in hr if b is not None), default=None)}
        if sc.get("m"):
            row["material_disagree"] = sum(1 for a, b in zip(sc["m"], rc["m"]) if a != b)
        out["cells"][name] = row
    for name, sv in s["voxels"].items():
        rv = {tuple(v[:3]): v[3] for v in r["voxels"].get(name, [])}
        svd = {tuple(v[:3]): v[3] for v in sv}
        keys = set(svd) | set(rv)
        out["voxels"][name] = {"studio": len(svd), "rustudio": len(rv),
                               "mean_abs_docc": round(sum(abs(svd.get(k, 0) - rv.get(k, 0)) for k in keys) / max(len(keys), 1) / 1000, 3),
                               "studio_list": sorted(sv)[:40], "rustudio_list": sorted(r["voxels"].get(name, []))[:40]}
    json.dump(out, open(os.path.join(base, "probe.json"), "w"), indent=1)
    for n, c in out["cells"].items():
        print(n, {k: v for k, v in c.items()})
    for n, v in out["voxels"].items():
        print(n, {k: v[k] for k in ("studio", "rustudio", "mean_abs_docc")})
    return 0


if __name__ == "__main__":
    sys.exit(main())
