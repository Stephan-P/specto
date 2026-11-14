#!/bin/sh
# Builds the Flatpak and installs it for the current user.
#
#     packaging/flatpak/build.sh [flatpak-builder options]
#
# Needs flatpak-builder (or the org.flatpak.Builder Flatpak) and, from
# Flathub, org.gnome.Sdk and org.freedesktop.Sdk.Extension.rust-stable in
# the versions the manifest names (--install-deps-from=flathub fetches
# them). Builds in target/flatpak.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
work=$root/target/flatpak

python3 "$here/cargo-sources.py" "$root/Cargo.lock" "$here/cargo-sources.json"

if command -v flatpak-builder >/dev/null; then
    builder=flatpak-builder
else
    builder="flatpak run --filesystem=$root org.flatpak.Builder"
fi
mkdir -p "$work"
$builder --user --install --force-clean --install-deps-from=flathub \
    --state-dir="$work/state" "$@" "$work/build" "$here/net.xfbs.pdfrs.yml"
