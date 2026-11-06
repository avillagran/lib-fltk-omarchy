# Persistent notices with actions

`overlay::Notice` is a non-modal card at the top-right of its containing
window (top-left in RTL). It never takes focus when shown and has no
expiry timer, regardless of urgency. The application owns action outcomes,
dismissal and the number of queued cards. Use `Notification::show` for
short-lived, dismiss-only toasts instead.

```rust
use fltk_omarchy::{i18n, overlay::{Notice, NoticeAction}, widgets::glyph,
                   App, AppWindow};
let app = App::new("dev.example.notice");
let mut win = AppWindow::new("", 640, 400);
win.bind_title(&i18n::text("Example"));
win.end();
let notice = Notice::new("", "")
    .icon(glyph::BELL)
    .bind_summary(&i18n::text("Operation interrupted"))
    .bind_body(&i18n::text("Choose an action to continue."))
    .action(NoticeAction::new("retry", "").bind_text(&i18n::text("Retry")))
    .action(NoticeAction::new("close", "").bind_text(&i18n::text("Close")))
    .on_action(|notice, id| {
        match id {
            "retry" => { /* hand the request to the app's task controller */ }
            "close" => notice.close(),
            _ => {}
        }
    })
    .show(&*win);
win.show();
app.run();
```

IDs are nonempty and unique within a card; labels are presentation and may
change language without changing identity. A disabled action is shown dim
and skipped by keyboard/AT activation. Use complete translated strings
and live `State<String>` bindings for summary, body and action labels.
Include custom messages in the app catalog; common labels such as Close
are supplied by the toolkit.
`Urgency::Critical` makes the accessibility announcement assertive; other
urgencies are polite. It does not change persistence.

## Actions and lifetime

`on_action` receives the stable ID and a handle to the current notice. It
runs on the UI thread. An action never closes the notice automatically:
start/update a task, open details or call `notice.close()` explicitly.
Callback reentry is ignored, and no borrow is held while application code
runs. A callback can close the card or open another one.

Dropping a handle leaves the card open. Explicit close is idempotent,
restores focus when the card had it, and releases captured action resources
when the current callback returns (immediately when none is running).
Deleting the containing window destroys its cards without
running application callbacks. Surviving handles become closed and do not
keep action/dismiss callback captures alive. The handle retains its text
states and action metadata until it is dropped.

`on_dismiss` runs once for explicit dismissal, with `NoticeDismiss::Closed`
(the handle), `CloseButton` (primary/AT close hit), or `Keyboard` (Esc,
Delete, Backspace, or activation of the close cursor). It does not run
after the native widget/window has been destroyed.

## Input and geometry

| Input/API | Behavior |
|---|---|
| Primary action press/release on the same hit | Run its enabled action once |
| Primary close press/release | Explicitly dismiss |
| Body click, middle, right, other buttons | No primary action or dismissal; auxiliary buttons preserve pane focus |
| Tab/Shift+Tab | Enter/leave the card as one control |
| Left/Right, Home/End | Move its cursor among enabled actions and Close; horizontal direction mirrors in RTL |
| Enter/Space | Activate the cursor |
| Esc/Delete/Backspace | Dismiss the focused card |
| `focus_action(id)` | Focus that enabled action without running it |
| `activate(id)` | Run the same action path; false for disabled/unknown/closed/deleted/missing callback or callback reentry |
| `rect()`, `action_rect(id)`, `close_rect()` | Actual `(x,y,width,height)` in window logical coordinates; None after close/deletion |

Screen readers receive an Alert with the full summary/body and virtual
Button children, including a translated, named Close action. AT Focus
moves the cursor without activation; AT Click uses the same action path.

Persistent and legacy cards share a stack **per window**. Dismissal closes
its gap; deleting one window does not move another window's cards. Width,
action wrapping, theme/font changes and RTL follow the window live. Text
measurement is cached by width/theme and invalidated by bound text edits;
unchanged stacks cause no redraw. No timer polls the stack or expires a
persistent card. Existing overlay motion follows the system setting and
stops when settled.

Body text occupies at most two wrapped lines; the accessible description
keeps the whole body. Cards are window overlays, not a scrolling
notification history. Keep the active queue within the available window
height and use an action for longer details/history.

## Example and validation

`cargo run --example notices` shows a persistent retry/close card and a
live counter. `-- --snapshot build/notice.png` renders it offscreen. Its
small example catalog includes English and Spanish.

`tests/notices_ui` covers action IDs, button ownership, focus restoration,
per-window stacks, legacy coexistence, live text, wrapped actions, resize,
RTL, callback reentry, resource lifetime and accessibility. These input
checks use the testkit event path; they do not certify native Wayland
input or a physical keyboard/pointer.

With feature `gpu`, the scene notice check compares actual framebuffer
read-back against CPU rendering for initial, updated, hovered and cleared
states. It covers native-child event routing in a moved window, unchanged
pointer hits, retained textures during shader frames, zero idle frames and
texture release on close. The event helpers include all native ancestor
offsets in screen coordinates; group positions are not counted twice.
This is rendering/routing evidence, separate from native OS input tests.
