# M0 spike findings (2026-11-14)

Two of the spikes described here have since been removed, because the desktop app and `pdfrs-text` replaced them: `spikes/gtk-canvas` and `spikes/text-check`. The commands below that use them need a checkout of commit f32a26a, the last one that has them. `spikes/lopdf-spike` is still there.

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

**HiDPI follow-up (M1, macOS, surface scale 2, GTK 4.24, `GskGLRenderer`):** `gtk-canvas file.pdf --probe out.png` prints how the frame maps to device pixels and saves the widget's render node at device resolution. Compared against hayro renders of the same page at the same device scale (edge contrast = mean luminance step across edges, higher is sharper):

| Zoom | Drawn with | Edge contrast | Exact hayro render |
|---|---|---|---|
| 1.0 (level 2, tiles 1:1) | `append_scaled_texture` | 38.2 | 90.5 |
| 1.0 (level 2, tiles 1:1) | `append_texture` | 89.8, pixel-identical (max channel difference 1) | 90.5 |
| Fit width (level 5 drawn at 90.6%) | `append_scaled_texture` | 40.2 | 86.1 |
| Fit width (level 5 drawn at 90.6%) | `append_texture` | 64.6 | 86.1 |

- GSK renders a scaled-texture node at logical resolution on a scaled surface and stretches the result, even with the `Nearest` filter. That halved the sharpness at scale 2. The spike now uses plain texture nodes. This happens in GSK, not in the macOS backend, so it should also hit HiDPI Linux surfaces, but that is untested.
- The surface reports scale 2, and the canvas and page origins land on whole device pixels, so part 1 of the fix works as intended.
- What remains at non-level zooms is the downsampling of √2-level tiles, which is independent of the toolkit (TASKS.md, "HiDPI, part 3").
- Not covered: the probe reads GSK's output, not the screen, so anything the macOS window server does afterwards is not measured.

**HiDPI part 3 (M1, Linux, headless mutter with fractional scaling, GTK 4.22, `GskVulkanRenderer`):** settled views now render tiles at the exact device scale (see ARCHITECTURE.md). The probe gained `PDFRS_LEVELS_ONLY=1` (never settle, the previous behaviour) and a second screenshot, `<out>-surface.png`, which renders the whole window with its surface transform, i.e. what GTK actually puts on the surface. Page 1 of the Valgrind manual against `pdfrs render -s <device scale>`, same edge-contrast measure (mean luminance step > 16 across horizontal neighbours; exact renders score 116–118):

| Surface scale | Zoom | Device px/pt | Levels only (before) | Settled (after) |
|---|---|---|---|---|
| 2 | fit width | 3.0588 (level 4 drawn at 76.5%) | 99.0, 3.3% of pixels off by > 8 | **pixel-identical** |
| 2 | 1.0 | 2.0 (= level 2) | pixel-identical | pixel-identical |
| 1.5 | fit width | 3.0784 (level 4 at 77.0%) | 98.6 | **pixel-identical** |
| 1.5 | 1.0 | 1.5 (level 2 at 75.0%) | 99.9 | **pixel-identical** |
| 1.25 | fit width | 2.1977 (level 3 at 77.7%) | 99.2 | **pixel-identical** (80.7 before part 4) |
| 1.25 | 1.0 | 1.25 (level 1 at 88.4%) | 87.0 | **pixel-identical** (74.0 before part 4) |

- **Part 4, fractional window offsets:** at 1.25 the window was not maximized, so GTK placed its content inside a 25 px CSD shadow: the canvas sat at device (31.25, 88.75). The canvas-only screenshot was still exact, but on the surface every tile was resampled a quarter pixel off, scoring 80.7, *worse* than level tiles. Both canvases now translate their content by the fractional part of their device origin; afterwards all cases above are pixel-identical on the surface too. At scales 2 and 1.5 the origins were already whole pixels.
- `append_texture` (the macOS fix) holds on Linux at 2, 1.5 and 1.25: page origins land on whole device pixels and exact tiles are not resampled.
- **Frame rate** (benchmark at surface scale 2, two runs each): levels only 46.7 / 47.3 fps, settled 51.6 / 50.1 fps, so settling costs nothing (exact tiles are smaller than the oversized level tiles). Both are below the 59 fps at scale 1 because the headless compositor moves 4× the pixels.
- Not covered: physical displays. Headless mutter's virtual monitor offers 1.25, 1.33, 1.5, 1.67, 2 and 2.5; 1.33 and 1.67 were not run.
- Reproduce: `PDFRS_SCALE=1.25 tools/headless/run.sh target/release/gtk-canvas file.pdf --probe out.png` (add `PDFRS_ZOOM=1.0` or `PDFRS_LEVELS_ONLY=1`), then compare the page area of `out-surface.png` (page rect plus the printed device origin) with `pdfrs render file.pdf -p 1 -s <printed device scale>`.

## Android (D4/D5)

`apps/android` is a Gradle project (AGP 9.2, Kotlin + Compose) with one activity. `crates/pdfrs-ffi` wraps `pdfrs-render` with UniFFI proc macros, and the Gradle build runs `cargo ndk` and `uniffi-bindgen` (`tools/uniffi-bindgen`) as generated-source tasks, so "Run" in Android Studio picks up Rust changes. The app renders page 1 of a bundled sample (or a file from the system picker) at screen width.

- hayro and the whole render stack cross-compile to `aarch64-linux-android` with no changes. `libpdfrs_ffi.so` is 5.8 MB stripped (release, no size tuning).
- UniFFI's generated scaffolding compiles under the workspace `unsafe_code = "deny"` lint. Only the JNI module needs an `allow`.
- Kotlin bindings need JNA (`net.java.dev.jna:jna` as an `aar`).

Two ways to get pixels into a `Bitmap`, measured warm on the same document object (40 interleaved rounds, 1080×1530 px, four runs, with the M1 render cache):

| Path | Median | Min |
|---|---|---|
| UniFFI returns `Vec<u8>` → `ByteArray` → `Bitmap.copyPixelsFromBuffer` | 8.7–9.8 ms | 7.5–8.0 ms |
| JNI function locks the `Bitmap` (`AndroidBitmap_lockPixels`) and Rust copies in once | 3.0–3.3 ms | 2.5–2.7 ms |

- Returning a 6.6 MB buffer through UniFFI costs about 6 ms per page on top of a 3 ms render, a third of a 60 Hz frame. UniFFI is fine for the coarse API, but **tiles should not go through it**. (A first measurement a few minutes after the emulator booted showed 61–72 ms vs 16–18 ms; those numbers were inflated by the emulator still settling and did not reproduce.)
- `HayroDocument` is not `Send` since M1 (render cache), but UniFFI objects are called from any thread. The spike's `PdfDocument` holds the shared bytes and opens one copy per calling thread; the real surface should sit on `pdfrs_render::scheduler` instead.
- The JNI side door is small: one `extern "system"` function taking `PdfDocument.rawHandle()` and the `Bitmap`, using only `jni-sys` types and the `ndk` crate (in the spike; the real viewer's JNI functions are in `crates/pdfrs-ffi/src/jni.rs` and take a session id instead of a raw handle). No `jni` crate needed.
- It is not truly zero-copy yet: hayro renders into its own `Pixmap`, which is then copied into the locked bitmap. Rendering straight into bitmap memory would need a hayro/vello_cpu target over a borrowed buffer.

**Conclusion:** Compose + UniFFI works for the API surface, with a hand-written JNI path for tile pixels. **D4 and D5 stand**, with D5's "JNI for the hot path" alternative now required rather than optional.

**Caveats:** measured on the arm64 emulator on an Apple Silicon host, not a phone, and on a simple page where the copy overhead dominates. `rawHandle()` relies on Kotlin keeping the document open during the call, which the real `pdfrs-ffi` surface should make safe by construction. The file picker path is untested.

**Viewer follow-up (M6, emulator, density 2.625):** the real viewer (`ViewSession` in `pdfrs-ffi`, Compose canvas) settles to exact-scale tiles like the desktop. With `adb shell setprop log.tag.specto DEBUG` it logs the device scale, canvas origin and page rectangle of each settled frame; the page area of an `adb exec-out screencap` was compared against `pdfrs render -p N -s <device scale>` on a 40-page text PDF:

| State | Device px/pt | Result |
|---|---|---|
| Fit width | 1.7445 (no √2 level) | pixel-identical |
| Fit width, scrolled | 1.7445 | pixel-identical |
| After double-tap zoom | 4.3613 | pixel-identical |
| Zoomed and scrolled to page 2 | 4.3613 | pixel-identical |

- The canvas origin is a whole pixel ((0, 310) here), so no fractional-offset correction is needed in Compose.
- Memory: about 40 MB native heap after opening, about 100 MB after heavy scrolling with a 96 MB tile budget, back to about 44 MB after `onTrimMemory`.
- Placeholders are evicted last rather than pinned: at up to 0.5 MB per page, pinning them all would exceed the whole budget on a long document.
- Not covered on the emulator: pinch (adb can't send multi-touch).

**On a phone (Pixel 9a, GrapheneOS, Android 17, 120 Hz, debug build):** the same sharpness check is pixel-identical at fit width (1.7445 px/pt). Pinch works by hand. Frame timing from `dumpsys gfxinfo` over a scripted run of 18 flings through the 40-page PDF:

| Per-frame path | Frame planning call | Slow frames | p50 / p90 / p95 / p99 |
|---|---|---|---|
| UniFFI (`scrollBy` + `frame()` through JNA) | 1.0–2.8 ms | 28 of 1694 (1.65%) | 8 / 12 / 14 / 19 ms |
| JNI (`Native.scrollBy` + `Native.frame` into a `ByteBuffer`) | 0.22–0.35 ms | 13 of 2147 (0.61%) | 7 / 10 / 12 / 16 ms |

- Planning a frame takes about 4 µs in Rust. Through UniFFI even a trivial call (`session.id()`) cost 200–370 µs on both the phone and the emulator, so the bindings, not the work, dominated. Everything that runs per frame (scroll, zoom, frame, finished tiles) now goes through plain JNI (`crates/pdfrs-ffi/src/hot.rs` and `jni.rs`); UniFFI stays for occasional calls. **This extends D5: UniFFI for the coarse API, JNI for pixels and for the per-frame path.**
- These are debug-build numbers. **Release build** (same phone, later code: paged mode with the thumbnail strip; 10 scripted page swipes per run, all of which turned the page):

  | Build, view | Median | 90th | 95th | 99th | Slow frames |
  |---|---|---|---|---|---|
  | Debug, with strip | 6–7 ms | 10–11 ms | 14–15 ms | 30 ms | 2.3% |
  | Debug, full screen | 7 ms | 10 ms | 10 ms | 12 ms | 0% |
  | Release, with strip (two runs) | 5 ms | 6–7 ms | 7–8 ms | 10–11 ms | 0–0.14% |
  | Release, full screen | 5 ms | 6 ms | 7 ms | 10 ms | 0% |

  The release build is not debuggable but also not shrunk (no R8), so this is the gain from Android compiling the Kotlin side properly; the Rust library is optimised in both. Memory after the run was about 350 MB PSS, as in debug (about 104 MB of it Android's GPU texture cache).
- Memory after the fling run: about 360 MB PSS, of which about 117 MB is Android's GPU texture cache and about 98 MB native heap (tile bitmaps within a 64 MB budget, plus render caches). The tile budget is now a quarter of `memoryClass` because every drawn bitmap also has a GPU copy.

## Render guards (M1)

Peak memory and time per corpus file (first 3 pages at 2×, each file in its own process): median 20 MB, p95 39 MB, p99 110 MB. The only outlier, `issue19517` (4.4 GB), is a 4.4 × 5.9 m page made of one 212-megapixel JPEG 2000 image. Most of that came from the CLI rendering the page as a single 25216 × 33612 bitmap, which the viewer never does. But even single tiles are expensive, because hayro decodes the image at a resolution that grows with the zoom, whatever the tile size:

| One 256 px tile of `issue19517` at scale | 0.05 | 0.25 | 1 | 2 |
|---|---|---|---|---|
| Time | 0.15 s | 1.4 s | 9.9 s | 8.2 s |
| Peak memory | 245 MB | 2.2 GB | 4.5 GB | 4.6 GB |

Ordinary pages stay flat: 512 px tiles of pages with 20–55 MP of images cost 8–107 MB and at most 160 ms at scales from 2 to 16 (`pdfrs render --region`, `pdfrs info` now shows image megapixels per page). So the image size only predicts trouble at the extreme end, and runtime slowness is the more general signal.

hayro can't be cancelled, so the scheduler limits the damage instead (`Limits`, see the `scheduler` module docs): expensive pages (> 100 MP of images, or a render > 500 ms) render one tile at a time, workers drop their document copy after a heavy page, and a watchdog adds workers while renders are stalled. On `issue19517` at fit width in the GTK spike (probe, 4 workers, `PDFRS_NO_GUARDS=1` for the baseline): peak memory 1.14 GB without guards, 0.66 GB with them, fully rendered in 2.0 s vs 2.35 s. At 100% the difference would be 4 × 4.5 GB against 4.5 GB; that wasn't run on this 5 GB machine.

What this doesn't fix: one tile at 100% still needs 4.5 GB and 10 s. That needs region- and resolution-aware image decoding in hayro (TASKS.md).

## Text extraction check (M2)

Question: can the text layer use hayro, or do we need our own font and `ToUnicode` handling? `spikes/text-check` implements hayro-interpret's `Device`, collects glyphs from `draw_glyph_run`, builds words with naive gap/baseline rules, and compares the words of the first pages with `pdftotext -raw` (Poppler): *recall* is the share of Poppler's words we also find (NFKC-normalized, lowercase).

hayro exposes what a text layer needs: per glyph `as_unicode()` (`ToUnicode` → Adobe Glyph List → `uniXXXX` names; for CID fonts `ToUnicode` only), `char_code()`, the placement (`props.transform * glyph.transform()`), `advance_width()` and `outline()` (1000 units per em), the run's writing mode, and marked-content callbacks.

| Documents | Recall |
|---|---|
| Everyday PDFs in `/usr/share` (manuals from FOP, pdfTeX and Doxygen, specs, CUPS forms incl. Russian) | 0.99–1.00 |
| Complex-script font specimens (Arabic, Myanmar) | 0.94–0.95 |
| pdf.js corpus, 163 files with ≥ 20 words | median 1.00, 72% ≥ 0.99, 83% ≥ 0.9 |

6% of all corpus glyphs had no Unicode. The files where *nothing* mapped while Poppler found the text fall into three groups:

- **CID TrueType without `ToUnicode`** (`ThuluthFeatures`, `issue19802`, `perf_text_cmap_mixed`): Poppler and pdf.js reverse-map glyph IDs through the embedded font's own `cmap` table. hayro doesn't, but `OutlineGlyph::font_data()` gives us the font bytes to do it ourselves.
- **Type 3 fonts without `ToUnicode`** and non-standard glyph names (`french_diacritics`, `issue918`, `perf_text_type3_*`): Poppler falls back to the character code as Latin-1 / the font's encoding.
- **Fonts *with* `ToUnicode` that hayro doesn't apply** (`complex_ttf_font`, `issue16538`): probably a hayro bug; to investigate and report upstream.

The other low scores come from the spike's naive word building (rotated text, Chinese without spaces, unusual spacing), which the real text layer replaces.

**Conclusion:** build `pdfrs-text` on hayro's `Device` interface and `as_unicode`, plus two fallbacks of our own (font `cmap` reverse lookup, char-code fallback), and report the third group upstream. No font parsing of our own beyond the `cmap` table.

## Text layer (M2)

`crates/pdfrs-text`, measured against `pdftotext -raw` at the **character** level (whitespace ignored, annotations included on both sides). Word-level comparisons turned out to be misleading: Poppler separates Japanese characters with spaces and splits letter-spaced headings into single letters, while we keep them together.

| | Recall | Order (sequence similarity) |
|---|---|---|
| Everyday PDFs (`/usr/share`) | 0.98–1.00 (words) | 0.92–1.00 |
| pdf.js corpus, 166 files with ≥ 100 characters | median 1.000; 87% ≥ 0.99 | median 1.000; 86% ≥ 0.95 |

- What's left is mostly glyphs without Unicode (the fallbacks in TASKS.md) and files whose content stream isn't in reading order (forms, flyers), where Poppler partly reorders by layout.
- Right-to-left text: most generators draw Hebrew and Arabic left to right (visual order). Text output reverses RTL words and lines drawn that way, and RTL glyphs placed right to left (logical order) don't break lines. Historic RTL scripts are left alone, as Poppler does.
- Speed: the 384-page Valgrind manual in 0.20 s (17 MB), the corpus's first three pages per file in 0.8 s. Search over whole documents doesn't need precomputed text.

## Not done yet

- Verify Preview's encryption behavior on a Mac (D9).
- PDFium reference renders and the PDFium test corpus.
- Manual check of the GTK canvas on a real HiDPI display with a touchpad.
