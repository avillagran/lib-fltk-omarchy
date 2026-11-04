# Accessibility (feature `a11y`)

FLTK has no accessibility bridge. The `a11y` module adds one at the toolkit
level with [AccessKit](https://accesskit.dev): screen readers and other
assistive technologies (AT) read the app and drive it.

| OS | Platform API | Adapter | Screen readers |
|---|---|---|---|
| Linux / BSD (Wayland, X11) | AT-SPI over D-Bus | `accesskit_unix` 0.23.0 | Orca |
| Windows | UI Automation | `accesskit_windows` 0.35.0 | NVDA, Narrator, JAWS |
| macOS | NSAccessibility | `accesskit_macos` 0.27.0 | VoiceOver |

```toml
lib-fltk-omarchy = { git = "https://github.com/avillagran/lib-fltk-omarchy", features = ["a11y"] }
```

The feature is **off by default** and needs **Rust 1.85** (the rest of the
crate keeps 1.80). Why: `accesskit_unix` 0.19, the last release that
builds with Rust 1.80, starts when AT-SPI's `ScreenReaderEnabled` turns on.
at-spi2-core 2.62 (current Arch/Omarchy) no longer has that property, so apps
never became accessible there. `accesskit_unix` 0.22+ follows `IsEnabled`
and needs 1.85.

## What it costs

| | `a11y` off | `a11y` on, no AT | `a11y` on, AT active |
|---|---|---|---|
| `bench_minimal` release binary | 1.28 MB | 3.30 MB (+2.03 MB) | same |
| PSS (median of 7) | 9.29 MB | 11.06 MB (+1.77 MB) | 12.04 MB (median of 5) |
| Startup to mapped (median) | 59 ms | 55 ms (no difference) | n/a (see below) |
| Idle CPU over 5 s | 0 | 0 | 0 |
| Threads / sockets | 2 / 2 | 5 / 3 | — |

Measured 2026-11-04 with `examples/measure.rs` (the method in
[PERFORMANCE.md](PERFORMANCE.md)) on an AMD Ryzen 9 5900X, Hyprland
0.56.2, release profile. Raw numbers are in
[evidence/a11y-20261104/cost.jsonl](evidence/a11y-20261104/cost.jsonl). The
"AT active" runs start inside a private D-Bus session with its own AT-SPI
bus. That setup takes ~0.4 s, so their startup time measures the bus, not
the app.

- **Nothing is built while no AT is active.** Each shown window gets an
  adapter right away. On Linux that is one background thread (AccessKit's
  executor plus the async-io reactor and a blocking pool: 3 threads) and a
  session-bus connection that only watches AT-SPI's `IsEnabled`. Widgets
  register a small describe callback when they are created; it never runs
  until an AT asks.
- **When an AT activates**, the first tree is built at the next turn of
  the event loop: 20 nodes take 55 µs, about 3 µs per node. Later
  updates rebuild the descriptions after a window repaint (or after a
  focus change, an AT action or `a11y::changed`) and send only the nodes
  that differ, measured at 46–130 µs for that window. They run in the
  toolkit's existing pre-flush check, never on a timer, so an idle app stays
  at 0 % CPU.
- `OMARCHY_FLTK_A11Y=off` keeps the platform adapters off at run time.
  `OMARCHY_FLTK_A11Y_DEBUG=1` prints the size and build time of every update.

**Recommendation.** Keep it off by default in the library, like `gpu`, for
three reasons: the 2 MB of D-Bus/async code and the three idle threads
are paid by every app that links it, apps like Files do not need it yet, and
the feature needs Rust 1.85 while the crate's MSRV is 1.80. Turn it on in
end-user apps that must be usable with a screen reader (o3-code). Revisit
default-on when the crate's MSRV reaches 1.85: then the only cost is
size and idle memory, and accessibility should be opt-out.

## How it works

1. `App::new` calls `a11y::install()`. Plain fltk apps call it after
   `fltk::app::App::default()`.
2. Each shown top-level window gets an adapter. Menus, tooltips and
   subwindows do not.
3. When an AT asks, the adapter bumps an activation generation and wakes
   the loop. The next pre-flush check builds the full tree (role, name,
   value, states, actions, bounds, focus) from the widget tree:
   - Omarchy components describe themselves.
   - Plain fltk widgets get a node by type.
   - Unnamed layout groups (Flex, Group) are left out; their children
     move up to the parent.
4. Later checks rebuild after any window damage, focus move or AT action
   and send only the changed nodes. The generation is read under the
   adapter's lock, so a re-activation always gets a full tree.
5. AT actions are queued from the adapter's thread and run on the UI
   thread. They go first to the widget's `on_action` handler, then to its
   default click (the same closure as Enter/Space and a mouse click), then
   to the generic fltk behavior (callback, value, focus).

Overlays are widgets on top of the window (menus, dropdown popups,
dialogs, toasts), so they appear in the tree while open and leave it when
they close. Modal ones take the focus, which the AT follows.

## What each component exposes

Names come from the visible text. Icon-only controls use their tooltip, or
`.accessible_name(..)`.

| Component | Role (AccessKit → AT-SPI) | Name, value, states | Actions |
|---|---|---|---|
| `AppWindow` | Window → frame | window title | — |
| Header title | Heading (level 1) | title | — |
| Header actions | Toolbar → tool bar, with Button children | tooltip of each action | Click, Focus |
| `Button` | Button → button (toggle button once `set_selected` was used) | text; checked when selected; keyboard shortcut | Click, Focus |
| `ActionButton` | Button | tooltip (or `accessible_name`) | Click, Focus |
| `ButtonGroup` | RadioGroup with RadioButton children | option text, checked | Click, Focus |
| `ToggleSwitch` | Switch → toggle button (pressed = on) | `accessible_name`; checked | Click, Focus |
| `Toggle` | Switch | title; description; checked | Click, Focus |
| `Slider` | Slider | `accessible_name`; numeric value, min, max, step; value text | Increment, Decrement, SetValue, Focus |
| `Progress` | ProgressIndicator | value 0..1, "42%" | — |
| `NumberField` | SpinButton → spin button | `accessible_name`; value, min, max, step | Increment, Decrement, SetValue, Focus |
| `TextField` / `password()` | TextInput / PasswordInput → entry / password text | placeholder as name; value (masked for passwords); caret and selection as text runs | SetValue, SetTextSelection, ReplaceSelectedText, Focus |
| `Label` | Label → label | its text | — |
| `Label::section` | Heading (level 2) | its text (not uppercased) | — |
| `Hero` | Heading (level 1) | title; description "detail · meta" | — |
| `Row` | Button | label; description = detail; current | Click, Focus |
| `TabBar` | TabList → page tab list, Tab children | label, selected, "2 of 5", bounds | Click (select), Focus, ScrollIntoView |
| `Dropdown` | ComboBox → combo box | caption; value = choice; expanded while the popup is open | Click, Expand |
| Dropdown popup | ListBox, ListBoxOption children | option, selected, position; the cursor option is focused | Click (pick) |
| `SearchableDropdown` | ComboBox | placeholder; value; expanded | Click, Expand |
| Searchable popup | ListBox (description = query), filtered options | label, description | Click (pick) |
| `PopupMenu` | Menu, MenuItem / MenuItemCheckBox (checked) | label, shortcut, disabled, position | Click (pick), Focus |
| `Menu` | Menu (description = filter), MenuItem | label, detail, disabled, position; ±50 entries around the cursor | Click (pick), Focus |
| `ConfirmDialog` | AlertDialog → alert, modal | message; Cancel / confirm buttons (the selected one is focused) | Click |
| `Notification` | Alert → notification, live (assertive when critical) | summary; description = body | Click (dismiss) |
| `OperationToast` | Status → status bar, live polite | title; description = detail (and "Paused"); ProgressIndicator; Pause/Resume and Cancel buttons | Click, Focus |
| `ShortcutHelp` | Dialog, modal; Group per section; Label per row ("Ctrl+S: Save") | — | — |
| `PopupCard` | Dialog, modal; its widgets inside | — | — |
| `List` | ListBox → list box, ListBoxOption → list item | rows in view plus the selected row: label, description = detail, selected, "43 of 100000" | Click (select), Focus, ScrollIntoView |
| `table::Table` | Grid → table; header Row of ColumnHeader (sort direction); Row → table row of Cell | row/column counts of the whole table; rows in view plus the cursor row; selected | Click (select by ID), Focus, ScrollIntoView; Click on a header sorts |
| `table::Tree` | Tree → tree, TreeItem → tree item | label, description, level, expanded, selected, position among siblings | Click, Focus, Expand, Collapse, ScrollIntoView |
| `focus::host` canvas (`KeyboardPanel`) | Group, one ListBox per panel (panel name) | 25 items each side of the cursor, labels from `KeyboardPanel::accessible_labels`; the cursor item of the active panel is focused | Click (select), Focus / ScrollIntoView (move the cursor) |
| `announce` | Status, live | the text | — |

**Virtual rows.** Lists, tables, trees and panels expose the rows in view
(plus the cursor/selected row), each with its position in the whole set,
so the AT reads "Song 43, 43 of 10000". They never create one node per
item. Rows are keyed by the stable item ID (`KeyboardPanel`, `Table`,
`Tree`) or the index (`List`), so the focused node follows the cursor.

**Plain fltk widgets.** Button family (Button, CheckButton → check box,
RoundButton/radio → radio button, LightButton → switch, toggle/radio
types), Input family (entry, password, multiline, read-only Output), Text
display/editor (document/multiline entry with the buffer text), Slider, Dial,
Roller, Counter, ValueInput, Spinner, Progress, Choice (combo box),
MenuButton, MenuBar, Browser (list box with up to 200 lines around the
selection), Frame/Box with a label (label), and labeled groups. Click on a
plain button runs its callback (toggling check/radio buttons first).
SetValue/Increment/Decrement set valuators and inputs and run their
callback.

## Naming icon-only controls

Every control needs a name that matches what a sighted user sees:

```rust
use fltk_omarchy::a11y::AccessibleExt; // for components other than Button / ActionButton
win.header().action(glyph::SETTINGS, "Preferences", open_prefs); // the tooltip is the name
let close = Button::icon(glyph::CLOSE, "").tooltip("Close tab");  // tooltip again
let add = Button::icon(glyph::PLUS, "").accessible_name("New tab");
let wifi = ToggleSwitch::new(&on).accessible_name("Wi-Fi");
let vol = Slider::new(&v, 0.0, 1.0, 0.05).accessible_name("Volume");
a11y::set_name(&plain_fltk_widget, "Path"); // any fltk widget
```

When a widget has a name and a different tooltip, the tooltip becomes its
description. `accessible_description(..)` sets the description directly.
Glyphs are never read.

## Custom widgets

Implement `a11y::Accessible`, or call `a11y::describe` and `a11y::on_action`:

```rust
use fltk_omarchy::a11y::{self, Accessible, Action, Node, Request, Role, Value};

impl Accessible for ChatView {
    fn describe(&self, n: &mut Node) {
        n.role(Role::Document).name("Conversation").read_only(true);
        for m in self.visible_messages() {          // virtual: only what is near the viewport
            n.child(m.id, |p| {
                p.role(Role::Paragraph).name(&m.text).bounds(m.x, m.y, m.w, m.h)
                 .position(m.index + 1, self.len());
            });
        }
    }
    fn act(&self, r: &Request) -> bool {
        match (r.action, r.key()) {
            (Action::ScrollIntoView, Some(id)) => { self.scroll_to(id); true }
            _ => false,
        }
    }
}
a11y::register(&*chat_widget, chat.clone());
```

- `Node` setters: `role`, `name`, `description`, `value`, `placeholder`,
  `numeric(value, min, max, step)`, `toggled`, `expanded`, `selected`,
  `current`, `disabled`, `read_only`, `modal`, `multiselectable`, `hidden`,
  `live`, `level`, `position(n, of)`, `row_index`, `column_index`,
  `counts(rows, cols)`, `sort`, `bounds` (window coordinates), `action`,
  `focusable`, `shortcut`, `text_selection(anchor, focus)` (characters),
  `child(key, ..)` (virtual children, any depth), `focused` (this child has
  the keyboard focus while the widget has it), and `leaf` (do not expose
  the fltk children).
- Text roles (TextInput, PasswordInput, SearchInput, MultilineTextInput,
  Document, Paragraph) get one AccessKit text run per line with character
  lengths and word starts. The AT can then read by character, word and
  line and follow the caret and selection. A Paragraph without a value uses
  its name as its text.
- The describe callback runs only while an AT is active, after a repaint
  of its window. If your data changes without a repaint, call
  `a11y::changed(&widget)`.
- `Request::path` lists the keys of the virtual children down to the
  target; `Request::value` carries `Value::Text`, `Value::Number` or
  `Value::Selection { anchor, focus }`.

For a future RichText/Markdown view: expose the view as `Role::Document`
with one `Paragraph`/`Heading`/`ListItem`/`Link` child per block near the
viewport (plus the block with the caret). Give each its text and
`position(n, of)`, put `text_selection` on the block that holds the
selection, and handle `ScrollIntoView` and `SetTextSelection`. Streaming
output can use `a11y::announce(.., Live::Polite)` or a live Status child.

## Announcements

`a11y::announce("3 results", Live::Polite)` puts the text in a hidden live
Status node of the focused window. The AT reads it without moving focus.
Announcing the same text twice reads it twice. `Live::Assertive`
interrupts; keep it for errors.

## Testing

**Without an AT** (unit and UI tests, agents):

```rust
let text = fltk_omarchy::a11y::snapshot(&*win).unwrap(); // one node per line
assert!(text.contains("Switch \"Wi-Fi\" [checked]"));
```

`a11y::testing::record(&*win)` (doc-hidden) attaches a recording adapter
to a window, shown or not. `sync()` runs the per-loop sync, `take()`
returns the `TreeUpdate`s, and `find(role, name)`, `focus()` and
`act(node, action, data)` send actions as an AT would. `tests/ui.rs` feeds
every recorded update to AccessKit's own consumer, which panics on an
inconsistent tree like the platform adapters do. It checks roles, names,
values, focus after Tab, that a State change sends only the changed
nodes, and actions on every component. Run it with
`cargo test --features a11y` (UI tests need a display; `xvfb-run -a`
works).

**Through AT-SPI on Linux, without a screen reader**: `script/a11y-check`
runs `examples/a11y_check` on the hidden Hyprland workspace
`special:omarchy-fltk-test` (silent: your focus is never touched) inside a
private D-Bus session with its own AT-SPI bus and registry. Turning AT-SPI
on there does not change your session, and nothing speaks.
`script/a11y_atspi.py` (python3 + `gi`/Atspi) prints the AT-SPI tree, then
checks four things through AT-SPI only, never with synthetic input:
`DoAction("click")` reaches the button's `on_click`, a switch toggles, the
slider's `Value.SetCurrentValue` lands, and `EditableText` reaches the
field. The 2026-11-04 report:
[evidence/a11y-20261104/atspi-report.txt](evidence/a11y-20261104/atspi-report.txt).

**With a screen reader** (by hand):

- **Linux, Orca**: run your app, start Orca (`orca`, or Super+Alt+S in
  GNOME). Tab through the controls, use Orca's flat review, press Enter on
  a list item. The app becomes accessible when AT-SPI's `IsEnabled` is
  on (desktops set it with the screen-reader setting). If it stays off in
  your session, set it:
  `gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled "<true>"`.
- **Windows, NVDA or Narrator**: start NVDA (or Ctrl+Win+Enter for
  Narrator), Tab through the window, NVDA+Tab reads the focus. Use
  Inspect.exe or Accessibility Insights to see the UIA tree.
- **macOS, VoiceOver**: Cmd+F5, VO+arrows to move, VO+Space to press;
  Accessibility Inspector (Xcode) shows the tree.

## Verified so far

- **Linux (x86_64, Hyprland 0.56.2, Wayland, at-spi2-core 2.62.1)**:
  - unit tests: 7 tree-generation tests, no bus
  - `tests/ui.rs`: 3 a11y checks through the recording adapter and
    AccessKit's consumer
  - the real AT-SPI check: 20/20 checks pass
- **Windows (x86_64-pc-windows-gnu)** and **macOS (aarch64-apple-darwin)**:
  type-checked with `cargo check` and `cargo clippy` with
  `--features a11y`. They have not been run: no Windows or macOS machine
  was available. To type-check without a cross C++ toolchain, skip the
  FLTK C++ build:

  ```sh
  mkdir -p target/cfltk-empty
  CFLTK_BUNDLE_DIR=$PWD/target/cfltk-empty cargo check --target x86_64-pc-windows-gnu --features a11y,fltk/fltk-bundled --lib
  ```

  This needs the platform layer to replace the Linux-only parts of the
  crate (inotify theme watcher, Hyprland Unix socket). Until then those
  files fail to build for those targets; the `a11y` code itself has no
  errors or warnings there.

## Limitations

- **Windows**: AccessKit's `SubclassingAdapter` must exist before the
  window is first shown, but fltk only creates the HWND in `show()`. The
  toolkit therefore installs its own window-procedure subclass for
  `WM_GETOBJECT` right after `show()`, on 64-bit Windows only. If an AT
  queries the window in that instant, it gets no answer and asks again.
- **macOS**: bounds are FLTK units × FLTK's screen scale. On a Retina
  display, VoiceOver's highlight box is half-size until the backing scale
  factor is applied; names, roles, values and actions are unaffected.
- **Wayland**: there are no global window coordinates, so AT-SPI bounds
  are window-relative (fine for Orca, wrong for screen magnifiers).
- Damage detection covers repaints of a top-level window. A change that
  repaints only inside a GL subwindow, or data that changes without any
  repaint, needs `a11y::changed`.
- A large document exposed as one text node is rebuilt and compared in
  full on every update. Expose the blocks near the viewport as virtual
  children instead (see "Custom widgets").
- `List` rows have no "activate" action of their own: Click selects, as a
  mouse click does. Enter from the keyboard still activates.
- A table row whose item ID is `u64::MAX - 7` would collide with the
  header row's key.
