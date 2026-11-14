#!/usr/bin/env bash
# Runs a command inside a headless mutter Wayland session with its own D-Bus.
#
#   tools/headless/run.sh <command…>
#   PDFRS_SCALE=1.25 tools/headless/run.sh <command…>   # HiDPI / fractional scale
#
# Fractional scales use mutter's scale-monitor-framebuffer feature, enabled
# through a throwaway GSettings keyfile, so the user's settings are untouched.
#
# Needs mutter and dbus-run-session (Fedora: mutter, dbus-daemon).
set -u
if [ -z "${PDFRS_HEADLESS_INNER:-}" ]; then
  exec env PDFRS_HEADLESS_INNER=1 dbus-run-session -- "$0" "$@"
fi
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
display=wayland-pdfrs-$$
log=$(mktemp -t pdfrs-mutter.XXXXXX)
config=$(mktemp -d -t pdfrs-xdg.XXXXXX)
mkdir -p "$config/glib-2.0/settings"
printf '[org/gnome/mutter]\nexperimental-features=["scale-monitor-framebuffer"]\n' \
  >"$config/glib-2.0/settings/keyfile"
GSETTINGS_BACKEND=keyfile XDG_CONFIG_HOME=$config mutter --headless --wayland --no-x11 --wayland-display="$display" \
  --virtual-monitor 1920x1200 >"$log" 2>&1 &
mutter=$!
for _ in $(seq 50); do [ -S "$XDG_RUNTIME_DIR/$display" ] && break; sleep 0.1; done
if [ -n "${PDFRS_SCALE:-}" ]; then
  sleep 0.5
  python3 -I "$(dirname "$0")/set-scale.py" "$PDFRS_SCALE" >&2
  sleep 0.5
fi
WAYLAND_DISPLAY=$display GDK_BACKEND=wayland GTK_A11Y=none "$@"
rc=$?
kill "$mutter"; wait "$mutter" 2>/dev/null
rm -rf "$log" "$config"
exit $rc
