//! OS-owned presentation policy; no compositor-specific fallback or env writes.
use crate::native::{PresentBackend, PresentError, PresentOptions, PresentStatus, Presentation};
use fltk::prelude::*;
use std::task::Poll;
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
    pub fn present<W: WindowExt + ?Sized>(
        _: &W,
        _: PresentOptions<'_>,
    ) -> Result<Presentation, PresentError> {
        Ok(Presentation {
            backend: PresentBackend::Other,
            status: PresentStatus::Unsupported,
        })
    }
}
pub(crate) fn present<W: WindowExt + ?Sized>(
    window: &W,
    options: PresentOptions<'_>,
) -> Result<Presentation, PresentError> {
    imp::present(window, options)
}

pub(super) struct OwnedOptions {
    activation_token: Option<String>,
    startup_id: Option<String>,
    timestamp: Option<u32>,
}
impl OwnedOptions {
    fn new(options: PresentOptions<'_>) -> Self {
        Self {
            activation_token: options.activation_token.map(str::to_owned),
            startup_id: options.startup_id.map(str::to_owned),
            timestamp: options.timestamp,
        }
    }
    fn borrowed(&self) -> PresentOptions<'_> {
        PresentOptions {
            activation_token: self.activation_token.as_deref(),
            startup_id: self.startup_id.as_deref(),
            timestamp: self.timestamp,
        }
    }
}

/// Captured before mapping can synchronously change FLTK's current event.
pub(crate) struct InputContext {
    #[cfg(target_os = "linux")]
    linux: linux::InputContext,
}
impl InputContext {
    pub(crate) fn capture() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            linux: linux::InputContext::capture(),
        }
    }
    pub(crate) fn begin<W: WindowExt + ?Sized>(
        self,
        window: &W,
        options: PresentOptions<'_>,
    ) -> Result<Request, PresentError> {
        #[cfg(target_os = "linux")]
        {
            self.linux.begin(window, options).map(|request| Request {
                inner: Inner::Linux(request),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = window;
            Ok(Request {
                inner: Inner::Deferred(Some(OwnedOptions::new(options))),
            })
        }
    }
}
enum Inner {
    #[cfg(target_os = "linux")]
    Linux(linux::Request),
    #[cfg(not(target_os = "linux"))]
    Deferred(Option<OwnedOptions>),
}
pub(crate) struct Request {
    inner: Inner,
}
impl Request {
    pub(crate) fn poll<W: WindowExt + ?Sized>(
        &mut self,
        window: &W,
    ) -> Poll<Result<Presentation, PresentError>> {
        match &mut self.inner {
            #[cfg(target_os = "linux")]
            Inner::Linux(request) => request.poll(window),
            #[cfg(not(target_os = "linux"))]
            Inner::Deferred(options) => Poll::Ready(match options.take() {
                Some(options) => imp::present(window, options.borrowed()),
                None => Err(PresentError::Cancelled),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deferred_hints_own_exact_context_without_importing_environment() {
        let owned = {
            let token = String::from("opaque-token");
            let startup = String::from("  startup-\u{1f4c1}_TIME123  ");
            OwnedOptions::new(PresentOptions {
                activation_token: Some(&token),
                startup_id: Some(&startup),
                timestamp: Some(321),
            })
        };
        let options = owned.borrowed();
        assert_eq!(options.activation_token, Some("opaque-token"));
        assert_eq!(options.startup_id, Some("  startup-\u{1f4c1}_TIME123  "));
        assert_eq!(options.timestamp, Some(321));
        assert!(!format!("{options:?}").contains("opaque-token"));
        assert!(!format!("{options:?}").contains("startup-"));
    }
}
