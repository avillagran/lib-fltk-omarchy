# Components (0.0.1)

All components deref to an fltk widget (place them in any `Flex`/`Group`)
and are created with their Omarchy implicit size; `flex.fit(&*w)` keeps it.
Colors, fonts, spacing and corner radius come from the live theme.

| Omarchy (`shell/Ui`) | Rust | Constructor and main methods |
|---|---|---|
| Button | `widgets::Button` | `new(text)`, `icon(glyph, text)`, `.primary()`, `.bordered()`, `.danger()`, `.left_aligned()`, `.tooltip()`, `.shortcut("Ctrl+S")`, `.on_click(f)`, `.bind_text(&State<String>)`, `.bind_enabled(&State<bool>)`, `set_selected`, `click()` |
| PanelActionButton | `widgets::ActionButton` | `new(glyph, tooltip)`, `destructive(glyph, tooltip)`, `.on_click(f)` |
| ButtonGroup | `widgets::ButtonGroup` | `new(&["Day","Week"], &State<usize>)` |
| TextField | `widgets::TextField` | `new()`, `password()`, `.placeholder()`, `.bind(&State<String>)` (two-way), `.on_change(f)`, `.on_submit(f)`, `type_text()` |
| NumberField | `widgets::NumberField` | `new(&State<i64>, from, to, step)` (Up/Down/wheel) |
| ToggleSwitch | `widgets::ToggleSwitch` | `new(&State<bool>)` |
| Toggle | `widgets::Toggle` | `new(title, description, &State<bool>)` |
| PanelSlider | `widgets::Slider` | `new(&State<f64>, min, max, step)` |
| (progress) | `widgets::Progress` | `new(&State<f64>)` 0..1 |
| text tokens | `widgets::Label` | `new`, `styled(text, TextStyle::Title)`, `.bold()`, `.tone(Tone::Dim)`, `.center()`, `.right()`, `.bind(&State<String>)`, `Label::section("OUTPUT")` |
| PanelSeparator | `widgets::Separator` | `new()` |
| PanelHero | `widgets::Hero` | `new(glyph, title, meta)`, `.detail("64%")` |
| CursorSurface row | `widgets::Row` | `new(glyph, label)`, `.detail()`, `.current(bool)`, `.on_click(f)` |
| Dropdown | `overlay::Dropdown` | `new(label, &options, &State<usize>)`, `open()` |
| menu plugin | `overlay::Menu` | `Menu::open(&*win, "Title", vec![MenuItem::new(glyph, label).detail(..).submenu()], |index| ..)` |
| ConfirmDialog | `overlay::ConfirmDialog` | `ConfirmDialog::open(&*win, message, "Delete", destructive, |ok| ..)` |
| notifications | `overlay::Notification` | `Notification::show(&*win, glyph, summary, body, Urgency::Normal)` |
| header menus / context menu | `popups::PopupMenu` | `PopupMenu::open(&*anchor_button, vec![PopupItem::new(glyph, "Copy").shortcut("Ctrl+C").checked(..).divider().disabled()], |i| ..)`, `open_at(&*win, x, y, items, f)` |
| SearchableDropdown | `popups::SearchableDropdown` | `new(placeholder, vec![(label, description)], &State<Option<usize>>)`, `open()`, `filter()` |
| F1 help | `popups::ShortcutHelp` | `ShortcutHelp::open(&*win, "Keyboard shortcuts", vec![(section, vec![(keys, description)])])` |
| operations tray | `popups::OperationToast` | `OperationToast::show(&*win, title)` → `.on_pause(|paused| ..)`, `.on_cancel(..)`, `set_progress`, `set_detail`, `close()` |
| tabs | `widgets::TabBar` | `new(&["Home", "Work"], &State<usize>)`, `.on_close(|i| ..)` (close glyph + middle click), `set_tabs` |
| PanelToolTip | fltk tooltips | styled globally; use `.tooltip()` / `set_tooltip` |
| list | `list::List` | `List::new(&State<usize>, |i| ListRow::new(..).icon(..).detail(..))`, `.bind_selected()`, `.on_activate(f)`, `select(i)` |
| window | `AppWindow` | `new(title, w, h)` (body open), `header().action(glyph, tooltip, f)`, `header().destructive_action(..)`, `body()`, `end()`, `set_mode(Mode::Floating)`, `shortcut()`, `show()`, `snapshot(path)` |

Interior corners are an app preference: `theme::set_corners(Corners::Omarchy | Square | Rounded)`
(survives theme reloads; `Corners::parse("rounded")`). Thumbnails, type
badges and previews should use `theme.badge_radius()`, which stays curved.

Window modes: `Mode::Normal` (default), `Mode::Floating`, `Mode::Fullscreen`
(`Mode::parse("fullscreen")` / `as_str()` for config files).

Glyphs: `widgets::glyph::*` (chevron, refresh, settings, wifi, bluetooth,
volume, headphones, monitor, plus, play, stop, trash, search, folder, file…).
