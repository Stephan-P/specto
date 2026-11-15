# Testing the desktop app by hand

[FEATURES.md](FEATURES.md) is the checklist: go through every feature there on each platform (Linux and macOS). This file has the setup, and the checks that need a particular test file or whose expected result isn't obvious from the feature list.

## Automated checks

Much of the feature list is also checked automatically, in CI and locally (Linux, needs `mutter`; `ci/install-deps-fedora.sh --headless` installs the packages on Fedora):

```sh
cargo build -p specto-desktop
tools/headless/run.sh python3 -I tools/headless/check.py
SPECTO_SCALE=2 tools/headless/run.sh python3 -I tools/headless/check.py   # also 1.25
tools/headless/run.sh python3 -I tools/headless/check.py zoom layouts    # some tests only
```

The app runs in a headless compositor with its own saved state, so yours isn't touched; the pointer and keyboard tests use a virtual pointer and keyboard in that compositor, never your own. What's covered is listed at the top of `tools/headless/check.py`. Not covered: pinch, dialogs, dragging thumbnails (the checks drop them through a test hook), how sharp the text looks, how the selection feels with a real mouse or touchpad, and macOS. Those still need the manual pass.

## Setup

All commands and paths are relative to the repository root.

```sh
cargo build --release -p specto-desktop
target/release/specto-gtk                 # start page
target/release/specto-gtk a.pdf b.pdf     # one window per file
```

macOS needs GTK 4, libadwaita 1.6 or newer, the Adwaita icons and gettext from Homebrew: `brew install gtk4 libadwaita adwaita-icon-theme gettext`, and `GETTEXT_DIR="$(brew --prefix gettext)"` set when building (see [BUILDING.md](BUILDING.md)). Without the icon theme, header bar and sidebar icons are missing.

*Primary* means Ctrl on Linux and Cmd on macOS.

Test files: run `corpus/fetch.sh` once. The files are in `corpus/files/pdfjs/test/pdfs/`:

- LONG: `freeculture.pdf`. 352 pages, an outline, web links, three page sizes
- LINKS: `issue14847.pdf`. In-document links
- LABELS: `labelled_pages.pdf`. Page labels i, ii, iii, iv, 1, 2, 3, a, b, 4, 5
- ROTATED: `hello_world_rotated.pdf`. Every page rotated
- SIZES: `sizes.pdf`. Pages of different sizes, no outline
- GIANT: `issue19517.pdf`. One 4.4 × 5.9 m page with a 212-megapixel image, slow on purpose. Needs about 0.7 GB at fit width and up to 4.5 GB zoomed in to 100%
- BROKEN: `REDHAT-1531897-0.pdf`. Not a valid PDF
- ENCRYPTED: generated as below; the password is `secret`

```sh
# qpdf: dnf install qpdf / brew install qpdf
qpdf --encrypt secret owner 256 -- corpus/files/pdfjs/test/pdfs/labelled_pages.pdf /tmp/encrypted.pdf
```

`tools/test-files.sh` puts copies of all of these in `target/test-files/`, named after their roles (`long.pdf`, `encrypted.pdf`, …), so saving doesn't change the corpus. It also makes `big.pdf`, LONG padded to over 64 MB, which is memory-mapped when opened. Run it again for fresh copies.

Resetting saved state:

- View state: delete `~/.local/state/specto/view-state.ini` (or `$XDG_STATE_HOME/specto/view-state.ini`). Same path on macOS.
- Recent files live in GTK's shared list (`~/.local/share/recently-used.xbel`), which other apps use too. Don't delete it; clear recent history in GNOME Settings → Privacy, or test with files you haven't opened before.

## Specific checks

### Opening files

- BROKEN, and a non-PDF file renamed to `.pdf`: a message "Couldn't open … not a valid PDF file", and the app keeps running.
- ENCRYPTED: a wrong password brings the dialog back saying it was incorrect; Cancel opens nothing; `secret` opens it (with the labels of LABELS).
- A recent file that was moved or deleted disappears from the start page after a restart.

### Viewing

- ROTATED: every page is shown the right way up, in its rotated orientation.
- SIZES and LONG: pages of different sizes are centred, with equal gaps between them.
- Switching layouts on page 50 of LONG stays on about page 50.
- Show Cover Page Alone: page 1 alone, then 2–3, 4–5 and so on.
- Resizing the window at fit width keeps the view at the same place in the document.

### Zoom and sharpness

- Ctrl+scroll and pinch: the point under the pointer or fingers stays put.
- Stopping at an odd zoom such as 137%: slightly soft for about a quarter second, then sharp.
- Holding a pinch without lifting the fingers: stays soft until released.
- Zooming far in and out stops at 5% and 3200%, without blank pages.
- On a HiDPI screen (Retina, or Linux at scale 2): text at fit width is as crisp as at 100%, and as crisp as in other apps. Thumbnails are sharp too.
- Linux with fractional scaling (125% or 150%) and a window that isn't maximized: just as crisp. This case was blurry before.
- Moving the window to a screen with a different scale: it re-renders sharp there.

### Navigation

- LABELS: the page entry shows "i (1 of 11)".
- LABELS: typing "4" goes to the page labelled 4 (the 10th page), not the 4th page. Typing "IV" goes to "iv" (case doesn't matter).
- Typing 999 or "abc": an error bell, and the entry goes back to the current page.
- LONG has Contents and Thumbnails tabs; SIZES (no outline) has thumbnails only.
- Contents entries in LONG jump to the exact place, often mid-page. Long titles wrap to up to three lines, then end in "…"; page numbers match the printed ones (on Linux, `/usr/share/doc/hwloc/hwloc-a4.pdf` starts its numbering after roman front matter).
- LINKS: in-document links jump to their target. LONG: web links open in the browser.
- Pressing on a link, moving a few pixels and releasing still follows it. A longer drag selects text instead (GTK's drag threshold).

### Text

- LONG: select a paragraph, copy, paste into a text editor: the text matches, with line breaks at the ends of lines and no hyphenation fixes (none are made yet).
- A two-column paper: dragging within a column stays in that column; dragging from one column into the other selects the rest of the first column and the start of the second (content order).
- Dragging quickly past the bottom of the window, with the window not maximized: it keeps scrolling while the pointer is outside, faster the further out.
- Linux: after selecting, middle-click in a text editor pastes the selection (primary selection), without pressing *Primary*+C.
- Typing in the page entry and pressing *Primary*+A or *Primary*+C acts on the entry's text, not the document.
- macOS: Edit → Copy and Edit → Select All in the menu bar work, and so do Cmd+C and Cmd+A.
- In dark mode the highlight is still readable on the white page.

### Search

- LONG: search for "copyright". Matches appear on the current page or the next ones first; the count grows with "…" until it settles (a second or so), and Enter steps through them, scrolling only when the match is off screen.
- LONG: a phrase that wraps onto the next line in the book is found as one match, highlighted on both lines.
- A document with accented text (French, German): searching without the accents finds the words.
- Opening the bar with *Primary*+F and typing straight away: nothing typed is lost.
- With matches on screen, zooming and switching layouts: the highlights stay on the words.
- GIANT: searching doesn't freeze the window (the page has no text: "No matches").
- macOS: Edit → Find, Find Next, Find Previous work, and so do Cmd+F, Cmd+G and Shift+Cmd+G.

### Editing

- LONG: *Primary*+Right on a page turns it clockwise, sharp after a moment; its thumbnail turns too, and the title says "Edited". *Primary*+Left twice turns it the other way.
- In Two Pages, rotating turns both pages of the spread.
- LONG: click thumbnail 3, Shift+click thumbnail 6, *Primary*+click thumbnail 4: pages 3, 5 and 6 are selected and the view stays on page 3. *Primary*+Right turns those three; scrolling the document keeps them selected. A plain click on a thumbnail selects just that page again.
- LONG: Delete on page 3 removes it and shows the next page; the menu says "Undo Delete Page". Select three thumbnails and press Backspace: all three go ("Undo Delete 3 Pages"). With every thumbnail selected, Delete only beeps.
- LONG: drag thumbnail 2 below thumbnail 4. While dragging, a line in the accent colour shows the place between thumbnails; after the drop the page is fourth, still selected, and the view hasn't moved. "Undo Move Page" puts it back.
- Select thumbnails 2 and 3, drag one of them down past the bottom of the sidebar: the sidebar scrolls; dropping moves both ("Move 2 Pages").
- Drag two PDFs from the file manager onto the thumbnails, between pages 3 and 4: a line shows the place, then their pages are inserted there, in order, and selected; one Undo ("Undo Insert N Pages") removes them all. Dropped on the page view instead, they open in new windows. A non-PDF file dropped on the thumbnails gives "Couldn't open … not a valid PDF file" and inserts nothing.
- Dropping a thumbnail where it already is changes nothing (no "Edited"). 
- LONG and LINKS in two windows: select two thumbnails in LONG and drag them onto LINKS's thumbnails. They're inserted where dropped, selected, with any rotation they had; LONG still has them; "Undo Insert 2 Pages" in LINKS removes them. Saved, LINKS has the pages.
- LABELS: delete "ii" and move "1" to the end. The page entry and thumbnails show i, iii, iv, 2, 3, a, b, 4, 5, 1. Saved and reopened (here and in another viewer), the labels are the same. Typing "1" goes to the last page.
- LABELS with SIZES inserted after "iv": the inserted pages are numbered by their position (5, 6, …); the others keep their labels.
- Typing in the page entry or the search field, Delete and Backspace edit the text and delete no pages.
- LONG with pages deleted, saved and reopened: the pages are gone; Contents entries that pointed at them do nothing, the others still work.
- With the sidebar focused, *Primary*+A selects every thumbnail, not the page text; with the document focused, it selects the text.
- The main menu shows "Undo Rotate Page" and "Redo Rotate Page" as appropriate; undoing every edit removes "Edited".
- LONG, on page 2: Insert Pages from File, choosing LINKS. Its pages follow page 2 and the view is on the first of them; the page count grows, the page entry shows plain numbers instead of LONG's printed ones, and LINKS's in-document links lead to its own (inserted) pages. Undo removes them.
- Inserting ENCRYPTED asks for its password; Cancel inserts nothing.
- With a search open, inserting a file: matches on the new pages are found too.
- LONG with LINKS inserted, saved: reopened, both are there; LONG's Contents still work. The file opens in other viewers.
- On a rotated page: selecting text, searching (highlights on the words), links and Contents entries still work.
- Typing in the page entry, *Primary*+Left moves the cursor by a word and *Primary*+Z undoes the typing; no page is rotated or edit undone.
- macOS: Edit → Undo and Redo, and Tools → Rotate Left and Rotate Right, with Cmd+Z, Shift+Cmd+Z, Cmd+Left and Cmd+Right.
- A PDF over 64 MB (a scanned book, say) opens at once, and the app's memory use stays well below the file's size.

### Markup

Work on copies of the test files.

- LONG: select a few words, Shift+*Primary*+H. They turn yellow, the text under the yellow stays black and sharp (not greyed or washed out), and the selection is gone. Zoom in and out and scroll: the highlight stays exactly on its words, also while zooming.
- Select from the middle of one line to the middle of the next and highlight: each line is covered just as far as it was selected.
- Underline (Shift+*Primary*+U) and Strikethrough (Shift+*Primary*+X): a red line under and through the words, at a sensible height for the text size.
- Select text over a page break and highlight: both pages are marked, and one *Primary*+Z takes both away.
- Without a selection, Highlight shows "Select the text to mark first".
- Click a highlight: a frame appears around it. Delete removes it and no page; Escape lets go of it instead. A click on a link under markup still follows the link.
- ROTATED: highlights lie on the text there too. Rotate a page with a highlight (*Primary*+Right): the highlight turns with it.
- Move a page with a highlight in the sidebar, and delete one and undo: the highlight stays with its page.
- Save, close, open again: the markup is there and looks the same as before saving (compare closely: colour, position, the height of the lines).
- Markup the file already has: click one of the reopened highlights and press Delete. It disappears, nothing else on the page changes or moves, and the thumbnail follows. *Primary*+Z brings it back. Save and reopen: it is gone.
- A file with other programs' annotations (`corpus/files/pdfjs/test/pdfs/freetexts.pdf`, `inks.pdf`, `comments.pdf`): each can be picked and deleted, one at a time, and the others stay as they were. Deleting one that has a reply or a popup takes those along.
- LONG, a highlight deleted on one page: scrolling, zooming, selecting text, links and search on that page work as before.
- The saved file in other programs (Papers or Evince, Firefox, Preview on macOS): the markup shows, in the same place and colour, and can be selected and deleted there. In a list of annotations it shows the marked text.
- ENCRYPTED: highlight, save, reopen with the password: the highlight is there.
- The markup bar (the pencil in the header bar): with Highlight pressed in, every selection is marked as soon as the button is let go, also a double-clicked word. Scrolling, links and picking markup still work with a tool pressed in. Pressing the tool again, or closing the bar, stops it.
- Colours: choose green with Highlight pressed in, mark something: green. Switch to Underline: the bar shows red; back to Highlight: green again.
- Pick a highlight: the bar shows its colour. Choose another: it changes at once, and *Primary*+Z changes it back. The same for a highlight the file already had: nothing else on the page changes.
- The swatches are easy to tell apart and to hit, in the light and the dark style; the one in use is visibly marked.
- Thumbnails show new markup at once, and lose it on undo.
- Freehand: draw a signature-like scribble with the mouse and with a touchpad. The line follows the pointer without lag, looks smooth rather than angular, and stays the same after the button is let go (no visible jump when it is thinned out).
- Draw across the edge of a page: the stroke stops at the edge. Draw with the page zoomed far in and far out: the line keeps its thickness relative to the page.
- Rectangle, ellipse and line: the shape follows the pointer while dragging, in any direction. Escape during a drag gives it up.
- With a drawing tool pressed in, scrolling with the wheel or touchpad still works, and text is not selected.
- Pick a drawn line by clicking on it. A click inside the area it spans, away from the line, picks nothing.
- Save and reopen: drawings and shapes look the same as before saving. Open the file in other programs too: they show, and can be moved and deleted there.
- Print (to a file is enough) with unsaved markup: it is on the printed page, in the same place and colour.
- Two windows: drag the thumbnail of a marked page into the other window's sidebar: the markup is there too, and one undo there removes page and markup.

### Saving

Work on copies of the test files.

- LONG: rotate a few pages, *Primary*+S. "Edited" goes away; reopened, the pages are rotated, and the Contents, links and printed page numbers still work. The file opens in other viewers too (Evince or Papers on Linux, Preview on macOS).
- ENCRYPTED: rotate a page and save: reopening asks for the password again, and `secret` still works.
- Save As into another folder: the window shows the new name; the original file is unchanged.
- LONG: select thumbnails 2–4 and 7, Export Selected Pages: the suggested name is "long (pages 2-4, 7).pdf". The exported file has those four pages (rotated if they were), opens in other viewers, and the window still shows LONG, with its edits. With one page selected, only that page is exported.
- With changes, close the window (*Primary*+W) or quit (*Primary*+Q): "Save Changes?" appears. Cancel keeps the window; Discard closes without saving; Save saves, then closes. With several edited windows, quitting asks for each one.
- Saving into a folder you can't write to: a message "Couldn't save … Permission denied", and the document stays "Edited".
- A file that is a symbolic link: after saving, the link is still a link, pointing at the updated file.
- GIANT: saving takes a few seconds and the window stays responsive meanwhile.

### Printing

Print to a real printer where you can; otherwise "Print to File" (PDF) shows the same.

- LONG: *Primary*+P, all pages to a file: the file has 352 pages in order, each centred on the paper, landscape pages (if any were rotated) on landscape paper. The window stays responsive while it prints.
- Rotate page 2, then print the current page (page 2): it's printed turned, on landscape paper.
- Select thumbnails 3, 5 and 9, *Primary*+P, choose "Selection": only those three. A range such as 10–12 prints those pages.
- With pages inserted from another file, they're printed in their place.
- Print again: the dialog starts with the printer and settings used last time.
- GIANT: prints (slowly), at the paper size, without freezing the window.
- macOS: File → Print, and Cmd+P. Whether real printers appear depends on how Homebrew's GTK was built; note what the dialog offers.

### Changes by other programs

Use a copy of LONG, and another program that saves PDFs (`qpdf`, a PDF editor, or just `cp`).

- LONG open on page 50, unedited: replace the file (`qpdf --rotate=+90:50 long.pdf --replace-input`). The window shows the new version at once, still on page 50 at the same zoom, page 50 now turned; no "Edited".
- With an edit, the same: a banner "“long.pdf” was changed by another program…" with Reload; the edit stays. Save asks "File Changed on Disk": Cancel leaves the other program's version (check in another viewer), Save Anyway replaces it. Reload shows the other version and drops the edit.
- Saving in Specto itself, then waiting a few seconds: no banner, no reload, Undo still there.
- Delete or move the file: a banner says so. Save writes it back where it was.
- ENCRYPTED, replaced by another encrypted copy with the same password: reloaded without asking; with a different password, the password dialog comes up.

### Flatpak

Build and install it with `packaging/flatpak/build.sh` (see packaging/README.md), then start it from the GNOME app grid or with `flatpak run net.xfbs.specto`. The app sees only the files it's given, so these go through the document portal.

- The app grid shows it with its icon; GNOME Files lists it under "Open With" for PDFs, and opening a PDF that way works.
- Open a PDF with the Open dialog, from GNOME Files, and by dropping it on the window. The start page later lists it with its real folder (not `/run/user/…`); the Save As and Export dialogs start in that folder.
- Rotate a page, save, open the file in another viewer: turned. No leftover hidden files next to it.
- Insert Pages from File, Export Selected Pages and Print work like outside the sandbox.
- Change the file with another program while the window is in the background, then switch back to it: the new version is shown (or with edits, the banner).

### Remembering state

- LONG: go to page 200, switch to Two Pages, zoom to 150%, open the Contents tab, close, reopen. All of it is restored, at the same spot on page 200.
- Fit width, closed and reopened in a different window size, is still fit width.
- A file never opened before starts on page 1, continuous, fit width, with the sidebar shown.
- With three documents open, *Primary*+Q: all three remember their state.

### Several windows and slow pages

- Opening the same file twice gives two independent windows.
- GIANT: a spinner on a small rounded card over the page while it renders, gone once the page is sharp (a few seconds). Meanwhile menus, resizing and scrolling stay responsive.
- GIANT and LONG open together: LONG scrolls and renders normally.
- Normal documents never show a spinner.

## Platform notes

- macOS: the app has a menu bar (File, Edit, View, Go, Tools, Help) in the system menu bar, with the shortcuts shown; GTK draws its own window decorations. Shortcuts should work with Cmd (and also with Ctrl). Opening PDFs from Finder with "Open With" isn't expected to work yet, since there's no app bundle; use the command line, the dialog or drag-and-drop. Retina screens are scale 2.
- Linux: fractional scaling needs to be enabled in GNOME (Settings → Displays). Recently opened PDFs should also appear in GNOME Files under Recent.

## Reporting

For each failure: the feature (section and bullet in FEATURES.md) or check (section here), the platform and OS version (e.g. "macOS 26.1, M2" or "Fedora 44, GNOME 49"), what happened instead, and a screenshot if it's visual. For crashes, run from a terminal and include the output.

The "Not yet" section of FEATURES.md lists what's missing on purpose; those aren't bugs.
