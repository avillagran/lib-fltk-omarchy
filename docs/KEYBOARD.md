# Keyboard first (0.0.2)

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
| PopupMenu | Up / Down, j / k (skips disabled entries); Enter; Esc |
| Menu | type to filter; Up / Down, PageUp / PageDown; Enter / Right; Backspace, Ctrl+Backspace, Ctrl+U; Esc clears, then closes |
| ConfirmDialog | Left / Right / Tab; Enter; Esc cancels |
| ShortcutHelp | Up / Down / PageUp / PageDown scroll; Esc / F1 close |
| Notification | reached with Tab; Enter / Space / Delete / Esc dismiss |
| OperationToast | reached with Tab or `toast.focus()`; Left / Right choose; Enter / Space press; Esc leaves |

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

`focus::press("Tab")`, `press("Shift+Tab")`, `press("Down")`,
`press("Ctrl+Tab")`, `press("Enter")` or `press("a")` send a key through the
same path as a real key press. It also works on windows that are never
shown. `tests/ui.rs` drives a whole window with it ("keyboard only" check),
and `examples/docs_shots.rs` uses it for the keyboard pictures.

Not validated yet: key delivery from a physical keyboard through the
compositor. The router sits after that point, but no test types on a real
device.
