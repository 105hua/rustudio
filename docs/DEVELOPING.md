# Developing rustudio

## Command-line options

Mostly for automated testing and benchmarking.

```sh
rustudio --screenshot out.png    # draw ~120 frames, save the window, exit
rustudio --self-test out.json    # drive the editor with scripted input, exit
rustudio place.rbxl --bench out.json [--play] [--seconds 10]   # frame times (avg, 1% low, slowest frames)
rustudio --stats place.rbxl      # what's in a place (counts by class)
rustudio --camera X,Y,Z,YAW,PITCH ...   # start the camera somewhere (degrees)
# Play a place and look at it like Studio's 863×675 view: click at 40 s, run
# Luau on the client at 45 s, screenshot (plus FILE.view.png) and Output at 50 s
rustudio place.rbxl --play --window 1148x1072 --click 110,143@40 \
  --luau client probe.luau@45 --screenshot shot.png --shot-after 50 --log out.txt
# Several shots timed from the start of Play: shot-2s.png, shot-10s.png, ...
# (each with .view.png, the viewport alone) and shot.frames.json (frame times)
rustudio place.rbxl --play --from-play --shots 2,5,10,20,40 --screenshot shot.png
# A fractional time keeps its decimals (--shots 2.5 -> shot-2.5s.png, 6 or 6.0
# -> shot-6s.png); every file written is printed as "[shot] /abs/path.png" on
# stdout and at the end of the --log file.
# Click a TextBox, type into it, press Enter; turn the wheel at a view position
# (over a ScrollingFrame) instead of the centre:
rustudio place.rbxl --play --from-play --shots 5 --screenshot t.png \
  --click 120,95@2 --type "hello"@2.5 --key Enter@3 --wheel -3@4:400,250
rustudio --write-corpus tools/fidelity/corpus   # regenerate the demo / control places
cargo test --workspace           # unit tests (undo/redo, files, Luau runtime, physics, meshes, FBX, GUI, PKCE)
```
`RUSTUDIO_GUI_DEBUG=1` logs GUI clicks and remote calls to Output;
`RUSTUDIO_NO_GUI=1` hides every GUI (to tell GUI from 3D in a picture).

The `--bench` report (round 20) has, besides avg / 1%-low fps and the
section timers per frame (`play_tick`, `p6b_lua`, `gui_walk`, `gui_mesh`,
`ray_parts`, … and `between_frames`, the time outside the frame's own work),
`lua`: the slowest Lua by connection ("Chunk:line Event"), by resumed
thread ("(resume) Chunk:line" where it waits) and by script start, and
`api`: the slowest engine members called from Lua ("call
FindPartOnRayWithIgnoreList", "index Position") with their call counts.
`RUSTUDIO_HITCH=1` prints every Lua resume over 10 ms (with the engine
calls inside one over 300 ms) and every engine call over 5 ms, to find
hitches.

`--screenshot`, `--self-test` and `--bench` run inside a headless gamescope,
so they never open a window on your screen (`RUSTUDIO_VISIBLE_TESTS=1` shows
them). gamescope's own lines (and its Xwayland's and Vulkan WSI layer's) stay
out of the console: rustudio's stdout / stderr come through, gamescope's are
shown only when the run fails. The self-test's last step opens your place
(`RUSTUDIO_PERF_PLACE`, default the Place A build, else og.rbxl, in `src/mech.rs`) read-only,
plays it for 5 s and records the average and 1%-low frame times.

## Layout of the code


| Path | What it does |
|---|---|
| `src/doc.rs` | the document: DOM, undo/redo, copy/paste, group, file I/O |
| `src/render.rs`, `src/shader.wgsl` | wgpu renderer (offscreen MSAA texture shown in egui) |
| `src/scene.rs` | turns parts and Lighting into draw data |
| `src/viewport.rs` | camera input, picking, selection boxes, gizmos |
| `src/explorer.rs`, `src/properties.rs` | the Explorer and Properties panels |
| `src/app.rs` | menus, toolbar, shortcuts, docking, screenshot harness |
| `src/account_ui.rs` | the account window (background threads) |
| `src/selftest.rs` | `--self-test` scripted input driver |
| `src/runtime.rs`, `src/ltypes.rs` | Luau runtime: Instance API, signals, scheduler; Roblox datatypes |
| `src/play.rs`, `src/physics.rs` | Play / Run / Stop, character, rapier3d physics; `physics::move_humanoid` is Studio's Humanoid controller (floor rays, hip spring, body box; docs/MECHANICS.md "Round 22") |
| `src/script_editor.rs` | script tabs: highlighting, line numbers, syntax check |
| `src/replicate.rs` | server → client copy and per-frame replication |
| `src/rig.rs`, `src/avatar.rs` | R15 / R6 rigs, posing from Motor6D.Transform, states; avatar loading and dressing |
| `src/default_scripts.rs` | Roblox's default player / character scripts, read from the Studio install at Play |
| `src/mesh.rs`, `src/texture.rs`, `src/assets.rs` | mesh parsing (Roblox mesh 1–7, OBJ), PNG/JPEG/DDS, asset download and cache, clothing |
| `src/runtime_ext.rs`, `src/library.luau` | engine members beyond the core API (ClickDetector, Sound, Mouse, tags, raycasts…) and services written in Luau (TweenService, Debris, DataStores…) |
| `src/play_game.rs` | in-game interaction in Play: clicks, chat, tools and hotbar, explosions, sounds, NPC MoveTo |
| `src/terrain.rs`, `src/audio.rs` | voxel terrain (SmoothGrid) decoding and meshing; sound playback (rodio) |
| `src/bench.rs` | frame-time measurement and the section timers |
| `src/gui.rs`, `src/gui_walk.rs`, `src/gui_input.rs` | GUIs: fonts, images, rich text; layout and drawing of one GUI; placing GUIs on screen / surfaces / billboards and the mouse and keyboard |
| `src/fbx.rs` | binary FBX reader and the Roblox-style import for `LoadAsset` |
| `crates/rustudio-account` | the Roblox session (cookie, keyring, CSRF), profile and avatar, experiences, downloads, publishing |

