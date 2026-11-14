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

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): crate layout, data model, rendering pipeline, how the frontends plug in
- [docs/DECISIONS.md](docs/DECISIONS.md): technology choices, the alternatives considered, and open questions
- [docs/TASKS.md](docs/TASKS.md): milestone-based task list

## Status

Planning. No code yet.
