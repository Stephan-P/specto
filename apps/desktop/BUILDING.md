# Building the desktop app

## Prerequisites

- Rust 1.92 or newer (`rust-version` in the workspace `Cargo.toml`), installed
  with [rustup](https://rustup.rs)
- GTK 4.14 or newer and libadwaita 1.6 or newer, with their development files

Fedora:

```sh
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel
```

macOS, with [Homebrew](https://brew.sh):

```sh
brew install gtk4 libadwaita adwaita-icon-theme gettext
export GETTEXT_DIR="$(brew --prefix gettext)"   # where the build finds libintl
```

The app's translations go through gettext, which is part of the C library on
Linux and a library of its own on macOS.

## Build and run

From the repository root:

```sh
cargo build --release -p specto-desktop
target/release/specto-gtk                 # start page
target/release/specto-gtk file.pdf        # open a file
target/release/specto-gtk a.pdf b.pdf     # one window per file
```

While working on it, `cargo run -p specto-desktop -- file.pdf` builds and
starts it in one step. A debug build is fine for this: the libraries that do
the rendering are optimized in debug builds too.

## Flatpak

```sh
packaging/flatpak/build.sh
flatpak run net.xfbs.specto
```

This builds the app in a sandbox and installs it for the current user. What it
needs and what it does is in
[packaging/README.md](../../packaging/README.md).

## Tests

```sh
cargo test --workspace                                       # the shared crates
tools/headless/run.sh python3 -I tools/headless/check.py     # the app itself, Linux only
```

The second one runs the app in a headless compositor and drives it. It needs a
few more packages (`ci/install-deps-fedora.sh --headless` installs them) and
is described, with the checks that need a person, in [TESTING.md](TESTING.md).

## Translations

Text shown to people goes through the functions in `src/i18n.rs`; the module's
comment says how to write such strings. After changing any, update the
template translations start from (needs GNU gettext 0.24 or newer):

```sh
apps/desktop/po/update-pot.sh
```

There are no translations yet. The app looks for them in `share/locale` next
to the `bin` directory it runs from.
