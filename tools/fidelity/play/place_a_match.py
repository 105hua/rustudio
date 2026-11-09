#!/usr/bin/env python3
"""Place A played for real in rustudio: the lobby, START GAME, then 60 s of a match
(walking with W, firing with clicks), with the probes in this folder. Prints
the match and player reports and the Output's error lines, and exits non-zero
unless the match started, a enemy spawned, the player walked and fired,
shots killed enemies for points, and Output stayed free of errors. (The self-test's fidelity-place_a step runs a shorter
version: the self-test has 90 s in all.)

    python3 tools/fidelity/play/place_a_match.py [--out DIR] [--gaps FILE]
"""
import argparse
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
RUSTUDIO = os.environ.get("RUSTUDIO_BIN") or os.path.join(APP, "target", "release", "rustudio")
CORPUS = json.load(open(os.path.join(HERE, "..", "corpus.json")))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="/tmp/rustudio-place-a-match")
    ap.add_argument("--gaps", help="also write the API-gap telemetry here (RUSTUDIO_GAPS)")
    ap.add_argument("--place", help="play this instead of the corpus place (a place file or a project folder)")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    place = os.path.expanduser(a.place or CORPUS["place_a"]["path"])
    cmd = [RUSTUDIO, place, "--play", "--from-play", "--window", "1148x1072",
           "--shots", "10,125", "--click", "110,143@12",
           "--luau", "server", os.path.join(HERE, "place-a-server.luau") + "@14",
           "--luau", "client", os.path.join(HERE, "place-a-client.luau") + "@14",
           "--key", "W@30:2", "--key", "W@40:2", "--key", "W@50:2",
           "--key", "W@60:3", "--key", "W@75:2"]
    for t in range(44, 111):
        cmd += ["--click", f"431,337@{t}"]
    log = os.path.join(a.out, "output.log")
    cmd += ["--screenshot", os.path.join(a.out, "shot.png"), "--log", log]
    env = dict(os.environ, RUSTUDIO_NO_AUDIO="1")  # test runs are silent, like the hidden Studios
    if a.gaps:
        env["RUSTUDIO_GAPS"] = a.gaps
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=300, env=env)
    text = open(log, errors="replace").read() if os.path.exists(log) else ""
    errors = [l for l in text.splitlines() if l.startswith("Error:")]

    def report(tag):
        m = re.search(re.escape(tag) + r"(\{.*\})", text)
        return json.loads(m.group(1)) if m else {}

    match, player = report("MATCH|"), report("PLAYER|")
    # Round 13: a match must play as a game: shots kill enemies and score
    # (the probe aims at the nearest enemy in sight, as a player would).
    ok = (not errors and match.get("started") and match.get("peakAlive", 0) >= 1
          and player.get("moved", 0) >= 2 and player.get("clipAfter", 99) < player.get("clipBefore", 0)
          and match.get("kills", 0) >= 1 and match.get("points", 0) > 500)
    print(json.dumps({"match": match, "player": player, "error_lines": len(errors),
                      "first_errors": errors[:5], "passed": bool(ok)}, indent=1))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
