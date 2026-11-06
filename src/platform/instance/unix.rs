//! Unix sockets (Linux, BSDs, macOS). Layout under the runtime dir:
//!
//! ```text
//! <runtime>/                     XDG_RUNTIME_DIR (0700, ours) or, when the
//!   [fltk-omarchy-<uid>/]        runtime dir is shared (/tmp), a private one
//!     <app id>/          0700
//!       instance.lock    0600    flock held by the primary for its lifetime
//!       instance.sock    0600    the primary's socket
//! ```
//!
//! Every directory is checked with `lstat` (no symlinks followed), must be
//! owned by this user and is made 0700. Only the socket file itself is
//! ever removed (a stale one, while holding the lock); nothing else is
//! deleted.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `sun_path` is 108 bytes on Linux, 104 on macOS/BSDs (with the NUL).
const MAX_SOCKET_PATH: usize = 100;

pub(crate) struct Endpoint {
    sock: PathBuf,
    lock: PathBuf,
}

fn euid() -> u32 {
    // SAFETY: no arguments, cannot fail.
    unsafe { libc::geteuid() }
}

fn denied(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, msg)
}

/// Checks (and with `create`, makes) a directory that only this user can
/// use. Never follows a symlink at `dir`.
fn private_dir(dir: &Path, create: bool) -> io::Result<()> {
    let meta = match fs::symlink_metadata(dir) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound && create => {
            match fs::DirBuilder::new().mode(0o700).create(dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            fs::symlink_metadata(dir)?
        }
        Err(e) => return Err(e),
    };
    if !meta.file_type().is_dir() {
        return Err(denied(format!("{} is not a directory", dir.display())));
    }
    if meta.uid() != euid() {
        return Err(denied(format!("{} belongs to another user", dir.display())));
    }
    if meta.mode() & 0o077 != 0 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// FNV-1a, 64 bit: a stable short name for a long path.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

pub(crate) fn endpoint(app_id: &str, runtime: &Path, create: bool) -> io::Result<Endpoint> {
    // The runtime root is given by the system or the app: follow it, but
    // only use it directly when it is private (XDG_RUNTIME_DIR is 0700).
    let meta = fs::metadata(runtime)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", runtime.display()),
        ));
    }
    let base = if meta.uid() == euid() && meta.mode() & 0o077 == 0 {
        runtime.to_path_buf()
    } else {
        let b = runtime.join(format!("fltk-omarchy-{}", euid()));
        private_dir(&b, create)?;
        b
    };
    let mut dir = base.join(app_id);
    if dir.join("instance.sock").as_os_str().len() > MAX_SOCKET_PATH {
        // A socket path has to fit sun_path (104 bytes on macOS, whose
        // per-user temp dir is long): use a short private dir in /tmp named
        // after the runtime dir and the app id.
        let short = Path::new("/tmp").join(format!("fltk-omarchy-{}", euid()));
        private_dir(&short, create)?;
        dir = short.join(format!(
            "{:016x}",
            fnv(&format!("{}\0{app_id}", runtime.display()))
        ));
    }
    private_dir(&dir, create)?;
    let sock = dir.join("instance.sock");
    if sock.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("socket path too long: {}", sock.display()),
        ));
    }
    Ok(Endpoint {
        lock: dir.join("instance.lock"),
        sock,
    })
}

impl Endpoint {
    pub(crate) fn describe(&self) -> String {
        self.sock.display().to_string()
    }
}

pub(crate) fn claim(ep: &Endpoint) -> io::Result<Option<Listener>> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&ep.lock)?;
    // SAFETY: a valid open descriptor.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = io::Error::last_os_error();
        return match e.raw_os_error() {
            Some(libc::EWOULDBLOCK) => Ok(None),
            _ => Err(e),
        };
    }
    // We hold the lock: a socket file left here belongs to a dead primary.
    match fs::symlink_metadata(&ep.sock) {
        Ok(m) if m.file_type().is_socket() => fs::remove_file(&ep.sock)?,
        Ok(_) => {
            return Err(denied(format!(
                "{} exists and is not a socket",
                ep.sock.display()
            )))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(&ep.sock)?;
    fs::set_permissions(&ep.sock, fs::Permissions::from_mode(0o600))?;
    let ino = fs::symlink_metadata(&ep.sock)?.ino();
    Ok(Some(Listener {
        listener,
        sock: ep.sock.clone(),
        ino,
        _lock: lock,
    }))
}

pub(crate) struct Listener {
    listener: UnixListener,
    sock: PathBuf,
    ino: u64,
    _lock: File,
}

/// The uid of the other end, where the OS reports it.
fn peer_uid(s: &UnixStream) -> Option<u32> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: valid socket, out buffer of the size given.
        let rc = unsafe {
            libc::getsockopt(
                s.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        (rc == 0).then_some(cred.uid)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: valid socket and out pointers.
        let rc = unsafe { libc::getpeereid(s.as_raw_fd(), &mut uid, &mut gid) };
        (rc == 0).then_some(uid)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    {
        let _ = s;
        None // the 0700 directory is the only guard here
    }
}

/// A peer of another user is refused; an unknown uid is accepted (the
/// private directory already keeps other users out).
fn same_user(s: &UnixStream) -> bool {
    peer_uid(s).map_or(true, |u| u == euid())
}

impl Listener {
    pub(crate) fn accept(&self) -> io::Result<Conn> {
        loop {
            let (s, _) = self.listener.accept()?;
            if same_user(&s) {
                return Ok(Conn(s));
            }
        }
    }

    pub(crate) fn wake(&self) {
        let _ = UnixStream::connect(&self.sock);
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // Remove our socket file only (not one a successor bound since).
        if fs::symlink_metadata(&self.sock)
            .is_ok_and(|m| m.ino() == self.ino && m.file_type().is_socket())
        {
            let _ = fs::remove_file(&self.sock);
        }
        // The lock is released when `_lock` closes.
    }
}

pub(crate) fn connect(ep: &Endpoint, timeout: Duration) -> io::Result<Conn> {
    let s = UnixStream::connect(&ep.sock)?;
    if !same_user(&s) {
        return Err(denied(format!(
            "{} is served by another user",
            ep.sock.display()
        )));
    }
    let mut c = Conn(s);
    c.set_timeout(Some(timeout))?;
    Ok(c)
}

pub(crate) struct Conn(UnixStream);

impl Conn {
    pub(crate) fn set_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        let d = d.map(|d| d.max(Duration::from_millis(1)));
        self.0.set_read_timeout(d)?;
        self.0.set_write_timeout(d)
    }

    pub(crate) fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }

    pub(crate) fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn raise_native<W: fltk::prelude::WindowExt>(_win: &W) {}

/// `[NSApp activateIgnoringOtherApps:YES]`: an app in the background
/// cannot make its window key otherwise. AppKit is loaded by FLTK.
#[cfg(target_os = "macos")]
pub(crate) fn raise_native<W: fltk::prelude::WindowExt>(_win: &W) {
    use std::ffi::{c_char, c_void};
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }
    type Send0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type SendBool = unsafe extern "C" fn(*mut c_void, *mut c_void, i8);
    // SAFETY: objc_msgSend called through each message's exact signature
    // (object receiver, selector, BOOL argument); results are checked.
    unsafe {
        let cls = objc_getClass(c"NSApplication".as_ptr());
        if cls.is_null() {
            return;
        }
        let msg = objc_msgSend as unsafe extern "C" fn();
        let send0: Send0 = std::mem::transmute(msg);
        let send_bool: SendBool = std::mem::transmute(msg);
        let app = send0(cls, sel_registerName(c"sharedApplication".as_ptr()));
        if !app.is_null() {
            send_bool(
                app,
                sel_registerName(c"activateIgnoringOtherApps:".as_ptr()),
                1,
            );
        }
    }
}
