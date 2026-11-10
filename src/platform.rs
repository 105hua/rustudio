//! What differs between desktops besides folders (src/paths.rs): process
//! statistics, the desktop trash, system fonts, shared library names.

use std::path::Path;

/// This process's resident memory in bytes, where the system tells it.
pub fn resident_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let pages: u64 = std::fs::read_to_string("/proc/self/statm").ok()?.split_whitespace().nth(1)?.parse().ok()?;
        // SAFETY: sysconf has no preconditions.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Some(pages * u64::try_from(page).ok()?)
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: task_info on our own task, into a buffer of the size given.
        unsafe {
            let mut info: libc::mach_task_basic_info = std::mem::zeroed();
            let mut count = (std::mem::size_of::<libc::mach_task_basic_info>() / std::mem::size_of::<libc::natural_t>())
                as libc::mach_msg_type_number_t;
            #[allow(deprecated)]
            let task = libc::mach_task_self();
            let r = libc::task_info(task, libc::MACH_TASK_BASIC_INFO, (&raw mut info).cast(), &mut count);
            (r == libc::KERN_SUCCESS).then_some(info.resident_size)
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// The 1-minute load average as text, "?" where unknown.
pub fn load_average() -> String {
    #[cfg(unix)]
    {
        let mut l = [0f64; 1];
        // SAFETY: getloadavg writes at most the one entry asked for.
        if unsafe { libc::getloadavg(l.as_mut_ptr(), 1) } == 1 {
            return format!("{:.2}", l[0]);
        }
    }
    "?".into()
}

/// Moves `path` to the desktop's own trash where rustudio uses it (macOS:
/// the Finder Trash, so Put Back works; Windows: the Recycle Bin, so Restore
/// works). None where the caller keeps the XDG trash itself
/// (project_write.rs).
pub fn native_trash(path: &Path) -> Option<Result<(), String>> {
    #[cfg(target_os = "macos")]
    {
        Some(trash_item(path).map(|_| ()))
    }
    #[cfg(windows)]
    {
        Some(recycle(path))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        None
    }
}

/// The shell's delete with undo: the item goes to the Recycle Bin. Where the
/// drive has no Recycle Bin (some network and removable drives) the shell
/// would delete it outright, so it asks first (FOF_WANTNUKEWARNING), and
/// answering No leaves the item where it is.
#[cfg(windows)]
fn recycle(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE,
        SHFILEOPSTRUCTW,
    };
    let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
    // pFrom is a list of paths, each ended by a NUL, the list by another.
    let from: Vec<u16> = abs.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_WANTNUKEWARNING) as u16,
        ..Default::default()
    };
    // SAFETY: `op` is fully initialised and `from` outlives the call.
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 {
        return Err(format!("the shell refused (error {r:#x})"));
    }
    if op.fAnyOperationsAborted != 0 {
        return Err("cancelled".into());
    }
    Ok(())
}

/// NSFileManager's trash: where the item ended up.
#[cfg(target_os = "macos")]
fn trash_item(path: &Path) -> Result<Option<std::path::PathBuf>, String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&abs.to_string_lossy()));
    let mut out = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut out))
        .map_err(|e| e.localizedDescription().to_string())?;
    Ok(out.and_then(|u| u.path()).map(|p| p.to_string().into()))
}

/// System fonts for the symbols and scripts rustudio's own fonts lack, in
/// the order to try them.
pub fn fallback_font_files() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/Apple Symbols.ttf", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"]
    } else {
        &[
            "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
            "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ]
    }
}

/// The names to dlopen a shared library by: `stem` without prefix or suffix
/// ("vpx"), `versions` the sonames to try first ("12" -> libvpx.so.12).
/// macOS also looks in Homebrew's and MacPorts' lib folders, which aren't on
/// the default search path.
#[cfg(unix)]
pub fn shared_library_names(stem: &str, versions: &[&str]) -> Vec<std::ffi::CString> {
    let mut v: Vec<String> = Vec::new();
    if cfg!(target_os = "macos") {
        for dir in ["", "/opt/homebrew/lib/", "/usr/local/lib/", "/opt/local/lib/"] {
            for ver in versions {
                v.push(format!("{dir}lib{stem}.{ver}.dylib"));
            }
            v.push(format!("{dir}lib{stem}.dylib"));
        }
    } else {
        for ver in versions {
            v.push(format!("lib{stem}.so.{ver}"));
        }
        v.push(format!("lib{stem}.so"));
    }
    v.into_iter().filter_map(|s| std::ffi::CString::new(s).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_stats() {
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            let rss = resident_bytes().expect("resident memory");
            assert!(rss > 1 << 20, "{rss}");
        }
        if cfg!(unix) {
            assert_ne!(load_average(), "?");
        }
    }

    #[cfg(unix)]
    #[test]
    fn library_names() {
        let n: Vec<String> = shared_library_names("vpx", &["9"]).iter().map(|c| c.to_string_lossy().into_owned()).collect();
        if cfg!(target_os = "macos") {
            assert!(n.contains(&"/opt/homebrew/lib/libvpx.dylib".to_string()), "{n:?}");
        } else {
            assert_eq!(n, ["libvpx.so.9", "libvpx.so"]);
        }
    }

    #[cfg(windows)]
    #[test]
    fn trash_goes_to_recycle_bin() {
        let dir = std::env::temp_dir().join(format!("rustudio-recycle-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let f = dir.join("gone.txt");
        std::fs::write(&f, "x").unwrap();
        for p in [f, dir.join("folder")] {
            match recycle(&p) {
                Ok(()) => assert!(!p.exists(), "{}", p.display()),
                // A CI runner's profile may have no Recycle Bin.
                Err(e) => eprintln!("Recycle Bin unavailable here: {e}"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn trash_goes_to_finder_trash() {
        let dir = std::env::temp_dir().join(format!("rustudio-trash-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("gone.txt");
        std::fs::write(&f, "x").unwrap();
        match trash_item(&f) {
            Ok(put) => {
                assert!(!f.exists());
                // Leave nothing behind in the person's Trash.
                if let Some(p) = put {
                    let _ = std::fs::remove_file(p);
                }
            }
            // A CI runner may have no Trash.
            Err(e) => eprintln!("trash unavailable here: {e}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
