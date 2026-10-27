# Performance: lib-fltk-omarchy vs QML (0.0.1)

Measured 2026-10-27 on Asahi Linux (Apple M2, aarch64), Hyprland 0.56.2,
Qt 6 `qml6`, release build of this crate (`opt-level = "s"`, thin LTO,
static FLTK 1.4.5 from fltk-sys 1.5.23 with `use-wayland`).

## Method (`examples/measure.rs`, `script/bench`)

- Each workload runs as a real Wayland client launched with
  `hl.dsp.exec_cmd(cmd, { workspace = "special:omarchy-fltk-bench silent", float = true })`:
  hidden workspace, floating at the requested 520×400 from the first frame,
  the user's focus is never touched.
- **startup_ms**: from the dispatch to the window appearing as mapped in
  Hyprland's client list (5 ms polling). Same method for every toolkit.
- **pss_kb**: `Pss + SwapPss` from `/proc/<pid>/smaps_rollup`, summed over
  the whole process tree, 2 s after mapping. Never RSS.
- **idle_cpu_s**: utime + stime of the tree over 5 s without input
  (10 ms tick resolution).
- 5 runs per case, medians reported. Runs were sequential; no Files build
  was running (checked with `pgrep`).

Workloads (`examples/bench_*.rs`, `bench/*.qml`), functionally equivalent:
- **minimal**: a label and a button.
- **app**: header, text field bound to a label, two buttons, a progress
  bar and a 100 000-row virtualized list with two columns. The Rust app
  also runs the live Omarchy theme watcher.

## Results

| Case | startup (median) | PSS (median) | PSS (max) | idle CPU / 5 s |
|---|---|---|---|---|
| lib-fltk-omarchy minimal | 57 ms | 16.0 MB | 16.0 MB | 0.00 s |
| QML software minimal | 100 ms | 61.7 MB | 61.8 MB | 0.00 s |
| QML GPU minimal | 155 ms | 111.5 MB | 112.4 MB | 0.00 s |
| lib-fltk-omarchy app | 63 ms | 16.0 MB | 16.1 MB | 0.00 s |
| QML software app | 134 ms | 72.6 MB | 72.8 MB | 0.00 s |
| QML GPU app | 168 ms | 125.4 MB | 126.5 MB | 0.00 s |

For these workloads: about 4.5× less memory than QML's software renderer
and 7.8× less than QML on the GPU; first frame 2–2.7× sooner. Binary size
of the release `bench_app`: 1.26 MB (FLTK statically linked).

## Window size matters

Memory grows with window area for any software-rendered toolkit (two
wl_shm buffers plus a draw buffer). The real-window scenario
(`script/real-window`) maps a normal tiled window at ~1900×1030 first and
reports ~52 MB PSS. `Mode::Floating` avoids the large first buffers by
mapping with a fixed size so Hyprland floats it at the requested size.

## Not claimed

Other workloads, other machines, GPU-heavy content, and long-running
memory behaviour were not measured. Numbers from the abandoned Ruby
prototype are not comparable and are not used here.
