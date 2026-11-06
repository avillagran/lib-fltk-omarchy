# Platforms

lib-fltk-omarchy builds on **Linux (Wayland and X11), macOS and Windows**.
Everything that depends on the operating system lives in one module,
`src/platform/` (`fltk_omarchy::platform`), with a working fallback on
each OS. The rest of the crate is portable and has no
`#[cfg(target_os = ..)]`. The public API is the same everywhere; on Linux
it behaves exactly as before the platform layer.

## What works where

| Feature | Linux Wayland | Linux X11 / XWayland | macOS | Windows |
|---|---|---|---|---|
| Components, layouts, State, tasks, lists, gfx, snapshots | yes | yes | yes | yes |
| Keyboard routing, focus, shortcuts | yes | yes | yes, primary modifier is Cmd | yes |
| Omarchy theme files (`colors.toml`, `shell.toml`) | `~/.local/state/omarchy/current/theme` | same | `~/Library/Application Support/omarchy/current/theme` if present | `%LOCALAPPDATA%\omarchy\current\theme` if present |
| Built-in theme when no files exist | Omarchy dark | Omarchy dark | dark or light, following the system appearance | dark or light, following "app mode" |
| Theme directory from code/env | `theme::set_theme_dir`, `OMARCHY_FLTK_THEME_DIR` | same | same | same |
| Live theme reload | inotify, blocking (no CPU while idle) | same | 1 s mtime poll + appearance | 1 s mtime poll + appearance |
| Hyprland rounding/gaps, IPC | when Hyprland runs | when Hyprland runs | never | never |
| Default font | fontconfig `monospace` (follows `omarchy-font-set`) | same | Menlo | Consolas |
| `Mode::Floating` header drag / corner resize | compositor (`xdg_toplevel.move/resize` via libdecor) | Hyprland IPC, else FLTK moves/resizes the window | FLTK moves/resizes the window | FLTK moves/resizes the window |
| `Mode::Floating` at map | Hyprland IPC | Hyprland IPC | the OS window is already floating | same |
| Glassy windows (`glass`) | translucent surfaces, Hyprland blurs | opaque | opaque | opaque |
| `native::pixels_per_unit` | FLTK scale × Wayland output scale | FLTK scale | FLTK's value (Retina included) | FLTK's value |
| Config/state/cache dirs (`platform::dirs`) | XDG | XDG | `~/Library/Application Support`, `~/Library/Caches` | `%APPDATA%`, `%LOCALAPPDATA%` |
| GPU shaders (`gpu` feature) | yes | yes | builds (OpenGL through FLTK) | builds (OpenGL through FLTK) |
| Single instance + app IPC (`instance`) | Unix socket in `$XDG_RUNTIME_DIR`, `flock`, `SO_PEERCRED` | same | Unix socket in `$TMPDIR`, `flock`, `getpeereid` | named pipe, first-instance flag, owner check |
| Raise a window from the background (`AppWindow::raise`) | Hyprland `focuswindow` on our own window; other compositors: none (FLTK has no xdg-activation) | FLTK `_NET_ACTIVE_WINDOW` (EWMH) | `NSApp activateIgnoringOtherApps:` | `SetForegroundWindow` (the caller grants `AllowSetForegroundWindow`) |
| Animations on/off (`platform::animations`, `anim`) | Hyprland `animations:enabled` (live: event socket), else GNOME `enable-animations` (`gsettings`) | same | `NSWorkspace.accessibilityDisplayShouldReduceMotion` | `SPI_GETCLIENTAREAANIMATION` |
| File dialogs (`dialog`) | xdg-desktop-portal FileChooser over D-Bus, else FLTK's native chooser (GTK, zenity, kdialog or FLTK's own) | same; the portal gets the X11 parent window | FLTK's native chooser (`NSOpenPanel`/`NSSavePanel`) | FLTK's native chooser (`IFileDialog`) |
| External frames (`frame`) | `memfd` + pipes (inherited), POSIX shm + FIFOs (named), mapped file + 8 ms polling | same | `shm_open` + pipes or FIFOs, mapped file | named file mapping + events, mapped file |

"FLTK moves/resizes the window" is the portable fallback in `AppWindow`:
the header drag sets the window position from the pointer's root
coordinates, the bottom corners resize it (not below the window's minimum
size). The window manager's own title bar works as usual on macOS and
Windows.

## The platform module

| Item | What it gives |
|---|---|
| `platform::OS`, `Os` | the OS of this build (`Linux`, `MacOs`, `Windows`, `Other`) |
| `platform::wayland()` | FLTK runs on its native Wayland backend |
| `platform::animations()` | the OS motion setting (`Some(false)`: reduce motion); read by `anim` on its watcher thread (docs/ANIMATION.md) |
| `platform::prefers_dark()` | the system appearance: `Some(true)` dark, `Some(false)` light, `None` unknown (Linux: the Omarchy theme decides) |
| `platform::process_cpu_time()` | this process's CPU time (benchmarks) |
| `platform::thread_cpu_time()` | the calling thread's CPU time (`testkit` frame log, blocked-UI warning) |
| `platform::dirs::{home, config, data, state, cache, runtime, app_config, app_data, app_state, app_cache}` | per-OS user directories, no dependencies (`runtime`: `$XDG_RUNTIME_DIR`, else the per-user temp dir; `OMARCHY_FLTK_RUNTIME_DIR` overrides) |
| `platform::fonts::{default_family, faces}` | default UI family and the FLTK face names (regular, bold) for a family |
| `keys::PRIMARY`, `keys::primary_held()`, `keys::label(spec)` | the primary modifier (Cmd on macOS, Ctrl elsewhere) and a spec shown the platform's way |

Backends: `src/platform/linux.rs` (FLTK Wayland internals, Hyprland Unix
socket, inotify), `macos.rs` (appearance through CoreFoundation),
`windows.rs` (appearance from the registry, CPU time from
`GetProcessTimes`), `generic.rs` (the fallbacks shared by every non-Linux
OS), `poll.rs` (the portable theme watcher), `other.rs` (other Unixes).
Existing modules keep their API and delegate: `native`, `glass`,
`hyprland`, `app::ThemeWatcher`, `theme`.

### Shortcuts

Write `Mod` for the primary modifier (`Primary` and `CmdOrCtrl` also
work): `win.shortcut("Mod+,", ..)` is Preferences on every OS (Ctrl+, on
Linux and Windows, Cmd+, on macOS). `Ctrl` always means the Control key,
so existing specs keep working unchanged. Popup menu hints
(`PopupItem::shortcut`) and the F1 `ShortcutHelp` show specs through
`keys::label`: on Linux and Windows a spec is shown exactly as written,
with `Mod` as `Ctrl`; on macOS with the system symbols (`⌘,`, `⇧⌘P`,
`⌃Tab`).

Keyboard panels (`panel::KeyboardPanel`, `table::Table`, `table::Tree`)
use the primary modifier for toggle (`Mod+Space`), select all (`Mod+A`),
cursor-only moves and click-to-toggle. TabBar keeps Ctrl+Tab on every OS
(macOS uses Ctrl+Tab for tabs too).

### Single instance and app IPC

`fltk_omarchy::instance` makes an app single-instance and lets a second
launch, a CLI (`myapp call FN ARG`, like o3's `ot3 call`) or a bar widget
call functions of the running app. The transport is in
`src/platform/instance/`; the protocol is portable.

```rust
let server = instance::acquire_or_forward("dev.you.app"); // a 2nd launch exits here
let app = App::new("dev.you.app");
app.keep_running(true);            // start hidden / hide to the background
let mut win = AppWindow::new("You", 640, 480);
win.end();
server.manage(&win);               // show, raise, hide, toggle, window, is_active
server.handle("state", |_| state_json());
app.run();
// elsewhere: instance::call("dev.you.app", "toggle", "") -> Result<String, Error>
```

- `instance::acquire(id)` → `Instance::Primary(Server)` or
  `Instance::Secondary(Client)`. Two launches racing get exactly one
  primary; a launch waits up to 2 s for a primary that holds the lock but
  is still starting. It never fails: when IPC cannot be used (no runtime
  dir, an endpoint of another user, an unsupported OS, a primary that
  holds the lock and never answers) it returns a primary without IPC and
  says so on stderr (`Server::unavailable_reason`).
- Requests are a function name, string arguments and the caller's working
  directory; replies are a string (JSON by convention) or an error.
  Frames: 4-byte big-endian length + UTF-8 JSON; requests ≤ 1 MiB,
  replies ≤ 8 MiB, 2 s per read/write, 16 connections at once (the rest
  get "busy"), all configurable with `instance::Options`.
- The server thread hands each request to the UI thread with
  `task::post` and waits up to 5 s for the handler's reply. Handlers are
  registered on the UI thread (`handle`, `handle_request`, `on_activate`).
- Built-ins (an app handler with the same name replaces one): `ping`,
  `functions`, `activate` (second launch: `on_activate`, then raise),
  `quit`; with `manage(&win)` also `show`, `raise`, `hide`, `toggle`,
  `window` and `is_active`.
- `App::keep_running(true)`: `App::run` keeps going with no window shown
  until `App::quit` (thread-safe). `AppWindow::raise()`, `is_active()`
  (Hyprland's active window; else FLTK's focus) and `toggle()` (hide when
  shown and active, raise otherwise).

| | Linux / BSDs | macOS | Windows |
|---|---|---|---|
| Endpoint | `<runtime>/<app id>/instance.sock` | same (`$TMPDIR` is per user) | `\\.\pipe\fltk-omarchy-<app id>-<hash of the runtime dir>` |
| Lock / race | `flock` on `instance.lock`, released by the kernel when the primary dies; a stale socket is replaced only while holding the lock | same | `FILE_FLAG_FIRST_PIPE_INSTANCE`; the pipe disappears with the process |
| Same user only | socket 0600 in a 0700 dir checked with `lstat` (no symlinks followed, owned by us); a shared runtime dir (`/tmp`) gets a private `fltk-omarchy-<uid>` dir first; peer uid on both ends (`SO_PEERCRED` on Linux, `getpeereid` on BSDs) | same with `getpeereid` | default pipe DACL (only the creator writes), remote clients rejected, the client checks the pipe owner's SID before sending |
| Raise from the background | Hyprland: `focuswindow` on our own window (found by pid + title); X11: FLTK's `_NET_ACTIVE_WINDOW`; other Wayland compositors: none (FLTK has no xdg-activation), a newly mapped window usually gets the focus | `NSApp activateIgnoringOtherApps:` + FLTK | `SetForegroundWindow` (the client calls `AllowSetForegroundWindow` for the primary's pid) |

`OMARCHY_FLTK_RUNTIME_DIR` (or `Options::runtime_dir`) moves the
endpoint, for tests and sandboxes. Requests wait while the UI thread runs a
nested event loop of its own (they are drained by `App::run`), up to the
handler timeout.

### External frames

`fltk_omarchy::frame` (`FrameView`, `Producer`, `Consumer`) shares frames
and input with another process. The OS part is in `src/platform/frame/`:
it maps memory and moves wake-ups; the layout (header, three slots, two
rings) is portable. Guide and wire format: [FRAMES.md](FRAMES.md).

| | Linux | macOS, BSDs | Windows | Everywhere |
|---|---|---|---|---|
| `Transport::Inherit` (default) | `memfd_create`, pipes, descriptors inherited by the child (close-on-exec cleared only in that child) | `shm_open` + `shm_unlink`, pipes | falls back to `Named` | falls back to `Named`, then `File` |
| `Transport::Named` | `shm_open("/ofrm-<id>")`, FIFOs in `<runtime>/fltk-omarchy-frames-<uid>` (0700, `lstat` checked, owner checked, `O_NOFOLLOW`) | same (names fit macOS's 31-byte limit) | `Local\fltk-omarchy-<id>` mapping and auto-reset events (default DACL: this user) | |
| `Transport::File` | | | | 0600 file in the private dir, mapped; the waiting side polls every 8 ms |
| Producer gone | pipe hang-up or `closed` bit | same | `closed` bit | `closed` bit |

The creator removes only its own names (shm name, two FIFOs, frame file).
Frame files (`show_file`, `watch_file`: PNG with `images`, BMP always)
work on every OS without shared memory. Linux is run-tested (unit tests,
a second process per transport in `tests/frame.rs`, `tests/frame_ui`,
`script/frame-bench` on X11 and on Hyprland); macOS and Windows are
type-checked.

### File dialogs

`fltk_omarchy::dialog` opens and saves files the way Omarchy's own apps
do: through xdg-desktop-portal (`org.freedesktop.portal.FileChooser`), so
the desktop's chooser opens and sandboxed apps receive their files. The
D-Bus side is a small built-in client in `src/platform/portal/` (wire
format, SASL `EXTERNAL`, method calls, signals; no dependency). A
one-shot `gdbus call` cannot be used: the portal closes a request when its
caller leaves the bus, so the connection must stay open until the
`Response` signal.

| | Linux / BSDs | macOS | Windows |
|---|---|---|---|
| First choice | portal: `OpenFile`/`SaveFile` with a predicted `handle_token` (the `Response` match is added before the call), `modal`, `multiple`, `directory`, `filters`, `current_filter`, `current_name`, `current_folder`, `accept_label` | FLTK native | FLTK native |
| Fallback | FLTK native when there is no session bus, no portal service, no FileChooser backend, or the portal ends the request with an error | — | — |
| Parent window | `x11:<XID>` on X11; none on Wayland (FLTK has no xdg-foreign export) | — | — |
| While open | `run`: worker thread + nested `Fl::wait` loop (redraws, timers, `task::post` keep going); `run_async`: worker thread, callback through `task::post` | the OS panel's modal loop | same |

The portal service quitting during a dialog ends the request
(`NameOwnerChanged`) instead of waiting forever. `OMARCHY_FLTK_FILE_DIALOG`
= `portal` | `native` | `none` changes the order; scripted testkit runs
never open dialogs; `dialog::set_picker` replaces every chooser in tests.
Verified: unit tests against a fake portal on a private bus socket (open,
save, cancel, old portals' own handle path, missing service, portal
quitting, no bus), and a read of the real portal's `version` property
over the session bus (`cargo test -- --ignored real_bus`). No test opens a
real dialog.

### Theme

On macOS and Windows there are no Omarchy files, so the built-in theme is
used: `theme::FALLBACK_COLORS` (Omarchy's default dark, Tokyo Night) or
`theme::FALLBACK_LIGHT_COLORS` (Tokyo Night Day) when the system asks for
light windows. `OMARCHY_FLTK_APPEARANCE=light|dark` forces one. An
Omarchy theme directory (a copy of `~/.local/share/omarchy/themes/<name>`)
can be used on any OS with `theme::set_theme_dir(Some(dir))` or
`OMARCHY_FLTK_THEME_DIR`, and is followed live. `OMARCHY_FLTK_STATE_DIR`,
`OMARCHY_FLTK_USER_SHELL` and `OMARCHY_FLTK_FONT` work everywhere.

## Building

The fltk dependency is declared per target in `Cargo.toml`: `use-wayland`
(FLTK's hybrid Wayland + X11 backend) is enabled only on Linux; macOS and
Windows use FLTK's Cocoa and WinAPI backends. Cargo merges the features of
the same dependency, so there is exactly one fltk-sys build per target,
and the `gpu` feature adds `enable-glwindow` on every OS.

| OS | Needs |
|---|---|
| Linux | cmake, a C++ compiler, and the X11, Wayland, xkbcommon, pango/cairo, OpenGL development files (see `.github/workflows/ci.yml` for the Ubuntu package list) |
| macOS | Xcode command line tools and cmake |
| Windows | Visual Studio (MSVC) build tools and cmake |

A dependent crate repeats the fltk pin. To keep the same features on every
OS:

```toml
[dependencies]
fltk = { version = "=1.5.23", features = ["no-images"] }
lib-fltk-omarchy = { git = "https://github.com/avillagran/lib-fltk-omarchy", tag = "v0.0.10" }

[target.'cfg(target_os = "linux")'.dependencies]
fltk = { version = "=1.5.23", features = ["use-wayland", "no-images"] }
```

The crate's Rust code can be type-checked for another OS from Linux
without building FLTK: fltk-sys skips its C++ build when `CARGO_CHECK` is
set.

```sh
rustup target add x86_64-pc-windows-msvc aarch64-apple-darwin
CARGO_CHECK=1 cargo check --target x86_64-pc-windows-msvc --all-targets
CARGO_CHECK=1 cargo check --target aarch64-apple-darwin --all-targets --features gpu
```

## Tests per OS

- `cargo test` runs unit, doc and UI tests. `tests/ui.rs` needs a display
  on Linux (Wayland or X11, e.g. `xvfb-run`) and is skipped without one.
  Its pixel expectations hold on every OS; run locally on macOS and
  Windows, failures are printed but not fatal unless
  `OMARCHY_FLTK_UI_STRICT=1` (CI sets it).
- `script/x11-check` runs `examples/platform_check` in an isolated Xvfb
  display with FLTK's X11 backend and drags its header and corner with
  XTest (only on that Xvfb display): the window must follow through the
  FLTK fallback.
- `tests/instance.rs` starts copies of the test binary as second launches
  against a primary in a private runtime dir (no display needed).
  `examples/instance --self-test` (with a private
  `OMARCHY_FLTK_RUNTIME_DIR`, under `xvfb-run`) starts hidden, toggles
  itself from child processes, forwards a launch and quits through IPC.
- `examples/platform_check` prints what the platform layer detected (OS,
  Wayland, Hyprland, appearance, font, dirs, primary modifier) on any OS.
- `script/real-window`, `examples/glass_check` and the bench scripts are
  Linux + Hyprland tools.

## CI (GitHub Actions, 2026-11-04)

`.github/workflows/ci.yml` passes on every job: Linux X11 (Xvfb), Linux
Wayland (headless sway: FLTK 1.5.23 crashed in its `output_done` callback
under weston 14 `--backend=headless`), macOS (arm64), Windows (MSVC) and the
cross-target type check. Unit, doc and every UI check pass on all of them,
strict on macOS and Windows, with and without `gpu` and `a11y`. The first
real runs found: FLTK's C++ event statics need the MSVC name mangling on
Windows (`platform::fl_abi`), tests must press `Mod+…` (Cmd on macOS),
Windows' GDI blit is ±1 per channel, Menlo/Consolas bold adds less ink.

## What was verified (2026-11-04, on main a967007)

On Linux (Omarchy, Hyprland 0.56.2):

| Check | Result |
|---|---|
| `cargo test` with a Wayland display | 41 unit, 31 UI, 9 doc: pass |
| `cargo test` under Xvfb (X11 backend) | 41 unit, 31 UI, 9 doc: pass |
| `cargo test --features gpu` (Wayland) | 41 unit, 31 UI, 10 doc: pass |
| `script/real-window` (real Wayland window, hidden workspace) | pass: Wayland backend, input, task cancel, live theme reload through inotify, Hyprland floating, 0.0012 s CPU over 3 s idle |
| `examples/glass_check` with Glassy on (real Wayland window) | pass: surfaces alpha 166 (0.65), text alpha 255, opaque region restored |
| `examples/popup_damage` under Xvfb (X11) | pass |
| `script/x11-check` (X11, no Wayland, no Hyprland) | pass: header drag moved the window by (150, 80); corner drag resized 420×300 → 480×340 |
| `CARGO_CHECK=1 cargo check/clippy --all-targets` (with and without `gpu`) for `x86_64-pc-windows-msvc`, `x86_64-pc-windows-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin` | pass, no warnings |
| `tests/ui.rs` without any display | skipped with a message, exit 0 |
| `examples/docs_shots`, main vs this change, same machine | 23 of 23 pictures byte-identical |
| Files (`~/Work/files` copy with this crate as its vendored toolkit) | `cargo check --workspace --all-targets` pass; `cargo test --workspace` 1629 passed, 0 failed |
| clippy, Linux, `--all-targets --features gpu` | same warnings per file as main; none in new code |
| `examples/bench_minimal` release size | 1 275 208 → 1 280 968 bytes (+5 760, +0.45 %) |

Not verified locally: building FLTK's C++ and linking for macOS and
Windows (no Apple SDK or MinGW/MSVC toolchain on the Linux machine), and
running anything there. The CoreFoundation, registry and
`GetProcessTimes` calls, the macOS/Windows font names and the FLTK
move/resize fallback on those OSes are type-checked only; the
`.github/workflows/ci.yml` macOS and Windows jobs build, test and run
`platform_check` there.
