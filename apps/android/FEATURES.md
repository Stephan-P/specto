# Android app: features

What the app does today, written as things to check by hand. Update this file in the same commit as any change to a feature. What is planned but not built is in [docs/TASKS.md](../../docs/TASKS.md) (M6); the short list at the end only names what a tester might look for and not find.

Most rows are also checked automatically on a phone-sized and a tablet-sized emulator (the tests in `test-ui`), but automated tests can't judge how anything looks or feels, and they don't cover the file picker, opening from another app, the share sheet, the print dialog beyond what it is handed, pinch zoom, dark mode, the camera cutout, whether the screen really stays on, the system's dialog asking where to save, saving over a file from the system's file picker, how dragging pages looks, or how the selection, its handles and the copy menu look.

Useful documents for a full round: a long book with an outline and links, a document mixing portrait and landscape pages, a scan or image-heavy file, a one-page file, a password-protected file, and something that is not a PDF.

## Opening

| Feature | How to check | Expected |
|---|---|---|
| Start screen | Launch the app from the launcher | "Open PDF" button, top bar with only the ⋮ menu (Open and Settings); below it "Recent", once documents have been opened |
| Recent documents | Open a few files with "Open PDF", then relaunch the app (or ⋮ → "Close") | Up to 20 documents, newest first, each with the page it was left on and when; tap one to reopen it. Files handed over by another app are usually not listed, because the app isn't allowed to read them again later |
| Continue reading | Read to some page, zoom or switch to continuous scrolling, leave the app, open the same file again (by any route) | It opens where it was left: same page and position, same mode and zoom. The file is recognised by its content, so a copy or a renamed file continues too |
| A recent file is gone | Delete or move a listed file, then tap it | "Couldn't open this file", and it disappears from the list |
| Close | ⋮ → "Close" | Back to the start screen |
| Pick a file | "Open PDF", or ⋮ → "Open" | System file picker limited to PDFs; the document opens on page 1 with its file name in the top bar |
| Open from another app | In Files or a browser, open a PDF and choose Specto | The document opens; if another one was open, it is replaced |
| Password | Open a protected file | Password dialog; a wrong password shows "That password didn't work." with the field empty again; Cancel leaves an explanation and "Open another PDF" |
| Broken file | Open something that isn't a PDF (e.g. renamed) | "Couldn't open this file" with "Open another PDF"; no crash |

## Sharing

| Feature | How to check | Expected |
|---|---|---|
| Share | With a document open, ⋮ → "Share", then pick an app (a mail or messaging app, a cloud drive) | The system's share sheet, naming the file; the app picked receives the whole file under its name, unchanged (a protected file stays protected). The reader is still on the same page afterwards. Try it with a file picked in the app and with one opened from another app With unsaved edits (see "Editing the pages"), what is shared is the document as edited, and sharing saves nothing |
| Only for documents | Open the ⋮ menu on the start screen, and after a file failed to open | No "Share" |

## Printing

| Feature | How to check | Expected |
|---|---|---|
| Print | With a document open, ⋮ → "Print" | After a moment ("Preparing to print…" for a large document) the system's print dialog, under the document's name, with a preview of every page. Printer, copies, paper, and which pages are the system's to offer; "Save as PDF" writes them to a file. Text stays text: the pages are handed over as they are, not as pictures |
| As edited | Rotate, delete or move pages, then print without saving | The preview shows the document as edited. Printing saves nothing: the title still says "Edited" |
| Leave | Back, or cancel in the dialog | Back in the document, on the same page |
| Wide pages | Print a document whose first page is wider than tall | The dialog starts with the paper lying on its side |
| Protected documents | Open a password-protected file and print it | The preview shows the pages: the print service gets a copy without the password, which is deleted when the dialog closes. The file itself stays protected |

## The top bar

| Feature | How to check | Expected |
|---|---|---|
| Buttons | Open a document | Left: Pages and Contents. Right: search (a magnifying glass), the view button, whose icon is a small picture of the current layout, and ⋮ |
| View menu | Tap the view button | The layouts, each with its picture ("Single page" and "Continuous"; on wide windows also "Two pages" and "Two pages, continuous"), then "Fit page" and "Fit width". The current layout and fit are ticked. Everything in this menu is remembered for the document |
| ⋮ menu | Tap ⋮ | Open, Share, Print, Save as, Go to page, Properties, Settings, Close: what isn't about how the document is shown. After an edit also "Save" (where the file can be written over) and "Undo …", and after undoing "Redo …" |
| Properties | ⋮ → "Properties" | A dialog with the file name, what the document says about itself (title, author, subject, keywords, the programs it was made with, when it was created and changed; whatever it doesn't state is left out), the number of pages, their size ("210 × 297 mm (A4)"; a note if pages differ), the file size and the PDF version. Values can be selected and copied |

## Reading, one page at a time (default)

| Feature | How to check | Expected |
|---|---|---|
| Fitted pages | Open a document with mixed page sizes | Every page is shown whole and as large as fits, whatever its size or orientation |
| Swipe | Drag left or right | The page follows the finger with the next one beside it; past half way, or with a flick, it turns; otherwise it slides back |
| One page per swipe | Flick as fast as you like | Exactly one page turns |
| Ends | Swipe back on page 1, forward on the last page | Nothing moves |
| Pinch zoom | Pinch in and out on a page | Zooms around the fingers; a page can't get smaller than fitted and re-centres when released |
| Pan and fling | Zoom in, drag, then flick | Moves within the page; a flick stops at the page edge and does not turn the page |
| Turn while zoomed | Zoomed in at the page edge, drag further | The next page slides in already fitted, and once it arrives the view is fitted again (going back, the page left behind is fitted too). Going to another page any other way (keys, the strip, a link, "Go to page") also returns to the fitted view |
| Double tap | Double-tap a fitted page, then again | Zooms to 2.5× around the tap, then back to fitted |
| Sharpness | Zoom to any level and wait a moment | Text sharpens about a quarter of a second after the zoom stops |

## Reading, continuous scrolling

| Feature | How to check | Expected |
|---|---|---|
| Switch | View button → "Continuous" | One column of pages fitted to the width, on the same page as before; switching back ("Single page") keeps the page too |
| Scroll | Drag and flick up and down | Smooth scrolling that slows down by itself |
| Zoom | Pinch, double tap | As in paged mode; double tap returns to fit width |
| Remembered per document | Switch a document to continuous, open another, come back | New documents open paged; a document opened before opens in the mode it was left in |

## Fit page and fit width

| Feature | How to check | Expected |
|---|---|---|
| Fit width | Hold the phone sideways (or use a tablet), view button → "Fit width" | Pages fill the width, in either layout. One page at a time, a page is now taller than the screen: drag to read down it, swipe sideways to turn; the next page starts at its top, fitted to the width again |
| Fit page | View button → "Fit page" | Every page is shown whole; with "Continuous", whole pages scroll past |
| Undoes zooming | Zoom in, then pick the ticked fit again | Back to the fitted view. Double tap also returns to the chosen fit |
| Starts as suits the layout | Switch between "Single page" and "Continuous" | Single pages start whole, continuous scrolling starts fitted to the width; a different choice lasts until the layout is switched |
| Remembered | Pick "Fit width", leave the app, open the document again | Still fitted to the width |

## Finding your way

| Feature | How to check | Expected |
|---|---|---|
| Page indicator | Turn pages or scroll | "n / total" above the strip follows; it reaches the last page at the end |
| Thumbnail strip | Look at the bottom of the normal view | Row of page thumbnails with the current page outlined in the middle; it follows shortly after a page turn; absent for one-page documents |
| Flip through | Scroll the strip sideways, slowly and with a flick | The page passing the middle of the strip is shown as it passes, so the document flips along; tapping a thumbnail goes straight there |
| Pages | Top-left button | Grid of all pages, current one outlined, starting near it; tap a page to go there; Back or the arrow returns |
| Contents | Top-left button, "Contents" tab (documents with an outline) | Collapsible outline with page numbers, the current section highlighted; tap an entry to go there; long outlines start collapsed to their top level |
| Tabs | Swipe between Pages and Contents, or tap the tab names | Both work; reopening shows the tab used last; documents without an outline show only the grid, without tabs |
| Go to page | ⋮ → "Go to page" | Field showing the valid range; "Go" is disabled for input that names no page |
| Printed page numbers | Open a document numbered i, ii, iii, 1, 2 … (or with an appendix "A-1") | The indicator shows the printed number with the position, e.g. "iii (3 / 240)"; Pages and Contents show printed numbers; "Go to page" accepts a printed number first ("iii", "A-2", "3") and otherwise a position, and says which page it found |
| Links in the document | Tap a cross-reference or a table-of-contents line | Jumps to the target |
| Links to the web | Tap a web or e-mail link | Dialog showing the address; opens only after "Open"; other kinds of links say they can't be opened |

## Searching

Needs a document with real text, like selecting.

| Feature | How to check | Expected |
|---|---|---|
| Search | Tap the magnifying glass and type a word | The top bar becomes a search field with the keyboard up. Matches are highlighted in yellow as you type, the one being looked at in orange, and the view goes to the first match from the current page on, turning or scrolling as needed |
| Count | Look right of the field | "3 of 17": which match is shown, and how many there are. The first search reads the whole document in the background, so for a moment the count grows and ends in "…"; later searches are answered at once |
| Next and previous | The arrows right of the count | The next or previous match, across pages; after the last comes the first again, and before the first the last |
| Case and accents | Search "resume" in a text that has "Résumé" | Found: capitals and accents don't matter, nor does a line break or a hyphen at a line's end inside what is searched for |
| Nothing found | Search for something that isn't there | "No matches", and the arrows are greyed out |
| Put the keyboard away | The keyboard's search key, or a tap on the page | The keyboard goes, the search and its highlights stay, and the arrows still work |
| Leave | The arrow left of the field, or Back (once the keyboard is away) | The usual top bar returns and the highlights go; the view stays where the search led |
| Scans | Search in a scanned document without a text layer | "No matches" |

## Selecting and copying text

Needs a document with real text; a scan without a text layer has nothing to select.

| Feature | How to check | Expected |
|---|---|---|
| Select a word | Long-press a word | A short vibration; the word is highlighted, with a round handle below each end, and the system's menu with "Copy", "Share" and "Select all" appears beside it |
| Not on blank paper | Long-press a margin or a picture | Nothing is selected, and the bars stay as they are |
| Word by word | Long-press a word and, without lifting the finger, move along the line or down the page | The selection grows and shrinks by whole words; the menu appears when the finger lifts |
| Handles | Drag the handle at either end | That end follows the finger letter by letter, also on to other lines; the other end stays. The ends can't be dragged past each other into nothing |
| Stays with the text | Select something, then scroll, zoom, turn the page and come back, rotate | The same text is selected; the menu leaves while things move and returns when they rest |
| Across pages | With "Continuous", drag a handle (or the long press) to the top or bottom edge | The view scrolls on while the finger stays there, faster the nearer the edge, and the selection grows with it over as many pages as you like |
| Copy | "Copy" in the menu | The text is on the clipboard, lines and pages separated by line breaks (paste it somewhere to check), and the selection is gone. Android 13 and later show what was copied; earlier versions say "Copied" |
| Select all | "Select all" in the menu | The whole document is selected; "Copy" then copies all of it. A very long document can be more than the clipboard takes: then a message says so |
| Share the text | "Share" in the menu, then pick an app (notes, mail, a messenger) | The system's share sheet with the text; the app picked receives it as plain text, lines and pages separated by line breaks, and the selection is gone. More than about a hundred thousand characters (some thirty full pages) is more than can be handed over: then a message says so, and "Copy" still works for up to three times as much |
| Let go | Tap the page, or press Back | The selection and the menu go; nothing else happens (no full screen, no link followed, and Back doesn't leave) |

## Editing the pages

Pages can be turned, removed and put in another order. Nothing changes a file until the document is saved: over the file it was opened from where the app may write to it, or as a new one.

| Feature | How to try it | What should happen |
|---|---|---|
| Pick pages | Open the page grid (top left; on a tablet it is beside the document) and long-press a thumbnail | A short vibration; the page is tinted, with a tick in its corner, and the top bar counts the pages picked ("1 selected") and offers what can be done with them. From then on a tap picks further pages or lets go of them; nothing jumps to a page |
| Let go | ✕ in the bar, or Back; or tap the picked pages until none is left | The usual top bar is back, and taps go to pages again. Back leaves the grid only after that |
| Select all | With a page picked, ⋮ → "Select all" | Every page is picked, and the bin is greyed out: a document keeps at least one page |
| Rotate | Pick pages, then one of the two round arrows | The pages turn by a quarter that way, in the grid and in the document; they stay picked, to be turned further. The title says "Edited" |
| Delete | Pick pages, then the bin | They are gone; the pages after them move up and keep their printed numbers, and the count ("3 / 11") follows. Nothing is picked any more |
| Reorder | Long-press a thumbnail and, without lifting the finger, drag it to another place | The page fades where it was and a count ("1 page") floats above the finger; a bar between two thumbnails shows where it would go. Lifting the finger moves it there, and it stays picked. Near the top or bottom edge the grid scrolls on |
| Reorder several | Pick several pages, then long-press one of them and drag | All of them move to the place shown, in the order they were in |
| A long press that goes nowhere | Long-press a picked page and lift the finger | It is let go of, as with a tap |
| Undo and redo | ⋮ → "Undo delete" (or rotate, or move), then "Redo …" | One edit at a time is taken back, or done again, in the order they were made. Back at the document as it was opened or saved, "Edited" goes |
| Save | With unsaved edits to a document opened with "Open" (the system's file picker): ⋮ → "Save" | The file is written over with the document as edited (a new PDF is written beside it first, so a failure leaves the file as it was). "Edited" goes; the document stays open where it was, and the edits can still be undone, which makes it "Edited" again |
| Save only where it can work | Open a PDF from another app (a mail attachment, a download notification), edit it, and look at the ⋮ menu; also look without edits | No "Save" for a document the app was only given to read, and none while there is nothing to save. "Save as" is always there |
| Save as | ⋮ → "Save as", then choose where and under what name (the document's own is suggested) | The document as edited is written there as a new PDF, and is then the one that is open, where the reader was, with nothing left to undo. The file opened before is untouched, and both are on the start screen. Saving over the file that is open works too: it is then listed once, as saved |
| Asked before edits are lost | With unsaved edits: ⋮ → "Close", or "Open" and pick a file, or open a PDF from another app | "Save changes?" with "Save" (or "Save as", where the file can't be written over), which saves and then goes on to what was asked for, "Discard" (goes on without saving) and "Cancel" (stays) |
| Not asked otherwise | The same without edits, or after saving | No question |
| Kept only while the app lives | Edit, leave the app, and come back later | The edits are still there unless Android ended the app in the meantime, in which case the document is back as it was last saved. There is no automatic saving yet |

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
| Two pages | View button → "Two pages" or "Two pages, continuous" | Pages are shown in pairs, fitted together, and the page indicator names both ("4–5 / 12"); a swipe turns a pair, or the pairs form one scrolling column. These layouts are only offered on wide windows |
| First page alone | In a two-page layout, view button → "First page alone" | The first page stands by itself and the pairs shift by one (2–3, 4–5, …), so a book's left and right pages land left and right |
| Remembered | Turn on two pages, leave the app, open the document again | Still two pages, on the same pair; other documents are unaffected |
| Falls back | Rotate to portrait, or put the app in split screen | The phone layout: Pages and Contents full screen on request, the strip at the bottom, one page at a time even if two pages are on (they return with the room); same document and page throughout |

## Keyboard

With a keyboard attached (tablets, Chromebooks, Bluetooth keyboards). The page takes the keyboard focus when a document opens.

| Feature | How to check | Expected |
|---|---|---|
| Turn pages | → and ←, Page Down and Page Up, Space and Shift+Space, Backspace | Next or previous page; with two pages, the next or previous pair; nothing at either end. In continuous scrolling, → and ← go to the top of the next or previous page, while Page Down, Page Up and Space scroll most of a screenful |
| Read down a tall page | With "Fit width" on a wide screen (or zoomed in), Page Down or Space repeatedly | Most of a screenful further down the page each time; at its end, the next page. Page Up likewise, upwards |
| Scroll | ↓ and ↑ | A small scroll; one page at a time, where the page has no more to show in that direction (always, if it is shown whole), they turn the page instead |
| Ends | Home and End | First and last page |
| Zoom | Ctrl with +, − and 0 | Zoom in and out around the middle of the view, and back to the fitted view |
| Select and copy | Ctrl+A, Ctrl+C, Escape | Selects all text, copies the selection, lets go of it |
| Undo and redo | Ctrl+Z; Shift+Ctrl+Z or Ctrl+Y | Takes back the last edit of the pages; does it again |
| Save | Ctrl+S | Saves over the file if there are edits and that is possible, else asks where ("Save as") |
| Print | Ctrl+P | The system's print dialog, as with ⋮ → "Print" |
| Search | Ctrl+F; in the search field Enter, Shift+Enter and Escape | Opens the search; next match, previous match, and leaving it |

## Settings

⋮ → "Settings", with or without a document open. They hold for the whole app and are kept; both switches are off at first. Back, or the arrow, returns to where you were.

| Feature | How to check | Expected |
|---|---|---|
| Keep the screen on | Switch it on, open a document and leave the phone alone | The screen stays on for ten minutes after the last touch or key, then goes dark after the system's usual wait. On the start screen, and with the switch off, it goes dark as usual |
| Volume keys turn pages | Switch it on, open a document, press volume down and volume up | Volume down goes on, volume up goes back, like Page Down and Page Up: a page turn, or most of a screenful when scrolling or on a page taller than the screen. Holding a key keeps going. The volume doesn't change |
| Crash log | Look at the last row of Settings | "Nothing went wrong so far", greyed out, unless something did: then when the last entry was written. Tapping it shows the log, headed by the app's version and the device: what ended the app (an error nothing caught) and errors inside the PDF library, which mostly end nothing. "Share" hands the text to another app, for a bug report; "Delete" empties the log. It stays on the device unless shared, and only its newest entries are kept. A crash outside the app's own code (out of memory, say) leaves no entry |
| Volume keys otherwise | With the switch on: on the start screen, in Settings, in Pages or Contents on a phone, or with a menu or dialog open; and anywhere with the switch off | The keys change the volume as usual |

## Behaviour of the app as a whole

| Feature | How to check | Expected |
|---|---|---|
| Language | Set the device to German; or, on Android 13 and later, leave it as it is and choose German for Specto alone (system settings → Apps → Specto → Language) | Menus, dialogs, messages and Settings are in German, dates and numbers as Germans write them; any other language gets English. Still English in either: what a failed save or open gives as the reason, and the crash log |
| Rotation | Rotate while reading, zoomed, and with Pages or Contents open | Same document and page; pages refit; nothing reloads |
| Landscape | Hold the phone sideways | The page view extends beside the camera cutout; the top bar, Pages, Contents and dialogs keep clear of it |
| Dark mode | Switch the system between light and dark, with the app open and closed | Bars, backgrounds and dialogs follow (on Android 12+ with the system's colours); pages stay as the document defines them |
| Background | Switch to another app and back | Same place; pages may re-render briefly |
| Large documents | Scroll quickly through several hundred pages | Pages appear blurry first, then sharp; the app stays responsive |
| Large files | Open a file of a few hundred megabytes (a long scan), from the file picker and from another app | It opens about as quickly as a small one and reads on normally; reopened later, it continues where it was left. Files from 64 MB are not loaded into memory but read piece by piece as pages are shown |
| Slow pages | Open a huge scan (a page with more than 100 megapixels of images) or a page with a great many drawing operations | While such a page isn't sharp yet, a spinner on a small plate turns in the middle of what is visible of it; it goes once the page is sharp and comes back while new parts are drawn after zooming in. Ordinary pages never show one, and scrolling, zooming and long presses work as usual under it |

## Not there yet

Adding pages of other files, keeping unsaved edits when Android ends the app, annotations and form filling.
