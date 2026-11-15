#!/usr/bin/env bash
# Fetches the test corpus into corpus/files/ (git-ignored).
# Currently: the pdf.js test PDFs (about 150 MB and 1,100 files). Only that
# directory of the pdf.js repository is checked out, without its history.
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
