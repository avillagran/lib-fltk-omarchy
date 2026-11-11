# API tour

Everything is in namespace `fo`, headers under `include/fltk-omarchy/`,
one static library target `fltk-omarchy::fltk-omarchy`.
`#include "fltk-omarchy/fo.h"` brings in all of it. Every header documents
its classes in place; this page is the map.

## App setup — `app.h`, `window.h`

```cpp
fo::init();                                  // once, before windows
auto *win = new fo::Window(1100, 700, "Flo");
win->shortcut("Ctrl+L", [] { /* ... */ });   // window-wide keys
```

- `fo::init(InitOptions)` — loads and follows the theme and the language,
  applies the font, starts the Hyprland watchers, enables `Fl::awake` for
  tasks, turns off FLTK's arrow-key focus moves and installs the
  keyboard-focus tracker. Options: theme dir, language file, which
  watchers to run, the window class.
- `fo::focus_visible()` — true while the keyboard is in use; focus rings
  are drawn only then.
- `fo::Window` — double-buffered window in the theme background that hosts
  **layers** (menus, dialogs, toasts, tooltips, help) above its content.
  `shortcut(spec, fn)`, `remove_shortcut`, `run_shortcut()` (runs the
  shortcut for the current key from a widget that would otherwise take
  it, e.g. type-ahead), `shortcut_specs`,
  `top_layer()`, `modal_open()`, `Window::of(widget)`.
  `move_area(fn(x, y))` makes a free area (Files' header without its
  controls) show a hand and move the window on a left press;
  `begin_move()` asks for that move from the press being handled:
  Wayland through FLTK's own `xdg_toplevel.move` path (a reposition while
  only the left button is down), X11 through EWMH `_NET_WM_MOVERESIZE`,
  or the window follows the drag itself without a window manager that
  supports it. Public FLTK API only; interactive resizing from the
  corners has no public path on Wayland and is left to the compositor.
- `fo::Layer` — base for anything drawn above the content: `open(win)`,
  `close()`, `modal()`, `passes_keys()`, `relayout()`. Modal layers take
  the keyboard and give focus back on close. Subclasses that call
  `fade_in()` and draw with `presence()` fade out on close as well
  (`closing_out()` while it lasts); dialogs, toasts and the help overlay
  do.
- `paint::pop_opacity(alpha, scale, cx, cy)` composites a card scaled
  around a point (cairo only).

## Theme — `theme.h`

- `fo::Rgb` — opaque 8-bit color; `.fl()` for `fl_color`.
- `parse_hex`, `to_hex`, `mix(a, b, t)`, `luminance`.
- `parse_colors_toml(text)` → `Palette` (`key = "#rrggbb"`, `mode = "light"`).
- `fo::Theme` — `color(key)` with the fallbacks below; `accent()`,
  `background()`, `dark_background()`, `lighter_background()` (derived when
  equal to the background), `foreground()`, `bright_foreground()`,
  `secondary_foreground()` (derived, ~4.5:1), `accent_foreground()` (black
  or white), `red() yellow() green() blue() magenta() cyan() orange()`,
  Files' roles `card()` (foreground at 8 %: cards, chips, hover, the
  current tab), `dim()` (metadata, idle icons), `muted()`, `selection()`
  (marked items), `hairline()`, `tint(color, alpha)`, `light()`, `radius()` (Hyprland
  rounding), `radius_small()` (≤ 4), `font_family()`. `Theme::load(dir)`,
  `Theme::fallback_colors()`.
- `fo::theme()`, `set_theme(t)`, `watch_theme(dir)`,
  `on_theme_change(fn)` / `remove_theme_listener(id)`.

The live theme is `~/.local/state/omarchy/current/theme/colors.toml`,
watched with inotify (debounced 80 ms, parents re-armed after every
switch). Rounding comes from Hyprland's `decoration:rounding` over its
request socket and is read again on `configreloaded`. Off Omarchy the
built-in Tokyo Night palette is used, as in Files. The UI font follows
Files' rule: fontconfig's `monospace` when it is a Nerd Font, otherwise an
installed JetBrainsMono or FiraCode Nerd Font; with a Nerd Font, icon
names draw as its glyphs (`fo::glyph`), Lucide otherwise.

## Motion — `anim.h`, `scroll.h`

- `anim::now_ms()`, test clock `freeze()`, `advance(ms)`, `resume()`.
- `anim::Ease` (Linear, OutCubic, InCubic, InOutCubic, OutQuint, OutBack,
  EaseOut = Omarchy's Bézier), `anim::ease(e, t)`, `anim::bezier(...)`.
- `anim::ms::*` — durations (BUTTON 120, ROW 60, FOCUS 120, LAYOUT 190,
  POPUP_IN 180, POPUP_OUT 120, SCROLL 120, TOOLTIP_DELAY 600, ...).
- `anim::enabled()`, `set_enabled(-1|0|1)`, `parse_switch`,
  `watch_system()`, `refresh_hz()`, `request_frame(widget)` /
  `request_frame(fn, data)`, `busy()`.
- `fo::Anim<T>` — retargetable tween of `double`, `Rgb` or `RectF`:
  `set(target, ms, ease)`, `jump(v)`, `value()`, `get(widget)` (schedules
  the next frame while moving), `running()`.
- `fo::Scroller` — wheel glide math (`wheel`, `scroll_to`, `jump`,
  `position`, `passes`); `fo::ScrollBar` — the thin overlay bar, vertical
  or `ScrollBar::Horizontal`;
  `fo::ScrollArea` — a scrolling container for one child, `axes(Vertical`
  (default) `| Horizontal | Both)`: `scroll_to` / `scroll_to_x`,
  `ensure_visible` / `ensure_visible_x`, `offset` / `offset_x`,
  `max_offset` / `max_offset_x`. Shift+wheel and horizontal wheel steps
  scroll sideways; a nested scroller under the pointer gets the wheel
  first.

See [ANIMATION.md](ANIMATION.md).

## Processes and tasks — `process.h`

```cpp
fo::ProcessOptions o;
o.argv = {"file", "--brief", "--mime-type", path};   // no shell
o.timeout_s = 2; o.stdout_cap = 64 * 1024;
o.limit_address_space = 256 << 20; o.limit_cpu_s = 5;
fo::run(o, [](fo::ProcessResult r) { if (r.ok()) use(r.out); });
fo::tasks::run([=] { heavy(); }, [=] { update_ui(); });
```

- `run_sync(opts)` (blocking), `run(opts, done)` (done on the FLTK thread
  via `Fl::add_fd`; on Windows through a waiting thread and `Fl::awake`),
  `cancel(id)`, `running_processes()`.
- `ProcessOptions`: argv, cwd, extra env, timeout (kills the whole process
  group), stdout cap (kills on overflow by default), stderr capture,
  `RLIMIT_AS`, `RLIMIT_CPU`.
- `ProcessResult`: started, exit_code, signal, timed_out, truncated, out,
  err, error, `ok()`.
- `tasks::run(work, done)` — pool of up to 4 threads, `done` on the FLTK
  thread; `tasks::post(fn)` from any thread; `tasks::pending()`.

## Icons — `icons.h`

- `icon::draw(name, x, y, size, color, stroke = 0)` — Lucide icons through
  `Fl_SVG_Image`, rasterized at the screen's real resolution, cached per
  size/color (LRU of 256). Square caps on Omarchy's square theme.
- `icon::exists`, `names()`, `resolve(alias)`, `svg(...)`,
  `clear_cache()`, `license()`.
- About 115 icons: folders and file kinds, places (home, desktop,
  documents, downloads, pictures, music, videos, recent, trash, drive,
  usb, network, server, cloud, laptop), view controls (grid, list,
  columns, panels, split, sort, eye/eye-off), actions (copy, scissors,
  clipboard, pencil, link, terminal, external-link, refresh, plus, x,
  check, pin, star, search, info, more, chevrons, arrows) and `eject`
  (drawn here in Lucide's style). Regenerate with `tools/gen_icons.py`.

## Text — `text.h`, `paint.h`

- `text::apply_fonts(family)` — FLTK's faces → the Omarchy font
  (regular, bold, medium = `FL_TIMES`, semibold = `FL_TIMES_BOLD`).
- `text::size::{SMALL, LABEL, BODY, TITLE, HEADING}` = 11/12/13/14/16.
- `elide_end`, `elide_middle` (keeps the extension), `wrap(s, w, m,
  lines)`, `upper`, `utf8_offsets`, `width`, `draw(s, box, color, font,
  size, align, Elide)`, `draw_spaced` / `width_spaced` (letter-spaced
  section labels). Measuring is injectable (`text::Measure`) for tests.
- `paint::fill`, `round_fill`, `round_frame`, `hline`, `vline`, `line`,
  `circle` — with alpha and anti-aliased corners through FLTK's cairo
  context (Wayland, and X11 when FLTK draws with cairo), opaque fallbacks
  otherwise; `push_opacity()` / `pop_opacity(alpha)` fade a whole card.

## Keys — `keys.h`

`parse_key("Ctrl+Shift+N")` → `Key`, `key_label(k)`, `key_matches(...)`,
`event_is(spec)`. See [KEYBOARD.md](KEYBOARD.md).

## Translations — `i18n.h`

```cpp
fo::i18n::add_catalog("es", kSpanishJson);   // before init()/watch()
label(fo::tr("Open"));
std::string s = fo::trn("%n items", n);
```

- Language from `~/.config/omarchy/language`, followed live;
  `set_language`, `language()`, `rtl()` (also `fo::rtl()`),
  `parse_language`.
- Right to left (Arabic, Hebrew, Persian, Urdu, or a registry entry with
  `"direction": "rtl"`) the widgets mirror, read at draw and event time so
  a language switch takes effect live: Sidebar rows (icon at the right,
  status and eject at the left, Left hands the keyboard back), Breadcrumbs
  (the trail starts at the right edge; Left goes deeper), Collection rows
  (`draw_row`), grids (tiles flow from the right; Left moves forward) and
  scrollbars (left edge), menus (open to the left, mirrored entries, Left
  opens a submenu), SplitPane (the first pane on the right) and
  ColumnView. See [components/rtl.png](components/rtl.png). Dialog button
  order, the help overlay and toolbar layouts are left to the app.
- Catalogs: embedded JSON, layered en → base → regional (pt → pt-BR);
  values are strings or CLDR plural forms. The toolkit's own strings
  (Cancel, OK, Close, Open, Delete, Rename, Yes, No, Show more, Keyboard
  shortcuts, AM/PM, size units, "%n items") are translated into all 50
  languages of Omarchy's locale registry (`toolkit_languages()`), reusing
  Omarchy's own wording for Cancel, Delete, Open, Yes and No; plain `es`
  gets Omarchy's `es-MX` strings and `pt` is European Portuguese.
- `tr`, `trn` (plural + localized number), `arg` (Qt-style `%1`),
  `plural_category(lang, n)` (Omarchy's languages, as Folio).
- Formatting with Omarchy's locale registry (`$OMARCHY_PATH`/
  `/usr/share/omarchy/default/i18n/locales.json`): `format_number`,
  `format_size` ("31.6 KB", decimal units), `format_date`,
  `format_time`, `format_when` (time today, date before); the system
  locale when installed, built-in short patterns otherwise
  (`use_system_locales(false)` forces them).
- `on_change(fn)` / `remove_listener(id)`.

## Platform — `platform.h`, `hyprland.h`, `json.h`

- `platform::env`, `home_dir`, `config_dir`, `state_dir`, `read_file`,
  `exists`, `parent`; `platform::FileWatch` (debounced; inotify on Linux,
  kqueue on macOS and the BSDs, polling elsewhere or with
  `Backend::Polling`; `backend()`, `poll_interval()`).
- `hypr::available`, `request(cmd, done)` (async on the FLTK loop),
  `request_sync`, `subscribe(fn)` (event socket), reply parsers.
- `json::parse` — tiny reader for Hyprland replies and catalogs.

OS pieces sit behind `#ifdef`s: processes run on POSIX and Windows, file
watching has inotify, kqueue and polling backends, and formatting falls
back to built-in patterns without `locale_t`. See
[PORTABILITY.md](PORTABILITY.md) for what is tested where.

## Widgets

| Widget | Header | Screenshot |
|---|---|---|
| `Button` (primary / plain) | `widgets.h` | [dialog](components/dialog.png) |
| `CheckBox` | `widgets.h` | (in `Dialog::ask`) |
| `ToolBar` (Files' header buttons) + tooltip | `shell.h`, `dialog.h` | [tooltip](components/tooltip.png) |
| `TabStrip`, `StatusBar`, `SortHeader` | `shell.h` | [groups](components/groups.png), [list](components/list.png) |
| `IconButton` | `widgets.h` | (any icon button) |
| `TextField` | `widgets.h` | [prompt](components/prompt.png) |
| `Breadcrumbs` (path chips) | `widgets.h` | [groups](components/groups.png) |
| `Rule`, `SplitPane` | `widgets.h` | (between panes) |
| `Sidebar` (Files' rail) | `sidebar.h` | [rail](components/rail.png), [sidebar-focus](components/sidebar-focus.png) |
| `Collection` (bands / grid / list) | `collection.h` | [groups](components/groups.png), [grid](components/grid.png), [list](components/list.png) |
| `PopupMenu` | `menu.h` | [menu](components/menu.png), [actions](components/actions.png) |
| `Dialog` (confirm / prompt) | `dialog.h` | [dialog](components/dialog.png), [prompt](components/prompt.png) |
| `Toast` | `dialog.h` | [toast](components/toast.png) |
| `HelpOverlay` | `dialog.h` | [help](components/help.png) |
| `ScrollArea`, `ScrollBar` (both axes) | `scroll.h` | (any scrolled view) |
| `ColumnView` (Miller columns) | `columns.h` | [columns](components/columns.png) |

- **ToolBar** — Files' header: `items({{icon, label, shortcut, on, enabled,
  action}...})`, `set_on`, `set_enabled`; hovering reveals a button's label
  (160 ms) and resting two seconds shows its shortcut under it;
  `preferred_width()` grows while a label shows and `on_relayout(fn)` asks
  the owner to place it again; `item_rect(i, ...)` to open a menu under a
  button; `plain(true)` + `metrics(30, 6)` for back/forward/up.
- **TabStrip** — `tabs(labels, current)`, `on_select`, `on_close` (the ×
  or a middle click), `on_new` ("+" or a click on empty space),
  `on_context`, `on_reorder(from, to)` (Files' drag: after 5 px an accent
  line shows the drop point, the release inside the strip asks for the
  move, leaving the strip cancels); the current tab sits on a card over a
  sliding accent line.
- **StatusBar** — `left(text, error)`, `hint(text)`, `right(text)`,
  `badge(text, color)` (Files: marked/listed, a tip, the filesystem, PSS).
- **SortHeader** — `columns({{label, icon, x, w, right}...})`,
  `sorted(column, descending)`, `on_sort(column)`.
- `draw_action_button(...)`, `draw_section_label(...)` — Files' panel
  buttons and "FOLDERS · 3" labels, for custom panels (an inspector).
- **Button** — `primary(bool)`, `on_click(fn)`, `preferred_width()`.
- **IconButton** — `icon(name)`, `tooltip_text(t)` (shown after 600 ms),
  `active_state(bool)` (toggled tint), `focusable(bool)`, `on_click(fn)`.
- **TextField** — `placeholder`, `on_enter`, `on_escape`, `on_change`,
  `on_complete` (Tab), `on_key` (first look), `set_text(text, from, to)`,
  `secret(bool)` (passwords).
- **CheckBox** — `checked()`, `checked(bool)`, `on_change(fn)`,
  `preferred_width()`; click, Space or Enter toggles.
- **Breadcrumbs** — Files' chips: `segments({{label, path}...})`,
  `on_navigate(index)`, `on_empty_click(fn)` (the current chip or empty
  space: Files edits the path); folds leading chips into a "…" chip (a
  menu, `open_fold()`) when narrow; Left/Right/Enter.
- **SplitPane** — two children, draggable hairline divider, `set_split(px,
  animate)`, `min_sizes`, `collapse_second(on, animate)`, `on_change`.
- **Sidebar** — Files' rail with `rail(true, 52, 170)`: icons that expand
  to labels on hover intent (350 ms, folds 140 ms after leaving), on
  keyboard focus or when `pinned(true)` (the pin control calls `on_pin`);
  `desired_width()` and `on_width(fn)` let the owner push the content.
  `add_section(title, id)`, `add_row(SidebarRow{label, icon,
  id, status, status_color, ejectable, footer})`, `section_visible`,
  `set_active(id)`, `set_cursor`; `on_activate`, `on_eject`, `on_header`,
  `on_finished` (Esc/Enter hand the keyboard back), `on_context`,
  `accepts_drop` / `on_drop`.
- **Collection** — subclass with `count()`, `draw_item(i, x, y, w, h,
  state)`, `item_text(i)`; `layout(List|Grid|Bands)`, `row_height`,
  `bands({{count, min_width, item_h, gap, header_h, after}...})` with
  `draw_band_header` and `band_header_clicked` (Files' Groups: folder
  cards, then rows by age), `always_ring`, `ring_radius`,
  `tile_size`, `padding`, `focus_ring_on_selected`, `reload(reset)`; `set_current`, `select`,
  `select_all`, `clear_selection`, `selection()`, `ensure_visible`,
  `scroll_to`, `item_rect`, `item_at`; callbacks `on_activate`,
  `on_selection_change`, `on_context`, `on_drag_start`, `on_key`,
  `on_type_ahead`, `accepts_drop` / `on_drop`; helpers `draw_row(...)` and
  `draw_tile(...)` in Files' look (the tile returns its art box for a
  thumbnail).
- **PopupMenu** — `open_at(win, x, y, items)`, `open_below(win, rect,
  items, align_right)`, `on_dismiss`; `MenuItem{label, shortcut, action,
  submenu}` with `.with_icon()`, `.check()`, `.disabled()`,
  `MenuItem::separator()`. Moving the pointer diagonally toward an open
  submenu keeps it open (a safe triangle with a 300 ms grace,
  `PopupMenu::aiming`); `depth()`, `highlighted(card)`, `card_rect`,
  `item_rect` report the open cards.
- **Dialog** — `confirm(win, title, message, action, done(bool), danger)`,
  `prompt(win, title, message, initial, action, done(ok, text), from,
  to)`, and the general form `ask(win, DialogSpec, done(DialogAnswer))`:
  any number of fields (`DialogField{placeholder, initial, secret}`),
  buttons (the last is primary), an optional check box ("Apply to all")
  and `scrim_cancels`; the answer carries the button index (-1 for
  cancelled), the values and the check. `on_validate`, `set_error`,
  `accept()`, `choose(i)`, `cancel()`.
- **ColumnView** — Miller columns in a horizontal `ScrollArea`:
  `column_width(px)` (260), `add_column(widget)` (owned, placed, kept in
  view with `follow_last`), `truncate(n)` / `clear()`, `columns()`,
  `column(i)`, `index_of(widget)`, `set_active(i, focus, animate)` (gives
  the column the keyboard and scrolls it into view), `active()`,
  `on_active_change`, `reveal(i)`. Left/Right that the focused column did
  not use (a list `Collection` leaves them) move between columns;
  `on_step(from, dir)` sees them first, e.g. to open the selected folder
  in a new column on Right; `keyboard_navigation(false)` turns them off.
  Mirrored when `fo::rtl()`. Flo's `ColumnsView` follows the same
  pattern (a trail of `FileView`s plus the selected folder's contents)
  and can move onto it without changing what the user sees.
- **Toast** — `Toast::show(win, text, seconds, Info|Error)`.
- **Tooltip** — `Tooltip::open_for(widget, text)`, `open_at(win, x, y, w,
  h, text)`, `Tooltip::close_all()`.
- **HelpOverlay** — `HelpOverlay::show(win, {{keys, description}...},
  title)`.

Screenshots are taken from the gallery, a Files-shaped window, in a
private Xvfb by `tools/screenshots.sh BUILD_DIR` (`gallery --demo=<name>`:
groups, list, grid, columns, rail, menu, actions, dialog, prompt, help,
toast, tooltip, sidebar-focus), with the Tokyo Night fallback palette.

## What is verified how

- Unit tests (ctest): JSON, colors.toml, theme fallbacks and derivation,
  easing and tweens, scroller, Hyprland reply parsing, eliding and
  wrapping, key specs, plural rules, catalogs (every Omarchy language has
  every toolkit string and a form for each plural category it uses) and
  size and date formatting,
  language file reading, icons, processes (exit codes, exec failure,
  timeout killing the group, output cap, cwd/env, CPU and address-space
  limits, async delivery, cancel), the thread pool, and headless widget
  logic (Collection geometry, selection, keyboard and type-ahead; Sidebar
  keyboard).
- Display tests (ctest `fo_display_tests`, inside a private Xvfb that
  `tests/run-xvfb.sh` starts for the run and stops by its PID; reported
  as skipped when Xvfb is missing): TextField erasing deleted text on a
  minimal update (with a check that the old no-box field really left
  glyphs behind), Collection's focus ring with and without
  `focus_ring_on_selected`, and fo::Window giving the keyboard back to the
  last widget after the window lost the X input focus (real XTest key
  presses when libXtst is available) and when a key or the focus comes
  back while nothing has the keyboard; TabStrip's drag-to-reorder and a
  window moving from its `move_area` (real XTest drags).
- Visually (Xvfb screenshots, at scale 1 and 1.25): every widget state in
  the table above.
- Not exercised automatically: live pointer and wheel input, drag and
  drop, inotify reloads of the live theme, Hyprland IPC against a running
  compositor, Wayland.
