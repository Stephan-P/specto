# Task list (draft)

Milestones are ordered by dependency. The desktop app is built first because iteration there is much faster. Android starts once the core API has settled (M4/M5), though the M0 spike de-risks it early.

Sizes: **S** ≤ 1 day, **M** 2–4 days, **L** 1–2 weeks.

---

## M0: Spikes & foundations

- [x] **S** Create Cargo workspace skeleton (`crates/`, `tools/`, `spikes/`)
- [x] **S** CI: GitHub Actions in `fedora:44` containers, with deps in `ci/install-deps-fedora.sh` for reuse in GitLab CI. Job `check` for the Rust code (fmt, clippy for the host and Android, tests), job `desktop` for the headless checks of the running app, side by side
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
- [x] **M** `pdfrs-source`: opening and decryption (`open_pdf`, `OpenError`, now shared with `pdfrs-render`), metadata with text-string decoding (UTF-16BE, UTF-8, PDFDocEncoding), outline, links (destinations, URIs, named actions), named destinations via `/Dests` and the `/Names` tree. Every walk is bounded (cycles, depth, item caps). Corpus: 1,077 files, no panics, 0.2 s; 153 with an outline, 49 with links. `pdfrs info --outline` prints both. Files of 64 MB and up are memory-mapped (`PdfData`), smaller ones read: a 100 MB file opens with 5 MB of memory instead of 105.
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
- [x] **S** Headless checks with pointer input: a virtual pointer and keyboard through mutter's RemoteDesktop D-Bus API (relative motion, with the window maximized so window and screen coordinates agree), and the clipboard read back through it. Link clicks, all the selection gestures, copy, select all, autoscroll, a rotated page
- [x] **S** Links (desktop): pointer cursor over links; a click follows in-document destinations (`Viewer::go_to_pdf_position`) and named actions, and opens `http`, `https` and `mailto` URIs with `GtkUriLauncher` (other schemes are refused with a toast)
- [x] **S** Android: outline and links through `pdfrs-ffi` (flattened outline with targets; in-document, web and mail links)
- [ ] **S** Back and forward after a jump (a link, an outline entry, "go to page", later a search result): `pdfrs-view` keeps the position left behind. The cheap answer to "look at the footnote, then carry on reading"
- [ ] **M** Trim margins ("fit to content"): a view option that lays pages out by the box of what is drawn on them instead of the CropBox. Matters most on a phone, where white margins take up to a third of the width. A crop in the view only (on `ViewPage`); cropping pages for good would be an M3 command. Open: where the box comes from (the text layer's quads are cheap but miss figures, and scans entirely; candidates are the bounds of everything drawn, collected through hayro's `Device` the way `pdfrs-text` collects glyphs, or the non-background pixels of the placeholder tile, which is rendered anyway), and how to keep it steady from page to page (one box per document, or one each for left and right pages, rather than one per page)
- [ ] **S** Document properties dialog (desktop): the metadata from `pdfrs-source` (which now has the dates and `SourceDocument::version`), plus file size, page count and page size, and how the file is protected (D9). Android has the dialog already, without the protection. Fonts as a follow-up: they need a walk over every page's resources, in the background
- [ ] **S** Bookmarks set by the reader, listed with the contents. Decide where they live: kept by the app per document (nothing to save, but they stay on the device), or written as outline entries (they travel with the file, but need M3's save). Kept by the app, they need a way to recognise a document that survives saving it, such as the trailer's `/ID`; so will the reading positions, once there is editing
- [ ] **M** Link preview (desktop): hovering over an in-document link shows a popover with the area around its target, for footnotes and citations in papers. Works where destinations carry a position (LaTeX's do); for a bare page there is only its top to show. After back and forward, which serve the same need

## M2: Text

- [x] **S** Spike: hayro's glyph → Unicode mapping is good enough to build on (recall 0.99–1.00 on everyday PDFs vs `pdftotext`; M0-FINDINGS.md, "Text extraction check")
- [x] **L** `pdfrs-text`: glyph extraction via hayro's `Device` (quads in PDF space, rotation and vertical writing), words and lines in content order, RTL (Hebrew/Arabic) in logical order for text output, `text_range`, `char_at`, `word_of`, `line_of`. Character-level match with `pdftotext -raw` on the corpus: median recall and order 1.000, 87% of files ≥ 0.99 recall; everyday PDFs 0.98–1.00. 384 pages in 0.2 s
- [ ] **M** Reading order by layout (columns, blocks) for files whose content stream isn't in reading order (about 5% of the corpus, mostly forms and flyers)
- [ ] **M** `pdfrs-text` fallbacks for glyphs without Unicode: reverse lookup in the embedded TrueType `cmap` (CID TrueType without `ToUnicode`); character code as Latin-1 / font encoding (Type 3)
- [ ] **S** Investigate and report upstream: hayro ignores existing `ToUnicode` tables in `complex_ttf_font.pdf` and `issue16538.pdf`
- [x] **M** Text selection (drag, double-click word, triple-click line) + copy. `pdfrs-text`: `position_at` (nearest position between characters, also in margins), word and line ranges, highlight quads per line. `pdfrs-view`: `Selection` (anchor and focus as page and character, across pages; character, word or line granularity), `nearest_pdf_point`/`pdf_to_view`. Desktop: drawn in the accent colour, I-beam cursor, Shift+click, autoscroll near the edges, *Primary*+C/A, Escape, primary selection, Edit menu on macOS
- [x] **M** Search (desktop): `pdfrs_text::SearchText` (per page: folded text, about 5 bytes per character, mapping back to characters; case and accents ignored, typographic quotes made plain, lines joined with a space and hyphenated line ends joined) and `Query`; `pdfrs_view::SearchResults` (document order, current match, wrap-around) and `Viewer::reveal`. Desktop: search bar (*Primary*+F, Enter / Shift+Enter, *Primary*+G / Shift+*Primary*+G, Escape), "3 of 17" status, all matches in yellow and the current one in orange, pages indexed in a background thread from the current page on. Not yet: right-to-left text in visual order, matches spanning pages, options (whole words, match case)
- [x] **S** Text-layer cache per page (desktop): full text pages extracted on the main thread when first needed (0.1–0.25 s for a whole 300-page manual) and kept for the 64 most recently used pages; search uses the compact `SearchText` of every page, built in the background

## M3: Page editing

- [x] **M** `pdfrs-doc`: `EditDoc` with `PageRef`s and undo/redo. Page edits keep a snapshot of the page list instead of inverse commands; a version number tracks "modified" relative to the last save
- [x] **S** Edits in the model: delete pages, move pages, rotate pages, insert pages
- [x] **S** Desktop: rotate left and right (current page or spread), undo and redo with the edit's name in the main menu, "Edited" in the title; text, search and links keyed by page of the file so they survive edits. Headless checks `rotate_and_undo`, `select_after_rotating`
- [x] **S** Desktop: Delete Pages (Delete or Backspace on the canvas or in the sidebar, main menu, macOS Edit menu): the selected thumbnails if several, else the current page; never the last page. Headless check `delete_pages`
- [x] **S** Desktop: move pages by dragging thumbnails (the selection, if the thumbnail dragged is in it), with a drop line and autoscroll; the view stays on the page it showed. Headless check `move_pages` (drops through a test hook: Mutter refuses about one in three drags from the virtual pointer)
- [x] **M** Desktop: documents with pages from several files (a `Source` per file: scheduler document, parsed file, geometries, links, search text); Insert Pages from File after the current page or spread, with a password prompt; text, links and search keyed by file and page; the search indexer covers inserted pages. Headless check `insert_file`
- [x] **L** `pdfrs-write`: full rewrite save: flat page tree, rotation, deleted pages removed with what only they used, pages copied in from other sources, object and xref streams, encryption kept. Streams repaired from hayro where lopdf reads them wrong; the result is read back and checked page by page before it's used. Corpus: 1,066 of 1,077 files identical when rendered, 11 refused, none damaged
- [ ] **M** `pdfrs-write`: save the 11 corpus files it refuses (lopdf can't read damaged cross-reference tables or page trees that hayro recovers; one has an encryption dictionary lopdf re-encrypts wrongly), e.g. by building the objects from hayro's reading of the file
- [ ] **S** `pdfrs-write`: page labels after deleting or moving pages: write labels that keep each remaining page's label instead of dropping them; the same in the viewer
- [ ] **S** `pdfrs-write`: deduplicate resources shared between sources (fonts embedded in both of two merged files)
- [x] **S** Desktop: several thumbnails selected (Shift+click, *Primary*+click, *Primary*+A in the sidebar); a selection of several stays while the view moves; rotating turns the selection. Headless check `thumbnail_selection`
- [ ] **M** Desktop: page organizer: drop a PDF on the sidebar to insert it there; drag pages between windows (copy)
- [x] **S** Desktop: Save (*Primary*+S) and Save As (Shift+*Primary*+S) in the main menu and the macOS File menu; written in the background to a temporary file renamed into place; edits made meanwhile stay unsaved; "Save Changes?" when closing or quitting with changes. Headless checks `save`, `close_prompt`
- [ ] **S** "Export selected pages" (with the page organizer)
- [x] **S** Property test: random edit sequence followed by undo-all restores the original state, and redo-all every state after it
- [x] **M** Round-trip harness: `crates/pdfrs-write/tests/write.rs` writes edited fixtures (inherited attributes, labels, outline, links, encryption, two sources), reopens them, compares every page's render with its source page, and runs `qpdf --check` (installed in CI). The corpus run is an ignored test
- [ ] **S** File changed on disk while open: notice it (file monitor), and never save over a file that changed since it was read without asking; the unedited case can simply reload

## M4: Markup

- [ ] **M** `pdfrs-annot`: annotation model, import existing annotations from source, preserve unknown kinds
- [ ] **L** Appearance-stream generation: Highlight/Underline/StrikeOut, Ink, Square/Circle/Line, FreeText, Text (note icon)
- [ ] **M** Overlay rendering of annotations (separate from page tiles)
- [ ] **M** Tools in `pdfrs-view`: text-markup (snaps to text quads), ink (pressure, smoothing), shapes, text box, note
- [ ] **M** Select / move / resize / recolor / delete annotations, properties popover
- [ ] **M** `pdfrs-write`: incremental-update save path (used when only annotations/forms changed)
- [ ] **S** Annotation sidebar/list (jump to annotation)
- [ ] **M** Signature and image stamps: `AnnotKind::Stamp` in the annotation model (decide before the model settles), whose appearance is an image or, for a drawn signature, vector paths; image embedding in `pdfrs-write` (JPEG as it is, anything else deflated with a soft mask for transparency); saved signatures kept in the app's data directory, never in a document until placed; placed, moved and resized like other annotations. This is a picture of a signature, not a digital signature, and the UI shouldn't suggest otherwise
- [ ] **M** Creating a signature on the desktop: drawn with the pointer in a signature box (relative motion, smoothed), with a stylus or touchscreen (`GtkGestureStylus`, shared with the ink tool), or imported from an image with the paper background made transparent. A drawn signature is kept as paths, so it stays sharp. No raw trackpad access: Linux offers apps no absolute finger positions
- [ ] **S** Android: draw a signature by finger or stylus; a way to bring it to the desktop (export/import as an image)

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
- [x] **S** UI tests in CI: the job `android-ui` runs them on an Android 15 emulator (x86-64 runner) for pushes to `main` and on request. Green in a phone and a tablet shape (`pixel_7`, `pixel_tablet`).
- [x] **S** Release build measured on the phone (`./gradlew :app:assembleRelease`, signed with the debug key, no code shrinking): page turns at median 5 ms, 99th percentile 10–11 ms, with or without the strip; numbers in M0-FINDINGS.md. A build for distribution still needs its own key and code shrinking with keep rules for the UniFFI bindings.
- [x] **S** CI: `cargo clippy -p pdfrs-ffi --all-targets --target aarch64-linux-android` lints the Android-only JNI code too (check only, no NDK needed; clean with `-D warnings`)
- [x] **S** Top bar: pages button (top left) opening a thumbnail grid, ⋮ menu (top right) with "Open…" and "Go to page…"; password prompt for encrypted files. Thumbnails are the pages' placeholder tiles, requested after the page view's own tiles.
- [x] **L** Paged mode as the phone default (UX decision, 2026-11-14): swipe left/right, one page at a time, each page fitted to the screen on its own. `pdfrs-view` got swipe paging for the paged layout modes (`Settings::swipe`, off by default, so the desktop is unchanged): all spreads side by side in viewport-sized slots, one zoom per spread, `snap_delta` to decide where a drag comes to rest, `scroll_in_page_by` for flings that stay on a zoomed page. Continuous vertical scrolling is an option in the ⋮ menu; the choice is not persisted. Checked on the emulator; the feel still needs judging on a phone.
- [x] **S** `Viewer::current_page` in continuous modes is the last page once scrolled to the very end (before, the page above could keep the larger share of the viewport forever)
- [x] **S** Printed page numbers (`/PageLabels` via `pdfrs-source`): page indicator ("iii (3 / 240)"), page grid, Contents, and "Go to page…", which takes a printed number first and a position otherwise
- [x] **S** Paged mode: turning the page returns to the fitted view. In swipe mode only the current spread is zoomed and its neighbours wait fitted, so the next page slides in at its final size (`pdfrs-view`).
- [ ] **S** Investigate what Preview does with the zoom when turning a page while zoomed in (iOS, and macOS in single-page mode), and decide whether we keep returning to the fitted view (the current behaviour) or carry the zoom over to the next page (what we did before; `pdfrs-view` would need both as a setting)
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
- [x] **S** Share: "Share…" in the ⋮ menu offers the open document through the system's share sheet. The receiving app reads the document where it is (the app passes its read permission on); where that isn't possible (a plain file path, or a provider that doesn't allow it) it gets a copy from the app's cache, served by a `FileProvider`. Both routes checked by hand against the system's Print; the UI tests cover the copy.
- [x] **S** Opening through a file descriptor instead of reading the file into the app's memory: `PdfFile.fromFd(pfd.detachFd())` and `ViewSession.openFile` (mapped from 64 MB, pipes read). The file's bytes no longer pass through the Java heap, a password retry reuses the `PdfFile`, and documents are still recognised by the same fingerprint (`PdfFile.read` for both ends of the file)
- [ ] **M** Save (in the ⋮ menu), through the storage access framework
- [ ] **M** Page organizer: drag to reorder, delete and rotate in the thumbnail grid (with M3)
- [ ] **M** Markup tools with stylus support (pressure, palm rejection, finger-scrolls-while-pen-draws)
- [ ] **M** Forms with native `TextField` overlay + IME
- [x] **M** Text selection with Android's handles and copy: a long press selects a word (and, moving on, further words), a handle at each end moves it by characters, dragging to the edge scrolls on in continuous mode, and the system's floating text menu offers Copy and Select all. `pdfrs-ffi` uses `pdfrs-view`'s `Selection` and reports the highlights and handle positions in each frame (the frame layout has a kind per highlight, for search matches later). Its text-page cache is a second copy of the desktop app's `TextCache`: one to share when the search session moves into a crate
- [ ] **S** Share the selected text (the system's text menu only knows Copy and Select all, so this needs a menu of our own)
- [x] **M** Search on Android: a magnifying glass in the top bar turns it into a search field; matches follow the typing, are highlighted (the current one differently) and counted ("3 of 17"), with arrows to the next and previous one. Through `pdfrs-ffi`: `search`, `search_step`, `search_status`, matches as highlights in each frame, and `ViewListener::search_changed` while the document is still being read. Matching and the list of matches are the shared `SearchText` and `SearchResults`; the session around them (the per-page search text, the background indexer, the search in progress) is in `pdfrs-ffi/src/search.rs`, a second copy of what the desktop app has in `document.rs`: one to share
- [ ] **M** F-Droid: check that the Gradle dependencies are all free software, make the build reproducible, write the metadata
- [ ] **S** Back after a jump: the system's Back returns to where a link or outline entry was followed from (with M1's back and forward)
- [ ] **S** Trim margins: a switch in the ⋮ menu, remembered per document (with M1's)
- [x] **S** Document properties: "Properties" in the ⋮ menu shows the file name, what the document states (title, author, subject, keywords, programs, dates), page count, page size by name ("210 × 297 mm (A4)"), file size and PDF version, through `ViewSession::info`. Not yet: how the file is protected, and fonts
- [ ] **S** Bookmarks in the Pages/Contents view (with M1's, through `pdfrs-ffi`)
- [x] **M** Top bar: a view button next to ⋮ (2026-11-14). Its icon pictures the current layout; its menu chooses one of single page, continuous, and on wide windows two pages and two pages continuous, with "First page alone" below, then "Fit page" or "Fit width" (`ViewSession::set_fit_width`; each layout starts with its own fit, and the choice is saved with the document). ⋮ keeps the rest: Open…, Share…, Go to page…, Properties, Settings, Close. Fitted to the width, a page taller than the view scrolls before Page Down, Space and the arrows turn it. The magnifying glass is left for search; "Trim margins" will join the view menu
- [x] **S** Settings screen, from the ⋮ menu: app-wide switches, kept across launches (`SharedPreferences`), apart from what is remembered per document
- [x] **S** Keep the screen on while reading: a switch in Settings, off by default; for ten minutes after the last touch or key, so a phone left lying on the table still goes dark
- [x] **S** Volume keys move through the document like Page Down and Page Up (in every layout, not only paged): a switch in Settings, off by default, since it takes the volume keys from someone listening to music while reading

## M7: Polish & hardening

- [ ] **M** Printing (desktop; Android via `PrintManager`)
- [x] **S** Encrypted PDFs: password prompt on open (desktop + Android)
- [ ] **L** Encrypted save: keep the original security handler on incremental + full-rewrite saves, and encrypt merged-in objects with the target key (D9)
- [ ] **M** Permission flags: honor owner-password restrictions (edit/annotate/print), with an "unlock" prompt for the owner password. A setting in both apps turns honouring off (D9); what a restriction blocks should say so and point there
- [ ] **S** Document security dialog (like Preview's, from the File menu): add / change / remove user and owner passwords (AES-256), saved with the document
- [ ] **M** Fuzzing (`cargo-fuzz`) for parser and save path; fix crashes
- [ ] **M** Performance pass: profile big/complex docs, startup time, memory
- [ ] **M** Accessibility: keyboard-only operation, screen-reader labels for chrome
- [ ] **S** Dark mode (UI chrome; optional inverted page rendering)
- [ ] **S** Packaging: Flatpak (desktop), signed APK/AAB (Android)
- [ ] **S** Decide the app's name and application ID before anything is published (Flathub ID, Android package name; both are painful to change later)
- [ ] **S** Desktop integration files: `.desktop` file with the PDF MIME type (so "Open with" lists the app), icon, AppStream metainfo with screenshots (all required by Flathub)
- [ ] **S–M** macOS app bundle: file association, Dock icon, bundled GTK
- [ ] **S** `cargo-deny` in CI: security advisories and licence compatibility with MIT OR Apache-2.0
- [ ] **M** Performance regression checks in CI: tile render times, frame times and peak memory on a few fixed documents (the gtk-canvas spike had a benchmark mode to start from: `spikes/gtk-canvas` at commit f32a26a, the last one with it), compared against stored numbers with a tolerance
- [ ] **S** Preferences: default layout and zoom for documents opened for the first time (desktop: a preferences window; Android: also persist paged or continuous)
- [x] **M** Desktop: large files without reading them into memory: mapped from 64 MB (`PdfData`). Re-opening safely after a save is part of the save task
- [ ] **L** Decide whether page content is exposed to screen readers (the text layer through AT-SPI and TalkBack, reading order, tagged PDF structure), and if so, plan it
- [ ] **S** Crash handling: a panic hook that writes a log file the user can attach to a bug report (no telemetry); desktop and Android
- [ ] **M** Mapped files that shrink: a file of 64 MB or more is mapped (`PdfData`), and if another program truncates it in place while it is open, reading the missing part kills the app with `SIGBUS` (on Android a sync or download app could do that; a panic hook doesn't see it). Decide whether and how to guard against it. Ideas: notice the change first and stop reading (the file monitor of "File changed on disk while open", or checking size and modification time before each render); map only files nobody else can write (on Android, copy large files from other apps into the app's cache first, at the cost of the space); a `SIGBUS` handler that turns the fault into a failed render; or reading by position instead of mapping, which hayro's one-slice input doesn't allow today
- [ ] **S–M** Touch on Linux (2-in-1 laptops, phones): drag to scroll, long-press to select, a layout for narrow windows; touch input in the headless checks

## M8: Internationalization

Each app uses its platform's own mechanism, and the core crates carry no user-facing text. The first two tasks are worth doing early, before M3–M5 add many more strings.

- [ ] **S** Desktop: move the UI strings into gettext (`gettext-rs`, catalogues in `apps/desktop/po`), English only at first. Sentences built with `format!` ("({} of {})", "Links of this kind aren't opened: …") become whole translatable strings with placeholders; counts use `ngettext`
- [ ] **S** Android: move the literal `Text("…")` strings into `strings.xml` (`stringResource`, `plurals` for counts)
- [ ] **S** Core: no user-facing text from the crates. Frontends map error variants (`OpenError`, the `pdfrs-ffi` errors) to their own translated messages instead of showing the `thiserror` text
- [ ] **S** Numbers through the locale: page counts, the zoom percentage
- [ ] **S** Desktop: build and install the `.mo` files (also in the Flatpak), `bindtextdomain` at startup, a `.pot` target; check that it works on macOS with Homebrew's gettext
- [ ] **S** A first translation (German?) in both apps, to prove the pipeline; a headless check that starts the desktop app under another locale
- [ ] **S** Right-to-left UI languages: check the chrome of both apps under an RTL locale (GTK and Compose mirror on their own; the sidebar side, the thumbnail strip and icons with a direction need a look). The page canvas does not mirror
- [ ] **M** Binding direction in `pdfrs-view`: one property for both the order of pages in a two-page spread and the swipe direction. Defaults to the document's `/ViewerPreferences /Direction` (read in `pdfrs-source`), independent of the UI language; a "Right to left" toggle per document, saved with the view state, since many right-to-left PDFs don't set it. Through `pdfrs-ffi` for Android
- [ ] **S** Decide how translations are contributed (plain `.po`/`strings.xml` in the repo, or a hosted tool such as Weblate)
