//! Frame-time measurement: `--bench OUT.json` and the self-test's perf step,
//! plus a tiny section profiler (`prof::scope`) whose totals go in the report.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Instant;

thread_local! {
    static SECTIONS: RefCell<Vec<(&'static str, f64, u64)>> = const { RefCell::new(Vec::new()) };
}

/// Adds the time until the guard drops to section `name`.
pub struct Scope {
    name: &'static str,
    t0: Instant,
    i0: u64,
}

pub fn scope(name: &'static str) -> Scope {
    Scope {
        name,
        t0: Instant::now(),
        i0: if instr_on() { instructions() } else { 0 },
    }
}

/// RUSTUDIO_BENCH_INSTR=1: sections count the thread's user-space
/// instructions (millions) instead of milliseconds: a measure of work that
/// a loaded machine doesn't move (comparing builds while other jobs run).
fn instr_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTUDIO_BENCH_INSTR").is_some())
}

/// This thread's user-space instructions retired so far (perf_event_open
/// on itself; 0 if unavailable).
#[cfg(target_os = "linux")]
fn instructions() -> u64 {
    thread_local! {
        static FD: i32 = unsafe {
            // struct perf_event_attr: type, size, config, …, flags.
            let mut attr = [0u64; 16];
            let a = attr.as_mut_ptr() as *mut u32;
            *a = 0; // PERF_TYPE_HARDWARE
            *a.add(1) = 128; // size
            attr[1] = 1; // PERF_COUNT_HW_INSTRUCTIONS
            attr[5] = (1 << 5) | (1 << 6); // exclude_kernel | exclude_hv
            libc::syscall(libc::SYS_perf_event_open, attr.as_ptr(), 0, -1, -1, 0) as i32
        };
    }
    FD.with(|fd| {
        if *fd < 0 {
            return 0;
        }
        let mut v = 0u64;
        unsafe { libc::read(*fd, &mut v as *mut u64 as *mut libc::c_void, 8) };
        v
    })
}

#[cfg(not(target_os = "linux"))]
fn instructions() -> u64 {
    0
}

impl Drop for Scope {
    fn drop(&mut self) {
        let ms = if instr_on() {
            instructions().saturating_sub(self.i0) as f64 / 1e6
        } else {
            self.t0.elapsed().as_secs_f64() * 1000.0
        };
        SECTIONS.with(|s| {
            let mut s = s.borrow_mut();
            match s.iter_mut().find(|e| std::ptr::eq(e.0, self.name) || e.0 == self.name) {
                Some(e) => {
                    e.1 += ms;
                    e.2 += 1;
                }
                None => s.push((self.name, ms, 1)),
            }
        });
    }
}

// ------------------------------------------------------------ Lua timing

thread_local! {
    /// On while a bench runs: Lua time per connection / thread.
    static LUA_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Resumes in progress (only the outermost one is timed).
    static LUA_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Key (function / thread pointer) → (label, ms, calls).
    static LUA: RefCell<std::collections::HashMap<usize, (String, f64, u64)>> =
        RefCell::new(std::collections::HashMap::new());
}

pub fn lua_prof_on() -> bool {
    LUA_ON.with(|c| c.get())
}

/// Times one resume of Lua code under `key`, labelled by `label` the first
/// time the key is seen. Nested resumes count in the outermost only.
pub fn lua_timed<R>(key: usize, label: impl FnOnce() -> String, f: impl FnOnce() -> R) -> R {
    let outer = LUA_DEPTH.with(|d| {
        d.set(d.get() + 1);
        d.get() == 1
    });
    let t0 = Instant::now();
    let r = f();
    LUA_DEPTH.with(|d| d.set(d.get() - 1));
    if outer {
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        LUA.with(|m| {
            let mut m = m.borrow_mut();
            let e = m.entry(key).or_insert_with(|| (label(), 0.0, 0));
            if ms > 20.0 && hitch_on() {
                eprintln!("HITCH lua {ms:.1} ms {}", e.0);
            }
            e.1 += ms;
            e.2 += 1;
        });
    }
    r
}

/// Whether an engine call has per-call diagnostics to keep (RUSTUDIO_HITCH,
/// RUSTUDIO_API_TOTALS, RUSTUDIO_API_SITES): the runtime's fast member
/// reads (runtime_routes.rs) leave those calls to the full path.
pub fn call_probes_on() -> bool {
    hitch_on() || totals_on() || sites_on()
}

/// RUSTUDIO_API_TOTALS=1: every engine call's time, summed per thread and
/// printed (top 15) to stderr every 5 s — where a slow script's time goes.
fn totals_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTUDIO_API_TOTALS").is_some())
}

thread_local! {
    static TOTALS: RefCell<(Option<Instant>, std::collections::HashMap<String, (f64, u64)>)> =
        RefCell::new((None, std::collections::HashMap::new()));
}

fn add_total(kind: &str, name: &str, ms: f64) {
    TOTALS.with(|t| {
        let mut t = t.borrow_mut();
        let e = t.1.entry(format!("{kind} {name}")).or_insert((0.0, 0));
        e.0 += ms;
        e.1 += 1;
        let last = *t.0.get_or_insert_with(Instant::now);
        if last.elapsed().as_secs_f64() >= 5.0 {
            t.0 = Some(Instant::now());
            let mut v: Vec<_> = t.1.iter().map(|(k, (ms, n))| (k.clone(), *ms, *n)).collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            let th = std::thread::current().name().unwrap_or("?").to_string();
            for (k, ms, n) in v.into_iter().take(15) {
                eprintln!("APITOTAL [{th}] {ms:10.1} ms {n:9}x {k}");
            }
            SECTIONS.with(|s| {
                for (k, ms, n) in s.borrow().iter() {
                    eprintln!("APISECT [{th}] {ms:10.1} ms {n:9}x {k}");
                }
            });
        }
    });
}

thread_local! {
    /// RUSTUDIO_HITCH: engine calls since the last `hitch_api_take`.
    static HITCH_API: RefCell<std::collections::HashMap<String, (f64, u64)>> =
        RefCell::new(std::collections::HashMap::new());
}

/// RUSTUDIO_HITCH=1: print every Lua resume over 10 ms (with the engine
/// calls inside one over 300 ms) and every engine call over 5 ms, to find
/// hitches (round 20). Read once.
pub fn hitch_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTUDIO_HITCH").is_some())
}

/// RUSTUDIO_HITCH: the engine calls gathered since last time, slowest first.
pub fn hitch_api_take() -> Vec<(String, f64, u64)> {
    let mut v: Vec<(String, f64, u64)> = HITCH_API
        .with(|m| std::mem::take(&mut *m.borrow_mut()))
        .into_iter()
        .map(|(k, (ms, n))| (k, ms, n))
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

thread_local! {
    /// Engine API time while a bench runs: name hash → (label, ms, calls).
    static API: RefCell<std::collections::HashMap<u64, (String, f64, u64)>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Times one engine API call from Lua (`kind` index / newindex / call).
pub fn api_timed<R>(kind: &'static str, name: &str, f: impl FnOnce() -> R) -> R {
    let hitch = hitch_on();
    let totals = totals_on();
    if !lua_prof_on() && !hitch && !totals {
        return f();
    }
    let t0 = Instant::now();
    let r = f();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if totals {
        add_total(kind, name, ms);
    }
    if hitch && ms > 5.0 {
        eprintln!("HITCH api {ms:.1} ms {kind} {name}");
    }
    if hitch {
        HITCH_API.with(|m| {
            let mut m = m.borrow_mut();
            let e = m.entry(format!("{kind} {name}")).or_insert((0.0, 0));
            e.0 += ms;
            e.1 += 1;
        });
    }
    if !lua_prof_on() {
        return r;
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (kind, name).hash(&mut h);
    API.with(|m| {
        let mut m = m.borrow_mut();
        let e = m.entry(h.finish()).or_insert_with(|| (format!("{kind} {name}"), 0.0, 0));
        e.1 += ms;
        e.2 += 1;
    });
    r
}

thread_local! {
    /// RUSTUDIO_API_SITES: "kind member @ chunk:line" → calls, while a bench measures.
    static SITES: RefCell<std::collections::HashMap<String, u64>> =
        RefCell::new(std::collections::HashMap::new());
}

/// RUSTUDIO_API_SITES=FILE: with `--bench`, counts every engine call by the
/// script line that made it (the nearest Lua frame, and the nearest game
/// frame when that one is rustudio's own library), written to FILE (one file
/// per Luau state: FILE.<thread name>.<n>) when the measurement ends. Slow
/// (walks the Lua stack on every call): for finding who calls what, not timing.
pub fn sites_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUSTUDIO_API_SITES").is_some())
}

/// Counts one engine call from `lua`'s running code (see `sites_on`).
pub fn site(lua: &mlua::Lua, kind: &'static str, name: &str) {
    if !sites_on() || !lua_prof_on() {
        return;
    }
    let frame = |level: usize| {
        lua.inspect_stack(level, |d| {
            let src = d
                .source()
                .short_src
                .map(|s| s.trim_start_matches(['=', '@']).to_string())
                .unwrap_or_default();
            (src, d.current_line().unwrap_or(0))
        })
    };
    let mut near: Option<(String, usize)> = None;
    let mut game: Option<(String, usize)> = None;
    for level in 0..24 {
        let Some((src, line)) = frame(level) else { break };
        if src.is_empty() || src == "[C]" {
            continue;
        }
        if near.is_none() {
            near = Some((src.clone(), line as usize));
        }
        if !src.starts_with("rustudio.") {
            game = Some((src, line as usize));
            break;
        }
    }
    let fmt = |f: &Option<(String, usize)>| match f {
        Some((s, l)) => format!("{s}:{l}"),
        None => "(engine)".to_string(),
    };
    let key = if near == game {
        format!("{kind} {name} @ {}", fmt(&near))
    } else {
        format!("{kind} {name} @ {} <- {}", fmt(&near), fmt(&game))
    };
    SITES.with(|m| *m.borrow_mut().entry(key).or_insert(0) += 1);
}

fn write_sites(frames: usize) {
    let Some(base) = std::env::var_os("RUSTUDIO_API_SITES") else { return };
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut v: Vec<(String, u64)> = SITES.with(|m| std::mem::take(&mut *m.borrow_mut())).into_iter().collect();
    if v.is_empty() {
        return;
    }
    v.sort_by(|a, b| b.1.cmp(&a.1));
    let frames = frames.max(1) as f64;
    let text: String = v.iter().map(|(k, n)| format!("{n:9} {:9.2}/f  {k}\n", *n as f64 / frames)).collect();
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let _ = std::fs::write(format!("{}.{n}", base.to_string_lossy()), text);
}

fn take_api() -> Vec<(String, f64, u64)> {
    let mut v: Vec<(String, f64, u64)> = API.with(|m| std::mem::take(&mut *m.borrow_mut())).into_values().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

/// Lua totals by label, slowest first: (label, ms, calls).
fn take_lua() -> Vec<(String, f64, u64)> {
    let m = LUA.with(|m| std::mem::take(&mut *m.borrow_mut()));
    let mut by: std::collections::HashMap<String, (f64, u64)> = std::collections::HashMap::new();
    for (_, (l, ms, n)) in m {
        let e = by.entry(l).or_default();
        e.0 += ms;
        e.1 += n;
    }
    let mut v: Vec<(String, f64, u64)> = by.into_iter().map(|(l, (ms, n))| (l, ms, n)).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

/// Adds `ms` and `calls` to section `name` without a guard (for totals
/// gathered in a loop, or counts with `ms` 0).
pub fn add(name: &'static str, ms: f64, calls: u64) {
    SECTIONS.with(|s| {
        let mut s = s.borrow_mut();
        match s.iter_mut().find(|e| std::ptr::eq(e.0, name) || e.0 == name) {
            Some(e) => {
                e.1 += ms;
                e.2 += calls;
            }
            None => s.push((name, ms, calls)),
        }
    });
}

/// Section `name`'s running total (ms, calls) since the last bench report
/// took the sections (Stats.PhysicsStepTime reads its deltas).
pub fn section_total(name: &str) -> (f64, u64) {
    SECTIONS.with(|s| {
        s.borrow().iter().find(|e| e.0 == name).map(|e| (e.1, e.2)).unwrap_or((0.0, 0))
    })
}

fn take_sections() -> Vec<(&'static str, f64, u64)> {
    SECTIONS.with(|s| std::mem::take(&mut *s.borrow_mut()))
}

/// CPU time this thread has used, in ms (not the time it waited for a core:
/// on a busy machine frame times stretch, this much less so).
fn thread_cpu_ms() -> f64 {
    #[cfg(unix)]
    {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: a valid timespec out-pointer and a clock id the OS defines.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) } == 0 {
            return ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6;
        }
    }
    0.0
}

/// Collects frame intervals for a fixed time after a warm-up.
pub struct Bench {
    pub out: Option<PathBuf>,
    pub label: String,
    pub play: bool,
    pub seconds: f32,
    pub warmup: f32,
    started: Option<Instant>,
    measuring: Option<Instant>,
    last: Option<Instant>,
    intervals: Vec<f32>,
    work: Vec<f32>,
    /// Section totals over the measured frames: name → (ms, calls).
    totals: Vec<(&'static str, f64, u64)>,
    /// The slowest frames: (frame ms, its sections).
    worst: Vec<(f32, Vec<(&'static str, f64)>)>,
    pub done: bool,
    /// Lua time by connection / thread over the measured frames.
    lua: Vec<(String, f64, u64)>,
    /// Engine API time by member over the measured frames.
    api: Vec<(String, f64, u64)>,
    /// The main thread's CPU time when measuring began, and over the window.
    cpu0: f64,
    cpu_ms: f64,
}

impl Bench {
    pub fn new(out: Option<PathBuf>, label: &str, play: bool, seconds: f32) -> Bench {
        Bench {
            out,
            label: label.to_string(),
            play,
            seconds,
            warmup: std::env::var("RUSTUDIO_BENCH_WARMUP")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(crate::mech::BENCH_WARMUP_S),
            started: None,
            measuring: None,
            last: None,
            intervals: Vec::new(),
            work: Vec::new(),
            totals: Vec::new(),
            worst: Vec::new(),
            done: false,
            lua: Vec::new(),
            api: Vec::new(),
            cpu0: 0.0,
            cpu_ms: 0.0,
        }
    }

    /// Called once per frame with the time the frame's own work took (ms).
    /// Returns true when the measurement window is over.
    pub fn frame(&mut self, work_ms: f32) -> bool {
        let now = Instant::now();
        let started = *self.started.get_or_insert(now);
        if self.measuring.is_none() {
            if now.duration_since(started).as_secs_f32() < self.warmup {
                self.last = Some(now);
                return false;
            }
            // This frame's sections are partly from before: it isn't counted.
            take_sections();
            take_lua();
            take_api();
            LUA_ON.with(|c| c.set(true));
            self.cpu0 = thread_cpu_ms();
            self.measuring = Some(now);
            self.last = Some(now);
            return false;
        }
        let mut frame_sections = take_sections();
        if let Some(last) = self.last {
            let ms = now.duration_since(last).as_secs_f32() * 1000.0;
            self.intervals.push(ms);
            self.work.push(work_ms);
            // Time between frames outside the frame's own work (present, GPU waits).
            frame_sections.push(("between_frames", (ms - work_ms).max(0.0) as f64, 1));
            for (n, t, c) in &frame_sections {
                match self.totals.iter_mut().find(|e| e.0 == *n) {
                    Some(e) => {
                        e.1 += t;
                        e.2 += c;
                    }
                    None => self.totals.push((n, *t, *c)),
                }
            }
            if self.worst.len() < 5 || ms > self.worst.last().map(|w| w.0).unwrap_or(0.0) {
                self.worst.push((
                    ms,
                    frame_sections.iter().map(|(n, t, _)| (*n, *t)).collect(),
                ));
                self.worst.sort_by(|a, b| b.0.total_cmp(&a.0));
                self.worst.truncate(5);
            }
        }
        self.last = Some(now);
        if now.duration_since(self.measuring.unwrap()).as_secs_f32() >= self.seconds {
            self.done = true;
            self.cpu_ms = thread_cpu_ms() - self.cpu0;
            LUA_ON.with(|c| c.set(false));
            self.lua = take_lua();
            self.api = take_api();
            write_sites(self.intervals.len());
        }
        self.done
    }

    pub fn report(&self) -> serde_json::Value {
        let stats = |v: &[f32]| {
            if v.is_empty() {
                return (0.0, 0.0);
            }
            let avg = v.iter().sum::<f32>() / v.len() as f32;
            let mut s = v.to_vec();
            s.sort_by(|a, b| b.total_cmp(a));
            let n = (s.len() / 100).max(1);
            let low = s[..n].iter().sum::<f32>() / n as f32;
            (avg, low)
        };
        let (avg, low) = stats(&self.intervals);
        let (wavg, wlow) = stats(&self.work);
        let frames = self.intervals.len().max(1) as f64;
        let sections: Vec<serde_json::Value> = {
            let mut s = self.totals.clone();
            s.sort_by(|a, b| b.1.total_cmp(&a.1));
            s.iter()
                .map(|(n, ms, c)| serde_json::json!({"name": n, "ms_per_frame": ms / frames, "calls": c}))
                .collect()
        };
        serde_json::json!({
            "label": self.label,
            "mode": if self.play { "play" } else { "edit" },
            "frames": self.intervals.len(),
            "avg_frame_ms": avg,
            "low1_frame_ms": low,
            "avg_fps": if avg > 0.0 { 1000.0 / avg } else { 0.0 },
            "low1_fps": if low > 0.0 { 1000.0 / low } else { 0.0 },
            "avg_work_ms": wavg,
            // The main thread's CPU time per frame: steadier than the wall
            // times above when other programs compete for the cores.
            "avg_cpu_ms": self.cpu_ms / frames,
            "low1_work_ms": wlow,
            "sections": sections,
            // Slowest Lua: "chunk:line event" per connection, "chunk:line"
            // per resumed thread (waits, spawns), "start Script" per script.
            // Slowest engine members called from Lua (index / newindex / call).
            "api": self
                .api
                .iter()
                .take(30)
                .map(|(l, ms, n)| serde_json::json!({"what": l, "ms_per_frame": ms / frames, "calls": n}))
                .collect::<Vec<_>>(),
            "lua": self
                .lua
                .iter()
                .take(30)
                .map(|(l, ms, n)| serde_json::json!({"what": l, "ms_per_frame": ms / frames, "calls": n}))
                .collect::<Vec<_>>(),
            "worst_frames": self
                .worst
                .iter()
                .map(|(ms, secs)| {
                    let mut secs = secs.clone();
                    secs.sort_by(|a, b| b.1.total_cmp(&a.1));
                    let parts: Vec<String> = secs.iter().take(6).map(|(n, t)| format!("{n} {t:.1}")).collect();
                    format!("{ms:.1} ms: {}", parts.join(", "))
                })
                .collect::<Vec<_>>(),
        })
    }
}
