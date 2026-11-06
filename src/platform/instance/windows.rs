//! Named pipes (Windows), through kernel32/advapi32/user32 like the rest
//! of `platform::windows` (no extra crates).
//!
//! The pipe name is `\\.\pipe\fltk-omarchy-<app id>-<hash>`, the hash of
//! the per-user runtime dir (so two users, or a test with its own runtime
//! dir, never meet). The first pipe instance is created with
//! `FILE_FLAG_FIRST_PIPE_INSTANCE`: that is the single-instance lock, and
//! it disappears with the process, so there is no stale state. The default
//! pipe DACL only lets the creator (and admins/SYSTEM) write, remote
//! clients are rejected, and a client checks that the pipe's owner is its
//! own user before sending anything (no squatting by another user).
//!
//! Synchronous pipe I/O has no timeouts: a watchdog thread per connection
//! cancels pending I/O on the handle (`CancelIoEx`) at the deadline.

use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

type Handle = *mut c_void;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_TYPE_BYTE: u32 = 0;
const PIPE_READMODE_BYTE: u32 = 0;
const PIPE_WAIT: u32 = 0;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const SECURITY_SQOS_PRESENT: u32 = 0x0010_0000;
const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_SEM_TIMEOUT: i32 = 121;
const ERROR_PIPE_CONNECTED: i32 = 535;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_OWNER_CLASS: u32 = 4;
const SE_KERNEL_OBJECT: u32 = 6;
const OWNER_SECURITY_INFORMATION: u32 = 0x0000_0001;
const SW_RESTORE: i32 = 9;
const BUFFER: u32 = 64 * 1024;

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_size: u32,
        in_size: u32,
        default_timeout: u32,
        sa: *mut c_void,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
    fn DisconnectNamedPipe(pipe: Handle) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *mut c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn WaitNamedPipeW(name: *const u16, timeout_ms: u32) -> i32;
    fn CancelIoEx(file: Handle, overlapped: *mut c_void) -> i32;
    fn FlushFileBuffers(file: Handle) -> i32;
    fn GetNamedPipeServerProcessId(pipe: Handle, pid: *mut u32) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn CloseHandle(h: Handle) -> i32;
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        info: *mut c_void,
        len: u32,
        ret_len: *mut u32,
    ) -> i32;
    fn GetSecurityInfo(
        handle: Handle,
        object_type: u32,
        info: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        sd: *mut *mut c_void,
    ) -> u32;
    fn EqualSid(a: *mut c_void, b: *mut c_void) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn AllowSetForegroundWindow(pid: u32) -> i32;
    fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
    fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
    fn IsIconic(hwnd: *mut c_void) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// FNV-1a, 64 bit: a stable short tag of the runtime dir.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

pub(crate) struct Endpoint {
    name: String,
    wname: Vec<u16>,
}

pub(crate) fn endpoint(app_id: &str, runtime: &Path, _create: bool) -> io::Result<Endpoint> {
    let tag = fnv(&runtime.to_string_lossy().to_lowercase());
    let name = format!(r"\\.\pipe\fltk-omarchy-{app_id}-{tag:016x}");
    Ok(Endpoint {
        wname: wide(&name),
        name,
    })
}

impl Endpoint {
    pub(crate) fn describe(&self) -> String {
        self.name.clone()
    }
}

fn create_instance(ep: &Endpoint, first: bool) -> io::Result<Handle> {
    let open = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    // SAFETY: NUL-terminated name; default security (only the creator writes).
    let h = unsafe {
        CreateNamedPipeW(
            ep.wname.as_ptr(),
            open,
            mode,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER,
            BUFFER,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(h)
    }
}

pub(crate) fn claim(ep: &Endpoint) -> io::Result<Option<Listener>> {
    match create_instance(ep, true) {
        Ok(h) => Ok(Some(Listener {
            ep: Endpoint {
                name: ep.name.clone(),
                wname: ep.wname.clone(),
            },
            next: std::sync::Mutex::new(h as usize),
        })),
        // The pipe exists: another instance (ours or another user's; the
        // client side checks the owner).
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => Ok(None),
        Err(e) => Err(e),
    }
}

pub(crate) struct Listener {
    ep: Endpoint,
    /// The instance that waits for the next client (a handle as usize).
    next: std::sync::Mutex<usize>,
}

impl Listener {
    pub(crate) fn accept(&self) -> io::Result<Conn> {
        let h = *self.next.lock().unwrap() as Handle;
        // SAFETY: a pipe instance we created.
        let ok = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) } != 0;
        if !ok {
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(ERROR_PIPE_CONNECTED) {
                return Err(e);
            }
        }
        // Create the next instance before handing this one out, so clients
        // never see "no such pipe" while the primary runs.
        let next = match create_instance(&self.ep, false) {
            Ok(n) => n,
            Err(e) => {
                // Keep this instance for the next client; drop this one.
                // SAFETY: our connected pipe instance.
                unsafe { DisconnectNamedPipe(h) };
                return Err(e);
            }
        };
        *self.next.lock().unwrap() = next as usize;
        Ok(Conn::new(h, true))
    }

    pub(crate) fn wake(&self) {
        // SAFETY: NUL-terminated name; the handle is closed at once.
        let h = unsafe {
            CreateFileW(
                self.ep.wname.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                std::ptr::null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(h) };
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let h = *self.next.lock().unwrap() as Handle;
        // SAFETY: our pipe instance, closed once.
        unsafe { CloseHandle(h) };
    }
}

/// The SID of this process's user is the owner of `handle`.
/// The SID a token information class starts with (`TOKEN_USER` and
/// `TOKEN_OWNER` both begin with a `PSID`), in a buffer that owns it.
///
/// # Safety
/// `token` must be an open token handle with `TOKEN_QUERY`.
unsafe fn token_sid(token: Handle, class: u32) -> Option<Vec<u64>> {
    let mut len = 0u32;
    GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut len);
    let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
    let ok = GetTokenInformation(token, class, buf.as_mut_ptr().cast(), len, &mut len) != 0;
    ok.then_some(buf)
}

fn owned_by_me(handle: Handle) -> bool {
    // SAFETY: standard token/SID queries with checked results; buffers are
    // sized by the first call; the security descriptor is freed.
    unsafe {
        let mut token: Handle = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        // Our user, and the default owner of what we create: an elevated
        // process creates objects owned by the Administrators group, so a
        // pipe made by our own (elevated) primary carries that owner.
        let user = token_sid(token, TOKEN_USER_CLASS);
        let default_owner = token_sid(token, TOKEN_OWNER_CLASS);
        CloseHandle(token);
        let (mut owner, mut sd) = (std::ptr::null_mut(), std::ptr::null_mut());
        let rc = GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sd,
        );
        let same = rc == 0
            && !owner.is_null()
            && [user, default_owner].iter().flatten().any(|buf| {
                let sid = *(buf.as_ptr() as *const *mut c_void);
                EqualSid(owner, sid) != 0
            });
        if !sd.is_null() {
            LocalFree(sd);
        }
        same
    }
}

pub(crate) fn connect(ep: &Endpoint, timeout: Duration) -> io::Result<Conn> {
    let deadline = std::time::Instant::now() + timeout;
    let h = loop {
        // SAFETY: NUL-terminated name. SQOS: the server may identify us, not impersonate us.
        let h = unsafe {
            CreateFileW(
                ep.wname.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                std::ptr::null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            break h;
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(ERROR_PIPE_BUSY) => {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "pipe busy"));
                }
                // SAFETY: NUL-terminated name.
                if unsafe {
                    WaitNamedPipeW(
                        ep.wname.as_ptr(),
                        left.as_millis().clamp(1, u32::MAX as u128) as u32,
                    )
                } == 0
                {
                    let e = io::Error::last_os_error();
                    if e.raw_os_error() == Some(ERROR_SEM_TIMEOUT) {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "pipe busy"));
                    }
                }
            }
            Some(ERROR_FILE_NOT_FOUND) => {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no instance"))
            }
            _ => return Err(e),
        }
    };
    if !owned_by_me(h) {
        // SAFETY: the handle we just opened.
        unsafe { CloseHandle(h) };
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is served by another user", ep.name),
        ));
    }
    // Let the primary come to the foreground for this request (activation).
    let mut pid = 0u32;
    // SAFETY: valid pipe handle and out pointer.
    if unsafe { GetNamedPipeServerProcessId(h, &mut pid) } != 0 && pid != 0 {
        unsafe { AllowSetForegroundWindow(pid) };
    }
    let mut c = Conn::new(h, false);
    c.set_timeout(Some(timeout))?;
    Ok(c)
}

/// A watchdog that cancels pending I/O on a handle at a deadline.
struct Watchdog {
    tx: mpsc::Sender<Option<Duration>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watchdog {
    fn new(handle: usize) -> Watchdog {
        let (tx, rx) = mpsc::channel::<Option<Duration>>();
        let thread = std::thread::Builder::new()
            .name("omarchy-ipc-deadline".into())
            .spawn(move || {
                let mut wait: Option<Duration> = None;
                loop {
                    let msg = match wait {
                        Some(d) => rx.recv_timeout(d),
                        None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                    };
                    match msg {
                        Ok(d) => wait = d,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            // SAFETY: the owner joins this thread before closing the handle.
                            unsafe { CancelIoEx(handle as Handle, std::ptr::null_mut()) };
                            wait = None;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .ok();
        Watchdog { tx, thread }
    }
}

pub(crate) struct Conn {
    watchdog: Option<Watchdog>,
    file: File,
    server: bool,
}

impl Conn {
    fn new(h: Handle, server: bool) -> Conn {
        // SAFETY: we own the handle; File closes it.
        let file = unsafe { File::from_raw_handle(h) };
        Conn {
            watchdog: Some(Watchdog::new(h as usize)),
            file,
            server,
        }
    }

    pub(crate) fn set_timeout(&mut self, d: Option<Duration>) -> io::Result<()> {
        if let Some(w) = &self.watchdog {
            let _ = w.tx.send(d);
        }
        Ok(())
    }

    fn map(e: io::Error) -> io::Error {
        // ERROR_OPERATION_ABORTED (995): the watchdog cancelled the call.
        if e.raw_os_error() == Some(995) {
            io::Error::new(io::ErrorKind::TimedOut, "timed out")
        } else {
            e
        }
    }

    pub(crate) fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.file.read(buf) {
            // The other end closed: end of stream.
            Err(e) if e.raw_os_error() == Some(109) => Ok(0), // ERROR_BROKEN_PIPE
            r => r.map_err(Self::map),
        }
    }

    pub(crate) fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf).map_err(Self::map)
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        let h = self.file.as_raw_handle() as Handle;
        if self.server {
            // Let the client read the reply before the instance goes away
            // (bounded by the watchdog's current deadline).
            // SAFETY: our open pipe handle.
            unsafe {
                FlushFileBuffers(h);
                DisconnectNamedPipe(h);
            }
        }
        if let Some(mut w) = self.watchdog.take() {
            drop(w.tx);
            if let Some(t) = w.thread.take() {
                let _ = t.join();
            }
        }
        // `file` closes the handle after the watchdog is gone.
    }
}

/// Restores a minimized window and brings it to the foreground (allowed
/// when the requesting process called `AllowSetForegroundWindow`).
pub(crate) fn raise_native<W: fltk::prelude::WindowExt>(win: &W) {
    let hwnd: *mut c_void = win.raw_handle();
    if hwnd.is_null() {
        return;
    }
    // SAFETY: a window handle of this process, from FLTK.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}
