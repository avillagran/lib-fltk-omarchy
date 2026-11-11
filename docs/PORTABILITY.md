# Portability

Linux (Wayland and X11) is the platform the toolkit is developed and
tested on. macOS and Windows code paths exist for everything that is not
inherently Linux, and are kept compiling; this page says how far each one
has been checked.

## What runs where

| Piece | Linux | macOS / BSD | Windows |
|---|---|---|---|
| Processes (`run`, `run_sync`, `cancel`) | fork/exec with `pipe2`, `execvpe`, `close_range`; process group killed on timeout | fork/exec with `pipe` + `FD_CLOEXEC`, a PATH search done before fork and `execve`, a `close()` loop; process group | `CreateProcessW`, the child in a job object (kills the tree; `limit_address_space` → per-process memory limit, `limit_cpu_s` → per-process CPU time), reader threads, `run()` waits on a thread and delivers through `Fl::awake` |
| Thread pool (`tasks::run`) | std::thread + `Fl::awake` | same | same |
| `FileWatch` | inotify through `Fl::add_fd` | kqueue `EVFILT_VNODE` through `Fl::add_fd` | polling (size, times, a directory's entries; 1 s) |
| Hyprland IPC, animation switch, refresh rate | Unix sockets | not available (60 Hz, animations on unless the app or `OMARCHY_FLTK_ANIM` says otherwise) | same as macOS |
| Theme | Omarchy's `colors.toml`, followed live | the built-in palette unless a theme dir is given | same |
| Language and catalogs | `~/.config/omarchy/language`, followed live | same file if present (`$XDG_CONFIG_HOME` / `$HOME`) | same, under `%USERPROFILE%` when `HOME` is unset |
| Number, size, date and time formats | the system locale for the language (`newlocale`) when installed, else built-in patterns | same | built-in patterns (no `locale_t`) |
| Drawing, widgets, layers | FLTK (cairo on Wayland) | FLTK (Quartz; no translucency or card scaling without cairo) | FLTK (GDI; same) |

## How it is checked

- **Linux**: the whole test suite (`ctest`), including the display tests
  in a private Xvfb, and the gallery screenshots.
- **Portable POSIX paths on Linux**: configure with
  `-DFO_PORTABLE_POSIX=ON` (a separate build directory) and run `ctest`
  there. The process tests then go through `pipe` + `FD_CLOEXEC`, the
  PATH search with `execve` and the `close()` loop that macOS and the BSDs
  use, and `FileWatch` polls.
- **macOS and Windows**: `tools/cross-check.sh` compiles every source for
  x86_64 and arm64 macOS and x86_64 Windows (MinGW) with zig's cross
  toolchains (headers only: nothing is linked or run).

## Not tested yet

- Nothing has been built with Xcode/Apple clang or MSVC, or run on macOS
  or Windows. In particular the Windows process backend (quoting, the
  environment block, job limits, cancellation, reading output past a cap)
  and the kqueue watcher are compile-checked only.
- Windows paths go through narrow `fopen`/`stat` (the ANSI code page), so
  file names outside it are not read by `platform::read_file`.
- On macOS, apps launched from Finder get no `HOME`-relative Omarchy files;
  the toolkit falls back to its built-in theme and English.
- The built-in date patterns are short CLDR-like patterns written by hand;
  AM/PM markers come from the catalogs, not CLDR.
