<p align="center">
  <img src="assets/icons/rustudio-256.png" width="128" alt="rustudio icon">
</p>

<h1 align="center">rustudio</h1>

<p align="center">
  <a href="https://discord.gg/pdVZpADpWr"><img src="https://img.shields.io/badge/Discord-join%20the%20server-5865F2?logo=discord&logoColor=white" alt="Join the rustudio Discord"></a>
</p>

**A native Linux/Windows/macOS Roblox Studio, written in Rust.** Open, edit and save
Roblox places, press Play and run them with real Luau, physics, characters
and GUIs, without Wine.

Rustudio is very much a beta/prototype, I wouldn't call it a studio replacement
quite yet.

![Playing a test place in rustudio](docs/images/play.png)

![The rustudio home screen](docs/images/home.png)

> This project is still early, so please please help!! Contributions are super welcome. Come hang out, ask for help or report bugs on the [Discord](https://discord.gg/pdVZpADpWr).

## Features

- **Editor:** Explorer, Properties, Output, script tabs, move/scale/rotate
  tools, terrain tools, undo/redo, and Studio's menus, ribbon and shortcuts.
- **Files:** `.rbxl` / `.rbxlx` places and `.rbxm` / `.rbxmx` models, plus
  project folders (`*.project.json` with plain files as the source of truth,
  Rojo/Argon sync rules, `sourcemap.json`, and File > Save as Project
  Folder… to turn any place into one).
- **Play / Run:** separate server and client, remotes, physics (rapier3d),
  R15/R6 characters and animation, sounds, tweens, 2D and 3D GUIs.
- **Luau:** the real Luau VM, with most of the engine API that real games
  call (see [docs/ENGINE-GAPS.md](docs/ENGINE-GAPS.md) for what's missing).
- **Rendering:** wgpu, with lighting, shadows, sky, atmosphere and materials
  tuned against Studio. Meshes, FBX import and terrain are supported.
- **Roblox account:** open your places, load assets and avatars, use
  DataStores, and join Team Create sessions.

## Install

You need a Rust toolchain (1.85+) and a GPU with Vulkan (or OpenGL), or on
a Mac, macOS 11 or later (Metal).

```sh
git clone https://github.com/rippelz/rustudio
cd rustudio
cargo build --release
./target/release/rustudio --demo           # sample scene
./target/release/rustudio my_place.rbxl    # open a place
```

On Windows (MSVC toolchain), run `.\target\release\rustudio.exe` instead,
and see the Windows notes under [Platforms](#platforms).

On Linux you can install it for your user instead, so it shows up in your
app launcher and runs as `rustudio` from anywhere:

```sh
./install.sh                # build, then install into ~/.local
./install.sh --uninstall    # remove it again
```

The install keeps working if you move or delete the cloned folder.
`./install.sh --help` lists the other options (`--prefix`, `--no-build`).

On first launch rustudio downloads Roblox Studio's built-in content
(default characters, camera and control scripts, fonts, textures, sounds;
about 60 MB) from Roblox's own CDN into `~/.local/share/rustudio/studio`
(`%LOCALAPPDATA%\rustudio\studio` on Windows).
To get it ahead of time, or update it, run
`rustudio --install-studio-content`.

If you already have Studio installed (e.g. via
[Vinegar](https://vinegarhq.org)), you can point rustudio at that instead:

```sh
export RUSTUDIO_STUDIO_DIR=~/.var/app/org.vinegarhq.Vinegar/data/vinegar/versions/version-XXXX
```

On macOS, download `rustudio-…-macos-universal.zip` from
[Releases](https://github.com/rippelz/rustudio/releases) (Apple Silicon and
Intel in one), unzip it and move `rustudio.app` to Applications. It isn't
notarized by Apple, so the first time right-click it and choose **Open** (or
allow it in System Settings > Privacy & Security). To build the app
yourself instead: `tools/macos/bundle.sh` (`--universal` for both
architectures).

## Platforms

| Platform | Status |
|---|---|
| Linux x86_64 (Wayland or X11) | Supported, and the only one tested |
| Linux ARM64 | Should build; untested |
| macOS 11+ (Apple Silicon and Intel) | Builds and runs (tested on Apple Silicon); see below |
| Windows x86_64 (MSVC) | Builds and runs (tested on Windows 11); see below |

CI checks every push on Linux, Windows and macOS (where it also runs the
tests and builds the app).

Windows is new, so a few things don't work there yet:

- Paths in this README are the Linux ones. On Windows, data and caches go
  in `%LOCALAPPDATA%\rustudio` (e.g. the Studio content download in
  `%LOCALAPPDATA%\rustudio\studio`) and settings in `%APPDATA%\rustudio`.
- Signing in is untested (the session cookie now goes through the
  Windows Credential Manager, like the publishing key).
- Video playback is off (libvpx/libopus are loaded only on Linux), and a
  native crash doesn't write a crash report (panics still do).

On macOS:

- Data and settings go in `~/Library/Application Support/rustudio`, caches
  and crash reports in `~/Library/Caches/rustudio`.
- With Roblox Studio installed (`RobloxStudio.app`), rustudio uses its
  content and downloads nothing.
- Shortcuts are Studio for Mac's (⌘ for Ctrl, ⌥ for Alt; the delete key
  deletes), ⌘Q asks about unsaved changes, and places and models open from
  Finder. The session cookie goes in the login Keychain, and deleted project
  files in the Trash.
- Video playback needs `brew install libvpx opus`. Gamepads aren't read yet.

## Signing in

Everything above works signed out. To open your own places or load private
assets, sign in with your Roblox session: use the account button at the top
right and **Sign in with your browser** (rustudio shows a code that you
confirm on roblox.com, in whatever browser you're signed in to), paste the
`.ROBLOSECURITY` cookie there, or put it in `~/.config/rustudio/roblox-cookie`
(macOS: `~/Library/Application Support/rustudio/roblox-cookie`; `chmod 600`).
rustudio stores it in your login keyring (the Keychain on macOS). Details are in
[docs/AUTH.md](docs/AUTH.md).

## Docs

- [docs/DEVELOPING.md](docs/DEVELOPING.md): command-line options, tests,
  code layout
- [docs/MECHANICS.md](docs/MECHANICS.md): every tuning number and where it
  comes from
- [docs/ENGINE-GAPS.md](docs/ENGINE-GAPS.md): engine API that's still missing
- [docs/TEAMCREATE.md](docs/TEAMCREATE.md): Team Create

## Not affiliated with Roblox

rustudio is an independent project. It is not made, endorsed or supported
by Roblox Corporation. "Roblox" and "Roblox Studio" are trademarks of Roblox
Corporation.

This repository doesn't redistribute Roblox Studio or its content. Studio's built-in
content (fonts, textures, avatar parts, `ReflectionMetadata.xml`) is
downloaded from Roblox's own CDN onto your machine on first launch and is
never redistributed here.

## License

MIT OR Apache-2.0, at your option. Vendored crates in `vendor/` keep their
own licences.
