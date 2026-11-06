//! OS-owned presentation policy; no compositor-specific fallback or env writes.
use crate::native::{PresentBackend, PresentError, PresentOptions, PresentStatus, Presentation};
use fltk::prelude::*;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as imp;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod imp {
    use super::*;
    pub fn present<W: WindowExt>(
        _: &W,
        _: PresentOptions<'_>,
    ) -> Result<Presentation, PresentError> {
        Ok(Presentation {
            backend: PresentBackend::Other,
            status: PresentStatus::Unsupported,
        })
    }
}
pub(crate) fn present<W: WindowExt>(
    window: &W,
    options: PresentOptions<'_>,
) -> Result<Presentation, PresentError> {
    imp::present(window, options)
}
