# Testing the desktop app by hand

[FEATURES.md](FEATURES.md) is the checklist: go through every feature there on each platform (Linux and macOS). This file has the setup, and the checks that need a particular test file or whose expected result isn't obvious from the feature list.

## Automated checks

Much of the feature list is also checked automatically, in CI and locally (Linux, needs `mutter`; `ci/install-deps-fedora.sh --headless` installs the packages on Fedora):

```sh
cargo build -p pdfrs-desktop
tools/headless/run.sh python3 -I tools/headless/check.py
PDFRS_SCALE=2 tools/headless/run.sh python3 -I tools/headless/check.py   # also 1.25
tools/headless/run.sh python3 -I tools/headless/check.py zoom layouts    # some tests only
```

The app runs in a headless compositor with its own saved state, so yours isn't touched; the pointer and keyboard tests use a virtual pointer and keyboard in that compositor, never your own. What's covered is listed at the top of `tools/headless/check.py`. Not covered: pinch, dialogs, dragging thumbnails (the checks drop them through a test hook), how sharp the text looks, how the selection feels with a real mouse or touchpad, and macOS. Those still need the manual pass.

## Setup

All commands and paths are relative to the repository root.

```sh
cargo build --release -p pdfrs-desktop
target/release/pdfrs-gtk                 # start page
target/release/pdfrs-gtk a.pdf b.pdf     # one window per file
```

macOS needs GTK 4, libadwaita 1.6 or newer and the Adwaita icons from Homebrew: `brew install gtk4 libadwaita adwaita-icon-theme`. Without the icon theme, header bar and sidebar icons are missing.

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

- View state: delete `~/.local/state/pdfrs/view-state.ini` (or `$XDG_STATE_HOME/pdfrs/view-state.ini`). Same path on macOS.
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

- macOS: the app has a menu bar (File, Edit, View, Go, Tools) in the system menu bar, with the shortcuts shown; GTK draws its own window decorations. Shortcuts should work with Cmd (and also with Ctrl). Opening PDFs from Finder with "Open With" isn't expected to work yet, since there's no app bundle; use the command line, the dialog or drag-and-drop. Retina screens are scale 2.
- Linux: fractional scaling needs to be enabled in GNOME (Settings → Displays). Recently opened PDFs should also appear in GNOME Files under Recent.

## Reporting

For each failure: the feature (section and bullet in FEATURES.md) or check (section here), the platform and OS version (e.g. "macOS 26.1, M2" or "Fedora 44, GNOME 49"), what happened instead, and a screenshot if it's visual. For crashes, run from a terminal and include the output.

The "Not yet" section of FEATURES.md lists what's missing on purpose; those aren't bugs.
