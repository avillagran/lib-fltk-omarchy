//! Presentation requests for an existing application-owned FLTK window.
use fltk::{app, prelude::*};
use std::{cell::Cell, fmt, task::Poll};

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

#[derive(Clone, Debug)]
pub enum PresentError {
    WrongThread,
    DeletedWindow,
    ChildWindow,
    InvalidHint,
    Busy,
    TimedOut,
    Cancelled,
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
            Self::Cancelled => f.write_str("native presentation was cancelled"),
            Self::Native(e) => write!(f, "native presentation failed: {e}"),
        }
    }
}
impl std::error::Error for PresentError {}
thread_local! {static PRESENTING:Cell<bool>=const{Cell::new(false)};}
thread_local! {static ASYNC_ACTIVE:Cell<bool>=const{Cell::new(false)};}

struct CallGuard;
impl CallGuard {
    fn enter() -> Result<Self, PresentError> {
        if PRESENTING.with(|value| value.replace(true)) {
            Err(PresentError::Busy)
        } else {
            Ok(Self)
        }
    }
}
impl Drop for CallGuard {
    fn drop(&mut self) {
        PRESENTING.with(|value| value.set(false));
    }
}
struct AsyncLease;
impl AsyncLease {
    fn acquire() -> Result<Self, PresentError> {
        if ASYNC_ACTIVE.with(|value| value.replace(true)) {
            Err(PresentError::Busy)
        } else {
            Ok(Self)
        }
    }
}
impl Drop for AsyncLease {
    fn drop(&mut self) {
        ASYNC_ACTIVE.with(|value| value.set(false));
    }
}

/// One application-owned window's manually polled presentation request.
///
/// Call [`poll`](Self::poll) once during each ordinary UI iteration, yielding
/// on `Pending`. Wayland discovery and token replies never wait for the
/// compositor inside a poll. Other backends call their native presentation
/// APIs on the first poll; X11's existing server roundtrips can still wait.
///
/// The handle is UI-thread-only and is neither `Send` nor `Sync`. It tracks
/// the original window without owning or extending the widget's lifetime.
/// Check application navigation/modal ownership before every poll; cancel or
/// drop the handle when that context changes. Cancellation before dispatch
/// prevents activation. It cannot undo normal mapping or a terminal native
/// request.
///
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// require_send::<fltk_omarchy::native::PresentRequest>();
/// ```
/// ```compile_fail
/// fn require_sync<T: Sync>() {}
/// require_sync::<fltk_omarchy::native::PresentRequest>();
/// ```
pub struct PresentRequest {
    target: Box<dyn WindowExt>,
    request: Option<crate::platform::presentation::Request>,
    result: Option<Result<Presentation, PresentError>>,
    lease: Option<AsyncLease>,
}
impl fmt::Debug for PresentRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PresentRequest")
            .field("pending", &self.is_pending())
            .finish()
    }
}
impl PresentRequest {
    /// Advance native work once, without dispatching application callbacks
    /// during Wayland handshakes. Repeated terminal polls return the cached
    /// result and never resend activation.
    pub fn poll(&mut self) -> Poll<Result<Presentation, PresentError>> {
        if !app::is_ui_thread() {
            return Poll::Ready(Err(PresentError::WrongThread));
        }
        if let Some(result) = &self.result {
            return Poll::Ready(result.clone());
        }
        if self.target.was_deleted() {
            return self.finish(Err(PresentError::DeletedWindow));
        }
        if self.target.parent().is_some() {
            return self.finish(Err(PresentError::ChildWindow));
        }
        if !self.target.shown() {
            return self.finish(Err(PresentError::Cancelled));
        }
        let _call = match CallGuard::enter() {
            Ok(guard) => guard,
            Err(error) => return self.finish(Err(error)),
        };
        let result = self
            .request
            .as_mut()
            .expect("pending native presentation")
            .poll(self.target.as_ref());
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => self.finish(result),
        }
    }

    /// Cancel pending native work and release its private resources. Returns
    /// false after completion or a previous cancellation, preserving the
    /// terminal result rather than claiming to undo dispatched activation.
    pub fn cancel(&mut self) -> bool {
        if self.result.is_some() {
            return false;
        }
        let _ = self.finish(Err(PresentError::Cancelled));
        true
    }

    pub fn is_pending(&self) -> bool {
        self.result.is_none()
    }

    fn finish(
        &mut self,
        result: Result<Presentation, PresentError>,
    ) -> Poll<Result<Presentation, PresentError>> {
        drop(self.request.take());
        self.result = Some(result.clone());
        // A completed handle must not release another request's lease when
        // it is dropped later. Taking ownership here makes release one-shot.
        drop(self.lease.take());
        Poll::Ready(result)
    }
}

/// Map an owned window and begin cancellable presentation without sending an
/// activation request yet. Poll the returned handle from the UI loop after
/// rechecking application ownership. At most one pending handle is allowed
/// on a UI thread; synchronous [`present`] also returns `Busy` while it lives.
///
/// Native input context is captured before mapping, so later polls cannot
/// borrow permission from an unrelated event. Explicit hints are owned and
/// redacted, and no startup environment is read or modified.
pub fn begin_present<W: WindowExt>(
    window: &mut W,
    options: PresentOptions<'_>,
) -> Result<PresentRequest, PresentError> {
    if !app::is_ui_thread() {
        return Err(PresentError::WrongThread);
    }
    validate(options)?;
    validate_target(window)?;
    let _call = CallGuard::enter()?;
    let lease = AsyncLease::acquire()?;
    let context = crate::platform::presentation::InputContext::capture();
    if !window.shown() {
        window.show();
    }
    validate_target(window)?;
    if !window.shown() {
        return Err(PresentError::Cancelled);
    }
    let target = window
        .as_window()
        .ok_or_else(|| PresentError::Native("FLTK window wrapper is unavailable".into()))?;
    let request = context.begin(target.as_ref(), options)?;
    Ok(PresentRequest {
        target,
        request: Some(request),
        result: None,
        lease: Some(lease),
    })
}

fn validate_target<W: WindowExt + ?Sized>(window: &W) -> Result<(), PresentError> {
    if window.was_deleted() {
        Err(PresentError::DeletedWindow)
    } else if window.parent().is_some() {
        Err(PresentError::ChildWindow)
    } else {
        Ok(())
    }
}

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
    validate_target(window)?;
    if ASYNC_ACTIVE.with(Cell::get) {
        return Err(PresentError::Busy);
    }
    let _call = CallGuard::enter()?;
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
    fn completed_lease_cannot_release_a_new_request() {
        let mut first = Some(AsyncLease::acquire().unwrap());
        assert!(matches!(AsyncLease::acquire(), Err(PresentError::Busy)));
        drop(first.take());
        let second = AsyncLease::acquire().unwrap();
        drop(first);
        assert!(ASYNC_ACTIVE.with(Cell::get));
        assert!(matches!(AsyncLease::acquire(), Err(PresentError::Busy)));
        drop(second);
        assert!(!ASYNC_ACTIVE.with(Cell::get));
        drop(AsyncLease::acquire().unwrap());
    }
    #[test]
    fn failed_reentrant_call_keeps_the_outer_guard() {
        let outer = CallGuard::enter().unwrap();
        assert!(matches!(CallGuard::enter(), Err(PresentError::Busy)));
        assert!(PRESENTING.with(Cell::get));
        drop(outer);
        assert!(!PRESENTING.with(Cell::get));
    }
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
