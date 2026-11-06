//! Presentation requests for an existing application-owned FLTK window.
use fltk::{app, prelude::*};
use std::{cell::Cell, fmt};

/// Explicit startup context. Empty strings mean no hint. No environment
/// variable is read, changed or propagated by this API.
#[derive(Clone, Copy, Default)]
pub struct PresentOptions<'a> {
    /// Opaque xdg-activation token. Preferred over startup_id on Wayland.
    pub activation_token: Option<&'a str>,
    /// X11 startup notification ID; also accepted as an opaque activation
    /// token on Wayland (e.g. FileManager1's startup-id argument).
    pub startup_id: Option<&'a str>,
    /// X11 user-event time, in the X server's 32-bit millisecond domain.
    /// Otherwise parsed from startup_id's _TIME suffix, or CurrentTime.
    pub timestamp: Option<u32>,
}
impl fmt::Debug for PresentOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PresentOptions")
            .field(
                "activation_token",
                &self.activation_token.map(|_| "<redacted>"),
            )
            .field("startup_id", &self.startup_id.map(|_| "<redacted>"))
            .field("timestamp", &self.timestamp)
            .finish()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentBackend {
    Wayland,
    X11,
    Windows,
    MacOs,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentStatus {
    /// The native activation mechanism is unavailable. A hidden window
    /// may still have been mapped normally.
    Unsupported,
    /// Dispatched, without a reliable acknowledgement. In particular,
    /// Wayland can silently ignore invalid/stale tokens. This is not grant.
    Requested,
    /// Native foreground/key-window state or a synchronous native result
    /// confirms activation at the time of the request.
    Granted,
    /// The native platform explicitly refused the request.
    Refused,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presentation {
    pub backend: PresentBackend,
    pub status: PresentStatus,
}

#[derive(Debug)]
pub enum PresentError {
    WrongThread,
    DeletedWindow,
    ChildWindow,
    InvalidHint,
    Busy,
    TimedOut,
    Native(String),
}
impl fmt::Display for PresentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongThread => f.write_str("presentation requires the FLTK UI thread"),
            Self::DeletedWindow => f.write_str("the window was deleted"),
            Self::ChildWindow => f.write_str("presentation requires a top-level window"),
            Self::InvalidHint => {
                f.write_str("startup hints must contain no NUL and be at most 60 KiB")
            }
            Self::Busy => f.write_str("presentation is already in progress on this UI thread"),
            Self::TimedOut => f.write_str("native presentation handshake timed out"),
            Self::Native(e) => write!(f, "native presentation failed: {e}"),
        }
    }
}
impl std::error::Error for PresentError {}
thread_local! {static PRESENTING:Cell<bool>=const{Cell::new(false)};}

/// Maps a hidden own window and requests its native activation. Existing
/// mapped windows are not hidden/remapped. Call on the UI thread; only a
/// top-level FLTK window is accepted, never an arbitrary native window ID.
///
/// The result describes activation, not just visibility. Wayland reports
/// Requested because xdg-activation has no granted/refused acknowledgement.
/// A compositor's refusal is never bypassed with IPC or a focus fallback.
/// Startup hints are explicit and never logged or put into the environment.
pub fn present<W: WindowExt>(
    window: &mut W,
    options: PresentOptions<'_>,
) -> Result<Presentation, PresentError> {
    if !app::is_ui_thread() {
        return Err(PresentError::WrongThread);
    }
    validate(options)?;
    if window.was_deleted() {
        return Err(PresentError::DeletedWindow);
    }
    if window.parent().is_some() {
        return Err(PresentError::ChildWindow);
    }
    if PRESENTING.with(|v| v.replace(true)) {
        return Err(PresentError::Busy);
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PRESENTING.with(|v| v.set(false));
        }
    }
    let _reset = Reset;
    if !window.shown() {
        window.show();
    }
    if window.was_deleted() {
        return Err(PresentError::DeletedWindow);
    }
    crate::platform::presentation::present(window, options)
}
fn validate(options: PresentOptions<'_>) -> Result<(), PresentError> {
    for hint in [options.activation_token, options.startup_id]
        .into_iter()
        .flatten()
    {
        if hint.len() > 60 * 1024 || hint.as_bytes().contains(&0) {
            return Err(PresentError::InvalidHint);
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hints_are_bounded_and_debug_redacts_context() {
        let options = PresentOptions {
            activation_token: Some("secret-token"),
            startup_id: Some("startup-secret_TIME123"),
            timestamp: None,
        };
        assert!(validate(options).is_ok());
        assert!(!format!("{options:?}").contains("secret"));
        assert!(validate(PresentOptions {
            startup_id: Some("bad\0id"),
            ..Default::default()
        })
        .is_err());
        assert!(validate(PresentOptions {
            activation_token: Some(&"a".repeat(60 * 1024 + 1)),
            ..Default::default()
        })
        .is_err());
        assert!(validate(PresentOptions::default()).is_ok());
    }
}
