#!/usr/bin/env bash
# Runs a command inside a headless mutter Wayland session with its own D-Bus.
#
#   apps/desktop/test-ui/run.sh <command>
#   SPECTO_SCALE=1.25 apps/desktop/test-ui/run.sh <command>        # HiDPI or fractional scale
#   SPECTO_MONITOR=2400x1800 apps/desktop/test-ui/run.sh <command> # monitor size, device pixels
#
# Fractional scales use mutter's scale-monitor-framebuffer feature, enabled
# through a throwaway GSettings keyfile, so the user's settings are untouched.
#
# Needs mutter and dbus-run-session. On Fedora they are in the packages mutter
# and dbus-daemon (see ci/install-deps-fedora.sh --headless).
set -u
if [ -z "${SPECTO_HEADLESS_INNER:-}" ]; then
  exec env SPECTO_HEADLESS_INNER=1 dbus-run-session -- "$0" "$@"
fi
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
# Containers (CI) have no runtime directory.
runtime=
if [ ! -w "$XDG_RUNTIME_DIR" ]; then
  runtime=$(mktemp -d -t specto-runtime.XXXXXX)
  export XDG_RUNTIME_DIR=$runtime
fi
display=wayland-specto-$$
log=$(mktemp -t specto-mutter.XXXXXX)
config=$(mktemp -d -t specto-xdg.XXXXXX)
mkdir -p "$config/glib-2.0/settings"
printf '[org/gnome/mutter]\nexperimental-features=["scale-monitor-framebuffer"]\n' \
  >"$config/glib-2.0/settings/keyfile"
GSETTINGS_BACKEND=keyfile XDG_CONFIG_HOME=$config mutter --headless --wayland --no-x11 --wayland-display="$display" \
  --virtual-monitor "${SPECTO_MONITOR:-1920x1200}" >"$log" 2>&1 &
mutter=$!
for _ in $(seq 100); do [ -S "$XDG_RUNTIME_DIR/$display" ] && break; sleep 0.1; done
if [ ! -S "$XDG_RUNTIME_DIR/$display" ]; then
  echo "mutter didn't start:" >&2
  cat "$log" >&2
  kill "$mutter" 2>/dev/null
  exit 1
fi
if [ -n "${SPECTO_SCALE:-}" ]; then
  sleep 0.5
  if ! python3 -I "$(dirname "$0")/set-scale.py" "$SPECTO_SCALE" >&2; then
    echo "couldn't set the scale to $SPECTO_SCALE" >&2
    kill "$mutter"
    exit 1
  fi
  sleep 0.5
fi
WAYLAND_DISPLAY=$display GDK_BACKEND=wayland GTK_A11Y=none "$@"
rc=$?
kill "$mutter"; wait "$mutter" 2>/dev/null
rm -rf "$log" "$config" $runtime
exit $rc
