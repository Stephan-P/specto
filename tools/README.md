# Tools

For working on Specto; none of this is installed with the apps.

## `specto`, the command-line tool

Looks into PDF files with the same crates the apps use, and runs the
rendering regression. In `specto-cli`.

```sh
cargo build --release -p specto-cli
target/release/specto info file.pdf                        # pages, sizes, metadata
target/release/specto render file.pdf -o out -s 2          # pages → PNG
target/release/specto text file.pdf -p 1                   # the text layer
target/release/specto save file.pdf -o out.pdf -p 3,1-2    # pick, reorder, rotate (-r 90) pages
target/release/specto corpus corpus/files/... --reference  # regression run
```

`target/release/specto help <command>` shows a command's options. The
regression run and the test corpus are described in [corpus/README.md](../corpus/README.md).

## `headless`

Runs the desktop app in a headless compositor. `check.py` is the automated
end-to-end check, which also runs in CI; `smoke.sh` takes screenshots. See
[apps/desktop/TESTING.md](../apps/desktop/TESTING.md).

## `test-files.sh`

Puts copies of the test files for testing by hand in `target/test-files/`.
Needs the corpus (`corpus/fetch.sh`) and `qpdf`.

## `uniffi-bindgen`

Generates the Kotlin bindings for the Android app. Gradle runs it as part of
the Android build.
