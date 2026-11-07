use super::*;
use std::ffi::{c_char, c_void};
#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
}
pub(super) fn present<W: WindowExt + ?Sized>(
    window: &W,
    _: PresentOptions<'_>,
) -> Result<Presentation, PresentError> {
    type Object = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type Boolean = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i8;
    type Activate = unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> i8;
    type Order = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
    type Responds = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i8;
    let raw = window.raw_handle();
    if raw.is_null() {
        return Err(PresentError::Native("missing Cocoa window".into()));
    }
    // SAFETY: UI-thread AppKit calls through each selector's exact ABI.
    // FLTK supplies its own NSWindow; check the selector before ordering it.
    let status = unsafe {
        let msg = objc_msgSend as unsafe extern "C" fn();
        let object: Object = std::mem::transmute(msg);
        let boolean: Boolean = std::mem::transmute(msg);
        let activate: Activate = std::mem::transmute(msg);
        let order: Order = std::mem::transmute(msg);
        let responds: Responds = std::mem::transmute(msg);
        let order_sel = sel_registerName(c"makeKeyAndOrderFront:".as_ptr());
        if responds(
            raw,
            sel_registerName(c"respondsToSelector:".as_ptr()),
            order_sel,
        ) == 0
        {
            return Err(PresentError::Native(
                "FLTK did not supply an NSWindow".into(),
            ));
        }
        let cls = objc_getClass(c"NSRunningApplication".as_ptr());
        if cls.is_null() {
            return Ok(Presentation {
                backend: PresentBackend::MacOs,
                status: PresentStatus::Unsupported,
            });
        }
        let application = object(cls, sel_registerName(c"currentApplication".as_ptr()));
        if application.is_null() {
            return Err(PresentError::Native("missing Cocoa application".into()));
        }
        if activate(
            application,
            sel_registerName(c"activateWithOptions:".as_ptr()),
            0,
        ) == 0
        {
            PresentStatus::Refused
        } else {
            order(raw, order_sel, std::ptr::null_mut());
            let app = object(
                objc_getClass(c"NSApplication".as_ptr()),
                sel_registerName(c"sharedApplication".as_ptr()),
            );
            if !app.is_null()
                && boolean(app, sel_registerName(c"isActive".as_ptr())) != 0
                && boolean(raw, sel_registerName(c"isKeyWindow".as_ptr())) != 0
            {
                PresentStatus::Granted
            } else {
                PresentStatus::Requested
            }
        }
    };
    Ok(Presentation {
        backend: PresentBackend::MacOs,
        status,
    })
}
