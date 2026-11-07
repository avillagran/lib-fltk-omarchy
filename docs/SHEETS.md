# Lazy modal sheets

`overlay::Sheet` provides a centered modal card with arbitrary toolkit/FLTK
widgets. It creates its content on the first opening, reuses the same hidden
controls on a quick reopen, and deletes them after 30 seconds closed. This
covers o3's deferred-popup lifecycle without keeping every infrequent dialog
in the initial widget tree.

```rust,no_run
use fltk::prelude::*;
use fltk_omarchy::{layout::FlexExt, overlay::Sheet, widgets::*, State, tr};
# let mut window = fltk::window::DoubleWindow::new(0, 0, 640, 440, "Example");
# window.end();
let draft = State::new(String::new());
let form = draft.clone();
let sheet = Sheet::new(&window, &tr!("Details"), 480, 320, move |body, owner| {
    body.fit(&*TextField::new().bind(&form));
    body.fit(&*Button::new(&tr!("Done")).on_click(move || {
        if let Some(sheet) = owner.upgrade() {
            sheet.close();
        }
    }));
});
sheet.open();
```

Keep the `Sheet` controller in the app. Its clones share one lazy instance
bound to the owning top-level window. The builder runs again after content
release and receives a fresh weak handle for its content callbacks. Capturing
that `WeakSheet` avoids a controller/content reference cycle. `downgrade()`
also supplies a weak handle for callbacks configured elsewhere.

Keep drafts, selection, scroll preferences and other durable presentation
choices in app-owned `State`/models. Widget-local state survives a warm
reopen; only app-owned state survives reconstruction. The builder must be
repeatable and create a new subtree each time. It receives an open column;
end any nested containers you create, as in an `AppWindow` body.

## Lifecycle and policy

- `open()` constructs on demand, raises the own in-window layer, and focuses
  its first content control. Calling it while already open preserves focus;
  calling it during a closing fade reverses the sampled transition.
- `close()` stops input and restores the opener immediately. A reduced-motion
  close hides immediately; otherwise the shared popup fade finishes first.
  The closed-only retention period starts when the layer becomes hidden.
  This is an explicit owner operation and bypasses the user-dismissal binding.
- `release_after(Duration)` changes the 30-second default. Zero releases as
  soon as the close finishes; open content never expires. `keep_content()`
  opts stateful forms out of expiry. These policies can change on a retained
  controller as well as during construction.
- `release()` deletes hidden closed content and returns whether there was
  content to dispose. It returns false while open, building or fading. Native
  deletion is deferred until the event handler returns; resource destruction
  finishes on a subsequent event-loop turn.
- Dropping the last controller disposes its subtree. Deleting its owning
  window deletes all native children even if a controller survives. Such a
  controller reports closed/unbuilt and cannot reopen. Direct window
  destruction does not invoke app dismissal callbacks.
- Hiding the owner dismisses the sheet with `SheetDismiss::OwnerHidden`.
  `on_dismiss` otherwise reports `Closed`, `Escape`, `CloseButton`, or
  `ClickOutside` once per opening. It may reopen the sheet synchronously.
  Owner hiding retires its focus-return target without refocusing/remapping
  that hidden window. Reopening from the `OwnerHidden` callback is rejected;
  show the owner again before reopening. Owner deletion always cleans up.

## Busy forms and acknowledgement ownership

Bind permission before opening a form whose Apply/rollback needs an ACK:

```rust,no_run
use fltk_omarchy::{overlay::Sheet, State};
# fn form(sheet: Sheet) {
let busy = State::new(false);
let sheet = sheet.bind_dismissible(&busy.map(|busy| !busy));
sheet.open();
busy.set(true); // Applying or waiting for ACK: Esc/outside/Close cannot dismiss.
// On a successful Apply or completed rollback, on the UI thread:
busy.set(false);
sheet.close(); // Explicit owner completion; never vetoed by the binding.
# }
```

`bind_dismissible(&State<bool>)` controls Escape, an outside primary click,
and the toolkit header Close action. False disables that action, including
its keyboard/accessibility state. Rejected input is consumed by the modal
without changing logical focus, open state, transition direction/phase,
retention deadlines, or emitting `on_dismiss`. An outside press begun while
blocked cannot acquire a release when permission becomes true; a press begun
while allowed is checked again at release. A newly blocked Close action also
cannot complete an earlier press.

The permission is UI-owned State, not an arbitrary application callback
invoked during input. No application model borrow is taken or held while
checking it. Native activation/deactivation runs after internal content and
subscription borrows have been released. The binding retains temporary
mapped States, keeps only a weak controller reference in its subscriber,
unsubscribes on rebind/final controller drop, and ignores already queued
notifications from an obsolete binding. `is_dismissible()` reads the current
user policy independently of whether content is open or built.

`close()` is deliberately an owner bypass, including calls through
`WeakSheet::upgrade()`. App-provided Cancel/Apply buttons and shortcuts are
owner actions too: disable them, or have Cancel request rollback and keep
the sheet open until the rollback ACK. The toolkit does not invent or run
that transaction. `release()` still cannot remove open/fading content.
Owner hide/deletion and final controller destruction bypass the binding and
retire modal input. A rejected user request never becomes `OwnerHidden`.

The binding adds no polling, timer, content construction or animation. It
survives cold reconstruction and can change while the sheet is closed.

No content widgets or timers are created before first use. Closed sheets
share one native deadline timer with weak entries; reopening, release and
controller destruction cancel obsolete deadlines. There is no periodic timer
or redraw while a retained sheet is idle. Expiry is UI lifecycle time, not a
motion duration: disabling animations does not change the 30-second policy.
Deterministic tests can freeze/advance `anim::clock` and pump the UI loop.

`is_open()` reports logical input ownership; `is_built()` includes open,
fading and hidden cached content. `rect()` and `close_rect()` return current
geometry in owning-window coordinates for testkit/native hit mapping.

## Input, theme and accessibility

Tab/Shift+Tab stay within the sheet through the shared focus router. Escape
closes the active sheet; a popup above it receives Escape first. Keep handles
for independent popups opened by your form and close them when your app
programmatically dismisses the form. The sheet does not own unrelated sibling
popups or notices.

Only primary press/release ownership dismisses an outside click or activates
the close button. A press outside followed by release inside is cancelled.
Unhandled pointer/wheel events remain with the modal instead of reaching the
underlying canvas, including input originally delivered to a native GL child.
The owner window and other windows keep their normal native focus policy;
opening an in-window sheet is not a native window activation request.

The sheet uses theme spacing/surfaces, clamps to a resized window and follows
RTL layout. `bind_title(&State<String>)` updates its visible title and Dialog
name live. The close action follows the toolkit's translated `Close` label.
It exposes a named modal Dialog and ordinary accessible content children;
hidden retained content is excluded from the accessibility tree.

The CPU renderer is the default. With a GPU scene behind it, the existing
widget-overlay bridge composes the sheet: static controls retain their texture
while the background shader changes. Hiding releases that texture even while
the native controls remain cached. Content with its own native child window
uses the bridge's existing fallback/recovery policy. No GPU/effects feature
is required by this API.

Hidden widgets do not draw, but app-owned background jobs remain app-owned:
pause/cancel them on dismissal or use resource guards captured by child
callbacks for release-time cleanup. Keeping a controller or app model alive
does not imply that arbitrary external work has been cancelled.

## Example and validation

Run `cargo run --example sheets`; `Mod+I` opens the form, `Mod+Enter` saves,
and F1 lists shortcuts. Draft state and the build counter make warm reuse
and cold reconstruction visible. The example includes English/Spanish text
and supports `-- --snapshot build/sheet-example.png`.

`tests/sheet_ui` checks exact 30-second boundaries with a frozen clock, no
initial build, warm reuse, reconstruction from State, widget/resource deletion,
weak callbacks, reentrant reopening, owner hide/destruction, keyboard trapping,
stacked popups, auxiliary buttons, primary cancellation, motion reversal,
reduced motion, resize/RTL and live accessible names. GPU scene checks use
real GL readback against the CPU image, native-child routing, intermediate
fade/reversal frames, retained captures during shader frames, zero cached idle
frames, and zero overlay bytes after hiding/release.

Local checkpoint validation (NODE, 2026-11-06): CPU 278 unit checks
(1 ignored), 6 integration checks, 186 UI checks and 49 doctests passed.
GPU: 283 unit checks (1 ignored), 6 integration checks, 201 UI checks and
50 doctests passed. Seven directed sheet/GPU/accessibility checks passed
on both NVIDIA RTX 3070 Ti and Mesa llvmpipe (LLVM 23.1.3). Complete GL/CPU
image parity stayed within 2 RGB levels, including intermediate closing and
reversal. Static shader intervals had 13/11 GPU frames, zero overlay
recaptures/uploads/parent redraws, zero cached idle frames, and zero overlay
texture bytes after hiding/release.

New-module formatting, repository Clippy correctness/suspicious gates,
Rust 1.80 all-target checks, and Windows MSVC/macOS ARM all-target typechecks
with GPU/accessibility passed. The English/Spanish example catalog passed
its source check (11 keys). Testkit drives input through the native-child
routing path; this is not a claim of physical-device or native Windows/M2
runtime validation for this component. No remote CI or publication was
started for the checkpoint.

The existing switch-transition check now uses the frozen clock for pixel
phases and drains its last native timer before asserting idle. Its original
wall-clock sampling could miss the intermediate frame under build load and
leave animations enabled after failure; a scope guard now restores state.
Child `Hide` callbacks may release the sheet: parent detachment and deletion
are deferred so FLTK can finish notifying sibling widgets safely.

### Dismissal permission validation (2026-11-06)

The guard revision passed the complete CPU suite (285 unit checks, one
ignored, six integration checks, 196 UI checks, 52 doctests) and the
all-feature suite (359 unit checks, one ignored, six integration checks,
246 UI checks, 59 doctests). Six directed guard checks cover temporary mapped
State lifetime, obsolete notification/rebind handling, a borrowed app model
during denied input, close enablement, press/release permission changes,
exact retention boundaries, frozen reversal parity, weak owner completion,
owner hide/deletion and subscription cleanup. Seven directed guard/GPU
checks passed with NVIDIA RTX 3070 Ti and Mesa llvmpipe: guarded native-child
events preserve modal focus and image parity, static shader intervals still
have zero overlay recaptures/uploads/parent redraws, and hiding frees GPU
overlay bytes. The existing child-Hide release/sibling-order regression also
passes without refocusing a hidden owner.

```sh
cargo build --offline --example sheet_guard_input_check
script/sheet-guard-input-check
script/sheet-guard-wayland-check
```

The first runner uses XTest inside its own Xvfb: rejected Escape, Close,
outside/auxiliary clicks, focused typing, enabled Escape, weak owner finish,
short native retention, owner hide/deletion and peer-window input all pass.
The second uses a private rootless Sway compositor and virtual keyboard:
Escape denial/typing, enabled keyboard Close, weak owner finish, retention,
hidden/deleted-owner cleanup, peer input and final unsubscribe pass. It
reserves wtype's compact keymap slots 10–19 because pinned FLTK normalizes
those physical codes to digits, and keeps devices alive while observing
focus. No host display, input device or runtime socket is exposed. The probe
paths default to `target/` and accept `SHEET_GUARD_BIN`.

New-module/platform formatting, repository Clippy correctness/suspicious
gates, Rust 1.80 all-target checks with GPU and all-feature Windows MSVC/macOS
ARM typechecks passed. Native Windows/macOS and Files' Sheet adoption on
NODE/M2 are not established by these toolkit checks. No remote publication
or CI was started.
