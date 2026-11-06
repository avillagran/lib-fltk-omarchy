---
name: omarchy-fltk-app
description: >
  Build a new desktop app for Omarchy in Rust with lib-fltk-omarchy
  (fltk-rs + Omarchy's look), following the conventions of Omarchy's own
  apps (Hype, Monologue, Omacut): one window, one job, keyboard first, the
  theme and language followed live, file dialogs through the portal, and an
  Arch package so it shows up in the app launcher. Use when asked to make,
  build, or scaffold a Rust GUI app, tool, or utility for the desktop, or
  an app with lib-fltk-omarchy / fltk_omarchy / fltk-rs. Triggers: new Rust
  app, fltk, fltk-rs, fltk_omarchy, lib-fltk-omarchy, Rust desktop app,
  cargo GUI, PKGBUILD for my Rust app.
---

# Building an Omarchy App in Rust

The same apps Omarchy's `omarchy-app` skill describes (dead simple on
purpose: one window, one job, driven from the keyboard, following the
desktop, installed like any other package), written in Rust on
lib-fltk-omarchy instead of C++ and Qt Quick. The toolkit already does the
Omarchy parts; this skill says which piece does what. `reference.md` maps
each Qt instruction of the original skill to the toolkit API.

- Template: `templates/omarchy-app/` in lib-fltk-omarchy, filled in by
  `cargo run --example new_app -- <name> [dir]` (`--path` depends on a
  local checkout instead of a git revision; existing files are never
  overwritten).
- Toolkit rules: `AGENTS.md`; guides in `docs/` (COMPONENTS, I18N,
  KEYBOARD, TESTKIT, PLATFORMS, MEDIA).
- Behavior reference: Omarchy's published apps (Omacut, Monologue, Hype,
  Signed) and its own skill, `/usr/share/omarchy/default/agents/skills/omarchy-app/`.

## Before writing code

Ask what the app should do if it isn't clear, and settle the one job it
does. Pick a short lowercase name (`tally`, `pomo`): it is the binary, the
package, the `.desktop` file, the icon, and the Wayland app id
(`App::new("<name>")`). Create the project in `~/Work/<name>` unless the
user says otherwise; the generator runs `git init`.

Building needs Rust, cmake and a C++ compiler (FLTK is compiled once per
target directory): `omarchy-pkg-add rust cmake base-devel`. Media work
needs `ffmpeg`, the tests `xorg-server-xvfb`.

## Shape

```text
Cargo.toml              lib-fltk-omarchy + the fltk pin, release profile
bin/build               cargo build --release → target/release/<name>
bin/test                cargo test, the UI test inside xvfb-run
bin/install             makepkg -fsi, so the app lands in the launcher
src/main.rs             thin: hands the arguments to cli::run
src/lib.rs              NAME, CATALOGS, load_catalogs
src/backend.rs          the app's state and work: plain Rust, no UI
src/ui.rs               the window: layout, keys, nothing else
src/cli.rs              check / render / skill commands, --json
i18n/<code>.json        Omarchy's per-language catalogs
skill/SKILL.md          the app's own agent skill, bundled in the binary
tests/ui.rs             drives the real window offscreen (harness = false)
pkgbuild/PKGBUILD  pkgbuild/<name>.desktop  pkgbuild/<name>.svg
README.md  LICENSE  .gitignore
```

`src/` stays flat. Commit `Cargo.lock`; never commit `target/`.

## How the pieces split

- **`backend` owns the logic**: state, files, processes, anything that can
  fail, as plain structs and functions returning `Result` with a message
  the window can show. Unit tests cover it with temp directories under
  `target/`.
- **`ui` owns layout and interaction**: an `AppWindow`, its header actions
  and every key. Data the window shows lives in `State<T>` bound to widgets
  (`Label::bind`, `i18n::map(&state, ..)`); change it with `set`/`update`,
  never from a thread.
- Slow work runs in `Task::new(|ctx| ..).on_done(..).start()` (call
  `ctx.check()?` often so it can be cancelled); `task::post` hands results
  to the UI thread.
- Heavy media work goes through the `ffmpeg`/`ffprobe` command-line tools
  (`std::process::Command` inside a Task), never by linking libav. Playback:
  the `playback` feature's `media::Video`/`media::Audio`, which drive the
  system ffmpeg as child processes.
- Open and save dialogs go through xdg-desktop-portal:
  `dialog::Request::open(..)`/`save(..)` with `Filter`s, `run_async` (answer
  on the UI thread) or `run` (blocks, the UI keeps drawing). The native
  chooser is only the fallback. Tests replace it with `dialog::set_picker`.

## Omarchy conventions

- **Follow the theme live.** `App::new` loads Omarchy's theme and follows
  `omarchy-theme-set`. Colors come from the theme (`theme::with(|t|
  t.palette.accent)`, `Tone::Accent`, gfx `u.accent`), never hex literals.
  Never change Omarchy's config.
- **Speak the user's language.** The toolkit follows Setup > Language live.
  Every visible string is `tr!("Save")`, with placeholders
  (`tr!("Exported to {path}", path = p)`) instead of glued sentences, and
  counted strings `trn!("{count} clips", n)`. Ship `i18n/<code>.json`
  (English text → translation, `{"one": .., "other": ..}` for counts;
  English needs only the plural forms), compiled in and loaded with
  `i18n::load_omarchy_catalog`. Bind live text so a switch re-labels the
  window: `i18n::text`, `i18n::map`, `bind_tooltip`, `i18n::bind`. Numbers
  with `i18n::format_number`. Arabic, Hebrew, Persian and Urdu mirror on
  their own: build left to right. Translate the `.desktop` `Comment` (and
  `Name` unless it is a brand). Keep brand names, units and keys as they are.
- **Corners match the windows.** The theme follows Hyprland's
  `decoration:rounding` (sharp on Omarchy's default); draw custom panels
  with `t.radius`, thumbnails with `t.badge_radius()`; offer
  `theme::set_corners(..)` in Preferences only if the app has them.
- **Place things where the pointer is.** Anything the user inserts lands
  under the mouse when it's over the content (`fltk::app::event_x/_y`,
  `PopupMenu::open_at`), else in the middle of what's visible.
- **Keyboard first.** `win.help_overlay(title, sections)` makes `?` (and
  F1) show every shortcut; `win.quit_key(|| unsaved)` makes `Q` quit,
  asking first when there is unsaved work; `win.shortcut("Space", ..)` is
  the main action, `Mod+Z` / `Mod+Shift+Z` undo and redo (`Mod` is Ctrl,
  Cmd on macOS). Every action has a key and is listed in the overlay and
  in the README's table. Never write your own Tab handling.
- **Careful with data.** Never modify the user's originals. Save atomically
  (write beside the target, `sync_all`, rename). Ask before discarding
  work (`ConfirmDialog::open`). A file that cannot be read is never
  overwritten.
- **Say what's wrong.** A missing tool, device, portal or permission is
  shown in the window (a status line, a toast), never worked around by
  quietly doing less. `dialog::Error` and backend errors have messages.
- **Agents can drive it.** Commands that need no window, as Hype has them:
  `<name> check`, `<name> render`, each with `--json`, exit 0 or 1 with the
  error on stderr, and `<name> skill install` copying the bundled
  `skill/SKILL.md` to `~/.agents/skills/<name>/` and linking it into
  `~/.claude/skills/`. `render` builds the window with `ui::build` and
  `win.snapshot(path)` without showing it (it needs a display connection:
  `xvfb-run`). The GUI itself is drivable with the test kit's env runner:
  `OMARCHY_FLTK_SCRIPT=steps.json <name>` (docs/TESTKIT.md).
- Comments explain why, in full sentences. Commit messages are short and
  imperative ("Count from the keyboard").

## Loop

1. `bin/build`, then `cargo run` to try it (pass a file argument if the app
   opens files). Never open windows on the user's focused workspace from
   scripts; `script/real-window`-style hidden workspaces or Xvfb only.
2. `bin/test` after each change: unit tests plus `tests/ui.rs`, which runs
   `testkit::run_script` against the real window offscreen (keys by name,
   `expect` by visible text, a snapshot) inside its own Xvfb.
3. Look at it: `<name> render --out shot.png` (or the snapshot from the UI
   test) and read the PNG against the theme before calling it done. Render
   once in another language (`--lang da`): longer words must not clip, and
   nothing should still be in English.
4. For interaction, script it: `{"press": "Space"}`, `{"click": "Export…"}`,
   `{"expect": "2 taps"}`, `{"tree": "t.json"}`, `{"snapshot": "a.png"}`.
   Set `anim::set_enabled(false)` for final frames.
5. `bin/install` builds the package with `makepkg -fsi`: binary, `.desktop`
   entry and icon. The app then appears in the launcher (`Super + Space`).
   Run it again after every change the user should see there.

## Toolkit traps

- `flex.fit(&*w)` keeps a component's Omarchy size; un-fitted widgets
  stretch. In fltk the creation order decides the parent: create inside the
  body, then fit. Call `win.end()` before `show()`.
- Every color from the theme, every string through `tr!`: a hex literal or
  an untranslated label is a bug even if it looks right today.
- App shortcuts are `Mod+…`; `Ctrl` is always the Control key. Plain keys
  (`Space`, `Q`, `?`) only reach window shortcuts when the focused control
  did not use them, so typing in a field still works.
- Icon-only header actions need a name: their tooltip, kept live with
  `.bind_tooltip(&i18n::text(".."))`.
- `w()`/`h()` and friends need `use fltk::prelude::*` (or `WidgetExt`).
- Lists of any size are `List::new(&count, |i| ListRow::new(..))` or a
  `ListModel`, never a widget per item.
- UI tests run on the main thread (`harness = false`); unset
  `WAYLAND_DISPLAY` and use `xvfb-run`, never the user's session.
- Build in the project's `target/`, never in `/tmp`.

## README and release

- The README opens with a picture of the app doing its job:
  `OMARCHY_FLTK_STATE_DIR=/nonexistent <name> render --out
  docs/screenshot.png` gives Omarchy's default theme; use made-up content,
  never the user's name, files or other personal data.
- To ship: tag `v<version>` and make a GitHub release, then add the package
  to `omacom/omarchy-pkgs` under `pkgbuilds/<name>/` building from the
  release tarball (`cargo build --release --locked`), with
  `.omarchy/package.json` set to `"source": "local"`, `"auto_merge": true`,
  and an `upstream.watch` on the GitHub repo so later releases sync on
  their own. Only once that package is published can Omarchy install it:
  `install/omarchy-base.packages`, the preinstall scripts, and a migration
  for existing machines.
