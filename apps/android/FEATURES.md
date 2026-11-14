# Android app: features

What the app does today, written as things to check by hand. Update this file in the same commit as any change to a feature. What is planned but not built is in [docs/TASKS.md](../../docs/TASKS.md) (M6); the short list at the end only names what a tester might look for and not find.

Most rows are also checked automatically on a phone-sized and a tablet-sized emulator (`ReaderTest` and `DocumentTest` in `app/src/androidTest`), but automated tests can't judge how anything looks or feels, and they don't cover the file picker, opening from another app, the share sheet, pinch zoom, dark mode or the camera cutout.

Useful documents for a full round: a long book with an outline and links, a document mixing portrait and landscape pages, a scan or image-heavy file, a one-page file, a password-protected file, and something that is not a PDF.

## Opening

| Feature | How to check | Expected |
|---|---|---|
| Start screen | Launch the app from the launcher | "Open PDF" button, top bar with only the ⋮ menu; below it "Recent", once documents have been opened |
| Recent documents | Open a few files with "Open PDF", then relaunch the app (or ⋮ → "Close") | Up to 20 documents, newest first, each with the page it was left on and when; tap one to reopen it. Files handed over by another app are usually not listed, because the app isn't allowed to read them again later |
| Continue reading | Read to some page, zoom or switch to continuous scrolling, leave the app, open the same file again (by any route) | It opens where it was left: same page and position, same mode and zoom. The file is recognised by its content, so a copy or a renamed file continues too |
| A recent file is gone | Delete or move a listed file, then tap it | "Couldn't open this file", and it disappears from the list |
| Close | ⋮ → "Close" | Back to the start screen |
| Pick a file | "Open PDF", or ⋮ → "Open…" | System file picker limited to PDFs; the document opens on page 1 with its file name in the top bar |
| Open from another app | In Files or a browser, open a PDF and choose pdfrs | The document opens; if another one was open, it is replaced |
| Password | Open a protected file | Password dialog; a wrong password shows "That password didn't work." with the field empty again; Cancel leaves an explanation and "Open another PDF" |
| Broken file | Open something that isn't a PDF (e.g. renamed) | "Couldn't open this file" with "Open another PDF"; no crash |

## Sharing

| Feature | How to check | Expected |
|---|---|---|
| Share | With a document open, ⋮ → "Share…", then pick an app (a mail or messaging app, a cloud drive) | The system's share sheet, naming the file; the app picked receives the whole file under its name, unchanged (a protected file stays protected). The reader is still on the same page afterwards. Try it with a file picked in the app and with one opened from another app |
| Only for documents | Open the ⋮ menu on the start screen, and after a file failed to open | No "Share…" |

## Reading, one page at a time (default)

| Feature | How to check | Expected |
|---|---|---|
| Fitted pages | Open a document with mixed page sizes | Every page is shown whole and as large as fits, whatever its size or orientation |
| Swipe | Drag left or right | The page follows the finger with the next one beside it; past half way, or with a flick, it turns; otherwise it slides back |
| One page per swipe | Flick as fast as you like | Exactly one page turns |
| Ends | Swipe back on page 1, forward on the last page | Nothing moves |
| Pinch zoom | Pinch in and out on a page | Zooms around the fingers; a page can't get smaller than fitted and re-centres when released |
| Pan and fling | Zoom in, drag, then flick | Moves within the page; a flick stops at the page edge and does not turn the page |
| Turn while zoomed | Zoomed in at the page edge, drag further | The next page slides in already fitted, and once it arrives the view is fitted again (going back, the page left behind is fitted too). Going to another page any other way (keys, the strip, a link, "Go to page…") also returns to the fitted view |
| Double tap | Double-tap a fitted page, then again | Zooms to 2.5× around the tap, then back to fitted |
| Sharpness | Zoom to any level and wait a moment | Text sharpens about a quarter of a second after the zoom stops |

## Reading, continuous scrolling

| Feature | How to check | Expected |
|---|---|---|
| Switch | ⋮ → "Continuous scrolling" (tick shown when on) | One column of pages fitted to the width, on the same page as before; switching back keeps the page too |
| Scroll | Drag and flick up and down | Smooth scrolling that slows down by itself |
| Zoom | Pinch, double tap | As in paged mode; double tap returns to fit width |
| Remembered per document | Switch a document to continuous, open another, come back | New documents open paged; a document opened before opens in the mode it was left in |

## Finding your way

| Feature | How to check | Expected |
|---|---|---|
| Page indicator | Turn pages or scroll | "n / total" above the strip follows; it reaches the last page at the end |
| Thumbnail strip | Look at the bottom of the normal view | Row of page thumbnails with the current page outlined in the middle; it follows shortly after a page turn; absent for one-page documents |
| Flip through | Scroll the strip sideways, slowly and with a flick | The page passing the middle of the strip is shown as it passes, so the document flips along; tapping a thumbnail goes straight there |
| Pages | Top-left button | Grid of all pages, current one outlined, starting near it; tap a page to go there; Back or the arrow returns |
| Contents | Top-left button, "Contents" tab (documents with an outline) | Collapsible outline with page numbers, the current section highlighted; tap an entry to go there; long outlines start collapsed to their top level |
| Tabs | Swipe between Pages and Contents, or tap the tab names | Both work; reopening shows the tab used last; documents without an outline show only the grid, without tabs |
| Go to page | ⋮ → "Go to page…" | Field showing the valid range; "Go" is disabled for input that names no page |
| Printed page numbers | Open a document numbered i, ii, iii, 1, 2 … (or with an appendix "A-1") | The indicator shows the printed number with the position, e.g. "iii (3 / 240)"; Pages and Contents show printed numbers; "Go to page…" accepts a printed number first ("iii", "A-2", "3") and otherwise a position, and says which page it found |
| Links in the document | Tap a cross-reference or a table-of-contents line | Jumps to the target |
| Links to the web | Tap a web or e-mail link | Dialog showing the address; opens only after "Open"; other kinds of links say they can't be opened |

## Full screen

| Feature | How to check | Expected |
|---|---|---|
| Hide the bars | Tap the page where there is no link | After a brief moment the top bar slides up and the strip slides down (about a fifth of a second) while the page glides to its new place; the page indicator and status bar go too; the navigation handle stays |
| Whole screen | Zoom in while in full screen | The page uses the entire screen, including the area around the camera cutout; the background has one colour throughout |
| Page number | Turn a page in full screen | "n / total" fades in at the bottom and out again after about a second and a half |
| Show them again | Tap again, or press Back | The bars slide back in and the page glides back, on the same page |
| Still usable | Swipe, zoom and tap links in full screen | All work as in the normal view |

## Tablets and other wide windows

Applies when the app's window is at least 840 dp wide: a tablet in landscape, but not in portrait or when sharing the screen.

| Feature | How to check | Expected |
|---|---|---|
| Sidebar | Open a document on a tablet in landscape | Pages, in one column (and Contents, if the document has an outline), are shown in a sidebar on the left, open from the start, with the document beside it; no thumbnail strip |
| Stays open | Tap a page or an outline entry in the sidebar | The document goes there; the sidebar stays, and its page list scrolls to keep the current page in view as you read on |
| Fold away | Top-left button | The sidebar slides out, the document takes the room and the thumbnail strip returns; the button brings the sidebar back. Whether it was open is remembered for the document |
| Full screen | Tap the page | The sidebar leaves along with the bars, and returns with them |
| Two pages | ⋮ → "Two pages" | Pages are shown in pairs, fitted together, and the page indicator names both ("4–5 / 12"); a swipe turns a pair, and with continuous scrolling the pairs form one column. The menu item only exists on wide windows |
| First page alone | With two pages on, ⋮ → "First page alone" | The first page stands by itself and the pairs shift by one (2–3, 4–5, …), so a book's left and right pages land left and right |
| Remembered | Turn on two pages, leave the app, open the document again | Still two pages, on the same pair; other documents are unaffected |
| Falls back | Rotate to portrait, or put the app in split screen | The phone layout: Pages and Contents full screen on request, the strip at the bottom, one page at a time even if two pages are on (they return with the room); same document and page throughout |

## Keyboard

With a keyboard attached (tablets, Chromebooks, Bluetooth keyboards). The page takes the keyboard focus when a document opens.

| Feature | How to check | Expected |
|---|---|---|
| Turn pages | → and ←, Page Down and Page Up, Space and Shift+Space, Backspace | Next or previous page; with two pages, the next or previous pair; nothing at either end. In continuous scrolling, → and ← go to the top of the next or previous page, while Page Down, Page Up and Space scroll most of a screenful |
| Scroll | ↓ and ↑ | A small scroll; on a fitted page in paged mode, where there is nothing to scroll, they turn the page instead |
| Ends | Home and End | First and last page |
| Zoom | Ctrl with +, − and 0 | Zoom in and out around the middle of the view, and back to the fitted view |

## Behaviour of the app as a whole

| Feature | How to check | Expected |
|---|---|---|
| Rotation | Rotate while reading, zoomed, and with Pages or Contents open | Same document and page; pages refit; nothing reloads |
| Landscape | Hold the phone sideways | The page view extends beside the camera cutout; the top bar, Pages, Contents and dialogs keep clear of it |
| Dark mode | Switch the system between light and dark, with the app open and closed | Bars, backgrounds and dialogs follow (on Android 12+ with the system's colours); pages stay as the document defines them |
| Background | Switch to another app and back | Same place; pages may re-render briefly |
| Large documents | Scroll quickly through several hundred pages | Pages appear blurry first, then sharp; the app stays responsive |

## Not there yet

Search, text selection and copy, saving, annotations and form filling, page reordering and printing.
