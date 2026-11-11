#!/bin/sh
# Takes the component screenshots in docs/components/ from the gallery's
# demo states, in a private Xvfb with a private 0700 HOME and XDG folders
# (no Omarchy theme there: the toolkit's Tokyo Night fallback, as Files').
# Each run is kept under build/manual-runs/; nothing is deleted.
#
#   tools/screenshots.sh BUILD_DIR [DEMO...]
set -eu
here=$(cd "$(dirname "$0")/.." && pwd -P)
build=$(cd "$1" && pwd -P)
shift
demos=${*:-"groups list grid columns rail menu actions dialog prompt help toast tooltip sidebar-focus"}
umask 077
mkdir -p "$here/build/manual-runs"
run=$(mktemp -d "$here/build/manual-runs/shots-$(date +%Y%m%d-%H%M%S)-XXXXXX")
for d in home home/.config home/.cache home/.local/share home/.local/state tmp runtime; do mkdir -p "$run/$d"; done
disp=${FO_SHOT_DISPLAY:-61}
Xvfb ":$disp" -screen 0 1280x800x24 -nolisten tcp > "$run/xvfb.log" 2>&1 &
xvfb=$!
sleep 1
for demo in $demos; do
  env -i PATH="$PATH" DISPLAY=":$disp" HOME="$run/home" TMPDIR="$run/tmp" XDG_CONFIG_HOME="$run/home/.config" \
    XDG_CACHE_HOME="$run/home/.cache" XDG_DATA_HOME="$run/home/.local/share" \
    XDG_STATE_HOME="$run/home/.local/state" XDG_RUNTIME_DIR="$run/runtime" \
    "$build/gallery" --demo="$demo" > "$run/$demo.log" 2>&1 &
  pid=$!
  sleep 1.6
  import -display ":$disp" -window "lib-fltk-omarchy gallery" "$run/$demo.png"
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  cp "$run/$demo.png" "$here/docs/components/$demo.png"
done
kill "$xvfb" 2>/dev/null || true
echo "$run"
