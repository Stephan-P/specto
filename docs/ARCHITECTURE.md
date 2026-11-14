# Architecture (draft)

## Guiding principles

1. **All logic lives in Rust; the frontends are thin.** Anything that isn't "draw pixels, receive input, show native widgets" belongs in a shared crate. That includes layout for the viewing modes, hit-testing, tool state machines, and undo/redo. A second frontend should then cost weeks, not months.
2. **Don't mutate the source PDF while editing.** An open document is an *edit model* layered over immutable source files. Bytes are only produced on save. This keeps rendering fast (no re-parsing after every edit) and makes undo trivial.
3. **Every third-party PDF engine sits behind a trait.** The pure-Rust ecosystem is young. We want to be able to swap or compare backends, such as using PDFium as a reference in tests.
4. **Coordinates are always explicit.** PDF user space (points, y-up, per-page rotation and CropBox) ≠ page space ≠ document-layout space ≠ device pixels. Each has its own type.

## Crate layout

```
pdfrs/
├── crates/
│   ├── pdfrs-geom      # typed coordinate spaces (euclid), PageGeometry: CropBox, /Rotate
│   ├── pdfrs-source    # opening/decryption, metadata, outline, links (hayro-syntax)
│   ├── pdfrs-render    # Document trait, hayro backend, TileCache<T>, render scheduler
│   ├── pdfrs-text      # text layer via hayro's Device: char quads (PDF space), words, lines
│   ├── pdfrs-doc       # (M3) edit model: page list, annotations, form values, undo/redo, commands
│   ├── pdfrs-annot     # (M4) annotation types + appearance-stream generation
│   ├── pdfrs-forms     # (M5) AcroForm model, field values, appearance regeneration
│   ├── pdfrs-write     # (M3) serialization: full rewrite + incremental update
│   ├── pdfrs-view      # UI-agnostic viewer: layout modes, viewport, zoom, hit-test, tools
│   └── pdfrs-ffi       # Android bindings: UniFFI (Kotlin), plus JNI for the per-frame path
├── apps/
│   ├── desktop/        # GTK4 + libadwaita (gtk4-rs), Linux (also runs on macOS)
│   └── android/        # Gradle project, Kotlin + Jetpack Compose, loads libpdfrs_ffi.so
└── tools/
    ├── pdfrs-cli       # the `pdfrs` dev tool: info, render to PNG, text, corpus runs
    └── headless/       # the desktop app in a headless compositor: end-to-end checks (CI)
```

Crates marked (M3)–(M5) don't exist yet. Dependency direction: `apps → pdfrs-view / pdfrs-ffi → pdfrs-doc → {pdfrs-annot, pdfrs-forms, pdfrs-text, pdfrs-render} → pdfrs-source → pdfrs-geom`. `pdfrs-write` depends on `pdfrs-doc` + `pdfrs-source`.

## Data model

```
Session
 ├── sources: Vec<SourceDoc>          # immutable, parsed PDFs (Arc), one per opened/merged file
 └── document: EditDoc
      ├── pages: Vec<PageRef>         # ordered; this is what the user sees
      │     PageRef { source: SourceId, index: u32, extra_rotation: Rotation,
      │               annotations: Vec<AnnotId>, ... }
      ├── annotations: SlotMap<AnnotId, Annotation>   # existing (imported) + new
      ├── form: FormState             # field values keyed by fully-qualified field name
      └── history: UndoStack<Command>
```

Why this shape:

- **Delete / reorder / rotate / merge** are just edits to `pages`. Merging another PDF adds a `SourceDoc` and splices its `PageRef`s in. Nothing gets re-parsed or re-rendered except what becomes newly visible.
- **Rendering** a `PageRef` renders the original page from its source *without* its annotations. The viewer then draws annotations from the edit model, so new or edited annotations look the same as existing ones and stay editable. Existing annotations are imported into the model on load (keeping their original appearance stream when we don't modify them).
- **Undo/redo** works on `Command`s (`DeletePages`, `MovePages`, `AddAnnotation`, `SetFieldValue`, ...). Each command is a small diff against `EditDoc`.

### Annotations

```
Annotation { page: PageId, kind: AnnotKind, rect, color, opacity, author, modified,
             contents, appearance: Appearance }
AnnotKind = Highlight{quads} | Underline{quads} | StrikeOut{quads} | Ink{paths}
          | Square | Circle | Line | FreeText{text, font, size} | Text(note) | Other(raw)
Appearance = Original(ObjRef) | Generated(Cached<Stream>) | Dirty
```

`pdfrs-annot` generates appearance streams (`/AP /N`) for anything we create or modify, because many viewers (including Preview and pdf.js in some modes) rely on `/AP` and ignore the raw parameters. `Other(raw)` keeps annotation types we don't support intact across save.

### Forms

AcroForm only. `pdfrs-forms` builds the field tree (inheritance of `/FT`, `/Ff`, `/DA`, `/V`, ...), maps widgets to pages, and regenerates widget appearance streams when values change. Tab order follows `/Tabs` or falls back to widget order.

While a field is focused, the frontend overlays a **native text input** (GTK entry / Compose `TextField`) at the widget's device rect. Otherwise the widget is drawn from its appearance. That gives us IME, autocorrect, and accessibility for free.

## Rendering pipeline

```
pdfrs-view (visible pages, zoom, device scale)
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
- **Thumbnails** draw the viewer's placeholder tiles (one small tile per page) from the shared cache, so each page is rendered once for both the sidebar and the canvas. The page organizer will reuse them.
- **Annotations and form widgets** are rendered as a separate overlay pass per page. That pass is cheap to redo when the user edits, so we never re-render page content for annotation changes.

## Text layer

`pdfrs-text` implements the interpreter's device callback (e.g. hayro-interpret's `Device`) to collect glyphs with unicode mapping and quads. It then groups them into words and lines, in content-stream order (layout-based reading order for the files that need it is still to do). This powers:

- text selection (hit-test → glyph index → selection range → quads)
- copy to clipboard
- search (with match quads for highlighting results)
- text-markup annotations (highlight/underline/strike need quads)

## pdfrs-view: the shared viewer core

UI-agnostic state that both frontends drive. **Implemented in M1** (`crates/pdfrs-view`) for layout, zoom, scrolling and tile planning. Text selection (`selection.rs`: a `Selection` between two page-and-character positions, growing by characters, words or lines, with highlight quads in view coordinates) came in M2; text pages are supplied by the frontend through the `TextPages` trait, so they can be cached however suits it. Tools come in M4.

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
| Only annotations / form values changed | **Incremental update** (append to original file). Fast, and preserves existing digital signatures' byte ranges. |
| Pages deleted / reordered / merged | **Full rewrite**: copy reachable objects from each source (dedup shared resources), write a fresh xref and object streams, and drop unreferenced objects. |

Annotations are always saved as standard PDF annotations, never flattened into page content, as Preview does (D8).

**Encrypted documents** (D9): the security handler, key and permissions of the document being saved are kept. Incremental updates encrypt the newly appended objects with the existing key. A full rewrite re-encrypts with the same password and permissions. Merged pages from other (decrypted) sources get encrypted with the target document's key. "Export…" can add, change or remove a password.

Always write to a temp file and rename atomically. Never overwrite the file a `SourceDoc` is still memory-mapping from without re-opening it afterwards.

## Frontend integration

### Desktop

GTK4 + libadwaita via gtk4-rs. Linux first; it also runs on macOS with Homebrew's GTK. Links `pdfrs-view` directly as a Rust crate. A custom `GtkWidget` subclass (implementing `GtkScrollable`) draws each frame's `PageDraw`s in `snapshot()`, turning tiles into `GdkMemoryTexture`s. The surrounding chrome uses stock widgets: `AdwOverlaySplitView` with a `GtkListView` of thumbnails and a `GtkTreeListModel` outline, `GtkFileDialog`, and `GtkGestureZoom` for touchpad pinch. Planned: `GtkPrintOperation`, `GtkGestureStylus` for pen input, a page organizer, and Flatpak packaging (file access through the portal).

### Android

- Kotlin + Jetpack Compose for UI chrome.
- `pdfrs-ffi` exposes a coarse API via **UniFFI** (`ViewSession`: viewport, zoom, layout, outline, labels, links). Avoid chatty per-glyph calls across the FFI boundary.
- The per-frame path (frame layout, tile copies) uses hand-written JNI instead, because UniFFI's per-call overhead was too high (M0-FINDINGS.md).
- Tiles are copied from Rust into an `android.graphics.Bitmap` through the NDK bitmap API, and Compose draws them on a `Canvas`.
- File access via the Storage Access Framework. Today Kotlin reads the file into memory; planned: Kotlin opens a `ParcelFileDescriptor` and Rust reads from the fd, with saving through the same mechanism.
- Build with `cargo-ndk` for `arm64-v8a` (+ `x86_64` for the emulator).

## Testing strategy

- **Corpus:** pdf.js test PDFs, PDFium test corpus, and our own fixtures (forms, rotated pages, odd CropBoxes, encrypted files).
- **Render regression:** `pdfrs corpus` renders the corpus and compares with Poppler (corpus/README.md). Planned: PDFium as the reference, since Poppler is often the one that's wrong.
- **Desktop end to end:** `tools/headless/check.py` drives the app in a headless compositor over D-Bus, at several scales, in CI.
- **Android:** Kotlin unit tests in CI; Compose UI tests on an emulator.
- **Round-trip:** open → op → save → reopen with our parser *and* with an external validator (qpdf `--check`, and pdf.js/PDFium render).
- **Fuzzing:** `cargo-fuzz` on parsing and on the save path.
- **Property tests:** command sequences followed by full undo should return the original `EditDoc`.
