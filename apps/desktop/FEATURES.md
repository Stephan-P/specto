# Desktop app features

What the desktop app (`specto-gtk`, GTK 4 and libadwaita) currently does. For testing these by hand, see [TESTING.md](TESTING.md).

Shortcuts use *Primary*: Ctrl on Linux, Cmd on macOS.

## Opening files

- Start page with an Open button and a list of recently opened PDFs
- Open with the file dialog (*Primary*+O), from the command line, or by dragging a file onto a window
- One window per document; opening a file while a document is shown opens a new window
- Password prompt for encrypted PDFs, with a retry on a wrong password
- Error message for files that aren't valid PDFs
- Large files (64 MB and up) are mapped into memory rather than read: they open at once, and only the parts in use take memory
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
- Several thumbnails can be selected: Shift+click for a range, *Primary*+click to add or remove one, *Primary*+A with the sidebar focused for all. A selection of several stays while you move around the document; a plain click selects one page again and goes there
- Contents tab with the document's outline, when it has one: titles wrap to up to three lines, with the page number (or label) right-aligned in grey; entries jump to the exact position
- Links: in-document links and named actions (next page, …) are followed; web and mail links open in the default app; other kinds are refused

## Text

- Drag over text to select it, across lines and pages; the selection is drawn in the accent colour
- Double-click selects a word, triple-click a line; dragging afterwards extends by words or lines
- Shift+click extends the selection
- An I-beam cursor over text; a hand over links. Clicking a link follows it, dragging from one selects
- Dragging near the top or bottom edge of the view scrolls
- *Primary*+C copies the selection, *Primary*+A selects the whole document, Escape or a click clears the selection
- On Linux the selection is also the primary selection, for middle-click paste
- Works on rotated pages and rotated text

## Search

- *Primary*+F or the search button opens a search bar under the header bar
- Finds text regardless of case and accents ("cafe" finds "Café"), across line breaks, and words hyphenated at the end of a line
- Shows "3 of 17"; Enter and Shift+Enter (or *Primary*+G and Shift+*Primary*+G, or the arrow buttons) go to the next and previous match, wrapping around at the ends
- The first match is the first one from the current page on; the view only scrolls when the match isn't already visible
- All matches are highlighted in yellow, the current one in orange
- Long documents are searched in the background: the first matches show up at once, and the count grows ("17…") until every page is done
- Escape closes the bar and removes the highlights

## Editing

- Rotate Left and Rotate Right (*Primary*+Left and *Primary*+Right, or the main menu) turn the pages selected in the sidebar if there are several, otherwise the current page, or both pages of a two-page spread
- Dragging thumbnails moves pages: dragging a selected thumbnail moves all the selected pages. A line between thumbnails shows where they'll go, and the sidebar scrolls when dragging near its top or bottom. The moved pages stay selected, and the view stays on the page it showed
- Dropping PDFs (from the file manager) on the thumbnails inserts all their pages there, as one edit; dropping them on the page view opens them in new windows as before
- Delete Pages (Delete or Backspace on the document or in the sidebar, or the main menu) deletes the pages selected in the sidebar if there are several, otherwise the current page. The last page can't be deleted. In text fields the keys edit the text
- Insert Pages from File (main menu): all pages of another PDF go after the current page (or spread), and the view goes to the first of them, with the new pages selected in the sidebar. Encrypted files ask for their password
- Undo and Redo (*Primary*+Z, Shift+*Primary*+Z, or the main menu, which names the edit: "Undo Rotate Page")
- "Edited" in the title while there are changes; undoing back to the state as opened removes it
- Selection, search, links and the table of contents keep working on edited pages, inserted ones included
- Pages keep their printed page numbers (labels) through rotating, deleting, moving and inserting; inserted pages of a file without labels are numbered by their position
- Thumbnails dragged into another window's sidebar are copied there (with their rotation), as one edit; the first window keeps them
- In the page entry or the search field, the shortcuts act on the text instead (Undo, moving by words)

## Saving

- Save (*Primary*+S) and Save As (Shift+*Primary*+S), in the main menu
- Export Selected Pages (main menu): saves the pages selected in the sidebar (or the current page) as a new PDF, suggesting a name like "Report (pages 2-4, 7).pdf"; the document stays as it was
- The new file is written next to the old one and then takes its place, so the old file is intact until the new one is complete
- Encrypted documents stay encrypted, with the same password
- Saved files keep the outline, links, metadata and page labels; after deleting, moving or inserting pages, each page keeps its label in the saved file too
- Closing a window or quitting with unsaved changes asks "Save Changes?": Save, Discard or Cancel
- Edits made while a save is running stay "Edited"
- When another program changes the open file: without edits, it's reloaded in place, at the same page and zoom; with edits, a banner says so and offers Reload (which discards the edits). Saving over a file changed that way asks first ("File Changed on Disk"). Specto's own saves don't count as changes
- When the open file is moved or deleted, a banner says so; Save writes it again
- A file Specto can't save correctly (a few damaged ones) isn't written: a message says so instead

## Printing

- Print (*Primary*+P, the main menu) with the system's print dialog: printer, copies, paper, all pages, the current page, a range, or the pages selected in the sidebar ("Selection")
- Prints the document as edited: pages in their order, with their rotation, including inserted pages
- Each page fitted to the paper and centred; pages wider than tall go on landscape paper
- Pages are printed as images at 300 dpi (sharp on paper, but a "Print to File" PDF is large and has no selectable text; Export Selected Pages keeps it)
- The dialog remembers the last printer and settings while the app runs

## macOS

- Menu bar in the system menu bar: File (Open, Save, Save As, Export Selected Pages, Print, Close Window, Quit), Edit (Undo, Redo, Copy, Select All, Insert Pages from File, Delete Pages, Find, Find Next, Find Previous), View (layouts, cover page, zoom, sidebar), Go (previous, next, first, last page, go to page), Tools (Rotate Left, Rotate Right), Help (About Specto)
- Shortcuts with Cmd

## About

- About Specto (the main menu): the version, the licence, and links to the project page and to where problems are reported

## Remembering state

- Per file: page and position, layout, zoom, sidebar visibility and tab
- Restored when the file is opened again; the last 200 files are kept
- Quitting (*Primary*+Q) closes all windows, so each one saves

## Under the hood

- One shared pool of render threads for all windows
- Protection against pathological pages: slow pages render one tile at a time, and extra workers start when renders stall

## Not yet

- Annotations, form filling
- Highlighting the current outline entry
- Dark mode for page content
