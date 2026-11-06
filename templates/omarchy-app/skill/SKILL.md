---
name: {{name}}
description: >
  Use {{Name}}, the keyboard tally counter, from the command line: read or
  check the saved count, render the window to a PNG, drive it headlessly.
  Triggers: {{name}}, tally, count, {{name}} check, {{name}} render.
---

# {{Name}}

{{Name}} counts things from the keyboard (Space counts, Ctrl+Z undoes).
The count lives in `~/.local/share/{{name}}/tally.json` as
`{"count": <n>}`; never edit it while the app is open with unsaved work.

## Commands

All take `--json`, exit 0 on success and 1 with the error on stderr.

- `{{name}} check [file] --json`: `{"file", "count", "ok", "problems":
  [{"severity", "message"}]}`.
- `{{name}} render --out shot.png [--count n] [--lang code] --json`: the
  window as a PNG without showing it (`{"png", "width", "height",
  "count"}`). Needs a display: wrap it in `xvfb-run -a` when there is none.
- `{{name}} version`, `{{name}} help`.

## Driving the window

The toolkit's test kit runs a script of steps against the real window,
headlessly, and exits 0 when every step passed:

```sh
cat > steps.json <<'JSON'
[{"press": "Space"}, {"press": "Space"}, {"expect": "2 taps"},
 {"press": "Mod+Z"}, {"expect": "1 tap"}, {"snapshot": "after.png"}]
JSON
env -u WAYLAND_DISPLAY xvfb-run -a env OMARCHY_FLTK_SCRIPT=steps.json \
  OMARCHY_FLTK_TREE=tree.json OMARCHY_FLTK_REPORT=report.json {{name}}
```

`tree.json` lists every element (kind, name, value, rect); look at the
PNG before calling a change done. Never send input to the user's own
window.
