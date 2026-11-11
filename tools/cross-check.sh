#!/bin/sh
# Compile-checks the library's sources for macOS (x86_64, arm64) and
# Windows (MinGW, x86_64) with zig's bundled cross toolchains. Nothing is
# linked or run: it catches missing headers, POSIX-only calls and type
# errors in the code paths Linux does not build.
#
#   ZIG=/path/to/zig tools/cross-check.sh            # every src/*.cpp
#   ZIG=... tools/cross-check.sh src/process.cpp     # some files
#
# zig can come from a throwaway Python environment (pip install ziglang;
# ZIG="python -m ziglang"). FLTK's headers come from FLTK_INCLUDE
# (default ~/.local/opt/fltk-1.4.5/include) with an fl_config.h that turns
# off the X11, Wayland and cairo backends, written under build/.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd -P)
zig=${ZIG:?set ZIG to a zig binary (or "python -m ziglang")}
fltk=${FLTK_INCLUDE:-$HOME/.local/opt/fltk-1.4.5/include}
work=$here/build/cross-check
mkdir -p "$work/overlay/FL" "$work/cache"
sed -e 's/^#define FLTK_USE_CAIRO 1/#define FLTK_USE_CAIRO 0/' \
    -e 's/^#define FLTK_USE_WAYLAND 1/#define FLTK_USE_WAYLAND 0/' \
    -e 's/^#define FLTK_USE_X11 1/#define FLTK_USE_X11 0/' \
    -e 's|^#define FLTK_HAVE_CAIRO 1|/* #undef FLTK_HAVE_CAIRO */|' \
    "$fltk/FL/fl_config.h" > "$work/overlay/FL/fl_config.h"
export ZIG_GLOBAL_CACHE_DIR="$work/cache" ZIG_LOCAL_CACHE_DIR="$work/cache"
cd "$here"
[ $# -gt 0 ] || set -- src/*.cpp
status=0
for target in x86_64-macos aarch64-macos x86_64-windows-gnu; do
  for f in "$@"; do
    if ! out=$($zig c++ -target "$target" -std=c++17 -Wall -Wextra -c -o "$work/out.o" \
        -I"$work/overlay" -Iinclude -I"$fltk" "$f" 2>&1); then
      status=1
    fi
    out=$(printf '%s\n' "$out" | grep -v 'unused-command-line-argument' | grep -E 'error|warning' || true)
    if [ -n "$out" ]; then
      echo "== $target $f"
      printf '%s\n' "$out"
      status=1
    fi
  done
done
[ $status -eq 0 ] && echo "cross-check: clean for macOS (x86_64, arm64) and Windows (MinGW)"
exit $status
