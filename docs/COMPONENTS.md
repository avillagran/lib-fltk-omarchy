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
| PanelToolTip | fltk tooltips | styled globally; use `.tooltip()` / `set_tooltip` |
| list | `list::List` | `List::new(&State<usize>, |i| ListRow::new(..).icon(..).detail(..))`, `.bind_selected()`, `.on_activate(f)`, `select(i)` |
| window | `AppWindow` | `new(title, w, h)` (body open), `header().action(glyph, tooltip, f)`, `header().destructive_action(..)`, `body()`, `end()`, `set_mode(Mode::Floating)`, `shortcut()`, `show()`, `snapshot(path)` |

Glyphs: `widgets::glyph::*` (chevron, refresh, settings, wifi, bluetooth,
volume, headphones, monitor, plus, play, stop, trash, search, folder, file…).
