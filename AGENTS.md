# AGENTS.md

Notes for coding agents (and people) working on rustudio. The README covers
what the project is; this is how to find your way around and check your work.

## Build and test

```sh
cargo build --release
cargo test --workspace
```

The workspace is the app (`src/`) plus `crates/rustudio-account` (Roblox
session, downloads, publishing) and `crates/rustudio-net` (Team Create /
network protocol). Crates in `vendor/` are third-party; change them only
when there is no way around it.

## Seeing a change work

Unit tests don't cover how things look or feel. rustudio has its own harness
for that, described in [docs/DEVELOPING.md](docs/DEVELOPING.md):

- `--screenshot out.png` draws a scene and saves the window.
- `place.rbxl --play --shots 2,5,10 --screenshot shot.png` plays a place and
  takes timed shots, with `--click`, `--type`, `--key`, `--wheel` and
  `--luau client|server file.luau@T` to drive it.
- `--self-test out.json` drives the editor with scripted input.
- `--bench out.json` measures frame times and the slowest Lua and engine calls.
- `--stats place.rbxl` lists what's in a place.

Look at the screenshots you take. A test that passes while the picture is
wrong isn't a fix.

## Where things are

[docs/DEVELOPING.md](docs/DEVELOPING.md) has a table of the main source files.
The rest of `docs/`:

- [docs/MECHANICS.md](docs/MECHANICS.md): every tuning number and where it
  comes from. The values live in `src/mech.rs` and `mechanics.json`.
- [docs/ENGINE-GAPS.md](docs/ENGINE-GAPS.md): engine API that's still missing,
  measured by playing real places.
- [docs/UI-PARITY.md](docs/UI-PARITY.md), [docs/MATERIALS.md](docs/MATERIALS.md):
  matching Studio's editor UI and materials.
- [docs/TEAMCREATE.md](docs/TEAMCREATE.md), [docs/PROTOCOL.md](docs/PROTOCOL.md),
  [docs/AUTH.md](docs/AUTH.md): Team Create, the network protocol and sign-in.
- `checks/parity/`: per-area notes and results comparing rustudio with Studio.

## How changes should look

- The goal is to behave like Roblox Studio. When you change a behaviour or a
  number, say what it's based on (Studio's behaviour, the API dump, a
  measurement) or mark it as an estimate, the way docs/MECHANICS.md does.
- New tuning values go in `src/mech.rs` / `mechanics.json` with a line in
  docs/MECHANICS.md, not as constants scattered in the code.
- Engine API is checked against Roblox's real API (`api-dump.json`). Only add
  members Roblox actually has, and update docs/ENGINE-GAPS.md when you close
  a gap.
- Keep a change to one thing, match the style of the surrounding code, and
  add a test for a bug fix where it's practical.
