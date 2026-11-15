# Packaging

How to build installable packages of the desktop app. Nothing is published yet.

The app ID is `net.xfbs.specto` and the name is "Specto". Both are set in
`apps/desktop/src/main.rs`, and repeated in the file names and contents in
`apps/desktop/data/` and `packaging/flatpak/`, and in
`apps/desktop/test-ui/check.py`.

## Shared files

In `apps/desktop/data/`, used by every Linux package:

- `net.xfbs.specto.desktop`: the launcher entry, which also registers the app
  for PDF files ("Open With")
- `net.xfbs.specto.metainfo.xml`: the description that software centers show,
  with the summary, links, screenshot and releases
- `icons/net.xfbs.specto.svg`: the icon
- `icons/net.xfbs.specto-symbolic.svg`: its one-color version, which GNOME uses
  where it draws small monochrome icons

Check them with:

```sh
desktop-file-validate apps/desktop/data/net.xfbs.specto.desktop
appstreamcli validate apps/desktop/data/net.xfbs.specto.metainfo.xml
```

## Flatpak

```sh
# build and install for the current user
packaging/flatpak/build.sh

# run
flatpak run net.xfbs.specto

# remove
flatpak uninstall --user net.xfbs.specto
```

The build happens in `target/flatpak`. It needs flatpak-builder, either as a
package or as the org.flatpak.Builder Flatpak, and fetches the GNOME SDK and
the Rust SDK extension from Flathub the first time.

- `flatpak/net.xfbs.specto.yml`: the manifest, on the GNOME runtime
- `flatpak/cargo-sources.py`: lists the crates from `Cargo.lock` for
  flatpak-builder to download, because the build itself has no network. The
  build script runs it each time

The app gets no access to files beyond the ones it's given. Files opened
through the file chooser, dragged in or opened with "Open With" come through
the document portal.

Saving works in the sandbox. The app writes a temporary file next to the
original and renames it into place, and the document portal allows that. File
monitors don't see changes other programs make through the portal, so the app
also looks at the file whenever its window becomes active again, and before
saving.

To check the installed Flatpak automatically, in a headless session with its
own document portal:

```sh
XDG_RUNTIME_DIR=$(mktemp -d /tmp/rt.XXXX) apps/desktop/test-ui/run.sh python3 -I apps/desktop/test-ui/check-flatpak.py
```

## Publishing on Flathub

Not done yet. Flathub builds the app itself, from a tagged commit of the public
repository, in a repository of its own that holds only the manifest and the
list of crates. `flatpak/flathub.sh` writes those two files from the local
manifest.

For each release:

1. In `apps/desktop/data/net.xfbs.specto.metainfo.xml`, add a release entry
   with the version and date on top of the others. The version is the one in
   the root `Cargo.toml`, which the About dialog shows.
2. Retake the screenshot if the window changed, with
   `apps/desktop/test-ui/readme-screenshot.sh`.
3. Commit, tag (v0.1.0) and push. The screenshot's URL in the metainfo must be
   reachable, since Flathub fetches the image from there.
4. Write the Flathub files and check them with Flathub's linter:

   ```sh
   packaging/flatpak/flathub.sh v0.1.0 "$(git rev-parse 'v0.1.0^{commit}')" ../flathub-specto
   flatpak run --filesystem="$(realpath ../flathub-specto)" --command=flatpak-builder-lint \
       org.flatpak.Builder manifest ../flathub-specto/net.xfbs.specto.yml
   flatpak run --filesystem="$PWD" --command=flatpak-builder-lint \
       org.flatpak.Builder appstream apps/desktop/data/net.xfbs.specto.metainfo.xml
   ```

The first time, the two files go into a pull request against the new-pr branch
of [flathub/flathub](https://github.com/flathub/flathub). The
[submission guide](https://docs.flathub.org/docs/for-app-authors/submission)
has the steps and the review rules. Once accepted, the app gets its own
repository, flathub/net.xfbs.specto, and a release is a pull request there with
the two files written anew.

The ID says the app belongs to whoever has xfbs.net. Flathub's verification
(the tick next to the developer's name) asks for a token at
`https://xfbs.net/.well-known/org.flathub.VerifiedApps.txt`, set up from the
app's page on Flathub after it is accepted.
