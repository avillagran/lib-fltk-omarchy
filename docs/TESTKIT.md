# testkit: look at a UI as data, drive it, check its motion

`fltk_omarchy::testkit` is for tests and coding agents. It is always
compiled (no feature) and costs nothing until used: one thread-local check
per frame and per callback, and component descriptions are only kept once
the kit is armed.

| Piece | API | Use |
|---|---|---|
| Element tree | `testkit::tree(&window)`, `tree_json`, `find(&window, name)` | what is on screen: path, kind, name, value, rect, visible, focused |
| Script runner | `testkit::run_script(&window, json)` → `Report` | input by accessible name, waits on the frozen clock, expectations, PNGs |
| Env runner | `OMARCHY_FLTK_SCRIPT`, `OMARCHY_FLTK_TREE`, `OMARCHY_FLTK_REPORT` | drive any toolkit app headlessly from outside |
| Motion gate | `testkit::Gate`, `Rules`, `Finding` | jumps, pops and overshoot frame by frame |
| Frame log | `OMARCHY_FLTK_FRAME_LOG=path`, `testkit::frames::log_to` | paint, wall and CPU time per window frame |
| Blocked-UI warning | debug builds; `OMARCHY_FLTK_BLOCK_WARN` | a UI callback that waits on I/O |
| Timer ownership | `testkit::timer_stats()` | pending/queued/executing callbacks and completion/cancellation counters; see [TIMERS.md](TIMERS.md) |

Input never leaves the app: events go straight to its own windows through
the same path as real input (`focus::press`, `click_at`, `wheel_at`…), and
windows need not be shown. Never drive or show windows on the user's
desktop: run under `xvfb-run` or on a hidden workspace (`script/real-window`).

## Element tree

```rust
use fltk_omarchy::testkit;
testkit::arm(); // before building windows, in builds without `a11y`
// ... build the window ...
for e in testkit::tree(&*win) {
    println!("{} {} {:?} {:?} {:?}", e.path, e.kind, e.name, e.value, e.rect);
}
let save = testkit::find(&*win, "Save").unwrap();
assert!(save.visible && save.kind == "Button");
std::fs::write("tree.json", testkit::tree_json(&*win)).unwrap();
```

Each `Element` has:

- `path`: child indices from the window (`"0/1/3"`); virtual rows add
  `#key` (`"0/1/4#17"`).
- `kind`: the accessible role (`Button`, `TextInput`, `Label`,
  `ListBoxOption`, `Document`…) or the fltk class of plain containers
  (`Window`, `Flex`, `Group`, `Scroll`).
- `name`: `accessible_name`, else the component's visible text, else the
  tooltip (header actions, `ActionButton`); `value`: field text, slider
  value, `on`/`off`. Both cut at 300 characters.
- `rect` in window coordinates, `visible` (the widget and its groups are
  visible and the rectangle meets what its parents show), `focused`,
  `selected` (rows, tabs, checked switches), `disabled`, `depth`, `key`.

Virtual rows come from the components' own descriptions, only near the
viewport: `List`, `Table`, `Tree`, `NavTree`, `DiffView`, keyboard panels
with `a11y`, and transcripts. A `RichText` lists its blocks with the
`a11y` feature and its messages (name = message key) without it.

Names come from the accessibility descriptions. With the `a11y` feature
they are always there. Without it, `a11y::describe`, `register` and
`set_name` hand them to the kit, which keeps them only while armed
(`testkit::arm()`, or any env variable below); widgets created before
arming fall back to labels and tooltips.

## Script runner

```rust
let report = testkit::run_script(&*win, r#"[
    {"click": {"kind": "TextInput"}},
    {"type": "Ada"},
    {"click": "Add"},
    {"press": "Mod+K"},
    {"wait_ms": 200},
    {"expect": "Count 1", "visible": true},
    {"expect": {"kind": "TextInput"}, "value": "Ada", "focused": true},
    {"snapshot": "build/after.png"}
]"#);
assert!(report.ok(), "{}", report.summary());
```

| Step | Does |
|---|---|
| `{"press": "Mod+K"}` | a key press through the keyboard router (`"repeat": n`) |
| `{"type": "héllo"}` | committed text, as an input method sends it |
| `{"click": target}` | left click at the center of the target's visible part (`"right": true`) |
| `{"hover": target}` | pointer move |
| `{"wheel": target, "dy": 3}` | wheel notches, positive down (no target: window center) |
| `{"drag": target, "to": target}` | press, drag in steps, release |
| `{"focus": target}` | keyboard focus to that widget (a row: its widget, then a click) |
| `{"wait_ms": 200}` | the frozen animation clock moves on in 16 ms frames; each frame is rendered offscreen so paint-driven motion advances |
| `{"sleep_ms": 50}` | real time passes while events, timers and posted work run (background tasks, debounces) |
| `{"expect": target, ...}` | `visible`, `text` (name or value contains), `value` (exact), `focused`, `selected`, `disabled`, `count` (visible matches); `visible: false` also passes when nothing matches |
| `{"snapshot": "a.png"}` | offscreen PNG of the window |
| `{"tree": "t.json"}` | `tree_json` to a file |

A target is a name (`"Save"`, exact) or an object with `name`, `contains`
(part of the name), `kind`, `path`, `index` (n-th match, visible first) or
`xy: [x, y]`. Paths are relative to the current directory. The clock is
frozen during the run (and stays frozen if it was); steps go on after a
failure. `Report::to_json` gives
`{"ok", "passed", "failed", "steps": [{"index", "op", "ok", "error"?}]}`.

## Driving any app from the environment

```sh
env -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 1280x800x24" \
  env OMARCHY_FLTK_SCRIPT=steps.json OMARCHY_FLTK_TREE=tree.json \
      OMARCHY_FLTK_REPORT=report.json \
  cargo run --example testkit_demo
```

With `OMARCHY_FLTK_SCRIPT` or `OMARCHY_FLTK_TREE` set:

1. `App::new` arms the kit; `AppWindow::show` does not map windows
   (headless; `OMARCHY_FLTK_SCRIPT_SHOW=1` maps them, under Xvfb or on a
   hidden workspace only).
2. `App::run` does not enter the event loop. It runs the script against the
   last `AppWindow` made (or `{"window": "Title", "steps": [..]}`), writes
   the tree after the script to `OMARCHY_FLTK_TREE` and the report to
   `OMARCHY_FLTK_REPORT` (else a summary on stderr), shuts the app down
   and exits: 0 when every step passed, 1 when one failed, 2 when the
   script could not run (unreadable, no window).
3. `OMARCHY_FLTK_TREE` alone dumps the tree of the started app.

Apps that do not call `App::run` (plain fltk loops) are not driven.

## Motion gate

```rust
use fltk_omarchy::{anim, testkit::{Gate, Rules}};
anim::set_enabled(true);
let mut gate = Gate::new(&*win);
gate.track_messages(&transcript);          // msg:<key>, RichText::message_rect
gate.track_items(&nav, &["r1", "r2"]);     // item:<key>, NavTree::item_rect
gate.track_widget("button", &*button);     // any widget while shown
gate.track_element("Row 3");               // el:<name>, from the tree
gate.track("custom", move || Some((x, y, w, h)));
gate.play(300);                            // settle
gate.act("prepend", || transcript.prepend_messages(older));
gate.play(400);
gate.verify(&Rules::default())?;           // Err lists the findings
```

Each frame advances the frozen clock by 16 ms (`frame_ms` changes it),
runs pending events and posted work, renders the window offscreen and
samples every probe. `check` finds:

- **Jump**: a move longer than `max_jump` (default 8 px) in one frame
  while the frames around it hold still (an eased move is not one: its
  neighbours move too). Frames where it is out of view give no verdict.
  An input frame (`act`) is excused only with `inputs_excuse`.
- **Pop**: shown → gone → shown, or gone → shown → gone, within
  `pop_frames` (default 4) while nothing tracked moves (content scrolled
  through a view does not count); an opacity drop of `opacity_drop` that
  comes back (`track_sample` gives opacity).
- **Overshoot**: within a run of moving frames, going past where it ends
  (or behind where it started) by more than `overshoot` (default 2 px).

`dump(track)` prints a track frame by frame for failure messages.

What the UI checks hold the toolkit to (tests/testkit_ui):

- layout motion (`layout::set_visible`): no findings; the same change with
  animations off is caught as a jump;
- `RichText` prepend while scrolled: messages in view do not move at all;
  `follow_end` from the middle (a cut, then a glide): no findings;
  streaming while following: each streamed line scrolls the view by one
  line at once (no spring growth on the CPU backend, docs/O3-PORT.md §8),
  so that check allows one line and catches anything bigger;
- `NavTree` row arrival: rows below glide down without jumps or pops.

## Frame log

`OMARCHY_FLTK_FRAME_LOG=frames.jsonl` (read at `App::new`) or
`testkit::frames::log_to(Some(path))` writes one JSON line per frame an
`AppWindow` paints:

```json
{"t":1.204,"window":"Chat","frame":42,"paint_ms":1.8,"cpu_ms":1.7,"wall_ms":16.7,"ui_cpu_ms":2.1,"damage":128,"size":[900,700]}
```

- `paint_ms`, `cpu_ms`: wall and UI-thread CPU time of the paint.
- `wall_ms`, `ui_cpu_ms`: since this window's previous frame started: the
  frame interval and all UI-thread work in it (events, callbacks, layout,
  paint). `null` on the first frame.
- `damage`: FLTK damage bits (128 full, 4 scroll, 1 child).

Thread CPU time comes from `platform::thread_cpu_time()`
(`CLOCK_THREAD_CPUTIME_ID` on Linux, macOS and BSDs, `GetThreadTimes` on
Windows); it is `null` where the OS has none. Offscreen renders
(`snapshot`, the gate) are not frames of a shown window and are not logged.

## Blocked UI thread

Debug builds time the callbacks of `Button::on_click`, `ActionButton`,
`AppWindow::shortcut` and `task::post`. When one takes more than 50 ms
longer in wall time than in CPU time (it waited on a file, the network, a
lock or a sleep), stderr gets:

```
fltk-omarchy: the UI thread waited 80 ms (0 ms CPU) in a callback set at src/main.rs:42:17: move blocking work into a Task
```

once per call site and at most once a second. `OMARCHY_FLTK_BLOCK_WARN=0`
silences it, `=1` enables it in release builds;
`testkit::frames::set_block_warnings` and `blocked()` serve tests.

## Gaps

- Keyboard panels (`panel::KeyboardPanel`) expose their items only with the
  `a11y` feature; without it the canvas is one element.
- Without `a11y`, a `RichText` exposes messages, not blocks, and other
  components only what their describe callback gives.
- The gate follows top-left positions and presence; size changes and
  opacity are only checked when a probe reports them (`track_sample`).
- The env runner drives one window per run and needs `App::run`.
- The frame log covers `AppWindow`s, not plain fltk windows.
