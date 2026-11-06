//! EWMH request and startup-notification completion on FLTK's connection.
use super::*;
use std::ffi::{c_char, c_int, c_long, c_uchar, c_ulong, c_void, CString};
type Atom = c_ulong;
#[repr(C)]
#[derive(Clone, Copy)]
union Data {
    bytes: [c_char; 20],
    longs: [c_long; 5],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct ClientMessage {
    kind: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut c_void,
    window: c_ulong,
    message_type: Atom,
    format: c_int,
    data: Data,
}
#[repr(C)]
union Event {
    message: ClientMessage,
    padding: [c_long; 24],
}
#[link(name = "X11")]
extern "C" {
    fn XDefaultRootWindow(display: *mut c_void) -> c_ulong;
    fn XInternAtom(display: *mut c_void, name: *const c_char, only_if_exists: c_int) -> Atom;
    fn XGetWindowProperty(
        display: *mut c_void,
        window: c_ulong,
        property: Atom,
        offset: c_long,
        length: c_long,
        delete: c_int,
        requested_type: Atom,
        actual_type: *mut Atom,
        actual_format: *mut c_int,
        nitems: *mut c_ulong,
        bytes_after: *mut c_ulong,
        data: *mut *mut c_uchar,
    ) -> c_int;
    fn XChangeProperty(
        display: *mut c_void,
        window: c_ulong,
        property: Atom,
        kind: Atom,
        format: c_int,
        mode: c_int,
        data: *const c_uchar,
        count: c_int,
    ) -> c_int;
    fn XSendEvent(
        display: *mut c_void,
        window: c_ulong,
        propagate: c_int,
        mask: c_long,
        event: *mut Event,
    ) -> c_int;
    fn XFlush(display: *mut c_void) -> c_int;
    fn XFree(data: *mut c_void) -> c_int;
}
unsafe fn atom(d: *mut c_void, name: &std::ffi::CStr) -> Atom {
    unsafe { XInternAtom(d, name.as_ptr(), 0) }
}
unsafe fn property(d: *mut c_void, window: c_ulong, name: Atom, expected: Atom) -> Vec<c_ulong> {
    let (mut ty, mut format, mut count, mut after, mut data) = (0, 0, 0, 0, std::ptr::null_mut());
    // SAFETY: Xlib owns the returned allocation; format 32 uses native
    // longs in XGetWindowProperty, including on LP64 platforms.
    unsafe {
        let result = XGetWindowProperty(
            d,
            window,
            name,
            0,
            2048,
            0,
            0,
            &mut ty,
            &mut format,
            &mut count,
            &mut after,
            &mut data,
        );
        let values = if result == 0 && ty == expected && format == 32 && !data.is_null() {
            std::slice::from_raw_parts(data.cast::<c_ulong>(), count as usize).to_vec()
        } else {
            Vec::new()
        };
        if !data.is_null() {
            XFree(data.cast());
        }
        values
    }
}
fn time_from_id(id: &str) -> Option<u32> {
    id.rsplit_once("_TIME")?.1.parse().ok()
}
pub(super) fn present<W: WindowExt>(
    window: &W,
    options: PresentOptions<'_>,
) -> Result<Presentation, PresentError> {
    let display = fltk::app::display();
    let xid = window.raw_handle() as c_ulong;
    if display.is_null() || xid == 0 {
        return Err(PresentError::Native("missing X11 display or window".into()));
    }
    // SAFETY: the facade supplies a live own top-level window on the UI
    // thread. All operations target it or the standard root WM protocol.
    let status = unsafe {
        let root = XDefaultRootWindow(display);
        let active = atom(display, c"_NET_ACTIVE_WINDOW");
        let id = options.startup_id.filter(|s| !s.is_empty());
        if let Some(id) = id {
            XChangeProperty(
                display,
                xid,
                atom(display, c"_NET_STARTUP_ID"),
                atom(display, c"UTF8_STRING"),
                8,
                0,
                id.as_ptr(),
                id.len() as c_int,
            );
        }
        let current = property(display, root, active, 33)
            .first()
            .copied()
            .unwrap_or(0);
        let supported =
            property(display, root, atom(display, c"_NET_SUPPORTED"), 4).contains(&active);
        let status = if !supported {
            PresentStatus::Unsupported
        } else if current == xid {
            PresentStatus::Granted
        } else {
            let time = options
                .timestamp
                .or_else(|| id.and_then(time_from_id))
                .unwrap_or(0);
            let mut event = Event { padding: [0; 24] };
            event.message = ClientMessage {
                kind: 33,
                serial: 0,
                send_event: 1,
                display,
                window: xid,
                message_type: active,
                format: 32,
                data: Data {
                    longs: [1, time as c_long, current as c_long, 0, 0],
                },
            };
            // Source=application, never pager. There is no WM acceptance
            // reply: sending successfully does not establish focus grant.
            if XSendEvent(display, root, 0, (1 << 20) | (1 << 19), &mut event) == 0 {
                return Err(PresentError::Native(
                    "X11 activation event could not be sent".into(),
                ));
            }
            PresentStatus::Requested
        };
        if let Some(id) = id {
            startup_done(display, root, xid, id);
        }
        XFlush(display);
        status
    };
    Ok(Presentation {
        backend: PresentBackend::X11,
        status,
    })
}
unsafe fn startup_done(display: *mut c_void, root: c_ulong, window: c_ulong, id: &str) {
    let escaped = id.replace('\\', "\\\\").replace('"', "\\\"");
    let message = CString::new(format!("remove: ID=\"{escaped}\"")).expect("hint validated");
    for (i, chunk) in message.as_bytes_with_nul().chunks(20).enumerate() {
        let mut bytes = [0; 20];
        for (dst, src) in bytes.iter_mut().zip(chunk) {
            *dst = *src as c_char;
        }
        let mut event = Event { padding: [0; 24] };
        event.message = ClientMessage {
            kind: 33,
            serial: 0,
            send_event: 1,
            display,
            window,
            message_type: unsafe {
                atom(
                    display,
                    if i == 0 {
                        c"_NET_STARTUP_INFO_BEGIN"
                    } else {
                        c"_NET_STARTUP_INFO"
                    },
                )
            },
            format: 8,
            data: Data { bytes },
        };
        // Startup-notification uses PropertyChangeMask, not the WM's
        // SubstructureRedirectMask. The message is chunked in bytes.
        unsafe {
            XSendEvent(display, root, 0, 1 << 22, &mut event);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_time_is_an_x_server_timestamp() {
        assert_eq!(time_from_id("launcher_TIME4294967295"), Some(u32::MAX));
        assert_eq!(time_from_id("launcher_TIME42junk"), None);
        assert_eq!(time_from_id("launcher_TIME4294967296"), None);
        assert_eq!(time_from_id("opaque"), None);
    }
}
