# Components (0.0.2)

Every component is a classic Omarchy control (the omarchy-shell `Ui/`
kit) drawn with the live theme: colors, fonts, spacing and corner radius
change when you run `omarchy-theme-set`. Each one derefs to an fltk widget,
so it goes in any `Flex`/`Group`; it is created with its Omarchy size and
`flex.fit(&*widget)` keeps that size.

You can drop the Rust handle: `row.fit(&*Toggle::new(..))` keeps following
its `State`, because bindings belong to the fltk widget and end when the
widget is deleted.

Every component works from the keyboard. Tab and Shift+Tab move between
controls, arrows act inside the focused one, and menus give focus back when
they close. The keys of each component are listed below, and the whole
model is in [KEYBOARD.md](KEYBOARD.md).

The pictures are rendered offscreen by `cargo run --example docs_shots --
docs/components` with the Omarchy theme of the machine that ran it (here
Matte Black). The code under each picture is the code that drew it,
without the picture frame (a bare window with a padded column `col`).

Imports used below:

```rust
use fltk::prelude::*;
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::widgets::{glyph, *};
use fltk_omarchy::{App, AppWindow, State};
```

## Buttons

![buttons](components/buttons.png)

```rust
let mut r = layout::row();
r.fit(&*Button::new("Default"));
r.fit(&*Button::new("Bordered").bordered());
r.fit(&*Button::icon(glyph::PLUS, "New").primary().on_click(|| println!("new")));
r.fit(&*Button::icon(glyph::TRASH, "Delete").danger().bordered());
r.fit(&*ActionButton::new(glyph::REFRESH, "Refresh"));       // icon only, tooltip
r.fit(&*ActionButton::destructive(glyph::CLOSE, "Close"));
r.close_fitted();
```

More: `.shortcut("Ctrl+S")`, `.tooltip(..)`, `.left_aligned()`,
`.bind_text(&State<String>)`, `.bind_enabled(&State<bool>)`, `click()`.
**Keys:** Tab to focus, Enter or Space to press; `.shortcut(..)` works from
anywhere in the window.

## ButtonGroup

![button group](components/button-group.png)

```rust
let period = State::new(1usize);              // selected index
let mut r = layout::row();
r.fit(&*ButtonGroup::new(&["Day", "Week", "Month"], &period));
r.close_fitted();
```

**Keys:** one Tab stop (the selected button); Left/Right change the
selection.

## TextField and NumberField

![text fields](components/text-fields.png)

```rust
let query = State::new(String::new());
col.fit(&*TextField::new().placeholder("Search apps…").bind(&query));   // two-way
let name = State::new("Omarchy".to_string());
col.fit(&*TextField::new().bind(&name));
let mut r = layout::row();
r.fit(&*TextField::password().placeholder("Password"));
r.fit(&*NumberField::new(&State::new(42), 0, 100, 1));
r.close_fitted();
```

More: `.on_change(|text| ..)`, `.on_submit(|text| ..)`, `type_text(..)`.
**Keys:** type; Enter submits; Up/Down step the NumberField.

## ToggleSwitch and Toggle

![toggles](components/toggles.png)

```rust
let wifi = State::new(true);
let mut r = layout::row();
r.fit(&*Label::new("Wi-Fi"));
layout::spacer();
r.fit(&*ToggleSwitch::new(&wifi));
r.close_fitted();
col.fit(&*Toggle::new("Night light", "Warmer colors after sunset", &State::new(true)));
col.fit(&*Toggle::new("Do not disturb", "Silence notifications", &State::new(false)));
```

**Keys:** Enter or Space flip it.

## Slider and Progress

![slider and progress](components/slider-progress.png)

```rust
let volume = State::new(0.64);
let mut r = layout::row();
r.fit(&*Label::new("Volume"));
let _slider = Slider::new(&volume, 0.0, 1.0, 0.05);             // stretches
r.fit(&*Label::new("").right().bind(&volume.map(|v| format!("{:.0}%", v * 100.0))));
r.close_fitted();
col.fit(&*Progress::new(&State::new(0.42)));                     // 0..1
```

**Keys:** Left/Right or Up/Down move the Slider by one step.

## Text styles, Hero, Separator

![labels](components/labels.png)

```rust
col.fit(&*Hero::new(glyph::VOLUME_HIGH, "Audio", "Built-in speakers").detail("64%"));
col.fit(&*Separator::new());
col.fit(&*Label::section("Output"));                   // PanelSectionHeader
col.fit(&*Label::styled("Title text", TextStyle::Title));
col.fit(&*Label::new("Body text"));
col.fit(&*Label::new("Dim caption").tone(Tone::Dim));
```

More: `.bold()`, `.center()`, `.right()`, `.bind(&State<String>)`.

## Row

![rows](components/rows.png)

```rust
col.fit(&*Row::new(glyph::MONITOR, "HDMI / DisplayPort").detail("Digital"));
col.fit(&*Row::new(glyph::HEADPHONES, "Built-in speakers").detail("Analog").current(true));
col.fit(&*Row::new(glyph::BLUETOOTH, "WH-1000XM4").detail("Bluetooth").on_click(|| println!("connect")));
```

**Keys:** Tab to focus, Enter or Space to activate.

## TabBar

![tabs](components/tabs.png)

```rust
let tab = State::new(1usize);
col.fit(&*TabBar::new(&["Home", "Downloads", "Work"], &tab).on_close(|i| println!("close {i}")));
```

More: `set_tabs(..)`, `request_close(i)`; a middle click closes a tab.
**Keys:** Ctrl+Tab / Ctrl+PageDown go to the next tab and Ctrl+Shift+Tab /
Ctrl+PageUp to the previous one, from anywhere in the window. When the bar
has focus, Left/Right/Home/End pick a tab and Delete closes it.

## Dropdown

![dropdown](components/dropdown.png)

```rust
use fltk_omarchy::overlay::Dropdown;
let output = State::new(0usize);
col.fit(&*Dropdown::new("Output", &["Speakers", "Headphones", "HDMI"], &output));
```

**Keys:** Enter, Space or Down open it; Up/Down or j/k move; Enter picks;
Esc closes. Focus returns to the trigger.

## SearchableDropdown

![searchable dropdown](components/searchable-dropdown.png)

```rust
use fltk_omarchy::popups::SearchableDropdown;
let place = State::new(None::<usize>);
let places = vec![
    ("Home".to_string(), "~/".to_string()),
    ("Downloads".to_string(), "~/Downloads".to_string()),
    ("Work".to_string(), "~/Work".to_string()),
    ("Pictures".to_string(), "~/Pictures".to_string()),
];
col.fit(&*SearchableDropdown::new("Go to…", places, &place));
```

**Keys:** Enter or Down open it; type to filter by label or description;
Up/Down move; Enter picks; Esc closes.

## List (virtual)

![list](components/list.png)

```rust
use fltk_omarchy::list::{List, ListRow};
let count = State::new(100_000usize);
let selected = State::new(Some(2usize));
let list = List::new(&count, |i| ListRow::new(&format!("Item {i}")).icon(glyph::FILE).detail(&format!("{} KB", i * 3)))
    .bind_selected(&selected)
    .on_activate(|i| println!("open {i}"));
```

Only the visible rows are built: 1 000 000 rows cost the same per frame as
20. **Keys:** Up/Down or j/k move the cursor row; PageUp/PageDown, Home/End;
Enter opens (`on_activate`).

## PopupMenu (Preferences / Actions / context menus)

![popup menu](components/popup-menu.png)

```rust
use fltk_omarchy::popups::{PopupItem, PopupMenu};
PopupMenu::open(&*actions_button, vec![
    PopupItem::new(glyph::FOLDER, "Open").shortcut("Enter"),
    PopupItem::new(glyph::SEARCH, "Preview").shortcut("Space").divider(),
    PopupItem::new(glyph::FILE, "Copy").shortcut("Ctrl+C"),
    PopupItem::new(glyph::FILE, "Show hidden files").checked(true),
    PopupItem::new(glyph::TRASH, "Move to trash").shortcut("Delete").disabled(),
], |index| println!("picked {index}"));
```

`PopupMenu::open_at(&*win, x, y, items, f)` opens it at a point (context
menu). **Keys:** Up/Down or j/k move and skip disabled entries; Enter picks;
Esc closes; Tab stays in the menu. Focus returns to the button that opened
it.

## Menu (Omarchy menu, type to filter)

![menu](components/menu.png)

```rust
use fltk_omarchy::overlay::{Menu, MenuItem};
Menu::open(&*win, "Go", vec![
    MenuItem::new(glyph::SEARCH, "Apps").detail("launch an app"),
    MenuItem::new(glyph::MONITOR, "Screenshot").detail("capture the screen"),
    MenuItem::new(glyph::MONITOR, "Screen record").detail("record the screen"),
    MenuItem::new(glyph::SETTINGS, "Setup").submenu(),
], |index| println!("picked {index}"));
// picture: after typing "sc"
```

**Keys:** type to filter; Up/Down, PageUp/PageDown; Enter or Right pick;
Backspace / Ctrl+Backspace / Ctrl+U edit the filter; Esc clears the filter,
then closes.

## ConfirmDialog

![confirm dialog](components/confirm-dialog.png)

```rust
use fltk_omarchy::overlay::ConfirmDialog;
ConfirmDialog::open(&*win, "Move 3 files to the trash?", "Move", true, |ok| println!("confirmed: {ok}"));
```

**Keys:** Left/Right/Tab switch buttons; Enter activates; Esc cancels.

## Notification

![notification](components/notification.png)

```rust
use fltk_omarchy::overlay::{Notification, Urgency};
Notification::show(&*win, glyph::BELL, "Copied 3 files", "to ~/Work/reports", Urgency::Normal);
Notification::show(&*win, glyph::WIFI, "Connected", "", Urgency::Low);
```

Low hides after 5 s, Normal after 8 s, Critical stays until dismissed.
**Keys:** Tab reaches a notification; Enter, Space, Delete or Esc dismiss it.

## OperationToast

![operation toast](components/operation-toast.png)

```rust
use fltk_omarchy::popups::OperationToast;
let mut op = OperationToast::show(&*win, "Copying 3 files").unwrap()
    .on_pause(|paused| println!("paused: {paused}"))
    .on_cancel(|| println!("cancel"));
op.set_progress(0.42);                       // from a Task's on_progress
op.set_detail("report.pdf · 12 of 30 MB");
// op.close() when the task ends
```

**Keys:** Tab reaches it (or call `op.focus()` from a window shortcut);
Left/Right choose Pause or Cancel; Enter or Space press; Esc gives focus
back without cancelling.

## ShortcutHelp (F1)

![shortcut help](components/shortcut-help.png)

```rust
use fltk_omarchy::popups::ShortcutHelp;
let row = |k: &str, d: &str| (k.to_string(), d.to_string());
let help = move || vec![
    ("Move".into(), vec![row("Tab", "Next control"), row("Shift+Tab", "Previous control"), row("Ctrl+Tab", "Next tab")]),
    ("Files".into(), vec![row("Enter", "Open"), row("F2", "Rename"), row("Delete", "Move to trash")]),
    ("Window".into(), vec![row("Ctrl+,", "Preferences"), row("F10", "Actions"), row("F1", "This help")]),
];
let w = win.clone();
win.shortcut("F1", move || ShortcutHelp::open(&*w, "Keyboard shortcuts", help()));
```

**Keys:** Up/Down/PageUp/PageDown scroll; Esc or F1 close.

## AppWindow

![app window](components/app-window.png)

```rust
let app = App::new("dev.example.notes");
let mut win = AppWindow::new("Notes", 600, 260);   // the body column is open
win.header().action(glyph::SETTINGS, "Preferences", || {});
win.header().action("\u{f0241}", "Actions", || {});
win.header().destructive_action(glyph::CLOSE, "Close", || fltk::app::quit());
let mut body = win.body();
body.fit(&*TextField::new().placeholder("Title"));
body.fit(&*Toggle::new("Pinned", "Keep at the top of the list", &State::new(true)));
let mut r = layout::row();
layout::spacer();
r.fit(&*Button::new("Cancel").bordered());
r.fit(&*Button::new("Save").primary());
r.close_fitted();
win.end();
win.show();
app.run();
```

More: `set_mode(Mode::Normal | Floating | Fullscreen)`,
`shortcut("Ctrl+Q", f)`, `set_min_size`, `snapshot(path)`. Header actions
are buttons: Tab reaches them.

## Keyboard focus

![keyboard: button group](components/keyboard-1-group.png)
![keyboard: tabs](components/keyboard-2-tabs.png)
![keyboard: list](components/keyboard-3-list.png)

The same window driven only by keys: Tab, Right (ButtonGroup → Week); Tab,
Ctrl+Tab (TabBar → Downloads); Tab, Down, Down (List → Item 1). The focused
control gets the accent ring. See [KEYBOARD.md](KEYBOARD.md).

## Reference

| Omarchy (`shell/Ui`) | Rust | Main API |
|---|---|---|
| Button | `widgets::Button` | `new`, `icon`, `.primary()`, `.bordered()`, `.danger()`, `.left_aligned()`, `.tooltip()`, `.shortcut()`, `.on_click()`, `.bind_text()`, `.bind_enabled()`, `set_selected`, `click()` |
| PanelActionButton | `widgets::ActionButton` | `new(glyph, tooltip)`, `destructive(..)`, `.on_click()` |
| ButtonGroup | `widgets::ButtonGroup` | `new(&labels, &State<usize>)`, `selected()`, `button(i)` |
| TextField | `widgets::TextField` | `new()`, `password()`, `.placeholder()`, `.bind()`, `.on_change()`, `.on_submit()`, `type_text()`, `input()` |
| NumberField | `widgets::NumberField` | `new(&State<i64>, from, to, step)` |
| ToggleSwitch / Toggle | `widgets::ToggleSwitch`, `widgets::Toggle` | `new(&State<bool>)`, `new(title, description, &State<bool>)` |
| PanelSlider / progress | `widgets::Slider`, `widgets::Progress` | `new(&State<f64>, min, max, step)`, `new(&State<f64>)` |
| text tokens | `widgets::Label` | `new`, `styled(text, TextStyle::..)`, `section(..)`, `.bold()`, `.tone()`, `.center()`, `.right()`, `.bind()` |
| PanelHero / PanelSeparator | `widgets::Hero`, `widgets::Separator` | `new(glyph, title, meta).detail(..)`, `new()` |
| CursorSurface row | `widgets::Row` | `new(glyph, label)`, `.detail()`, `.current()`, `.on_click()` |
| tabs | `widgets::TabBar` | `new(&labels, &State<usize>)`, `.on_close()`, `set_tabs`, `request_close` |
| Dropdown | `overlay::Dropdown` | `new(label, &options, &State<usize>)`, `open()`, `set_options` |
| SearchableDropdown | `popups::SearchableDropdown` | `new(placeholder, vec![(label, description)], &State<Option<usize>>)`, `open()`, `filter()` |
| list | `list::List` | `new(&State<usize>, \|i\| ListRow)`, `.bind_selected()`, `.on_select()`, `.on_activate()`, `select(i)` |
| header / context menus | `popups::PopupMenu` | `open(&anchor, items, f)`, `open_at(&win, x, y, items, f)` |
| menu plugin | `overlay::Menu` | `open(&win, title, items, f)` |
| ConfirmDialog | `overlay::ConfirmDialog` | `open(&win, message, confirm, destructive, f)` |
| notifications | `overlay::Notification` | `show(&win, glyph, summary, body, Urgency::..)`, `count()` |
| operations tray | `popups::OperationToast` | `show(&win, title)`, `.on_pause()`, `.on_cancel()`, `set_progress`, `set_detail`, `focus()`, `close()` |
| F1 help | `popups::ShortcutHelp` | `open(&win, title, sections)` |
| PanelToolTip | fltk tooltips | styled globally; `.tooltip()` / `set_tooltip` |
| window | `AppWindow` | `new`, `header().action(..)`, `body()`, `end()`, `show()`, `set_mode()`, `shortcut()`, `snapshot()` |

Interior corners are an app preference: `theme::set_corners(Corners::Omarchy |
Square | Rounded)`; it survives theme reloads. Thumbnails, type badges and
previews use `theme.badge_radius()`, which stays curved. The focus ring is
another preference: `theme::set_focus_ring(FocusRing::Accent | Theme)`.

Glyphs: `widgets::glyph::*` (chevron, refresh, settings, wifi, bluetooth,
volume, headphones, monitor, plus, play, stop, trash, search, folder, file…).
