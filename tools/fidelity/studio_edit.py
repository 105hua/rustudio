#!/usr/bin/env python3
"""Runs Luau files in a offline Studio's edit DataModel (plugin security, no
Play) on a copy of a place, and keeps the Output: for probes that need
edit-only services (SerializationService) or read a big place's voxels.

    studio_edit.py OUT_DIR PLACE SCRIPT.luau [PLACE SCRIPT.luau …]

Each pair writes OUT_DIR/<script name>.log. Run under the offline Studio lock
(flock ~/rustudio/queued/offline-studio.lock …).
"""
import os
import re
import shutil
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import fidelity  # noqa: E402


def run_one(mcp, place, script, out):
    os.makedirs(fidelity.RUN_TMP, exist_ok=True)
    copy = os.path.join(fidelity.RUN_TMP, f"edit-{os.getpid()}-{int(time.time())}{os.path.splitext(place)[1]}")
    shutil.copy(place, copy)
    text = mcp.call("new_instance", {"place": copy, "hidden": True, "timeout_ms": 240000})
    m = re.search(r"slot (\d+)", text)
    if not m:
        raise RuntimeError("new_instance: " + text[:300])
    a = {"instance": m.group(1)}
    try:
        for _ in range(120):
            fidelity.answer_modal(mcp, a)
            if "Returned" in mcp.call("run_luau", dict(a, code="return #game:GetChildren()")):
                break
            time.sleep(2)
        r = mcp.call("run_luau", dict(a, code=open(script).read(), timeout_ms=300000))
        time.sleep(2)
        log = mcp.call("logs", dict(a, lines=20000))
        name = os.path.splitext(os.path.basename(script))[0]
        open(os.path.join(out, name + ".log"), "w").write(log + "\n--- run_luau ---\n" + r)
        print(name, r[:400], flush=True)
    finally:
        try:
            mcp.call("close_instance", dict(a, discard=True))
        except Exception:
            pass
        os.remove(copy)


def main():
    out, pairs = sys.argv[1], sys.argv[2:]
    os.makedirs(out, exist_ok=True)
    mcp = fidelity.Mcp()
    host = fidelity.ensure_host(mcp)
    try:
        for i in range(0, len(pairs), 2):
            run_one(mcp, os.path.expanduser(pairs[i]), pairs[i + 1], out)
    finally:
        fidelity.stop_host(host)


if __name__ == "__main__":
    main()
