use super::*;
use std::ffi::c_void;
#[link(name = "user32")]
extern "system" {
    fn IsIconic(window: *mut c_void) -> i32;
    fn ShowWindow(window: *mut c_void, command: i32) -> i32;
    fn SetForegroundWindow(window: *mut c_void) -> i32;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, pid: *mut u32) -> u32;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
}
pub(super) fn present<W: WindowExt + ?Sized>(
    window: &W,
    _: PresentOptions<'_>,
) -> Result<Presentation, PresentError> {
    let hwnd = window.raw_handle();
    if hwnd.is_null() {
        return Err(PresentError::Native("missing HWND".into()));
    }
    // SAFETY: the UI-thread facade supplies a live FLTK window; check its
    // native process ownership before any show/foreground operation.
    let status = unsafe {
        let mut pid = 0;
        if GetWindowThreadProcessId(hwnd, &mut pid) == 0 || pid != GetCurrentProcessId() {
            return Err(PresentError::Native(
                "window is not owned by this process".into(),
            ));
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, 9);
        }
        if GetForegroundWindow() == hwnd || SetForegroundWindow(hwnd) != 0 {
            PresentStatus::Granted
        } else {
            PresentStatus::Refused
        }
    };
    Ok(Presentation {
        backend: PresentBackend::Windows,
        status,
    })
}
