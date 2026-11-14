#!/usr/bin/env python3
"""Checks the installed Flatpak in the sandbox, where files come through
the document portal. Install it first with packaging/flatpak/build.sh, then
from the repository root:

    XDG_RUNTIME_DIR=$(mktemp -d /tmp/rt.XXXX) tools/headless/run.sh python3 -I tools/headless/check-flatpak.py

The separate XDG_RUNTIME_DIR gives the session its own document portal
(the user's own is mounted at /run/user/UID/doc); it must be a short path,
because the Wayland socket goes in it.

- opens a file given through the portal, as "Open With" does
- saves an edit: the file is replaced, with no leftover temporary file
- the start page names the file's real folder, not /run/user/…/doc/…
- a change by another program is noticed when the window becomes active
  again (file monitors don't see it through the portal)
- saving over such a change asks first
"""

import json
import re
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import check  # noqa: E402
from check import CheckFailed  # noqa: E402

APP = check.APP_ID
WIN1 = f"{check.APP_PATH}/window/1"
WIN2 = f"{check.APP_PATH}/window/2"
KEY_W = 17
# The user's Flatpak installation (the tests don't change HOME, but say it
# anyway, in case a wrapper does).
ENV = dict(os.environ, FLATPAK_USER_DIR=os.path.expanduser("~/.local/share/flatpak"))


class Sandboxed:
    def __init__(self, tmp, pdf):
        self.out = os.path.join(tmp, "out")
        os.makedirs(self.out)
        # What `flatpak run --file-forwarding` does, but with the path the
        # sandbox sees: it mounts the portal at /run/user/UID/doc, whatever
        # XDG_RUNTIME_DIR is outside.
        exported = subprocess.run(
            ["flatpak", "document-export", "--app", APP, "-r", "-w", pdf],
            capture_output=True, text=True, check=True, env=ENV,
        ).stdout.strip()
        doc_id = exported.rstrip("/").split("/")[-2]
        inside = f"/run/user/{os.getuid()}/doc/{doc_id}/{os.path.basename(pdf)}"
        self.log = open(os.path.join(tmp, "app.log"), "w")
        # The debug hooks write their output to a folder the app may write.
        self.proc = subprocess.Popen(
            ["flatpak", "run", "--env=PDFRS_DEBUG=1", f"--filesystem={self.out}",
             APP, inside],
            env=ENV, stdout=self.log, stderr=self.log,
        )

    def state(self, window=WIN1):
        path = os.path.join(self.out, "state.json")
        check.gdbus_activate(window, "debug-state", check.gvariant_str(path))
        with open(path) as f:
            return json.load(f)

    def wait(self, what, pred, timeout=check.TIMEOUT):
        deadline = time.monotonic() + timeout
        s = None
        while True:
            try:
                s = self.state()
                if pred(s):
                    return s
            except (CheckFailed, OSError, json.JSONDecodeError):
                pass
            if self.proc.poll() is not None:
                raise CheckFailed(f"app exited with {self.proc.returncode}")
            if time.monotonic() > deadline:
                raise CheckFailed(f"timed out waiting for {what}; state: {s}")
            time.sleep(0.1)

    def stop(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()
        self.log.close()


def replace(pdf):
    """Another program writes a new version (unrotated, 12 pages)."""
    check.pages_pdf(pdf + ".new")
    os.rename(pdf + ".new", pdf)


def wait_for_window(path):
    deadline = time.monotonic() + check.TIMEOUT
    while subprocess.run(
        ["gdbus", "introspect", "--session", "--dest", APP, "--object-path", path],
        capture_output=True,
    ).returncode != 0:
        if time.monotonic() > deadline:
            raise CheckFailed(f"{path} didn't open")
        time.sleep(0.1)


def run(tmp):
    docs = os.path.join(tmp, "docs")
    os.makedirs(docs)
    pdf = os.path.join(docs, "test.pdf")
    check.pages_pdf(pdf)
    app = Sandboxed(tmp, pdf)
    try:
        s = app.wait("the document", lambda s: s.get("view") == "document" and s.get("page_count"))
        check.check(s["title"] == "test.pdf", f"title {s['title']!r}")
        print("ok    opens a file from the portal", flush=True)

        before = os.stat(pdf)
        check.gdbus_activate(WIN1, "rotate-right")
        app.wait("the edit", lambda s: s["modified"])
        check.gdbus_activate(WIN1, "save")
        s = app.wait("saving", lambda s: not s["modified"] and not s["saving"])
        check.check(not s["toast"], f"toast {s['toast']!r}")
        check.check(os.stat(pdf).st_ino != before.st_ino, "the file wasn't replaced")
        check.check(os.listdir(docs) == ["test.pdf"], f"left over: {os.listdir(docs)}")
        r = subprocess.run(["pdfinfo", pdf], capture_output=True, text=True)
        pages = re.search(r"^Pages:\s+(\d+)", r.stdout, re.M)
        check.check(r.returncode == 0 and pages and pages[1] == "12", f"saved file: {r.stdout}{r.stderr}")
        print("ok    saves through the portal", flush=True)

        # A second window (the start page), which becomes the active one.
        subprocess.run(["flatpak", "run", APP], env=ENV, timeout=check.TIMEOUT)
        wait_for_window(WIN2)
        time.sleep(1)  # until it's the active window
        s = app.state(WIN2)
        check.check(s["view"] == "empty", f"window 2 shows {s['view']!r}")
        check.check(s["recent"] == [["test.pdf", docs]], f"recent files: {s['recent']}")
        print("ok    the start page names the real folder", flush=True)

        # Changed outside while window 2 is active: nothing yet...
        replace(pdf)
        time.sleep(1)
        check.check(app.state()["edit_pages"][0][1] == 90, "reloaded while in the background")
        # ...then Ctrl+W closes window 2, window 1 becomes active and looks.
        check.Input().keys(check.Input.CONTROL, KEY_W)
        app.wait("the reload", lambda s: s["edit_pages"][0][1] == 0)
        print("ok    notices outside changes when active again", flush=True)

        check.gdbus_activate(WIN1, "rotate-right")
        app.wait("the edit", lambda s: s["modified"])
        time.sleep(0.5)
        replace(pdf)
        check.gdbus_activate(WIN1, "save")
        app.wait("the question", lambda s: s["alert"] == "File Changed on Disk")
        print("ok    asks before saving over an outside change", flush=True)
    finally:
        app.stop()


def main():
    tmp = tempfile.mkdtemp(prefix="pdfrs-flatpak-")
    try:
        run(tmp)
    except CheckFailed as e:
        print(f"FAIL  {e}")
        with open(os.path.join(tmp, "app.log")) as f:
            log = f.read().strip()
        if log:
            print(f"\napp output (end):\n{log[-3000:]}")
        sys.exit(1)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("\nall passed")


if __name__ == "__main__":
    main()
