//! The operating-system layer. Every piece of code that depends on the OS
//! lives here; the rest of the crate is portable and calls these functions.
//! Each OS backend has a working fallback, so nothing outside this module
//! needs `#[cfg(target_os = ..)]`.
//!
//! | | Linux | macOS | Windows |
//! |---|---|---|---|
//! | Native window move/resize | Wayland `xdg_toplevel` through libdecor | FLTK drag fallback | FLTK drag fallback |
//! | Glass (Glassy windows) | native Wayland | opaque | opaque |
//! | Hyprland IPC | when Hyprland runs | never | never |
//! | Theme files | Omarchy state dir | [`dirs::state`]`/omarchy/current` if present | same |
//! | Built-in theme | Omarchy dark | dark or light, following the OS | dark or light, following the OS |
//! | Live reload | inotify (blocking) | mtime polling, 1 s | mtime polling, 1 s |
//! | Default font | fontconfig `monospace` | Menlo | Consolas |
//! | Primary modifier | Ctrl | Cmd | Ctrl |
//! | File dialogs ([`crate::dialog`]) | xdg-desktop-portal over D-Bus, else FLTK's native chooser | FLTK's native chooser (`NSOpenPanel`) | FLTK's native chooser (`IFileDialog`) |
//! | Motion setting ([`animations`]) | Hyprland `animations:enabled` (live), GNOME `enable-animations` | Reduce motion | Client area animation |
//!
//! Details and how each one was verified: docs/PLATFORMS.md.

/// Accessibility adapters per OS for `crate::a11y` (feature `a11y`).
#[cfg(feature = "a11y")]
pub(crate) mod a11y;
pub mod dirs;
pub(crate) mod fl_abi;
pub mod fonts;
/// Shared memory and wake-ups for `crate::frame` (memfd, POSIX shm, file
/// mappings; pipes, FIFOs, events).
pub(crate) mod frame;
mod generic;
/// Local IPC transport for `crate::instance` (Unix sockets, named pipes).
pub(crate) mod instance;
pub mod languages;
pub mod memory;
/// Child processes and audio outputs for `crate::media` playback.
#[cfg(feature = "playback")]
pub(crate) mod playback;
pub(crate) mod poll;
/// xdg-desktop-portal file chooser over a minimal D-Bus client (Linux and
/// BSDs; not macOS, which has no portal).
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) mod portal;
/// Native activation protocols for `native::present`.
pub(crate) mod presentation;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod other;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use other as imp;

/// The operating system this build targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
    /// Other Unix-likes: portable fallbacks everywhere.
    Other,
}

impl Os {
    pub fn name(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::MacOs => "macos",
            Os::Windows => "windows",
            Os::Other => "other",
        }
    }
}

/// The OS of this build.
pub const OS: Os = if cfg!(target_os = "linux") {
    Os::Linux
} else if cfg!(target_os = "macos") {
    Os::MacOs
} else if cfg!(target_os = "windows") {
    Os::Windows
} else {
    Os::Other
};

/// True when FLTK runs on its native Wayland backend (Linux only; false on
/// X11/XWayland and on every other OS).
pub fn wayland() -> bool {
    imp::wayland()
}

/// The OS asks for dark windows: `Some(true)` dark, `Some(false)` light,
/// `None` when the OS has no such setting or it cannot be read (Linux:
/// the Omarchy theme decides). macOS: `AppleInterfaceStyle`; Windows:
/// `AppsUseLightTheme`.
pub fn prefers_dark() -> Option<bool> {
    imp::prefers_dark()
}

/// The OS motion setting: `Some(true)` animations on, `Some(false)` off
/// (reduce motion), `None` when the OS has no such setting or it cannot
/// be read. May block briefly (a Hyprland socket request, `gsettings`):
/// [`crate::anim`] calls it from its watcher thread.
///
/// Linux: Hyprland's `animations:enabled` (Omarchy's Animations switch),
/// else GNOME's `enable-animations` on GNOME-family desktops. macOS:
/// `NSWorkspace.accessibilityDisplayShouldReduceMotion`. Windows:
/// `SPI_GETCLIENTAREAANIMATION`.
pub fn animations() -> Option<bool> {
    imp::animations()
}

/// Starts following changes of [`animations`] where the OS announces them
/// (Hyprland's event socket: `configreloaded` and focus changes), sending
/// on `kick` for each. Returns at once; elsewhere it does nothing.
pub(crate) fn watch_animations(kick: std::sync::mpsc::Sender<()>) {
    imp::watch_animations(kick)
}

/// CPU time used by this process so far (user + system), for benchmarks
/// and diagnostics. None where the OS does not report it.
pub fn process_cpu_time() -> Option<std::time::Duration> {
    imp::process_cpu_time()
}

/// CPU time used by the calling thread so far (user + system): the frame
/// log and the blocked-UI warning of [`crate::testkit`] compare it with
/// wall time. None where the OS does not report it.
pub fn thread_cpu_time() -> Option<std::time::Duration> {
    imp::thread_cpu_time()
}

/// True when theme files live on this OS by default (Omarchy). Elsewhere
/// the built-in theme is used unless a theme directory is configured.
pub fn has_omarchy() -> bool {
    OS == Os::Linux
}

// ------------------------------------------------------- crate-internal API

/// Interactive move/resize started by the compositor (Wayland): `edge` is
/// `None` for a move, else the libdecor edge. False when not possible.
/// `win` is a shown top-level `Fl_Window*`, during its `Event::Push`.
pub(crate) fn native_gesture(
    win: *const std::ffi::c_void,
    edge: Option<i32>,
    dry_run: bool,
) -> bool {
    imp::native_gesture(win, edge, dry_run)
}

/// Integer scale of the output a window is on, when the OS applies it
/// outside FLTK's own scaling: Linux gives `Some` (the Wayland buffer scale
/// for a shown window, else 1); macOS and Windows give `None` (FLTK's
/// `pixels_per_unit` already includes it).
pub(crate) fn output_scale(win: *const std::ffi::c_void, shown: bool) -> Option<i32> {
    imp::output_scale(win, shown)
}

/// Hyprland's request socket when Hyprland runs (Linux only).
pub(crate) fn hyprland_socket() -> Option<std::path::PathBuf> {
    imp::hyprland_socket()
}

/// One request/response exchange on a local IPC socket (Hyprland).
pub(crate) fn ipc_request(
    path: &std::path::Path,
    cmd: &str,
    timeout: std::time::Duration,
) -> Option<String> {
    imp::ipc_request(path, cmd, timeout)
}

/// Theme watcher loop: blocks until `stop`, calls `changed` (debounced,
/// from the watcher thread) when the theme, the user shell.toml, the glass
/// toggles or the OS appearance change.
pub(crate) fn watch_theme(stop: std::sync::Arc<std::sync::atomic::AtomicBool>, changed: &dyn Fn()) {
    imp::watch_theme(stop, changed)
}

/// Translucent surfaces for [`crate::glass`] (native Wayland only).
pub(crate) mod glass {
    pub(crate) use super::imp::glass::{
        begin, current_surface, end, set_opaque_region, target, Target,
    };
}

/// Shows `req` through xdg-desktop-portal's FileChooser and waits for the
/// answer (call it from a worker thread). `parent` is the portal's
/// parent-window handle ([`portal_parent`]). `Err(Unavailable)` where there
/// is no portal (macOS, Windows, no session bus, no portal service), so
/// [`crate::dialog`] falls back to the native chooser.
pub(crate) fn file_portal(
    req: &crate::dialog::Request,
    parent: &str,
) -> Result<Vec<std::path::PathBuf>, crate::dialog::Error> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        portal::run(req, parent)
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = (req, parent);
        Err(crate::dialog::Error::Unavailable(
            "no xdg-desktop-portal on this OS".into(),
        ))
    }
}

/// True where [`file_portal`] can work at all (Linux, BSDs).
pub(crate) fn has_file_portal() -> bool {
    cfg!(all(unix, not(target_os = "macos")))
}

/// The portal's parent-window handle for the active window (`x11:<XID>` on
/// X11, else empty). UI thread.
pub(crate) fn portal_parent() -> String {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        portal::parent_handle()
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        String::new()
    }
}

/// Whether views move their pixels for scroll-only frames (`fl_scroll`)
/// by default: on (measured 2–3× less CPU than repainting while scrolling
/// on X11 and Wayland, docs/PERFORMANCE.md). Builds without `fl_scroll`
/// (MSVC) repaint anyway.
pub fn scroll_blit_default() -> bool {
    true
}
