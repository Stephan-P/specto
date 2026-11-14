# Testing the desktop app by hand

[FEATURES.md](FEATURES.md) is the checklist: go through every feature there on each platform (Linux and macOS). This file has the setup, and the checks that need a particular test file or whose expected result isn't obvious from the feature list.

## Automated checks

Much of the feature list is also checked automatically, in CI and locally (Linux, needs `mutter`; see `ci/install-deps-fedora.sh` for the packages):

```sh
cargo build -p pdfrs-desktop
tools/headless/run.sh python3 -I tools/headless/check.py
PDFRS_SCALE=2 tools/headless/run.sh python3 -I tools/headless/check.py   # also 1.25
tools/headless/run.sh python3 -I tools/headless/check.py zoom layouts    # some tests only
```

The app runs in a headless compositor with its own saved state, so yours isn't touched. What's covered is listed at the top of `tools/headless/check.py`. Not covered: anything needing the pointer (links, dragging, pinch), dialogs, how sharp the text looks, and macOS. Those still need the manual pass.

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
- Pressing on a link, moving a few pixels and releasing still follows it. A longer drag doesn't (GTK's drag threshold; later it will start a text selection).

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

- macOS: the app has a menu bar (File, View, Go) in the system menu bar, with the shortcuts shown; GTK draws its own window decorations. Shortcuts should work with Cmd (and also with Ctrl). Opening PDFs from Finder with "Open With" isn't expected to work yet, since there's no app bundle; use the command line, the dialog or drag-and-drop. Retina screens are scale 2.
- Linux: fractional scaling needs to be enabled in GNOME (Settings → Displays). Recently opened PDFs should also appear in GNOME Files under Recent.

## Reporting

For each failure: the feature (section and bullet in FEATURES.md) or check (section here), the platform and OS version (e.g. "macOS 26.1, M2" or "Fedora 44, GNOME 49"), what happened instead, and a screenshot if it's visual. For crashes, run from a terminal and include the output.

The "Not yet" section of FEATURES.md lists what's missing on purpose; those aren't bugs.
