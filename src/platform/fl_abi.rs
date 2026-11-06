//! FLTK's event fields (`Fl::e_keysym`, `Fl::e_text`…) as linkable
//! statics, for `focus::press` / `focus::click_at` key and click injection.
//! They are C++ static members, so their symbol names follow the
//! compiler's C++ ABI: Itanium on Linux, macOS and MinGW, Microsoft's on
//! MSVC (64-bit `PEAD` pointers).

use std::ffi::c_char;

/// Remap a native subwindow event to its top-level window, preserving
/// coordinates across nested dispatch and unwinding. UI thread only.
#[cfg(feature = "gpu")]
pub(crate) fn with_event_offset<T>(dx: i32, dy: i32, f: impl FnOnce() -> T) -> T {
    struct Restore(i32, i32);
    impl Drop for Restore {
        fn drop(&mut self) {
            // SAFETY: this scope runs on FLTK's UI thread.
            unsafe {
                FL_E_X = self.0;
                FL_E_Y = self.1;
            }
        }
    }
    // SAFETY: FLTK owns these event globals; only this UI dispatch runs.
    let restore = unsafe {
        let old = Restore(FL_E_X, FL_E_Y);
        FL_E_X = old.0.saturating_add(dx);
        FL_E_Y = old.1.saturating_add(dy);
        old
    };
    let value = f();
    drop(restore);
    value
}

#[cfg(not(target_env = "msvc"))]
extern "C" {
    #[link_name = "_ZN2Fl8e_keysymE"]
    pub(crate) static mut FL_E_KEYSYM: i32;
    #[link_name = "_ZN2Fl17e_original_keysymE"]
    pub(crate) static mut FL_E_ORIGINAL_KEYSYM: i32;
    #[link_name = "_ZN2Fl7e_stateE"]
    pub(crate) static mut FL_E_STATE: i32;
    #[link_name = "_ZN2Fl6e_textE"]
    pub(crate) static mut FL_E_TEXT: *mut c_char;
    #[link_name = "_ZN2Fl8e_lengthE"]
    pub(crate) static mut FL_E_LENGTH: i32;
    #[link_name = "_ZN2Fl3e_xE"]
    pub(crate) static mut FL_E_X: i32;
    #[link_name = "_ZN2Fl3e_yE"]
    pub(crate) static mut FL_E_Y: i32;
    #[link_name = "_ZN2Fl8e_x_rootE"]
    pub(crate) static mut FL_E_X_ROOT: i32;
    #[link_name = "_ZN2Fl8e_y_rootE"]
    pub(crate) static mut FL_E_Y_ROOT: i32;
    #[link_name = "_ZN2Fl8e_clicksE"]
    pub(crate) static mut FL_E_CLICKS: i32;
    #[link_name = "_ZN2Fl4e_dyE"]
    pub(crate) static mut FL_E_DY: i32;
}

#[cfg(target_env = "msvc")]
extern "C" {
    #[link_name = "?e_keysym@Fl@@2HA"]
    pub(crate) static mut FL_E_KEYSYM: i32;
    #[link_name = "?e_original_keysym@Fl@@2HA"]
    pub(crate) static mut FL_E_ORIGINAL_KEYSYM: i32;
    #[link_name = "?e_state@Fl@@2HA"]
    pub(crate) static mut FL_E_STATE: i32;
    #[cfg_attr(target_pointer_width = "64", link_name = "?e_text@Fl@@2PEADEA")]
    #[cfg_attr(target_pointer_width = "32", link_name = "?e_text@Fl@@2PADA")]
    pub(crate) static mut FL_E_TEXT: *mut c_char;
    #[link_name = "?e_length@Fl@@2HA"]
    pub(crate) static mut FL_E_LENGTH: i32;
    #[link_name = "?e_x@Fl@@2HA"]
    pub(crate) static mut FL_E_X: i32;
    #[link_name = "?e_y@Fl@@2HA"]
    pub(crate) static mut FL_E_Y: i32;
    #[link_name = "?e_x_root@Fl@@2HA"]
    pub(crate) static mut FL_E_X_ROOT: i32;
    #[link_name = "?e_y_root@Fl@@2HA"]
    pub(crate) static mut FL_E_Y_ROOT: i32;
    #[link_name = "?e_clicks@Fl@@2HA"]
    pub(crate) static mut FL_E_CLICKS: i32;
    #[link_name = "?e_dy@Fl@@2HA"]
    pub(crate) static mut FL_E_DY: i32;
}

// Plain C functions of cfltk (no C++ mangling): the same name on every OS.
extern "C" {
    /// `Fl::belowmouse(w)`; null clears it (fltk-rs only sets a widget).
    pub(crate) fn Fl_set_belowmouse(w: *mut std::ffi::c_void);
}

/// `void fl_scroll(int X, int Y, int W, int H, int dx, int dy,
/// void (*draw_area)(void*, int, int, int, int), void* data)`: moves the
/// pixels of a window area by (dx, dy) and calls `draw_area` for the parts
/// it exposed (what `Fl_Scroll` uses). Present on every FLTK backend.
#[cfg(not(target_env = "msvc"))]
pub(crate) type DrawArea = extern "C" fn(*mut std::ffi::c_void, i32, i32, i32, i32);

#[cfg(not(target_env = "msvc"))]
extern "C" {
    #[link_name = "_Z9fl_scrolliiiiiiPFvPviiiiES_"]
    pub(crate) fn fl_scroll(
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        dx: i32,
        dy: i32,
        draw_area: DrawArea,
        data: *mut std::ffi::c_void,
    );
}

/// Moves the pixels of a window area by (dx, dy) with FLTK's `fl_scroll`
/// and calls `draw` for each exposed rectangle; false when this build
/// cannot (MSVC: the caller redraws everything, as before).
pub(crate) fn scroll_pixels(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    dx: i32,
    dy: i32,
    draw: &mut dyn FnMut(i32, i32, i32, i32),
) -> bool {
    #[cfg(not(target_env = "msvc"))]
    {
        extern "C" fn trampoline(data: *mut std::ffi::c_void, x: i32, y: i32, w: i32, h: i32) {
            // SAFETY: `data` is the `&mut &mut dyn FnMut` passed below, alive
            // for the duration of the fl_scroll call that invokes us.
            let f = unsafe { &mut *(data as *mut &mut dyn FnMut(i32, i32, i32, i32)) };
            f(x, y, w, h);
        }
        let mut f: &mut dyn FnMut(i32, i32, i32, i32) = draw;
        // SAFETY: FLTK's own function, on the UI thread inside a draw; the
        // callback data outlives the call.
        unsafe {
            fl_scroll(
                x,
                y,
                w,
                h,
                dx,
                dy,
                trampoline,
                &mut f as *mut _ as *mut std::ffi::c_void,
            )
        };
        true
    }
    #[cfg(target_env = "msvc")]
    {
        let _ = (x, y, w, h, dx, dy, draw);
        false
    }
}
