# Task list (draft)

Milestones are ordered by dependency. The desktop app is built first because iteration there is much faster. Android starts once the core API has settled (M4/M5), though the M0 spike de-risks it early.

Sizes: **S** ≤ 1 day, **M** 2–4 days, **L** 1–2 weeks.

---

## M0: Spikes & foundations

- [x] **S** Create Cargo workspace skeleton (`crates/`, `tools/`, `spikes/`)
- [x] **S** CI (fmt, clippy, test): GitHub Actions in a `fedora:44` container, with deps in `ci/install-deps-fedora.sh` for reuse in GitLab CI
- [ ] **S** GitLab CI equivalent, if the repo moves there
- [x] **S** License decision (D6): MIT OR Apache-2.0
- [x] **S** Test corpus, part 1: pdf.js test PDFs (`corpus/fetch.sh`)
- [ ] **M** Test corpus, part 2: PDFium test files + hand-picked fixtures (forms, rotated pages, CropBox ≠ MediaBox, huge pages, scanned, encrypted)
- [x] **M** hayro spike: `pdfrs corpus` over the corpus, compared with Poppler, recording failures and timing (D1, see M0-FINDINGS.md)
- [ ] **M** PDFium reference renders for `pdfrs corpus`. Poppler turned out to be wrong in about half of the worst cases.
- [x] **S** hayro spike: render page content without annotations (`Content::PageOnly`, tested)
- [x] **M** lopdf spike: delete, merge, reorder, incremental highlight (D2). Validated with hayro and Poppler; qpdf isn't installed.
- [x] **S** lopdf spike: AES-256 encrypt, encrypted incremental save, decrypt → edit → re-encrypt (D9)
- [ ] **S** Verify Preview's encryption behavior on a Mac (user password, owner-password restrictions, Export → Encrypt) and update D9
- [x] **M** GTK4 canvas spike: custom widget, tiles as `GdkMemoryTexture`, continuous scroll + zoom, about 55 fps (D3, see M0-FINDINGS.md)
- [x] **S** Manual check of the canvas spike on real hardware: works on macOS (GTK), but HiDPI output looks off
- [x] **M** Android spike: Compose app + `cargo-ndk` + UniFFI, Rust renders page 1 into a `Bitmap`; tile pixels need a JNI path, not UniFFI (D4/D5, see M0-FINDINGS.md)

## M1: Core viewer (desktop)

- [ ] **M** Render guards. hayro interprets the whole page for every tile, so a huge or complex page (e.g. `issue19517`, 41 s at 2×) blocks a worker. Needed: a deadline after which the page shows "too complex", spare workers so one stuck page can't starve the rest, and a memory cap for huge images.
- [x] **S** Keep hayro's `RenderCache` across renders (`self_cell`), reset per worker after 32 pages. Measured 1.4–6× faster tiles.

- [x] **S** `pdfrs-geom`: typed coordinate spaces (euclid), `PageGeometry` (CropBox ∩ MediaBox, `/Rotate`, extra rotation). Matches hayro on every corpus page (`cargo test -p pdfrs-render -- --ignored`).
- [ ] **M** `pdfrs-source`: open file (mmap), page count, page sizes/rotation, outline, metadata, password prompt hook
- [x] **M** `pdfrs-render`: `Document` trait + hayro backend, tile rendering at arbitrary scale (done during M0)
- [x] **M** Render scheduler (`pdfrs_render::scheduler`): worker pool, multiple documents, a priority queue that's replaced every frame, panics isolated per tile. `TileCache<T>` (`pdfrs_render::tile`) has an LRU byte budget, protects tiles used this frame, and is generic over the frontend's texture type.
- [x] **S** HiDPI fix, part 1: zoom levels round *up* (tiles are only downsampled), and pages and tiles are placed on whole device pixels (gtk-canvas spike)
- [x] **S** HiDPI, part 2: the blur was `append_scaled_texture`, which GSK renders at logical resolution on a HiDPI surface. With plain `append_texture`, zoom 1.0 at scale 2 is pixel-identical to hayro's own render (see M0-FINDINGS.md). Not yet confirmed by eye on the Mac's screen, or on a HiDPI Linux surface.
- [x] **M** HiDPI, part 3: settled views use tiles at the exact device scale, drawn 1:1 (`TileScale` in `TileKey`, settling in `Viewer::frame(now, …)`, `begin/end_zoom_gesture`); √2 levels remain for zooming and as fallback. Probe at fit width: pixel-identical to `pdfrs render` at the device scale on Linux scales 1.25, 1.5 and 2 (was edge contrast 99 vs 117 with levels). No frame-rate cost.
- [x] **S** HiDPI, part 4 (found by the part 3 probe on Linux): at fractional scales a non-maximized window puts the canvas on a fractional device pixel (25 px CSD shadow × 1.25 = 31.25), blurring every tile (edge contrast 80.7). Both canvases now translate to snap their origin to a whole device pixel. `apps/desktop` also switched to `append_texture`.
- [ ] **S** Confirm parts 3 and 4 by eye on the Mac (scale 2) and on a real fractional-scale Linux display
- [x] **M** `pdfrs-view`: layout for single page / continuous / two-page / two-page continuous, plus "cover page alone"
- [x] **M** `pdfrs-view`: viewport, zoom (fit width/page, fixed) anchored at the pointer, scrolling, paging, hit-testing, device-pixel-snapped `Frame` with placeholder and neighbouring-level fallback tiles and prioritized requests. The gtk-canvas spike runs on it at 59.5 fps (headless, quiet machine).
- [x] **M** Desktop (`apps/desktop`, binary `pdfrs-gtk`): canvas widget drawing `pdfrs-view` frames, implementing `GtkScrollable` (native scrollbars, kinetic scrolling), with ctrl+scroll and pinch zoom
- [x] **M** Desktop: thumbnail sidebar (`GtkListView`, shares placeholder tiles with the canvas, follows the current page), page number entry, layout and zoom menus, keyboard navigation and shortcuts
- [ ] **S** Desktop: outline (table of contents) sidebar, which needs `pdfrs-source`
- [x] **S** Desktop: open via dialog, drag-and-drop or command line (one window per document), password prompt, start page
- [ ] **S** Desktop: recent files, remember per-file view state (page, zoom, layout)
- [ ] **S** Desktop: one shared scheduler per app instead of per window (needs per-document request groups in `Scheduler`)
- [x] **S** Headless GUI smoke test: `tools/headless/run.sh tools/headless/smoke.sh file.pdf out/` (screenshots via a debug-only `app.screenshot` action). Could run in CI.
- [ ] **S** Links: internal (goto) and external URI annotations clickable

## M2: Text

- [ ] **L** `pdfrs-text`: glyph extraction via interpreter device, ToUnicode handling, word/line grouping, reading order
- [ ] **M** Text selection (drag, double-click word, triple-click line) + copy
- [ ] **M** Search: find bar, incremental results across pages (background), highlight matches, next/prev
- [ ] **S** Text-layer cache per page (lazy, evictable)

## M3: Page editing

- [ ] **M** `pdfrs-doc`: `EditDoc` with `PageRef`s, `Command` trait, undo/redo stack
- [ ] **S** Commands: delete pages, move pages, rotate pages
- [ ] **M** Commands: insert pages from another PDF / merge (multi-source sessions)
- [ ] **L** `pdfrs-write`: full rewrite save (object copying across sources, resource dedup, unreferenced-object GC, xref/object streams)
- [ ] **M** Desktop: page organizer (multi-select thumbnails, drag to reorder, drop PDF to insert, delete key)
- [ ] **S** Save / Save As / "Export selected pages", atomic write, dirty-state tracking + close prompt
- [ ] **S** Property test: random command sequence followed by undo-all restores the original state

## M4: Markup

- [ ] **M** `pdfrs-annot`: annotation model, import existing annotations from source, preserve unknown kinds
- [ ] **L** Appearance-stream generation: Highlight/Underline/StrikeOut, Ink, Square/Circle/Line, FreeText, Text (note icon)
- [ ] **M** Overlay rendering of annotations (separate from page tiles)
- [ ] **M** Tools in `pdfrs-view`: text-markup (snaps to text quads), ink (pressure, smoothing), shapes, text box, note
- [ ] **M** Select / move / resize / recolor / delete annotations, properties popover
- [ ] **M** `pdfrs-write`: incremental-update save path (used when only annotations/forms changed)
- [ ] **S** Annotation sidebar/list (jump to annotation)

## M5: Forms

- [ ] **M** `pdfrs-forms`: AcroForm field tree, inheritance, widget→page mapping, flags (read-only, required, multiline, comb, password)
- [ ] **M** Field interaction: text (native input overlay), checkbox, radio groups, combo/list box
- [ ] **L** Widget appearance regeneration (`/DA` parsing, font selection, fallback font embedding for non-Latin text)
- [ ] **S** Tab order, field highlighting toggle, reset form
- [ ] **S** Save filled forms (incremental); optional "flatten" export

## M6: Android

- [x] **M** `pdfrs-ffi`: `ViewSession` over `pdfrs-view` and the scheduler (viewport, scroll, zoom, layout and zoom modes, frames, `tiles_ready` callback). Tiles cross the FFI as opaque ids, so `TileKey` can change without touching Kotlin.
- [ ] **M** `pdfrs-ffi`: editing commands and events, `ViewHost`-style callbacks (with M3–M5)
- [x] **M** Tile path: Kotlin owns one `Bitmap` per tile and fills it through JNI; the cache bookkeeping (LRU, byte budget from `memoryClass`) lives in Rust and reports evictions; `onTrimMemory` keeps only what is on screen
- [x] **M** Compose viewer: canvas, scroll with fling, pinch zoom, double-tap zoom, page indicator, rotation without reload. Settles to exact-scale tiles after zooming; pixel-identical to `pdfrs render` on the emulator (M0-FINDINGS.md). Pinch is untested: it needs a real touch screen.
- [ ] **S** Compose viewer: switch view modes (the session already supports them), jump to page, scrollbar
- [x] **S** Open from the system file picker and via "Open with" (`ACTION_VIEW`)
- [ ] **M** SAF fd-based I/O instead of reading the file into memory, save, recent documents, password prompt
- [ ] **M** Thumbnails/page organizer (LazyVerticalGrid + drag reorder)
- [ ] **M** Markup tools with stylus support (pressure, palm rejection, finger-scrolls-while-pen-draws)
- [ ] **M** Forms with native `TextField` overlay + IME
- [ ] **S** Text selection with Android selection handles + copy/share

## M7: Polish & hardening

- [ ] **M** Printing (desktop; Android via `PrintManager`)
- [ ] **S** Encrypted PDFs: password prompt on open (desktop + Android)
- [ ] **L** Encrypted save: keep the original security handler on incremental + full-rewrite saves, and encrypt merged-in objects with the target key (D9)
- [ ] **M** Permission flags: honor owner-password restrictions (edit/annotate/print), with an "unlock" prompt for the owner password
- [ ] **S** Export dialog: add / change / remove password (AES-256)
- [ ] **M** Fuzzing (`cargo-fuzz`) for parser and save path; fix crashes
- [ ] **M** Performance pass: profile big/complex docs, startup time, memory
- [ ] **M** Accessibility: keyboard-only operation, screen-reader labels for chrome
- [ ] **S** Dark mode (UI chrome; optional inverted page rendering)
- [ ] **S** Packaging: Flatpak (desktop), signed APK/AAB (Android)
