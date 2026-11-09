"""Diff a rustudio corpus run (run.sh OUTDIR) against studio.log.
usage: compare.py OUTDIR [--list]   prints mismatch counts (normalized:
script locations and pointer hashes removed)."""
import re,sys
from pathlib import Path
p=Path(__file__).parent
def parse(files):
    d={}
    for f in files:
        if not Path(f).exists(): continue
        for l in Path(f).read_text(errors='replace').splitlines():
            if 'DT|' not in l: continue
            x=l[l.index('DT|'):].split('|',3)
            if len(x)<4: continue
            side,k,v=x[1:]; d[side,k]=v.split('  —  ')[0].replace('\\r','').rstrip()
    return d
def norm(s):
    s=re.sub(r'(?:Players\.\w+\.PlayerScripts|ServerScriptService|Workspace\.DatatypeActor)\.\w+:\d+: ','',s)
    s=re.sub(r'0x[0-9a-f]+','0xPTR',s)
    s=re.sub(r'Players\.\w+\.PlayerScripts','Players.P.PlayerScripts',s)
    return s
st=parse([p/'studio.log'])
o=Path(sys.argv[1])
ru=parse([o/'client.log',o/'server.log',o/'extras.log',o/'actor.log'])
keys=[k for k in st]
mis=[k for k in keys if k in ru and norm(st[k])!=norm(ru[k])]
missing=[k for k in keys if k not in ru]
direct=[k for k in keys if '.direct.' in k[1]]
print(f'studio rows {len(keys)}  measured {len(keys)-len(missing)}  unmeasured {len(missing)}  mismatches {len(mis)}  (direct-call rows {len(direct)}, mismatched {sum(1 for k in mis if ".direct." in k[1])})')
if "--missing" in sys.argv:
    import collections; print(collections.Counter((k[0],k[1].split(".")[0]) for k in missing).most_common(60))
if "--list" in sys.argv:
    for k in mis: print(f'{k[0]}|{k[1]}\n   S: {norm(st[k])[:200]}\n   R: {norm(ru[k])[:200]}')
