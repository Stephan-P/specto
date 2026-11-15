# Testing the Android app

The app is tested in two ways. Unit tests and UI tests cover much of
[FEATURES.md](FEATURES.md), the list of what the app does. The rest is tested
by hand, with the specific checks further down.

Either way, install the build prerequisites first, as explained in
[BUILDING.md](BUILDING.md).

## Automated tests

From `apps/android`:

```sh
# unit tests
./gradlew testDebugUnitTest

# UI tests on an emulator
ANDROID_SERIAL=emulator-5554 ./gradlew connectedDebugAndroidTest
```

Set the serial when a phone is plugged in as well, because a UI test run
installs and then removes the app on its target. The tests are in `test/` and
`test-ui/`, and the UI tests run on a phone-sized and a tablet-sized emulator.

CI runs the unit tests on every change, and the UI tests for pushes to the main
branch.

Not covered, and left to the manual pass:

- How anything looks or feels
- The system's file picker, and opening from another app
- The share sheet, and the print dialog beyond what it is handed
- Saving over a file, and the system's dialog asking where to save
- Pinch zoom
- Dark mode and the camera cutout
- Whether the screen really stays on
- How dragging pages looks, and how the selection, its handles and the text
  menu look

## Setup

Install a build on a phone or an emulator, as described in
[BUILDING.md](BUILDING.md). The release build is the one to judge speed by.

Useful documents for a full round:

- a long book with an outline and links
- a document with printed page numbers (i, ii, iii, then 1, 2, 3)
- a document mixing portrait and landscape pages
- a scan or image-heavy file, ideally one of a few hundred megabytes
- a one-page file
- a password-protected file
- something that is not a PDF

## Specific checks

Go through FEATURES.md for everything else. These are the checks that need a
particular setup, or whose expected result isn't obvious from the feature list.

### Opening files

- Read to some page, zoom or switch to continuous scrolling, leave the app, and
  open the same file again by another route. It opens where it was left. A copy
  of the file, or the file renamed, continues there too.
- Delete or move a file in the recent list, then tap it: "Couldn't open this
  file", and it disappears from the list.
- With a document open, open a PDF from another app (Files, a browser). It
  replaces the open one.
- A wrong password shows "That password didn't work." with the field empty
  again. Cancel leaves an explanation and "Open another PDF".
- A file of a few hundred megabytes, from the file picker and from another app:
  it opens about as quickly as a small one, and reopened later it continues
  where it was left.

### Reading

- Flick as fast as you like: exactly one page turns.
- Swipe back on page 1 and forward on the last page: nothing moves.
- Zoom in and flick: the view stops at the page edge and the page doesn't turn.
- Zoomed in at the page edge, drag further: the next page slides in already
  fitted, and once it arrives the view is fitted again. Going back, the page
  left behind is fitted too.
- After zooming, text sharpens about a quarter of a second after the zoom
  stops.
- Switch a document to continuous scrolling, open another, and come back: the
  first is still continuous, and the other opened one page at a time.
- Hold the phone sideways and choose Fit width. A page is now taller than the
  screen. Drag to read down it and swipe sideways to turn. The next page starts
  at its top.

### Navigation

- Scroll the thumbnail strip slowly and with a flick: the page passing the
  middle of the strip is shown as it passes.
- Open Pages or Contents, switch tabs, close and reopen: the tab used last is
  shown. A document without an outline shows only the grid, without tabs.
- In a document with printed page numbers, Go to page accepts "iii", an
  appendix number like "A-2", and a plain position, and says which page it
  found.
- Tap a web link: a dialog shows the address, and it opens only after Open.

### Search

- Search "resume" in a text that has "Résumé": it is found.
- Search a phrase that wraps onto the next line: it is found as one match.
- In a long document, the first search shows a growing count for a moment.
  Later searches are answered at once.
- Search for something that isn't there: "No matches", and the arrows are
  grayed out.
- Put the keyboard away with its search key: the highlights stay and the arrows
  still work. Back then leaves the search.
- A scanned document without a text layer: "No matches".

### Text

These need a document with real text.

- Long-press a margin or a picture: nothing is selected, and the bars stay as
  they are.
- Drag one handle toward the other: the ends can't be dragged past each other
  into nothing.
- Select something, then scroll, zoom, turn the page and come back, and rotate
  the device. The same text is selected. The menu leaves while things move and
  returns when they rest.
- Copy, then paste somewhere: lines and pages are separated by line breaks.
  Android 13 and later show what was copied, earlier versions say "Copied".
- Select all in a very long document, then Copy or Share: where the text is
  more than can be handed over, a message says so. Copy takes about three times
  as much as Share.
- With a selection, tap the page or press Back: the selection goes and nothing
  else happens. The app doesn't enter full screen, follow a link or leave the
  document.

### Editing pages

- Pick a page and choose Select all in the overflow menu: every page is picked,
  and Delete is grayed out.
- Pick several pages, long-press one of them and drag: all of them move to the
  place shown, in the order they were in.
- Long-press a picked page and lift the finger without moving: it is let go of,
  as with a tap.
- Delete pages in a document with printed page numbers: the remaining pages
  keep theirs, and the count follows.
- Undo every edit: "Edited" goes.

### Saving

- Open a document with the file picker, edit it and choose Save. "Edited" goes,
  the document stays open where it was, and Undo still works and makes it
  "Edited" again.
- Open a PDF from another app (a mail attachment, a download notification) and
  edit it: the overflow menu has no Save, only Save as.
- Save as under a new name: the new file is the one that is open, with nothing
  left to undo. The original is untouched, and both are on the start screen.
- Save as over the file that is open: it works, and the file is listed once.
- With unsaved edits, choose Close, or Open and pick a file, or open a PDF from
  another app: "Save changes?" appears. Save saves and then goes on, Discard
  goes on without saving, and Cancel stays. Without edits, nothing is asked.
- Edit, leave the app, and come back later: the edits are still there, unless
  Android ended the app in the meantime.

### Sharing and printing

- Share a file picked in the app, and one opened from another app. The
  receiving app gets the whole file under its name, and the reader is still on
  the same page afterwards.
- Share a protected file: it arrives protected.
- Rotate, delete or move pages, then print without saving: the preview shows
  the document as edited, and the title still says "Edited".
- Print a document whose first page is wider than tall: the dialog starts with
  the paper lying on its side.
- Print a protected file: the preview shows the pages.

### Full screen

- Tap the page: the top bar slides up and the strip slides down while the page
  glides to its new place.
- Zoom in while in full screen: the page uses the entire screen, including the
  area around the camera cutout, and the background has one color throughout.
- Turn a page in full screen: the page number fades in at the bottom and out
  again after about a second and a half.

### Tablets

- Tap a page or an outline entry in the sidebar: the document goes there and
  the sidebar stays.
- Turn on Two pages, leave the app and open the document again: still two
  pages, on the same pair. Other documents are unaffected.
- With First page alone, the pairs are 2 and 3, 4 and 5, and so on.
- Rotate to portrait, or put the app in split screen: the phone layout, one
  page at a time, on the same page. Two pages returns with the room.

### Settings

- Keep the screen on: with a document open, the screen stays on for ten minutes
  after the last touch, then goes dark after the system's usual wait. On the
  start screen, and with the switch off, it goes dark as usual.
- Volume keys turn pages: on a page taller than the screen they scroll most of
  a screenful, like Page Down. The volume doesn't change.
- With that switch on, the keys still change the volume on the start screen, in
  Settings, in Pages or Contents on a phone, and with a menu or dialog open.
- Crash log: the last row of Settings says "Nothing went wrong so far", grayed
  out, unless there is an entry.

### General

- Set the device to German, or on Android 13 and later choose German for
  Specto alone under Apps in the system settings. Menus, dialogs, messages and
  Settings are in German, with dates and numbers as Germans write them. Any
  other language gets English.
- Rotate the device while reading, while zoomed, and with Pages or Contents
  open: the same document and page, and nothing reloads.
- Switch the system between light and dark, with the app open and with it
  closed: bars, backgrounds and dialogs follow.
- Switch to another app and back: the same place. Pages may re-render briefly.
- A huge scan: a spinner turns in the middle of the visible part of the page
  until it is sharp, and comes back while new parts are drawn after zooming in.
  Scrolling, zooming and long presses work as usual under it. Ordinary pages
  never show one.

## Reporting

For each failure, note:

- the feature (section and bullet in FEATURES.md) or check (section here)
- the device and Android version
- what happened instead, with a screenshot or recording if it's visual

For crashes, attach the crash log from Settings.

The "Not yet" section of FEATURES.md lists what's missing on purpose. Those
aren't bugs.
