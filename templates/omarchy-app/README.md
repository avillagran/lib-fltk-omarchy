# {{Name}}

![{{Name}}](docs/screenshot.png)
<!-- Render it with the command under Develop (default theme, made-up content). -->

Count things from the keyboard. One window, one job, the Omarchy theme
followed live, in your language.

Built with Rust and [lib-fltk-omarchy](https://github.com/avillagran/lib-fltk-omarchy)
(fltk-rs with Omarchy's look and conventions).

## Keyboard

| Key | Does |
|---|---|
| Space | Count one more |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z | Redo |
| Ctrl+S | Save |
| Ctrl+E | Export… (the desktop's file chooser) |
| ? or F1 | Keyboard shortcuts |
| Q | Quit (asks first when there are unsaved changes) |

On macOS, Cmd takes Ctrl's place.

## Install

```sh
bin/install        # makepkg -fsi: {{name}} appears in the app launcher
```

## Develop

```sh
bin/build          # target/release/{{name}}
bin/test           # unit tests + the offscreen UI test (inside xvfb-run)
cargo run          # open the window
# The picture above, with Omarchy's default theme and made-up content:
OMARCHY_FLTK_STATE_DIR=/nonexistent cargo run -- render --out docs/screenshot.png --count 7
```

Translations live in `i18n/<code>.json` (English text → translation;
counted strings as `{"one": …, "other": …}`), one file per Omarchy
language code, listed in `src/lib.rs`. The app follows Setup > Language
live.

## For agents and scripts

Every command needs no window, takes `--json`, exits 0 on success and 1
with the error on stderr:

```sh
{{name}} check [file] --json          # problems in the saved tally
{{name}} render --out shot.png --count 7 --lang da --json   # needs a display (xvfb-run)
{{name}} skill install                 # ~/.agents/skills/{{name}}, linked into ~/.claude/skills
```

The window itself can be driven headlessly by name through the toolkit's
test kit:

```sh
echo '[{"press": "Space"}, {"expect": "1 tap"}, {"snapshot": "after.png"}]' > steps.json
env -u WAYLAND_DISPLAY xvfb-run -a env OMARCHY_FLTK_SCRIPT=steps.json OMARCHY_FLTK_TREE=tree.json {{name}}
```

## License

MIT
