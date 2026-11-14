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
import os
import shutil
import subprocess
import sys
import tempfile
import time
import traceback

APP_ID = "net.xfbs.pdfrs"
APP_PATH = "/net/xfbs/pdfrs"
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

    def click(self, x, y, times=1, shift=False):
        self.new_sequence()
        self.move(x, y)
        if shift:
            self.key_down(self.SHIFT)
        for _ in range(times):
            self.button(True)
            self.button(False)
        if shift:
            self.key_up(self.SHIFT)

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
        """The clipboard's text."""
        r, fds = self.bus.call_with_unix_fd_list_sync(
            self.dest, self.session, self.iface, "SelectionRead",
            self.GLib.Variant("(s)", ("text/plain;charset=utf-8",)),
            None, 0, 5000, None, None,
        )
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
    return [r for _, r in s["edit_pages"]]


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
