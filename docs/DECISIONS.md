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

## D5: FFI binding generator: UniFFI (Proposed)

UniFFI (Mozilla) generates Kotlin bindings with callback interfaces, which we need for `ViewHost`/tile-ready events. Alternative: hand-written `jni` crate code for the hot path (bitmap rendering) if UniFFI overhead shows up in profiles.

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

Goal is to match Preview. My understanding of Preview's behavior, still **to be verified** against a Mac:

- **User (open) password:** prompt on open. Saving keeps the document encrypted with the same password.
- **Owner password / permission flags only** (opens without a prompt, but e.g. editing or printing is restricted): viewing works, and restricted actions ask for the owner password first.
- **Adding or removing a password** happens in "Export…" (an Encrypt checkbox plus password).

Implications:

- We need to **write** encrypted PDFs, not just read them: RC4-40/128 for legacy files, AES-128 (V4) and AES-256 (V5/R6). Re-saving keeps the original algorithm, and new encryption uses AES-256.
- ✅ M0: lopdf 0.47 handles all of this. It writes RC4 through AES-256, and `IncrementalDocument` re-encrypts appended objects with the original key. Decrypt → edit → re-encrypt with the original `EncryptionState` works for full rewrites. hayro 0.9 decrypts for viewing.
- Honoring permission flags is policy: the file is technically readable without the owner password. We honor them anyway, like Preview.

## Open questions

1. Do we need to handle signed PDFs specially beyond "use incremental save so signatures stay valid"? (Preview warns that edits invalidate signatures.)
2. Min Android API level: 26+ is a reasonable default.
