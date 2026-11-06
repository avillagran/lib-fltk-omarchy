//! Unix backend: memfd (Linux) or unlinked POSIX shm (macOS, BSDs) with
//! pipes for inherited descriptors; named POSIX shm with FIFOs; mapped
//! files. Every descriptor is close-on-exec except while a child that
//! should inherit it is being spawned ([`Remote::share`]).

use super::{unique_id, Kind, Mapping, Parts, Role, Wait};
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, IntoRawFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct MapOs;

fn last() -> io::Error {
    io::Error::last_os_error()
}

fn check(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 {
        Err(last())
    } else {
        Ok(r)
    }
}

/// An owned descriptor (closed on drop).
struct Fd(libc::c_int);

impl Drop for Fd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            // SAFETY: we own this descriptor.
            unsafe { libc::close(self.0) };
        }
    }
}

fn set_cloexec(fd: libc::c_int, on: bool) -> io::Result<()> {
    // SAFETY: plain fcntl on a descriptor we hold.
    unsafe {
        let flags = check(libc::fcntl(fd, libc::F_GETFD))?;
        let next = if on {
            flags | libc::FD_CLOEXEC
        } else {
            flags & !libc::FD_CLOEXEC
        };
        check(libc::fcntl(fd, libc::F_SETFD, next))?;
    }
    Ok(())
}

fn set_nonblock(fd: libc::c_int) -> io::Result<()> {
    // SAFETY: plain fcntl on a descriptor we hold.
    unsafe {
        let flags = check(libc::fcntl(fd, libc::F_GETFL))?;
        check(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK))?;
    }
    Ok(())
}

/// A close-on-exec, non-blocking pipe: (read end, write end).
fn pipe() -> io::Result<(Fd, Fd)> {
    let mut p = [0 as libc::c_int; 2];
    // SAFETY: `p` has room for the two descriptors.
    check(unsafe { libc::pipe(p.as_mut_ptr()) })?;
    let (r, w) = (Fd(p[0]), Fd(p[1]));
    for fd in [r.0, w.0] {
        set_cloexec(fd, true)?;
        set_nonblock(fd)?;
    }
    Ok((r, w))
}

/// A close-on-exec duplicate of `fd`.
fn dup(fd: libc::c_int) -> io::Result<Fd> {
    // SAFETY: F_DUPFD_CLOEXEC returns a new descriptor we own.
    Ok(Fd(check(unsafe {
        libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3)
    })?))
}

fn map_fd(fd: libc::c_int, len: Option<usize>) -> io::Result<Mapping> {
    let len = match len {
        Some(l) => l,
        None => {
            // SAFETY: `st` is written by fstat.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            check(unsafe { libc::fstat(fd, &mut st) })?;
            st.st_size as usize
        }
    };
    if len == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty frame surface",
        ));
    }
    // SAFETY: a fresh shared mapping of `len` bytes of `fd`; checked below.
    let p = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if p == libc::MAP_FAILED {
        return Err(last());
    }
    Ok(Mapping {
        ptr: p as *mut u8,
        len,
        _os: MapOs,
    })
}

pub(crate) fn unmap(ptr: *mut u8, len: usize) {
    // SAFETY: `ptr`/`len` come from our own mmap.
    unsafe { libc::munmap(ptr as *mut libc::c_void, len) };
}

fn truncate(fd: libc::c_int, len: usize) -> io::Result<()> {
    // SAFETY: plain ftruncate on our descriptor.
    check(unsafe { libc::ftruncate(fd, len as libc::off_t) }).map(|_| ())
}

// ------------------------------------------------------------------ bells

pub(crate) struct Bell(Arc<Fd>);

impl Bell {
    pub(crate) fn ring(&self) {
        let b = 1u8;
        // SAFETY: one byte from a valid buffer; a full pipe (EAGAIN) means
        // the other side has wake-ups pending already.
        unsafe { libc::write(self.0 .0, &b as *const u8 as *const libc::c_void, 1) };
    }
}

pub(crate) struct Waiter {
    fd: Arc<Fd>,
    wake_r: Fd,
    wake_w: Arc<Fd>,
}

#[derive(Clone)]
pub(crate) struct Interrupter(Arc<Fd>);

impl Interrupter {
    pub(crate) fn interrupt(&self) {
        let b = 1u8;
        // SAFETY: one byte into our own non-blocking self-pipe.
        unsafe { libc::write(self.0 .0, &b as *const u8 as *const libc::c_void, 1) };
    }
}

fn drain(fd: libc::c_int) -> usize {
    let mut buf = [0u8; 256];
    let mut n = 0;
    loop {
        // SAFETY: reads into a local buffer from a non-blocking descriptor.
        let r = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if r <= 0 {
            return n;
        }
        n += r as usize;
    }
}

impl Waiter {
    fn new(fd: Arc<Fd>) -> io::Result<Waiter> {
        let (r, w) = pipe()?;
        Ok(Waiter {
            fd,
            wake_r: r,
            wake_w: Arc::new(w),
        })
    }

    pub(crate) fn interrupter(&self) -> Interrupter {
        Interrupter(self.wake_w.clone())
    }

    pub(crate) fn wait(&mut self, timeout: Option<Duration>) -> Wait {
        let ms = timeout.map_or(-1, |d| d.as_millis().min(i32::MAX as u128) as libc::c_int);
        let mut fds = [
            libc::pollfd {
                fd: self.fd.0,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.wake_r.0,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        loop {
            // SAFETY: two valid pollfd entries.
            let r = unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) };
            if r < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Wait::Closed;
            }
            if r == 0 {
                return Wait::Timeout;
            }
            break;
        }
        if fds[1].revents != 0 {
            drain(self.wake_r.0);
            return Wait::Interrupted;
        }
        let got = drain(self.fd.0);
        if got > 0 {
            Wait::Rung
        } else if fds[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            Wait::Closed
        } else {
            Wait::Timeout
        }
    }
}

// ------------------------------------------------------- remote and names

/// The creator's copies of the descriptors the other process inherits.
#[derive(Default)]
pub(crate) struct Remote(Vec<Fd>);

impl Remote {
    pub(crate) fn share(&self, cmd: &mut std::process::Command) {
        use std::os::unix::process::CommandExt;
        if self.0.is_empty() {
            return;
        }
        let mut fds = [-1 as libc::c_int; 3];
        for (slot, fd) in fds.iter_mut().zip(&self.0) {
            *slot = fd.0;
        }
        // SAFETY: the closure runs in the forked child before exec and
        // only calls fcntl (async-signal-safe) on descriptors it inherited.
        unsafe {
            cmd.pre_exec(move || {
                for fd in fds {
                    if fd >= 0 {
                        let flags = libc::fcntl(fd, libc::F_GETFD);
                        if flags >= 0 {
                            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                        }
                    }
                }
                Ok(())
            });
        }
    }

    pub(crate) fn release(&mut self) {
        self.0.clear();
    }
}

/// Names this side created and removes on drop.
#[derive(Default)]
pub(crate) struct Cleanup {
    shm: Option<CString>,
    files: Vec<PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(n) = &self.shm {
            // SAFETY: a NUL-terminated name we created.
            unsafe { libc::shm_unlink(n.as_ptr()) };
        }
        for f in &self.files {
            // Only the exact file this process created (never a tree).
            let _ = fs::remove_file(f);
        }
    }
}

fn euid() -> u32 {
    // SAFETY: no arguments, cannot fail.
    unsafe { libc::geteuid() }
}

/// `<base>/fltk-omarchy-frames-<uid>`, 0700, owned by us, not a symlink.
fn private_dir(base: &Path) -> io::Result<PathBuf> {
    let dir = base.join(format!("fltk-omarchy-frames-{}", euid()));
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = fs::symlink_metadata(&dir)?;
    if !meta.file_type().is_dir() || meta.uid() != euid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a private directory", dir.display()),
        ));
    }
    if meta.mode() & 0o077 != 0 {
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(dir)
}

fn shm_name(id: &str) -> CString {
    // macOS limits shm names to 31 bytes: "/ofrm-" + a 22-byte id fits.
    CString::new(format!("/ofrm-{id}")).expect("no NUL in id")
}

fn shm_open(name: &CString, create: bool) -> io::Result<Fd> {
    let flags = libc::O_RDWR
        | if create {
            libc::O_CREAT | libc::O_EXCL
        } else {
            0
        };
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    // SAFETY: NUL-terminated name; variadic mode argument as an int.
    let fd = unsafe { libc::shm_open(name.as_ptr(), flags, 0o600 as libc::c_uint) };
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    // SAFETY: NUL-terminated name.
    let fd = unsafe { libc::shm_open(name.as_ptr(), flags, 0o600 as libc::mode_t) };
    let fd = Fd(check(fd)?);
    set_cloexec(fd.0, true)?;
    Ok(fd)
}

/// Anonymous shared memory: memfd on Linux, else an unlinked shm object,
/// else an unlinked file in the private dir.
fn anonymous(base: &Path) -> io::Result<Fd> {
    #[cfg(target_os = "linux")]
    {
        let name = CString::new("fltk-omarchy-frame").expect("static");
        // SAFETY: NUL-terminated name; returns a new descriptor or -1.
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd >= 0 {
            return Ok(Fd(fd));
        }
    }
    let name = shm_name(&unique_id());
    if let Ok(fd) = shm_open(&name, true) {
        // SAFETY: the name we just created; the descriptor keeps it alive.
        unsafe { libc::shm_unlink(name.as_ptr()) };
        return Ok(fd);
    }
    let path = private_dir(base)?.join(format!("{}.anon", unique_id()));
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let _ = fs::remove_file(&path);
    Ok(Fd(f.into_raw_fd()))
}

/// Opens a FIFO read-write and non-blocking: it never reports EOF and a
/// write never blocks or fails for lack of a reader.
fn open_fifo(path: &Path) -> io::Result<Arc<Fd>> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)?;
    let meta = f.metadata()?;
    if meta.uid() != euid() || !std::os::unix::fs::FileTypeExt::is_fifo(&meta.file_type()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "not our FIFO",
        ));
    }
    Ok(Arc::new(Fd(f.into_raw_fd())))
}

fn mkfifo(path: &Path) -> io::Result<()> {
    let c = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: NUL-terminated path.
    check(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }).map(|_| ())
}

/// (ring end, wait end) for `role` from the present and input channels.
fn pick<T>(role: Role, present: T, input: T) -> (T, T) {
    match role {
        Role::Producer => (present, input),
        Role::Consumer => (input, present),
    }
}

pub(crate) fn create(kind: Kind, len: usize, base: &Path, role: Role) -> io::Result<Parts> {
    match kind {
        Kind::Fd => {
            let mem = anonymous(base)?;
            truncate(mem.0, len)?;
            let map = map_fd(mem.0, Some(len))?;
            let (present_r, present_w) = pipe()?;
            let (input_r, input_w) = pipe()?;
            // Ours and theirs, by role.
            let (mine, theirs) = match role {
                // We ring present, wait on input; they wait on present, ring input.
                Role::Producer => ((present_w, input_r), (present_r, input_w)),
                Role::Consumer => ((input_w, present_r), (input_r, present_w)),
            };
            let (ring, wait) = mine;
            // Handle order is always mem, present, input.
            let (t_present, t_input) = match role {
                Role::Producer => (theirs.0, theirs.1),
                Role::Consumer => (theirs.1, theirs.0),
            };
            let handle = format!(
                "fd:{}:{}:{}:{}",
                std::process::id(),
                mem.0,
                t_present.0,
                t_input.0
            );
            Ok(Parts {
                map,
                bell: Some(Bell(Arc::new(ring))),
                waiter: Some(Waiter::new(Arc::new(wait))?),
                handle,
                kind,
                remote: Remote(vec![mem, t_present, t_input]),
                cleanup: Cleanup::default(),
            })
        }
        Kind::Named => {
            let id = unique_id();
            let dir = private_dir(base)?;
            let name = shm_name(&id);
            let mem = shm_open(&name, true)?;
            let mut cleanup = Cleanup {
                shm: Some(name),
                files: Vec::new(),
            };
            truncate(mem.0, len)?;
            let map = map_fd(mem.0, Some(len))?;
            let (present, input) = (
                dir.join(format!("{id}.present")),
                dir.join(format!("{id}.input")),
            );
            for p in [&present, &input] {
                mkfifo(p)?;
                cleanup.files.push(p.clone());
            }
            let (ring, wait) = pick(role, open_fifo(&present)?, open_fifo(&input)?);
            let dir_s = dir.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "runtime dir is not UTF-8")
            })?;
            Ok(Parts {
                map,
                bell: Some(Bell(ring)),
                waiter: Some(Waiter::new(wait)?),
                handle: format!("named:{id}:{dir_s}"),
                kind,
                remote: Remote::default(),
                cleanup,
            })
        }
        Kind::File => {
            let path = private_dir(base)?.join(format!("{}.frame", unique_id()));
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            let cleanup = Cleanup {
                shm: None,
                files: vec![path.clone()],
            };
            f.set_len(len as u64)?;
            let map = map_fd(f.as_raw_fd(), Some(len))?;
            let p = path
                .to_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))?;
            Ok(Parts {
                map,
                bell: None,
                waiter: None,
                handle: format!("file:{p}"),
                kind,
                remote: Remote::default(),
                cleanup,
            })
        }
    }
}

fn bad(h: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("bad frame handle {h:?}"),
    )
}

pub(crate) fn open(handle: &str, role: Role) -> io::Result<Parts> {
    if let Some(rest) = handle.strip_prefix("fd:") {
        let n: Vec<i64> = rest
            .split(':')
            .map(|s| s.parse().map_err(|_| bad(handle)))
            .collect::<io::Result<_>>()?;
        let [pid, mem, present, input] = n[..] else {
            return Err(bad(handle));
        };
        let ours = pid == std::process::id() as i64;
        // Inherited descriptors become ours (close-on-exec from now on);
        // in the creating process itself, duplicates.
        let take = |fd: i64| -> io::Result<Fd> {
            let fd = libc::c_int::try_from(fd).map_err(|_| bad(handle))?;
            set_cloexec(fd, true)?;
            if ours {
                dup(fd)
            } else {
                Ok(Fd(fd))
            }
        };
        let (mem, present, input) = (take(mem)?, take(present)?, take(input)?);
        let map = map_fd(mem.0, None)?;
        let (ring, wait) = pick(role, present, input);
        set_nonblock(ring.0)?;
        set_nonblock(wait.0)?;
        return Ok(Parts {
            map,
            bell: Some(Bell(Arc::new(ring))),
            waiter: Some(Waiter::new(Arc::new(wait))?),
            handle: String::new(),
            kind: Kind::Fd,
            remote: Remote::default(),
            cleanup: Cleanup::default(),
        });
    }
    if let Some(rest) = handle.strip_prefix("named:") {
        let (id, dir) = rest.split_once(':').ok_or_else(|| bad(handle))?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(bad(handle));
        }
        let mem = shm_open(&shm_name(id), false)?;
        let map = map_fd(mem.0, None)?;
        let dir = Path::new(dir);
        let present = open_fifo(&dir.join(format!("{id}.present")))?;
        let input = open_fifo(&dir.join(format!("{id}.input")))?;
        let (ring, wait) = pick(role, present, input);
        return Ok(Parts {
            map,
            bell: Some(Bell(ring)),
            waiter: Some(Waiter::new(wait)?),
            handle: String::new(),
            kind: Kind::Named,
            remote: Remote::default(),
            cleanup: Cleanup::default(),
        });
    }
    if let Some(path) = handle.strip_prefix("file:") {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        let meta = f.metadata()?;
        if meta.uid() != euid() || !meta.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "frame file belongs to another user",
            ));
        }
        let map = map_fd(f.as_raw_fd(), None)?;
        return Ok(Parts {
            map,
            bell: None,
            waiter: None,
            handle: String::new(),
            kind: Kind::File,
            remote: Remote::default(),
            cleanup: Cleanup::default(),
        });
    }
    Err(bad(handle))
}
