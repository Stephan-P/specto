#!/usr/bin/env bash
# Opens a PDF in pdfrs-gtk, switches layouts and zoom via D-Bus actions, and
# saves a screenshot after each step.
#
#   cargo build --release -p pdfrs-desktop
#   tools/headless/run.sh tools/headless/smoke.sh <file.pdf> <out-dir>
set -u
pdf=$1 out=$2
app=net.xfbs.pdfrs obj=/net/xfbs/pdfrs/window/1
bin=${PDFRS_GTK:-target/release/pdfrs-gtk}
mkdir -p "$out"
shot() { sleep "${2:-1.5}"; gapplication action $app screenshot "'$(realpath "$out")/$1.png'"; sleep 0.3; }
win() { gdbus call --session --dest $app --object-path $obj \
          --method org.gtk.Actions.Activate "$1" "${2:-[]}" '{}' >/dev/null; }

PDFRS_DEBUG=1 "$bin" "$pdf" 2>"$out/stderr.log" &
pid=$!
sleep 2
shot 1-open
win layout "[<'two-page-continuous'>]"; shot 2-two-page-continuous
win layout "[<'single'>]"; win fit-page; shot 3-single-fit-page
win layout "[<'continuous'>]"; win zoom-actual; shot 4-actual-size
win show-sidebar; win zoom-in; win zoom-in; shot 5-zoomed-no-sidebar
kill $pid; wait $pid 2>/dev/null
ls "$out"/*.png
