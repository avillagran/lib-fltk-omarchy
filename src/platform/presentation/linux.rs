use super::*;
mod wayland;
mod x11;
extern "C" {
    #[link_name = "_Z13fl_wl_displayv"]
    fn fl_wl_display() -> *mut std::ffi::c_void;
}
pub(super) fn present<W: WindowExt + ?Sized>(
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

pub(super) struct InputContext {
    wayland: Option<wayland::InputContext>,
}
impl InputContext {
    pub(super) fn capture() -> Self {
        // SAFETY: the UI-thread facade captures before mapping. A null
        // display means there can be no current native Wayland permission.
        let wayland = if unsafe { fl_wl_display() }.is_null() {
            None
        } else {
            Some(wayland::InputContext::capture())
        };
        Self { wayland }
    }
    pub(super) fn begin<W: WindowExt + ?Sized>(
        self,
        window: &W,
        options: PresentOptions<'_>,
    ) -> Result<Request, PresentError> {
        // SAFETY: show() has selected FLTK's actual backend. A display that
        // opened during mapping receives an empty permission snapshot rather
        // than borrowing an event that happened after capture.
        if unsafe { fl_wl_display() }.is_null() {
            Ok(Request::X11(Some(OwnedOptions::new(options))))
        } else {
            wayland::Request::begin(
                window,
                options,
                self.wayland.unwrap_or_else(wayland::InputContext::empty),
            )
            .map(Request::Wayland)
        }
    }
}
pub(super) enum Request {
    Wayland(wayland::Request),
    X11(Option<OwnedOptions>),
}
impl Request {
    pub(super) fn poll<W: WindowExt + ?Sized>(
        &mut self,
        window: &W,
    ) -> Poll<Result<Presentation, PresentError>> {
        match self {
            Self::Wayland(request) => request.poll(window),
            Self::X11(options) => Poll::Ready(match options.take() {
                Some(options) => x11::present(window, options.borrowed()),
                None => Err(PresentError::Cancelled),
            }),
        }
    }
}
