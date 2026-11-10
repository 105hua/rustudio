//! Per-user folders. Linux (and the other XDG Unixes) keeps the layout
//! rustudio has always used (`~/.local/share`, `~/.config`, `~/.local/state`,
//! `~/.cache`); Windows and macOS use their own folders through `dirs` (on
//! Windows `%LOCALAPPDATA%` for data, state and cache, `%APPDATA%` for
//! config; on macOS `~/Library/Application Support` for data, config and
//! state, `~/Library/Caches` for caches), the same config folder
//! rustudio-account uses.

use std::path::PathBuf;

/// The user's home folder ("." when it can't be found).
pub fn home() -> PathBuf {
    #[cfg(unix)]
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(not(unix))]
    let home = dirs::home_dir();
    home.unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}

/// Shared data: `~/.local/share` on Linux. Not `$XDG_DATA_HOME`, so files
/// rustudio already put there (the content link, the Studio download) keep
/// being found.
pub fn data() -> PathBuf {
    #[cfg(all(unix, not(target_os = "macos")))]
    return home().join(".local/share");
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    return dirs::data_local_dir().unwrap_or_else(home);
}

/// Settings: `$XDG_CONFIG_HOME` or `~/.config` on Linux.
pub fn config() -> PathBuf {
    #[cfg(all(unix, not(target_os = "macos")))]
    return xdg("XDG_CONFIG_HOME", ".config");
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    return dirs::config_dir().unwrap_or_else(home);
}

/// State that outlives a run: `$XDG_STATE_HOME` or `~/.local/state` on Linux.
pub fn state() -> PathBuf {
    #[cfg(all(unix, not(target_os = "macos")))]
    return xdg("XDG_STATE_HOME", ".local/state");
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    return dirs::data_local_dir().unwrap_or_else(home);
}

/// Caches: `$XDG_CACHE_HOME` or `~/.cache` on Linux.
pub fn cache() -> PathBuf {
    #[cfg(all(unix, not(target_os = "macos")))]
    return xdg("XDG_CACHE_HOME", ".cache");
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    return dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
}

/// Where screenshots go: `~/Pictures` on Linux, the Pictures folder elsewhere.
pub fn pictures() -> PathBuf {
    #[cfg(all(unix, not(target_os = "macos")))]
    return home().join("Pictures");
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    return dirs::picture_dir().unwrap_or_else(|| home().join("Pictures"));
}

/// Roblox Studio installed on this Mac (`RobloxStudio.app` in /Applications
/// or ~/Applications): the folder holding `content/`, `ExtraContent/` and
/// `PlatformContent/`, laid out like the Windows install and the CDN
/// packages. None on other systems.
pub fn installed_studio() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    [PathBuf::from("/Applications"), home().join("Applications")]
        .into_iter()
        .map(|a| a.join("RobloxStudio.app/Contents/Resources"))
        .find(|r| r.join("ExtraContent/scripts/PlayerScripts").is_dir() && r.join("content").is_dir())
}

/// A data file that ships with rustudio (`api-dump.json`), wherever this
/// build keeps it: next to the executable, in a macOS app bundle's
/// `Contents/Resources`, in `../share/rustudio` (an install prefix), else the
/// source tree it was built from (`cargo run`).
pub fn resource(name: &str) -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.canonicalize().ok())
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf));
    if let Some(dir) = exe_dir {
        for p in [dir.join(name), dir.join("../Resources").join(name), dir.join("../share/rustudio").join(name)] {
            if p.is_file() {
                return p;
            }
        }
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_per_platform() {
        if cfg!(target_os = "macos") {
            let lib = home().join("Library");
            assert_eq!(data(), lib.join("Application Support"));
            assert_eq!(config(), lib.join("Application Support"));
            assert_eq!(state(), lib.join("Application Support"));
            assert_eq!(cache(), lib.join("Caches"));
        } else if cfg!(unix) {
            assert_eq!(data(), home().join(".local/share"));
        }
    }

    #[test]
    fn api_dump_found() {
        assert!(resource("api-dump.json").is_file());
    }
}
