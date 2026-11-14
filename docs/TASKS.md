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
- [x] **S** Verify Preview's encryption behavior on a Mac: saving keeps the password; user and owner passwords can be added, changed or removed in a File-menu settings panel (D9)
- [ ] **S** Check how Preview handles owner-password restrictions (no open password, but editing or printing restricted) and update D9
- [x] **M** GTK4 canvas spike: custom widget, tiles as `GdkMemoryTexture`, continuous scroll + zoom, about 55 fps (D3, see M0-FINDINGS.md)
- [x] **S** Manual check of the canvas spike on real hardware: works on macOS (GTK), but HiDPI output looks off
- [x] **M** Android spike: Compose app + `cargo-ndk` + UniFFI, Rust renders page 1 into a `Bitmap`; tile pixels need a JNI path, not UniFFI (D4/D5, see M0-FINDINGS.md)

## M1: Core viewer (desktop)

- [x] **M** Render guards (`pdfrs_render::scheduler::Limits`): pages with > 100 MP of images (checked at open) or a render > 500 ms are *expensive* and render one tile at a time; workers drop their document copy after a heavy page (frees decoded images); a watchdog starts up to 4 extra workers while renders are stalled > 1 s and other work waits. Giant JPEG 2000 page at fit width: peak memory 1.14 → 0.66 GB at the same speed (M0-FINDINGS.md)
- [x] **S** Desktop: a spinner (`AdwSpinnerPaintable`) over visible pages that are expensive (`Scheduler::is_expensive`) and not sharp yet (`PageDraw::complete`). The paintable only exists while a spinner shows, so there are no idle redraws (0 ms CPU in 3 s once the page is done)
- [ ] **S** Android: the same spinner (needs `PageDraw::complete` and `is_expensive` through `pdfrs-ffi`)
- [ ] **M** Upstream (hayro): decode images only for the region and resolution being drawn. One tile of a 212 MP JPEG 2000 page still costs 4.5 GB and 10 s at 100%, which no scheduling can fix
- [x] **S** Keep hayro's `RenderCache` across renders (`self_cell`), reset per worker after 32 pages. Measured 1.4–6× faster tiles.

- [x] **S** `pdfrs-geom`: typed coordinate spaces (euclid), `PageGeometry` (CropBox ∩ MediaBox, `/Rotate`, extra rotation). Matches hayro on every corpus page (`cargo test -p pdfrs-render -- --ignored`).
- [x] **M** `pdfrs-source`: opening and decryption (`open_pdf`, `OpenError`, now shared with `pdfrs-render`), metadata with text-string decoding (UTF-16BE, UTF-8, PDFDocEncoding), outline, links (destinations, URIs, named actions), named destinations via `/Dests` and the `/Names` tree. Every walk is bounded (cycles, depth, item caps). Corpus: 1,077 files, no panics, 0.2 s; 153 with an outline, 49 with links. `pdfrs info --outline` prints both. (No mmap: files are read into memory, as both apps already do.)
- [x] **S** Page labels (`pdfrs_source::PageLabels`: all five styles, prefixes, start numbers, number trees split over kids; reverse lookup). Desktop: the page entry shows the label with "(3 of 120)" and accepts labels ("iv") or numbers; thumbnail captions show labels. Documents whose labels are just 1, 2, 3 are treated as unlabelled
- [x] **S** Android: page labels through `pdfrs-ffi` (`ViewSession::page_labels`; the indicator, grid and Contents show them, and "Go to page…" accepts them)
- [x] **M** `pdfrs-render`: `Document` trait + hayro backend, tile rendering at arbitrary scale (done during M0)
- [x] **M** Render scheduler (`pdfrs_render::scheduler`): worker pool, multiple documents, a priority queue that's replaced every frame, panics isolated per tile. `TileCache<T>` (`pdfrs_render::tile`) has an LRU byte budget, protects tiles used this frame, and is generic over the frontend's texture type.
- [x] **S** HiDPI fix, part 1: zoom levels round *up* (tiles are only downsampled), and pages and tiles are placed on whole device pixels (gtk-canvas spike)
- [x] **S** HiDPI, part 2: the blur was `append_scaled_texture`, which GSK renders at logical resolution on a HiDPI surface. With plain `append_texture`, zoom 1.0 at scale 2 is pixel-identical to hayro's own render (see M0-FINDINGS.md). Not yet confirmed by eye on the Mac's screen, or on a HiDPI Linux surface.
- [x] **M** HiDPI, part 3: settled views use tiles at the exact device scale, drawn 1:1 (`TileScale` in `TileKey`, settling in `Viewer::frame(now, …)`, `begin/end_zoom_gesture`); √2 levels remain for zooming and as fallback. Probe at fit width: pixel-identical to `pdfrs render` at the device scale on Linux scales 1.25, 1.5 and 2 (was edge contrast 99 vs 117 with levels). No frame-rate cost.
- [x] **S** HiDPI, part 4 (found by the part 3 probe on Linux): at fractional scales a non-maximized window puts the canvas on a fractional device pixel (25 px CSD shadow × 1.25 = 31.25), blurring every tile (edge contrast 80.7). Both canvases now translate to snap their origin to a whole device pixel. `apps/desktop` also switched to `append_texture`.
- [x] **S** Follow-ups from the Android review: `TileCache` evicts placeholders last instead of pinning them (`with_evict_last`; pinning every page of a 1,000-page document would take ~500 MB) and is now `Send`; the scheduler no longer re-renders tiles that are finished but not yet collected; the viewer omits the placeholder layer under fully covered pages; desktop thumbnails use `append_texture`, and the debug screenshot renders at device scale
- [ ] **S** Confirm parts 3 and 4 by eye on the Mac (scale 2) and on a real fractional-scale Linux display
- [x] **M** `pdfrs-view`: layout for single page / continuous / two-page / two-page continuous, plus "cover page alone"
- [x] **M** `pdfrs-view`: viewport, zoom (fit width/page, fixed) anchored at the pointer, scrolling, paging, hit-testing, device-pixel-snapped `Frame` with placeholder and neighbouring-level fallback tiles and prioritized requests. The gtk-canvas spike runs on it at 59.5 fps (headless, quiet machine).
- [x] **M** Desktop (`apps/desktop`, binary `pdfrs-gtk`): canvas widget drawing `pdfrs-view` frames, implementing `GtkScrollable` (native scrollbars, kinetic scrolling), with ctrl+scroll and pinch zoom
- [x] **M** Desktop: thumbnail sidebar (`GtkListView`, shares placeholder tiles with the canvas, follows the current page), page number entry, layout and zoom menus, keyboard navigation and shortcuts
- [x] **S** Desktop: outline sidebar ("Contents" tab next to Thumbnails, shown when the document has an outline; `GtkTreeListModel` with expanders; `win.sidebar-page` action)
- [x] **S** Desktop: open via dialog, drag-and-drop or command line (one window per document), password prompt, start page
- [x] **S** Desktop: recent files (GTK `RecentManager`, so they also show up in GNOME; listed on the start page) and per-file view state: page and position, layout, zoom, sidebar (`pdfrs_view::ViewState`, stored in `$XDG_STATE_HOME/pdfrs/view-state.ini`, last 200 files). Ctrl+Q closes windows one by one so each saves.
- [x] **S** Desktop: one scheduler for the whole app. `Scheduler::client` gives each view its own queue, results and wake-up callback; workers serve clients round-robin. Three windows now share 4 workers instead of starting 4 each.
- [x] **S** Headless GUI smoke test: `tools/headless/run.sh tools/headless/smoke.sh file.pdf out/` (screenshots via a debug-only screenshot action)
- [x] **S** Headless desktop checks in CI: `tools/headless/check.py` drives the app over D-Bus (window actions plus debug-only `debug-state`, `debug-screenshot`, `debug-go-to`), with generated PDFs whose pages each have their own colour, so screenshots show which page is on screen. Opening, start page, broken files, navigation, page labels, layouts, zoom and its limits, sidebar, saved state, two windows; at scales 1, 1.25 and 2.
- [ ] **S** Headless checks with pointer input (link clicks, drags, later text selection): needs synthetic input, e.g. mutter's RemoteDesktop D-Bus API
- [x] **S** Links (desktop): pointer cursor over links; a click follows in-document destinations (`Viewer::go_to_pdf_position`) and named actions, and opens `http`, `https` and `mailto` URIs with `GtkUriLauncher` (other schemes are refused with a toast)
- [x] **S** Android: outline and links through `pdfrs-ffi` (flattened outline with targets; in-document, web and mail links)

## M2: Text

- [x] **S** Spike: hayro's glyph → Unicode mapping is good enough to build on (recall 0.99–1.00 on everyday PDFs vs `pdftotext`; M0-FINDINGS.md, "Text extraction check")
- [x] **L** `pdfrs-text`: glyph extraction via hayro's `Device` (quads in PDF space, rotation and vertical writing), words and lines in content order, RTL (Hebrew/Arabic) in logical order for text output, `text_range`, `char_at`, `word_of`, `line_of`. Character-level match with `pdftotext -raw` on the corpus: median recall and order 1.000, 87% of files ≥ 0.99 recall; everyday PDFs 0.98–1.00. 384 pages in 0.2 s
- [ ] **M** Reading order by layout (columns, blocks) for files whose content stream isn't in reading order (about 5% of the corpus, mostly forms and flyers)
- [ ] **M** `pdfrs-text` fallbacks for glyphs without Unicode: reverse lookup in the embedded TrueType `cmap` (CID TrueType without `ToUnicode`); character code as Latin-1 / font encoding (Type 3)
- [ ] **S** Investigate and report upstream: hayro ignores existing `ToUnicode` tables in `complex_ttf_font.pdf` and `issue16538.pdf`
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
- [x] **M** Compose viewer: canvas, scroll with fling, pinch zoom, double-tap zoom, page indicator, rotation without reload. Settles to exact-scale tiles after zooming; pixel-identical to `pdfrs render` on the emulator (M0-FINDINGS.md). Works on a Pixel 9a, including pinch. The per-frame path uses JNI instead of UniFFI (M0-FINDINGS.md).
- [x] **S** Kotlin unit tests (`./gradlew :app:testDebugUnitTest` in `apps/android`): the frame decoder against `crates/pdfrs-ffi/fixtures/frame.bin`, which a Rust test keeps current, so the two sides can't drift apart; the document store; the outline's collapse logic. CI builds the app and runs them (job `android`).
- [x] **S** UI tests, first slice (`ANDROID_SERIAL=emulator-5554 ./gradlew :app:connectedDebugAndroidTest` in `apps/android`; the run reinstalls the app, hence the serial): opening, page turns by swipe, a slow short drag returning and a short flick turning, the page grid, "Go to page…", full screen, and continuing where the reader left off. Compose's test clock makes the gestures deterministic, unlike adb swipes. Test PDFs are fixtures in `app/src/androidTest/assets`, written by `make-fixtures.py`.
- [x] **M** UI tests for most of `apps/android/FEATURES.md`: 25 tests in `ReaderTest` (paging, keys, strip, full screen, sidebar, two pages, rotation, what is remembered) and `DocumentTest` (printed page numbers, Contents, links, password), passing on the phone emulator (Android 17) and the tablet emulator (Android 16). A third fixture, `locked.pdf`, is RC4-encrypted by `make-fixtures.py`.
- [ ] **S** Run the UI tests in CI on a stable system image (a job of its own, on pushes to `main` rather than every change)
- [x] **S** Release build measured on the phone (`./gradlew :app:assembleRelease`, signed with the debug key, no code shrinking): page turns at median 5 ms, 99th percentile 10–11 ms, with or without the strip; numbers in M0-FINDINGS.md. A build for distribution still needs its own key and code shrinking with keep rules for the UniFFI bindings.
- [x] **S** CI: `cargo clippy -p pdfrs-ffi --all-targets --target aarch64-linux-android` lints the Android-only JNI code too (check only, no NDK needed; clean with `-D warnings`)
- [x] **S** Top bar: pages button (top left) opening a thumbnail grid, ⋮ menu (top right) with "Open…" and "Go to page…"; password prompt for encrypted files. Thumbnails are the pages' placeholder tiles, requested after the page view's own tiles.
- [x] **L** Paged mode as the phone default (UX decision, 2026-11-14): swipe left/right, one page at a time, each page fitted to the screen on its own. `pdfrs-view` got swipe paging for the paged layout modes (`Settings::swipe`, off by default, so the desktop is unchanged): all spreads side by side in viewport-sized slots, one zoom per spread, `snap_delta` to decide where a drag comes to rest, `scroll_in_page_by` for flings that stay on a zoomed page. Continuous vertical scrolling is an option in the ⋮ menu; the choice is not persisted. Checked on the emulator; the feel still needs judging on a phone.
- [x] **S** `Viewer::current_page` in continuous modes is the last page once scrolled to the very end (before, the page above could keep the larger share of the viewport forever)
- [x] **S** Printed page numbers (`/PageLabels` via `pdfrs-source`): page indicator ("iii (3 / 240)"), page grid, Contents, and "Go to page…", which takes a printed number first and a position otherwise
- [x] **S** Paged mode: turning the page returns to the fitted view. In swipe mode only the current spread is zoomed and its neighbours wait fitted, so the next page slides in at its final size (`pdfrs-view`).
- [ ] **S** Paged mode follow-up: rubber-band at the first and last page
- [x] **M** Thumbnail strip along the bottom: scrollable, the current page highlighted and kept in the middle (it follows once the page view rests), tap to jump. Bars read their own `currentPage` and thumbnail state rather than the per-frame `ViewFrame`, which brought page turns with the strip back to the frame times without it on a Pixel 9a (median 6–7 ms, debug build).
- [x] **S** Full screen: a tap on the page (not on a link) hides the top bar, the strip and the status bar; a tap or Back brings them back. The navigation bar stays, since hiding it triggers Android's "viewing full screen" explanation.
- [x] **S** Strip: scrolling it flips through the document (the page passing the middle is shown)
- [ ] **S** Strip follow-up: the occasional slow frame when it loads new thumbnails after a page turn (99th percentile 30 ms against 12 ms in full screen)
- [x] **S** Outline: the top-left button opens "Pages" and, for documents with an outline, "Contents" as a second tab (a collapsible tree from `pdfrs-source`, the reader's position highlighted); the tabs can be swiped, and the last one used is reopened
- [x] **S** Links: a tap follows internal links and named actions (`Viewer::go_to_pdf_position`, which now stays on the target page in swipe mode); external links show their address and open only after confirmation, and only http, https and mailto
- [x] **M** Tablet layout, minimal: windows at least 840 dp wide show Pages and Contents in a sidebar beside the document, open by default and folded away with the top-left button; narrower ones (tablet portrait, split screen) keep the phone layout. The strip is hidden while the sidebar is open, and full screen hides the sidebar. UI tests run on the phone and tablet emulators (Android 17 and 16).
- [x] **M** Two-page view on wide windows: "Two pages" and "First page alone" in the ⋮ menu, with swiping (pairs turn together) and with continuous scrolling; saved per document, and shown one page at a time while the window is narrow. `pdfrs-view` needed no change, only a test for swiping through spreads.
- [x] **S** Keyboard, as on the desktop: arrows, Page Up/Down, Space, Backspace, Home and End turn pages or scroll, Ctrl with +, − and 0 zooms. The page view takes the focus when a document opens.
- [x] **S** Tablet polish: the page indicator names both pages of a pair ("4–5 / 12"), the sidebar shows one column of thumbnails, and whether it was open is remembered per document
- [ ] **S** Tablet follow-up: the page number in the top bar, as on the desktop
- [x] **S** Open from the system file picker and via "Open with" (`ACTION_VIEW`)
- [x] **M** Reading position and recent documents: page, offset, mode and zoom are saved per document (recognised by a fingerprint of its content, since the same file arrives under different URIs) when the app stops or the document is closed, and restored on open; the start screen lists the last 20 documents that can be reopened (those with a persistable read permission, i.e. picked in the app). "Close" in the ⋮ menu returns to the start screen.
- [ ] **M** SAF fd-based I/O instead of reading the file into memory, save and share (in the ⋮ menu)
- [ ] **M** Page organizer: drag to reorder, delete and rotate in the thumbnail grid (with M3)
- [ ] **M** Markup tools with stylus support (pressure, palm rejection, finger-scrolls-while-pen-draws)
- [ ] **M** Forms with native `TextField` overlay + IME
- [ ] **S** Text selection with Android selection handles + copy/share

## M7: Polish & hardening

- [ ] **M** Printing (desktop; Android via `PrintManager`)
- [x] **S** Encrypted PDFs: password prompt on open (desktop + Android)
- [ ] **L** Encrypted save: keep the original security handler on incremental + full-rewrite saves, and encrypt merged-in objects with the target key (D9)
- [ ] **M** Permission flags: honor owner-password restrictions (edit/annotate/print), with an "unlock" prompt for the owner password
- [ ] **S** Document security dialog (like Preview's, from the File menu): add / change / remove user and owner passwords (AES-256), saved with the document
- [ ] **M** Fuzzing (`cargo-fuzz`) for parser and save path; fix crashes
- [ ] **M** Performance pass: profile big/complex docs, startup time, memory
- [ ] **M** Accessibility: keyboard-only operation, screen-reader labels for chrome
- [ ] **S** Dark mode (UI chrome; optional inverted page rendering)
- [ ] **S** Packaging: Flatpak (desktop), signed APK/AAB (Android)
