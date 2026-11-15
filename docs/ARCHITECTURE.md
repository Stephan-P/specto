# Architecture (draft)

## Guiding principles

1. **All logic lives in Rust; the frontends are thin.** Anything that isn't "draw pixels, receive input, show native widgets" belongs in a shared crate. That includes layout for the viewing modes, hit-testing, tool state machines, and undo/redo. A second frontend should then cost weeks, not months.
2. **Don't mutate the source PDF while editing.** An open document is an *edit model* layered over immutable source files. Bytes are only produced on save. This keeps rendering fast (no re-parsing after every edit) and makes undo trivial.
3. **Every third-party PDF engine sits behind a trait.** The pure-Rust ecosystem is young. We want to be able to swap or compare backends, such as using PDFium as a reference in tests.
4. **Coordinates are always explicit.** PDF user space (points, y-up, per-page rotation and CropBox) ≠ page space ≠ document-layout space ≠ device pixels. Each has its own type.

## Crate layout

```
specto/
├── crates/
│   ├── specto-geom      # typed coordinate spaces (euclid), PageGeometry: CropBox, /Rotate
│   ├── specto-source    # opening/decryption (PdfData: read or mapped), metadata, outline, links (hayro-syntax)
│   ├── specto-render    # Document trait, hayro backend, TileCache<T>, render scheduler
│   ├── specto-text      # text layer via hayro's Device: char quads (PDF space), words, lines
│   ├── specto-doc       # edit model: page list, annotation edits, undo/redo (later form values)
│   ├── specto-annot     # annotation types, reading a file's annotations, shapes and appearance streams
│   ├── specto-forms     # (M5) AcroForm model, field values, appearance regeneration
│   ├── specto-write     # saving: full rewrite (lopdf), checked against hayro; incremental update later
│   ├── specto-view      # UI-agnostic viewer: layout modes, viewport, zoom, hit-test, tools
│   ├── specto-session   # an open document as both apps hold it: files, edited pages, viewer, text, selection, search, saving
│   └── specto-ffi       # Android bindings: UniFFI (Kotlin), plus JNI for the per-frame path
├── apps/
│   ├── desktop/        # GTK4 + libadwaita (gtk4-rs), Linux (also runs on macOS)
│   └── android/        # Gradle project, Kotlin + Jetpack Compose, loads libspecto_ffi.so
└── tools/
    ├── specto-cli       # the `specto` dev tool: info, render to PNG, text, corpus runs
    └── headless/       # the desktop app in a headless compositor: end-to-end checks (CI)
```

Crates marked (M5) don't exist yet. Planned dependency direction: `apps (desktop, specto-ffi) → specto-session → specto-view / specto-doc → {specto-annot, specto-forms, specto-text, specto-render} → specto-source → specto-geom`. `specto-write` depends on `specto-doc` + `specto-source`. `specto-doc` holds the page list and the annotation edits, and depends on `specto-annot` and `specto-geom`; `specto-session` turns its pages into the viewer's `ViewPage`s.

### File data

`specto-source::PdfData` holds a file's bytes, shared between threads (hayro needs the whole file as one slice). Files under 64 MB are read into memory; larger ones are memory-mapped, so they open at once and only the parts in use take memory. A mapped file must not be truncated while open (reading the missing part is a `SIGBUS`), so saving always writes a new file and renames it into place, never over the open file. On Android, `PdfFile.fromFd` takes a file descriptor from the Storage Access Framework; a pipe (as some document providers hand out) is read instead.

The desktop app watches the open file (a GIO file monitor) and keeps a fingerprint of it, its entity tag and size, from when it was read or last saved. A change that doesn't match the fingerprint came from another program: an unedited document is reloaded in place, an edited one gets a banner offering to reload, and saving checks the fingerprint once more and asks before overwriting. Another program truncating a mapped file in place, instead of replacing it, can still crash the app (see TASKS.md).

## Data model

```
Session (specto-session; the desktop's Document and the Android ViewSession each hold one)
 ├── sources                          # immutable, parsed PDFs, one per opened/merged file
 └── EditDoc                          # specto-doc
      ├── pages: Vec<PageRef>         # ordered; this is what the user sees
      │     PageRef { source: SourceId, page: u32, rotation: Rotation }
      ├── undo, redo: Vec<Step>       # earlier page lists, each with the Edit that changed it
      ├── annots: AnnotEdits          # what was done to annotations: deleted, changed, added
      └── form                        # (M5) field values keyed by fully-qualified field name
```

Why this shape:

- **Delete / reorder / rotate / merge** are just edits to `pages`. Inserting pages of another PDF adds a source and splices its `PageRef`s in. Nothing gets re-parsed or re-rendered except what becomes newly visible. In `specto-session`, each source has its own scheduler document, parsed file, page geometries, links and search text; the first is the file opened, whose outline the document shows. Each page keeps the page label it has in its file (pages of files without labels are numbered by position). Pages copied from another document (dragging between windows) come as a bundle of the files' data, shared rather than copied, and each page with its rotation. Sources stay open while undo can bring their pages back. Saving passes the first source plus those pages still come from to `specto-write`.
- **Rendering** a `PageRef` renders the original page from its source *without* its annotations. The viewer then draws annotations from the edit model, so new or edited annotations look the same as existing ones and stay editable. Existing annotations are imported into the model on load (keeping their original appearance stream when we don't modify them).
- **Undo/redo** for pages keeps a copy of the page list from before each edit (12 bytes a page, so 120 KB a step for 10,000 pages; 100 steps are kept). Undo can't drift out of step with redo, as hand-written inverse commands can. Each step records what the edit was (`Edit::Rotate { pages }`, …), which the frontends name in their menus ("Undo Rotate Page"). Annotations and form values are larger and will get commands of their own.
- **Modified** compares a version number of the current page list with the one last saved, so undoing back to the saved state counts as unmodified.
- **Positions and pages of the file**: the viewer, selections and search results work with positions in the edited list; what doesn't change with editing (text layers, search text, links, outline destinations) is keyed by the file and its page, and mapped to its current position when needed. A link's destination is looked up in the file the link is in.

### Annotations

```
Annotation { kind: AnnotKind, rect, color, opacity, line_width, author, contents, modified,
             name, flags, origin: Option<Origin>, appearance: Appearance }
AnnotKind  = TextMarkup{style, quads} | Ink{strokes} | Square{fill} | Circle{fill}
           | Line{from, to} | Stamp(strokes | image) | Other{subtype}
Origin     = { index in the page's /Annots, object number }      # None for a new one
Appearance = Original | Missing | Generated
AnnotId    = File{source, page, index} | New(n)
```

Annotations are plain data in the coordinates of their page of the file (`specto-annot`), so rotating or moving the page doesn't touch them. `specto_annot::import` reads the markup a page has when it is first asked for; links, form fields, popups and media aren't markup and are left out (saving leaves them alone). Kinds without parameters here (text boxes, notes, stamps of other programs, polygons) are `Other`: shown as the file draws them, deletable, not changeable.

An `EditDoc` doesn't hold the annotations, only what was done to them relative to the files: which are deleted or changed (by source, page and index) and the new ones. That is small whatever the files contain, costs nothing for pages nobody looked at, is what saving needs, and can be kept per undo step (shared until it changes), so page and annotation edits are undone in the order they were made. Annotations belong to a page of a file, not to a position: they follow the page, come back with it when its deletion is undone, and copies of one page share them.

**Drawing.** `specto_annot::shapes` turns an annotation into a few shapes (filled quads, lines). `specto_annot::appearance` writes those shapes as the appearance stream (`/AP /N`) saved with the annotation, because many viewers (hayro, Preview, pdf.js in some modes) show only that. Who draws an annotation on screen depends on whether the file has such a look for it:

- The annotations of a file are part of the page tiles: hayro draws them from their appearance streams.
- The apps draw the rest over the page, from the same shapes (`Session::overlay`; a highlight darkens what is beneath through the toolkit's multiply blend): annotations made in the session, and ones the file gives no appearance for. A test saves a file and renders it with hayro to check that the two agree.

hayro draws all of a page's annotations or none; its public interface has no way to draw or leave out just one. So when an annotation of a file is deleted, its page is rendered from a **variant**: a copy of that one page without the annotation, written by `specto-write` exactly as saving would and opened as a scheduler document of its own. Only the tiles come from the copy; text, links, search and saving still use the file. Undo closes the copy and goes back to the file. A page whose copy can't be written keeps its annotation, and the app says so.

The same happens when text markup of a file is recoloured: the copy leaves it out, and the app draws it from then on. Thumbnails draw the same overlay, scaled. Printing, which renders pages from their files on a thread of its own, renders a page with edited annotations from such a copy too, this one written with all the changes in it. Pages copied to another window take the changes to their annotations along.

Saving names a changed or deleted annotation of a file by its `Origin`, and checks that lopdf finds the same object there before touching it.

### Forms

AcroForm only. `specto-forms` builds the field tree (inheritance of `/FT`, `/Ff`, `/DA`, `/V`, ...), maps widgets to pages, and regenerates widget appearance streams when values change. Tab order follows `/Tabs` or falls back to widget order.

While a field is focused, the frontend overlays a **native text input** (GTK entry / Compose `TextField`) at the widget's device rect. Otherwise the widget is drawn from its appearance. That gives us IME, autocorrect, and accessibility for free.

## Rendering pipeline

```
specto-view (visible pages, zoom, device scale)
   │  requests TileKey { page, zoom_bucket, scale_factor, tile_x, tile_y, content_version }
   ▼
Scheduler (one per app; a Client per view with its own priority queue, replaced every frame; clients served round-robin; guards for expensive pages)
   │  dedicated worker threads (extra ones start when renders stall)
   ▼
Document trait ──► HayroDocument           | PDFium (planned, for reference renders)
   │  RGBA8 premultiplied tile
   ▼
TileCache (LRU, memory budget; lower budget on Android)
   │
   ▼
Frontend turns the tile into a GdkTexture (GTK) / Bitmap (Android)
```

- **HiDPI:** the device scale factor (`1.0`, `1.25`, `2.0` ...) gets multiplied into the render scale, and tiles are rendered at physical pixels. A settled view is drawn from tiles at the *exact* device scale, 1:1, so it is pixel-identical to a direct render at that scale (measured on macOS at scale 2 and on Linux at 1.25, 1.5 and 2, see M0-FINDINGS.md).
- **Zoom:** while zooming, tiles come from √2 levels (rendered at or above the device scale and downsampled), so continuous zooming reuses a few cached levels instead of rendering every intermediate zoom. Once the zoom has been still for `settle_delay` (250 ms) and no pinch is in progress, the viewer switches to exact-scale tiles, with level tiles as fallback until they arrive. This is the "blurry → sharp" behavior Preview has.
- **Toolkit pitfalls (GTK):** draw tiles with `append_texture`, not `append_scaled_texture` (GSK renders the latter at logical resolution on HiDPI surfaces), and translate the canvas so its own origin lands on a whole device pixel of the surface (CSD shadows put it at e.g. 31.25 px at scale 1.25).
- **Tiling:** 512×512 physical-pixel tiles, so a 400% zoom on a poster PDF doesn't allocate a 100 MP bitmap.
- **Thumbnails** draw the viewer's placeholder tiles (one small tile per page) from the shared cache, so each page is rendered once for both the sidebar and the canvas. On the desktop the sidebar is also the page organizer: a multi-selection, thumbnails dragged within it (the move is made once the drag has ended), and PDFs dropped on it to insert.
- **Annotations** made in the session are drawn as an overlay over the page tiles, from a few shapes per annotation, so editing them never re-renders page content. The file's own annotations are part of the tiles; deleting one renders its page from a copy without it (see "Annotations" above). Form widgets (M5) are still to come.

## Text layer

`specto-text` implements the interpreter's device callback (e.g. hayro-interpret's `Device`) to collect glyphs with unicode mapping and quads. It then groups them into words and lines, in content-stream order (layout-based reading order for the files that need it is still to do). This powers:

- text selection (hit-test → glyph index → selection range → quads)
- copy to clipboard
- search (with match quads for highlighting results)
- text-markup annotations (highlight/underline/strike need quads)

## specto-session: the open document

What both apps mean by an open document (`crates/specto-session`, `Session`): the files its pages come from (the one opened, and any that pages were inserted from), the `EditDoc`, the `Viewer`, the text layer with its cache, the selection, the search with its background indexer, links, page labels, and what saving needs (`SaveJob`). Edits go through the session, which then updates everything that holds page positions: the viewer's pages, the labels (each page keeps the one it has in its file), the selection (dropped) and the search (run again). What doesn't change with editing (text, search text, links) is cached by page of the file, not by position.

No toolkit, no pixels and no threads of its own apart from the search indexer. A frontend adds what differs between them:

- **Tiles.** The desktop keeps `GdkTexture`s in a `TileCache`; Android keeps ids of Kotlin `Bitmap`s in its `TileBook`. Each asks the viewer for a frame and requests the tiles itself.
- **The scheduler.** The desktop shares one between its windows; Android has one per document. So the session doesn't own it: `Session::open`, `add_source` and `close` take it as an argument.
- **Getting called back.** The indexer hands each page's search text to a closure the frontend supplies (`start_indexing`), on the indexer's thread; the frontend gets it to wherever it holds the session and calls `indexed`. The desktop sends it through a channel to the main loop; Android locks the session where it is.
- **Threads.** The desktop holds its session in an `Rc<RefCell<…>>` on the main thread; Android holds it in a mutex, since Kotlin calls from several threads. A `Session` is `Send`.

Tests: `crates/specto-session/tests/session.rs` (text, labels, links and search across edits, pages of a second file, saving and reopening, starting and stopping the indexer).

## specto-view: the shared viewer core

UI-agnostic state that both frontends drive. **Implemented in M1** (`crates/specto-view`) for layout, zoom, scrolling and tile planning. Text selection (`selection.rs`: a `Selection` between two page-and-character positions, growing by characters, words or lines, with highlight quads in view coordinates) came in M2; text pages are supplied through the `TextPages` trait, so the viewer doesn't decide how they are cached (`specto-session` does: by page of the file, the least recently used going first). Search results (`search.rs`: `SearchResults`, matches in document order with a current one, filled page by page as a background indexer delivers `specto_text::SearchText`s) and `Viewer::reveal` came with it. Tools come in M4.

```rust
struct ViewPage { doc: DocId, page: u32, geometry: PageGeometry, rotation: Rotation }

impl Viewer {
    fn new(pages: Vec<ViewPage>, settings: Settings) -> Self;
    fn set_pages(&mut self, pages: Vec<ViewPage>);       // after reorder/delete/merge/rotate
    fn set_viewport(&mut self, size: ViewSize, scale_factor: f64);
    fn set_layout_mode(&mut self, LayoutMode);           // SinglePage | Continuous | TwoPage | TwoPageContinuous
    fn set_cover_alone(&mut self, bool);
    fn set_zoom_mode(&mut self, ZoomMode);               // Fixed | FitWidth | FitPage
    fn zoom_by(&mut self, factor: f64, anchor: ViewPoint);   // pinch, ctrl+scroll
    fn scroll_by(&mut self, ViewVector);
    fn go_to_page / next_page / previous_page / current_page;
    fn hit_test(&self, ViewPoint) -> Option<(usize, PagePoint)>;
    fn page_to_view(&self, usize, PagePoint) -> Option<ViewPoint>;
    fn begin_zoom_gesture / end_zoom_gesture;            // pinch: never settled while active
    fn frame(&mut self, now: Instant, cached: impl Fn(&TileKey) -> bool) -> Frame;
    fn view_state(&self) -> ViewState;                   // page + offset, layout, zoom: to remember per file
    fn restore_view_state(&mut self, &ViewState);
}

struct Frame {
    pages: Vec<PageDraw>,      // per visible page: device rect + layers to draw in order
    requests: Vec<TileKey>,    // for Scheduler::request, highest priority first
    complete: bool,            // everything visible is sharp
    settles_at: Option<Instant>, // redraw then: the view switches to exact-scale tiles
}
```

Design points:

- **Margins and gaps are constant in logical pixels**, like Preview, so layout is recomputed on zoom changes (microseconds). Zoom anchoring tracks a point within a spread in PDF points, plus any offset into the margin in pixels, so the content under the pointer stays put.
- **Everything the frontend draws is in device pixels**, snapped so page origins are whole pixels. A tile rendered at exactly the device scale has `exact = true` and is drawn 1:1 without filtering.
- **Layers per page**: a one-tile placeholder (the largest level where the whole page fits in 512 px), then cached tiles from neighbouring zoom levels wherever sharp tiles are missing, then sharp tiles: √2-level tiles while zooming, exact-scale tiles once settled. Zooming shows the previous level's tiles instead of the blurry placeholder.
- **Settling** is decided in the viewer, driven by the frontend's clock: `frame(now, …)` notes when the device scale last changed, and `Frame::settles_at` tells the frontend when to redraw. Exact tiles are never requested during a pinch or right after a ctrl+scroll step, so intermediate zooms don't churn the cache.
- **Request order**: visible placeholders, visible sharp tiles, neighbouring pages' placeholders, then a prefetch band of half a viewport above and below.
- `TileKey` is plain data: `TileKey::to_raw()` is `[u32; 7]` (doc, page, rotation, content, scale, tx, ty), where the scale is a `TileScale` in fixed point (2^-20 px/pt). √2 levels are particular values (`TileScale::from_level`). Page rectangles are sized from the same quantized scale, so exact tiles always map 1:1. The Android `Bitmap` cache can live on the Kotlin side, keyed by these numbers.

- **Swipe paging** (`Settings::swipe`, used on Android): in the paged modes all spreads are laid out side by side, each centred in a slot at least as large as the viewport and fitted to it on its own, so pages of different sizes each fill the screen. Scrolling can slide to the neighbouring spread, and `Viewer::snap_delta` tells the frontend where the view should come to rest after a drag (a flick or more than half way turns the page). Vertical scrolling and the zoom anchor belong to the current spread; the neighbours' tiles are prepared at their own exact scale, and turning a page does not restart the settle delay.

Still to come: a `ViewHost`-style callback for redraws and native text inputs (form fields, M5), and pointer/key handling for tools (ink smoothing, highlight snapping to text, moving and resizing annotations, M4). Those are implemented once here, and frontends only translate native events.

## Saving

| Change type | Strategy |
|---|---|
| Only annotations / form values changed | **Incremental update** (append to original file). Fast, and preserves existing digital signatures' byte ranges. Not implemented yet: annotations go through the full rewrite too. |
| Pages rotated / deleted / reordered / merged | **Full rewrite**: copy reachable objects from each source, write a fresh xref and object streams, and drop unreferenced objects. (Implemented; deduplicating resources shared between sources isn't yet.) |

Annotations are always saved as standard PDF annotations, never flattened into page content, as Preview does (D8).

**Encrypted documents** (D9): the security handler, key and permissions of the document being saved are kept. Incremental updates encrypt the newly appended objects with the existing key. A full rewrite re-encrypts with the same password and permissions. Merged pages from other (decrypted) sources get encrypted with the target document's key. "Export…" can add, change or remove a password.

Always write to a temp file and rename atomically (`specto_write::replace_file`, which keeps permissions and follows symbolic links). The renamed-over file stays readable through an open mapping, so the document doesn't need re-opening after a save: its edits stay relative to the original data, and saving again writes the same result.

How the full rewrite works (`specto_write::write`):

- lopdf loads (and decrypts) the main source. Inherited page attributes are pushed down onto each page, the page tree becomes one flat list of the pages kept, and each page's `/Rotate` gets the user's rotation added. Deleted pages are removed; references to them (outline entries, links) dangle, which readers treat as null. Page labels stay as they are while every page is still there, in order; otherwise new ones are written in which each page keeps the label it had in its file. A page used twice gets its own page dictionary and copies of its annotations. Pages from other sources are copied in, renumbered.
- lopdf loses some data that hayro recovers: the contents of streams without a valid `/Length`, and of streams it decrypts wrongly (per-stream `/Crypt` filters). Every stream's bytes are compared with hayro's reading of the same object, and hayro's win, so the saved file has what the app showed.
- After writing, the result is read back with hayro and each page compared with its source page (content, boxes, resource names). Any difference fails the save with `WriteError::Unsupported` instead of writing a damaged file.
- On the corpus (`cargo test --release -p specto-write -- --ignored`), 1,066 of 1,077 files are rewritten with every page rendering identically, unchanged and with pages reversed and rotated. The other 11 are refused, nearly all damaged files that lopdf can't read; none is written damaged.

## Frontend integration

### Desktop

GTK4 + libadwaita via gtk4-rs. Linux first; it also runs on macOS with Homebrew's GTK. Links `specto-view` directly as a Rust crate. A custom `GtkWidget` subclass (implementing `GtkScrollable`) draws each frame's `PageDraw`s in `snapshot()`, turning tiles into `GdkMemoryTexture`s. The surrounding chrome uses stock widgets: `AdwOverlaySplitView` with a `GtkListView` of thumbnails and a `GtkTreeListModel` outline, `GtkFileDialog`, and `GtkGestureZoom` for touchpad pinch. Printing uses `GtkPrintOperation`, with pages rendered at 300 dpi on a thread of their own (deferred drawing). Planned: `GtkGestureStylus` for pen input, and Flatpak packaging (file access through the portal).

### Android

- Kotlin + Jetpack Compose for UI chrome.
- `specto-ffi` exposes a coarse API via **UniFFI** (`ViewSession`: viewport, zoom, layout, outline, labels, links, selection, search, page edits with undo, saving to a file descriptor, and a copy without password protection for the system's print service, which prints PDFs itself). Underneath is the same `specto-session` as on the desktop. Avoid chatty per-glyph calls across the FFI boundary.
- The per-frame path (frame layout, tile copies) uses hand-written JNI instead, because UniFFI's per-call overhead was too high (M0-FINDINGS.md).
- Tiles are copied from Rust into an `android.graphics.Bitmap` through the NDK bitmap API, and Compose draws them on a `Canvas`.
- File access via the Storage Access Framework. Today Kotlin reads the file into memory. Ready on the Rust side: `PdfFile.fromFd(pfd.detachFd())` reads or maps the file, and `ViewSession.openFile` opens it (reusable for a password retry). Saving will go through the same mechanism, writing to a temporary file first (see File data).
- Build with `cargo-ndk` for `arm64-v8a` (+ `x86_64` for the emulator).
- Texts are Android string resources (`res/values/strings.xml`); the Rust side contributes none, except the reasons in error messages.
- A crash log in the app's private storage, which Settings shows and nothing sends: Kotlin records exceptions nothing caught, and `specto-ffi` installs a panic hook that appends to the same file (`log_panics_to`).

## Testing strategy

- **Corpus:** pdf.js test PDFs, PDFium test corpus, and our own fixtures (forms, rotated pages, odd CropBoxes, encrypted files).
- **Render regression:** `specto corpus` renders the corpus and compares with Poppler (corpus/README.md). Planned: PDFium as the reference, since Poppler is often the one that's wrong.
- **Desktop end to end:** `tools/headless/check.py` drives the app in a headless compositor over D-Bus, at several scales, in CI.
- **Android:** Kotlin unit tests in CI; Compose UI tests on an emulator.
- **Round-trip:** open → op → save → reopen with our parser *and* with an external validator (qpdf `--check`, and pdf.js/PDFium render).
- **Fuzzing:** `cargo-fuzz` on parsing and on the save path.
- **Property tests:** random edit sequences, then undoing everything, return the original page list, and redoing walks forward through the same states (`crates/specto-doc/tests/edit.rs`).
