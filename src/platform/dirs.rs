//! Per-OS user directories, without dependencies.
//!
//! | | Linux (XDG) | macOS | Windows |
//! |---|---|---|---|
//! | [`config`] | `$XDG_CONFIG_HOME` or `~/.config` | `~/Library/Application Support` | `%APPDATA%` |
//! | [`data`] | `$XDG_DATA_HOME` or `~/.local/share` | `~/Library/Application Support` | `%APPDATA%` |
//! | [`state`] | `$XDG_STATE_HOME` or `~/.local/state` | `~/Library/Application Support` | `%LOCALAPPDATA%` |
//! | [`cache`] | `$XDG_CACHE_HOME` or `~/.cache` | `~/Library/Caches` | `%LOCALAPPDATA%` |
//!
//! XDG variables that are empty or relative are ignored (XDG spec). The
//! `app_*` helpers append the app id (`dev.you.app`); they do not create
//! the directory.

use super::{Os, OS};
use std::ffi::OsString;
use std::path::PathBuf;

fn env_abs(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// The user's home directory (`$HOME`; `%USERPROFILE%` on Windows).
pub fn home() -> PathBuf {
    let var = |n: &str| {
        std::env::var_os(n)
            .filter(|v: &OsString| !v.is_empty())
            .map(PathBuf::from)
    };
    match OS {
        Os::Windows => var("USERPROFILE")
            .or_else(|| var("HOME"))
            .unwrap_or_else(|| PathBuf::from("C:\\")),
        _ => var("HOME").unwrap_or_else(|| PathBuf::from("/")),
    }
}

fn mac_support() -> PathBuf {
    home().join("Library/Application Support")
}

fn roaming() -> PathBuf {
    env_abs("APPDATA").unwrap_or_else(|| home().join("AppData\\Roaming"))
}

fn local() -> PathBuf {
    env_abs("LOCALAPPDATA").unwrap_or_else(|| home().join("AppData\\Local"))
}

/// Settings written by the user or the app.
pub fn config() -> PathBuf {
    match OS {
        Os::MacOs => mac_support(),
        Os::Windows => roaming(),
        _ => env_abs("XDG_CONFIG_HOME").unwrap_or_else(|| home().join(".config")),
    }
}

/// Data files the app keeps.
pub fn data() -> PathBuf {
    match OS {
        Os::MacOs => mac_support(),
        Os::Windows => roaming(),
        _ => env_abs("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local/share")),
    }
}

/// State that survives restarts but is not worth a backup (history, window size).
pub fn state() -> PathBuf {
    match OS {
        Os::MacOs => mac_support(),
        Os::Windows => local(),
        _ => env_abs("XDG_STATE_HOME").unwrap_or_else(|| home().join(".local/state")),
    }
}

/// Disposable cache.
pub fn cache() -> PathBuf {
    match OS {
        Os::MacOs => home().join("Library/Caches"),
        Os::Windows => local(),
        _ => env_abs("XDG_CACHE_HOME").unwrap_or_else(|| home().join(".cache")),
    }
}

/// Runtime files (sockets, locks) that live as long as the session.
/// Linux/BSDs: `$XDG_RUNTIME_DIR` (private, 0700), else the temporary
/// directory; macOS: `$TMPDIR` (per user); Windows: `%TEMP%` (per user).
/// The temporary-directory fallback can be shared by every user (`/tmp`):
/// code that puts sockets here makes a private subdirectory first, as
/// [`crate::instance`] does. `OMARCHY_FLTK_RUNTIME_DIR` overrides it
/// (tests, sandboxes).
pub fn runtime() -> PathBuf {
    if let Some(p) = env_abs("OMARCHY_FLTK_RUNTIME_DIR") {
        return p;
    }
    match OS {
        Os::MacOs | Os::Windows => std::env::temp_dir(),
        _ => env_abs("XDG_RUNTIME_DIR").unwrap_or_else(std::env::temp_dir),
    }
}

/// `config()/<app_id>`.
pub fn app_config(app_id: &str) -> PathBuf {
    config().join(app_id)
}

/// `data()/<app_id>`.
pub fn app_data(app_id: &str) -> PathBuf {
    data().join(app_id)
}

/// `state()/<app_id>`.
pub fn app_state(app_id: &str) -> PathBuf {
    state().join(app_id)
}

/// `cache()/<app_id>`.
pub fn app_cache(app_id: &str) -> PathBuf {
    cache().join(app_id)
}

/// Omarchy's `current` state directory (theme, theme.name). Linux:
/// `~/.local/state/omarchy/current` as Omarchy writes it; elsewhere
/// `state()/omarchy/current`, where a theme can be dropped by hand.
pub(crate) fn omarchy_current() -> PathBuf {
    match OS {
        Os::Linux => home().join(".local/state/omarchy/current"),
        _ => state().join("omarchy/current"),
    }
}

/// The language picked under Omarchy's Setup > Language (`~/.config/
/// omarchy/language`, one registry code such as `da` or `pt-BR`); None
/// where Omarchy has no such file (macOS, Windows).
pub(crate) fn omarchy_language_file() -> Option<PathBuf> {
    match OS {
        Os::Linux => Some(omarchy_config().join("language")),
        _ => None,
    }
}

/// Omarchy's registry of languages (`$OMARCHY_PATH/default/i18n/locales.json`:
/// names, direction, decimal separator, digits); None without Omarchy.
pub(crate) fn omarchy_locales_registry() -> Option<PathBuf> {
    match OS {
        Os::Linux => {
            let root = std::env::var_os("OMARCHY_PATH")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/usr/share/omarchy"));
            Some(root.join("default/i18n/locales.json"))
        }
        _ => None,
    }
}

/// Omarchy's user configuration directory (`shell.toml`).
pub(crate) fn omarchy_config() -> PathBuf {
    match OS {
        // Same as before the platform layer (XDG_CONFIG_HOME as given).
        Os::Linux => std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"))
            .join("omarchy"),
        _ => config().join("omarchy"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_are_absolute_and_app_dirs_append_the_id() {
        for d in [config(), data(), state(), cache()] {
            assert!(d.is_absolute() || OS == Os::Windows, "{d:?}");
        }
        assert_eq!(app_config("dev.x.y"), config().join("dev.x.y"));
        assert_eq!(app_cache("dev.x.y"), cache().join("dev.x.y"));
        assert!(omarchy_current().ends_with("omarchy/current"));
    }
}
