#!/usr/bin/env bash
# Makes the files for testing the apps by hand (apps/desktop/TESTING.md) in
# target/test-files/: copies of the corpus test files, so saving doesn't
# change the corpus, plus an encrypted file and one over 64 MB.
#
# Needs the corpus (corpus/fetch.sh) and qpdf (`dnf install qpdf` or
# `brew install qpdf`). Running it again starts over with fresh copies.
set -euo pipefail
cd "$(dirname "$0")/.."

corpus=corpus/files/pdfjs/test/pdfs
out=target/test-files
if [[ ! -d "$corpus" ]]; then
  echo "no corpus: run corpus/fetch.sh first" >&2
  exit 1
fi
command -v qpdf >/dev/null || { echo "qpdf is needed" >&2; exit 1; }

rm -rf "$out"
mkdir -p "$out"
# Named after their roles in TESTING.md.
cp "$corpus/freeculture.pdf" "$out/long.pdf"
cp "$corpus/issue14847.pdf" "$out/links.pdf"
cp "$corpus/labelled_pages.pdf" "$out/labels.pdf"
cp "$corpus/hello_world_rotated.pdf" "$out/rotated.pdf"
cp "$corpus/sizes.pdf" "$out/sizes.pdf"
cp "$corpus/issue19517.pdf" "$out/giant.pdf"
cp "$corpus/REDHAT-1531897-0.pdf" "$out/broken.pdf"

# Password "secret" (the owner password is "owner").
qpdf --encrypt secret owner 256 -- "$out/labels.pdf" "$out/encrypted.pdf"

# Over 64 MB, so it's memory-mapped: the long book with 100 MB of random
# data attached (random, so it doesn't compress).
dd if=/dev/urandom of="$out/filler.bin" bs=1048576 count=100 2>/dev/null
qpdf "$out/long.pdf" --add-attachment "$out/filler.bin" -- "$out/big.pdf"
rm "$out/filler.bin"

ls -lh "$out"
