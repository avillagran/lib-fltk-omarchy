# Keyboard first (0.0.7)

Every lib-fltk-omarchy app can be used without a mouse, without writing any
keyboard code. `App::new` installs one keyboard router (`focus::install`)
and every component follows the same rules.

## Rules

1. **Tab / Shift+Tab** go through every control of the window in layout
   order and wrap around. The order is the widget tree, depth first, so it
   follows your columns and rows.
2. **Arrows act inside the focused control** (list cursor, slider, tabs,
   button group). They never jump to another control. fltk's per-group
   arrow navigation is off. An arrow the control does not use goes to the
   arrow scopes around it (see below), then to window shortcuts.
3. **Enter or Space** activate the focused control (button, toggle, row,
   dropdown trigger); Enter opens the list's cursor row.
4. **Menus and dialogs** take the keyboard when they open. Tab does not
   leave them, Esc closes them, and focus goes back to the control that had
   it.
5. **Esc never closes an app window.**
6. **Window-wide keys** keep working wherever focus is:
   `AppWindow::shortcut("F10", ..)`, `Button::shortcut("Ctrl+S")`, and
   Ctrl+Tab / Ctrl+PageUp / Ctrl+PageDown for a `TabBar`.
7. **The focus ring is drawn only while the keyboard is in use**, like
   `:focus-visible` on the web; a mouse click hides it again. The ring is
   the theme's `[controls] focus` state. When the theme's focus looks the
   same as its hover cursor (the Omarchy default), the ring gets a
   full-strength accent border so it cannot be missed.
   `theme::set_focus_ring(FocusRing::Theme)` uses the theme tokens exactly.

## Keyboard panels (one canvas, several Tab stops)

A canvas that draws several regions (a dual pane, a sidebar and a grid)
registers them as keyboard panels: `focus::host(&canvas, &[&left, &right])`.

- Each panel is a Tab stop, in that order. The router moves between them
  inside the same widget; the canvas's own handler never sees Tab.
- While a panel has focus it sees every key first, with any modifiers.
  Keys it does not use go on as usual (the canvas handler, then window
  shortcuts), once.
- A menu or dialog opened from a panel gives focus back to that panel.
- Calling `focus::host` again replaces the list, e.g. when F9 turns the dual
  pane back into one list.

Panel keys and the API: [COMPONENTS.md](COMPONENTS.md#keyboardpanel-regions-of-one-canvas).

## Keys per component

| Component | Keys |
|---|---|
| Button, ActionButton, Row, Toggle, ToggleSwitch | Enter / Space |
| ButtonGroup | one Tab stop; Left / Right change the selection |
| TextField | typing; Enter submits |
| NumberField | Up / Down |
| Slider | Left / Right / Up / Down |
| TabBar | anywhere: Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+PageDown, Ctrl+PageUp. Focused: Left / Right / Home / End, Delete closes |
| List | Up / Down, j / k, PageUp / PageDown, Home / End, Enter |
| Dropdown | Enter / Space / Down open; Up / Down, j / k; Enter; Esc |
| SearchableDropdown | Enter / Down open; type to filter; Up / Down; Enter; Esc |
| Menu | type to filter; Up / Down, PageUp / PageDown; Enter / Right; Backspace, Ctrl+Backspace, Ctrl+U; Esc clears, then closes |
| ConfirmDialog | Left / Right / Tab; Enter; Esc cancels |
| ShortcutHelp | Up / Down / PageUp / PageDown scroll; Esc / F1 close |
| Notification | reached with Tab; Enter / Space / Delete / Esc dismiss |
| OperationToast | reached with Tab or `toast.focus()`; Left / Right choose; Enter / Space press; Esc leaves |
| PopupMenu | Up / Down, j / k (skip disabled, wrap), PageUp / PageDown, Home / End; Enter; Esc |
| PopupCard | Tab / Shift+Tab stay inside; Esc closes, focus comes back |
| Table | the KeyboardPanel keys; Enter activates |
| Tree | the KeyboardPanel keys, plus Right (expand / first child) and Left (collapse / parent) |
| ShortcutHelp | Up / Down, PageUp / PageDown, Home / End (bounded); Esc / F1 close |
| KeyboardPanel | Up / Down (Left / Right in a grid), with Ctrl (cursor only), Shift (range), Ctrl+Shift (add range); PageUp / PageDown, Home / End; Ctrl+Space, Ctrl+A; Enter |

## For app code

```rust
use fltk_omarchy::focus;

// Arrow keys for your own group of controls (panels, a sidebar):
focus::scope(&sidebar_and_content, |key| match key {
    fltk::enums::Key::Right => { /* focus the content */ true }
    fltk::enums::Key::Left => { /* focus the sidebar */ true }
    _ => false,
});

focus::first(&*win);          // focus the first control (e.g. after building)
focus::step(&*win, true);     // same as Tab (false = Shift+Tab)
focus::is_focused(&*widget);  // who has focus
focus::show_ring(&*widget);   // draw a focus ring in a custom widget?
```

A custom widget joins the Tab order when it is visible, active, has
`set_visible_focus()` and returns `true` for `Event::Focus` in its
`handle`. Draw its ring with `theme.controls.focus` when
`focus::show_ring(w)` is true.

## Testing without a keyboard

`focus::click_at(&*win, x, y)` sends a left click the same way.
`focus::press("Tab")`, `press("Shift+Tab")`, `press("Down")`,
`press("Ctrl+Tab")`, `press("Enter")` or `press("a")` send a key through the
same path as a real key press. It also works on windows that are never
shown. `tests/ui.rs` drives a whole window with it ("keyboard only" check),
and `examples/docs_shots.rs` uses it for the keyboard pictures.

## Real keyboard through the compositor (0.0.5)

`script/os-input-check` opens `examples/os_input_check` floating on the
current workspace and gives it focus. It types real keys with `wtype` (the
Wayland virtual-keyboard protocol, delivered by Hyprland like hardware
keys), then gives focus back to the window that had it. It needs the
user's go-ahead: for about 3 s, keystrokes go to the test window.

Result on 2026-10-28 (`docs/os-input-2026-10-28.json`), Hyprland 0.56.2:

| Keys | Result |
|---|---|
| `omarchy` typed into the focused field | text `"omarchy"` |
| Tab, Enter, Space | Add pressed twice |
| Tab, Right, Right | ButtonGroup on Month (2) |
| Tab, Ctrl+Tab | TabBar on Work (1), app-wide shortcut |
| Tab, Down ×3 | List cursor on Item 2 |
| F10, Down, Enter | window shortcut opened the PopupMenu, Copy (1) picked, focus back on the list |
| Shift+Tab | TabBar |

Focus trace: field → Add → ButtonGroup → TabBar → List → menu → List →
TabBar → none. The last step is the test giving focus back to the user's
window.

## Real mouse (0.0.7)

Validated twice with the user's go-ahead:
- `script/os-mouse-check` ran `ydotoold` for the duration of the test only.
  Two button clicks and a switch click registered. A header drag moved
  the window with the native compositor gesture (+80, +40).
- The user then tested by hand with `examples/playground`, every action
  logged to `build/playground.log`. The log shows the native move, the
  native resize from the corners (960×730 down to 462×402), Normal /
  Floating / Fullscreen modes, menus, lists, the table, the tree and the
  toasts, with no panic.

These tests found three bugs, now fixed:
- **The first real click on a button crashed the app** (a re-entrant
  `RefCell` borrow when the button took focus). A regression test clicks
  every clickable control through fltk's event path.
- **Partial redraws erased controls.** Hovering a control made it
  disappear, and hovering a menu hid the window behind it. Group draws now
  skip cfltk's pre-draw, and the window redraws everything that overlaps
  the damaged area in stacking order (`examples/redraw_check.rs` checks
  real window pixels).
- **A window opened floating in Hyprland could not be tiled or resized.**
  It was mapped with a fixed size to make it float. It now maps normally
  and is floated right away (`examples/mode_check.rs`).

Pointer warps over IPC (`hl.dsp.cursor.move`) were not reliable for
aiming on this two-monitor, mixed-scale setup; the real-mouse evidence
comes from the hand test.
