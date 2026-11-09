#!/usr/bin/env python3
"""Round 22's movement comparison: the FIDMOVE_<segment> traces that
movement_probe.luau prints in both engines, resampled on a 1/30 s grid from
each segment's start and compared frame by frame.

Per segment (weights in the score): root position error (0.4, 2 studs =
zero), velocity error (0.15, 8 studs/s = zero), facing error (0.15, 1 rad =
zero), the same Humanoid state (0.2) and FloorMaterial (0.1). The movement
score is 100 × the mean over segments both engines ran. It also lists each
engine's state changes with their times.

    movement_compare.py [checks/compare/<place>]   (prints the table)
"""
import json
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

GRID = 1 / 30
WEIGHTS = {"pos": 0.4, "vel": 0.15, "yaw": 0.15, "state": 0.2, "floor": 0.1}
SCALES = {"pos": 2.0, "vel": 8.0, "yaw": 1.0}


def traces(rows):
    """{segment: {"frames": [...], "events": [...]}} from parsed log rows."""
    import fidelity
    names = set()
    for _, text in rows:
        i = text.find("FIDMOVE_")
        if i >= 0:
            names.add(text[i + 8:].split("|", 1)[0])
    out = {}
    for n in names:
        cap = fidelity.assemble(rows, "FIDMOVE_" + n)
        if cap:
            out[n] = cap
    return out


def sample(frames, t):
    """Linear in position / velocity / facing, nearest for the names."""
    if t <= frames[0][0]:
        return frames[0]
    for a, b in zip(frames, frames[1:]):
        if a[0] <= t <= b[0]:
            k = (t - a[0]) / max(b[0] - a[0], 1e-6)
            near = a if k < 0.5 else b
            num = [a[i] + (b[i] - a[i]) * k for i in range(1, 8)]
            dy = (b[7] - a[7] + math.pi) % (2 * math.pi) - math.pi
            num[6] = a[7] + dy * k
            return [t] + num + [near[8], near[9]] + list(near[10:])
    return frames[-1]


def segment(s, r):
    fs, fr = s["frames"], r["frames"]
    if not fs or not fr:
        return None
    end = min(fs[-1][0], fr[-1][0])
    n = max(1, int(end / GRID))
    err = {"pos": 0.0, "vel": 0.0, "yaw": 0.0, "state": 0, "floor": 0}
    worst = (0.0, 0.0)
    for k in range(n + 1):
        t = k * GRID
        a, b = sample(fs, t), sample(fr, t)
        dp = math.dist(a[1:4], b[1:4])
        if dp > worst[1]:
            worst = (round(t, 2), round(dp, 2))
        err["pos"] += dp
        err["vel"] += math.dist(a[4:7], b[4:7])
        # Facing: the yaw where both stand up (|look.y| < 0.9), else the tilt.
        ly_a = a[12] if len(a) > 12 and isinstance(a[12], (int, float)) else 0.0
        ly_b = b[12] if len(b) > 12 and isinstance(b[12], (int, float)) else 0.0
        if abs(ly_a) < 0.9 and abs(ly_b) < 0.9:
            err["yaw"] += abs((a[7] - b[7] + math.pi) % (2 * math.pi) - math.pi)
        else:
            err["yaw"] += abs(math.asin(max(-1, min(1, ly_a))) - math.asin(max(-1, min(1, ly_b))))
        err["state"] += a[8] == b[8]
        err["floor"] += a[9] == b[9]
    m = n + 1
    means = {k: v / m for k, v in err.items()}
    parts = {
        "pos": max(0.0, 1 - means["pos"] / SCALES["pos"]),
        "vel": max(0.0, 1 - means["vel"] / SCALES["vel"]),
        "yaw": max(0.0, 1 - means["yaw"] / SCALES["yaw"]),
        "state": means["state"],
        "floor": means["floor"],
    }
    score = sum(WEIGHTS[k] * parts[k] for k in WEIGHTS)
    last_s, last_r = sample(fs, end), sample(fr, end)
    return {
        "score": round(100 * score, 1),
        "pos_err": round(means["pos"], 3),
        "vel_err": round(means["vel"], 3),
        "yaw_err": round(means["yaw"], 3),
        "state_same": round(means["state"], 3),
        "floor_same": round(means["floor"], 3),
        "worst_pos": worst,
        "end_studio": [round(x, 2) for x in last_s[1:4]],
        "end_rustudio": [round(x, 2) for x in last_r[1:4]],
        "states_studio": [[round(e[0], 2), e[1]] for e in s.get("events", [])],
        "states_rustudio": [[round(e[0], 2), e[1]] for e in r.get("events", [])],
    }


def compare(studio_rows, rustudio_rows):
    s, r = traces(studio_rows), traces(rustudio_rows)
    segs = {}
    for name in sorted(set(s) & set(r)):
        row = segment(s[name], r[name])
        if row:
            segs[name] = row
    score = round(sum(v["score"] for v in segs.values()) / len(segs), 1) if segs else None
    return {"score": score, "segments": segs, "only_studio": sorted(set(s) - set(r)),
            "only_rustudio": sorted(set(r) - set(s))}


def main():
    import fidelity
    base = sys.argv[1] if len(sys.argv) > 1 else os.path.join(fidelity.CHECKS, "movement-r15")
    m = compare(fidelity.parse_log(os.path.join(base, "studio", "output.log")),
                fidelity.parse_log(os.path.join(base, "rustudio", "output.log")))
    print(f"movement {m['score']}  (only Studio: {m['only_studio']}, only rustudio: {m['only_rustudio']})")
    for name, v in sorted(m["segments"].items(), key=lambda kv: kv[1]["score"]):
        print(f"{name:15s} {v['score']:5.1f}  pos {v['pos_err']:6.2f}  vel {v['vel_err']:6.2f}  yaw {v['yaw_err']:5.2f}  "
              f"state {v['state_same']:4.2f}  floor {v['floor_same']:4.2f}  end S {v['end_studio']} R {v['end_rustudio']}")


if __name__ == "__main__":
    main()
