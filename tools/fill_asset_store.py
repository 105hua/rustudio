#!/usr/bin/env python3
"""Copy rustudio's downloaded assets into an offline Studio's store.

  python3 -I tools/fill_asset_store.py [--dry-run]

The reference Studio (tools/fidelity) runs offline: it can only show assets
in ~/.local/share/rustudio/assets (<id>.<ext>, the type sniffed from the bytes
by offline-studio/assets.py). rustudio's signed-in downloads sit in
~/.cache/rustudio/assets/<id>_vlatest.bin. Copying them over lets Studio draw
the same meshes and textures, so place comparisons (Place B's) compare like
with like. Ids already in the store are left alone; nothing is uploaded.
"""
import os
import shutil
import sys

CACHE = os.path.expanduser(os.environ.get("RUSTUDIO_CACHE_DIR", "~/.cache/rustudio")) + "/assets"
STORE = os.path.join(os.path.expanduser(os.environ.get("RUSTUDIO_DATA", "~/.local/share/rustudio")), "assets")


def ext_of(head):
    if head[:8] == b"\x89PNG\r\n\x1a\n":
        return ".png"
    if head[:3] == b"\xff\xd8\xff":
        return ".jpg"
    if head[:8] == b"version ":
        return ".mesh"
    if head[:8] == b"<roblox!":
        return ".rbxm"
    if head[:7] == b"<roblox":
        return ".rbxmx"
    if head[:4] == b"OggS":
        return ".ogg"
    if head[:3] == b"ID3" or head[:2] in (b"\xff\xfb", b"\xff\xf3"):
        return ".mp3"
    if head[:4] == b"DDS ":
        return ".dds"
    if head[4:12] == b"ftypavif":
        return ".avif"
    return ".bin"


def main(dry):
    have = {f.split(".")[0] for f in os.listdir(STORE) if f.split(".")[0].isdigit()}
    added = skipped = 0
    kinds = {}
    for name in sorted(os.listdir(CACHE)):
        if not name.endswith("_vlatest.bin"):
            continue
        aid = name.split("_", 1)[0]
        if not aid.isdigit() or aid in have:
            skipped += 1
            continue
        src = os.path.join(CACHE, name)
        with open(src, "rb") as f:
            ext = ext_of(f.read(16))
        kinds[ext] = kinds.get(ext, 0) + 1
        if not dry:
            tmp = os.path.join(STORE, f".{aid}.tmp")
            shutil.copyfile(src, tmp)
            os.replace(tmp, os.path.join(STORE, aid + ext))
        added += 1
    print(f"{'would add' if dry else 'added'} {added}, already there {skipped}: {kinds}")


if __name__ == "__main__":
    main("--dry-run" in sys.argv[1:])
