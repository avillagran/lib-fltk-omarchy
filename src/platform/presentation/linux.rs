use super::*;
mod wayland;
mod x11;
extern "C" {
    #[link_name = "_Z13fl_wl_displayv"]
    fn fl_wl_display() -> *mut std::ffi::c_void;
}
pub(super) fn present<W: WindowExt>(
    window: &W,
    options: PresentOptions<'_>,
) -> Result<Presentation, PresentError> {
    // SAFETY: FLTK returns null when its Wayland backend is not active.
    if unsafe { fl_wl_display() }.is_null() {
        x11::present(window, options)
    } else {
        wayland::present(window, options)
    }
}
