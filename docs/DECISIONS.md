# Technology decisions (draft)

Status legend: **Proposed** (recommendation, not validated), **Spike** (needs a prototype before deciding), **Decided**.

## D1: Rendering engine: `hayro` (Decided, 2026-11-14, after the M0 spike)

| Option | Pros | Cons |
|---|---|---|
| **hayro** (pure Rust, LaurenzV) | No C deps, `forbid(unsafe)` core, modular (`hayro-syntax`, `hayro-interpret` with an abstract `Device`, `hayro` raster, `hayro-svg`), own JPEG2000/CCITT/Type1/CFF decoders, MIT/Apache | Still pre-1.0 and self-described as experimental. CPU-only. Performance on heavy pages is unknown. |
| pdfium-render (PDFium bindings) | Battle-tested (Chromium), renders + forms + annotations | Must ship a PDFium binary per platform (incl. Android ABIs). C++ attack surface. Less control. |
| mupdf bindings | Excellent quality and speed | **AGPL** (or commercial license). Viral for the whole app. |
| pdf-rs/`pdf` + own renderer | Pure Rust | Far more work than hayro. |

**Recommendation:** hayro behind the `Renderer` trait, with a `PdfiumRenderer` used in CI as a reference for render regression tests and kept as an emergency fallback. The `Device` abstraction in `hayro-interpret` is also exactly what we need for text extraction.

**M0 result:** 1095 pdf.js test files, 0 crashes, median 2.8 ms per page at 2×. It decrypts encrypted files, and `RenderContent::PAGE` renders pages without annotations. Known gaps: knockout groups, transfer functions, and no cancellation. Details in [M0-FINDINGS.md](M0-FINDINGS.md).

## D2: PDF object model / writer: lopdf (Decided, 2026-11-14, after the M0 spike)

Rendering only needs read access. Editing and saving need a mutable object model and a writer.

| Option | Notes |
|---|---|
| **lopdf** | Mature pure-Rust read/write object model, widely used (also by form crates like `pdfer_forms`). Good for copying objects between documents and writing. |
| hayro-syntax + own writer | One parser for everything, but we'd write the serializer (incl. xref streams, object streams, incremental updates) ourselves. |
| pdf-writer (typst) | Excellent low-level *writer*, but no reader. Could be combined with hayro-syntax. |

**Concern:** lopdf for editing + hayro-syntax for rendering means two parsers seeing the same file. That's acceptable because the edit model never re-parses (sources are immutable). But the two parsers may disagree on malformed files.

**Decision:** lopdf in `pdfrs-write` for the object model and saving, with hayro for viewing. In M0, lopdf loaded 1092/1095 corpus files and passed every editing scenario (delete, merge, reorder, incremental annotation, encryption). Merging needs our own page-import code (renumber objects, push down inherited attributes), because lopdf has none. See [M0-FINDINGS.md](M0-FINDINGS.md).

## D3: Desktop UI toolkit: GTK4 + libadwaita (Decided, 2026-11-14)

The desktop app targets **Linux only**. That removes gpui's main advantage (consistency across platforms), while GTK4 already has what a Preview-class app needs: print, DnD list/grid views, file dialogs via portals, stylus and touchpad gestures, accessibility. We give up idiomatic Rust ergonomics (GObject subclassing) and some rendering headroom. The comparison below is kept for reference.

| | GTK4 (+ libadwaita) via gtk-rs | gpui (Zed) |
|---|---|---|
| Maturity | Very mature, stable API | Pre-1.0, API changes with Zed's needs, versioning is tied to the Zed repo |
| Widgets | Complete: sidebars, list views with DnD (page organizer), popovers, file chooser, **print dialog**, accessibility | Minimal built-in widgets; `gpui-component` (third-party) fills gaps. No print support. |
| Custom canvas | `GtkSnapshot` + `GdkTexture`s, GPU-composited through GSK. Good enough for tiles. | Excellent: GPU-first, immediate-feeling, great for a fast scrolling canvas |
| Rust ergonomics | GObject subclassing is verbose; `Rc<RefCell<…>>` everywhere | Idiomatic Rust, entity/context model |
| Platforms | Linux great; macOS/Windows functional but look foreign | macOS + Linux good, Windows maturing |
| Look | Native GNOME look | Custom look (whatever you design) |

Because `pdfrs-view` is UI-agnostic, a later port (e.g. to gpui) would only replace `apps/desktop`. The M0 canvas spike measured about 55 fps during sustained fast scroll and zoom of a 384-page PDF, with negligible snapshot cost. See [M0-FINDINGS.md](M0-FINDINGS.md).

## D4: Android UI: Kotlin + Jetpack Compose with Rust core via UniFFI (Proposed)

| Option | Notes |
|---|---|
| **Compose + UniFFI** | Native look, system file picker (SAF), stylus/palm rejection, IME, accessibility, Material components all work out of the box. Rust only renders and does logic. |
| Slint / egui / Dioxus native | Single-language, but weaker on IME, accessibility, stylus input, and native feel. |
| gpui-mobile | Third-party, 0.1.0, platform implementation "coming soon". Too early. |
| Android's built-in `PdfRenderer` | Not our engine. No editing. Defeats the purpose. |

**Recommendation:** Compose + UniFFI, with tiles written directly into `Bitmap`s from Rust via the NDK bitmap API.

## D5: FFI: UniFFI for the coarse API, hand-written JNI for the hot path (Decided, 2026-11-14)

UniFFI (Mozilla) generates the Kotlin bindings for everything called occasionally: opening documents, changing modes, callbacks such as `tiles_ready`. Anything per frame or per tile goes through a few hand-written JNI functions instead (`crates/pdfrs-ffi/src/jni.rs` over `hot.rs`), using only `jni-sys` and the `ndk` crate:

- **Tile pixels:** returning a 6.6 MB page through UniFFI cost about 6 ms extra; JNI copies once into the locked `Bitmap` (M0).
- **Per-frame calls** (scroll, zoom, frame, finished tiles): even a trivial UniFFI call cost 200–370 µs through JNA, against about 4 µs of actual work. On a Pixel 9a, moving these to JNI cut slow frames from 1.65% to 0.61% (debug build; M0-FINDINGS.md, "On a phone").

JNI functions take a session id, not a raw pointer, and frames are written into a direct `ByteBuffer` in a documented layout (`hot.rs`). `jni.rs` only compiles for Android, so CI lints the FFI crate a second time with `--target aarch64-linux-android`.

## D6: License: MIT OR Apache-2.0 (Decided, 2026-11-14)

Dual license, the Rust ecosystem convention, matching hayro and lopdf. Users can pick either. Apache-2.0 adds an explicit patent grant, and MIT keeps us compatible with GPLv2 projects. Avoid mupdf (AGPL) and other copyleft dependencies.

## D7: Prior art to study

- **PDFCraft** (getartcraft.com): Rust PDF editor, Acrobat-style. Worth looking at for scope and stack choices.
- **Evince / Papers (GNOME)**: GTK document viewers. The page organizer and annotation UX are good references for the GTK path.
- **pdf.js**: reference for annotation and form rendering edge cases and appearance-stream generation.
- **Rnote**: Rust + GTK4 handwriting app. Good reference for ink smoothing, stylus input, and GTK canvas performance (and it has PDF import).

## D8: Annotations are saved as annotations (Decided, 2026-11-14)

Same as Preview: markup is written as standard PDF annotation objects with generated appearance streams, so other viewers can show, edit and delete it. Flattening into page content isn't the default. At most, it's a later "Export (flattened)" option.

## D9: Encrypted PDFs follow Preview (Decided, 2026-11-14)

Goal is to match Preview. Preview's behavior (checked on a Mac, 2026-11-14, except where marked):

- **User (open) password:** prompt on open. ✅ Saving keeps the document encrypted with the same password.
- ✅ **Setting passwords:** Preview manages both the user (open) password and the owner (permissions) password in a settings panel reached from the File menu, where each can be added, changed or removed. This is a document setting that is saved with the file, not an Export-only option as first assumed.
- **Owner password / permission flags only** (opens without a prompt, but e.g. editing or printing is restricted): *not yet checked.* Assumed: viewing works, and restricted actions ask for the owner password first.

For pdfrs this means a "Security" (or "Passwords") dialog on the document, with user and owner password fields. Its changes are saved like any other edit (a full rewrite, since the encryption dictionary changes).

Implications:

- We need to **write** encrypted PDFs, not just read them: RC4-40/128 for legacy files, AES-128 (V4) and AES-256 (V5/R6). Re-saving keeps the original algorithm, and new encryption uses AES-256.
- ✅ M0: lopdf 0.47 handles all of this. It writes RC4 through AES-256, and `IncrementalDocument` re-encrypts appended objects with the original key. Decrypt → edit → re-encrypt with the original `EncryptionState` works for full rewrites. hayro 0.9 decrypts for viewing.
- Honoring permission flags is policy: the file is technically readable without the owner password. We honor them like Preview, but only as a default: a setting turns that off (2026-11-14), as Evince and Okular allow too. Whether honoring should be the default at all is open.

## D10: Other document formats: images on desktop later, PDF only on Android (Decided, 2026-11-14)

Preview and GNOME Papers open more than PDF. We stay PDF-first until the PDF features work well, then aim for Preview parity on desktop, which in practice means PDF plus images.

| Format | Cost | Verdict |
|---|---|---|
| **Images** (PNG, JPEG, TIFF, WebP) | Low. A one-page document, and the `image` crate is already in the workspace. | Desktop, after the PDF milestones. Also gives "insert image as page" and "images to PDF". |
| Comic archives (CBZ) | Low. A zip of images. | Maybe, once images exist. |
| DjVu, XPS, PostScript | High. Each needs its own engine, and the mature DjVu library (DjVuLibre) is GPL, which conflicts with D6. | Out of scope. |
| EPUB | Very high. Reflowable, so it doesn't fit the page-and-tile model. | Out of scope. |

**Decision:**

- **Desktop:** add image viewing as a later milestone. Nothing is built for it now.
- **Android:** PDF only. The system already handles images, and the gap (notably on GrapheneOS) is a good PDF viewer. Each non-PDF backend is a cargo feature that the Android build leaves off, so it adds no size there.
- Editing (page operations, annotations, forms, saving) stays PDF-specific. Marking up images the way Preview does is a separate feature and not part of this decision.

**What this asks of M1–M5:** keep the door open, at no extra cost.

- `Document`, the scheduler and `pdfrs-view` stay free of PDF-only assumptions. Today's `Document` trait (page count, page geometry, render a region) already is.
- Text, outline and editing are capabilities a document may or may not have, not things every document is assumed to support.

## Open questions

1. Do we need to handle signed PDFs specially beyond "use incremental save so signatures stay valid"? (Preview warns that edits invalidate signatures.)
2. Min Android API level: 26+ is a reasonable default.
