# Keyboard

Every widget works without a mouse, and the rules are the same
everywhere.

## Rules

1. **Tab / Shift+Tab** move between focusable widgets (FLTK's normal
   navigation). Toolbar `IconButton`s are not Tab stops by default
   (Folio's choice); `focusable(true)` makes one.
2. **Arrows act inside the focused widget** (collection cursor, sidebar
   rows, breadcrumb segments, menu entries). `fo::init()` turns off FLTK's
   option that lets arrows move focus between widgets.
3. **Enter** activates (buttons, rows, items); **Space** activates buttons
   and menu entries.
4. **Focus rings show only while the keyboard is in use**, like CSS
   `:focus-visible`: a key press turns `fo::focus_visible()` on, a click
   turns it off. The ring is the accent.
5. **Menus and dialogs take the keyboard** while open and give focus back
   to the widget that had it when they close.
6. **Window shortcuts** (`fo::Window::shortcut(spec, fn)`) run when the
   focused widget did not use the key and no modal layer is open.

## Key specs

`fo::parse_key("Ctrl+Shift+N")`, also used by `Window::shortcut` and
`event_is`:

- Modifiers: `Ctrl`, `Shift`, `Alt`, `Super` (`Meta`, `Cmd`), and `Mod`,
  the platform's primary modifier (Ctrl on Linux and Windows, Cmd on
  macOS).
- Keys: letters and digits, symbols (`?`, `/`, `,`, `+`), `F1`–`F24`,
  `Left Right Up Down Home End PageUp PageDown Enter Escape Tab Space
  Backspace Delete Insert Menu`.
- Letters ignore case and match the Shift state as written. A symbol
  written without Shift matches the character typed: `?` matches
  Shift+/ on a US layout and whatever key types `?` elsewhere.
- Num Lock and Caps Lock never matter; keypad Enter is Enter.
- `fo::key_label(k)` gives the display form: `Ctrl+Shift+N`, `Alt+←`, `⏎`.

## Widgets

| Widget | Keys |
|---|---|
| Collection | ← ↑ → ↓ (grid-aware; in a list ← and → are left to `on_key`, e.g. to go into a folder and back), Home/End, PageUp/PageDown, Shift extends the selection, Ctrl+A selects all, Enter activates, typing jumps by name (type-ahead, 1 s buffer). `on_key` sees every key first (Backspace, Delete, F2, Menu...) |
| ColumnView | ← → that the focused column did not use move to the previous / next column (mirrored right to left); `on_step` decides first (e.g. → opens the selected folder in a new column) |
| Sidebar | ↑ ↓ Home End move the cursor (skipping hidden sections), Enter / → / Space open the row and call `on_finished`, Esc calls `on_finished` (hand the keyboard back to the files) |
| Breadcrumbs | ← → move between segments (and the "…" fold), Enter / Space go there |
| TextField | Enter → `on_enter`, Esc → `on_escape`, Tab → `on_complete` when set (otherwise Tab moves focus), `on_key` first look; FLTK's editing keys |
| Button, IconButton | Enter or Space |
| PopupMenu | ↑ ↓ (and j k with no modifier), Home/End, Enter or Space, → opens a submenu, ← or Esc closes it, a letter jumps to the next entry starting with it, Esc closes; Tab stays inside. **Any other key closes the menu and goes on to the app**, so window shortcuts work while a menu is open |
| CheckBox | Space or Enter toggles |
| Dialog | Enter in a field moves to the next field, in the last one (or anywhere else) confirms (or cancels when Cancel has focus), Esc cancels, Tab moves between the field and the buttons. A dialog keeps the keyboard: window shortcuts wait until it is answered |
| HelpOverlay | Esc, ? or Enter close it |

Right to left (`fo::rtl()`), Left and Right swap their meaning wherever
they follow the reading direction: Collection grids, Breadcrumbs, the
Sidebar's "toward the files" key, menus (Left opens a submenu) and
ColumnView.

## Mouse, for completeness

Collection: click selects, Ctrl+click or Alt+click toggles, Shift+click
selects a range, double-click activates, drag on empty space draws a
selection rectangle (Ctrl/Alt adds), drag a selected item starts a drag
(`on_drag_start`), right click opens `on_context`. Wheel and touchpad glide.
