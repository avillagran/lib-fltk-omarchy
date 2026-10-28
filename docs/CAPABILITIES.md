# Capability matrix vs typical QML apps (0.0.4)

**Implemented** = in the code and covered by `tests/ui.rs`, unit tests or the
real-window scenario. **Partial** = usable with the stated gaps.
**Planned** = intended, not in the code. **Unsupported** = not available.
lib-fltk-omarchy does not claim QML equivalence.

| Area | QML typical | lib-fltk-omarchy 0.0.4 | Status |
|---|---|---|---|
| Omarchy look | omarchy-shell `Ui/` kit | Button, ActionButton, ButtonGroup, TextField, NumberField, ToggleSwitch, Toggle, Slider, Hero, section header, Separator, Row, Dropdown, Menu, ConfirmDialog, notification cards, tooltips — same tokens as `Style.qml`/`Color.qml` plus SearchableDropdown, PopupMenu (header/context menus, clamped and scrolling, dismissal handle), TabBar, ShortcutHelp, OperationToast | Partial: MultiSelect widget, PopupCard anchored to a bar are planned (the KeyboardPanel model is implemented) |
| Theme | — | colors.toml + shell.toml (theme + user override), Hyprland rounding/gaps, `monospace` font via fontconfig, live reload via inotify; strict validation on reload keeps the last valid theme (`theme::last_error`) | Implemented (tested with a fake state dir and in a real window) |
| Layouts | Row/Column/Grid/anchors | fltk-rs `Flex`/`Group`/`Grid` + `layout::{column,row,section,fit,close_fitted}` with Omarchy spacing | Implemented (anchors: use fltk positioning) |
| Reactive state | property bindings | `State<T>`, `map`, `combine`, `watch`/`subscribe`, bound components, two-way inputs | Implemented |
| Controls | Qt Quick Controls | the Omarchy set above plus every fltk-rs widget | Partial: no date/color pickers, no rich text |
| Virtualized lists | ListView/TableView | `List` (count + row closure, selection, keyboard, wheel, thumb) | Partial: single column + detail; tables/trees via fltk-rs `Table`/`Tree` without Omarchy styling yet |
| Keyboard / focus | focus chain, Shortcut | keyboard-first router: Tab order over the whole window, arrows only inside the focused control (`focus::scope` for groups), focus returns after menus/dialogs, every component keyboard-usable, keyboard panels (several Tab stops inside one canvas, cursor and multi-selection by stable IDs, list and grid), focus ring only in keyboard mode, `AppWindow::shortcut`, `Button::shortcut`, `focus::press` for tests ([KEYBOARD.md](KEYBOARD.md)) | Implemented (driven key by key in `tests/ui.rs`); key delivery from a physical keyboard through the compositor not validated |
| Accessibility | AT-SPI | — | Unsupported (FLTK has no AT-SPI bridge) |
| Menus / dialogs | Menu, Dialog | Omarchy Menu (filter, fold), ConfirmDialog, Dropdown popup; fltk-rs `dialog` and native file chooser | Implemented / Partial (file chooser not themed) |
| Drawing | Canvas/Shape | `gfx`: paths (Béziers, arcs, per-corner radii, squircles), anti-aliased fill/stroke with joins and caps, linear/radial/conic gradients, shadows, blur, layer opacity, blend modes, clip by shape, shape hit-testing; plus fltk-rs `draw` and `paint::*` | Implemented (pixel-tested; [SHADERS.md](SHADERS.md)) |
| Animations | Behavior/NumberAnimation | `gfx::View::animate(fps)` (timer, paused while hidden); Omarchy control color transitions are still drawn instantly | Partial (no property/easing animations for controls yet) |
| Images | Image | fltk-rs images (PNG/JPEG/SVG…) | Implemented by fltk-rs |
| Clipboard / DnD | Clipboard, DropArea | fltk-rs `app::copy/paste`, DnD events | Implemented by fltk-rs, no Omarchy helpers |
| DPI | automatic | FLTK per-screen scaling on Wayland; spacing and fonts scale with `[font] base-size` like the shell | Implemented |
| Localization | qsTr | UTF-8 everywhere | Planned |
| Async tasks | WorkerScript | `task::Task` (bounded pool, cooperative cancel, coalesced progress, UI-thread callbacks), `task::post` | Implemented |
| Window modes | Window flags | `Mode::Normal` / `Mode::Floating` (float at map, header drag, corner resize via Hyprland IPC on own windows only) / `Mode::Fullscreen` (FLTK) | Implemented (drag/resize with a real pointer not yet validated; native xdg_toplevel move/resize without Hyprland is planned) |
| Corner preference | — | `Corners::{Omarchy, Square, Rounded}` interior radius, fixed `badge_radius` | Implemented (pixel-tested) |
| Headless rendering | grabToImage | `AppWindow::snapshot(path)` PNG without showing | Implemented |
| Shader effects | ShaderEffect | CPU fragment-style shaders (`gfx::Shader`) with uniforms (time, size, pointer, theme palette), tiled, area-limited, time-budgeted | Implemented in software; GPU not implemented (would be an optional feature, off by default) |
