# Qt instruction → lib-fltk-omarchy

Read this next to Omarchy's `omarchy-app` skill when its Qt advice needs a
Rust equivalent. `use fltk_omarchy::*` names below; `win` is an
`AppWindow`.

| Omarchy's Qt app | Rust + lib-fltk-omarchy |
|---|---|
| `qmake6` + `make` → `build/<name>` | `cargo build --release` → `target/release/<name>` (`bin/build`) |
| Qt Test offscreen (`QT_QPA_PLATFORM=offscreen`) | `cargo test`; `tests/ui.rs` with `harness = false` under `xvfb-run` (`bin/test`) |
| `QGuiApplication` + `setDesktopFileName("<name>")` | `App::new("<name>")` (Wayland app id = `.desktop` name), `app.run()` |
| `ApplicationWindow` in `Main.qml` | `AppWindow::new("<Name>", w, h)`, widgets into `win.body()`, `win.end()`, `win.show()` |
| `QObject` backend with `Q_PROPERTY … NOTIFY` | plain struct in `backend.rs`, shown through `State<T>` + `bind` |
| `Q_INVOKABLE` actions | methods called from `on_click` / `win.shortcut` closures |
| `Shortcut { context: Qt.ApplicationShortcut }` | `win.shortcut("Mod+S", ..)` (`Mod` = Ctrl, Cmd on macOS) |
| `?` overlay, `Q` quits and asks | `win.help_overlay(title, sections)`, `win.quit_key(\|\| unsaved)` |
| Material dark + theme accent | automatic: `App::new` applies and follows the Omarchy theme; `theme::with(\|t\| t.palette.accent)`, `Tone::Accent` |
| `Theme` class watching `colors.toml` | built in (inotify; polling on macOS/Windows) |
| `theme.radius` from `hyprctl getoption decoration:rounding` | built in: `t.radius`, `t.badge_radius()`, `theme::set_corners(Corners::Omarchy)` |
| `qsTr("Save")`, `.arg()` | `tr!("Save")`, `tr!("Delete {name}?", name = n)` |
| `qsTr("%n clips", "", n)` | `trn!("{count} clips", n)` |
| `src/i18n/<code>.json` + `Translator` | same files (`{count}` placeholders), `i18n::load_omarchy_catalog(code, include_str!(..))`; Setup > Language followed live |
| re-evaluate bindings on `translator.language` | `i18n::text`, `i18n::map(&state, ..)`, `bind_tooltip`, `win.bind_title`, `i18n::bind(&w, ..)` |
| `value.toLocaleString(Qt.locale(), 'f', 1)` | `i18n::format_number(value, 1)` |
| `LayoutMirroring` root | automatic for ar/he/fa/ur; `layout::set_direction(&w, Some(Direction::Ltr))` to keep timelines LTR |
| `HoverHandler` to place under the pointer | `fltk::app::event_x()`/`event_y()` relative to the widget; `PopupMenu::open_at` |
| `QProcess` + `ffmpeg`/`ffprobe` | `std::process::Command` inside a `Task`; playback: feature `playback`, `media::Video`/`media::Audio` |
| xdg-desktop-portal FileChooser (Omacut's `PortalFilePicker`) | `dialog::Request::open/save(..).filter(..).run_async(..)`; fake it in tests with `dialog::set_picker` |
| `QSaveFile` (atomic write) | write `.<name>.tmp` beside the target, `sync_all`, `std::fs::rename` |
| confirmation dialog | `overlay::ConfirmDialog::open(&*win, msg, confirm, destructive, \|yes\| ..)` |
| progress of long work | `Task` + `popups::OperationToast` |
| `QStandardPaths` | `platform::dirs::{app_data, app_config, app_cache}(NAME)` |
| `hype check/render --json`, `hype skill install` | `cli.rs`: `check`, `render` (`ui::build` + `win.snapshot`), `skill install` (`~/.agents/skills/<name>`, linked into `~/.claude/skills`) |
| offscreen harness + `grabWindow()` | `testkit::run_script(&*win, json)` (press, click, expect, snapshot, tree) or `OMARCHY_FLTK_SCRIPT=steps.json <name>` |
| `QT_FORCE_STDERR_LOGGING=1` | toolkit warnings already go to stderr (`fltk-omarchy: …`) |
