//! Crash reports: a panic (any thread) or a fatal signal writes
//! `~/.cache/rustudio/crash/<timestamp>-<pid>.txt` with a backtrace, and the
//! next start lists any report it hasn't mentioned yet in Output. A crash is
//! never silent, even when rustudio was started from a launcher with no
//! terminal.

use std::backtrace::Backtrace;
use std::ffi::CString;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Where reports go.
pub fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("rustudio").join("crash")
}

/// The report file named for this process (made at install, so the signal
/// handler needs no allocation to find it).
static SIGNAL_PATH: OnceLock<CString> = OnceLock::new();
/// The first fatal event writes the report; a panic that then aborts must
/// not overwrite its own report with the abort's.
static WRITTEN: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Inside `recoverable`: a panic here is caught and handled, not a crash.
    static RECOVERING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Run `f`, catching a panic in it (the caller falls back). Such a panic is
/// expected, so it writes no crash report.
pub fn recoverable<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    RECOVERING.with(|r| r.set(r.get() + 1));
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    RECOVERING.with(|r| r.set(r.get() - 1));
    out
}

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Command-line arguments with anything that looks like a credential removed.
fn args_line() -> String {
    std::env::args()
        .map(|a| {
            if a.contains("ROBLOSECURITY") || a.contains("WARNING:-DO-NOT-SHARE") || a.len() > 200 {
                "<redacted>".to_string()
            } else {
                a
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn header(kind: &str) -> String {
    format!(
        "rustudio {} ({}) crashed: {kind}\npid {}\nargs: {}\n\n",
        env!("CARGO_PKG_VERSION"),
        option_env!("RUSTUDIO_GIT").unwrap_or("unknown commit"),
        std::process::id(),
        args_line()
    )
}

fn write_report(body: &str) -> Option<PathBuf> {
    let d = dir();
    std::fs::create_dir_all(&d).ok()?;
    let path = d.join(format!("{}-{}.txt", stamp(), std::process::id()));
    let mut f = std::fs::File::create(&path).ok()?;
    f.write_all(body.as_bytes()).ok()?;
    Some(path)
}

/// Install the panic hook and the fatal-signal handlers. Call first in main.
pub fn install() {
    let path = dir().join(format!("{}-{}.txt", stamp(), std::process::id()));
    let _ = SIGNAL_PATH.set(CString::new(path.to_string_lossy().as_bytes()).unwrap_or_default());

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if RECOVERING.with(|r| r.get()) > 0 {
            return;
        }
        // Write first, then let the default hook print to stderr as before.
        if !WRITTEN.swap(true, Ordering::SeqCst) {
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>").to_string();
            let msg = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".into());
            let loc = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_default();
            let bt = Backtrace::force_capture();
            let body = format!(
                "{}panic in thread '{name}' at {loc}:\n{msg}\n\nbacktrace:\n{bt}\n",
                header("panic")
            );
            if let Some(p) = write_report(&body) {
                eprintln!("rustudio: crash report written to {}", p.display());
            }
        }
        default_hook(info);
    }));

    let _ = std::fs::create_dir_all(dir());
    // SAFETY: plain sigaction setup; the handler below only uses
    // async-signal-safe calls until it has written the basic note.
    unsafe {
        // An alternate stack, so a stack overflow can still run the handler.
        let size = 64 * 1024;
        let stack = libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        if stack != libc::MAP_FAILED {
            let ss = libc::stack_t {
                ss_sp: stack,
                ss_flags: 0,
                ss_size: size,
            };
            libc::sigaltstack(&ss, std::ptr::null_mut());
        }
        for sig in [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE, libc::SIGABRT] {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as *const () as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK | libc::SA_RESETHAND;
            libc::sigemptyset(&mut sa.sa_mask);
            let mut old: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(sig, &sa, &mut old) == 0 {
                OLD[slot(sig)] = old;
            }
        }
    }
}

static mut OLD: [libc::sigaction; 5] = unsafe { std::mem::zeroed() };

fn slot(sig: libc::c_int) -> usize {
    match sig {
        libc::SIGSEGV => 0,
        libc::SIGBUS => 1,
        libc::SIGILL => 2,
        libc::SIGFPE => 3,
        _ => 4,
    }
}

fn sig_name(sig: libc::c_int) -> &'static [u8] {
    match sig {
        libc::SIGSEGV => b"SIGSEGV (invalid memory access)",
        libc::SIGBUS => b"SIGBUS",
        libc::SIGILL => b"SIGILL",
        libc::SIGFPE => b"SIGFPE",
        _ => b"SIGABRT (abort)",
    }
}

extern "C" fn on_signal(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    // SAFETY: async-signal-safe calls (open/write/close/alarm/sigaction) for
    // the note; the backtrace after it is best effort under a 5 s alarm, so
    // a lock held by the faulting thread turns into an exit, not a hang.
    unsafe {
        if !WRITTEN.swap(true, Ordering::SeqCst) {
            if let Some(p) = SIGNAL_PATH.get() {
                let fd = libc::open(p.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, 0o644);
                if fd >= 0 {
                    let w = |b: &[u8]| {
                        libc::write(fd, b.as_ptr().cast(), b.len());
                    };
                    w(b"rustudio crashed: fatal signal ");
                    w(sig_name(sig));
                    w(b"\n\n");
                    libc::alarm(5);
                    // Not async-signal-safe; we are dying anyway.
                    let body = format!("{}backtrace:\n{}\n", header("signal"), Backtrace::force_capture());
                    w(body.as_bytes());
                    libc::close(fd);
                    libc::alarm(0);
                }
            }
        }
        // Hand the signal to whoever had it before (Rust's stack-overflow
        // reporter, or the default action that dumps core).
        let old = &raw const OLD;
        let old = &(*old)[slot(sig)];
        libc::sigaction(sig, old, std::ptr::null_mut());
        if old.sa_sigaction != libc::SIG_DFL && old.sa_sigaction != libc::SIG_IGN {
            if old.sa_flags & libc::SA_SIGINFO != 0 {
                let f: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) =
                    std::mem::transmute(old.sa_sigaction);
                f(sig, info, ctx);
            } else {
                let f: extern "C" fn(libc::c_int) = std::mem::transmute(old.sa_sigaction);
                f(sig);
            }
            return;
        }
        if sig == libc::SIGABRT {
            libc::raise(sig);
        }
        // Returning re-runs the faulting instruction under the default action.
    }
}

/// Reports from earlier runs not yet shown, oldest first, as Output lines.
/// Each is shown once (`.shown` remembers them).
pub fn unseen_reports() -> Vec<String> {
    let d = dir();
    let seen_file = d.join(".shown");
    let seen = std::fs::read_to_string(&seen_file).unwrap_or_default();
    let mine = SIGNAL_PATH
        .get()
        .and_then(|p| p.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let mut found: Vec<PathBuf> = std::fs::read_dir(&d)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.len() > 16
                && n.ends_with(".txt")
                && n.as_bytes()[..8].iter().all(u8::is_ascii_digit)
                && n.as_bytes()[8] == b'-'
                && !seen.lines().any(|l| l == n)
                && p.to_str() != Some(mine.as_str())
        })
        .collect();
    found.sort();
    let mut lines = Vec::new();
    let mut add = String::new();
    for p in found {
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        // The first line after the header that says what happened.
        let what = text
            .lines()
            .find(|l| l.starts_with("panic in thread") || l.starts_with("rustudio crashed: fatal"))
            .unwrap_or("crash")
            .to_string();
        let msg = text
            .lines()
            .skip_while(|l| !l.starts_with("panic in thread"))
            .nth(1)
            .map(|m| format!(" — {m}"))
            .unwrap_or_default();
        lines.push(format!(
            "rustudio crashed last time: {what}{msg}. Report: {}",
            p.display()
        ));
        if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
            add.push_str(n);
            add.push('\n');
        }
    }
    if !add.is_empty() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&seen_file) {
            let _ = f.write_all(add.as_bytes());
        }
    }
    lines
}
