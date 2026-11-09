//! The fidelity gate: a real playthrough of Place A judged against real Studio.
//! rustudio runs Place A the way tools/fidelity does (its own process, signed in,
//! the viewport 863×675 like Studio's hidden window), started on the
//! self-test's first frame so it runs alongside the other steps. The lobby
//! at 10 s is compared with Studio's own 10 s shot
//! (checks/compare/place_a/studio/shot-10s.png, corpus.json's max_mae); then it
//! presses START GAME and plays a match: walking (W) and firing (clicks),
//! with tools/fidelity/play probes reporting the round, the enemies and the
//! player. The step fails on any error line in Output, an empty view, the
//! pixel gate, or a match that doesn't start, spawn enemies, let the player
//! move and shoot, and run 60 s.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use super::{SelfTest, Step};
use crate::app::Editor;
use crate::mech;

const CORPUS: &str = include_str!("../../tools/fidelity/corpus.json");
const STUDIO_SHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../checks/compare/place_a/studio/shot-10s.png"
);
/// tools/fidelity's rustudio window: an 863×675 viewport, as Studio's.
const WINDOW: &str = "1148x1072"; // viewport 863x675 (round 16 chrome; was 1148x1105)
const SHOT_AT_S: f32 = 10.0;
/// The match: START GAME at 12 s; the probes watch WATCH_S of play from
/// round 1 (the self-test has 90 s in all; tools/fidelity/play/place_a_match.py
/// runs the full 60 s).
const START_GAME_AT: (u32, u32, f32) = (110, 143, 12.0);
const WATCH_S: u32 = 18;
const RUN_S: f32 = 74.0;
const TIMEOUT_S: f32 = 95.0;
const SERVER_PROBE: &str = include_str!("../../tools/fidelity/play/place-a-server.luau");
const CLIENT_PROBE: &str = include_str!("../../tools/fidelity/play/place-a-client.luau");
/// tools/fidelity's comparison: Studio's device bar cut off, both images on
/// one aspect, 480 px wide, per-channel mean absolute difference.
const STUDIO_TOP_CROP: u32 = 38;
const COMPARE_WIDTH: u32 = 480;

#[derive(Default)]
pub(super) struct FidTest {
    child: Option<Child>,
    dir: PathBuf,
    t0: Option<Instant>,
    /// Why it didn't start (reported at phase 92).
    skip: Option<String>,
}

fn corpus_place_a() -> Option<(PathBuf, f64)> {
    let v: serde_json::Value = serde_json::from_str(CORPUS).ok()?;
    let place_a = v.get("place_a")?;
    let path = place_a.get("path")?.as_str()?;
    let path = match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME")?).join(rest),
        None => PathBuf::from(path),
    };
    Some((path, place_a.get("max_mae")?.as_f64()?))
}

fn rgb(path: &Path) -> Result<image::RgbImage, String> {
    image::open(path)
        .map(|i| i.to_rgb8())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Both images cropped to one aspect and scaled to COMPARE_WIDTH.
fn common(a: image::RgbImage, b: image::RgbImage) -> (image::RgbImage, image::RgbImage) {
    use image::imageops::{crop_imm, resize, FilterType};
    // Studio's device bar lies over the top of its view: the same rows go
    // from both (round 21; fidelity.py common()).
    let cb = (STUDIO_TOP_CROP as f32 * b.height() as f32 / a.height().max(1) as f32).round() as u32;
    let a = crop_imm(&a, 0, STUDIO_TOP_CROP, a.width(), a.height() - STUDIO_TOP_CROP).to_image();
    let b = crop_imm(&b, 0, cb, b.width(), b.height().saturating_sub(cb)).to_image();
    let r = (a.width() as f32 / a.height() as f32).min(b.width() as f32 / b.height() as f32);
    let fit = |img: &image::RgbImage| {
        let w = ((img.height() as f32 * r) as u32).min(img.width());
        crop_imm(img, (img.width() - w) / 2, 0, w, img.height()).to_image()
    };
    let size = (COMPARE_WIDTH, (COMPARE_WIDTH as f32 / r) as u32);
    (
        resize(&fit(&a), size.0, size.1, FilterType::Triangle),
        resize(&fit(&b), size.0, size.1, FilterType::Triangle),
    )
}

fn mae(a: &image::RgbImage, b: &image::RgbImage) -> f64 {
    let sum: u64 = a
        .as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64)
        .sum();
    sum as f64 / a.as_raw().len().max(1) as f64
}

/// Empty or near-empty: almost no detail (luma deviation under 6), or one
/// colour (in 16-level bins) covering over 90 % of the view.
fn empty_view(img: &image::RgbImage) -> (bool, f64, f64) {
    let n = (img.width() * img.height()).max(1) as f64;
    let luma: Vec<f64> = img
        .pixels()
        .map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64)
        .collect();
    let mean = luma.iter().sum::<f64>() / n;
    let std = (luma.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / n).sqrt();
    let mut bins = std::collections::HashMap::new();
    for p in img.pixels() {
        *bins.entry([p[0] / 16, p[1] / 16, p[2] / 16]).or_insert(0u64) += 1;
    }
    let top = bins.values().copied().max().unwrap_or(0) as f64 / n;
    (std < 6.0 || top > 0.9, std, top)
}

impl SelfTest {
    fn fid_done(&mut self, passed: bool, detail: String) {
        self.steps.push(Step {
            step: "fidelity-place_a",
            passed,
            detail,
        });
        self.r4.phase = 96;
    }

    /// Starts the Place A run (frame 1); `fid_done` / phase 92 judge it later.
    pub(super) fn fid_spawn(&mut self) {
        let Some((place, _)) = corpus_place_a() else {
            self.fid.skip = Some("tools/fidelity/corpus.json has no place_a entry".into());
            return;
        };
        if !place.exists() || !Path::new(STUDIO_SHOT).exists() {
            self.fid.skip = Some(format!(
                "skipped: needs {} and Studio's reference shot {STUDIO_SHOT} (tools/fidelity/fidelity.py run --places place_a)",
                place.display()
            ));
            return;
        }
        let dir = std::env::temp_dir().join("rustudio-selftest-fidelity");
        let _ = std::fs::create_dir_all(&dir);
        for f in ["output.log", "place-a-server.luau", "place-a-client.luau"] {
            let _ = std::fs::write(dir.join(f), "");
        }
        let watch = |src: &str, from: &str| src.replace(from, &format!("local WATCH = {WATCH_S}"));
        let _ = std::fs::write(dir.join("place-a-server.luau"), watch(SERVER_PROBE, "local WATCH = 60"));
        let _ = std::fs::write(dir.join("place-a-client.luau"), watch(CLIENT_PROBE, "local WATCH = 25"));
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("rustudio"));
        let mut cmd = Command::new(exe);
        cmd.arg(&place)
            .args(["--play", "--from-play", "--window", WINDOW])
            .args(["--shots", &format!("{SHOT_AT_S},{RUN_S}")])
            .args(["--click", &format!("{},{}@{}", START_GAME_AT.0, START_GAME_AT.1, START_GAME_AT.2)])
            .arg("--luau")
            .arg("server")
            .arg(format!("{}@14", dir.join("place-a-server.luau").display()))
            .arg("--luau")
            .arg("client")
            .arg(format!("{}@14", dir.join("place-a-client.luau").display()))
            // Walk in bursts across the window the character may spawn in
            // (round 27: the faster nav build moved the spawn from ~50 s to
            // ~27 s, before the old single W press).
            .args(["--key", "W@30:2", "--key", "W@38:2", "--key", "W@46:2", "--key", "W@54:3", "--key", "W@62:2"]);
        // Fire every second once the match may be up (the character spawns
        // 45–55 s in; a player downed before firing would leave the clip full).
        for t in 44..=(RUN_S as u32 - 4) {
            cmd.args(["--click", &format!("431,337@{t}")]);
        }
        let child = cmd
            .arg("--screenshot")
            .arg(dir.join("shot.png"))
            .arg("--log")
            .arg(dir.join("output.log"))
            // As a person runs it: the real caches and content; silent
            // (the person can hear test runs; sounds still keep their time).
            .env_remove("RUSTUDIO_CACHE_DIR")
            .env_remove("RUSTUDIO_CONTENT_DIR")
            .env("RUSTUDIO_NO_AUDIO", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match child {
            Ok(c) => {
                self.fid.child = Some(c);
                self.fid.dir = dir;
                self.fid.t0 = Some(Instant::now());
            }
            Err(e) => self.fid.skip = Some(format!("FAIL: couldn't start the playthrough: {e}")),
        }
    }

    pub(super) fn tick_fidelity(&mut self, _f: u64, _ed: &mut Editor) -> bool {
        if let Some(why) = self.fid.skip.take() {
            let failed = why.starts_with("FAIL") || why.contains("no place_a entry");
            self.fid_done(!failed, why);
            return false;
        }
        let waited = self.fid.t0.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
        let status = match self.fid.child.as_mut().map(|c| c.try_wait()) {
            Some(Ok(Some(s))) => s,
            Some(Ok(None)) if waited < TIMEOUT_S => return false,
            _ => {
                if let Some(c) = self.fid.child.as_mut() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                self.fid_done(false, format!("the Place A playthrough didn't finish in {TIMEOUT_S} s"));
                return false;
            }
        };
        self.fid.child = None;
        match self.judge(status.success(), waited) {
            Ok((passed, detail)) => self.fid_done(passed, detail),
            Err(e) => self.fid_done(false, e),
        }
        false
    }

    fn judge(&self, exited_ok: bool, secs: f32) -> Result<(bool, String), String> {
        let (_, max_mae) = corpus_place_a().ok_or("no place_a entry in corpus.json")?;
        let dir = &self.fid.dir;
        let log = std::fs::read_to_string(dir.join("output.log"))
            .map_err(|e| format!("no Output log from the playthrough: {e}"))?;
        let errors: Vec<&str> = log.lines().filter(|l| l.starts_with("Error:")).collect();
        let shot = dir.join(format!("shot-{SHOT_AT_S}s.view.png"));
        let ours = rgb(&shot)?;
        let (empty, std, top) = empty_view(&ours);
        let (a, b) = common(rgb(Path::new(STUDIO_SHOT))?, ours);
        let diff = mae(&a, &b);
        let json_after = |tag: &str| -> Option<serde_json::Value> {
            let line = log.lines().find(|l| l.contains(tag))?;
            let start = line.find(tag)? + tag.len();
            let body = &line[start..];
            let end = body.rfind('}')? + 1;
            serde_json::from_str(&body[..end]).ok()
        };
        let m = json_after("MATCH|").unwrap_or_default();
        let p = json_after("PLAYER|").unwrap_or_default();
        let f = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(-1.0);
        let started = m.get("started").and_then(|x| x.as_bool()) == Some(true);
        let enemies = f(&m, "peakAlive");
        let moved = f(&p, "moved");
        let (c0, c1) = (f(&p, "clipBefore"), f(&p, "clipAfter"));
        let shot_ok = c0 >= 0.0 && c1 >= 0.0 && c1 < c0;
        let match_ok = started && enemies >= 1.0 && moved >= mech::PLACE_A_GATE_MIN_MOVE as f64 && shot_ok;
        let passed = exited_ok && errors.is_empty() && !empty && diff <= max_mae && match_ok;
        Ok((
            passed,
            format!(
                "Place A in its own process ({secs:.0} s): {} error line(s) in Output{}; lobby at {SHOT_AT_S} s: viewport {} (luma deviation {std:.1}, top colour {:.0} %), pixel MAE {diff:.1} against Studio's (allowed {max_mae}); match: {} ({:.0} s of play watched; round 1 active {:.0} s after the probe began; enemies alive at once, peak {enemies:.0}; round {:.0}; downed {}), player walked {moved:.1} studs (need {}) and fired the clip {c0:.0} → {c1:.0}{}",
                errors.len(),
                errors
                    .first()
                    .map(|e| format!(", first: {}", e.chars().take(160).collect::<String>()))
                    .unwrap_or_default(),
                if empty { "EMPTY" } else { "has content" },
                top * 100.0,
                if started { "started" } else { "DID NOT START" },
                f(&m, "watched"),
                f(&m, "activeAfter"),
                f(&m, "round"),
                m.get("downed").and_then(|x| x.as_bool()).unwrap_or(false),
                mech::PLACE_A_GATE_MIN_MOVE,
                if exited_ok { "" } else { "; the playthrough exited with an error" }
            ),
        ))
    }
}
