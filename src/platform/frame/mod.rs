//! Shared memory and wake-ups between two processes for [`crate::frame`]
//! (an app showing frames that a helper process draws). This module only
//! maps memory and moves wake-ups; the layout of the memory (header,
//! triple buffer, event rings) is portable and lives in `crate::frame`.
//!
//! | | Memory | Wake-up | Handle for the other process |
//! |---|---|---|---|
//! | Linux, [`Kind::Fd`] | `memfd_create` (fallback: unlinked file in the runtime dir) | pipes (one byte per wake-up) | inherited descriptors, `fd:<mem>:<present>:<input>` |
//! | macOS, BSDs, [`Kind::Fd`] | `shm_open` + `shm_unlink` (fallback: unlinked file) | pipes | same |
//! | Unix, [`Kind::Named`] | `shm_open("/ofrm-<id>")` | FIFOs in a private dir of the runtime dir | `named:<id>:<dir>` |
//! | Windows, [`Kind::Named`] (also for `Fd`) | `CreateFileMappingW("Local\fltk-omarchy-<id>")` | named auto-reset events | `named:<id>` |
//! | Every OS, [`Kind::File`] | a 0600 file in a private dir of the runtime dir, mapped | none: the waiting side polls (capped rate) | `file:<path>` |
//! | Other OSes | unsupported (`crate::frame`'s PNG/BMP file mode still works) | | |
//!
//! Nothing here deletes a directory. The creator removes only the names it
//! made itself (its shm name, its two FIFOs, its frame file) when dropped.

use std::io;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
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

/// How the memory is shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Anonymous memory and pipes inherited by a child process (Unix).
    /// Windows uses [`Kind::Named`] instead.
    Fd,
    /// Named objects any process of the same user can open.
    Named,
    /// A mapped regular file and polling: works wherever files can be
    /// mapped, the slowest wake-up.
    File,
}

/// Which side of a surface this process is: it decides which channel it
/// rings (producer: "a frame is ready"; consumer: "input is queued") and
/// which one it waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Producer,
    Consumer,
}

/// How often a polling waiter ([`Kind::File`]) looks at the memory.
pub(crate) const POLL: Duration = Duration::from_millis(8);

/// A shared mapping, readable and writable. `Send + Sync`: what is shared
/// through it is synchronized by atomics in `crate::frame`.
pub(crate) struct Mapping {
    ptr: *mut u8,
    len: usize,
    _os: imp::MapOs,
}

// SAFETY: the mapping is plain memory that stays valid until drop; every
// access from `crate::frame` goes through atomics or through buffers that
// the triple-buffer protocol gives to one side at a time.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    pub(crate) fn ptr(&self) -> *mut u8 {
        self.ptr
    }
    pub(crate) fn len(&self) -> usize {
        self.len
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        imp::unmap(self.ptr, self.len);
    }
}

/// What a wait ended with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wait {
    /// The other side rang at least once (all pending rings consumed).
    Rung,
    Timeout,
    /// [`Interrupter::interrupt`] was called.
    Interrupted,
    /// The other side is gone (its end of the pipe closed).
    Closed,
}

/// The ringing end of a channel.
pub(crate) struct Bell(Option<imp::Bell>);

impl Bell {
    /// Wakes the other side (never blocks; coalesced when it is not
    /// draining yet). A polling channel does nothing here.
    pub(crate) fn ring(&self) {
        if let Some(b) = &self.0 {
            b.ring();
        }
    }
}

struct PollState {
    interrupted: Mutex<bool>,
    cv: Condvar,
}

enum WaitImpl {
    Os(imp::Waiter),
    Poll(Arc<PollState>),
}

/// The waiting end of a channel.
pub(crate) struct Waiter(WaitImpl);

impl Waiter {
    fn polling() -> Waiter {
        Waiter(WaitImpl::Poll(Arc::new(PollState {
            interrupted: Mutex::new(false),
            cv: Condvar::new(),
        })))
    }

    /// Blocks until the other side rings, `timeout` passes or an
    /// [`Interrupter`] fires. A polling waiter sleeps at most [`POLL`] and
    /// returns `Timeout`: the caller then looks at the memory itself.
    pub(crate) fn wait(&mut self, timeout: Option<Duration>) -> Wait {
        match &mut self.0 {
            WaitImpl::Os(w) => w.wait(timeout),
            WaitImpl::Poll(p) => {
                let d = timeout.map_or(POLL, |t| t.min(POLL));
                let mut g = p.interrupted.lock().unwrap_or_else(|e| e.into_inner());
                if !*g {
                    g = p.cv.wait_timeout(g, d).unwrap_or_else(|e| e.into_inner()).0;
                }
                if std::mem::replace(&mut *g, false) {
                    Wait::Interrupted
                } else {
                    Wait::Timeout
                }
            }
        }
    }

    /// Some(interval) when nobody rings and the memory must be polled.
    pub(crate) fn poll_interval(&self) -> Option<Duration> {
        match self.0 {
            WaitImpl::Os(_) => None,
            WaitImpl::Poll(_) => Some(POLL),
        }
    }

    /// A thread-safe handle that ends a pending (or the next) wait.
    pub(crate) fn interrupter(&self) -> Interrupter {
        match &self.0 {
            WaitImpl::Os(w) => Interrupter(IntImpl::Os(w.interrupter())),
            WaitImpl::Poll(p) => Interrupter(IntImpl::Poll(p.clone())),
        }
    }
}

#[derive(Clone)]
enum IntImpl {
    Os(imp::Interrupter),
    Poll(Arc<PollState>),
}

/// Ends a [`Waiter::wait`] from another thread.
#[derive(Clone)]
pub(crate) struct Interrupter(IntImpl);

impl Interrupter {
    pub(crate) fn interrupt(&self) {
        match &self.0 {
            IntImpl::Os(i) => i.interrupt(),
            IntImpl::Poll(p) => {
                *p.interrupted.lock().unwrap_or_else(|e| e.into_inner()) = true;
                p.cv.notify_all();
            }
        }
    }
}

/// One side's view of a surface: the mapping, the bell it rings, the
/// channel it waits on and (on the creating side) the handle for the
/// other process. The fields are meant to be moved out.
pub(crate) struct Segment {
    pub(crate) map: Mapping,
    pub(crate) bell: Bell,
    pub(crate) waiter: Waiter,
    /// What the other process passes to `open` (creator side; empty on
    /// the opening side).
    pub(crate) handle: String,
    pub(crate) kind: Kind,
    pub(crate) extras: Extras,
}

/// The creator's copies of the other side's descriptors and the names to
/// remove when this side goes away. Drop it after the mapping.
pub(crate) struct Extras {
    remote: imp::Remote,
    _cleanup: imp::Cleanup,
}

impl Extras {
    /// Lets the child spawned by `cmd` inherit the other side's ends
    /// ([`Kind::Fd`] only; nothing to do for named objects and files).
    pub(crate) fn share(&self, cmd: &mut std::process::Command) {
        self.remote.share(cmd)
    }

    /// Closes this process' copies of the other side's ends (after the
    /// child was spawned), so a child that exits is seen as `Closed`.
    pub(crate) fn release_remote(&mut self) {
        self.remote.release()
    }
}

/// Creates a surface of `len` bytes (zeroed). `dir` is where named FIFOs
/// and frame files go (default: a private dir in
/// [`super::dirs::runtime`]). Falls back from `Fd` to `Named` where
/// descriptors cannot be inherited (Windows).
pub(crate) fn create(
    kind: Kind,
    len: usize,
    dir: Option<&Path>,
    role: Role,
) -> io::Result<Segment> {
    let base = dir
        .map(Path::to_path_buf)
        .unwrap_or_else(super::dirs::runtime);
    let parts = imp::create(kind, len, &base, role)?;
    Ok(assemble(parts))
}

/// Opens the surface behind `handle` (from the other side's `create`).
pub(crate) fn open(handle: &str, role: Role) -> io::Result<Segment> {
    let parts = imp::open(handle, role)?;
    Ok(assemble(parts))
}

/// What a backend returns: `waiter` None means "poll".
pub(crate) struct Parts {
    map: Mapping,
    bell: Option<imp::Bell>,
    waiter: Option<imp::Waiter>,
    handle: String,
    kind: Kind,
    remote: imp::Remote,
    cleanup: imp::Cleanup,
}

fn assemble(p: Parts) -> Segment {
    let waiter = match p.waiter {
        Some(w) => Waiter(WaitImpl::Os(w)),
        None => Waiter::polling(),
    };
    Segment {
        map: p.map,
        bell: Bell(p.bell),
        waiter,
        handle: p.handle,
        kind: p.kind,
        extras: Extras {
            remote: p.remote,
            _cleanup: p.cleanup,
        },
    }
}

/// A short id that is unique on this machine for a while (pid, counter,
/// clock), safe in object names and paths, at most 22 characters.
fn unique_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32).rotate_left(13))
        .unwrap_or(0);
    format!("{:x}-{:x}-{:06x}", std::process::id(), n, t & 0xff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("fltk-omarchy-frame-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Removes the (empty) test directories: never a tree.
    fn tidy(d: &Path) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let _ = std::fs::remove_dir(e.path());
        }
        let _ = std::fs::remove_dir(d);
    }

    fn roundtrip(kind: Kind, dir: &Path) {
        let mut a = create(kind, 64 * 1024, Some(dir), Role::Consumer).unwrap();
        let mut b = open(&a.handle.clone(), Role::Producer).unwrap();
        assert!(a.map.len() >= 64 * 1024 && b.map.len() >= 64 * 1024);
        // SAFETY: both mappings are at least 64 KiB; test-only bytes.
        unsafe {
            *b.map.ptr().add(100) = 42;
            assert_eq!(*a.map.ptr().add(100), 42, "{kind:?}: same memory");
        }
        // Producer rings present, consumer waits on it.
        b.bell.ring();
        let w = &mut a.waiter;
        let got = w.wait(Some(Duration::from_secs(2)));
        if w.poll_interval().is_none() {
            assert_eq!(got, Wait::Rung, "{kind:?}");
            assert_eq!(
                w.wait(Some(Duration::from_millis(10))),
                Wait::Timeout,
                "rings were drained"
            );
        }
        // Consumer rings input, producer waits on it.
        a.bell.ring();
        let w = &mut b.waiter;
        let got = w.wait(Some(Duration::from_secs(2)));
        if w.poll_interval().is_none() {
            assert_eq!(got, Wait::Rung, "{kind:?} input");
        }
        // Interrupt from another thread.
        let i = w.interrupter();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            i.interrupt();
        });
        let end = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = w.wait(Some(Duration::from_secs(5)));
        while got == Wait::Timeout && w.poll_interval().is_some() && std::time::Instant::now() < end
        {
            got = w.wait(Some(Duration::from_secs(5)));
        }
        assert_eq!(got, Wait::Interrupted, "{kind:?}");
        t.join().unwrap();
        a.extras.release_remote();
    }

    #[test]
    fn file_kind_shares_memory_and_polls() {
        let d = temp_dir("file");
        roundtrip(Kind::File, &d);
        // The creator removed its frame file when dropped (the private
        // directory itself stays).
        let files = |dir: &Path| -> usize {
            std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .count()
        };
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() {
                assert_eq!(files(&e.path()), 0, "{:?}", e.path());
            }
        }
        tidy(&d);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn named_kind_shares_memory_and_rings() {
        let d = temp_dir("named");
        roundtrip(Kind::Named, &d);
        tidy(&d);
    }

    #[cfg(unix)]
    #[test]
    fn fd_kind_shares_memory_and_rings_in_process() {
        let d = temp_dir("fd");
        roundtrip(Kind::Fd, &d);
        tidy(&d);
    }

    #[test]
    fn ids_are_short_and_unique() {
        let a = unique_id();
        let b = unique_id();
        assert_ne!(a, b);
        assert!(a.len() <= 22, "{a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }
}
