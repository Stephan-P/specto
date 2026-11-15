# Packaging

How to build installable packages of the desktop app. Nothing is published yet.

The app ID is `net.xfbs.specto` and the name is "Specto". They are set in `APP_ID` and `APP_NAME` in `apps/desktop/src/main.rs`, and repeated in the file names and contents in `apps/desktop/data/` and `packaging/flatpak/` and in `tools/headless/check.py`.

## Shared files

In `apps/desktop/data/`, used by every Linux package:

- `net.xfbs.specto.desktop`: the launcher entry, which also registers the app for PDF files ("Open With")
- `net.xfbs.specto.metainfo.xml`: the description software centres show
- `icons/net.xfbs.specto.svg`: the icon
- `icons/net.xfbs.specto-symbolic.svg`: its one-colour version, which GNOME uses where it draws small monochrome icons

Check them with:

```sh
desktop-file-validate apps/desktop/data/net.xfbs.specto.desktop
appstreamcli validate apps/desktop/data/net.xfbs.specto.metainfo.xml
```

## Flatpak

```sh
packaging/flatpak/build.sh
flatpak run net.xfbs.specto
```

This builds the app in `target/flatpak` and installs it for the current user. It needs `flatpak-builder` (Fedora: `flatpak-builder`, or the `org.flatpak.Builder` Flatpak), and fetches the GNOME SDK and the Rust SDK extension from Flathub the first time. To remove it: `flatpak uninstall --user net.xfbs.specto`.

- `flatpak/net.xfbs.specto.yml`: the manifest, on the GNOME runtime
- `flatpak/cargo-sources.py`: lists the crates from `Cargo.lock` for flatpak-builder to download, because the build itself has no network; `build.sh` runs it each time

The app gets no access to files beyond the ones it's given: files opened through the file chooser, dragged in or opened with "Open With" come through the document portal.

Saving works in the sandbox: the app writes a temporary file next to the original and renames it into place, and the document portal allows that. File monitors don't see changes other programs make through the portal, so the app also looks at the file whenever its window becomes active again, and before saving.

To check the installed Flatpak automatically (in a headless session with its own document portal):

```sh
XDG_RUNTIME_DIR=$(mktemp -d /tmp/rt.XXXX) tools/headless/run.sh python3 -I tools/headless/check-flatpak.py
```
