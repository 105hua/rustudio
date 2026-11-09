#!/usr/bin/env python3
"""Round 23's asset audit for a place: rustudio's asset report
(RUSTUDIO_ASSET_REPORT, written at each screenshot as rustudio/assets.json)
joined with the ids Studio's Output says it couldn't load.

Every image and mesh id the place's properties name, with rustudio's status
(loaded / failed + why / downloading / not requested) and whether Studio
failed it too. `regressions` are ids rustudio failed for its own reasons
(not decoded, or a fetch error other than Roblox refusing the download to
this session) that Studio didn't report failing: a texture that stopped
loading. The gate fails on any.

    asset_audit.py checks/compare/<place>     (prints the summary and table)
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

# Download refusals no engine gets past without the right signed-in account.
ACCESS = ("Authentication required", "not authorized", "not approved", "sign in")
ID = re.compile(r"(?:rbxassetid://|[?&]id=)(\d+)")


def studio_failed(rows):
    out = set()
    for _, text in rows:
        if "fail" in text.lower() or "could not" in text.lower():
            out.update(ID.findall(text))
            for m in re.findall(r"rbxasset://\S+", text):
                out.add(m.rstrip(":"))
    return out


def asset_num(cid):
    m = ID.search(cid)
    if m:
        return m.group(1)
    if cid.isdigit():
        return cid
    return cid if cid.startswith("rbxasset://") else None


def audit(base, studio_rows=None):
    import fidelity
    path = os.path.join(base, "rustudio", "assets.json")
    if not os.path.exists(path):
        return None
    rep = json.load(open(path))
    if studio_rows is None:
        studio_rows = fidelity.parse_log(os.path.join(base, "studio", "output.log"))
    sfail = studio_failed(studio_rows)
    rows = []
    regressions = []
    for x in rep.get("ids", []):
        num = asset_num(x["id"])
        x["studio_failed"] = bool(num and num in sfail)
        why = x.get("why") or ""
        own = x["status"] == "failed" and not any(a in why for a in ACCESS) and "No such file" not in why
        if own and not x["studio_failed"]:
            regressions.append(x["id"])
        rows.append(x)
    return {"summary": rep.get("summary", {}), "ids": rows, "regressions": regressions,
            "studio_failed_count": len(sfail)}


def markdown(a):
    L = ["## Assets (images and meshes the place names)", "",
         "rustudio: " + "; ".join(f"{k} " + ", ".join(f"{n} {s}" for s, n in sorted(v.items()))
                                  for k, v in sorted(a["summary"].items())),
         f"Regressions (rustudio fails, Studio doesn't, not an access refusal): {len(a['regressions'])}", "",
         "| id | kind | rustudio | Studio's log | used by | example | why |", "|---|---|---|---|---|---|---|"]
    order = {"failed": 0, "not requested": 1, "downloading": 2, "loaded": 3}
    for x in sorted(a["ids"], key=lambda x: (order.get(x["status"], 9), x["kind"], x["id"])):
        users = ", ".join(f"{u} ×{n}" for u, n in x["users"].items())
        L.append(f"| {x['id']} | {x['kind']} | {x['status']} | {'failed' if x['studio_failed'] else '—'} | {users} | "
                 f"{x['example'][:60]} | {(x.get('why') or '')[:90]} |")
    return "\n".join(L) + "\n"


def main():
    import fidelity
    base = sys.argv[1] if len(sys.argv) > 1 else os.path.join(fidelity.CHECKS, "place_b")
    a = audit(base)
    if not a:
        print("no rustudio/assets.json in", base)
        return 1
    print(json.dumps(a["summary"]), "regressions:", a["regressions"])


if __name__ == "__main__":
    sys.exit(main())
