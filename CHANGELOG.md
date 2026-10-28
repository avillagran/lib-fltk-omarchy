# Changelog

## 0.0.3 — 2026-10-28

For Files (issues #1, #3, #4, #5):
- **PopupMenu fits the window** (#1). A menu taller than the window is
  clamped to it and scrolls with the wheel and with Up/Down, PageUp/PageDown
  and Home/End, keeping the active entry visible. It opens below the anchor,
  or above it when there is more room. Wide labels end in "…" and keep
  their shortcut.
- **PopupMenu lifecycle** (#4). `open`/`open_at` return a `PopupHandle`:
  - `.on_dismiss(|Dismiss|)` reports `Picked(i)`, `Escape`, `ClickOutside`
    or `Closed`.
  - `.close()` closes it without picking.
  - `.is_open()`, `.dismissed()` and `.rect()` report its state.

  Focus still comes back to the control that had it. If the window is
  deleted with the menu open, no callback runs. **Breaking:** a closure that
  returned the old `()` from `PopupMenu::open(..)` now needs a `;`.
- **Keyboard panels** (#3, `panel::KeyboardPanel`, `focus::host`). One
  canvas can hold several Tab stops, each with a virtual cursor and a
  selection kept by stable IDs:
  - List and grid navigation.
  - Ctrl (cursor only), Shift (range) and Ctrl+Shift (add a range).
  - Ctrl+Space and Ctrl+A; select a range, invert, clear.
  - `on_activate`, `on_edge`, `on_key`, `on_change`.

  The focused panel sees keys first. Keys it does not use (an app's own
  Ctrl+Tab) are handled once.
- **Validated theme reload** (#5). `Theme::try_load()` and a strict TOML
  parser (`toml::parse_strict`). `reload()` keeps the last valid theme when a
  file is cut, malformed, missing required colors or mid-rename, and
  `theme::last_error()` says why. The built-in fallback still loads when
  Omarchy is not installed.
- `focus::click_at(&win, x, y)` sends a click through fltk's path (tests).

Fix: the natural-size registry of `fit` was keyed by widget address. A new
widget at a reused address could inherit a deleted widget's size. Entries
are now checked against the widget's deletion tracker.

## 0.0.2 — 2026-10-28

Keyboard first:
- One keyboard router installed by `App::new` (`focus` module). Tab and
  Shift+Tab move through every control in layout order and wrap. Arrows go
  only to the focused control and its arrow scopes (`focus::scope`); fltk's
  per-group arrow navigation is off.
- Menus, dialogs and popups give focus back to the control that opened
  them. A modal layer keeps Tab.
- New keyboard support:
  - TabBar: focusable, Left/Right/Home/End, Delete closes, and
    Ctrl+Tab / Ctrl+Shift+Tab / Ctrl+PageUp / Ctrl+PageDown from anywhere.
  - ButtonGroup: one Tab stop, Left/Right.
  - OperationToast: Tab reaches it, Left/Right/Enter/Space/Esc, and
    `focus()`.
  - Notification: Tab reaches it, Enter/Space/Delete/Esc dismiss it.
- The List draws a focus ring on the cursor row.
- The focus ring is drawn only while the keyboard is in use
  (`focus::keyboard_mode`). New `FocusRing::{Accent, Theme}`
  (`theme::set_focus_ring`). The accent ring is used when the theme's focus
  looks like its hover cursor, which is the Omarchy default.
- `focus::press("Tab")` sends a key through the real path (tests, scripted
  tours).

Fixes:
- Component bindings now belong to the fltk widget, not the Rust handle
  (`state::bind_widget`):
  - `row.fit(&*ButtonGroup::new(..))` used to stop updating once the
    temporary was dropped.
  - Label and Button bindings used to leak through an Rc cycle.
- Theme listeners of deleted windows are removed (`theme::on_change_while`).
  Restyling a deleted window used to panic.
- A Dropdown popup that loses focus to another layer now closes on the next
  loop turn. Closing it inside fltk's focus change used to crash.
- Closing a layer twice is a no-op.
- The declared minimum Rust version (1.80) really builds: `Option::is_none_or`
  (1.82) is replaced.

Docs: docs/COMPONENTS.md with a picture, code and keys per component
(`examples/docs_shots.rs`), docs/KEYBOARD.md, and losslessly recompressed
PNGs.

## 0.0.1 — 2026-10-27 (unreleased)

Added after review of the Files coordination thread: interior corner
preference (omarchy/square/rounded, fixed badge radius), `Mode::Fullscreen`
and mode parsing, PopupMenu (anchored and context), SearchableDropdown,
TabBar, ShortcutHelp, OperationToast; fltk features aligned with Files
(`use-wayland`, `no-images`).

First version: Omarchy theme (colors.toml, shell.toml, user override,
Hyprland tokens, live reload), classic components, overlays, virtual list,
AppWindow with normal/floating modes, State, tasks, keys, layout helpers,
offscreen snapshots, UI tests, real-window scenario and QML comparison.
