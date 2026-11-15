<p align="center">
  <img src="apps/desktop/data/icons/net.xfbs.specto.svg" width="128" height="128" alt="">
</p>

<h1 align="center">Specto</h1>

<p align="center">
  A fast document viewer and light editor for Linux and Android, written in Rust.
</p>

<p align="center">
  <a href="apps/desktop/README.md">Desktop</a> ·
  <a href="apps/android/README.md">Android</a> ·
  <a href="docs/ARCHITECTURE.md">Architecture</a> ·
  <a href="docs/TASKS.md">Plan</a>
</p>

<p align="center">
  <img src="apps/desktop/screenshot.png" width="60%" alt="Specto on the desktop: two pages side by side, with the table of contents in the sidebar">
  &nbsp;
  <img src="apps/android/screenshot-phone.png" width="21%" alt="Specto on a phone: a page of a manual, the page number, and a strip of page thumbnails along the bottom">
</p>

Specto is a shared core with two apps on top of it:

- [Desktop](apps/desktop/README.md): GTK 4 and libadwaita, for Linux (also runs on macOS)
- [Android](apps/android/README.md): Kotlin and Jetpack Compose

## Why?

I enjoy working with Preview on macOS. When I want to read a PDF, or do some light editing (reorder or delete pages, highlight something, fill in a form), it's what I reach for: it's fast, it stays out of the way, and it does the common things well. On Linux I haven't found anything equivalent, so this is my attempt at one.

I also miss a decent open-source PDF viewer for Android that is pleasant to use. Since the hard parts (rendering, layout, zooming, text) are the same everywhere, they live in a shared Rust core, and each app is a thin native layer on top of it.

## Goals

Rough feature parity with Preview, not Adobe Acrobat:

- **Viewing**: sharp rendering on HiDPI screens, zoom, single page, continuous and two pages side by side, thumbnails, table of contents, search, text selection
- **Page editing**: delete, reorder and rotate pages, insert pages from another PDF, merge documents
- **Markup**: highlight, underline and strike through text, freehand drawing, shapes, text boxes, sticky notes
- **Forms**: fill in form fields (text, checkboxes, radio buttons, dropdowns and lists) and save the filled values

Not planned, at least for now: editing existing page content (reflowing text), XFA forms, creating digital signatures, JavaScript, OCR, PDF/A conversion.

## Status

Both apps are usable as PDF viewers. They render pages sharply at any zoom and on HiDPI screens, and stay responsive on huge documents. They have the usual layouts, thumbnails, the table of contents, links and printed page numbers, open encrypted files, and remember where you were in each document. In both you can select, copy and search text, and edit pages: rotate, delete and reorder them, and save the result. The desktop app can also insert pages from other PDFs, export a selection of pages, and print. Annotations and forms come next. The detailed plan is in [docs/TASKS.md](docs/TASKS.md).

## Building

```sh
cargo build --release
target/release/specto-gtk file.pdf                         # the desktop app
target/release/specto info file.pdf                        # the command-line dev tool
target/release/specto render file.pdf -o out -s 2          # pages → PNG
target/release/specto text file.pdf -p 1                   # the text layer
target/release/specto save file.pdf -o out.pdf -p 3,1-2    # pick, reorder, rotate (-r 90) pages
target/release/specto corpus corpus/files/... --reference  # regression run, see corpus/README.md
```

The desktop app needs the GTK 4 and libadwaita development packages; see [apps/desktop/README.md](apps/desktop/README.md). For Android, see [apps/android/README.md](apps/android/README.md). To build a Flatpak and install it, see [packaging/README.md](packaging/README.md).

## Layout

- `crates/specto-geom`: typed coordinate spaces and page geometry (crop box, rotation)
- `crates/specto-source`: opening (large files are memory-mapped), metadata, outline, links and page labels
- `crates/specto-text`: text layer (characters with positions, words, lines)
- `crates/specto-doc`: edit model (the pages of a document being edited, undo and redo)
- `crates/specto-write`: saving (a full rewrite with lopdf, checked against the original)
- `crates/specto-render`: `Document` trait + hayro backend, tile grid and cache, background render scheduler
- `crates/specto-view`: UI-agnostic viewer core (layout modes, zoom, scrolling, tile planning)
- `crates/specto-session`: an open document as both apps hold it (its files, the pages as edited, viewer, text, selection, search, saving)
- `crates/specto-ffi`: bindings for Android (UniFFI, plus JNI for the per-frame path)
- `apps/desktop`: GTK 4 + libadwaita app (`specto-gtk`)
- `apps/android`: Android app (Kotlin + Compose)
- `tools/specto-cli`: the `specto` dev tool
- `tools/test-files.sh`: copies of the test files for testing by hand, in `target/test-files/`
- `tools/headless`: run the desktop app under a headless compositor; `check.py` is the automated end-to-end check (also in CI), `smoke.sh` takes screenshots
- `spikes/`: early prototypes behind the measurements in docs/M0-FINDINGS.md. Only `lopdf-spike` is left (editing and saving, with the incremental and encrypted saving that `specto-write` doesn't do yet); the tiled GTK viewer and the text extraction check were removed once the desktop app and `specto-text` had replaced them
- `packaging/`: installable packages of the desktop app (Flatpak)
- `corpus/`: test corpus fetch script (the files are git-ignored)

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): crate layout, data model, rendering pipeline, how the apps plug in
- [docs/DECISIONS.md](docs/DECISIONS.md): technology choices, the alternatives considered, and open questions
- [docs/TASKS.md](docs/TASKS.md): the task list, by milestone
- [docs/M0-FINDINGS.md](docs/M0-FINDINGS.md): measurements from the prototypes and since (rendering, HiDPI, text, Android)
- [apps/desktop/FEATURES.md](apps/desktop/FEATURES.md) and [apps/android/FEATURES.md](apps/android/FEATURES.md): what each app does
- [apps/desktop/TESTING.md](apps/desktop/TESTING.md): testing the desktop app, by hand and automated
- [corpus/README.md](corpus/README.md): the test corpus and regression runs

## Similar projects

- [Censor](https://codeberg.org/censor/Censor)
- [GNOME Papers/Document Viewer](https://apps.gnome.org/Papers/)
- [Evince](https://gitlab.gnome.org/GNOME/evince)
- [Apple Preview](https://en.wikipedia.org/wiki/Preview_(Apple))

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
