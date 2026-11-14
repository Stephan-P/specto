# pdfrs for Android

A fast, pleasant PDF reader for Android phones and tablets. Built with Rust and Jetpack Compose, it shares its core with the pdfrs desktop app.

(A screenshot will follow.)

Android has few open-source PDF viewers, and the ones there are tend to feel clunky: slow to render, blurry while zooming, or awkward to navigate. pdfrs aims to be a reader that is simply nice to use, with smooth page turns, sharp pages at any zoom, and quick ways to find your way around a long document. Light editing, such as annotations and filling in forms, will follow.

## Features

What it can do today:

- Paged reading with swipes, each page fitted to the screen, or continuous scrolling
- Pinch and double-tap zoom, with pages sharp at any zoom
- A thumbnail strip, a grid of all pages, the table of contents, links, and printed page numbers ("iii (3 / 240)")
- Two pages side by side on tablets and other wide windows, optionally with the cover page on its own
- Full screen, keyboard navigation, light and dark mode
- Password-protected PDFs
- Recent documents, and continuing where you left off

Coming next: search, text selection, and sharing. After that: annotations, filling in forms, and reordering pages.

The full list of features, with how to check each one, is in [FEATURES.md](FEATURES.md).

## Implementation

The core of pdfrs is implemented in Rust and shared with the desktop app. The Android app is a Kotlin and Compose layer over these shared crates:

- `pdfrs-view`: page layout, zoom, scrolling, and which parts of which pages to render
- `pdfrs-render`: rendering pages into tiles in the background, with [hayro](https://github.com/LaurenzV/hayro), a PDF renderer written in Rust
- `pdfrs-source`: reading the table of contents, links and page labels

They're compiled into `libpdfrs_ffi.so`. The crate `pdfrs-ffi` exposes them to Kotlin through UniFFI, plus a few hand-written JNI functions for the work done on every frame, where UniFFI's overhead was too high. The app itself draws the rendered tiles and handles gestures, the file picker and recent documents. More detail is in [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md).

## Building

Needs Android Studio or the Android SDK, the NDK version named in `app/build.gradle.kts`, Rust with the Android target, and `cargo-ndk`:

```sh
rustup target add aarch64-linux-android
cargo install cargo-ndk
cd apps/android
./gradlew :app:installDebug      # build and install on a connected device or emulator
```

Gradle cross-compiles the Rust library and generates the Kotlin bindings as part of the build. The app runs on Android 8.0 (API 26) and newer.

## Testing

```sh
cd apps/android
./gradlew :app:testDebugUnitTest                                       # unit tests (also in CI)
ANDROID_SERIAL=emulator-5554 ./gradlew :app:connectedDebugAndroidTest  # UI tests on an emulator
```

The UI tests (`ReaderTest`) cover opening, swiping, the page grid, "Go to page…", full screen, continuing where you left off, the layout on wide windows, and turning pages with keys. Everything else is checked by hand with [FEATURES.md](FEATURES.md).

## Project structure

In `app/src/main/kotlin/net/xfbs/pdfrs/`:

- `MainActivity.kt`: the activity, theme, top bar, menus and dialogs
- `StartScreen.kt`: the start screen with recent documents
- `PdfView.kt`, `PdfViewState.kt`: the page view, its gestures, keys and state
- `ViewFrame.kt`: one frame of the view as planned in Rust, decoded from the JNI call
- `Native.kt`: the JNI functions (the rest of the Rust API comes through UniFFI's generated bindings)
- `ThumbnailStrip.kt`, `PagesGrid.kt`, `OutlineList.kt`: the strip, the page grid and the table of contents
- `DocumentStore.kt`: recent documents and where each was left off

Elsewhere:

- `app/src/test/`: unit tests
- `app/src/androidTest/`: UI tests, with their test PDFs in `assets/` (written by `make-fixtures.py`)
- `app/build.gradle.kts`: the build, including cross-compiling the Rust library
- `FEATURES.md`: the feature list, as things to check by hand
