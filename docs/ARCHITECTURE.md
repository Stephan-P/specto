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
│   ├── pdfrs-source    # parsing + read access to source PDFs (wraps parser backend)
│   ├── pdfrs-render    # Document trait, hayro backend, TileCache<T>, render scheduler
│   ├── pdfrs-text      # text extraction with glyph quads → selection, search, highlight targets
│   ├── pdfrs-doc       # edit model: page list, annotations, form values, undo/redo, commands
│   ├── pdfrs-annot     # annotation types + appearance-stream generation
│   ├── pdfrs-forms     # AcroForm model, field values, appearance regeneration
│   ├── pdfrs-write     # serialization: full rewrite + incremental update
│   ├── pdfrs-view      # UI-agnostic viewer: layout modes, viewport, zoom, hit-test, tools
│   └── pdfrs-ffi       # UniFFI bindings (Kotlin) over pdfrs-view/pdfrs-doc
├── apps/
│   ├── desktop/        # GTK4 + libadwaita (gtk4-rs), Linux
│   └── android/        # Gradle project, Kotlin + Jetpack Compose, loads libpdfrs_ffi.so
└── tools/
    └── pdfrs-cli       # headless: render to PNG, dump text, apply ops, used heavily in tests
```

Dependency direction: `apps → pdfrs-view / pdfrs-ffi → pdfrs-doc → {pdfrs-annot, pdfrs-forms, pdfrs-text, pdfrs-render} → pdfrs-source → pdfrs-geom`. `pdfrs-write` depends on `pdfrs-doc` + `pdfrs-source`.

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
RenderScheduler (priority queue: visible > prefetch; cancel on scroll/zoom)
   │  worker thread pool (rayon or dedicated threads)
   ▼
Renderer trait ──► HayroRenderer (default)   | PdfiumRenderer (tests / optional fallback)
   │  RGBA8 premultiplied tile
   ▼
TileCache (LRU, memory budget; lower budget on Android)
   │
   ▼
Frontend uploads tile to a GPU texture (gpui) / GdkTexture (GTK) / Bitmap (Android)
```

- **HiDPI:** the device scale factor (`1.0`, `1.5`, `2.0`, `2.625` ...) gets multiplied into the render scale. Tiles are always rendered at physical pixels.
- **Zoom:** zoom levels are bucketed (e.g. powers of √2). While zooming, the previous bucket's tiles are scaled on the GPU, and sharp tiles replace them as they finish. This is the "blurry → sharp" behavior Preview has.
- **Tiling:** 256×256 or 512×512 physical-pixel tiles, so a 400% zoom on a poster PDF doesn't allocate a 100 MP bitmap.
- **Thumbnails:** a separate low-priority queue at a fixed small scale. They're cached and reused by the page-organizer UI.
- **Annotations and form widgets** are rendered as a separate overlay pass per page. That pass is cheap to redo when the user edits, so we never re-render page content for annotation changes.

## Text layer

`pdfrs-text` implements the interpreter's device callback (e.g. hayro-interpret's `Device`) to collect glyphs with unicode mapping and quads. It then groups them into words, lines, and blocks in reading order. This powers:

- text selection (hit-test → glyph index → selection range → quads)
- copy to clipboard
- search (with match quads for highlighting results)
- text-markup annotations (highlight/underline/strike need quads)

## pdfrs-view: the shared viewer core

UI-agnostic state machine that both frontends drive:

```rust
trait ViewHost {                      // implemented by each frontend
    fn request_redraw(&self);
    fn tile_ready(&self, key: TileKey);
    fn show_text_input(&self, field: FieldId, rect: DeviceRect, value: &str);
    ...
}

struct Viewer {
    layout: LayoutMode,               // Single | Continuous | TwoUp { cover_alone: bool } | TwoUpContinuous
    viewport: Viewport,               // scroll offset, size, scale factor
    zoom: Zoom,                       // Fixed(f32) | FitWidth | FitPage
    tool: Tool,                       // Select | Highlight | Ink | Text | Shape | Note | Hand
    selection: Selection,
}

impl Viewer {
    fn visible_items(&self) -> Vec<DrawItem>;   // page rects + tiles + overlays to draw this frame
    fn on_pointer(&mut self, ev: PointerEvent) -> Vec<Effect>;  // includes stylus pressure
    fn on_key(&mut self, ev: KeyEvent) -> Vec<Effect>;
    fn on_scroll / on_pinch / set_layout / set_zoom ...
}
```

Frontends translate native events into `PointerEvent`/`KeyEvent` and draw `DrawItem`s. Tool logic (ink smoothing, highlight snapping to text, shape drawing, moving/resizing annotations) is implemented once.

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

GTK4 + libadwaita via gtk4-rs, Linux only. Links `pdfrs-view` directly as a Rust crate. A custom `GtkWidget` subclass draws `DrawItem`s in `snapshot()`, turning tiles into `GdkMemoryTexture`s. The surrounding chrome uses stock widgets: `AdwNavigationSplitView` with a `GtkGridView` for the thumbnail sidebar and page organizer, `GtkFileDialog`, `GtkPrintOperation`, and `GtkGestureZoom` / `GtkGestureStylus` for touchpad pinch and pen input. It's packaged as a Flatpak, so file access goes through the portal.

### Android

- Kotlin + Jetpack Compose for UI chrome.
- `pdfrs-ffi` exposes a coarse API via **UniFFI** (`Session`, `Viewer`, commands, events). Avoid chatty per-glyph calls across the FFI boundary.
- Tiles go into an `android.graphics.Bitmap` via `AndroidBitmap_lockPixels` (NDK) from Rust. That's zero-copy into the bitmap, and Compose draws them on a `Canvas`.
- File access via the Storage Access Framework: Kotlin opens a `ParcelFileDescriptor`, and Rust reads from the fd. Saving uses the same mechanism.
- Build with `cargo-ndk` for `arm64-v8a` (+ `x86_64` for the emulator).

## Testing strategy

- **Corpus:** pdf.js test PDFs, PDFium test corpus, and our own fixtures (forms, rotated pages, odd CropBoxes, encrypted files).
- **Render regression:** `pdfrs-cli render` → PNG, diffed against stored goldens, with PDFium output as the reference where licensing allows.
- **Round-trip:** open → op → save → reopen with our parser *and* with an external validator (qpdf `--check`, and pdf.js/PDFium render).
- **Fuzzing:** `cargo-fuzz` on parsing and on the save path.
- **Property tests:** command sequences followed by full undo should return the original `EditDoc`.
