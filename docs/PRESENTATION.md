# Presenting an existing window

`native::present` maps a hidden application-owned FLTK top-level window and
requests activation through the native platform. For an `AppWindow` wrapper,
pass `&mut *win` to borrow its underlying FLTK window. Call on the UI thread,
for example after a FileManager1 request has been handed to the UI queue:

```rust,no_run
use fltk_omarchy::native::{self, PresentOptions, PresentStatus};
# fn example(mut window: fltk::window::DoubleWindow, startup_id: Option<&str>) -> Result<(), native::PresentError> {
let outcome = native::present(&mut window, PresentOptions {
    startup_id,
    ..Default::default()
})?;
if outcome.status == PresentStatus::Unsupported {
    // The window is visible, but native activation is unavailable.
}
# Ok(()) }
```

Pass the startup context from the incoming request explicitly. On Wayland,
`activation_token` takes precedence over `startup_id`; either is treated as
an opaque xdg-activation token. On X11, `startup_id` is used for startup
notification. `timestamp` is an optional X server timestamp, not Unix time;
otherwise the `_TIME` suffix in `startup_id` is used, then CurrentTime.
Empty strings mean absent. Hints containing NUL or exceeding 60 KiB are
rejected before mapping the window. Debug output redacts both hints.
Windows and macOS ignore these hints.

The API never reads or mutates `XDG_ACTIVATION_TOKEN` or `DESKTOP_STARTUP_ID`.
If an application imports its initial launch environment, it owns that
startup-only policy. Worker threads should forward owned strings to the UI
thread with `task::post`; they must not call `present` directly.

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
failures and handshake timeout. An error after mapping need not undo mapping.

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

Wayland discovery/token acquisition is synchronous but bounded (at most two
2-second private-queue handshakes on the first no-token call, one thereafter).
It dispatches only its private protocol queue, avoiding application callback
reentrancy. The cached registry/queue is reused per FLTK connection. There
are no threads, timers, polling while idle, external commands, compositor
IPC, foreign-window IDs, or retries that bypass platform policy.

## Local validation

Build `cargo build --offline --example present_check`, then run:

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
