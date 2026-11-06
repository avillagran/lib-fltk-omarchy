//! Local IPC transport for [`crate::instance`] (single instance + app
//! IPC), one backend per OS:
//!
//! | | Endpoint | Single-instance lock | Same user only |
//! |---|---|---|---|
//! | Linux, BSDs, macOS | Unix socket `<runtime>/<app id>/instance.sock`, 0600 in a 0700 dir | `flock` on `instance.lock` (released by the kernel when the process dies) | private dirs, peer uid (`SO_PEERCRED` / `getpeereid`) on both ends |
//! | Windows | named pipe `\\.\pipe\fltk-omarchy-<app id>-<hash of the runtime dir>` | `FILE_FLAG_FIRST_PIPE_INSTANCE` (the pipe goes away with the process) | default pipe DACL (only the creator can write), remote clients rejected, client checks the pipe owner |
//! | Other | none: [`claim`] and [`connect`] return `Unsupported` | | |
//!
//! The portable protocol (framing, size limits, timeouts, UI-thread
//! hand-off) lives in `crate::instance`; this module only moves bytes.

use std::io;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(not(any(unix, windows)))]
mod none;
#[cfg(not(any(unix, windows)))]
use none as imp;

/// Where an instance of an app listens (resolved, not necessarily live).
pub(crate) struct Endpoint(imp::Endpoint);

/// The result of trying to become the primary instance.
pub(crate) enum Claim {
    /// This process owns the endpoint and listens on it.
    Primary(Listener),
    /// Another process owns it (it may still be starting).
    Taken,
}

/// The primary's listening end.
pub(crate) struct Listener(imp::Listener);

/// One connection (either side). Blocking reads and writes, bounded by
/// [`Conn::set_timeout`].
pub(crate) struct Conn(imp::Conn);

/// Resolves the endpoint of `app_id` under `runtime` (normally
/// [`super::dirs::runtime`]). `create` makes the private directories the
/// endpoint needs (primary side); a client passes false and gets
/// `NotFound` when nothing was ever created.
pub(crate) fn endpoint(app_id: &str, runtime: &Path, create: bool) -> io::Result<Endpoint> {
    imp::endpoint(app_id, runtime, create).map(Endpoint)
}

impl Endpoint {
    /// Human-readable address (socket path or pipe name), for messages.
    pub(crate) fn describe(&self) -> String {
        self.0.describe()
    }
}

/// Tries to become the primary instance (atomic: of two processes racing,
/// exactly one gets `Primary`). A stale endpoint left by a dead primary is
/// taken over.
pub(crate) fn claim(ep: &Endpoint) -> io::Result<Claim> {
    Ok(match imp::claim(&ep.0)? {
        Some(l) => Claim::Primary(Listener(l)),
        None => Claim::Taken,
    })
}

/// Connects to the primary. `NotFound` / `ConnectionRefused` mean no
/// primary listens; `PermissionDenied` that the endpoint belongs to
/// another user.
pub(crate) fn connect(ep: &Endpoint, timeout: Duration) -> io::Result<Conn> {
    imp::connect(&ep.0, timeout).map(Conn)
}

impl Listener {
    /// Blocks until a client of the same user connects.
    pub(crate) fn accept(&self) -> io::Result<Conn> {
        self.0.accept().map(Conn)
    }

    /// Unblocks a pending [`Listener::accept`] (shutdown).
    pub(crate) fn wake(&self) {
        self.0.wake()
    }
}

impl Conn {
    /// Deadline for each following read or write (None: no deadline).
    pub(crate) fn set_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        self.0.set_timeout(d)
    }
}

impl io::Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl io::Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Brings a shown top-level window to the front where the OS lets an app
/// do that itself: macOS activates the app (`NSApp
/// activateIgnoringOtherApps:`), Windows restores and foregrounds the
/// window (`SetForegroundWindow`, allowed because the client of the
/// request called `AllowSetForegroundWindow` for us). Linux does nothing
/// here: FLTK's own `take_focus` sends `_NET_ACTIVE_WINDOW` on X11, and
/// Wayland needs the compositor (Hyprland `focuswindow`, in
/// `AppWindow::raise`).
pub(crate) fn raise_native<W: fltk::prelude::WindowExt>(win: &W) {
    imp::raise_native(win)
}
