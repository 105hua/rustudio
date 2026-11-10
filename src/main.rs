//! rustudio: a native Linux editor for Roblox place and model files.

mod account_ui;
mod ribbon_quick;
mod cmeta;
mod studio_content;
mod paths;
mod dtmeta;
mod cloud_ui;
mod join_ui;
mod asset_report;
mod app;
mod frame_watchdog;
mod class_icons;
mod assets;
mod audio;
mod audio_dsp;
mod audio_scene;
mod audio_capture;
mod avatar;
mod bench;
mod bin_dump;
mod camera;
mod collision;
mod collision_groups;
mod coverage;
mod crash;
mod default_scripts;
mod doc;
mod effects;
mod attachment;
mod explorer;
mod fbx;
mod api;
mod api_stubs;
mod client_joints;
mod query_shapes;
mod fidelity;
mod telemetry;
mod gui;
mod gui_modal;
mod gui_viewport;
mod icons;
mod live_drafts;
mod layout_store;
mod lighting;
mod lightgrid;
mod keys;
mod lightocc;
mod ltypes;
mod math;
mod matpack;
mod mech;
mod part_drag;
mod output_panel;
mod props_layout;
mod prop_slider;
mod color_editors;
mod mesh;
mod mesh_bvh;
mod physics;
mod play;
mod project;
mod project_config;
mod project_export;
mod project_ui;
mod project_sync;
mod project_values;
mod properties;
mod render;
mod sun_shadow;
mod replicate;
mod anim_replicate;
mod rig;
mod runtime;
mod globals_parity;
mod sharedtable;
mod scene;
mod skin;
mod script_editor;
mod selftest;
mod terrain;
mod terrain_editor;
mod terrain_region;
mod terrain_mat;
mod world_gui;
mod texture;
mod theme;
mod ti;
mod studio_ui;
mod studio_actions;
mod studio_ribbon;
mod studio_panels;
mod start_page;
mod ui_shots;
mod viewport;
mod water;
mod wrap;

use std::path::PathBuf;

const USAGE: &str = "\
rustudio — edit Roblox places (.rbxl/.rbxlx) and models (.rbxm/.rbxmx)

usage: rustudio [FILE] [--demo] [--play] [--screenshot OUT.png] [--self-test OUT.json]
                [--bench OUT.json [--seconds N]]

  FILE                 open a place or model on start
  --demo               start with the sample scene
  --open-roblox ID|URL open a place from Roblox (place id, roblox.com/games/… or
                       create.roblox.com/…/places/… link) once signed in; with
                       --local-copy, a Team Create place opens as a local copy
  --roblox-picker TAB  show File > Open from Roblox (recent, mine, groups, shared),
                       or File > Experience Settings (settings)
  --play               start Play (F5) right after loading
  --team-create U P    join the live Team Create server for universe U, place P
  --screenshot FILE    draw ~120 frames, save the window as PNG, exit
  --self-test FILE     drive the editor with scripted input, write results, exit
  --shot-after S       with --screenshot: take it S seconds after start
  --shots T1,T2,…      with --screenshot: a shot (window and viewport) at each time,
                       the Output log and frame times with each (FILE-<t>s.png …)
  --from-play          harness times (shots, --luau, --key) count from Play start
  --write-corpus DIR   write the fidelity corpus places (demo, control) and exit
  --api-coverage OUT [GAPS…]  measure API coverage against app/api-dump.json
                       (RUSTUDIO_GAPS files from real runs add call gaps)
  --dump-scripts PLACE DIR  write every script's source under DIR
  --log FILE           with --screenshot: write the Output lines to FILE
  --key NAME@S[:HOLD]  press a key at S seconds (held HOLD seconds)
  --move X,Y@S         move the pointer to view position X,Y (hover)
  --wheel N@S[:X,Y]    turn the mouse wheel N notches over the 3D view's centre
                       (or at view position X,Y)
  --type TEXT@S        type TEXT at S seconds (into a focused TextBox)
  --rdrag DX,DY@S      right-drag over the 3D view by DX,DY pixels
  --ldrag X1,Y1>X2,Y2@S  left-drag in the 3D view between view positions
  --click X,Y@S        click the 3D view at X,Y (pixels) S seconds after start
  --key NAME@S         press a key (egui name, e.g. Enter, B) S seconds after start
  --window WxH         window size for test runs
  --camera X,Y,Z,YAW,PITCH  start the editor camera there (degrees)
  --bench FILE         measure frame times (after a 3 s warm-up) for N seconds
                       (default 10) at 1265x900 without vsync, write JSON, exit
";

/// Test runs (--screenshot / --self-test) re-launch themselves inside a headless
/// gamescope so they never open a window on the user's screen.
/// RUSTUDIO_VISIBLE_TESTS=1 keeps them on screen.
#[cfg(target_os = "linux")]
fn rerun_headless() -> Option<i32> {
    use std::os::unix::fs::OpenOptionsExt;
    let argv: Vec<String> = std::env::args().collect();
    let testing = argv
        .iter()
        .any(|a| a == "--screenshot" || a == "--self-test" || a == "--bench" || a == "--ui-shots");
    if !testing
        || std::env::var_os("RUSTUDIO_HEADLESS_CHILD").is_some()
        || std::env::var_os("RUSTUDIO_VISIBLE_TESTS").is_some()
    {
        return None;
    }
    // The benchmark measures at the 1265x900 size the person asked for.
    let h = if argv.iter().any(|a| a == "--bench") {
        "900"
    } else {
        "1376"
    };
    // --window WxH: another window size (e.g. to give the 3D view Studio's size).
    let (w, h) = window_arg(&argv)
        .map(|(w, h)| (w.to_string(), h.to_string()))
        .unwrap_or(("1265".into(), h.into()));
    // gamescope's own lines (and its Xwayland's: swapchain notes, "failed to
    // read Wayland events" at exit) go to a file. gamescope closes inherited
    // descriptors, so rustudio inside writes to two FIFOs (RUSTUDIO_PARENT_OUT
    // / _ERR, see restore_parent_fds) that this process copies to its own
    // stdout / stderr.
    let tmp = std::env::temp_dir().join(format!("rustudio-headless-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let gs_log = tmp.join("gamescope.log");
    let gs_out = std::fs::File::create(&gs_log).ok();
    let mut cmd = std::process::Command::new("gamescope");
    let mut pumps = Vec::new();
    if let Some(f) = gs_out {
        let fifos = ["out", "err"].map(|n| tmp.join(n));
        let made = fifos.iter().all(|p| {
            let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).unwrap_or_default();
            // SAFETY: mkfifo on a NUL-terminated path.
            unsafe { libc::mkfifo(c.as_ptr(), 0o600) == 0 }
        });
        if let (true, Ok(f2)) = (made, f.try_clone()) {
            cmd.stdout(f)
                .stderr(f2)
                .env("RUSTUDIO_PARENT_OUT", &fifos[0])
                .env("RUSTUDIO_PARENT_ERR", &fifos[1])
                // (lsfg-vk's frame generation layer, when installed, only
                // adds errors about its own config to a test run.)
                .env("DISABLE_LSFGVK", "1");
            for (i, p) in fifos.iter().enumerate() {
                let p = p.clone();
                let (tx, rx) = std::sync::mpsc::channel::<()>();
                std::thread::spawn(move || {
                    // (Line by line, leaving out the notes gamescope's Vulkan
                    // WSI layer prints from inside rustudio's process.)
                    if let Ok(r) = std::fs::File::open(&p) {
                        use std::io::{BufRead, Write};
                        let mut r = std::io::BufReader::new(r);
                        let mut line = Vec::new();
                        let mut wsi = false;
                        while r.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                            // (its blocks continue on indented lines)
                            wsi = line.starts_with(b"[Gamescope WSI]") || (wsi && line.starts_with(b"  "));
                            if !wsi {
                                let _ = if i == 0 {
                                    let mut o = std::io::stdout().lock();
                                    o.write_all(&line).and_then(|_| o.flush())
                                } else {
                                    std::io::stderr().write_all(&line)
                                };
                            }
                            line.clear();
                        }
                    }
                    let _ = tx.send(());
                });
                pumps.push((fifos[i].clone(), rx));
            }
        }
    }
    let status = cmd
        .args([
            "--backend",
            "headless",
            "-W",
            &w,
            "-H",
            &h,
            "-w",
            &w,
            "-h",
            &h,
            "--",
        ])
        .args(&argv)
        .env("RUSTUDIO_HEADLESS_CHILD", "1")
        .status()
        .ok(); // no gamescope: fall back to a normal window
    for (fifo, done) in pumps {
        // (A FIFO rustudio never opened: open its write end so the copier's
        // open returns, then let it see the end.)
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo);
        let _ = done.recv_timeout(std::time::Duration::from_secs(2));
    }
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let gs_text = std::fs::read_to_string(&gs_log).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&tmp);
    let status = status?;
    // A failed run shows gamescope's lines after all (it may never have
    // started rustudio).
    if !status.success() && !gs_text.is_empty() {
        eprintln!("gamescope output:\n{gs_text}");
    }
    Some(status.code().unwrap_or(1))
}

#[cfg(not(target_os = "linux"))]
fn rerun_headless() -> Option<i32> {
    None
}

/// Inside the headless gamescope: write to the launching process's stdout /
/// stderr (FIFOs it copies from, RUSTUDIO_PARENT_OUT / _ERR), not gamescope's.
#[cfg(target_os = "linux")]
fn restore_parent_fds() {
    for (var, to) in [("RUSTUDIO_PARENT_OUT", 1), ("RUSTUDIO_PARENT_ERR", 2)] {
        let Some(p) = std::env::var_os(var) else { continue };
        // (Our own children, e.g. a nested test run, don't inherit it.)
        std::env::remove_var(var);
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&p) {
            use std::os::fd::IntoRawFd;
            let fd = f.into_raw_fd();
            // SAFETY: dup2 / close on a descriptor we just opened.
            unsafe {
                libc::dup2(fd, to);
                libc::close(fd);
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn restore_parent_fds() {}

fn window_arg(argv: &[String]) -> Option<(u32, u32)> {
    let i = argv.iter().position(|a| a == "--window")?;
    let (w, h) = argv.get(i + 1)?.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn main() -> eframe::Result<()> {
    restore_parent_fds();
    // First: a panic or fatal signal from here on leaves a report.
    crash::install();
    // RUSTUDIO_CRASH_TEST=panic|segv|thread: check the reports themselves.
    match std::env::var("RUSTUDIO_CRASH_TEST").as_deref() {
        Ok("panic") => panic!("RUSTUDIO_CRASH_TEST panic"),
        Ok("thread") => {
            let _ = std::thread::Builder::new()
                .name("crash-test".into())
                .spawn(|| panic!("RUSTUDIO_CRASH_TEST thread panic"))
                .map(|h| h.join());
        }
        // SAFETY: deliberately faults, to test the signal report.
        Ok("segv") => unsafe { libc::raise(libc::SIGSEGV); },
        _ => {}
    }
    // os.clock counts from here, as Studio's does from its process start.
    let _ = runtime::process_start();
    // Before any thread makes a Luau VM: the flags are process-global.
    runtime::luau_flags();
    matpack::init_from_env();
    // RUSTUDIO_PLACE_CONTEXT=ID: ask asset delivery in that Roblox place's
    // context for a local file (a copy of a Roblox place).
    if let Some(p) = std::env::var("RUSTUDIO_PLACE_CONTEXT").ok().and_then(|v| v.parse().ok()) {
        rustudio_account::session::set_place_context(p);
    }
    // `--install-studio-content`: download Studio's built-in content from
    // Roblox's CDN into ~/.local/share/rustudio/studio, then exit.
    if std::env::args().any(|a| a == "--install-studio-content") {
        match studio_content::install(&|s| eprintln!("studio content: {s}")) {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                eprintln!("studio content: {e}");
                std::process::exit(1);
            }
        }
    }
    // `--write-corpus DIR`: the fidelity corpus's place files, then exit.
    let argv: Vec<String> = std::env::args().collect();
    if let Some(i) = argv.iter().position(|a| a == "--write-corpus") {
        let dir = argv.get(i + 1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        match fidelity::write_corpus(&dir) {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("write-corpus: {e}");
                std::process::exit(1);
            }
        }
    }
    // `--export-project OUT_DIR PLACE.rbxl [--lua] [--containers A,B/C]`: a
    // place written out as a Rojo / Argon project folder (src/project_export.rs).
    if let Some(i) = argv.iter().position(|a| a == "--export-project") {
        let (Some(out), Some(src)) = (argv.get(i + 1), argv.get(i + 2)) else {
            eprintln!("usage: rustudio --export-project OUT_DIR PLACE.rbxl [--lua] [--no-sourcemap] [--containers ReplicatedStorage,StarterPlayer/StarterPlayerScripts,...]");
            std::process::exit(2);
        };
        let out = PathBuf::from(out);
        let mut o = project_export::ExportOptions {
            name: out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "game".into()),
            lua: argv.iter().any(|a| a == "--lua"),
            sourcemap: !argv.iter().any(|a| a == "--no-sourcemap"),
            ..Default::default()
        };
        if let Some(j) = argv.iter().position(|a| a == "--containers") {
            o.containers = argv.get(j + 1).map(|s| s.split(',').map(String::from).collect()).unwrap_or_default();
        }
        let res = doc::Document::open(std::path::Path::new(src)).and_then(|d| project_export::export(&d, &out, &o));
        match res {
            Ok(r) => {
                for w in &r.warnings {
                    eprintln!("warning: {w}");
                }
                println!(
                    "{}: {} scripts, {} folders, {} models",
                    r.project_file.display(), r.scripts, r.folders, r.models
                );
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("export-project: {e}");
                std::process::exit(1);
            }
        }
    }
    // `--sourcemap FOLDER [OUT] [--include-non-scripts]`: Rojo's sourcemap.json.
    if let Some(i) = argv.iter().position(|a| a == "--sourcemap") {
        let Some(src) = argv.get(i + 1) else {
            eprintln!("usage: rustudio --sourcemap FOLDER|FILE.project.json [OUT.json] [--include-non-scripts]");
            std::process::exit(2);
        };
        let src = PathBuf::from(src);
        let file = if src.is_dir() { project::project_files(&src).into_iter().next() } else { Some(src.clone()) };
        let all = argv.iter().any(|a| a == "--include-non-scripts");
        let res = file
            .ok_or_else(|| format!("no *.project.json in {}", src.display()))
            .and_then(|f| project_sync::open(&f))
            .map(|(doc, st)| st.sourcemap_bytes(&doc, all));
        match res {
            Ok(bytes) => {
                match argv.get(i + 2).filter(|a| !a.starts_with("--")) {
                    Some(out) => {
                        if let Err(e) = std::fs::write(out, &bytes) {
                            eprintln!("sourcemap: {out}: {e}");
                            std::process::exit(1);
                        }
                    }
                    None => {
                        use std::io::Write;
                        let _ = std::io::stdout().write_all(&bytes);
                    }
                }
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("sourcemap: {e}");
                std::process::exit(1);
            }
        }
    }
    // `--build OUT.rbxl FOLDER`: a project folder (its default project file,
    // over the place file it names) saved as one place file, then exit.
    if let Some(i) = argv.iter().position(|a| a == "--build") {
        let (Some(out), Some(src)) = (argv.get(i + 1), argv.get(i + 2)) else {
            eprintln!("usage: rustudio --build OUT.rbxl FOLDER|FILE.project.json");
            std::process::exit(2);
        };
        let src = PathBuf::from(src);
        let file = if src.is_dir() {
            project::project_files(&src).into_iter().next()
        } else {
            Some(src.clone())
        };
        let res = file
            .ok_or_else(|| format!("no *.project.json in {}", src.display()))
            .and_then(|f| project_sync::open(&f))
            .and_then(|(mut doc, _)| doc.save_to(std::path::Path::new(out)));
        match res {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("build: {e}");
                std::process::exit(1);
            }
        }
    }
    if let Some(code) = rerun_headless() {
        std::process::exit(code);
    }
    // First launch without a Studio install: fetch Studio's built-in content
    // in the background (never in test runs, which must be reproducible).
    let test_run = std::env::args()
        .any(|a| matches!(a.as_str(), "--screenshot" | "--self-test" | "--bench" | "--ui-shots"));
    if !test_run {
        studio_content::spawn_background_install_if_missing();
    }
    let mut open: Option<PathBuf> = None;
    let mut screenshot: Option<PathBuf> = None;
    let mut selftest: Option<PathBuf> = None;
    let mut ui_shots: Option<PathBuf> = None;
    let mut demo = false;
    let mut start_play = false;
    let mut bench: Option<PathBuf> = None;
    let mut camera: Option<[f32; 5]> = None;
    let mut seconds = mech::PERF_SECONDS;
    let mut shot_after: Option<f32> = None;
    let mut shots: Vec<f32> = Vec::new();
    let mut from_play = false;
    let mut log_out: Option<PathBuf> = None;
    let mut actions: Vec<(f32, app::HarnessAction)> = Vec::new();
    let mut open_roblox: Option<u64> = None;
    let mut team_create: Option<(u64, u64)> = None;
    let mut roblox_picker: Option<cloud_ui::PickerTab> = None;
    let mut roblox_settings = false;
    let mut local_copy = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--bench" => bench = args.next().map(PathBuf::from),
            "--camera" => {
                let v: Vec<f32> = args
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|x| x.trim().parse().ok())
                    .collect();
                if v.len() == 5 {
                    camera = Some([v[0], v[1], v[2], v[3], v[4]]);
                }
            }
            "--shot-after" => shot_after = args.next().and_then(|s| s.parse().ok()),
            "--shots" => {
                shots = args
                    .next()
                    .map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect())
                    .unwrap_or_default()
            }
            "--from-play" => from_play = true,
            "--log" => log_out = args.next().map(PathBuf::from),
            "--click" | "--key" => {
                let v = args.next().unwrap_or_default();
                match app::HarnessAction::parse(&v, a == "--click") {
                    Some(x) => actions.extend(x),
                    None => eprintln!("{a}: can't read {v:?}"),
                }
            }
            "--move" => {
                // The pointer to view position X,Y at S (hover, no buttons).
                let v = args.next().unwrap_or_default();
                match v.split_once('@').and_then(|(xy, at)| {
                    let (x, y) = xy.split_once(',')?;
                    Some((
                        at.trim().parse::<f32>().ok()?,
                        app::HarnessAction::PointerAt(x.trim().parse().ok()?, y.trim().parse().ok()?),
                    ))
                }) {
                    Some(x) => actions.push(x),
                    None => eprintln!("--move: can't read {v:?} (X,Y@SECONDS)"),
                }
            }
            "--wheel" => {
                let v = args.next().unwrap_or_default();
                match app::HarnessAction::parse_wheel(&v) {
                    Some(x) => actions.push(x),
                    None => eprintln!("--wheel: can't read {v:?} (NOTCHES@SECONDS or NOTCHES@SECONDS:X,Y)"),
                }
            }
            "--type" => {
                let v = args.next().unwrap_or_default();
                match app::HarnessAction::parse_type(&v) {
                    Some(x) => actions.push(x),
                    None => eprintln!("--type: can't read {v:?} (TEXT@SECONDS)"),
                }
            }
            "--shift" => {
                let v = args.next().unwrap_or_default();
                match v.split_once(':').and_then(|(a, h)| {
                    Some((a.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?))
                }) {
                    Some((at, hold)) => {
                        actions.push((at, app::HarnessAction::Shift(true)));
                        actions.push((at + hold, app::HarnessAction::Shift(false)));
                    }
                    None => eprintln!("--shift: can't read {v:?} (SECONDS:HOLD)"),
                }
            }
            "--action" => {
                let v = args.next().unwrap_or_default();
                match v.rsplit_once('@').and_then(|(n, at)| Some((at.trim().parse::<f32>().ok()?, n.trim().to_string()))) {
                    Some((at, name)) => actions.push((at, app::HarnessAction::UiAction(name))),
                    None => eprintln!("--action: can't read {v:?} (NAME@SECONDS)"),
                }
            }
            "--ldrag" => {
                let v = args.next().unwrap_or_default();
                match app::HarnessAction::parse_ldrag(&v) {
                    Some(list) => actions.extend(list),
                    None => eprintln!("--ldrag: can't read {v:?} (X1,Y1>X2,Y2@SECONDS)"),
                }
            }
            "--rdrag" => {
                let v = args.next().unwrap_or_default();
                match app::HarnessAction::parse_rdrag(&v) {
                    Some(x) => actions.extend(x),
                    None => eprintln!("--rdrag: can't read {v:?} (DX,DY@SECONDS)"),
                }
            }
            "--luau" => {
                // --luau server|client FILE@S
                let side = args.next().unwrap_or_default();
                let v = args.next().unwrap_or_default();
                match v.rsplit_once('@').and_then(|(f, at)| {
                    Some((std::fs::read_to_string(f).ok()?, at.parse::<f32>().ok()?))
                }) {
                    Some((code, at)) => {
                        actions.push((at, app::HarnessAction::Luau(side != "client", code)))
                    }
                    None => eprintln!("--luau: can't read {v:?} (FILE@SECONDS)"),
                }
            }
            "--window" => {
                args.next();
            }
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(seconds),
            "--self-test" => selftest = args.next().map(PathBuf::from),
            "--ui-shots" => ui_shots = args.next().map(PathBuf::from),
            "--stats" => {
                // Counts of what's in a place (no window).
                let path = args.next().map(PathBuf::from).unwrap_or_default();
                match doc::Document::open(&path) {
                    Ok(d) => print!("{}", d.stats()),
                    Err(e) => eprintln!("{e}"),
                }
                return Ok(());
            }
            "--api-coverage" => {
                // Measures API coverage against app/api-dump.json (no window):
                // --api-coverage OUT [GAPS.json …] (real-run gap files).
                let out = args.next().map(PathBuf::from).unwrap_or_else(|| "coverage.json".into());
                let gaps: Vec<PathBuf> = args.by_ref().map(PathBuf::from).collect();
                match coverage::run(&out, &gaps) {
                    Ok(r) => {
                        for k in ["functions", "properties", "events", "callbacks"] {
                            println!(
                                "{k}: exists {} / real {} / {} (game scripts: exists {} / real {} / {})",
                                r[k]["exists"], r[k]["implemented"], r[k]["total"],
                                r[k]["game_scripts_exists"], r[k]["game_scripts_implemented"],
                                r[k]["game_scripts_total"]
                            );
                        }
                    }
                    Err(e) => eprintln!("coverage: {e}"),
                }
                return Ok(());
            }
            "--dump-scripts" => {
                // Writes every script's full source under DIR as <Full.Name>.luau (no window).
                let path = args.next().map(PathBuf::from).unwrap_or_default();
                let dir = args.next().map(PathBuf::from).unwrap_or_default();
                match doc::Document::open(&path) {
                    Ok(d) => {
                        let _ = std::fs::create_dir_all(&dir);
                        let mut n = 0;
                        for (name, src) in d.all_script_sources() {
                            if std::fs::write(dir.join(format!("{name}.luau")), src).is_ok() {
                                n += 1;
                            }
                        }
                        println!("wrote {n} scripts to {}", dir.display());
                    }
                    Err(e) => eprintln!("{e}"),
                }
                return Ok(());
            }
            "--fetch-assets" => {
                // Downloads public assets by id (rustudio's cache path) into
                // DIR as <id>.<ext>; a texture pack's maps come along. Used to
                // give an offline Studio the terrain packs (round 21).
                let dir = args.next().map(PathBuf::from).unwrap_or_default();
                let _ = std::fs::create_dir_all(&dir);
                let mut queue: Vec<u64> = args.by_ref().filter_map(|a| a.parse().ok()).collect();
                let mut done = std::collections::HashSet::new();
                while let Some(id) = queue.pop() {
                    if !done.insert(id) {
                        continue;
                    }
                    match assets::public_cached(id) {
                        Ok(b) => {
                            let text = std::str::from_utf8(&b).ok();
                            let ext = if b.starts_with(b"\x89PNG") {
                                "png"
                            } else if b.starts_with(b"\xff\xd8\xff") {
                                "jpg"
                            } else if text.is_some_and(|t| t.contains("texturepack_version")) {
                                "texturepack.xml"
                            } else {
                                "bin"
                            };
                            if let Some(p) = text.and_then(|t| matpack::parse_pack(t).ok()) {
                                queue.extend([p.color, p.normal, p.roughness, p.metalness, p.height].into_iter().flatten());
                            }
                            let out = dir.join(format!("{id}.{ext}"));
                            match std::fs::write(&out, &b[..]) {
                                Ok(()) => println!("{id} {} bytes -> {}", b.len(), out.display()),
                                Err(e) => eprintln!("{id}: {e}"),
                            }
                        }
                        Err(e) => eprintln!("{id}: {e}"),
                    }
                }
                return Ok(());
            }
            "--script" => {
                // Prints the source of scripts whose full name contains PATTERN (no window).
                let path = args.next().map(PathBuf::from).unwrap_or_default();
                let pat = args.next().unwrap_or_default();
                match doc::Document::open(&path) {
                    Ok(d) => print!("{}", d.script_sources(&pat)),
                    Err(e) => eprintln!("{e}"),
                }
                return Ok(());
            }
            "--team-create" => {
                let universe = args.next().and_then(|v| v.parse::<u64>().ok()).filter(|v| *v != 0);
                let place = args.next().and_then(|v| v.parse::<u64>().ok()).filter(|v| *v != 0);
                match (universe, place) {
                    (Some(u), Some(p)) => team_create = Some((u, p)),
                    _ => { eprintln!("--team-create requires UNIVERSE_ID PLACE_ID"); std::process::exit(2); }
                }
            }
            "--open-roblox" => {
                let v = args.next().unwrap_or_default();
                match rustudio_account::parse_place_ref(&v) {
                    Ok(p) => open_roblox = Some(p.place_id),
                    Err(e) => {
                        eprintln!("--open-roblox: {e}");
                        std::process::exit(2);
                    }
                }
            }
            "--local-copy" => local_copy = true,
            "--roblox-picker" => {
                let v = args.next().unwrap_or_default();
                match cloud_ui::PickerTab::parse(&v) {
                    Some(t) => roblox_picker = Some(t),
                    // File > Experience Settings instead of the picker.
                    None if v == "settings" => roblox_settings = true,
                    None => {
                        eprintln!("--roblox-picker: {v:?} (recent, mine, groups, shared or settings)");
                        std::process::exit(2);
                    }
                }
            }
            "--demo" => demo = true,
            "--play" => start_play = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            _ if a.starts_with("--") => {
                eprint!("unknown option {a}\n\n{USAGE}");
                std::process::exit(2);
            }
            _ => open = Some(PathBuf::from(a)),
        }
    }
    // The screenshot shows the editor with something in it to look at.
    if screenshot.is_some() && open.is_none() {
        demo = true;
    }

    let harness = app::Harness {
        shot_after,
        log_out,
        actions,
        started: std::time::Instant::now(),
        screenshot,
        selftest: selftest.map(selftest::SelfTest::new),
        ui_shots: ui_shots.map(ui_shots::UiShots::new),
        screenshot_requested: false,
        start_play,
        camera,
        shift: false,
        shots,
        shot_next: 0,
        shot_lines: Vec::new(),
        from_play,
        play_started: None,
        frame_ms: Vec::new(),
        last_frame: None,
        open_roblox: open_roblox.map(|p| (p, local_copy)),
        team_create,
        roblox_picker,
        roblox_settings,
        ui_due: Vec::new(),
        bench: bench.map(|out| {
            let label = open
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "new place".into());
            bench::Bench::new(Some(out), &label, start_play, seconds)
        }),
    };
    // No vsync when measuring, so frame times show the real cost.
    let bench_mode = harness.bench.is_some() || harness.selftest.is_some();
    let argv: Vec<String> = std::env::args().collect();
    let size = match window_arg(&argv) {
        Some((w, h)) => [w as f32, h as f32],
        None if harness.bench.is_some() => [1265.0, 900.0],
        // The self-test's lighting references were read in a 952×519 view
        // (rounds 7–15); round 16's chrome is 25 px shorter, so 33 px less window.
        None if harness.selftest.is_some() => [1265.0, 867.0],
        None => [1600.0, 900.0],
    };
    // The saved window (src/layout_store.rs), for plain interactive runs.
    let saved_layout = layout_store::enabled(&harness).then(layout_store::load).flatten();
    let icon = egui::IconData {
        rgba: icons::logo_rgba(64),
        width: 64,
        height: 64,
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("rustudio")
        .with_app_id("rustudio")
        .with_inner_size(size)
        .with_min_inner_size([900.0, 560.0])
        .with_icon(icon);
    if let Some(l) = &saved_layout {
        viewport = l.window_builder(viewport);
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: {
                let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
                let descriptor = setup.device_descriptor.clone();
                setup.device_descriptor = std::sync::Arc::new(move |adapter| {
                    let mut d = descriptor(adapter);
                    d.required_limits.max_sampled_textures_per_shader_stage = 24;
                    d
                });
                egui_wgpu::WgpuSetup::CreateNew(setup)
            },
            surface: egui_wgpu::SurfaceConfig {
                present_mode: if bench_mode {
                    egui_wgpu::wgpu::PresentMode::AutoNoVsync
                } else {
                    egui_wgpu::wgpu::PresentMode::AutoVsync
                },
                ..egui_wgpu::SurfaceConfig::LOW_LATENCY
            },
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "rustudio",
        options,
        Box::new(move |cc| Ok(Box::new(app::RustudioApp::new(cc, open, harness, demo)))),
    )
}
