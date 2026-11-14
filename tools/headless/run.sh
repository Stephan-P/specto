#!/usr/bin/env bash
# Runs a command inside a headless mutter Wayland session with its own D-Bus.
#
#   tools/headless/run.sh <command…>
#
# Needs mutter and dbus-run-session (Fedora: mutter, dbus-daemon).
set -u
if [ -z "${PDFRS_HEADLESS_INNER:-}" ]; then
  exec env PDFRS_HEADLESS_INNER=1 dbus-run-session -- "$0" "$@"
fi
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
display=wayland-pdfrs-$$
log=$(mktemp -t pdfrs-mutter.XXXXXX)
mutter --headless --wayland --no-x11 --wayland-display="$display" \
  --virtual-monitor 1920x1200 >"$log" 2>&1 &
mutter=$!
for _ in $(seq 50); do [ -S "$XDG_RUNTIME_DIR/$display" ] && break; sleep 0.1; done
WAYLAND_DISPLAY=$display GDK_BACKEND=wayland GTK_A11Y=none "$@"
rc=$?
kill "$mutter"; wait "$mutter" 2>/dev/null
rm -f "$log"
exit $rc
