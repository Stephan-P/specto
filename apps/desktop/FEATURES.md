# Desktop app features

What the desktop app (`pdfrs-gtk`, GTK 4 and libadwaita) currently does. For testing these by hand, see [TESTING.md](TESTING.md).

Shortcuts use *Primary*: Ctrl on Linux, Cmd on macOS.

## Opening files

- Start page with an Open button and a list of recently opened PDFs
- Open with the file dialog (*Primary*+O), from the command line, or by dragging a file onto a window
- One window per document; opening a file while a document is shown opens a new window
- Password prompt for encrypted PDFs, with a retry on a wrong password
- Error message for files that aren't valid PDFs
- Opened files are added to the system's recent files (shown in GNOME Files too)

## Viewing

- Continuous scrolling with native overlay scrollbars and kinetic touchpad scrolling
- Layouts: single page, continuous, two pages, two pages continuous (*Primary*+1 to 4); optionally the first page alone (book cover)
- Rotated pages and mixed page sizes shown correctly
- Sharp on HiDPI and fractionally scaled screens: once zooming stops, pages are drawn at exactly the screen resolution
- Low-resolution previews while pages render, then sharp tiles
- A spinner over pages that take long to render (huge images, very complex pages)

## Zoom

- Zoom in, zoom out, actual size, fit width, fit page (keyboard and zoom menu)
- Ctrl+scroll (also Cmd+scroll on macOS) and touchpad pinch, zooming around the pointer
- Zoom range 5% to 3200%; the current zoom is shown in the header

## Navigation

- Page entry showing the current page; type a page number to go there (*Primary*+L)
- Page labels (e.g. "iv", "A-3") in the page entry and thumbnails; labels can be typed too
- Keyboard: Page Up/Down, Space, Backspace, arrow keys, Home, End
- Thumbnail sidebar (F9); the current page is highlighted and kept in view
- Contents tab with the document's outline, when it has one; entries jump to the exact position
- Links: in-document links and named actions (next page, …) are followed; web and mail links open in the default app; other kinds are refused

## macOS

- Menu bar in the system menu bar: File (Open, Close Window, Quit), View (layouts, cover page, zoom, sidebar), Go (previous, next, first, last page, go to page)
- Shortcuts with Cmd

## Remembering state

- Per file: page and position, layout, zoom, sidebar visibility and tab
- Restored when the file is opened again; the last 200 files are kept
- Quitting (*Primary*+Q) closes all windows, so each one saves

## Under the hood

- One shared pool of render threads for all windows
- Protection against pathological pages: slow pages render one tile at a time, and extra workers start when renders stall

## Not yet

- Text selection, copying and search (next)
- Printing, saving, page editing, annotations, form filling
- Rotating pages from the app
- Highlighting the current outline entry
- Dark mode for page content
