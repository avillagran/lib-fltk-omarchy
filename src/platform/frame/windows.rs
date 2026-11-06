//! Windows backend: named file mappings (`Local\` namespace, the default
//! DACL of the creator's token: only this user, admins and SYSTEM) with
//! named auto-reset events; mapped files for [`Kind::File`]. Descriptor
//! inheritance ([`Kind::Fd`]) is not used: it falls back to named objects.
//! Through kernel32 like the rest of `platform::windows` (no extra crates).

use super::{unique_id, Kind, Mapping, Parts, Role, Wait};
use std::ffi::c_void;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

type Handle = *mut c_void;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const PAGE_READWRITE: u32 = 0x04;
const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
const EVENT_MODIFY_STATE: u32 = 0x0002;
const SYNCHRONIZE: u32 = 0x0010_0000;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const INFINITE: u32 = 0xFFFF_FFFF;
const ERROR_ALREADY_EXISTS: i32 = 183;

#[repr(C)]
struct MemoryBasicInformation {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    #[cfg(target_pointer_width = "64")]
    partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileMappingW(
        file: Handle,
        sa: *mut c_void,
        protect: u32,
        size_hi: u32,
        size_lo: u32,
        name: *const u16,
    ) -> Handle;
    fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn MapViewOfFile(
        mapping: Handle,
        access: u32,
        off_hi: u32,
        off_lo: u32,
        bytes: usize,
    ) -> *mut c_void;
    fn UnmapViewOfFile(base: *const c_void) -> i32;
    fn VirtualQuery(addr: *const c_void, info: *mut MemoryBasicInformation, len: usize) -> usize;
    fn CreateEventW(sa: *mut c_void, manual_reset: i32, initial: i32, name: *const u16) -> Handle;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForMultipleObjects(count: u32, handles: *const Handle, wait_all: i32, ms: u32) -> u32;
    fn CloseHandle(h: Handle) -> i32;
    fn GetLastError() -> u32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// An owned kernel handle.
struct H(Handle);

// SAFETY: kernel handles may be used from any thread.
unsafe impl Send for H {}
unsafe impl Sync for H {}

impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: we own the handle.
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn last() -> io::Error {
    io::Error::last_os_error()
}

fn owned(h: Handle) -> io::Result<H> {
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        Err(last())
    } else {
        Ok(H(h))
    }
}

pub(crate) struct MapOs(#[allow(dead_code)] H);

fn view(mapping: H, len: Option<usize>) -> io::Result<Mapping> {
    // SAFETY: maps the whole section read-write; checked below.
    let p = unsafe { MapViewOfFile(mapping.0, FILE_MAP_ALL_ACCESS, 0, 0, len.unwrap_or(0)) };
    if p.is_null() {
        return Err(last());
    }
    let len = match len {
        Some(l) => l,
        None => {
            // SAFETY: `info` is written by VirtualQuery for our own view.
            let mut info: MemoryBasicInformation = unsafe { std::mem::zeroed() };
            let n = unsafe {
                VirtualQuery(p, &mut info, std::mem::size_of::<MemoryBasicInformation>())
            };
            if n == 0 {
                unsafe { UnmapViewOfFile(p) };
                return Err(last());
            }
            info.region_size
        }
    };
    Ok(Mapping {
        ptr: p as *mut u8,
        len,
        _os: MapOs(mapping),
    })
}

pub(crate) fn unmap(ptr: *mut u8, _len: usize) {
    // SAFETY: `ptr` is the base of our own view.
    unsafe { UnmapViewOfFile(ptr as *const c_void) };
}

// ------------------------------------------------------------------ bells

pub(crate) struct Bell(Arc<H>);

impl Bell {
    pub(crate) fn ring(&self) {
        // SAFETY: a valid event handle.
        unsafe { SetEvent(self.0 .0) };
    }
}

pub(crate) struct Waiter {
    ev: Arc<H>,
    wake: Arc<H>,
}

#[derive(Clone)]
pub(crate) struct Interrupter(Arc<H>);

impl Interrupter {
    pub(crate) fn interrupt(&self) {
        // SAFETY: a valid event handle.
        unsafe { SetEvent(self.0 .0) };
    }
}

impl Waiter {
    fn new(ev: Arc<H>) -> io::Result<Waiter> {
        // SAFETY: an unnamed auto-reset event.
        let wake = owned(unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null()) })?;
        Ok(Waiter {
            ev,
            wake: Arc::new(wake),
        })
    }

    pub(crate) fn interrupter(&self) -> Interrupter {
        Interrupter(self.wake.clone())
    }

    pub(crate) fn wait(&mut self, timeout: Option<Duration>) -> Wait {
        let ms = timeout.map_or(INFINITE, |d| {
            d.as_millis().min((INFINITE - 1) as u128) as u32
        });
        let hs = [self.wake.0, self.ev.0];
        // SAFETY: two valid handles.
        let r = unsafe { WaitForMultipleObjects(2, hs.as_ptr(), 0, ms) };
        match r {
            x if x == WAIT_OBJECT_0 => Wait::Interrupted,
            x if x == WAIT_OBJECT_0 + 1 => Wait::Rung,
            WAIT_TIMEOUT => Wait::Timeout,
            _ => Wait::Closed,
        }
    }
}

// ------------------------------------------------------- remote and names

#[derive(Default)]
pub(crate) struct Remote;

impl Remote {
    pub(crate) fn share(&self, _cmd: &mut std::process::Command) {}
    pub(crate) fn release(&mut self) {}
}

#[derive(Default)]
pub(crate) struct Cleanup {
    file: Option<PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if let Some(f) = &self.file {
            // Only the exact file this process created (never a tree). It
            // fails while the other side still maps it; harmless.
            let _ = fs::remove_file(f);
        }
    }
}

fn names(id: &str) -> (String, String, String) {
    let base = format!("Local\\fltk-omarchy-{id}");
    (
        base.clone(),
        format!("{base}-present"),
        format!("{base}-input"),
    )
}

fn event(name: &str, create: bool) -> io::Result<Arc<H>> {
    let w = wide(name);
    // SAFETY: NUL-terminated name.
    let h = unsafe {
        if create {
            CreateEventW(std::ptr::null_mut(), 0, 0, w.as_ptr())
        } else {
            OpenEventW(EVENT_MODIFY_STATE | SYNCHRONIZE, 0, w.as_ptr())
        }
    };
    let h = owned(h)?;
    // SAFETY: no arguments.
    if create && unsafe { GetLastError() } as i32 == ERROR_ALREADY_EXISTS {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "frame event name taken",
        ));
    }
    Ok(Arc::new(h))
}

fn pick<T>(role: Role, present: T, input: T) -> (T, T) {
    match role {
        Role::Producer => (present, input),
        Role::Consumer => (input, present),
    }
}

/// A private directory for frame files under `base`.
fn private_dir(base: &Path) -> io::Result<PathBuf> {
    let dir = base.join("fltk-omarchy-frames");
    match fs::create_dir(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    if !fs::symlink_metadata(&dir)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "frame dir is not a directory",
        ));
    }
    Ok(dir)
}

pub(crate) fn create(kind: Kind, len: usize, base: &Path, role: Role) -> io::Result<Parts> {
    let size = len as u64;
    match kind {
        Kind::Fd | Kind::Named => {
            let id = unique_id();
            let (mem, present, input) = names(&id);
            let w = wide(&mem);
            // SAFETY: a new pagefile-backed section with a NUL-terminated name.
            let h = owned(unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    std::ptr::null_mut(),
                    PAGE_READWRITE,
                    (size >> 32) as u32,
                    size as u32,
                    w.as_ptr(),
                )
            })?;
            if unsafe { GetLastError() } as i32 == ERROR_ALREADY_EXISTS {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "frame mapping name taken",
                ));
            }
            let map = view(h, Some(len))?;
            let (ring, wait) = pick(role, event(&present, true)?, event(&input, true)?);
            Ok(Parts {
                map,
                bell: Some(Bell(ring)),
                waiter: Some(Waiter::new(wait)?),
                handle: format!("named:{id}"),
                kind: Kind::Named,
                remote: Remote,
                cleanup: Cleanup::default(),
            })
        }
        Kind::File => {
            let path = private_dir(base)?.join(format!("{}.frame", unique_id()));
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)?;
            let cleanup = Cleanup {
                file: Some(path.clone()),
            };
            f.set_len(size)?;
            // SAFETY: a section over our open file handle.
            let h = owned(unsafe {
                CreateFileMappingW(
                    f.as_raw_handle() as Handle,
                    std::ptr::null_mut(),
                    PAGE_READWRITE,
                    0,
                    0,
                    std::ptr::null(),
                )
            })?;
            let map = view(h, Some(len))?;
            let p = path
                .to_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path is not UTF-8"))?;
            Ok(Parts {
                map,
                bell: None,
                waiter: None,
                handle: format!("file:{p}"),
                kind,
                remote: Remote,
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
    if let Some(id) = handle.strip_prefix("named:") {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(bad(handle));
        }
        let (mem, present, input) = names(id);
        let w = wide(&mem);
        // SAFETY: NUL-terminated name.
        let h = owned(unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, w.as_ptr()) })?;
        let map = view(h, None)?;
        let (ring, wait) = pick(role, event(&present, false)?, event(&input, false)?);
        return Ok(Parts {
            map,
            bell: Some(Bell(ring)),
            waiter: Some(Waiter::new(wait)?),
            handle: String::new(),
            kind: Kind::Named,
            remote: Remote,
            cleanup: Cleanup::default(),
        });
    }
    if let Some(path) = handle.strip_prefix("file:") {
        let f = OpenOptions::new().read(true).write(true).open(path)?;
        let len = f.metadata()?.len() as usize;
        // SAFETY: a section over our open file handle.
        let h = owned(unsafe {
            CreateFileMappingW(
                f.as_raw_handle() as Handle,
                std::ptr::null_mut(),
                PAGE_READWRITE,
                0,
                0,
                std::ptr::null(),
            )
        })?;
        let map = view(h, Some(len))?;
        return Ok(Parts {
            map,
            bell: None,
            waiter: None,
            handle: String::new(),
            kind: Kind::File,
            remote: Remote,
            cleanup: Cleanup::default(),
        });
    }
    Err(bad(handle))
}
