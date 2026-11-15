"""Checks the desktop app end to end in a headless session.

    cargo build -p pdfrs-desktop
    tools/headless/run.sh python3 -I tools/headless/check.py [BINARY] [TEST…]
    PDFRS_SCALE=2 tools/headless/run.sh python3 -I tools/headless/check.py

Starts pdfrs-gtk with PDFRS_DEBUG=1 and drives it over D-Bus with its window
actions plus the test hooks in apps/desktop/src/debug.rs: `debug-state`
dumps the window's state as JSON, `debug-screenshot` saves a PPM image.

The test PDFs are generated here. Each page is filled with its own colour, so
a screenshot shows which page is on screen and that it was rendered.

Tests (run some by naming them, e.g. `zoom layouts`):

- start_page: no file opens the start page
- open_and_render: title, page labels in the entry, default layout, zoom and
  sidebar; the screenshot is at device resolution and shows page 1
- navigation: next, previous, first and last page; a rotated page renders
- page_entry: labels win over numbers ("4" is the page labelled 4), case
  doesn't matter, unknown pages are refused
- layouts: all four keep the current page; two-page shows two pages; cover
  page alone
- zoom: actual size, in, out, fit page (the whole page visible), limits
  5% and 3200% and rendering there
- sidebar: hiding it, the Contents tab
- remembers_state: page, layout, zoom and sidebar survive closing the app
- broken_file: an error message, and the app keeps running
- two_windows: one window per file, navigated independently
- rotate_and_undo: Ctrl+Right and Ctrl+Left turn the current page, both
  pages of a spread in the two-page layout; "Edited" in the title; Ctrl+Z
  and Shift+Ctrl+Z; in the page entry, Ctrl+Left moves the cursor instead
- save: a rotated page is saved (opened again, it's still rotated, and
  the page labels are kept); Save As writes a copy and shows it, leaving
  the original as it was, and the copy opens where the original was left
- close_prompt: quitting with unsaved changes asks first; Cancel keeps the
  window, Discard closes without saving, Save saves and closes
- thumbnail_selection: clicking a thumbnail goes to its page; Shift+click
  and Ctrl+click select several, which stay selected while the view moves
  and are what Rotate turns; Ctrl+A in the sidebar selects every page
- delete_pages: Delete on the document deletes the current page, Backspace
  in the sidebar the selected ones; not in the page entry; never the last
  page; undo brings them back
- move_pages: thumbnails dropped (through a test hook) on the lower or
  upper half of another go after or before it; the view stays on the page
  it showed; a drop in place does nothing
- drop_files_and_export: two PDFs dropped (through a test hook) on the
  sidebar are inserted where dropped, as one edit; Export Selected Pages
  writes those pages as a new file and leaves the document as it was
- pages_between_windows: pages of one window dropped (through a test
  hook) on another's thumbnails are copied there, rotation included; the
  first window keeps them; saved, they're in the second file
- file_changed: the file replaced by another program is reloaded in place
  if there are no edits; with edits, a banner offers to, and Save asks
  before overwriting; our own saves don't count; a deleted file gets a
  banner
- print_to_file: printing (to a PDF file, through a test hook) prints the
  pages as edited, landscape ones on landscape paper, each with its own
  content; "Selection" prints the selected thumbnails only
- insert_file: pages of another file inserted after the current page, shown
  and searched; the pages keep their labels, the inserted ones numbered by
  position; undo; saved, they're in the file

With the virtual pointer and keyboard, on a Courier text PDF:

- link_click: clicking a link follows it
- select_by_dragging: the I-beam cursor over text; dragging selects,
  forwards and backwards over lines; Escape and a click clear it
- select_words_and_lines: double-click selects a word, triple-click a line,
  Shift+click extends by lines, double-click and drag extends by words
- drag_on_link_selects: a drag starting on a link selects instead
- copy_and_select_all: Ctrl+C puts the selection on the clipboard (read back
  through the compositor); Ctrl+A selects every page
- select_across_pages: holding a drag at the bottom edge scrolls, and the
  selection continues onto the next page
- select_rotated_page: selecting on a page rotated by 90°
- select_after_rotating: selecting and searching on a page the user rotated
- search: Ctrl+F and typing; case, a line break, a hyphenated word and
  accents don't stop a match; Enter, Shift+Enter and wrapping around; the
  view follows; "No matches"; Escape ends the search
- search_indexes_in_background: the first match after the current page,
  found while the pages are indexed in the background

Saved state and recent files go to a temporary directory, not the user's.
Screenshots of failed tests are kept in $PDFRS_CHECK_OUT (default: a
temporary directory, printed at the end).

Standard library only, except for the pointer, keyboard and clipboard
tests, which use mutter's remote desktop API through PyGObject.
"""

import json
import re
import os
import shutil
import subprocess
import sys
import tempfile
import time
import traceback

APP_ID = "net.xfbs.specto"
APP_PATH = "/net/xfbs/specto"
TIMEOUT = float(os.environ.get("PDFRS_CHECK_TIMEOUT", "20"))

# One colour per page: the corners of the RGB cube at 0, 128 and 255,
# without greys (the UI's colours) and blues close to the accent colour.
PALETTE = [
    (r, g, b)
    for r in (0, 128, 255)
    for g in (0, 128, 255)
    for b in (0, 128, 255)
    if not (r == g == b) and (r, g, b) not in ((0, 128, 255), (0, 0, 255), (128, 128, 255))
]

PAGES = 12
# Labels: i, ii, iii, then 1 to 9.
LABELS = ["i", "ii", "iii"] + [str(n) for n in range(1, PAGES - 2)]
ROTATED = 5


# --- Test PDFs ----------------------------------------------------------


def write_pdf(path, objects):
    """Writes `objects` (bodies of objects 1…n, object 1 the catalog)."""
    out = bytearray(b"%PDF-1.7\n")
    offsets = []
    for i, body in enumerate(objects):
        offsets.append(len(out))
        out += f"{i + 1} 0 obj\n".encode() + body + b"\nendobj\n"
    xref = len(out)
    out += f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode()
    for o in offsets:
        out += f"{o:010} 00000 n \n".encode()
    out += f"trailer << /Root 1 0 R /Size {len(objects) + 1} >>\n".encode()
    out += f"startxref\n{xref}\n%%EOF\n".encode()
    with open(path, "wb") as f:
        f.write(out)


def stream(data):
    return f"<< /Length {len(data)} >>\nstream\n".encode() + data + b"\nendstream"


def pages_pdf(path):
    """PAGES letter pages, page i filled with PALETTE[i] and titled "Page
    i+1". Page labels, an outline with two entries, and page ROTATED rotated
    by 90°."""
    # 1 catalog, 2 pages, 3 font, 4 outlines, 5-6 outline items,
    # then a page and a content stream per page.
    first = 7
    page_ids = [first + 2 * i for i in range(PAGES)]
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R /PageMode /UseOutlines"
        b" /PageLabels << /Nums [0 << /S /r >> 3 << /S /D >>] >> >>",
        f"<< /Type /Pages /Kids [{' '.join(f'{p} 0 R' for p in page_ids)}]"
        f" /Count {PAGES} >>".encode(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica"
        b" /Encoding /WinAnsiEncoding >>",
        b"<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>",
        f"<< /Title (Chapter One) /Parent 4 0 R /Next 6 0 R"
        f" /Dest [{page_ids[3]} 0 R /XYZ 0 792 0] >>".encode(),
        f"<< /Title (Chapter Two) /Parent 4 0 R /Prev 5 0 R"
        f" /Dest [{page_ids[8]} 0 R /XYZ 0 400 0] >>".encode(),
    ]
    for i in range(PAGES):
        r, g, b = (c / 255 for c in PALETTE[i])
        content = (
            f"{r:.3f} {g:.3f} {b:.3f} rg 36 36 540 690 re f "
            f"0 g BT /F1 24 Tf 72 740 Td (Page {i + 1}) Tj ET"
        ).encode()
        rotate = " /Rotate 90" if i == ROTATED else ""
        objects.append(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]{rotate}"
            f" /Resources << /Font << /F1 3 0 R >> >> /Contents {page_ids[i] + 1} 0 R >>".encode()
        )
        objects.append(stream(content))
    write_pdf(path, objects)


# Text for selection tests, in Courier: every character is 0.6 em wide, so
# positions are easy to compute. Page 3 is rotated by 90°.
TEXT = [
    ["The quick brown fox", "jumps over the lazy dog", "Link to page two"],
    ["Second page", "and its last line", "Search across a line", "break and a hyphen-",
     "ated word, Café"],
    ["Rotated text"],
]
FONT, CHAR, LEFT, TOP, LEADING = 20, 12, 72, 700, 30
LINK_LINE = 2


def text_point(page, line, col, before=True):
    """PDF point inside character `col` of `line`: in its left quarter
    (selects from before it) or its right quarter (to after it), at half
    height."""
    x = LEFT + col * CHAR + (CHAR * 0.25 if before else CHAR * 0.75)
    return x, TOP - line * LEADING + FONT * 0.3


def text_pdf(path):
    """TEXT, one page per entry; a link on page 1's last line to page 2."""
    n = len(TEXT)
    page_ids = [5 + 2 * i for i in range(n)]
    link_rect = (LEFT - 2, TOP - LINK_LINE * LEADING - 6,
                 LEFT + len(TEXT[0][LINK_LINE]) * CHAR + 2, TOP - LINK_LINE * LEADING + 18)
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        f"<< /Type /Pages /Kids [{' '.join(f'{p} 0 R' for p in page_ids)}]"
        f" /Count {n} >>".encode(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>",
        f"<< /Type /Annot /Subtype /Link /Rect [{' '.join(map(str, link_rect))}]"
        f" /Border [0 0 0] /Dest [{page_ids[1]} 0 R /XYZ 0 792 0] >>".encode(),
    ]
    for i, lines in enumerate(TEXT):
        ops = " ".join(
            f"BT /F1 {FONT} Tf {LEFT} {TOP - k * LEADING} Td ({t}) Tj ET"
            for k, t in enumerate(lines)
        )
        extra = " /Annots [4 0 R]" if i == 0 else ""
        extra += " /Rotate 90" if i == 2 else ""
        objects.append(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]{extra}"
            f" /Resources << /Font << /F1 3 0 R >> >> /Contents {page_ids[i] + 1} 0 R >>".encode()
        )
        # WinAnsi: "é" is one byte.
        objects.append(stream(ops.encode("cp1252")))
    write_pdf(path, objects)


# --- Screenshots ---------------------------------------------------------


def read_ppm(path):
    """Returns (width, height, pixel bytes as RGB) of a binary PPM."""
    with open(path, "rb") as f:
        data = f.read()
    magic, size, maxval, pixels = data.split(b"\n", 3)
    assert magic == b"P6" and maxval == b"255", "not an 8-bit binary PPM"
    width, height = map(int, size.split())
    return width, height, pixels


def page_colours(path, step=4):
    """Fraction of (sampled) pixels showing each page's colour: {page: share}."""
    width, height, pixels = read_ppm(path)
    colours = {c: i for i, c in enumerate(PALETTE[:PAGES])}
    counts, total = {}, 0
    for y in range(0, height, step):
        row = pixels[y * width * 3 : (y + 1) * width * 3]
        for x in range(0, width * 3, step * 3):
            total += 1
            i = colours.get(tuple(row[x : x + 3]))
            if i is not None:
                counts[i] = counts.get(i, 0) + 1
    return {i: n / total for i, n in counts.items()}


# --- Driving the app ----------------------------------------------------


class CheckFailed(Exception):
    pass


def check(cond, message):
    if not cond:
        raise CheckFailed(message)


def gdbus_activate(path, action, param=None):
    arg = "[]" if param is None else f"[{param}]"
    r = subprocess.run(
        ["gdbus", "call", "--session", "--dest", APP_ID, "--object-path", path,
         "--method", "org.gtk.Actions.Activate", action, arg, "{}"],
        capture_output=True, text=True, timeout=10,
    )
    if r.returncode != 0:
        raise CheckFailed(f"{action} on {path}: {r.stderr.strip()}")


def gvariant_str(s):
    return "<'" + s.replace("\\", "\\\\").replace("'", "\\'") + "'>"


class App:
    """A running pdfrs-gtk."""

    def __init__(self, ctx, files=()):
        self.ctx = ctx
        log = open(os.path.join(ctx.tmp, "app.log"), "a")
        self.proc = subprocess.Popen(
            [ctx.binary, *files], env=ctx.env, stdout=log, stderr=log
        )
        deadline = time.monotonic() + TIMEOUT
        while True:
            r = subprocess.run(
                ["gdbus", "introspect", "--session", "--dest", APP_ID,
                 "--object-path", f"{APP_PATH}/window/1"],
                capture_output=True, timeout=10,
            )
            if r.returncode == 0 and b"org.gtk.Actions" in r.stdout:
                return
            if self.proc.poll() is not None:
                raise CheckFailed(f"app exited with {self.proc.returncode}")
            if time.monotonic() > deadline:
                raise CheckFailed("app didn't start")
            time.sleep(0.1)

    def window(self, n=1):
        return Window(self, n)

    def quit(self):
        gdbus_activate(APP_PATH, "quit")
        try:
            self.proc.wait(timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            raise CheckFailed("app didn't quit")
        check(self.proc.returncode == 0, f"app exited with {self.proc.returncode}")

    def kill(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()


class Input:
    """A virtual pointer and keyboard, and the clipboard, through mutter's
    remote desktop D-Bus API (in the headless session only). Coordinates are
    logical screen pixels; with the window maximized they are window
    coordinates. Needs PyGObject."""

    BUTTON_LEFT = 0x110  # BTN_LEFT
    # Keys as Linux input key codes (linux/input-event-codes.h), which
    # don't depend on the keyboard layout.
    ESCAPE, CONTROL, SHIFT, ENTER, KEY_A, KEY_C, KEY_F = 1, 29, 42, 28, 30, 46, 33
    LEFT, RIGHT, KEY_Z = 105, 106, 44
    BACKSPACE, DELETE = 14, 111
    # Key codes for typing, on a US layout (mutter's default).
    TYPING = {
        **{c: 16 + i for i, c in enumerate("qwertyuiop")},
        **{c: 30 + i for i, c in enumerate("asdfghjkl")},
        **{c: 44 + i for i, c in enumerate("zxcvbnm")},
        **{c: 2 + i for i, c in enumerate("1234567890")},
        " ": 57, "-": 12, ",": 51, ".": 52,
    }
    # GTK counts clicks within this many seconds as one sequence (double-
    # and triple-clicks); wait this long before clicking somewhere new.
    CLICK_GAP = 0.6

    def __init__(self):
        from gi.repository import Gio, GLib

        self.GLib = GLib
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION)
        self.dest = "org.gnome.Mutter.RemoteDesktop"
        self.session = self._call(
            "/org/gnome/Mutter/RemoteDesktop", self.dest, "CreateSession"
        )[0]
        self.iface = self.dest + ".Session"
        self._session("Start")
        self._session("EnableClipboard", "(a{sv})", ({},))
        # Relative motion only (absolute needs a screen cast): park the
        # pointer in the top-left corner, then track where it is.
        self.last_press = 0.0
        self.window = None
        self.screen = None
        self.park()
        # Mutter creates the virtual keyboard on its first event, and that
        # event is lost: send a harmless one first.
        self.keys(self.SHIFT)
        time.sleep(0.2)

    def _call(self, path, iface, method, sig=None, args=None):
        params = self.GLib.Variant(sig, args) if sig else None
        r = self.bus.call_sync(self.dest, path, iface, method, params, None, 0, 5000, None)
        return r.unpack() if r else None

    def _session(self, method, sig=None, args=None):
        return self._call(self.session, self.iface, method, sig, args)

    def park(self):
        """Moves the pointer to the top-left corner, where it stops."""
        # Twice: the first event of a new device can be lost (see below).
        for _ in range(2):
            self._session("NotifyPointerMotionRelative", "(dd)", (-1e5, -1e5))
        self.pos = (0.0, 0.0)

    def move(self, x, y):
        """Moves the pointer to window (x, y). With a window `attach`ed and
        the target on its canvas, also makes sure it got there: relative
        motion is dead reckoning, and a single event that's lost or handled
        late (it happens, more under load) shifts every later position.
        So it reads where the canvas saw the pointer and corrects."""
        self._relative(x - self.pos[0], y - self.pos[1])
        if self.screen:
            # The pointer stops at the screen's edges.
            x = min(max(x, 0.0), self.screen[0] - 1)
            y = min(max(y, 0.0), self.screen[1] - 1)
        self.pos = (x, y)
        if self.window is None or not self.window.on_canvas(x, y):
            return
        for _ in range(8):
            try:
                self.window.pointer_at(x, y, timeout=0.5)
                return
            except CheckFailed:
                s = self.window.state()
                seen = (s["pointer"][0] + s["canvas"][0], s["pointer"][1] + s["canvas"][1])
                # Correct by the difference; a stale report gets a fresh
                # one this way too, and the next round corrects that.
                self._relative(x - seen[0], y - seen[1])
        self.window.pointer_at(x, y)

    def _relative(self, dx, dy):
        if abs(dx) < 0.5 and abs(dy) < 0.5:
            # No motion would mean no event to see: a detour.
            self._session("NotifyPointerMotionRelative", "(dd)", (1.0, 0.0))
            dx -= 1.0
        self._session("NotifyPointerMotionRelative", "(dd)", (dx, dy))

    def attach(self, window):
        """Waits for `window` to see each pointer move (see `move`). The
        window is maximized, so its size is the screen's."""
        s = window.state()
        self.window, self.screen = window, (s["width"], s["height"])
        self.park()
        return self

    def button(self, down):
        self._session("NotifyPointerButton", "(ib)", (self.BUTTON_LEFT, down))
        if down:
            self.last_press = time.monotonic()

    def new_sequence(self):
        """Waits until the next press starts a new click sequence."""
        wait = self.last_press + self.CLICK_GAP - time.monotonic()
        if wait > 0:
            time.sleep(wait)

    def click(self, x, y, times=1, shift=False, control=False):
        self.new_sequence()
        self.move(x, y)
        held = [k for k, on in ((self.SHIFT, shift), (self.CONTROL, control)) if on]
        for k in held:
            self.key_down(k)
        for _ in range(times):
            self.button(True)
            self.button(False)
        for k in reversed(held):
            self.key_up(k)

    def drag(self, start, end, steps=10, release=True, press=True):
        """Presses at `start` (unless the button is already down there) and
        moves to `end` in steps."""
        if press:
            self.new_sequence()
            self.move(*start)
            self.button(True)
        for i in range(1, steps + 1):
            t = i / steps
            self.move(start[0] + (end[0] - start[0]) * t, start[1] + (end[1] - start[1]) * t)
        if release:
            self.button(False)

    def key_down(self, code):
        self._session("NotifyKeyboardKeycode", "(ub)", (code, True))

    def key_up(self, code):
        self._session("NotifyKeyboardKeycode", "(ub)", (code, False))

    def keys(self, *codes):
        """Presses keys together (modifiers first), then releases them."""
        for k in codes:
            self.key_down(k)
        for k in reversed(codes):
            self.key_up(k)

    def type(self, text):
        """Types `text` (letters, digits, spaces, a little punctuation)."""
        for c in text:
            code = self.TYPING[c.lower()]
            if c.isupper():
                self.keys(self.SHIFT, code)
            else:
                self.keys(code)

    def clipboard(self):
        """The clipboard's text; None while nothing owns the clipboard (e.g.
        before the app has handled the copy shortcut)."""
        try:
            r, fds = self.bus.call_with_unix_fd_list_sync(
                self.dest, self.session, self.iface, "SelectionRead",
                self.GLib.Variant("(s)", ("text/plain;charset=utf-8",)),
                None, 0, 5000, None, None,
            )
        except self.GLib.GError as e:
            if "No selection owner" in e.message:
                return None
            raise
        fd = fds.get(r.unpack()[0])
        os.set_blocking(fd, True)
        data = b""
        while chunk := os.read(fd, 65536):
            data += chunk
        os.close(fd)
        return data.decode()


class Window:
    def __init__(self, app, n):
        self.app, self.n = app, n
        self.path = f"{APP_PATH}/window/{n}"

    def action(self, name, param=None):
        gdbus_activate(self.path, name, param)

    def state(self):
        out = os.path.join(self.app.ctx.tmp, f"state-{self.n}.json")
        self.action("debug-state", gvariant_str(out))
        with open(out) as f:
            return json.load(f)

    def wait(self, what, pred, timeout=TIMEOUT):
        """Waits until `pred(state)` holds; returns that state."""
        deadline = time.monotonic() + timeout
        while True:
            s = self.state()
            if pred(s):
                return s
            if time.monotonic() > deadline:
                raise CheckFailed(f"timed out waiting for {what}; state: {s}")
            time.sleep(0.05)

    def settled(self, what="the page to render", pred=lambda s: True):
        return self.wait(
            what, lambda s: s["view"] == "document" and s["complete"] and pred(s)
        )

    def maximize(self):
        """Maximizes the window, so window and screen coordinates agree."""
        self.action("debug-maximize", gvariant_str(""))
        self.wait("maximized", lambda s: s["maximized"])
        return self.stable()

    def stable(self, polls=4):
        """Waits until the window is rendered and its size and page
        positions stay the same for a few polls. GTK reports a window as
        maximized before it has the new size, and the last frame at the old
        size may still count as complete."""
        last, same = None, 0
        deadline = time.monotonic() + TIMEOUT
        while True:
            s = self.state()
            key = (s["width"], s["height"], json.dumps(s["pages"]))
            same = same + 1 if key == last and s["complete"] else 0
            if same >= polls:
                return s
            if time.monotonic() > deadline:
                raise CheckFailed(f"the window didn't settle; state: {s}")
            last = key
            time.sleep(0.1)

    def screen(self, page, x, y, state=None):
        """Window coordinates of PDF point (x, y) on `page`."""
        s = state or self.state()
        for p in s["pages"]:
            if p[0] == page:
                _, a, b, c, d, e, f = p
                return a * x + c * y + e, b * x + d * y + f
        raise CheckFailed(f"page {page} isn't near the current page: {s['pages']}")

    def char(self, page, line, col, before=True, state=None):
        """Window coordinates inside a character of TEXT (see text_point)."""
        return self.screen(page, *text_point(page, line, col, before), state=state)

    def on_canvas(self, x, y):
        """Whether window (x, y) is on the canvas, where the pointer's
        position can be read back."""
        s = self.state()
        cx, cy = s["canvas"]
        return cx <= x < s["width"] and cy <= y < s["height"]

    def pointer_at(self, x, y, timeout=TIMEOUT):
        """Waits until the canvas has seen the pointer at window (x, y)."""

        def there(s):
            px, py = s["pointer"][0] + s["canvas"][0], s["pointer"][1] + s["canvas"][1]
            return abs(px - x) < 1 and abs(py - y) < 1
        return self.wait(f"the pointer at {x:.0f}, {y:.0f}", there, timeout)

    def screenshot(self, name):
        path = os.path.join(self.app.ctx.out, f"{self.app.ctx.test}-{name}.ppm")
        if os.path.exists(path):
            os.remove(path)
        self.action("debug-screenshot", gvariant_str(path))
        # The app saves it after the next frame is painted, under another
        # name, then renames it: once it's there, it's complete.
        deadline = time.monotonic() + 5
        while not os.path.exists(path):
            if time.monotonic() > deadline:
                raise CheckFailed(f"no screenshot written to {path}")
            time.sleep(0.05)
        return path

    def colours(self, name):
        return page_colours(self.screenshot(name))

    def showing(self, page, name, share=0.25):
        """Checks that `page`'s colour covers at least `share` of the window."""
        c = self.colours(name)
        check(
            c.get(page, 0) >= share,
            f"{name}: expected page {page} on screen, colours {fmt(c)}",
        )
        return c


def fmt(colours):
    return {p: round(s, 2) for p, s in sorted(colours.items())}


# --- Tests --------------------------------------------------------------


def test_start_page(ctx):
    app = ctx.app()
    s = app.window().state()
    check(s["view"] == "empty", f"start page expected, got {s['view']}")
    app.quit()


def test_open_and_render(ctx):
    w = ctx.app([ctx.pages]).window()
    s = w.settled()
    check(s["title"] == "pages.pdf", s["title"])
    check(s["page_count"] == PAGES, s)
    check(s["page"] == 0 and s["page_entry"] == "i" and s["page_total"] == f"(1 of {PAGES})", s)
    check(s["layout"] == "continuous" and s["zoom_mode"] == "fit-width", s)
    check(s["sidebar"] and s["sidebar_page"] == "thumbnails", s)
    check(abs(s["scale"] - ctx.scale) < 0.01, f"scale {s['scale']}, expected {ctx.scale}")
    shot = w.screenshot("open")
    width, height, _ = read_ppm(shot)
    expect = (round(s["width"] * ctx.scale), round(s["height"] * ctx.scale))
    check(
        abs(width - expect[0]) <= 2 and abs(height - expect[1]) <= 2,
        f"screenshot {width}×{height}, expected {expect} (device pixels)",
    )
    w.showing(0, "open", share=0.4)


def test_navigation(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    w.action("next-page")
    w.settled("page 2", lambda s: s["page"] == 1 and s["page_entry"] == "ii")
    w.action("last-page")
    s = w.settled("the last page", lambda s: s["page"] == PAGES - 1)
    check(s["page_entry"] == LABELS[-1], s)
    w.showing(PAGES - 1, "last")
    w.action("previous-page")
    w.settled("the second to last page", lambda s: s["page"] == PAGES - 2)
    w.action("first-page")
    w.settled("the first page", lambda s: s["page"] == 0)
    w.showing(0, "first")
    # The rotated page is landscape on screen and still rendered.
    w.action("debug-go-to", gvariant_str(LABELS[ROTATED]))
    s = w.settled("the rotated page", lambda s: s["page"] == ROTATED)
    w.showing(ROTATED, "rotated")


def test_page_entry(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    # Labels win over page numbers: "4" is the page labelled 4.
    w.action("debug-go-to", gvariant_str("4"))
    s = w.settled("label 4", lambda s: s["page"] == LABELS.index("4"))
    check(s["page_entry"] == "4", s)
    w.showing(LABELS.index("4"), "label-4")
    # Case doesn't matter for roman labels.
    w.action("debug-go-to", gvariant_str("II"))
    w.settled("label ii", lambda s: s["page"] == 1)
    # Nonsense is refused, and the entry goes back to the current page.
    for text in ("999", "abc"):
        w.action("debug-go-to", gvariant_str(text))
        s = w.state()
        check(s["page"] == 1 and s["page_entry"] == "ii", f"after {text!r}: {s}")


def test_layouts(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    target = 6
    w.action("debug-go-to", gvariant_str(LABELS[target]))
    w.settled("page 7", lambda s: s["page"] == target)
    for layout in ("two-page-continuous", "two-page", "single", "continuous"):
        w.action("layout", gvariant_str(layout))
        s = w.settled(
            layout, lambda s: s["layout"] == layout and abs(s["page"] - target) <= 1
        )
        c = w.showing(target, layout, share=0.1)
        if layout.startswith("two-page"):
            # Pages 7 and 8 side by side.
            check(c.get(target + 1, 0) > 0.1, f"{layout}: one page only: {fmt(c)}")
    w.action("layout", gvariant_str("two-page"))
    w.action("cover-alone")
    s = w.settled("cover alone", lambda s: s["cover_alone"])
    check(s["layout"] == "two-page", s)


def test_zoom(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    w.action("zoom-actual")
    s = w.settled("actual size", lambda s: s["zoom_mode"] == "fixed")
    check(abs(s["zoom"] - 1.0) < 1e-9 and s["zoom_label"] == "100%", s)
    w.action("zoom-in")
    s = w.settled("zoom in", lambda s: s["zoom"] > 1.0)
    check(abs(s["zoom"] - 1.25) < 1e-9 and s["zoom_label"] == "125%", s)
    w.action("zoom-out")
    w.settled("zoom out", lambda s: abs(s["zoom"] - 1.0) < 1e-9)
    w.action("layout", gvariant_str("single"))
    w.action("fit-page")
    s = w.settled("fit page", lambda s: s["zoom_mode"] == "fit-page")
    # The whole page is visible: its coloured area, 540 × 690 of 612 × 792
    # points, at the current zoom.
    c = w.showing(0, "fit-page", share=0.0)
    area = 540 * 690 * s["zoom"] ** 2 / (s["width"] * s["height"])
    check(abs(c.get(0, 0) - area) < 0.05, f"page area {fmt(c)}, expected {area:.2f}")
    # Limits: 5% to 3200%, and it still renders there.
    for _ in range(30):
        w.action("zoom-in")
    s = w.settled("maximum zoom", lambda s: s["zoom"] >= 32 - 1e-9)
    check(s["zoom_label"] == "3200%", s)
    w.showing(0, "max-zoom", share=0.5)
    for _ in range(40):
        w.action("zoom-out")
    s = w.settled("minimum zoom", lambda s: s["zoom"] <= 0.05 + 1e-9)
    check(s["zoom_label"] == "5%", s)


def test_sidebar(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    w.action("show-sidebar")
    w.settled("hidden sidebar", lambda s: not s["sidebar"])
    w.action("sidebar-page", gvariant_str("outline"))
    w.settled("contents", lambda s: s["sidebar"] and s["sidebar_page"] == "outline")


def test_remembers_state(ctx):
    app = ctx.app([ctx.pages])
    w = app.window()
    w.settled()
    w.action("debug-go-to", gvariant_str("6"))
    w.action("layout", gvariant_str("two-page"))
    w.action("zoom-actual")
    w.action("sidebar-page", gvariant_str("outline"))
    before = w.settled("changes", lambda s: s["layout"] == "two-page" and s["zoom_mode"] == "fixed")
    app.quit()

    w = ctx.app([ctx.pages]).window()
    s = w.settled("restored state", lambda s: s["layout"] == "two-page")
    for key in ("page", "layout", "zoom_mode", "zoom", "sidebar", "sidebar_page"):
        check(s[key] == before[key], f"{key}: {s[key]!r}, before closing {before[key]!r}")


def test_broken_file(ctx):
    w = ctx.app([ctx.broken]).window()
    s = w.wait("an error", lambda s: s["toast"] != "")
    check(s["view"] == "empty", s)
    check(s["toast"].startswith("Couldn't open “broken.pdf”"), s["toast"])


def test_two_windows(ctx):
    app = ctx.app([ctx.pages, ctx.pages])
    one, two = app.window(1), app.window(2)
    # Only the window on top is sure to draw: on a small screen it covers
    # the other one, and the compositor stops sending frames to hidden
    # windows. So wait for the documents to load, not to render.
    for w in (one, two):
        w.wait("the document", lambda s: s["view"] == "document")
    one.action("last-page")
    one.wait("last page", lambda s: s["page"] == PAGES - 1)
    check(two.state()["page"] == 0, "the second window moved too")


def selection_window(ctx):
    """A maximized window on the text PDF, and an input device."""
    w = ctx.app([ctx.text]).window()
    w.settled()
    w.maximize()
    return w, ctx.input().attach(w)


def selected(w, text):
    return w.wait(f"selection {text!r}", lambda s: s["selection"] == text)


def test_link_click(ctx):
    w, inp = selection_window(ctx)
    inp.click(*w.char(0, LINK_LINE, 3))
    w.wait("the link to page 2", lambda s: s["page"] == 1)


def test_select_by_dragging(ctx):
    w, inp = selection_window(ctx)
    # Over text the cursor is an I-beam.
    x, y = w.char(0, 0, 6)
    inp.move(x, y)
    s = w.pointer_at(x, y)
    check(s["cursor"] == "text", f"cursor over text: {s['cursor']!r}")
    # From the start of "quick" (character 4) to the end of "brown" (14).
    inp.drag(w.char(0, 0, 4), w.char(0, 0, 14, before=False))
    selected(w, "quick brown")
    w.screenshot("selected")
    # Backwards and over two lines: from the end of "lazy" back to the
    # start of "brown".
    inp.drag(w.char(0, 1, 18, before=False), w.char(0, 0, 10))
    selected(w, "brown fox\njumps over the lazy")
    # Escape clears it, and so does a click.
    inp.keys(Input.ESCAPE)
    selected(w, "")
    inp.drag(w.char(0, 0, 0), w.char(0, 0, 2, before=False))
    selected(w, "The")
    inp.click(*w.screen(0, 300, 300))
    selected(w, "")


def test_select_words_and_lines(ctx):
    w, inp = selection_window(ctx)
    inp.click(*w.char(0, 0, 12), times=2)
    selected(w, "brown")
    inp.click(*w.char(0, 1, 3), times=3)
    selected(w, "jumps over the lazy dog")
    # Shift+click extends by lines.
    inp.click(*w.char(0, 0, 5), shift=True)
    selected(w, "The quick brown fox\njumps over the lazy dog")
    # Double-click and drag: whole words.
    inp.new_sequence()
    inp.move(*w.char(0, 0, 5))
    inp.button(True)
    inp.button(False)
    inp.button(True)
    inp.drag(w.char(0, 0, 5), w.char(0, 0, 11), press=False)
    selected(w, "quick brown")


def test_drag_on_link_selects(ctx):
    w, inp = selection_window(ctx)
    inp.drag(w.char(0, LINK_LINE, 0), w.char(0, LINK_LINE, 6, before=False))
    s = selected(w, "Link to")
    check(s["page"] == 0, "the link was followed")


def test_copy_and_select_all(ctx):
    w, inp = selection_window(ctx)
    inp.drag(w.char(0, 0, 4), w.char(0, 0, 14, before=False))
    selected(w, "quick brown")
    inp.keys(Input.CONTROL, Input.KEY_C)
    deadline = time.monotonic() + TIMEOUT
    while (clip := inp.clipboard()) != "quick brown" and time.monotonic() < deadline:
        time.sleep(0.1)
    check(clip == "quick brown", f"clipboard: {clip!r}")
    inp.keys(Input.CONTROL, Input.KEY_A)
    everything = "\n".join(line for page in TEXT for line in page)
    selected(w, everything)
    inp.keys(Input.CONTROL, Input.KEY_C)
    while (clip := inp.clipboard()) != everything and time.monotonic() < deadline:
        time.sleep(0.1)
    check(clip == everything, f"clipboard: {clip!r}")


def test_select_across_pages(ctx):
    w, inp = selection_window(ctx)
    # Hold the drag below the window: it scrolls to page 2.
    s = w.state()
    start = w.char(0, 1, 0, state=s)
    inp.drag(start, (start[0], s["height"] + 40), release=False)
    w.wait("scrolling to page 2", lambda s: "Second page" in s["selection"])
    inp.button(False)
    s = w.state()
    check(s["selection"].startswith("jumps over the lazy dog\nLink to page two\nSecond page"),
          f"selection: {s['selection']!r}")


def test_select_rotated_page(ctx):
    w, inp = selection_window(ctx)
    w.action("last-page")
    s = w.settled("the rotated page", lambda s: s["page"] == 2)
    # The text runs downwards on screen; drag along it.
    inp.drag(w.char(2, 0, 0, state=s), w.char(2, 0, 6, before=False, state=s))
    selected(w, "Rotated")


def search_for(w, inp, text):
    """Opens the search bar (or selects its text) and types `text`."""
    inp.keys(Input.CONTROL, Input.KEY_F)
    # Keys are only delivered once the bar has started sliding in.
    w.wait("the search bar", lambda s: s["search_ready"] and s["focus"] == "GtkText")
    inp.type(text)
    return w.wait(f"a search for {text!r}", lambda s: s["search_query"] == text)


def status(w, text):
    return w.wait(f"search status {text!r}", lambda s: s["search_status"] == text)


def test_search(ctx):
    w, inp = selection_window(ctx)
    search_for(w, inp, "the")
    # "The" and "the": case doesn't matter. The first is current.
    s = status(w, "1 of 2")
    check(s["match_page"] == 0, s)
    w.screenshot("the")
    # Enter goes on, wrapping around; Shift+Enter goes back.
    inp.keys(Input.ENTER)
    status(w, "2 of 2")
    inp.keys(Input.ENTER)
    status(w, "1 of 2")
    inp.keys(Input.SHIFT, Input.ENTER)
    status(w, "2 of 2")
    # Across a line break, on page 2: the view goes there.
    search_for(w, inp, "line break")
    s = status(w, "1 of 1")
    w.wait("page 2", lambda s: s["match_page"] == 1 and s["page"] == 1)
    # A word broken with a hyphen, and one with an accent.
    search_for(w, inp, "hyphenated")
    status(w, "1 of 1")
    search_for(w, inp, "cafe")
    status(w, "1 of 1")
    search_for(w, inp, "zebra")
    status(w, "No matches")
    # Escape closes the bar and ends the search.
    inp.keys(Input.ESCAPE)
    s = w.wait("the search to end", lambda s: not s["search_bar"])
    check("matches" not in s and s["focus"] == "PdfrsCanvas", s)


def test_search_indexes_in_background(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    w.maximize()
    inp = ctx.input().attach(w)
    w.action("debug-go-to", gvariant_str(LABELS[5]))
    w.settled("page 6", lambda s: s["page"] == 5)
    # "Page 1", "Page 10", "Page 11", "Page 12": the first after page 6
    # is current, found as the pages are indexed from there.
    search_for(w, inp, "page 1")
    s = status(w, "2 of 4")
    check(s["match_page"] == 9 and s["indexed"] == PAGES, s)
    w.wait("page 10", lambda s: s["page"] == 9)
    inp.keys(Input.SHIFT, Input.ENTER)
    status(w, "1 of 4")
    w.wait("page 1", lambda s: s["page"] == 0)


def rotations(s):
    """The rotation of each page, in degrees."""
    return [p[1] for p in s["edit_pages"]]


def test_rotate_and_undo(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    inp = ctx.input().attach(w)
    w.action("next-page")
    w.settled("page 2", lambda s: s["page"] == 1)
    unrotated = [0] * PAGES
    check(rotations(w.state()) == unrotated, w.state()["edit_pages"])
    inp.keys(Input.CONTROL, Input.RIGHT)
    s = w.wait("page 2 rotated", lambda s: rotations(s)[1] == 90)
    check(rotations(s) == [0, 90] + [0] * (PAGES - 2), s["edit_pages"])
    check(s["modified"] and s["subtitle"].endswith("— Edited"), s["subtitle"])
    check(s["undo"] == "Rotate Page" and s["redo"] == "", s)
    # The page stays current, now landscape: its x axis runs down the screen.
    s = w.settled("the rotated page", lambda s: s["page"] == 1)
    a, b = next(p[1:3] for p in s["pages"] if p[0] == 1)
    check(abs(a) < 1e-6 and b > 0, f"page 2 isn't turned clockwise: {s['pages']}")
    w.showing(1, "rotated")
    inp.keys(Input.CONTROL, Input.LEFT)
    inp.keys(Input.CONTROL, Input.LEFT)
    w.wait("page 2 turned back past 0", lambda s: rotations(s)[1] == 270)
    # Undo all three, then redo one.
    for _ in range(3):
        inp.keys(Input.CONTROL, Input.KEY_Z)
    s = w.wait("everything undone", lambda s: rotations(s) == unrotated and s["undo"] == "")
    check(not s["modified"] and "Edited" not in s["subtitle"], s)
    check(s["redo"] == "Rotate Page", s)
    inp.keys(Input.SHIFT, Input.CONTROL, Input.KEY_Z)
    w.wait("the first rotation redone", lambda s: rotations(s)[1] == 90 and s["modified"])
    # Two pages side by side turn together.
    w.action("layout", gvariant_str("two-page"))
    w.action("debug-go-to", gvariant_str(LABELS[5]))
    # Pages 5 and 6 side by side; the left one counts as current.
    s = w.settled("pages 5 and 6", lambda s: s["page"] == 4)
    w.action("rotate-right")
    s = w.wait("a spread rotated", lambda s: sum(r == 90 for r in rotations(s)) == 3)
    check(rotations(s)[4] == 90 and rotations(s)[5] == 90, s["edit_pages"])
    # In the page entry, Ctrl+Left moves the cursor by a word instead.
    w.action("go-to-page")
    w.wait("the page entry", lambda s: s["focus"] == "GtkText")
    before = w.state()["edit_pages"]
    inp.keys(Input.CONTROL, Input.LEFT)
    time.sleep(0.5)
    check(w.state()["edit_pages"] == before, "Ctrl+Left in the page entry rotated a page")


def turned(s, page):
    """Whether `page` is shown turned by 90° or 270°: its x axis runs
    vertically on screen."""
    a, b = next(p[1:3] for p in s["pages"] if p[0] == page)
    return abs(a) < 1e-6 and abs(b) > 0


def copy_of_pages(ctx, name="doc.pdf"):
    """A copy of the pages PDF that a test can change."""
    path = os.path.join(ctx.home, name)
    shutil.copy(ctx.pages, path)
    return path


def test_save(ctx):
    path = copy_of_pages(ctx)
    app = ctx.app([path])
    w = app.window()
    w.settled()
    w.action("next-page")
    w.settled("page 2", lambda s: s["page"] == 1)
    w.action("rotate-right")
    w.wait("page 2 rotated", lambda s: s["modified"])
    w.action("save")
    s = w.wait("the save", lambda s: not s["modified"] and not s["saving"])
    check("Edited" not in s["subtitle"] and s["toast"] == "", s)
    app.quit()
    # Opened again (on page 2, where it was closed): page 2 is turned,
    # nothing else, and it isn't an edit. Not "next-page": whether that
    # arrives before or after the saved position is restored is a race.
    app = ctx.app([path])
    w = app.window()
    s = w.settled("page 2", lambda s: s["page"] == 1)
    check(turned(s, 1) and not turned(s, 0), s["pages"])
    check(not s["modified"] and s["page_entry"] == LABELS[1], s)
    # Save As: a copy, shown from then on; the file opened stays as it was.
    before = open(path, "rb").read()
    w.action("rotate-left")
    w.wait("page 2 turned back", lambda s: s["modified"])
    copy = os.path.join(ctx.home, "copy.pdf")
    w.action("debug-save-as", gvariant_str(copy))
    s = w.wait("the copy", lambda s: s["title"] == "copy.pdf" and not s["modified"])
    check(open(path, "rb").read() == before, "Save As changed the original file")
    check(os.path.exists(copy), "no copy written")
    app.quit()
    # The copy took over the reading position too.
    w = ctx.app([copy]).window()
    s = w.settled("page 2 of the copy", lambda s: s["page"] == 1)
    check(not turned(s, 1), s["pages"])


def test_close_prompt(ctx):
    path = copy_of_pages(ctx)
    app = ctx.app([path])
    w = app.window()
    w.settled()
    w.action("rotate-right")
    w.wait("page 1 rotated", lambda s: s["modified"])
    # Cancel: the window stays, still edited.
    gdbus_activate(APP_PATH, "quit")
    w.wait("the question", lambda s: s["alert"] == "Save Changes?")
    time.sleep(0.5)  # the dialog's animation
    w.screenshot("prompt")
    w.action("debug-alert-response", gvariant_str("cancel"))
    s = w.wait("the question to go", lambda s: s["alert"] == "")
    time.sleep(0.5)
    check(app.proc.poll() is None and w.state()["modified"], "the window closed")
    # Save: saved, then closed.
    gdbus_activate(APP_PATH, "quit")
    w.wait("the question", lambda s: s["alert"] == "Save Changes?")
    w.action("debug-alert-response", gvariant_str("save"))
    try:
        app.proc.wait(timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        raise CheckFailed("the app didn't quit after saving")
    check(app.proc.returncode == 0, f"app exited with {app.proc.returncode}")
    # Discard: closed without saving the second rotation.
    app = ctx.app([path])
    w = app.window()
    s = w.settled()
    check(turned(s, 0), "the first rotation wasn't saved")
    w.action("rotate-right")
    w.wait("page 1 rotated again", lambda s: s["modified"])
    gdbus_activate(APP_PATH, "quit")
    w.wait("the question", lambda s: s["alert"] == "Save Changes?")
    w.action("debug-alert-response", gvariant_str("discard"))
    try:
        app.proc.wait(timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        raise CheckFailed("the app didn't quit after discarding")
    s = ctx.app([path]).window().settled()
    check(turned(s, 0), "the discarded rotation was saved")


def click_thumbnail(w, inp, page, **keys):
    """Clicks the middle of `page`'s thumbnail (on screen in the sidebar)."""
    s = w.state()
    rect = next((t[1:] for t in s["thumbnails"] if t[0] == page), None)
    check(rect is not None, f"no thumbnail of page {page} on screen: {s['thumbnails']}")
    x, y, width, height = rect
    # From a place on the canvas, where the pointer's position is checked,
    # so the move into the sidebar starts from a known point.
    inp.move(s["width"] * 0.75, s["height"] / 2)
    inp.click(x + width / 2, y + height / 2, **keys)


def selected_thumbnails(w, pages):
    return w.wait(
        f"thumbnails {pages} selected", lambda s: s.get("thumbnails_selected") == pages
    )


def test_thumbnail_selection(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    w.maximize()
    s = w.settled("maximized")
    inp = ctx.input().attach(w)
    check(s["thumbnails_selected"] == [0], s)
    # A click goes to the page.
    click_thumbnail(w, inp, 1)
    selected_thumbnails(w, [1])
    w.settled("page 2", lambda s: s["page"] == 1)
    # Shift+click and Ctrl+click select more, without going there. (Only
    # three thumbnails fit at scale 2.)
    click_thumbnail(w, inp, 2, shift=True)
    selected_thumbnails(w, [1, 2])
    click_thumbnail(w, inp, 0, control=True)
    selected_thumbnails(w, [0, 1, 2])
    click_thumbnail(w, inp, 1, control=True)
    s = selected_thumbnails(w, [0, 2])
    check(s["page"] == 1, f"the view moved to page {s['page'] + 1}")
    # Rotate turns the selected pages, which stay selected.
    w.action("rotate-right")
    s = w.wait("the selected pages rotated", lambda s: rotations(s)[:3] == [90, 0, 90])
    check(sum(rotations(s)) == 180 and s["thumbnails_selected"] == [0, 2], s)
    # Moving the view keeps the selection.
    w.action("last-page")
    w.settled("the last page", lambda s: s["page"] == PAGES - 1)
    check(w.state()["thumbnails_selected"] == [0, 2], "the selection didn't stay")
    w.action("first-page")
    w.settled("page 1", lambda s: s["page"] == 0)
    # A plain click selects one again; then Rotate turns the current page.
    click_thumbnail(w, inp, 1)
    selected_thumbnails(w, [1])
    w.settled("page 2", lambda s: s["page"] == 1)
    w.action("rotate-right")
    s = w.wait("page 2 rotated", lambda s: rotations(s)[1] == 90)
    check(sum(rotations(s)) == 270, s["edit_pages"])
    # With the sidebar focused, Ctrl+A selects every page, not the text.
    # Rarely (2 in about 40 runs) neither ever happens, as if the key never
    # arrived, so it gets a second press then.
    w.settled()
    everything = list(range(PAGES))
    inp.keys(Input.CONTROL, Input.KEY_A)
    try:
        s = w.wait("every thumbnail selected", lambda s: s["thumbnails_selected"] == everything, 2)
    except CheckFailed:
        check(w.state()["selection"] == "", "Ctrl+A selected the text")
        inp.keys(Input.CONTROL, Input.KEY_A)
        s = selected_thumbnails(w, everything)
    check(s["selection"] == "", f"text selected: {s['selection']!r}")
    # Undo: the selection goes back to following the current page.
    w.action("undo")
    w.wait("page 2 turned back", lambda s: rotations(s)[1] == 0)
    selected_thumbnails(w, [w.state()["page"]])


def file_pages(s):
    """The page of the file at each position."""
    return [p[0] for p in s["edit_pages"]]


def test_delete_pages(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    w.maximize()
    w.settled("maximized")
    inp = ctx.input().attach(w)
    w.action("debug-go-to", gvariant_str(LABELS[2]))
    w.settled("page 3", lambda s: s["page"] == 2)
    # The document has the focus: Delete deletes the current page, and the
    # next one takes its place.
    w.wait("the canvas focused", lambda s: s["focus"] == "PdfrsCanvas")
    inp.keys(Input.DELETE)
    s = w.wait("page 3 deleted", lambda s: len(s["edit_pages"]) == PAGES - 1)
    check(file_pages(s) == [0, 1] + list(range(3, PAGES)), s["edit_pages"])
    check(s["undo"] == "Delete Page" and s["subtitle"].startswith("11 pages"), s)
    s = w.settled("the page after it", lambda s: s["page"] == 2)
    w.showing(3, "after-delete")
    # It keeps its label.
    check(s["page_entry"] == LABELS[3], s)
    # Two selected in the sidebar: Backspace deletes both.
    w.action("first-page")
    w.settled("page 1", lambda s: s["page"] == 0 and s["thumbnails"][0][0] == 0)
    click_thumbnail(w, inp, 0)
    click_thumbnail(w, inp, 1, control=True)
    selected_thumbnails(w, [0, 1])
    inp.keys(Input.BACKSPACE)
    s = w.wait("pages 1 and 2 deleted", lambda s: len(s["edit_pages"]) == PAGES - 3)
    check(file_pages(s) == list(range(3, PAGES)), s["edit_pages"])
    check(s["undo"] == "Delete 2 Pages", s)
    # In the page entry, Backspace edits the text.
    w.action("go-to-page")
    w.wait("the page entry", lambda s: s["focus"] == "GtkText")
    inp.keys(Input.BACKSPACE)
    time.sleep(0.5)
    check(len(w.state()["edit_pages"]) == PAGES - 3, "Backspace in the page entry deleted a page")
    # Every page selected: nothing is deleted (a document keeps a page).
    click_thumbnail(w, inp, 0)
    inp.keys(Input.CONTROL, Input.KEY_A)
    selected_thumbnails(w, list(range(PAGES - 3)))
    inp.keys(Input.DELETE)
    time.sleep(0.5)
    check(len(w.state()["edit_pages"]) == PAGES - 3, "every page deleted")
    # Undo brings them back.
    w.action("undo")
    w.action("undo")
    s = w.wait("both undone", lambda s: len(s["edit_pages"]) == PAGES)
    check(not s["modified"], s)


def drop_thumbnails(w, pages, onto, lower=True):
    """Drops `pages` on the lower (or upper) half of `onto`'s thumbnail.
    Through a test hook that does what a drop does: Mutter refuses about
    one drag in three from the virtual pointer (GTK cancels it at once),
    so real drags would make the check flaky."""
    s = w.state()
    rect = next((t[1:] for t in s["thumbnails"] if t[0] == onto), None)
    check(rect is not None, f"no thumbnail of page {onto} on screen: {s['thumbnails']}")
    _, y, _, height = rect
    # Just off the middle: the lower part may be below the window's edge.
    at = y + height * (0.6 if lower else 0.4)
    check(at < s["height"], f"page {onto}'s thumbnail is too far down: {rect}")
    w.action("debug-drop-thumbnails", gvariant_str(f"{','.join(map(str, pages))}@{at}"))


def test_move_pages(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    w.maximize()
    w.settled("maximized")
    # Page 1 dropped on the lower half of page 3: it goes after it (only
    # three thumbnails fit at scale 2).
    drop_thumbnails(w, [0], 2)
    w.wait("page 1 moved", lambda s: file_pages(s)[:4] == [1, 2, 0, 3])
    # The view stays on the page it showed, now the third.
    s = w.settled("page 1 in its new place", lambda s: s["page"] == 2)
    check(s["undo"] == "Move Page" and s["modified"], s)
    check(s["thumbnails_selected"] == [2], s)
    w.showing(0, "moved")
    # Two pages (2 and 3, now first) dropped on the lower half of the
    # third: they go after page 1, where they were.
    w.action("first-page")
    w.settled("the first page", lambda s: s["page"] == 0 and s["thumbnails"][0][0] == 0)
    drop_thumbnails(w, [0, 1], 2)
    w.wait("both moved", lambda s: file_pages(s) == list(range(PAGES)))
    s = w.settled()
    check(s["undo"] == "Move 2 Pages" and s["thumbnails_selected"] == [1, 2], s)
    # Dropped where they are: nothing changes.
    drop_thumbnails(w, [1, 2], 1, lower=False)
    time.sleep(0.5)
    check(w.state()["undo"] == "Move 2 Pages", "a drop in place was an edit")
    # Undone, the order is as before the second move.
    w.action("undo")
    w.wait("the move undone", lambda s: file_pages(s)[:4] == [1, 2, 0, 3])


def test_drop_files_and_export(ctx):
    path = copy_of_pages(ctx)
    w = ctx.app([path]).window()
    w.settled()
    w.maximize()
    s = w.settled("maximized")
    # Two copies of the three-page text PDF, dropped below page 1.
    rect = next(t[1:] for t in s["thumbnails"] if t[0] == 0)
    at = rect[1] + rect[3] * 0.6
    w.action("debug-drop-files", gvariant_str(f"{ctx.text}|{ctx.text}@{at}"))
    s = w.wait("the files inserted", lambda s: len(s["edit_pages"]) == PAGES + 6)
    check([p[2] for p in s["edit_pages"][:8]] == [0, 1, 1, 1, 2, 2, 2, 0], s["edit_pages"])
    check(s["undo"] == "Insert 6 Pages", s)
    s = w.settled("the first inserted page", lambda s: s["page"] == 1)
    check(s["thumbnails_selected"] == list(range(1, 7)), s)
    # The selected pages exported: a new file with just those.
    out = os.path.join(ctx.home, "export.pdf")
    w.action("debug-export", gvariant_str(out))
    s = w.wait("the export", lambda s: s["toast"] == "Exported 6 pages to “export.pdf”")
    check(s["title"] == "doc.pdf" and s["modified"], s)
    # pdfinfo (poppler-utils) reads it independently of pdfrs.
    info = subprocess.run(["pdfinfo", out], capture_output=True, text=True)
    check("Pages:           6" in info.stdout, f"pdfinfo: {info.stdout}{info.stderr}")


def test_pages_between_windows(ctx):
    path = os.path.join(ctx.home, "text.pdf")
    shutil.copy(ctx.text, path)
    app = ctx.app([ctx.pages, path])
    one, two = app.window(1), app.window(2)
    one.wait("the document", lambda s: s["view"] == "document")
    # The second window is on top, so it draws (see two_windows).
    s = two.settled()
    one.action("debug-go-to", gvariant_str(LABELS[2]))
    one.wait("page 3", lambda s: s["page"] == 2)
    one.action("rotate-right")
    one.wait("page 3 rotated", lambda s: rotations(s)[2] == 90)
    # Pages 1 and 3 of the first window, dropped below page 1 of the second.
    rect = next(t[1:] for t in s["thumbnails"] if t[0] == 0)
    at = rect[1] + rect[3] * 0.6
    two.action("debug-drop-pages-from", gvariant_str(f"1:0,2@{at}"))
    s = two.wait("the pages copied", lambda s: len(s["edit_pages"]) == 5)
    check([(p[0], p[2]) for p in s["edit_pages"]] == [(0, 0), (0, 1), (2, 1), (1, 0), (2, 0)],
          s["edit_pages"])
    check(rotations(s) == [0, 0, 90, 0, 0], s["edit_pages"])
    check(s["undo"] == "Insert 2 Pages" and s["thumbnails_selected"] == [1, 2], s)
    # Its page, not its render: at scale 2 the windows overlap and the
    # compositor may stop sending this one frames.
    two.wait("the first copied page", lambda s: s["page"] == 1)
    # The first window still has them.
    check(len(one.state()["edit_pages"]) == PAGES, "pages left the first window")
    # Saved, the second file has them.
    two.action("save")
    two.wait("the save", lambda s: not s["modified"] and not s["saving"])
    info = subprocess.run(["pdfinfo", path], capture_output=True, text=True)
    check("Pages:           5" in info.stdout, f"pdfinfo: {info.stdout}{info.stderr}")


def replace_file(src, dst):
    """Replaces `dst` with a copy of `src` the way editors do: written
    next to it, then renamed over it."""
    tmp = dst + ".tmp"
    shutil.copy(src, tmp)
    os.replace(tmp, dst)


def test_file_changed(ctx):
    path = copy_of_pages(ctx)
    w = ctx.app([path]).window()
    w.settled()
    w.action("debug-go-to", gvariant_str(LABELS[2]))
    w.settled("page 3", lambda s: s["page"] == 2)
    # No edits: the new file is shown, at the same place.
    replace_file(ctx.text, path)
    s = w.wait("the reload", lambda s: len(s["edit_pages"]) == 3)
    check(s["title"] == "doc.pdf" and not s["modified"] and s["banner"] == "", s)
    w.wait("the same page", lambda s: s["page"] == 2)
    # Our own save doesn't count as a change: the edit history stays.
    w.action("rotate-right")
    w.wait("a page rotated", lambda s: s["modified"])
    w.action("save")
    w.wait("the save", lambda s: not s["modified"] and not s["saving"])
    time.sleep(1)
    s = w.state()
    check(s["undo"] == "Rotate Page" and s["banner"] == "", f"reloaded after saving: {s}")
    # With edits: a banner, and the edits stay.
    w.action("rotate-right")
    w.wait("another rotation", lambda s: s["modified"])
    replace_file(ctx.pages, path)
    s = w.wait("the banner", lambda s: "changed by another program" in s["banner"])
    check(len(s["edit_pages"]) == 3 and s["modified"], s)
    # Saving asks first; Cancel leaves the other program's file.
    w.action("save")
    w.wait("the question", lambda s: s["alert"] == "File Changed on Disk")
    w.action("debug-alert-response", gvariant_str("cancel"))
    w.wait("the question to go", lambda s: s["alert"] == "")
    check(open(path, "rb").read() == open(ctx.pages, "rb").read(), "the file was overwritten")
    # Reload: the other program's version, edits gone.
    w.action("reload")
    s = w.wait("the reload", lambda s: len(s["edit_pages"]) == PAGES)
    check(not s["modified"] and s["banner"] == "" and s["undo"] == "", s)
    # Deleted: a banner; Save writes the file again.
    os.remove(path)
    w.wait("the banner", lambda s: "moved or deleted" in s["banner"])
    w.action("save")
    w.wait("the file written again", lambda s: os.path.exists(path) and s["banner"] == "")


def printed_pages(path):
    """Each page of the PDF at `path`: (landscape, colour covering most of
    it), read with poppler (pdfinfo, pdftoppm) rather than pdfrs."""
    info = subprocess.run(["pdfinfo", "-f", "1", "-l", "999", path],
                          capture_output=True, text=True).stdout
    sizes = re.findall(r"^Page +\d+ size: +([\d.]+) x ([\d.]+)", info, re.M)
    out = []
    for i, (w, h) in enumerate(sizes, 1):
        prefix = os.path.join(os.path.dirname(path), f"printed-{i}")
        subprocess.run(["pdftoppm", "-r", "20", "-f", str(i), "-l", str(i),
                        "-singlefile", path, prefix], check=True)
        # Printing and reading back may move a channel by one or two.
        width, height, pixels = read_ppm(prefix + ".ppm")
        counts = {}
        for k in range(0, len(pixels), 3):
            px = pixels[k : k + 3]
            for page, c in enumerate(PALETTE[:PAGES]):
                if all(abs(a - b) <= 3 for a, b in zip(px, c)):
                    counts[page] = counts.get(page, 0) + 1
                    break
        top = max(counts, key=counts.get) if counts else None
        out.append((float(w) > float(h), top))
    return out


def test_print_to_file(ctx):
    w = ctx.app([ctx.pages]).window()
    w.settled()
    w.maximize()
    w.settled("maximized")
    inp = ctx.input().attach(w)
    w.action("next-page")
    w.wait("page 2", lambda s: s["page"] == 1)
    w.action("rotate-right")
    w.wait("page 2 rotated", lambda s: rotations(s)[1] == 90)
    out = os.path.join(ctx.home, "printed.pdf")
    w.action("debug-print-to-file", gvariant_str(out))
    s = w.wait("the print", lambda s: s["printed"] == 1)
    check(s["toast"] == "", s)
    pages = printed_pages(out)
    check(len(pages) == PAGES, f"{len(pages)} pages printed")
    # Page 2 turned by the user and page ROTATED by its file are landscape.
    landscape = [i for i, (wide, _) in enumerate(pages) if wide]
    check(landscape == sorted([1, ROTATED]), f"landscape: {landscape}")
    check([c for _, c in pages] == list(range(PAGES)), f"colours: {pages}")
    # Selection: thumbnails 1 and 3 only.
    click_thumbnail(w, inp, 0)
    click_thumbnail(w, inp, 2, control=True)
    selected_thumbnails(w, [0, 2])
    out = os.path.join(ctx.home, "selection.pdf")
    w.action("debug-print-to-file", gvariant_str(out + "|selection"))
    w.wait("the second print", lambda s: s["printed"] == 2)
    pages = printed_pages(out)
    check([c for _, c in pages] == [0, 2], f"selection printed: {pages}")


def test_insert_file(ctx):
    path = copy_of_pages(ctx)
    app = ctx.app([path])
    w = app.window()
    w.settled()
    inp = ctx.input().attach(w)
    w.action("next-page")
    w.settled("page 2", lambda s: s["page"] == 1)
    # The text PDF's three pages go after page 2, and the view goes there.
    w.action("debug-insert", gvariant_str(ctx.text))
    s = w.wait("the inserted pages", lambda s: len(s["edit_pages"]) == PAGES + 3)
    inserted = [[0, 0, 1], [1, 0, 1], [2, 0, 1]]
    check(s["edit_pages"][2:5] == inserted, s["edit_pages"])
    check(s["edit_pages"][5] == [2, 0, 0], s["edit_pages"])
    check(s["undo"] == "Insert 3 Pages" and s["modified"], s)
    s = w.settled("the first inserted page", lambda s: s["page"] == 2)
    # The inserted pages have no labels: numbered by position. The others
    # keep theirs.
    check(s["page_entry"] == "3" and s["page_total"] == "(3 of 15)", s)
    check(s["subtitle"].startswith("15 pages"), s)
    w.action("debug-go-to", gvariant_str(LABELS[2]))
    w.wait("page iii, now sixth", lambda s: s["page"] == 5)
    w.action("debug-go-to", gvariant_str("3"))
    w.wait("the first inserted page", lambda s: s["page"] == 2)
    # (Original page 3's thumbnail shows in the sidebar, but not the page.)
    c = w.colours("inserted")
    check(c.get(2, 0) < 0.1, f"page 3 of the original still shown: {fmt(c)}")
    # Searching finds text on the inserted pages, at their new positions.
    search_for(w, inp, "lazy")
    s = status(w, "1 of 1")
    check(s["match_page"] == 2, s)
    inp.keys(Input.ESCAPE)
    # Undone, and done again.
    w.action("undo")
    s = w.wait("the insert undone", lambda s: len(s["edit_pages"]) == PAGES)
    check(not s["modified"] and s["page_entry"] == LABELS[s["page"]], s)
    w.action("redo")
    w.wait("the insert redone", lambda s: len(s["edit_pages"]) == PAGES + 3)
    w.action("save")
    w.wait("the save", lambda s: not s["modified"] and not s["saving"])
    app.quit()
    # One file now: the inserted pages are its pages 3 to 5.
    w = ctx.app([path]).window()
    s = w.settled()
    check([p[2] for p in s["edit_pages"]] == [0] * (PAGES + 3), s["edit_pages"])
    w.action("debug-go-to", gvariant_str("4"))
    w.settled("page 4", lambda s: s["page"] == 3)
    search_for(w, inp.attach(w), "Second page")
    s = status(w, "1 of 1")
    check(s["match_page"] == 3, s)


def test_select_after_rotating(ctx):
    w, inp = selection_window(ctx)
    inp.keys(Input.CONTROL, Input.RIGHT)
    w.wait("page 1 rotated", lambda s: rotations(s)[0] == 90)
    s = w.stable()
    # The text runs downwards on screen now; drag along it.
    inp.drag(w.char(0, 0, 4, state=s), w.char(0, 0, 14, before=False, state=s))
    selected(w, "quick brown")
    # Search highlights still find their page.
    search_for(w, inp, "lazy")
    s = status(w, "1 of 1")
    check(s["match_page"] == 0, s)
    w.screenshot("rotated-search")


TESTS = [v for k, v in list(globals().items()) if k.startswith("test_")]


# --- Runner -------------------------------------------------------------


class Context:
    def __init__(self, binary, tmp, out):
        self.binary, self.tmp, self.out = binary, tmp, out
        self.scale = float(os.environ.get("PDFRS_SCALE", "1"))
        self.pages = os.path.join(tmp, "pages.pdf")
        self.broken = os.path.join(tmp, "broken.pdf")
        self.text = os.path.join(tmp, "text.pdf")
        text_pdf(self.text)
        self._input = None
        pages_pdf(self.pages)
        with open(self.broken, "wb") as f:
            f.write(b"%PDF-1.7\nthis is not a PDF\n")
        self.apps = []
        self.test = ""

    def fresh_env(self):
        """Fresh saved state and recent files for each test."""
        home = tempfile.mkdtemp(prefix=f"{self.test}-", dir=self.tmp)
        self.home = home
        self.env = dict(os.environ, PDFRS_DEBUG="1")
        for var in ("XDG_STATE_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
            self.env[var] = os.path.join(home, var.lower())

    def input(self):
        """The virtual pointer and keyboard (one session for the whole run)."""
        if self._input is None:
            self._input = Input()
        return self._input

    def app(self, files=()):
        a = App(self, files)
        self.apps.append(a)
        return a

    def cleanup(self):
        for a in self.apps:
            a.kill()
        self.apps = []
        # The test's state and caches (shader caches add up); screenshots
        # and the log are elsewhere.
        shutil.rmtree(self.home, ignore_errors=True)


def main():
    args = sys.argv[1:]
    binary = os.path.abspath(args.pop(0) if args and os.sep in args[0] else "target/debug/pdfrs-gtk")
    tests = [t for t in TESTS if not args or t.__name__.removeprefix("test_") in args]
    tmp = tempfile.mkdtemp(prefix="pdfrs-check-")
    open(os.path.join(tmp, "app.log"), "w").close()
    out = os.environ.get("PDFRS_CHECK_OUT") or os.path.join(tmp, "out")
    os.makedirs(out, exist_ok=True)
    ctx = Context(binary, tmp, out)
    failed = []
    for t in tests:
        ctx.test = t.__name__.removeprefix("test_")
        ctx.fresh_env()
        start = time.monotonic()
        try:
            t(ctx)
            for a in ctx.apps:
                if a.proc.poll() not in (None, 0):
                    raise CheckFailed(f"app exited with {a.proc.returncode}")
            print(f"ok    {ctx.test} ({time.monotonic() - start:.1f} s)", flush=True)
        except Exception as e:
            failed.append(ctx.test)
            detail = str(e) if isinstance(e, CheckFailed) else traceback.format_exc()
            print(f"FAIL  {ctx.test}: {detail}", flush=True)
        finally:
            ctx.cleanup()
    with open(os.path.join(tmp, "app.log")) as f:
        log = f.read().strip()
    if failed:
        if log:
            print(f"\napp output:\n{log}")
        print(f"\n{len(failed)} of {len(tests)} failed: {' '.join(failed)}")
        print(f"screenshots: {out}")
        sys.exit(1)
    print(f"\nall {len(tests)} passed (scale {ctx.scale})")
    shutil.rmtree(tmp)


if __name__ == "__main__":
    main()
