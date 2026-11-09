# Material textures

Where Studio's material textures come from, and how to fetch them. Verified
2026-11-01; nothing here needs an account, a cookie or an API key.

## Short version

Studio's material textures are **not files on disk**. The Studio copy has only
`textures/plastic` and `textures/studs` (plus sky and water); everything else is
a **texture pack** — a small XML document stored as a Roblox asset that names
four image assets (colour, normal, roughness, height). Studio downloads them at
run time. So drawing real materials is a fetch, not a file copy.

## The endpoint that works

Public asset delivery, no authentication:

```
GET https://assetdelivery.roblox.com/v2/assetId/{id}
  -> {"locations":[{"location":"https://fts.rbxcdn.com/..."}]}
GET <that location>            -> gzip; inflate it
```

`GET https://assetdelivery.roblox.com/v1/asset?id={id}` 302s to the same CDN.

**Do not use** `https://apis.roblox.com/asset-delivery-api/v1/assetId/{id}`.
That is Open Cloud: it needs the `legacy-asset:manage` scope, and with the
account key in `~/.config/rbxasset/key` it answers
`{"errors":[{"code":0,"message":"Scope not authorized."}]}`. The public
endpoint above serves the same bytes anonymously.

## Two hops, not one

The ids in `PartMaterialInfos2022` / the pre-2022 table (128 of them, extracted
from `Studio`) are **pack descriptors**, not images. Each body is
XML:

```xml
<roblox>
  <texturepack_version>2</texturepack_version>
  <usage>3</usage>
  <alphamode>2</alphamode>
  <tiling>1</tiling>
  <color>9439631732</color>
  <normal>9439632006</normal>
  <roughness>9439632145</roughness>
  <height>9439632299</height>
</roblox>
```

So: pack id -> read the four child ids -> fetch each child, which is the real
map. Verified end to end with no credentials:

| id | what | bytes (gzip) | payload |
|---|---|---|---|
| 9439632387 | pack | 165 | the XML above |
| 9439631732 | colour | 2,473,505 | 1024x1024 RGBA **PNG** |
| 9439632006 | normal | 2,473,369 | PNG |
| 9439632145 | roughness | 885,676 | PNG |
| 9439632299 | height | 128,376 | PNG |

Some entries carry only `<normal>` (9438453972) and some maps are 4x4
placeholders (9438453869 is a 4x4 PNG). Treat a missing map as absent, not an
error.

## Rules that matter when drawing

- `MaterialService.Use2022Materials` picks the 2022 table; false picks the
  pre-2022 (legacy) table. Both are in the exe.
- On top of the table: per-material `MaterialService` overrides, then
  `MaterialVariant`s (each variant is itself a texture pack plus a tiling
  scale).
- Legacy surfaces draw studs/inlets from `textures/studs` — the one material
  atlas that *is* on disk (a DDS atlas; the tiles are laid out side by side).
- Terrain materials come from `terrain/materials.json`,
  `materials2022.json` and `materialTransitions.json` in `PlatformContent/pc`
  and blend between materials; they are a separate path from parts.
- Tiling comes from the pack (`<tiling>`); the same pack repeats over a face
  according to studs-per-tile, so a 4x4 stud part shows one tile, not four.
- Cache by asset id: material assets never change, so a second run is offline.

## Status

Implemented (round 9, `src/matpack.rs`): the three material tables are read
from Studio (pre-2022, 2022 and 4K 2022; ids and colours with
their addresses in `MECHANICS.md`), packs and maps come from the endpoint
above and are cached by id, the maps are decoded into texture arrays and
drawn repeating every 8 studs, as Studio's part shader does. Not drawn yet:
the second (detail normal) pack, and roughness / metalness / height in the
lighting. The self-test step "material-packs" checks Brick, Wood, Concrete,
Metal, Grass, Slate and Fabric. Plastic's 2022 detail pack (9475422736)
answers 401 without a login.
