#!/usr/bin/env python3
"""Fidelity harness: real Roblox Studio is the oracle.

For each corpus place, the SAME moments are captured twice: once in real
Studio (offline Studio, through its MCP; never the person's own
Studio) and once in rustudio. Then the two are compared.

    fidelity.py run [--places place_a,demo,control] [--times 2,5,10,20,40]
                    [--skip-studio] [--skip-rustudio] [--round N]
    fidelity.py compare [--places …]          # only re-compare captures
    fidelity.py spread [--places …] [--runs 3] # run-to-run spread per area
    fidelity.py perf [--round N]               # the fps gate alone (also part of run)

A round fails on a gate, or on an area score below the last round that
scored it the same way by more than that place's measured run-to-run
spread (checks/compare/spread.json); drops inside it are listed as noise.
It also fails on the perf gate (round 20): Place B (Play and Edit), Place C
and Place A benched with `rustudio --bench` on /tmp copies at 1265x900 must hold
PERF_FLOOR_FPS and not fall more than PERF_MAX_DROP below the last round
that benched them.

Per place, under checks/compare/<place>/:
    studio/  shot-<t>s.png, output.log, capture.json, physics.json
    rustudio/ shot-<t>s.png (the 3D view), output.log, capture.json,
              physics.json, frames.json
    report.md, report.json, side-<t>s.png (Studio | rustudio), diff-<t>s.png
and checks/compare/score.json gains this round's scores per area.

Captures run tools/fidelity/capture.luau (structure, lighting, camera,
character, frame time) and physics.luau (resting height, walk speed, jump
apex, fall time) on the client in both engines: identical code, identical
moments counted from the start of Play.
"""
import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.dirname(os.path.dirname(HERE))
PROJECT = os.path.dirname(APP)
# A git worktree (app/.worktrees/NAME) shares the main checkout's
# checks/ (its Studio captures), while RUSTUDIO stays the worktree's build.
if os.path.basename(os.path.dirname(os.path.dirname(APP))) == ".worktrees":
    PROJECT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(APP))))
# RS_FIDELITY_CHECKS / RS_FIDELITY_TMP: a private checks folder and temp
# folder, so several runs (parallel agents) don't overwrite each other.
CHECKS = os.environ.get("RS_FIDELITY_CHECKS") or os.path.join(PROJECT, "checks", "compare")
# RUSTUDIO_BIN: score a kept binary (a round's own) while target/ rebuilds.
RUSTUDIO = os.environ.get("RUSTUDIO_BIN") or os.path.join(APP, "target", "release", "rustudio")
# Below this pixel MAE a near-empty rustudio view matches Studio's (round 23).
EMPTY_MAE_MIN = 8.0
RUN_TMP = os.environ.get("RS_FIDELITY_TMP") or os.path.join("/tmp", "rs-fidelity-run")  # /tmp is RAM: wiped at the start of every run
CORPUS = json.load(open(os.path.join(HERE, "corpus.json")))
ALIGN_AT = 0.5  # seconds after Play: align.luau (the character to the SpawnLocation's centre)
CAPTURE_AT = 10.0  # seconds after Play: capture.luau
PHYSICS_AT = 12.0  # physics.luau (takes ~8 s)
TOL = {"position": 0.05, "angle": 0.5, "color": 1, "transparency": 0.01}
PHYS_TOL = {"walkMeasured": 0.5, "jumpApex": 0.25, "fallTime30": 0.04, "restingHeight": 0.05,
            "hipHeight": 0.05, "walkSpeed": 0.0, "jumpPower": 0.0, "gravity": 0.01}


# --------------------------------------------------------------- an offline Studio MCP
class Mcp:
    """The offline Studio MCP (http://127.0.0.1:28100/mcp reaches every Studio)."""

    def __init__(self, port=28100):
        self.url = f"http://127.0.0.1:{port}/mcp"
        self.sid = None

    def _post(self, body):
        h = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
        if self.sid:
            h["Mcp-Session-Id"] = self.sid
        req = urllib.request.Request(self.url, json.dumps(body).encode(), h)
        with urllib.request.urlopen(req, timeout=900) as r:
            sid = r.headers.get("Mcp-Session-Id")
            return sid, r.read().decode(errors="replace")

    def up(self):
        try:
            self.sid, _ = self._post({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-03-26", "capabilities": {},
                "clientInfo": {"name": "rustudio-fidelity", "version": "1"}}})
            self._post({"jsonrpc": "2.0", "method": "notifications/initialized"})
            return True
        except Exception:
            return False

    def call(self, tool, args=None):
        if not self.sid and not self.up():
            raise RuntimeError("an offline Studio MCP is not reachable")
        _, out = self._post({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                             "params": {"name": tool, "arguments": args or {}}})
        texts = []
        for line in out.splitlines():
            line = line[5:].strip() if line.startswith("data:") else line
            if not line.startswith("{"):
                continue
            msg = json.loads(line)
            res = msg.get("result") or msg.get("error") or {}
            for c in res.get("content", []) if isinstance(res, dict) else []:
                if c.get("type") == "text":
                    texts.append(c["text"])
            if isinstance(res, dict) and "message" in res and not res.get("content"):
                texts.append("MCP error: " + res["message"])
        return "\n".join(texts)


def ensure_host(mcp):
    """A offline Studio to talk to: one already serving the MCP, else a hidden
    one this harness starts (and closes at the end)."""
    if mcp.up():
        return None
    env = dict(os.environ, OFFLINE_STUDIO_HIDDEN="1")
    proc = subprocess.Popen(["offline-studio"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            start_new_session=True)
    for _ in range(120):
        time.sleep(2)
        if mcp.up():
            return proc
    raise RuntimeError("couldn't start a hidden offline Studio")


def stop_host(host):
    """Stops an offline Studio this harness started (its own process group only):
    the MCP won't close a Studio the `offline-studio` command started."""
    if host is None:
        return
    import signal
    try:
        os.killpg(host.pid, signal.SIGTERM)
        host.wait(timeout=20)
    except Exception:
        pass
    # (gamescope ignores SIGTERM and kept hidden Studios alive)
    time.sleep(5)
    try:
        os.killpg(host.pid, signal.SIGKILL)
    except Exception:
        pass


def luau_wrapped(path, cfg=None):
    """capture/physics code run as a task (rustudio's harness chunks can't yield)."""
    pre = ""
    if cfg and cfg.get("camera_at") is not None:
        pre = f"local FID_CAMERA_AT = {float(cfg['camera_at'])}\n"
    return pre + "task.spawn(function()\n" + open(path).read() + "\nend)\n"


def place_events(cfg):
    """[(seconds after Play, script path)] for a corpus place: align,
    capture and physics unless it turns them off, plus its own scripts."""
    ev = []
    if cfg.get("align", True):
        ev.append((ALIGN_AT, os.path.join(HERE, "align.luau")))
    ev.append((cfg.get("capture_at", CAPTURE_AT), os.path.join(HERE, "capture.luau")))
    if cfg.get("physics", True):
        ev.append((PHYSICS_AT, os.path.join(HERE, "physics.luau")))
    for path, at in cfg.get("scripts", []):
        ev.append((float(at), os.path.join(HERE, path)))
    return sorted(ev)


def project_copy(name, cfg):
    """A corpus place kept as a project folder: copied to /tmp (never read
    in place by either engine, never written), with the place file it names
    put beside its project file. Returns (folder for rustudio, place file for
    Studio): Studio gets `argon build` of the copy when the project is all
    files, else rustudio's `--build` (project over its place file)."""
    src = os.path.expanduser(cfg["project"])
    dst = os.path.join(RUN_TMP, "projects", name)
    os.makedirs(dst, exist_ok=True)
    subprocess.run(["rsync", "-a", "--delete", "--exclude", ".git", src.rstrip("/") + "/", dst + "/"], check=True)
    proj = json.load(open(os.path.join(dst, "default.project.json")))
    if cfg.get("place_file"):
        shutil.copy(os.path.join(dst, cfg["place_file"]), os.path.join(dst, proj["name"] + ".rbxl"))
    built = os.path.join(RUN_TMP, f"{name}-built.rbxl")
    if cfg.get("studio_build") == "none":  # the perf bench: rustudio alone
        return dst, None
    if cfg.get("studio_build", "argon") == "argon":
        subprocess.run(["argon", "build", os.path.join(dst, "default.project.json"), "-o", built, "-y"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        subprocess.run([RUSTUDIO, "--build", built, dst], check=True, stdout=subprocess.DEVNULL)
    return dst, built


def answer_modal(mcp, a):
    """Answers a dialog that would block the run (never one that saves)."""
    info = mcp.call("instance_info", a)
    m = re.search(r'"modal_dialog":\s*"([^"]+)"', info)
    if not m:
        return None
    for button in ("Ignore", "Don't Restore", "No", "Cancel", "Close", "OK"):
        r = mcp.call("click_button", dict(a, text=button))
        if "clicked" in r.lower():
            return f"{m.group(1)} -> {button}"
    return m.group(1)


def studio_capture(mcp, place, out, times, cfg=None):
    os.makedirs(out, exist_ok=True)
    # A fresh copy each run: Studio offers to restore an autosave of a path
    # it saw before, and that dialog would stand in front of the game.
    tmp = RUN_TMP
    os.makedirs(tmp, exist_ok=True)
    ext = os.path.splitext(place)[1]
    copy = os.path.join(tmp, f"{os.path.splitext(os.path.basename(place))[0]}-{os.getpid()}-{int(time.time())}{ext}")
    shutil.copy(place, copy)
    place = copy
    text = mcp.call("new_instance", {"place": place, "hidden": True, "timeout_ms": 180000})
    m = re.search(r"slot (\d+)", text)
    if not m:
        raise RuntimeError("new_instance: " + text[:300])
    slot = int(m.group(1))
    a = {"instance": str(slot)}
    try:
        dialogs = []
        for _ in range(90):  # the place may still be loading
            d = answer_modal(mcp, a)
            if d:
                dialogs.append(d)
            r = mcp.call("run_luau", dict(a, code="return #game:GetChildren()"))
            if "Returned" in r:
                break
            time.sleep(2)
        # Play (F5) through Studio's own action; playtest (toolbar clicks)
        # only when the action doesn't start it.
        info = mcp.call("invoke_action", dict(a, name="simulationPlayAction"))
        t0 = time.time()
        # Play must really be running on a client before anything counts.
        playing = False
        for _ in range(30):
            r = mcp.call("run_luau", dict(a, code="return game:GetService('RunService'):IsClient() and game:GetService('RunService'):IsRunning()"))
            if "Returned: true" in r:
                playing = True
                break
            if _ == 10:
                answer_modal(mcp, a)
                info += " | " + mcp.call("playtest", dict(a, mode="play"))
                t0 = time.time()
            time.sleep(1)
        json.dump({"playtest": info, "playing": playing, "dialogs": dialogs, "place_copy": place},
                  open(os.path.join(out, "session.json"), "w"), indent=1)
        if not playing:
            raise RuntimeError("Studio's playtest did not start (see session.json)")
        events = sorted([(t, "shot") for t in times] + place_events(cfg or {}))
        for at, kind in events:
            wait = t0 + at - time.time()
            if wait > 0:
                time.sleep(wait)
            if kind == "shot":
                r = mcp.call("screenshot", dict(a, target="viewport", max_width=4096))
                m = re.search(r"saved to (\S+\.png)", r)
                if m:
                    shutil.copy(m.group(1), os.path.join(out, f"shot-{fmt(at)}s.png"))
                    if m.group(1).startswith("/tmp/"):
                        os.remove(m.group(1))
                else:
                    with open(os.path.join(out, "shot-errors.txt"), "a") as f:
                        f.write(f"{fmt(at)} s: {r[:500]}\n")
            else:
                code = luau_wrapped(kind, cfg)
                r = mcp.call("run_luau", dict(a, code=code, timeout_ms=60000))
                # (a script's error comes back here, not in the Output)
                with open(os.path.join(out, "luau-results.txt"), "a") as f:
                    f.write(f"{fmt(at)} s {os.path.basename(kind)}: {r[:2000]}\n")
        time.sleep(1)
        log = mcp.call("logs", dict(a, lines=20000))
        open(os.path.join(out, "output.log"), "w").write(log)
        mcp.call("invoke_action", dict(a, name="simulationResetAction"))
    finally:
        try:
            mcp.call("close_instance", dict(a, discard=True))
        except Exception:
            pass
        try:
            os.remove(place)  # this run's copy (/tmp is RAM)
        except OSError:
            pass
    return info


def studio_clock_base(studio_dir):
    """Studio's os.clock at its Play start (os.clock - time() at capture), or None."""
    cap = assemble(parse_log(os.path.join(studio_dir, "output.log")), "FIDCAP")
    if not cap or cap.get("clock") is None or cap.get("gameTime") is None:
        return None
    return float(cap["clock"]) - float(cap["gameTime"])


def rustudio_capture(place, out, times, clock_base=None, cfg=None):
    os.makedirs(out, exist_ok=True)
    scripts = []
    for at, src in place_events(cfg or {}):
        p = os.path.join(out, os.path.splitext(os.path.basename(src))[0] + ".task.luau")
        open(p, "w").write(luau_wrapped(src, cfg))
        scripts += ["--luau", "client", f"{p}@{fmt(at)}"]
    shot = os.path.join(out, "shot.png")
    cmd = [RUSTUDIO, place, "--play", "--from-play", "--window", "1148x1072",  # viewport 863x675, the same as Studio's hidden window
           "--shots", ",".join(fmt(t) for t in times), "--screenshot", shot,
           "--log", os.path.join(out, "output.log")] + scripts
    env = dict(os.environ, RUSTUDIO_NO_AUDIO="1")  # test runs are silent, like the hidden Studios
    # A place's own switches (terrain: material packs off, as the offline
    # Studio can't load them).
    env.update((cfg or {}).get("rustudio_env", {}))
    # Every API member scripts touch, and the ones rustudio lacks or stubs:
    # scripting_api's call-weighted coverage.
    gaps = os.path.join(out, "gaps.json")
    if os.path.exists(gaps):
        os.remove(gaps)
    env["RUSTUDIO_GAPS"] = gaps
    # Round 23: every image / mesh the place names, loaded or not (asset_audit.py).
    env["RUSTUDIO_ASSET_REPORT"] = os.path.join(out, "assets.json")
    if clock_base is not None:
        # Time-driven game code (Place A's menu camera sways on os.clock) runs in
        # the same phase as in Studio's capture.
        env["RUSTUDIO_CLOCK_BASE"] = f"{clock_base:.4f}"
    subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=max(times) + 240, env=env)
    for t in times:
        v = os.path.join(out, f"shot-{fmt(t)}s.view.png")
        if os.path.exists(v):
            shutil.move(v, os.path.join(out, f"shot-{fmt(t)}s.png"))
        w = os.path.join(out, f"shot-{fmt(t)}s.png")
        frames = os.path.join(out, f"shot-{fmt(t)}s.frames.json")
        if os.path.exists(frames):
            shutil.copy(frames, os.path.join(out, "frames.json"))
            os.remove(frames)
    return " ".join(os.path.basename(c) if c.startswith("/") else c for c in cmd)


def fmt(t):
    return f"{t:g}"


# ------------------------------------------------------------ parse captures
LINE = re.compile(r"^(?:(?P<lvl>Info|Warn|Error|info|warning|error|output|print)\s*:\s*)?(?P<text>.*)$")


def parse_log(path):
    """[(level, text)] from either engine's Output log."""
    rows = []
    if not os.path.exists(path):
        return rows
    for raw in open(path, errors="replace").read().splitlines():
        m = LINE.match(raw.strip())
        if not m or not m.group("text"):
            continue
        lvl = (m.group("lvl") or "info").lower()
        lvl = {"warn": "warning", "output": "info", "print": "info"}.get(lvl, lvl)
        text = m.group("text")
        text = re.sub(r"\s+—\s+\S+:\d+$", "", text)  # rustudio's "— Script:line"
        rows.append((lvl, text))
    return rows


SETTLED_S = 10.0  # shots from here on count for the pixel gate


def assemble(rows, tag):
    chunks, n = {}, 0
    for _, text in rows:
        m = re.search(tag + r"\|(\d+)\|(\d+)\|(.*)$", text)
        if m:
            chunks[int(m.group(1))] = m.group(3)
            n = int(m.group(2))
    if n and len(chunks) == n:
        try:
            cap = json.loads("".join(chunks[i] for i in range(1, n + 1)))
        except json.JSONDecodeError:
            return None
        uid = str(cap.get("userId") or "")
        if uid and uid != "0":
            seg = re.compile(r"(?<=\.)" + re.escape(uid) + r"(?=\.|$)")
            for p in cap.get("parts", []):
                p[0] = seg.sub("<UserId>", p[0])
        return cap
    return None


def physics_of(rows):
    for _, text in rows:
        m = re.search(r"FIDPHYS\|(\{.*\})", text)
        if m:
            try:
                return json.loads(m.group(1))
            except json.JSONDecodeError:
                pass
    return None


# Studio offline can't reach Roblox: these lines are the oracle's environment,
# not the place's behaviour, and are kept apart in the log diff.
OFFLINE = re.compile(r"Failed to load animation|failed due to: Request asset was not found|HTTP 5\d\d|"
                     r"isn't available offline|Request failed|HttpError|GetHumanoidDescription", re.I)


def norm(text):
    text = re.sub(r"\[(Client|Server)\]\s*", "", text)
    # Offline Studio uses Player; rustudio uses the signed-in player's name.
    # The same game error under PlayerScripts must compare equal in either
    # session (otherwise errors_as_studio rejects even the baseline build).
    text = re.sub(r"\bPlayers\.[^.\"\s]+(?=\.PlayerScripts\b)", "Players.<Player>", text)
    # rustudio puts the script and line in front ("ServerScriptService.X:12:
    # msg"); Studio prints the message alone (its "Script 'X', Line 12" is a
    # separate Info line).
    text = re.sub(r"^(?:[\w \-()]+\.)*[\w \-()]+:\d+:\s+", "", text.strip())
    text = re.sub(r"\d+(\.\d+)?", "#", text)
    return text.strip()[:200]


# ----------------------------------------------------------------- compare
def load_rgb(path):
    from PIL import Image
    return Image.open(path).convert("RGB")


def common(a, b, top_crop_a=0):
    """Both images on one aspect and size. Studio's 38 px device bar lies
    over the top of its 3D view (the view is the whole 675 rows), so those
    rows are cut from both images (round 21: until then only Studio's were,
    which compared its view shifted and zoomed 6% against rustudio's:
    demo 22.5 -> 13.5, Place A 32.4 -> 18.7 MAE once aligned)."""
    from PIL import Image
    if top_crop_a:
        a = a.crop((0, top_crop_a, a.width, a.height))
        cb = round(top_crop_a * b.height / (a.height + top_crop_a))
        b = b.crop((0, cb, b.width, b.height))
    ra, rb = a.width / a.height, b.width / b.height
    r = min(ra, rb)

    def crop(img):
        w = int(img.height * r)
        x = (img.width - w) // 2
        return img.crop((x, 0, x + w, img.height))

    a, b = crop(a), crop(b)
    size = (480, int(480 / r))
    return a.resize(size, Image.BILINEAR), b.resize(size, Image.BILINEAR)


def empty_view(img):
    """Empty or near-empty: almost no detail, or one colour almost everywhere."""
    import numpy as np
    a = np.asarray(img.convert("L"), dtype=np.float32)
    q = (np.asarray(img.convert("RGB")) // 16).reshape(-1, 3)
    _, counts = np.unique(q, axis=0, return_counts=True)
    top = counts.max() / q.shape[0]
    return bool(a.std() < 6.0 or top > 0.9), float(a.std()), float(top)


def pixel_diff(sa, rb, out_side, out_diff):
    import numpy as np
    from PIL import Image
    a, b = common(load_rgb(sa), load_rgb(rb), top_crop_a=38)
    x, y = np.asarray(a, dtype=np.int16), np.asarray(b, dtype=np.int16)
    d = np.abs(x - y)
    mae = float(d.mean())
    over = float((d.max(axis=2) > 32).mean() * 100)
    side = Image.new("RGB", (a.width * 2, a.height))
    side.paste(a, (0, 0))
    side.paste(b, (a.width, 0))
    side.save(out_side)
    heat = (np.clip(d.max(axis=2) * 3, 0, 255)).astype(np.uint8)
    Image.fromarray(heat, "L").save(out_diff)
    return mae, over


def coarse_mae(sa, rb, k=4):
    """MAE after a k× box downscale: shape, colour and light without the
    phase of texture detail (round 21's terrain score)."""
    import numpy as np
    a, b = common(load_rgb(sa), load_rgb(rb), top_crop_a=38)
    w, h = a.width // k, a.height // k
    x = np.asarray(a.resize((w, h), 4), dtype=np.float32)  # 4 = BOX
    y = np.asarray(b.resize((w, h), 4), dtype=np.float32)
    return float(np.abs(x - y).mean())


def state_diff(sc, rc, volatile=None):
    out = {"services": {}, "parts": {}, "lighting": {}, "camera": {}, "character": {}, "classes": {}}
    if not sc or not rc:
        out["missing"] = "capture missing on " + ("Studio" if not sc else "rustudio")
        return out
    s_srv, r_srv = set(sc["services"]), set(rc["services"])
    out["services"] = {"only_studio": sorted(s_srv - r_srv), "only_rustudio": sorted(r_srv - s_srv)}

    def keyed(parts):
        d, seen = {}, {}
        for p in parts:
            k = p[0]
            seen[k] = seen.get(k, 0) + 1
            d[f"{k}#{seen[k]}"] = p
        return d

    def pair_duplicates(sparts, rparts):
        # Same-path parts (Place A has many "P004016_0"s) come back in no fixed
        # order: pair each Studio one with the rustudio one that looks the
        # same (class, size, colour, material), the nearest among those.
        # (By position alone, a group posed differently, like the lobby
        # avatar's gun mid-animation, paired unrelated parts.)
        import math

        def look(a, b):
            return ((a[1] != b[1]) + (math.dist(a[3], b[3]) > TOL["position"])
                    + (max(abs(x - y) for x, y in zip(a[5], b[5])) > TOL["color"]) + (a[6] != b[6]))
        groups = {}
        for p in rparts:
            groups.setdefault(p[0], []).append(p)
        ordered = []
        for p in sparts:
            pool = groups.get(p[0])
            if not pool:
                continue
            best = min(range(len(pool)), key=lambda i: (look(pool[i], p), math.dist(pool[i][2], p[2])))
            ordered.append(pool.pop(best))
        for pool in groups.values():
            ordered.extend(pool)
        return ordered

    sp, rp = keyed(sc["parts"]), keyed(pair_duplicates(sc["parts"], rc["parts"]))
    matched = bad = 0
    issues = []
    for k, s in sp.items():
        r = rp.get(k)
        if not r:
            continue
        matched += 1
        why = []
        if s[1] != r[1]:
            why.append(f"class {s[1]} vs {r[1]}")
        if max(abs(a - b) for a, b in zip(s[2], r[2])) > TOL["position"]:
            why.append(f"position {s[2]} vs {r[2]}")
        if max(abs(a - b) for a, b in zip(s[3], r[3])) > TOL["position"]:
            why.append(f"size {s[3]} vs {r[3]}")
        if max(abs(((a - b + 180) % 360) - 180) for a, b in zip(s[4], r[4])) > TOL["angle"]:
            why.append(f"orientation {s[4]} vs {r[4]}")
        if max(abs(a - b) for a, b in zip(s[5], r[5])) > TOL["color"]:
            why.append(f"color {s[5]} vs {r[5]}")
        if s[6] != r[6]:
            why.append(f"material {s[6]} vs {r[6]}")
        if abs(s[7] - r[7]) > TOL["transparency"]:
            why.append(f"transparency {s[7]} vs {r[7]}")
        # A place's parts its own scripts change every frame (corpus
        # "volatile_parts": Place C's scenery rings), read at two moments.
        skip = (volatile or {}).get(k.rsplit("#", 1)[0], ())
        why = [w for w in why if w.split(" ")[0] not in skip]
        if why:
            bad += 1
            if len(issues) < 40:
                issues.append({"part": k.rsplit("#", 1)[0], "diff": why})
    out["parts"] = {"studio": len(sp), "rustudio": len(rp), "matched": matched, "differ": bad,
                    "only_studio": sorted(k.rsplit("#", 1)[0] for k in sp if k not in rp)[:40],
                    "only_rustudio": sorted(k.rsplit("#", 1)[0] for k in rp if k not in sp)[:40],
                    "issues": issues}
    li = {}
    for k, v in sc["lighting"].items():
        rv = rc["lighting"].get(k)
        if v == "?":
            # Studio wouldn't let the capture read it (plugin context): a
            # blocker, not a difference.
            out.setdefault("unreadable_in_studio", []).append(k)
            continue
        if isinstance(v, list):
            ok = rv is not None and max(abs(a - b) for a, b in zip(v, rv)) <= TOL["color"]
        elif isinstance(v, (int, float)) and not isinstance(v, bool):
            ok = rv is not None and abs(v - rv) <= 0.01
        else:
            ok = v == rv
        li[k] = {"studio": v, "rustudio": rv, "ok": ok}
    out["lighting"] = li
    scam, rcam = sc.get("camera") or {}, rc.get("camera") or {}
    if scam and rcam:
        import math
        # From the subject when both have one: the spawn spot is random.
        if scam.get("offset") and rcam.get("offset"):
            dp = math.dist(scam["offset"], rcam["offset"])
        else:
            dp = math.dist(scam["position"], rcam["position"])
        dot = sum(a * b for a, b in zip(scam["look"], rcam["look"]))
        ang = math.degrees(math.acos(max(-1.0, min(1.0, dot))))
        out["camera"] = {"position_studio": scam["position"], "position_rustudio": rcam["position"],
                         "offset_studio": scam.get("offset"), "offset_rustudio": rcam.get("offset"),
                         "distance": round(dp, 3), "look_angle_deg": round(ang, 2),
                         "fov": [scam["fov"], rcam["fov"]], "type": [scam["type"], rcam["type"]],
                         "subject": [scam["subject"], rcam["subject"]]}
    sch, rch = sc.get("character") or {}, rc.get("character") or {}
    sparts = {p[0]: p for p in sch.get("parts", [])}
    rparts = {p[0]: p for p in rch.get("parts", [])}
    same = [n for n in sparts if n in rparts and sparts[n][1] == rparts[n][1]
            and max(abs(a - b) for a, b in zip(sparts[n][2], rparts[n][2])) <= TOL["position"]
            and sparts[n][4] == rparts[n][4]]
    out["character"] = {"rig": [sch.get("rig"), rch.get("rig")], "parts_studio": len(sparts),
                        "parts_rustudio": len(rparts), "parts_same": len(same),
                        "tracks_studio": sch.get("tracks", []), "tracks_rustudio": rch.get("tracks", [])}
    sc_cls, rc_cls = sc.get("classes", {}), rc.get("classes", {})
    out["classes"] = {k: [sc_cls.get(k, 0), rc_cls.get(k, 0)] for k in sorted(set(sc_cls) | set(rc_cls))
                      if sc_cls.get(k, 0) != rc_cls.get(k, 0)}
    return out


def physics_diff(sp, rp):
    if not sp or not rp:
        return {"missing": "physics capture missing on " + ("Studio" if not sp else "rustudio")}
    if "skipped" in sp or "skipped" in rp:
        return {"skipped": [sp.get("skipped"), rp.get("skipped")]}
    rows = {}
    for k, tol in PHYS_TOL.items():
        a, b = sp.get(k), rp.get(k)
        ok = isinstance(a, (int, float)) and isinstance(b, (int, float)) and abs(a - b) <= tol + 1e-9
        rows[k] = {"studio": a, "rustudio": b, "tolerance": tol, "ok": ok}
    rows["rigType"] = {"studio": sp.get("rigType"), "rustudio": rp.get("rigType"),
                       "ok": sp.get("rigType") == rp.get("rigType")}
    return rows


def tagged(rows, tag):
    for _, t in rows:
        if t.startswith(tag):
            try:
                return json.loads(t[len(tag):].split("  —  ")[0])
            except ValueError:
                return None
    return None


def api_coverage(path):
    """Call-weighted API coverage from a RUSTUDIO_GAPS file: per member,
    its touches (property reads/writes, method lookups) against its gap
    counts (missing, stubbed, unimplemented, default-only, write-ignored,
    unwired events). score = 100 x (1 - gap calls / calls)."""
    import math
    if not os.path.exists(path):
        return None
    g = json.load(open(path))
    touch = {(t["class"], t["member"]): t["count"] for t in g.get("touches", [])}
    gap = {}
    for row in g.get("gaps", []):
        k = (row["class"], row["member"])
        gap[k] = gap.get(k, 0) + row["count"]
    calls = gapped = 0
    for k in set(touch) | set(gap):
        w = max(touch.get(k, 0), gap.get(k, 0))
        calls += w
        gapped += min(gap.get(k, 0), w)
    top = sorted(gap.items(), key=lambda kv: -kv[1])[:15]
    return {"calls": calls, "gap_calls": gapped, "members": len(set(touch) | set(gap)),
            "members_with_gaps": len(gap),
            # (floored: 100 only when no call hit a gap)
            "score": math.floor(1000.0 * (1 - gapped / calls)) / 10 if calls else None,
            "gap_share_pct": round(100.0 * gapped / calls, 4) if calls else None,
            "top_gaps": [{"member": f"{c}.{m}", "calls": n} for (c, m), n in top]}


def frac(ok, total):
    return round(100.0 * ok / total, 1) if total else None


def compare_place(name, times):
    base = os.path.join(CHECKS, name)
    sdir, rdir = os.path.join(base, "studio"), os.path.join(base, "rustudio")
    slog, rlog = parse_log(os.path.join(sdir, "output.log")), parse_log(os.path.join(rdir, "output.log"))
    sc, rc = assemble(slog, "FIDCAP"), assemble(rlog, "FIDCAP")
    studio_valid = sc is not None
    sp, rp = physics_of(slog), physics_of(rlog)
    rep = {"place": name, "file": CORPUS[name].get("path") or CORPUS[name]["project"], "times": times, "shots": [], "log": {}, "state": {},
           "physics": {}, "frame": {}, "gate": {}, "scores": {}}
    # Pixels.
    for t in times:
        sa, rb = os.path.join(sdir, f"shot-{fmt(t)}s.png"), os.path.join(rdir, f"shot-{fmt(t)}s.png")
        row = {"t": t}
        if os.path.exists(rb):
            row["rustudio_empty"], row["rustudio_std"], row["rustudio_top_colour"] = empty_view(load_rgb(rb))
            # A view Studio draws near-empty too (a dark cave wall) isn't a
            # broken render (round 21).
            if row["rustudio_empty"] and os.path.exists(sa) and empty_view(load_rgb(sa))[0]:
                row["rustudio_empty"] = False
                row["studio_empty_too"] = True
        if not studio_valid:
            row["missing"] = ["Studio capture invalid (no FIDCAP in its Output: the playtest didn't run)"]
        elif os.path.exists(sa) and os.path.exists(rb):
            row["mae"], row["over32_pct"] = pixel_diff(sa, rb, os.path.join(base, f"side-{fmt(t)}s.png"),
                                                       os.path.join(base, f"diff-{fmt(t)}s.png"))
            if CORPUS[name].get("terrain"):
                row["mae_coarse"] = coarse_mae(sa, rb)
                row["spot"] = CORPUS[name].get("spots", {}).get(fmt(t))
        else:
            row["missing"] = [p for p in (sa, rb) if not os.path.exists(p)]
        rep["shots"].append(row)
    # Log.
    def counts(rows):
        c = {"error": 0, "warning": 0, "info": 0}
        for lvl, _ in rows:
            c[lvl] = c.get(lvl, 0) + 1
        return c

    def place_lines(rows):
        return [(l, t) for l, t in rows if not re.match(r"(FIDCAP|FIDPHYS)\|", t) and "a Studio MCP bridge" not in t]

    sl, rl = place_lines(slog), place_lines(rlog)
    s_err = {norm(t) for l, t in sl if l == "error" and not OFFLINE.search(t)}
    s_off = {norm(t) for l, t in sl if l == "error" and OFFLINE.search(t)}
    r_err = {norm(t) for l, t in rl if l == "error"}
    s_warn, r_warn = {norm(t) for l, t in sl if l == "warning"}, {norm(t) for l, t in rl if l == "warning"}
    rep["log"] = {"studio": counts(sl), "rustudio": counts(rl),
                  "rustudio_error_lines": sum(1 for l, _ in rl if l == "error"),
                  "errors_only_rustudio": sorted(r_err - s_err)[:30],
                  "errors_only_studio": sorted(s_err - r_err)[:30],
                  "studio_offline_errors": sorted(s_off)[:15],
                  "warnings_only_rustudio": sorted(r_warn - s_warn)[:30],
                  "warnings_only_studio": sorted(s_warn - r_warn)[:30]}
    rep["state"] = state_diff(sc, rc, CORPUS[name].get("volatile_parts"))
    # The match moment each engine reached (corpus places with place_a_match.luau).
    rep["match"] = {"studio": tagged(slog, "FIDMATCH|"), "rustudio": tagged(rlog, "FIDMATCH|")}
    rep["physics"] = physics_diff(sp, rp)
    rf = os.path.join(rdir, "frames.json")
    rep["frame"] = {"rustudio": json.load(open(rf)) if os.path.exists(rf) else None,
                    "rustudio_capture_ms": (rc or {}).get("frame", {}).get("mean_ms"),
                    "studio_capture_ms": (sc or {}).get("frame", {}).get("mean_ms"),
                    "note": "Studio runs hidden under a headless gamescope, so its frame time is not a fair speed reference"}
    # Gate: what makes a round red.
    maes = [s["mae"] for s in rep["shots"] if "mae" in s]
    # Offline Studio is still loading at 2 s and 5 s (sky only, fading
    # menus); rustudio is there sooner. Load time isn't what the pixel gate
    # measures, so it judges the settled shots; the early ones are reported.
    settled = [s["mae"] for s in rep["shots"] if "mae" in s and s["t"] >= SETTLED_S]
    # Round 23: a near-empty view is a broken render only when it's also far
    # from Studio's; terrain's cave wall is dark in both (MAE 4.8) and only
    # lacks Studio's light-grid creases (reported as "flat_close").
    for s in rep["shots"]:
        if s.get("rustudio_empty") and s.get("mae", EMPTY_MAE_MIN) < EMPTY_MAE_MIN:
            s["rustudio_empty"], s["flat_close"] = False, True
    empties = [s["t"] for s in rep["shots"] if s.get("rustudio_empty")]
    thr = CORPUS[name].get("max_mae", 40.0)
    # Places whose Studio view isn't reproducible shot for shot (corpus
    # "pixel_gate": false, with its why) report their pixels and keep the
    # render score's drop check, but don't fail on one shot's MAE.
    pixel_gate = CORPUS[name].get("pixel_gate", True)
    # A place whose own scripts error in Studio too (Place B: DataStores in an
    # unpublished place, remotes its kit never made) fails only on errors
    # Studio doesn't show; every other place on any error line.
    gate_errors = (len(rep["log"]["errors_only_rustudio"]) if CORPUS[name].get("errors_as_studio")
                   else rep["log"]["rustudio_error_lines"])
    # Round 23: images / meshes rustudio stopped showing (asset_audit.py).
    import asset_audit
    aud = asset_audit.audit(base, slog)
    if aud:
        rep["assets"] = aud
        open(os.path.join(base, "assets.md"), "w").write(asset_audit.markdown(aud))
        gate_errors += len(aud["regressions"])
    rep["gate"] = {"studio_valid": studio_valid,
                   "error_lines": rep["log"]["rustudio_error_lines"], "gate_errors": gate_errors, "empty_views_at": empties,
                   "asset_regressions": len(aud["regressions"]) if aud else None,
                   "worst_mae": max(settled) if settled else None, "settled_from_s": SETTLED_S,
                   "mae_threshold": thr if pixel_gate else None,
                   "passed": studio_valid and gate_errors == 0 and not empties
                   and bool(settled) and (not pixel_gate or max(settled) <= thr)}
    # Scores per area (0..100; None = not measurable for this place).
    st, ph = rep["state"], rep["physics"]
    sc_ = {}
    sc_["render"] = round(max(0.0, 100 - (sum(settled) / len(settled)) * 100 / 64), 1) if settled else None
    li = st.get("lighting") or {}
    sc_["lighting"] = frac(sum(1 for v in li.values() if v["ok"]), len(li)) if li else None
    parts = st.get("parts") or {}
    mat_total = parts.get("matched", 0)
    sc_["materials"] = frac(mat_total - len([i for i in parts.get("issues", [])
                                              if any(d.startswith(("material", "color", "transparency")) for d in i["diff"])]),
                            mat_total) if mat_total else None
    prow = [v for k, v in ph.items() if isinstance(v, dict) and "ok" in v]
    sc_["physics"] = frac(sum(1 for v in prow if v["ok"]), len(prow)) if prow else None
    # Round 15: the share of the place's API calls rustudio really serves,
    # weighted by how often its scripts make them (RUSTUDIO_GAPS: every
    # member touched, and the gaps among them). It used to be 100 minus
    # Output errors, which read 100 with ~10% of the API there.
    rep["api"] = api_coverage(os.path.join(rdir, "gaps.json"))
    sc_["scripting_api"] = rep["api"]["score"] if rep["api"] else None
    cam = st.get("camera") or {}
    if cam:
        # A game that points the view itself (Place A's match: the aim follows the
        # nearest enemy) is judged on where the camera sits, its type and
        # its subject's class instead of the look direction.
        if CORPUS[name].get("look_follows_game"):
            look_ok = cam["subject"][0].rsplit(".", 1)[-1] == cam["subject"][1].rsplit(".", 1)[-1]
        else:
            look_ok = cam["look_angle_deg"] <= 2
        c_ok = (cam["distance"] <= 0.5) + look_ok + (abs(cam["fov"][0] - cam["fov"][1]) < 0.01) \
            + (cam["type"][0] == cam["type"][1])
        sc_["camera_controls"] = frac(c_ok, 4)
    else:
        sc_["camera_controls"] = None
    ch = st.get("character") or {}
    sc_["avatar"] = frac(ch.get("parts_same", 0), ch.get("parts_studio", 0)) if ch.get("parts_studio") else None
    # Slot when captured (Animate picks a slot's variant at random), else id.
    ts = {t[3] if len(t) > 3 else t[0] for t in ch.get("tracks_studio", [])}
    tr = {t[3] if len(t) > 3 else t[0] for t in ch.get("tracks_rustudio", [])}
    sc_["animation"] = frac(len(ts & tr), len(ts | tr)) if (ts or tr) else None
    # Round 21: terrain, for places that are terrain views (corpus
    # "terrain": true): every shot (no settling), half full-size MAE and
    # half MAE after a 4x downscale, on render's scale (64 = 0).
    if CORPUS[name].get("terrain"):
        rows = [s for s in rep["shots"] if "mae" in s]
        sc_["terrain"] = round(max(0.0, 100 - sum((s["mae"] + s["mae_coarse"]) / 2 for s in rows)
                                   / len(rows) * 100 / 64), 1) if rows else None
    else:
        sc_["terrain"] = None
    # Round 22: movement, for places running movement_probe.luau (corpus
    # "movement": true): the per-frame traces compared by movement_compare.
    if CORPUS[name].get("movement"):
        import movement_compare
        rep["movement"] = movement_compare.compare(slog, rlog)
        sc_["movement"] = rep["movement"]["score"]
    else:
        sc_["movement"] = None
    rep["scores"] = sc_
    json.dump(rep, open(os.path.join(base, "report.json"), "w"), indent=1)
    open(os.path.join(base, "report.md"), "w").write(report_md(rep))
    return rep


def report_md(rep):
    L = [f"# Fidelity: {rep['place']}", "", f"Place: `{rep['file']}`. Studio (offline Studio, offline) is the reference.", ""]
    g = rep["gate"]
    L += [f"**Gate: {'PASS' if g['passed'] else 'FAIL'}**: rustudio error lines {g['error_lines']}, "
          f"empty views at {g['empty_views_at'] or 'none'}, worst pixel MAE {g['worst_mae']} (threshold {g['mae_threshold']}).", ""]
    L += ["## Scores", "", "| Area | Score |", "|---|---|"]
    for k, v in rep["scores"].items():
        L.append(f"| {k} | {'not measurable here' if v is None else v} |")
    if rep.get("assets"):
        a = rep["assets"]
        L += ["", "## Assets", "",
              "rustudio: " + "; ".join(f"{k}: " + ", ".join(f"{n} {s}" for s, n in sorted(v.items()))
                                       for k, v in sorted(a["summary"].items())),
              f"Asset regressions (rustudio fails, Studio doesn't, not an access refusal): {len(a['regressions'])}. "
              "Every id with its status: assets.md."]
    if rep.get("movement"):
        mv = rep["movement"]
        L += ["", "## Movement (movement_probe.luau traces)", "",
              "| Segment | Score | pos err | vel err | yaw err | same state | same floor | end Studio | end rustudio |",
              "|---|---|---|---|---|---|---|---|---|"]
        for n, v in sorted(mv["segments"].items(), key=lambda kv: kv[1]["score"]):
            L.append(f"| {n} | {v['score']} | {v['pos_err']} | {v['vel_err']} | {v['yaw_err']} | {v['state_same']} | "
                     f"{v['floor_same']} | {v['end_studio']} | {v['end_rustudio']} |")
    L += ["", "## Screenshots (Studio | rustudio)", "", "| t (s) | MAE | % px > 32 | rustudio empty |", "|---|---|---|---|"]
    for s in rep["shots"]:
        L.append(f"| {s['t']} | {round(s['mae'], 2) if 'mae' in s else '-'} | {round(s['over32_pct'], 1) if 'over32_pct' in s else '-'} | {s.get('rustudio_empty', '-')} |")
        if "mae" in s:
            L.append(f"\n![t={s['t']}](side-{fmt(s['t'])}s.png)\n")
    lg = rep["log"]
    L += ["", "## Output", "", f"Studio {lg['studio']}, rustudio {lg['rustudio']}.", ""]
    for k in ("errors_only_rustudio", "errors_only_studio", "studio_offline_errors", "warnings_only_rustudio", "warnings_only_studio"):
        if lg[k]:
            L.append(f"**{k.replace('_', ' ')}**:")
            L += [f"- `{x}`" for x in lg[k][:15]]
            L.append("")
    st = rep["state"]
    L += ["## State", ""]
    if "missing" in st:
        L.append(st["missing"])
    else:
        p = st["parts"]
        L.append(f"Services only in Studio: {st['services']['only_studio']}; only in rustudio: {st['services']['only_rustudio']}.")
        L.append(f"Parts: Studio {p['studio']}, rustudio {p['rustudio']}, matched {p['matched']}, differing {p['differ']}.")
        for i in p["issues"][:15]:
            L.append(f"- {i['part']}: {'; '.join(i['diff'])}")
        bad = {k: v for k, v in st["lighting"].items() if not v["ok"]}
        L.append(f"Lighting differences: {bad if bad else 'none'}.")
        if st["camera"]:
            L.append(f"Camera: {st['camera']}.")
        L.append(f"Character: {st['character']}.")
    L += ["", "## Physics facts", ""]
    ph = rep["physics"]
    if "missing" in ph or "skipped" in ph:
        L.append(str(ph))
    else:
        L += ["| Fact | Studio | rustudio | Tolerance | OK |", "|---|---|---|---|---|"]
        for k, v in ph.items():
            L.append(f"| {k} | {v.get('studio')} | {v.get('rustudio')} | {v.get('tolerance', '=')} | {v['ok']} |")
    api = rep.get("api")
    L += ["", "## Scripting API (call-weighted)", ""]
    if api:
        L.append(f"{api['calls']} API calls by the place's scripts over {api['members']} members; "
                 f"{api['gap_calls']} of them hit a gap ({api['members_with_gaps']} members): score {api['score']}.")
        L += [f"- `{g['member']}`: {g['calls']}" for g in api["top_gaps"]]
    else:
        L.append("No gaps.json from the rustudio run.")
    if rep.get("match", {}).get("studio") or rep.get("match", {}).get("rustudio"):
        L += ["", "## Match", "", f"Studio {rep['match']['studio']}; rustudio {rep['match']['rustudio']}."]
    L += ["", "## Frame time", "", str(rep["frame"]), ""]
    return "\n".join(L)


# How each area is scored; a round only compares an area with an earlier
# round scored the same way. (Round 15: scripting_api became call-weighted
# API coverage; before, 100 minus Output errors.)
# render 2 (round 21): the device bar cut from both views (see common()).
METHODS = {"scripting_api": 2, "render": 2}


SPREAD = os.path.join(CHECKS, "spread.json")
# Scores are rounded to 0.1: a drop has to be past the spread by more than
# that rounding to count.
SPREAD_ROUNDING = 0.05


def load_spread():
    return json.load(open(SPREAD)) if os.path.exists(SPREAD) else {"places": {}}


def tolerance(spread, place, area):
    """How far a place's area score moves run to run on one build (max −
    min over `fidelity.py spread`), plus the rounding; 0 when not measured."""
    s = spread.get("places", {}).get(place, {}).get(area)
    return (s["spread"] if s else 0.0) + SPREAD_ROUNDING


def scorecard(reps, round_no):
    """Adds this round to score.json and returns (entry, drops): every
    place/area whose score fell against the latest earlier round that
    measured it the same way by more than that place's run-to-run spread
    (checks/compare/spread.json; round 19). Any such drop fails the round;
    smaller ones are listed as `within_noise`."""
    path = os.path.join(CHECKS, "score.json")
    card = json.load(open(path)) if os.path.exists(path) else {"areas": [], "rounds": []}
    spread = load_spread()
    areas = ["render", "lighting", "materials", "physics", "scripting_api", "camera_controls", "animation", "avatar",
             "terrain", "movement"]
    card["areas"] = areas
    # Places run separately in one round add up into its entry.
    prev = next((r for r in card["rounds"] if r["round"] == round_no), None)
    places = dict(prev["places"]) if prev else {}
    places.update({r["place"]: r["scores"] for r in reps})
    gates = dict(prev.get("gates", {})) if prev else {}
    gates.update({r["place"]: r["gate"]["passed"] for r in reps})
    api = dict(prev.get("api_calls", {})) if prev else {}
    api.update({r["place"]: [r["api"]["calls"], r["api"]["gap_calls"]] for r in reps if r.get("api")})
    overall = {}
    for a in areas:
        vals = [p[a] for p in places.values() if p.get(a) is not None]
        overall[a] = round(sum(vals) / len(vals), 1) if vals else None
    # scripting_api across the corpus: weighted by every place's calls.
    calls = sum(c for c, _ in api.values())
    if calls:
        import math
        overall["scripting_api"] = math.floor(1000.0 * (1 - sum(g for _, g in api.values()) / calls)) / 10
    methods = {a: METHODS.get(a, 1) for a in areas}
    drops, noise = [], []
    for name, sc in places.items():
        for a in areas:
            now = sc.get(a)
            if now is None:
                continue
            for r in sorted((r for r in card["rounds"] if r["round"] < round_no or not round_no), key=lambda r: -r["round"]):
                was = r["places"].get(name, {}).get(a)
                if was is None:
                    continue
                if r.get("methods", {}).get(a, 1) == methods[a] and now < was:
                    tol = round(tolerance(spread, name, a), 2)
                    d = {"place": name, "area": a, "was": was, "now": now, "since_round": r["round"], "tolerance": tol}
                    (drops if now < was - tol else noise).append(d)
                break
    entry = {"round": round_no, "at": datetime.datetime.now().isoformat(timespec="seconds"),
             "places": places, "overall": overall, "gates": gates, "methods": methods,
             "api_calls": api, "drops": drops, "within_noise": noise,
             "spread_measured": spread.get("at"),
             "passed": all(gates.values()) and not drops and not (prev or {}).get("perf_fails")}
    # The perf gate's results stay with the round (perf_gate writes them).
    for k in ("perf", "perf_floor_fps", "perf_fails"):
        if prev and k in prev:
            entry[k] = prev[k]
    # Round 0 (the default) is a try: judged the same way, never recorded.
    if round_no:
        card["rounds"] = [r for r in card["rounds"] if r["round"] != round_no] + [entry]
        json.dump(card, open(path, "w"), indent=1)
    return entry, drops


# ------------------------------------------------------------------ perf gate
# (place, mode): what `rustudio --bench` measures for the gate. The copies
# are the harness's own (project folders with their place file beside).
PERF = [("place_b", "play"), ("place_b", "edit"), ("place_c", "play"), ("place_a", "play")]
PERF_FLOOR_FPS = 60.0  # the person's target (round 20), Play and Edit
PERF_MAX_DROP = 0.25  # a later round may not lose more than this share of a place's fps
PERF_SECONDS = 15
PERF_TRIES = 2  # best of: other programs on the machine cost a run a few fps


def perf_bench(n, mode):
    """Benches one corpus place: {"fps", "low1_fps", "frame_ms", "top"} of the
    best of PERF_TRIES runs, or None when no run finished."""
    cfg = CORPUS[n]
    if cfg.get("project"):
        place, _ = project_copy(n, dict(cfg, studio_build="none"))
    else:
        place = os.path.expanduser(cfg["path"])
        if not os.path.isabs(place):
            place = os.path.join(HERE, place)
    out = os.path.join(RUN_TMP, f"perf-{n}-{mode}.json")
    best = None
    for _ in range(PERF_TRIES):
        if os.path.exists(out):
            os.remove(out)
        cmd = [RUSTUDIO, place, "--bench", out, "--seconds", str(PERF_SECONDS)] + (["--play"] if mode == "play" else [])
        subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=600,
                       env=dict(os.environ, RUSTUDIO_NO_AUDIO="1"))
        if not os.path.exists(out):
            continue
        r = json.load(open(out))
        if best is None or r["avg_fps"] > best["fps"]:
            best = {"fps": round(r["avg_fps"], 1), "low1_fps": round(r["low1_fps"], 1),
                    "frame_ms": round(r["avg_frame_ms"], 2),
                    "top": [f"{s['name']} {s['ms_per_frame']:.2f}" for s in r.get("sections", [])[:6]]}
    return best


def perf_gate(round_no, results):
    """Puts the bench results in score.json's round entry ("perf") and judges
    them: under PERF_FLOOR_FPS, or more than PERF_MAX_DROP below the latest
    earlier round that benched the same thing, fails. Returns the failures."""
    path = os.path.join(CHECKS, "score.json")
    card = json.load(open(path)) if os.path.exists(path) else {"areas": [], "rounds": []}
    fails = []
    for key, r in results.items():
        if r is None:
            fails.append({"what": key, "why": "the bench did not finish"})
            continue
        if r["fps"] < PERF_FLOOR_FPS:
            fails.append({"what": key, "why": f"{r['fps']} fps < {PERF_FLOOR_FPS}"})
        for old in sorted((x for x in card["rounds"] if x["round"] < round_no or not round_no),
                          key=lambda x: -x["round"]):
            was = old.get("perf", {}).get(key)
            if was:
                if r["fps"] < was["fps"] * (1 - PERF_MAX_DROP):
                    fails.append({"what": key, "why": f"{r['fps']} fps, round {old['round']} had {was['fps']}"})
                break
    if round_no:
        entry = next((x for x in card["rounds"] if x["round"] == round_no), None)
        if entry is None:
            entry = {"round": round_no, "at": datetime.datetime.now().isoformat(timespec="seconds"),
                     "places": {}, "gates": {}, "passed": True}
            card["rounds"].append(entry)
        entry["perf"] = dict(entry.get("perf", {}), **results)
        entry["perf_floor_fps"] = PERF_FLOOR_FPS
        entry["perf_fails"] = fails
        entry["passed"] = bool(entry.get("passed", True)) and not fails and not entry.get("drops")
        json.dump(card, open(path, "w"), indent=1)
    return fails


def run_perf(round_no):
    results = {}
    for n, mode in PERF:
        print(f"[perf] {n} {mode} …", flush=True)
        r = perf_bench(n, mode)
        results[f"{n}/{mode}"] = r
        print(f"[perf] {n} {mode}: " + (f"{r['fps']} fps (1% low {r['low1_fps']})" if r else "no result"), flush=True)
    fails = perf_gate(round_no, results)
    for f in fails:
        print(f"PERF FAIL {f['what']}: {f['why']}", flush=True)
    return not fails


def run_place(n, a, state):
    """Captures one corpus place in Studio (unless skipped) and rustudio,
    then compares them. `state` holds the offline Studio MCP and its host."""
    cfg = CORPUS[n]
    times = [float(t) for t in (a.times.split(",") if a.times else cfg.get("times", [2, 5, 10, 20, 40]))]
    base = os.path.join(CHECKS, n)
    if a.cmd in ("run", "spread"):
        if cfg.get("project"):
            rplace, splace = project_copy(n, cfg)
        else:
            rplace = splace = os.path.expanduser(cfg["path"])
            if not os.path.isabs(rplace):
                rplace = splace = os.path.join(HERE, rplace)
        # Studio first: rustudio's os.clock is then put in phase with it.
        if not a.skip_studio:
            print(f"[{n}] Studio …", flush=True)
            try:
                studio_capture(state["mcp"], splace, os.path.join(base, "studio"), times, cfg)
            except (urllib.error.URLError, ConnectionError, OSError) as e:
                # The MCP's Studio went away: a fresh hidden one, once.
                print(f"[{n}] an offline Studio MCP lost ({e}); restarting it", flush=True)
                stop_host(state["host"])
                state["mcp"] = Mcp()
                state["host"] = ensure_host(state["mcp"])
                studio_capture(state["mcp"], splace, os.path.join(base, "studio"), times, cfg)
        if not a.skip_rustudio:
            print(f"[{n}] rustudio …", flush=True)
            rustudio_capture(rplace, os.path.join(base, "rustudio"), times,
                             studio_clock_base(os.path.join(base, "studio")), cfg)
    rep = compare_place(n, times)
    print(f"[{n}] gate {'PASS' if rep['gate']['passed'] else 'FAIL'} {rep['gate']} scores {rep['scores']}", flush=True)
    return rep


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["run", "compare", "spread", "perf"],
                    help="spread: play each place --runs times on this build and write each area's "
                         "run-to-run spread to checks/compare/spread.json (the round gate's noise allowance)")
    # Calibration scenes (bloom) run only when named.
    ap.add_argument("--places", default=",".join(n for n, c in CORPUS.items() if not c.get("calibration")))
    ap.add_argument("--times", help="seconds after Play to shoot (default: the place's corpus times, else 2,5,10,20,40)")
    ap.add_argument("--skip-studio", action="store_true")
    ap.add_argument("--skip-rustudio", action="store_true")
    ap.add_argument("--round", type=int, default=0)
    ap.add_argument("--runs", type=int, default=3, help="spread: plays per place")
    ap.add_argument("--skip-perf", action="store_true", help="run: leave out the fps gate")
    a = ap.parse_args()
    names = [n for n in a.places.split(",") if n]
    if a.cmd in ("run", "spread", "perf"):
        # Old copies, built places and Studio copies of earlier runs.
        shutil.rmtree(RUN_TMP, ignore_errors=True)
    if a.cmd == "perf":
        ok = run_perf(a.round)
        print("PERF " + ("PASS" if ok else "FAIL"), flush=True)
        sys.exit(0 if ok else 1)
    state = {"mcp": None, "host": None}
    reps = []
    rc = 1
    try:
        if a.cmd in ("run", "spread") and not a.skip_studio:
            state["mcp"] = Mcp()
            state["host"] = ensure_host(state["mcp"])
        if a.cmd == "spread":
            out = load_spread()
            for n in names:
                runs = []
                for i in range(a.runs):
                    print(f"[{n}] spread run {i + 1} / {a.runs}", flush=True)
                    runs.append(run_place(n, a, state)["scores"])
                areas = {}
                for k in runs[0]:
                    vals = [r[k] for r in runs if r.get(k) is not None]
                    if vals:
                        areas[k] = {"scores": vals, "spread": round(max(vals) - min(vals), 2)}
                out["places"][n] = areas
                out["at"] = datetime.datetime.now().isoformat(timespec="seconds")
                out["runs"] = a.runs
                out["about"] = ("each place played --runs times on one build (Studio and rustudio both re-captured); "
                                "spread = max − min per area; fidelity.py fails a round only on drops past it")
                json.dump(out, open(SPREAD, "w"), indent=1)
                print(f"[{n}] spread {{{', '.join(f'{k}: {v['spread']}' for k, v in areas.items())}}}", flush=True)
            rc = 0
            return
        for n in names:
            reps.append(run_place(n, a, state))
        stop_host(state["host"])
        state["host"] = None
        entry, drops = scorecard(reps, a.round)
        if a.cmd == "run" and not a.skip_perf:
            # After the captures, with Studio closed: the bench wants the machine.
            perf_ok = run_perf(a.round)
            entry["passed"] = entry["passed"] and perf_ok
        print(json.dumps(entry["overall"]))
        for d in entry["within_noise"]:
            print(f"within noise {d['place']} {d['area']}: {d['was']} (round {d['since_round']}) -> {d['now']} "
                  f"(spread {d['tolerance']})", flush=True)
        for d in drops:
            print(f"DROP {d['place']} {d['area']}: {d['was']} (round {d['since_round']}) -> {d['now']} "
                  f"(beyond its spread {d['tolerance']})", flush=True)
        print("ROUND " + ("PASS" if entry["passed"] else "FAIL"), flush=True)
        rc = 0 if entry["passed"] else 1
    finally:
        stop_host(state["host"])
    sys.exit(rc)


if __name__ == "__main__":
    main()
