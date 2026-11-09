"""Diff a rustudio bridge run (run.sh OUTDIR) against studio-extended.log.
usage: compare.py OUTDIR [--list]   (rows normalized: script positions,
player names, pointers)."""
import re, sys
from pathlib import Path
p = Path(__file__).parent
def parse(files):
    d = {}
    for f in files:
        if not Path(f).exists(): continue
        for l in Path(f).read_text(errors='replace').splitlines():
            if 'BRIDGE|' not in l: continue
            x = l[l.index('BRIDGE|'):].split('|')
            if len(x) < 3: continue
            d[x[1], x[2]] = '|'.join(x[3:]).split('  —  ')[0].rstrip()
    return d
def norm(s):
    s = re.sub(r'(?:Players\.\w+\.PlayerScripts|ServerScriptService)\.BridgeProbe:\d+: ', '', s)
    s = re.sub(r'harness:\d+: ', '', s)
    s = re.sub(r'0x[0-9a-f]+', '0xPTR', s)
    s = re.sub(r'Players\.\w+\.PlayerScripts', 'Players.P.PlayerScripts', s)
    return s
st = parse([p / 'studio-extended.log'])
o = Path(sys.argv[1])
ru = parse([o / 'client.log', o / 'server.log'])
# The local player's name (random per run here) as Studio's "Player".
for side in ('client', 'server'):
    meta = ru.get((side, 'player.meta'), '')
    name = meta.split('|')[-1] if meta else ''
    if name and name != 'Player':
        for k in [k for k in ru if k[0] == side]:
            ru[k] = re.sub(r'\b' + re.escape(name) + r'\b', 'Player', ru[k])
keys = list(st)
mis = [k for k in keys if k in ru and norm(st[k]) != norm(ru[k])]
missing = [k for k in keys if k not in ru]
print(f'studio rows {len(keys)}  measured {len(keys) - len(missing)}  matching {len(keys) - len(missing) - len(mis)}  mismatches {len(mis)}  unmeasured {len(missing)}')
if '--list' in sys.argv:
    for k in mis: print(f'{k[0]}|{k[1]}\n   S: {norm(st[k])[:200]}\n   R: {norm(ru[k])[:200]}')
    for k in missing: print(f'{k[0]}|{k[1]}  (unmeasured)')
