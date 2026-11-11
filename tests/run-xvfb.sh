#!/bin/sh
# Runs a test binary inside a private Xvfb, started for this run only.
#
#   run-xvfb.sh BINARY [ARGS...]
#
# Exits 77 (ctest's SKIP_RETURN_CODE) when Xvfb is not installed. The X
# server's display number is chosen by Xvfb itself (-displayfd), its log
# and display file go to a fresh 0700 directory under $FO_TEST_RUNS, and
# the server is stopped by its own PID afterwards. Nothing is deleted.
set -u
bin=$1
shift
if ! command -v Xvfb >/dev/null 2>&1; then
  echo "Xvfb not found: skipping the display tests"
  exit 77
fi
case ${FO_TEST_RUNS:-} in
/*) ;;
*) echo "FO_TEST_RUNS must be an absolute directory (run through ctest)" >&2; exit 2 ;;
esac
umask 077
mkdir -p "$FO_TEST_RUNS" || exit 2
dir=$(mktemp -d "$FO_TEST_RUNS/xvfb-XXXXXX") || exit 2
Xvfb -displayfd 3 -screen 0 1280x800x24 -nolisten tcp 3>"$dir/display" >"$dir/xvfb.log" 2>&1 &
xvfb=$!
stop() { kill "$xvfb" 2>/dev/null; wait "$xvfb" 2>/dev/null; }
trap 'stop; exit 130' INT TERM
n=0
while [ ! -s "$dir/display" ] && [ $n -lt 100 ]; do
  if ! kill -0 "$xvfb" 2>/dev/null; then
    echo "Xvfb did not start: skipping (see $dir/xvfb.log)"
    exit 77
  fi
  sleep 0.05
  n=$((n + 1))
done
if [ ! -s "$dir/display" ]; then
  echo "Xvfb did not report a display: skipping (see $dir/xvfb.log)"
  stop
  exit 77
fi
DISPLAY=":$(head -n 1 "$dir/display")"
export DISPLAY
echo "private Xvfb on $DISPLAY ($dir)"
"$bin" "$@"
status=$?
stop
exit $status
