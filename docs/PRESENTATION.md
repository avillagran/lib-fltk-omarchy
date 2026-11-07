# Presenting an existing window

`native::begin_present` maps a hidden application-owned FLTK top-level window
and returns a manually polled `PresentRequest`. It does not send activation
before the first poll. For an `AppWindow` wrapper, pass `&mut *win` to borrow
its underlying FLTK window. Begin and poll on the UI thread, for example
after a FileManager1 request has been handed to the UI queue:

```rust,no_run
use fltk_omarchy::native::{self, PresentOptions};
# fn example(mut window: fltk::window::DoubleWindow, startup_id: Option<&str>) -> Result<native::PresentRequest, native::PresentError> {
let request = native::begin_present(&mut window, PresentOptions {
    startup_id,
    ..Default::default()
})?;
# Ok(request) }
```

Store the handle in UI-owned state. Once during each ordinary UI iteration,
recheck that the request still belongs to the current navigation, modal and
window context, then poll it outside application model borrows:

```rust,no_run
use std::task::Poll;
use fltk_omarchy::native::{PresentRequest, PresentStatus};
# fn advance(request: &mut PresentRequest, context_is_current: bool) {
if !context_is_current {
    request.cancel();
}
match request.poll() {
    Poll::Pending => {
        // Keep the handle and return to the ordinary UI loop.
    }
    Poll::Ready(Ok(outcome)) => {
        if outcome.status == PresentStatus::Unsupported {
            // The window is visible, but native activation is unavailable.
        }
        // Consume the outcome and stop scheduling polls for this handle.
    }
    Poll::Ready(Err(error)) => {
        // Consume the error, including intentional cancellation.
        let _ = error;
    }
}
# }
```

There is no internal timer, worker or future executor. While a request is
pending, the application's loop must continue to make ordinary UI turns;
do not spin on `Pending` or sleep inside a callback. Stop polling after a
terminal result. The handle is neither `Send` nor `Sync`, tracks deletion
without extending the widget's lifetime, and must remain on the UI thread.
Before each pending poll, the facade rejects a deleted or child window and
cancels a window that has become hidden.

Pass the startup context from the incoming request explicitly. On Wayland,
`activation_token` takes precedence over `startup_id`; either is treated as
an opaque xdg-activation token. On X11, `startup_id` is used for startup
notification. `timestamp` is an optional X server timestamp, not Unix time;
otherwise the `_TIME` suffix in `startup_id` is used, then CurrentTime.
Empty strings mean absent. Hints containing NUL or exceeding 60 KiB are
rejected before mapping the window. The request owns the retained hints,
so caller strings can be dropped after `begin_present` returns. Debug
output redacts both hints and the request exposes no token in its errors.
Windows and macOS ignore these hints.

The API never reads or mutates `XDG_ACTIVATION_TOKEN` or `DESKTOP_STARTUP_ID`.
If an application imports its initial launch environment, it owns that
startup-only policy. Worker threads should forward owned strings to the UI
thread with `task::post`; they must not call either presentation API directly.

## Completion and cancellation

At most one asynchronous presentation lease can be pending on a UI thread.
Another `begin_present`, or the synchronous `present` API, returns `Busy`
while it is held. Reentrant presentation calls are also rejected. Completion,
`cancel` or dropping a pending handle releases the lease. Keeping or later
dropping a completed handle cannot release a newer request's lease.

The first terminal `Result<Presentation, PresentError>` is cached. Repeated
polls return that result without resending activation or touching the window.
`cancel()` returns `true` for pending work and makes subsequent polls return
`PresentError::Cancelled`; after completion or a previous cancellation it
returns `false` and preserves the terminal result. Dropping pending work
retires its private resources. Neither operation automatically retries.

Cancellation before native activation dispatch prevents that activation.
Mapping already performed by `begin_present` follows normal FLTK/compositor
policy and cannot be undone by cancellation. A terminal `Requested` result
means native dispatch already happened; cancel or drop cannot recall it or
claim that focus was restored. Application ownership checks remain the
caller's responsibility before every pending poll.

## Outcomes

| Status | Meaning |
| --- | --- |
| `Unsupported` | The native activation mechanism is unavailable. A hidden window may still have been mapped. |
| `Requested` | The request was dispatched; acceptance is not known. |
| `Granted` | Native foreground/key-window state or a synchronous native result confirms activation at this call. |
| `Refused` | The native API explicitly refused activation. |

`Presentation.backend` identifies the actual native backend. `Requested`
is **not** a focus guarantee and must not be displayed as successful focus.
The status describes activation rather than visibility. Errors identify
wrong-thread calls, deleted/child windows, invalid hints, reentrancy, native
failures, cancellation and handshake timeout. An error after mapping need
not undo mapping.

## Platform behavior

- **Wayland:** xdg-activation-v1 on FLTK's own connection and surface. A
  supplied token is forwarded unchanged. Without one, a token is requested;
  a native event's seat/serial and the focused origin surface are included
  when available. Synthetic toolkit input cannot establish user permission.
  The compositor decides whether the token is valid. The protocol has no
  granted/refused acknowledgement, so dispatch returns `Requested`, even
  for stale/invalid tokens. An absent protocol returns `Unsupported`.
- **X11:** checks `_NET_SUPPORTED`, observes `_NET_ACTIVE_WINDOW`, and sends
  an EWMH application-source activation request. An already active target
  returns `Granted`; dispatch returns `Requested`. A supplied startup ID is
  set on the own window and completed with startup-notification messages.
  There is no `XSetInputFocus` or unconditional raise fallback.
- **Windows:** restores an iconic own window and uses `SetForegroundWindow`.
  The foreground window or a successful synchronous result yields
  `Granted`; a zero result yields `Refused`. Windows foreground restrictions
  remain in force.
- **macOS:** activates the current application with options zero and orders
  its own NSWindow. Explicit refusal yields `Refused`; current AppKit active
  and key-window state yields `Granted`, otherwise `Requested`.
- **Other platforms:** returns `Unsupported` after normal mapping.

Shown windows are not hidden/remapped. The initial map of a hidden window
follows normal FLTK/compositor policy; the activation outcome does not change
that policy. Minimized-window restoration on Wayland remains subject to the
compositor. A newly mapped surface may require an event-loop turn before an
activation is honored. `AppWindow::raise` is a separate legacy convenience
with different fallback behavior; this API does not call it.

With `begin_present`, Wayland discovery and token acquisition advance without
waiting for the compositor. Each poll makes one protocol pass with a
zero-timeout socket poll and pairs every prepared read with a read or
cancellation before returning. Only the private protocol queue is dispatched,
so these handshakes do not run application callbacks. Each handshake phase
has a two-second elapsed deadline checked when polled; discovery and token
acquisition have separate deadlines. A first no-token request can require
both phases; a cached connection skips completed discovery, and an explicit
token skips token acquisition. These deadlines do not create an internal
wakeup or bound normal window mapping.

Wayland input permission is captured at API entry, before `window.show()`
can dispatch another event. Later polls never substitute a newer gesture's
seat/serial. Before using captured context, the backend checks the same
display/socket identity and current live seat. The origin window has a
deletion tracker; its current surface is resolved only if it is still a
shown top-level window. The target surface is resolved again immediately
before activation. A replaced display terminates the request instead of
accessing old proxies. Private callback storage remains alive until its
proxy is retired, including cancellation and drop.

The cached registry/queue is reused per live FLTK connection and exclusively
leased while a request is pending. The API adds no background threads,
internal timers, external commands, compositor IPC, foreign-window IDs, or
retries that bypass platform policy.

On X11, Windows, macOS and other backends, `begin_present` defers the existing
native presentation call until the first poll. In particular, X11's existing
server roundtrips can still wait in that poll. The portable handle provides
cancellation and ownership checks before dispatch; it does not promise
asynchronous execution of every platform's native API.

## Synchronous compatibility

`native::present(&mut window, options)` remains available and returns
`Result<Presentation, PresentError>` directly. Its Wayland discovery/token
acquisition still waits on the private queue: at most two two-second
handshakes on a first no-token call, one after discovery is cached. It does
not use the new manually polled flow. Prefer `begin_present` when the UI
must remain responsive during Wayland handshakes, and call either API outside
application model borrows. Neither API bypasses compositor focus policy.

## Local validation

NODE CPU/default validation passed 284 unit tests (one ignored), six
integration checks, 189 UI checks and 51 doctests. Four final directed async
UI checks cover rejection before mapping, cancellation/lease ownership,
terminal caching and hidden/deleted targets. All-target Windows MSVC,
macOS ARM and Rust 1.80 checks passed; these are typechecks rather than native
runtime or nonblocking guarantees for those platforms.

`script/async-present-wayland-check` verifies that a real keyboard gesture
captured in `begin_present` can activate the target when polled later, while
a synthetic key does not borrow an earlier native serial. It uses private
Sway policies. `script/async-present-x11-check` verifies wire-level absence
of activation/startup messages before polling or after cancel/drop, one
activation on the first poll and no replay on terminal polls. It also checks
that dropping an old completed handle cannot release a newer lease.

`script/async-present-tls-exit-check` retains a pending request in thread-local
state created before the connection cache. It reproduced an exit abort when
cleanup accessed an already destroyed cache, and passed after cleanup used
fallible TLS access. Core dumps were disabled. The X11 wire and Wayland
gesture drivers passed again after this fix, as did the repository Clippy
correctness/suspicious gates (unrelated existing warnings remain).

The runners default to probes built in this checkout's `target/` directory.
They accept `PRESENT_ASYNC_BIN=/absolute/path/to/probe` for an alternate
build location, and reject a missing executable before starting a display.
Use the appropriate probe for each runner:

```sh
cargo build --offline --example async_present_check --example async_present_x11_check
script/async-present-x11-check
script/async-present-wayland-check
script/async-present-tls-exit-check
```

The [integration receipt](evidence/async-presentation-integration-20261106.json)
records the combined timer/renderer/presentation checkout after cherry-picking
the delivered API unchanged. It includes the post-TLS-fix full-feature suite,
CPU presentation probes and canonical runner paths. The earlier receipt below
describes the original isolated delivery; its manifest predates the timer
test/example registrations and runner-path adjustments in this checkout.

The [local receipt](evidence/async-presentation-20261106/summary.json) records
the exact scope, source/fixture hashes and retained failures. Delayed-peer UI
progress, pending cancellation during both handshake phases and all origin/
reconnect edge cases still need broader integration evidence. Do not
interpret the synchronous checkpoint below as proof of those async paths.

### Synchronous checkpoint 767235b

The following records the original synchronous implementation and its
historical validation. Build `cargo build --offline --example present_check`,
then run:

```sh
script/present-check
script/present-wayland-check
```

The X11 driver starts its own Xvfb, verifies the EWMH wire payload, already
active/unsupported cases, UTF-8 startup property, chunked/escaped startup
completion, timestamp override, invalid hints, unchanged environment and
absence of focus fallback. It emulates WM root properties; it does not
claim acceptance by a real window manager.

The Wayland driver uses a private rootless headless Sway container with no
host display/runtime/input mounts. It checks that invalid explicit tokens
(in both option fields) return `Requested` while focus stays on A, and a
real virtual-keyboard event respects urgent-only policy and activates own B
under smart policy. Fresh launcher tokens also activate B when passed
explicitly through either option field.
It requires the local `o3-qml-wayland-tests:local` image, host `wtype`, and
three read-only X11 libraries needed by the hybrid FLTK binary. Fixture
artifacts stay in a new `build/present-*` directory. The probe itself
intentionally requests focus: do not launch it on the user's desktop.

The native-versus-synthetic serial regression was also checked red/green:
removing the origin guard incorrectly activated B from `focus::press`; the
guarded implementation keeps A focused while the native keyboard is alive.

These are Linux native tests. Windows/macOS typechecks are not native runtime
validation; this checkpoint has no physical M2/Windows evidence. Compositor
policies, expired/reused tokens, minimized/newly mapped windows, protocol
removal/reconnect and timeout paths are not all covered by the Sway driver.

Protocol references: [xdg-activation-v1](https://wayland.app/protocols/xdg-activation-v1),
[EWMH](https://specifications.freedesktop.org/wm/latest-single/),
[startup notification](https://specifications.freedesktop.org/startup-notification-spec/0.1/),
[SetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow).
The minimal xdg-activation descriptors retain the upstream license in
`src/platform/presentation/linux/xdg-activation-LICENSE`.

Checkpoint validation on NODE (2026-11-06, CPU/default features): 278 unit
checks passed (1 ignored), 6 integration checks, 181 UI checks and 49
doctests. New-module/platform formatting, repository Clippy correctness and
suspicious gates, Rust 1.80 all-target checks, and Windows MSVC/macOS ARM
all-target typechecks passed. Native drivers printed the PASS lines described
above; stale-serial regression failed without the guard and passed with it.
The doctests were rerun alone after a concurrent build temporarily made the
library artifact unavailable; all 49 then passed. No CI was dispatched and
no remote publication was performed for this checkpoint.
