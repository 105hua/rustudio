#!/usr/bin/env python3
"""Diff rustudio globals-probe logs against the captured Studio 0.744 results.

usage: compare.py [RUSTUDIO_LOG ...]   (default: the *-final / extra rustudio logs)

Each probe key is compared per side (server/client) as a set of normalized
values across GlobalsProbe1/2. Normalization removes only what is legitimately
dynamic: scrambled addresses (raw host pointers 0x00005.../0x00007f... are kept
as RAWPTR so they still count as mismatches), player names, probe script
numbers, memory totals and timings (reduced to integer/fraction shape).
"""
import re, sys
from pathlib import Path

P = Path(__file__).parent
STUDIO = ['studio-complete.log']
DEFAULT = ['rustudio-server-final.log', 'rustudio-client-final.log',
           'rustudio-extra-server.log', 'rustudio-extra-client.log']
LINE = re.compile(r'GLOBALS\|(server|client)\|([^|]+)\|(.*)')
# Keys whose values are timings / memory / clocks: compare number *shape* only.
DYNAMIC = re.compile(r'^(gc\.count|gc\.info|os$|clock\.delta|task\.wait|wait$|misc\.(elapsedTime|time|tick))')
SKIP = re.compile(r'^(inventory\.env\.|meta\.env|metakey\.env)')


def addr(m):
    h = m.group(1)
    if re.match(r'0000[5-7]', h) or h.startswith('00007f'):
        return '0xRAWPTR'
    return '0xADDR'


def num(m):
    s = m.group(0)
    return 'INT' if re.fullmatch(r'-?\d+', s) else 'FRAC'


def norm(key, v):
    v = v.split('  —  ')[0].rstrip()
    v = re.sub(r'0x([0-9a-f]{16})', addr, v)
    v = re.sub(r'Players\.[^.\s]+\.PlayerScripts', 'Players.P.PlayerScripts', v)
    v = re.sub(r'GlobalsProbe[12]', 'GlobalsProbeN', v)
    if key == 'require.table.table: ADDR' or key.startswith('require.table.'):
        pass
    if DYNAMIC.match(key):
        v = re.sub(r'-?\d+(\.\d+)?(e-?\d+)?', num, v)
    return v


def load(files):
    out = {}
    for fn in files:
        path = Path(fn) if Path(fn).exists() else P / fn
        for l in path.read_text(errors='replace').splitlines():
            m = LINE.search(l)
            if not m:
                continue
            side, key, val = m.groups()
            key = re.sub(r'0x[0-9a-f]{16}', 'ADDR', key)
            out.setdefault((side, key), set()).add(norm(key, val))
    return out


def main():
    rfiles = sys.argv[1:] or DEFAULT
    s, r = load(STUDIO), load(rfiles)
    keys = sorted(k for k in s if not SKIP.match(k[1]))
    mism, missing = [], []
    for k in keys:
        if k not in r:
            missing.append(k)
        elif not (s[k] & r[k]) and s[k] != r[k]:
            mism.append(k)
    for k in mism:
        print(f'MISMATCH {k[0]} {k[1]}\n   studio:   {sorted(s[k])[:2]}\n   rustudio: {sorted(r[k])[:2]}')
    for k in missing:
        print(f'MISSING  {k[0]} {k[1]}')
    # environment surface: names visible through getfenv() that Studio lacks
    sv = {k[1] for k in s if k[1].startswith('inventory.visible.')}
    rv = {k[1] for k in r if k[1].startswith('inventory.visible.')}
    extra = sorted(rv - sv)
    print(f'EXTRA_VISIBLE {len(extra)}: {" ".join(x.removeprefix("inventory.visible.") for x in extra)[:600]}')
    envk = {k[1] for k in r if k[1].startswith('inventory.env.') and k[1].count('.') == 2}
    print(f'ENV_PAIRS rustudio: {sorted(x.removeprefix("inventory.env.") for x in envk)}')
    print(f'TOTAL compared={len(keys)} mismatch={len(mism)} missing={len(missing)} extra_visible={len(extra)}')


main()
