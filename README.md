# lib-fltk-omarchy

Omarchy toolkit for [fltk-rs](https://github.com/fltk-rs/fltk-rs): the
classic Omarchy components (the omarchy-shell `Ui/` kit) ready to use, with
the live Omarchy theme, Hyprland window modes and small app helpers
(reactive state, cancellable background tasks, virtual lists, offscreen
snapshots). Import as `fltk_omarchy`. License MIT. Version 0.0.1, not
published.

![gallery](docs/gallery.png)

- **Theme**: reads `~/.local/state/omarchy/current/theme/{colors,shell}.toml`
  and `~/.config/omarchy/shell.toml` with the same keys and defaults as
  omarchy-shell, Hyprland rounding/gaps, the `monospace` font; follows
  `omarchy-theme-set` live.
- **Components**: see [docs/COMPONENTS.md](docs/COMPONENTS.md).
- **Capabilities vs QML**: [docs/CAPABILITIES.md](docs/CAPABILITIES.md).
- **Measured**: ~16 MB PSS and ~60 ms to first frame for a 520×400 app vs
  62–73 MB (QML software) and 111–125 MB (QML GPU) on the same machine —
  method and limits in [docs/PERFORMANCE.md](docs/PERFORMANCE.md).
- **For agents**: [AGENTS.md](AGENTS.md).

```sh
cargo run --example gallery      # all components
cargo run --example overlays     # dropdown, menu, confirm, notification, list
cargo test --offline             # unit + doc + main-thread UI tests
```

Requires the build dependencies of fltk-sys (cmake, a C++ compiler,
Wayland/X11/pango/cairo development files).
