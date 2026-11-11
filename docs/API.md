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
- `fo::Layer` — base for anything drawn above the content: `open(win)`,
  `close()`, `modal()`, `passes_keys()`, `relayout()`. Modal layers take
  the keyboard and give focus back on close.

## Theme — `theme.h`

- `fo::Rgb` — opaque 8-bit color; `.fl()` for `fl_color`.
- `parse_hex`, `to_hex`, `mix(a, b, t)`, `luminance`.
- `parse_colors_toml(text)` → `Palette` (`key = "#rrggbb"`, `mode = "light"`).
- `fo::Theme` — `color(key)` with Folio's fallbacks; `accent()`,
  `background()`, `dark_background()`, `lighter_background()` (derived when
  equal to the background), `foreground()`, `bright_foreground()`,
  `secondary_foreground()` (derived, ~4.5:1), `accent_foreground()` (black
  or white), `red() yellow() green() blue() magenta() cyan() orange()`,
  `hairline()`, `tint(color, alpha)`, `light()`, `radius()` (Hyprland
  rounding), `radius_small()` (≤ 4), `font_family()`. `Theme::load(dir)`,
  `Theme::fallback_colors()`.
- `fo::theme()`, `set_theme(t)`, `watch_theme(dir)`,
  `on_theme_change(fn)` / `remove_theme_listener(id)`.

The live theme is `~/.local/state/omarchy/current/theme/colors.toml`,
watched with inotify (debounced 80 ms, parents re-armed after every
switch). Rounding comes from Hyprland's `decoration:rounding` over its
request socket and is read again on `configreloaded`. Off Omarchy the
built-in dark palette with Omarchy's yellow accent is used.

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
  `position`, `passes`); `fo::ScrollBar` — the thin overlay bar;
  `fo::ScrollArea` — a vertical scrolling container for one child.

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
  via `Fl::add_fd`), `cancel(id)`, `running_processes()`.
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
  `set_language`, `language()`, `rtl()`, `parse_language`.
- Catalogs: embedded JSON, layered en → base → regional (pt → pt-BR);
  values are strings or CLDR plural forms. The toolkit ships its own few
  strings in es, pt, fr, de and it.
- `tr`, `trn` (plural + localized number), `arg` (Qt-style `%1`),
  `plural_category(lang, n)` (Omarchy's languages, as Folio).
- Formatting with Omarchy's locale registry (`$OMARCHY_PATH`/
  `/usr/share/omarchy/default/i18n/locales.json`): `format_number`,
  `format_size` ("31.6 KB", decimal units as Folio), `format_date`,
  `format_time`, `format_when` (time today, date before).
- `on_change(fn)` / `remove_listener(id)`.

## Platform — `platform.h`, `hyprland.h`, `json.h`

- `platform::env`, `home_dir`, `config_dir`, `state_dir`, `read_file`,
  `exists`, `parent`; `platform::FileWatch` (inotify through
  `Fl::add_fd`, debounced; no-op off Linux).
- `hypr::available`, `request(cmd, done)` (async on the FLTK loop),
  `request_sync`, `subscribe(fn)` (event socket), reply parsers.
- `json::parse` — tiny reader for Hyprland replies and catalogs.

POSIX pieces (processes, inotify, sockets, `locale_t`) are behind
`#ifdef`s with fallbacks so the library can be brought to macOS and
Windows later.

## Widgets

| Widget | Header | Screenshot |
|---|---|---|
| `Button` (primary / plain) | `widgets.h` | [dialog](components/dialog.png) |
| `CheckBox` | `widgets.h` | (in `Dialog::ask`) |
| `IconButton` + tooltip | `widgets.h`, `dialog.h` | [tooltip](components/tooltip.png) |
| `TextField` | `widgets.h` | [prompt](components/prompt.png) |
| `Breadcrumbs` | `widgets.h` | [grid](components/grid.png) |
| `Rule`, `SplitPane` | `widgets.h` | [grid](components/grid.png) |
| `Sidebar` | `sidebar.h` | [sidebar-focus](components/sidebar-focus.png) |
| `Collection` (grid / list) | `collection.h` | [grid](components/grid.png), [list](components/list.png) |
| `PopupMenu` | `menu.h` | [menu](components/menu.png), [sort](components/sort.png) |
| `Dialog` (confirm / prompt) | `dialog.h` | [dialog](components/dialog.png), [prompt](components/prompt.png) |
| `Toast` | `dialog.h` | [toast](components/toast.png) |
| `HelpOverlay` | `dialog.h` | [help](components/help.png) |
| `ScrollArea`, `ScrollBar` | `scroll.h` | (any scrolled view) |

- **Button** — `primary(bool)`, `on_click(fn)`, `preferred_width()`.
- **IconButton** — `icon(name)`, `tooltip_text(t)` (shown after 600 ms),
  `active_state(bool)` (toggled tint), `focusable(bool)`, `on_click(fn)`.
- **TextField** — `placeholder`, `on_enter`, `on_escape`, `on_change`,
  `on_complete` (Tab), `on_key` (first look), `set_text(text, from, to)`,
  `secret(bool)` (passwords).
- **CheckBox** — `checked()`, `checked(bool)`, `on_change(fn)`,
  `preferred_width()`; click, Space or Enter toggles.
- **Breadcrumbs** — `segments({{label, path}...})`, `on_navigate(index)`;
  folds leading segments into "…" (a menu) when narrow; Left/Right/Enter.
- **SplitPane** — two children, draggable hairline divider, `set_split(px,
  animate)`, `min_sizes`, `collapse_second(on, animate)`, `on_change`.
- **Sidebar** — `add_section(title, id)`, `add_row(SidebarRow{label, icon,
  id, status, status_color, ejectable})`, `section_visible`,
  `set_active(id)`, `set_cursor`; `on_activate`, `on_eject`, `on_header`,
  `on_finished` (Esc/Enter hand the keyboard back), `on_context`,
  `accepts_drop` / `on_drop`.
- **Collection** — subclass with `count()`, `draw_item(i, x, y, w, h,
  state)`, `item_text(i)`; `layout(List|Grid)`, `row_height`,
  `tile_size`, `padding`, `reload(reset)`; `set_current`, `select`,
  `select_all`, `clear_selection`, `selection()`, `ensure_visible`,
  `scroll_to`, `item_rect`, `item_at`; callbacks `on_activate`,
  `on_selection_change`, `on_context`, `on_drag_start`, `on_key`,
  `on_type_ahead`, `accepts_drop` / `on_drop`; helpers `draw_row(...)` and
  `draw_tile(...)` in Folio's look (the tile returns its art box for a
  thumbnail).
- **PopupMenu** — `open_at(win, x, y, items)`, `open_below(win, rect,
  items, align_right)`, `on_dismiss`; `MenuItem{label, shortcut, action,
  submenu}` with `.with_icon()`, `.check()`, `.disabled()`,
  `MenuItem::separator()`.
- **Dialog** — `confirm(win, title, message, action, done(bool), danger)`,
  `prompt(win, title, message, initial, action, done(ok, text), from,
  to)`, and the general form `ask(win, DialogSpec, done(DialogAnswer))`:
  any number of fields (`DialogField{placeholder, initial, secret}`),
  buttons (the last is primary), an optional check box ("Apply to all")
  and `scrim_cancels`; the answer carries the button index (-1 for
  cancelled), the values and the check. `on_validate`, `set_error`,
  `accept()`, `choose(i)`, `cancel()`.
- **Toast** — `Toast::show(win, text, seconds, Info|Error)`.
- **Tooltip** — `Tooltip::open_for(widget, text)`, `Tooltip::close_all()`.
- **HelpOverlay** — `HelpOverlay::show(win, {{keys, description}...},
  title)`.

Screenshots are taken from the gallery in a private Xvfb with
`gallery --demo=<name>` (grid, list, menu, sort, dialog, prompt, help,
toast, tooltip, sidebar-focus), with Folio's fallback palette.

## What is verified how

- Unit tests (ctest): JSON, colors.toml, theme fallbacks and derivation,
  easing and tweens, scroller, Hyprland reply parsing, eliding and
  wrapping, key specs, plural rules, catalogs and size formatting,
  language file reading, icons, processes (exit codes, exec failure,
  timeout killing the group, output cap, cwd/env, CPU and address-space
  limits, async delivery, cancel), the thread pool, and headless widget
  logic (Collection geometry, selection, keyboard and type-ahead; Sidebar
  keyboard).
- Visually (Xvfb screenshots, at scale 1 and 1.25): every widget state in
  the table above.
- Not exercised automatically: live pointer and wheel input, drag and
  drop, inotify reloads of the live theme, Hyprland IPC against a running
  compositor, Wayland.
