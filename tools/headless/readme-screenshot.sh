#!/usr/bin/env bash
# Makes the screenshot in apps/desktop/README.md: hwloc's manual (BSD
# licence, `dnf install hwloc` puts it in /usr/share/doc/hwloc), two pages
# side by side with the Contents tab open, at scale 2, in a window of the
# default size (the monitor must be big enough for it).
#
#   cargo build --release -p pdfrs-desktop
#   PDFRS_SCALE=2 PDFRS_MONITOR=2400x1800 tools/headless/run.sh tools/headless/readme-screenshot.sh
set -eu
pdf=${1:-/usr/share/doc/hwloc/hwloc-a4.pdf}
out=$(realpath "${2:-apps/desktop/screenshot.png}")
bin=${PDFRS_GTK:-target/release/pdfrs-gtk}
app=net.xfbs.pdfrs obj=/net/xfbs/pdfrs/window/1
win() { gdbus call --session --dest $app --object-path $obj \
          --method org.gtk.Actions.Activate "$1" "${2:-[]}" '{}' >/dev/null; }

# Fresh saved state and recent files, so yours aren't used or touched.
home=$(mktemp -d)
trap 'kill $pid 2>/dev/null; rm -rf "$home"' EXIT
XDG_STATE_HOME=$home/state XDG_DATA_HOME=$home/data PDFRS_DEBUG=1 "$bin" "$pdf" 2>/dev/null &
pid=$!
sleep 2
win sidebar-page "[<'outline'>]"
win layout "[<'two-page-continuous'>]"
win debug-go-to "[<'3'>]"
sleep 3 # let the pages render sharp
win debug-screenshot "[<'$out'>]"
sleep 1
echo "$out"
