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
- [ ] **S** Manual check of the canvas spike on a real HiDPI display with a touchpad (pinch, fractional scale)
- [ ] **M** Android spike: Compose app + `cargo-ndk` + UniFFI, Rust renders page 1 into a `Bitmap` (D4/D5). Do this on the other machine.

## M1: Core viewer (desktop)

- [ ] **M** Render guards: per-render deadline (drop late results), memory cap for huge images and decompression bombs, from the M0 timeouts
- [ ] **S** Per-thread hayro `RenderCache` (it's `!Send` and borrows the `Pdf`); measure the speedup on text-heavy pages

- [ ] **S** `pdfrs-geom`: coordinate-space newtypes, page rotation/CropBox transform, tests
- [ ] **M** `pdfrs-source`: open file (mmap), page count, page sizes/rotation, outline, metadata, password prompt hook
- [ ] **M** `pdfrs-render`: `Renderer` trait + hayro backend, tile rendering at arbitrary scale
- [ ] **M** Render scheduler: worker pool, priorities, cancellation, LRU tile cache with memory budget
- [ ] **M** `pdfrs-view`: layout engine for Single / Continuous / Two-up / Two-up continuous (+ "cover page alone" option)
- [ ] **M** `pdfrs-view`: viewport, zoom (fit width/page, fixed), zoom buckets, scroll/pinch handling, HiDPI scale factor
- [ ] **M** Desktop: canvas widget drawing tiles, scaled placeholders while sharp tiles render
- [ ] **M** Desktop: thumbnail sidebar, outline sidebar, page number entry, keyboard navigation
- [ ] **S** Desktop: open via dialog / DnD / CLI arg, recent files, remember per-file view state
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

- [ ] **M** `pdfrs-ffi`: UniFFI surface (Session, Viewer, commands, events, `ViewHost` callback)
- [ ] **M** NDK bitmap tile path, memory budget tuned for mobile, `onTrimMemory` → cache eviction
- [ ] **M** Compose viewer: canvas, scroll/fling, pinch zoom, double-tap zoom, view modes
- [ ] **M** SAF open/save (fd-based I/O), share intent ("Open with"), recent documents
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
