# pdfrs

A PDF viewer and light editor written in Rust, with a shared core and two frontends:

- **Desktop**: Linux, GTK4 + libadwaita
- **Android**

The goal is rough feature parity with macOS Preview, not Adobe Acrobat:

| Area | Scope |
|---|---|
| Viewing | HiDPI-sharp rendering, zoom, single page / continuous / two-up (side-by-side), thumbnails, outline (TOC), search, text selection |
| Page editing | Delete, reorder, rotate, insert pages from another PDF, merge documents |
| Markup | Highlight / underline / strike-through, freehand ink, shapes, text boxes, sticky notes |
| Forms | Fill AcroForm fields (text, checkbox, radio, dropdown/list), save filled values |

Explicit **non-goals** (at least initially): editing existing page content (text reflow), XFA forms, digital signature creation, JavaScript, OCR, PDF/A conversion.

## Similar Projects

- [Censor](https://codeberg.org/censor/Censor)
- [GNOME Papers/Document Viewer](https://apps.gnome.org/Papers/)
- [Evince](https://gitlab.gnome.org/GNOME/evince)
- [Apple Preview](https://en.wikipedia.org/wiki/Preview_(Apple))

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): crate layout, data model, rendering pipeline, how the frontends plug in
- [docs/DECISIONS.md](docs/DECISIONS.md): technology choices, the alternatives considered, and open questions
- [docs/TASKS.md](docs/TASKS.md): milestone-based task list
- [apps/desktop/FEATURES.md](apps/desktop/FEATURES.md): what the desktop app does
- [apps/desktop/TESTING.md](apps/desktop/TESTING.md): manual tests for the desktop app on Linux and macOS

## Status

M0 (spikes) is done; M1 (core viewer) is in progress. See [docs/M0-FINDINGS.md](docs/M0-FINDINGS.md).

```sh
cargo build --release
target/release/pdfrs-gtk file.pdf                         # the desktop app
target/release/pdfrs info file.pdf
target/release/pdfrs render file.pdf -o out -s 2          # pages → PNG
target/release/pdfrs corpus corpus/files/... --reference  # regression run, see corpus/README.md
```

## Layout

- `crates/pdfrs-geom`: typed coordinate spaces and page geometry (crop box, rotation)
- `crates/pdfrs-source`: opening, metadata, outline and links
- `crates/pdfrs-text`: text layer (characters with positions, words, lines)
- `crates/pdfrs-render`: `Document` trait + hayro backend, tile grid and cache, background render scheduler
- `crates/pdfrs-view`: UI-agnostic viewer core (layout modes, zoom, scrolling, tile planning)
- `crates/pdfrs-ffi`: UniFFI bindings for Android
- `apps/desktop`: GTK4 + libadwaita app (`pdfrs-gtk`)
- `apps/android`: Android app (Kotlin + Compose)
- `tools/pdfrs-cli`: the `pdfrs` dev tool
- `tools/headless`: run the desktop app under a headless compositor; `check.py` is the automated end-to-end check (also in CI), `smoke.sh` takes screenshots
- `spikes/`: throwaway prototype code from M0 (to be deleted once ported): `lopdf-spike` (editing/saving), `gtk-canvas` (tiled GTK viewer, `cargo run --release -p gtk-canvas -- file.pdf`)
- `corpus/`: test corpus fetch script (files are git-ignored)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
