# M0 spike findings (2026-11-14)

Machine: aarch64 Fedora 44, 4 cores, Rust 1.100, release builds.
Versions: hayro 0.9.0, lopdf 0.47.0, Poppler (`pdftoppm`) as the reference.

## hayro rendering (D1)

Corpus: pdf.js `test/pdfs`, 1095 files. These are deliberately nasty edge cases, much harder than everyday PDFs.
Rendered the first 3 pages of each at scale 2 (144 dpi). Each file ran in its own subprocess with a 60 s timeout.

| | |
|---|---|
| Files rendered | 1073 / 1095 |
| Crashes (panic / abort) | **0** |
| Timeouts (> 60 s) | 4, but see the correction below: 3 of them were Poppler, not hayro |
| Failed to open | 18: 6 need a password, 12 invalid. Poppler can't open most of the invalid ones either. |
| Page time | median 2.8 ms · p90 22 ms · p99 213 ms · max 5.2 s |
| Total wall time | 147 s for 1095 files including Poppler reference renders |

**Comparison with Poppler.** The metric counts a pixel as differing only if no reference pixel within 1 px is within ±48 per channel. This ignores anti-aliasing and sub-pixel glyph placement. On everyday documents (system manuals, font specimens) the noise floor is below 3%.
Of 1208 compared pages: 168 are > 1% different, 68 > 5%, 17 > 20%.

Manual review of the 8 worst:

| File | Who is right |
|---|---|
| `xobject-image`, `issue19484_1/2` | hayro. Poppler renders blank (odd encryption / XObject edge case). |
| `bug1552113` | hayro. The test says "this text should be visible", and Poppler covers it. |
| `jpx_subsampling` | hayro. Poppler drops parts of the JPEG 2000 images. |
| `smask_alpha_oob` | Unclear: an out-of-bounds soft-mask edge case. |
| `knockout_blend_multiply` | **Poppler.** hayro doesn't support knockout groups (a documented limitation). |
| `transfer_maps` | **Poppler.** hayro ignores transfer functions (rare, mostly print-production files). |
| `devicen-none-zero-domain` | Probably Poppler. hayro differs in one swatch. |

**Conclusions**

- hayro is good enough to build on. It was robust (no crashes) and fast on typical pages, and roughly half the "worst" differences were Poppler's bugs, not hayro's.
- Its README is outdated. 0.9 **does** decrypt RC4/AES-128/AES-256 (`Pdf::new_with_password`). It also has `RenderContent::{PAGE, ANNOTATIONS}`, so page content can be rendered without annotations. That's exactly what our "annotations as overlay" design needs, and it's verified by a test in `pdfrs-render`.
- We need **guards** against pathological files. hayro has no cancellation API, so the viewer must render on worker threads and be able to drop late results.
- **Correction (M1):** the corpus runner rendered with hayro *and* `pdftoppm` within the same per-file timeout. Re-timed with hayro alone: `tiling-pattern-tiny-steps` 0.5 ms, `tiling-pattern-tiny-steps-large` 4.6 ms, `bomb_giant` 1.3 ms. Those were **Poppler** hangs. Only `issue19517` is slow in hayro: 41 s for a 4.4 × 5.9 m page at 2× (25216 × 33612 px). (The runner also orphaned the hung `pdftoppm` processes; it now gives them their own 20 s timeout.)
- `RenderCache` borrows the `Pdf` and uses `Rc`, so it's `!Send` and can't easily be stored next to the document. For the viewer we'll want one cache per worker thread (e.g. a thread owns a `Pdf` and its cache). We need to measure how much the cache helps on text-heavy pages.
- Poppler is a weak oracle. Getting PDFium (BSD) reference renders would make the regression suite more useful.

## lopdf editing and saving (D2, D9)

Parsing the same 1095 files: lopdf loaded 1092 and hayro 1086. Only 1 file opened in hayro but not lopdf (`bug1606566.pdf`), with 0 panics.
Caveat: lopdf is lenient, so "loaded" doesn't guarantee every object parsed correctly.

Editing scenarios (`spikes/lopdf-spike`). Every output was re-opened and fully rendered by hayro, checked with Poppler's `pdfinfo`/`pdftoppm`, and passed `qpdf --check`:

| # | Scenario | Result |
|---|---|---|
| 1 | Delete page (full rewrite) | ✅ |
| 2 | Merge two files (full rewrite) | ✅ but requires manual work (see below) |
| 3 | Reorder pages (full rewrite) | ✅ |
| 4 | Add highlight with appearance stream (**incremental**, original bytes preserved) | ✅ visible in hayro and Poppler |
| 5 | Encrypt with AES-256 | ✅ Poppler reports `algorithm:AES-256` |
| 6 | Add highlight to an **encrypted** file (incremental, re-encrypts appended objects) | ✅ |
| 7 | Delete page from an encrypted file and **keep the encryption** (decrypt → edit → re-encrypt with the original state) | ✅ |

**Conclusions**

- lopdf covers everything D9 needs: decrypt, encrypt (RC4 through AES-256), and encrypted incremental updates. No need to write our own security handler.
- lopdf has no "import pages from another document" API. Merging means renumbering objects, copying them, and splicing the page tree. You must also **push inheritable attributes** (`Resources`, `MediaBox`, `CropBox`, `Rotate`) down onto each page first, or merged pages lose their resources. That's about 80 lines in the spike, and it belongs in `pdfrs-write`.
- Merging doesn't yet handle catalog-level data from the second file: outlines, named destinations, AcroForm fields, structure tree. Form fields especially need merging into `/AcroForm /Fields` (M5).
- lopdf loads the whole document into memory as an object graph. That's fine for saving, but it's another reason to keep hayro (which parses lazily) for viewing.

## GTK4 canvas (D3)

`spikes/gtk-canvas` is a custom `GtkWidget` with continuous vertical layout. It uses 512 px tiles at zoom levels in √2 steps, rendered by a worker pool that each frame replaces its queue with the currently visible tiles. A 0.25× render of each page is drawn first as a placeholder, tiles are kept in an LRU cache (384 MB), and tiles become `GdkMemoryTexture`s drawn with `append_scaled_texture`. `--bench` runs a scripted 26 s session: a 4000 px/s scroll, zoom to 3×, scroll while zoomed, zoom out to 0.4×, then a 6000 px/s scroll.

Run on the 384-page Valgrind manual under a headless `mutter` (1100×854 window, scale 1, GSK Vulkan renderer on the real GPU):

| Render threads | fps | p95 frame | p99 frame | frames > 33 ms | tiles | tile render median / p95 |
|---|---|---|---|---|---|---|
| 1 | 53.8 | 31.6 ms | 33.4 ms | 13 | 498 | 3.0 / 9.5 ms |
| 2 | 54.5 | 30.7 ms | 33.2 ms | 6 | 499 | 2.7 / 9.8 ms |
| 4 | 55.6 | 30.2 ms | 33.0 ms | 5 | 495 | 2.3 / 10.0 ms |

- Building the frame (`snapshot()`) is negligible: median 0.01 ms, max about 9 ms.
- The only frame over 50 ms is the first one (about 167 ms, window mapping and Vulkan setup). During scrolling and zooming, the worst case is a single missed vsync (33 ms) in about 5% of frames.
- Render threads hardly matter on this document: hayro renders a 512 px text tile in about 3 ms. The bottleneck for heavier documents will be render time, not GTK.
- About 13% of frames showed a placeholder somewhere, almost all during the 4000–6000 px/s flicks. That's the intended blurry-then-sharp behavior.

**Conclusion:** GTK4 is fast enough for the canvas. **Decision D3 stands.**

**Caveats:** this was headless (no real display or vsync from a physical monitor) at scale 1. Fractional HiDPI (`Surface::scale()`) is wired up but untested, as is touchpad pinch, which needs real hardware. Worth a manual run: `cargo run --release -p gtk-canvas -- some.pdf`.

## Not done yet

- Verify Preview's encryption behavior on a Mac (D9).
- PDFium reference renders and the PDFium test corpus.
- Manual check of the GTK canvas on a real HiDPI display with a touchpad.
