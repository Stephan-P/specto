# Android app: features

What the app does today, written as things to check by hand. Update this file in the same commit as any change to a feature. What is planned but not built is in [docs/TASKS.md](../../docs/TASKS.md) (M6); the short list at the end only names what a tester might look for and not find.

Useful documents for a full round: a long book with an outline and links, a document mixing portrait and landscape pages, a scan or image-heavy file, a one-page file, a password-protected file, and something that is not a PDF.

## Opening

| Feature | How to check | Expected |
|---|---|---|
| Start screen | Launch the app from the launcher | "Open PDF" button, top bar with only the ⋮ menu |
| Pick a file | "Open PDF", or ⋮ → "Open…" | System file picker limited to PDFs; the document opens on page 1 with its file name in the top bar |
| Open from another app | In Files or a browser, open a PDF and choose pdfrs | The document opens; if another one was open, it is replaced |
| Password | Open a protected file | Password dialog; a wrong password shows "That password didn't work."; Cancel leaves an explanation and "Open another PDF" |
| Broken file | Open something that isn't a PDF (e.g. renamed) | "Couldn't open this file" with "Open another PDF"; no crash |

## Reading, one page at a time (default)

| Feature | How to check | Expected |
|---|---|---|
| Fitted pages | Open a document with mixed page sizes | Every page is shown whole and as large as fits, whatever its size or orientation |
| Swipe | Drag left or right | The page follows the finger with the next one beside it; past half way, or with a flick, it turns; otherwise it slides back |
| One page per swipe | Flick as fast as you like | Exactly one page turns |
| Ends | Swipe back on page 1, forward on the last page | Nothing moves |
| Pinch zoom | Pinch in and out on a page | Zooms around the fingers; a page can't get smaller than fitted and re-centres when released |
| Pan and fling | Zoom in, drag, then flick | Moves within the page; a flick stops at the page edge and does not turn the page |
| Turn while zoomed | Zoomed in at the page edge, drag further | The next page slides in from its top; it has the same zoom |
| Double tap | Double-tap a fitted page, then again | Zooms to 2.5× around the tap, then back to fitted |
| Sharpness | Zoom to any level and wait a moment | Text sharpens about a quarter of a second after the zoom stops |

## Reading, continuous scrolling

| Feature | How to check | Expected |
|---|---|---|
| Switch | ⋮ → "Continuous scrolling" (tick shown when on) | One column of pages fitted to the width, on the same page as before; switching back keeps the page too |
| Scroll | Drag and flick up and down | Smooth scrolling that slows down by itself |
| Zoom | Pinch, double tap | As in paged mode; double tap returns to fit width |
| Not remembered | Open another document | It opens paged again |

## Finding your way

| Feature | How to check | Expected |
|---|---|---|
| Page indicator | Turn pages or scroll | "n / total" above the strip follows; it reaches the last page at the end |
| Thumbnail strip | Look at the bottom of the normal view | Row of page thumbnails, current page outlined and brought to the middle shortly after a page turn; scroll it sideways; tap a thumbnail to go there; absent for one-page documents |
| Pages | Top-left button | Grid of all pages, current one outlined, starting near it; tap a page to go there; Back or the arrow returns |
| Contents | Top-left button, "Contents" tab (documents with an outline) | Collapsible outline with page numbers, the current section highlighted; tap an entry to go there; long outlines start collapsed to their top level |
| Tabs | Swipe between Pages and Contents, or tap the tab names | Both work; reopening shows the tab used last; documents without an outline show only the grid, without tabs |
| Go to page | ⋮ → "Go to page…" | Number field with the valid range; "Go" is disabled for invalid input |
| Links in the document | Tap a cross-reference or a table-of-contents line | Jumps to the target |
| Links to the web | Tap a web or e-mail link | Dialog showing the address; opens only after "Open"; other kinds of links say they can't be opened |

## Full screen

| Feature | How to check | Expected |
|---|---|---|
| Hide the bars | Tap the page where there is no link | Top bar, strip, page indicator and status bar disappear after a brief moment; the navigation handle stays |
| Whole screen | Zoom in while in full screen | The page uses the entire screen, including the area around the camera cutout; the background has one colour throughout |
| Page number | Turn a page in full screen | "n / total" fades in at the bottom and out again after about a second and a half |
| Show them again | Tap again, or press Back | Everything returns, on the same page |
| Still usable | Swipe, zoom and tap links in full screen | All work as in the normal view |

## Behaviour of the app as a whole

| Feature | How to check | Expected |
|---|---|---|
| Rotation | Rotate while reading, zoomed, and with Pages or Contents open | Same document and page; pages refit; nothing reloads |
| Landscape | Hold the phone sideways | The page view extends beside the camera cutout; the top bar, Pages, Contents and dialogs keep clear of it |
| Dark mode | Switch the system between light and dark, with the app open and closed | Bars, backgrounds and dialogs follow (on Android 12+ with the system's colours); pages stay as the document defines them |
| Background | Switch to another app and back | Same place; pages may re-render briefly |
| Large documents | Scroll quickly through several hundred pages | Pages appear blurry first, then sharp; the app stays responsive |

## Not there yet

Printed page numbers (a document numbered i, ii, iii, 1, 2 … is shown as 1, 2, 3 …), search, text selection and copy, saving and sharing, annotations and form filling, page reordering, recent files, remembering the position in a document, two-page view and a tablet layout, printing, and dragging along the strip to flip pages.
