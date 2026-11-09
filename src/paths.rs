//! Per-user folders. Unix keeps the XDG layout rustudio has always used
//! (`~/.local/share`, `~/.config`, `~/.local/state`, `~/.cache`); other
//! platforms use their own folders through `dirs` (on Windows
//! `%LOCALAPPDATA%` for data, state and cache, `%APPDATA%` for config,
//! the same config folder rustudio-account uses).

use std::path::PathBuf;

/// The user's home folder ("." when it can't be found).
pub fn home() -> PathBuf {
    #[cfg(unix)]
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(not(unix))]
    let home = dirs::home_dir();
    home.unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(unix)]
fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}

/// Shared data: `~/.local/share` on Unix. Not `$XDG_DATA_HOME`, so files
/// rustudio already put there (the content link, the Studio download) keep
/// being found.
pub fn data() -> PathBuf {
    #[cfg(unix)]
    return home().join(".local/share");
    #[cfg(not(unix))]
    return dirs::data_local_dir().unwrap_or_else(home);
}

/// Settings: `$XDG_CONFIG_HOME` or `~/.config` on Unix.
pub fn config() -> PathBuf {
    #[cfg(unix)]
    return xdg("XDG_CONFIG_HOME", ".config");
    #[cfg(not(unix))]
    return dirs::config_dir().unwrap_or_else(home);
}

/// State that outlives a run: `$XDG_STATE_HOME` or `~/.local/state` on Unix.
pub fn state() -> PathBuf {
    #[cfg(unix)]
    return xdg("XDG_STATE_HOME", ".local/state");
    #[cfg(not(unix))]
    return dirs::data_local_dir().unwrap_or_else(home);
}

/// Caches: `$XDG_CACHE_HOME` or `~/.cache` on Unix.
pub fn cache() -> PathBuf {
    #[cfg(unix)]
    return xdg("XDG_CACHE_HOME", ".cache");
    #[cfg(not(unix))]
    return dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
}

/// Where screenshots go: `~/Pictures` on Unix, the Pictures folder elsewhere.
pub fn pictures() -> PathBuf {
    #[cfg(unix)]
    return home().join("Pictures");
    #[cfg(not(unix))]
    return dirs::picture_dir().unwrap_or_else(|| home().join("Pictures"));
}
