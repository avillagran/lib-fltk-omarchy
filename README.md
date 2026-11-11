# lib-fltk-omarchy

An Omarchy toolkit for [FLTK](https://www.fltk.org) 1.4, written in C++17:
the live Omarchy theme, motion that follows Hyprland's animation switch,
keyboard-first widgets and small app helpers, for lightweight desktop apps
that start fast and use little memory.

## From Rust to C++

This toolkit started in Rust, on top of fltk-rs:
[avillagran/lib-fltk-omarchy-rust](https://github.com/avillagran/lib-fltk-omarchy-rust)
is the original project. It worked, but every Rust project carried gigabytes
of build output and long release builds for a small app. FLTK itself is C++,
so this version talks to it directly: no binding layer, a build folder of a
few megabytes, and the same low memory use.

The Rust project stays available as the reference for behavior and design
decisions; new work happens here.

## Status

Early. See [docs/](docs/) as it grows.

## Building

Requires CMake 3.16+, a C++17 compiler and FLTK 1.4 (`pacman -S fltk` on
Arch).

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build
ctest --test-dir build
```

## Platforms

Linux (Wayland and X11) first. FLTK also runs on macOS and Windows, and the
toolkit keeps OS specifics behind one platform layer so it can follow.

## License

MIT. See [LICENSE](LICENSE).
