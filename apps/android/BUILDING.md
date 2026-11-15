# Building the Android app

## Prerequisites

- Rust 1.92 or newer (`rust-version` in the workspace `Cargo.toml`), installed
  with [rustup](https://rustup.rs), with the Android target and `cargo-ndk`:

  ```sh
  rustup target add aarch64-linux-android
  cargo install cargo-ndk
  ```

- Android Studio, or the Android SDK on its own, with the NDK version named
  in `app/build.gradle.kts` (`ndkVersion`). In Android Studio it is under
  Settings → Languages & Frameworks → Android SDK → SDK Tools, with "Show
  Package Details" ticked.
- A JDK. Android Studio brings one; to use it from a terminal on macOS:

  ```sh
  export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
  export ANDROID_HOME=~/Library/Android/sdk
  ```

Gradle cross-compiles the Rust library and generates the Kotlin bindings as
part of the build, so there is no separate Rust step. The app runs on Android
8.0 (API 26) and newer.

## Build and install

From `apps/android`, with a phone connected or an emulator running:

```sh
./gradlew :app:installDebug       # build and install
./gradlew :app:assembleDebug      # only build: app/build/outputs/apk/debug/
```

With more than one device connected, say which one:

```sh
adb devices
ANDROID_SERIAL=emulator-5554 ./gradlew :app:installDebug
```

The release build is the one to judge speed by, and the one users get: its
code is shrunk by R8, which removes what the app doesn't use. Without a key
of its own (see below) it is signed with the debug key, which is fine for
trying it:

```sh
./gradlew :app:installRelease
```

The UI tests run against the debug build, which isn't shrunk. What R8 must
not touch is listed in `app/proguard-rules.pro`; after changing that file, or
how Kotlin reaches the Rust library, open a document in the release build
and see that it draws.

## A release for others

A build to hand out has to be signed with a key that stays the same for as
long as the app exists: Android installs an update only if it is signed with
the key of the version it replaces. Create one once, and keep the file and
its password somewhere safe, outside the repository:

```sh
keytool -genkeypair -v -keystore ~/specto-release.jks -alias specto \
    -keyalg RSA -keysize 4096 -validity 10000
```

Then say where it is in `apps/android/keystore.properties`, which Git
ignores:

```properties
storeFile=/home/me/specto-release.jks
storePassword=…
keyAlias=specto
keyPassword=…
```

With that file in place, `./gradlew :app:assembleRelease` signs
`app/build/outputs/apk/release/app-release.apk` with the key (and
`:app:bundleRelease` an app bundle, for stores that want one); without it,
the debug key is used as before. To see which key signed an APK:

```sh
$ANDROID_HOME/build-tools/<version>/apksigner verify --print-certs app-release.apk
```

Before a release, raise `versionCode` (and `versionName`) in
`app/build.gradle.kts`: Android takes an update only with a higher code.

## Emulators on Intel and AMD machines

The Rust library is built for `arm64-v8a`, which covers phones and the
emulator on Apple Silicon. An emulator on an Intel or AMD machine needs
`x86_64`:

```sh
rustup target add x86_64-linux-android
./gradlew :app:installDebug -Pspecto.abis=x86_64
```

To build both, pass `-Pspecto.abis=arm64-v8a,x86_64`, or change `specto.abis`
in `gradle.properties`.

## Tests

```sh
./gradlew :app:testDebugUnitTest                                       # unit tests
ANDROID_SERIAL=emulator-5554 ./gradlew :app:connectedDebugAndroidTest  # UI tests on an emulator
```

The serial matters when a phone is plugged in as well: a UI test run installs
and then removes the app on its target. To run only some test classes:

```sh
ANDROID_SERIAL=emulator-5554 ./gradlew :app:connectedDebugAndroidTest \
    -Pandroid.testInstrumentationRunnerArguments.class=net.xfbs.specto.EditTest,net.xfbs.specto.ShareTest
```

What the tests cover is in the [README](README.md#testing).

## When something is off

- After the Rust library or its bindings were renamed or removed, run
  `./gradlew clean`: generated files from before are otherwise packaged next
  to the new ones.
- The first install on a freshly started emulator sometimes fails; a second
  try works.
