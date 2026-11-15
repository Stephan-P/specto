# Tools

For working on Specto. None of this is installed with the apps.

## The specto command-line tool

Looks into PDF files with the same crates the apps use, and runs the rendering
regression. It is in `specto-cli`.

```sh
cargo build --release -p specto-cli

# pages, sizes, metadata
target/release/specto info file.pdf

# pages as PNG images, at twice the size
target/release/specto render file.pdf -o out -s 2

# the text layer of page 1
target/release/specto text file.pdf -p 1

# a copy with pages picked and reordered (and rotated, with -r 90)
target/release/specto save file.pdf -o out.pdf -p 3,1-2

# the regression run
target/release/specto corpus corpus/files/pdfjs/test/pdfs --reference

# a command's options
target/release/specto help render
```

The regression run and the test corpus are described in
[corpus/README.md](../corpus/README.md).

## test-files.sh

Puts copies of the test files for testing by hand in `target/test-files/`. It
needs the corpus and qpdf. See
[apps/desktop/TESTING.md](../apps/desktop/TESTING.md).

## uniffi-bindgen

Generates the Kotlin bindings for the Android app. Gradle runs it as part of
the Android build.

## Elsewhere

The desktop app's automated checks are in
[apps/desktop/test-ui](../apps/desktop/test-ui), described in
[apps/desktop/TESTING.md](../apps/desktop/TESTING.md).
