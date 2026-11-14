# Test corpus

Run `./fetch.sh` to download the corpus into `files/` (not committed).

Then, from the repository root:

```sh
cargo build --release -p pdfrs-cli
target/release/pdfrs corpus corpus/files/pdfjs/test/pdfs --reference --max-pages 3
```

This writes `out/corpus/report.md`, `results.tsv`, and side-by-side PNGs for pages that differ from
`pdftoppm` (Poppler) in `out/corpus/diffs/`.

**Poppler is not ground truth.** In the M0 run, about half of the worst differences were Poppler
bugs (see `docs/M0-FINDINGS.md`). Treat a high diff as "look at this page", not "hayro is wrong".

To do: add the PDFium test corpus and our own fixtures (forms, rotation, CropBox, encryption).
