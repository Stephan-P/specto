#!/usr/bin/env bash
# Fetches the test corpus into corpus/files/ (git-ignored).
# Currently: the pdf.js test PDFs (~150 MB, ~1100 files).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p files
if [ ! -d files/pdfjs ]; then
  git clone --depth 1 --filter=blob:none --sparse https://github.com/mozilla/pdf.js.git files/pdfjs
  git -C files/pdfjs sparse-checkout set test/pdfs
else
  git -C files/pdfjs pull --depth 1
fi
echo "corpus ready: $(find files -name '*.pdf' | wc -l) PDFs"
