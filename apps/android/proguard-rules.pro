# What R8 must leave alone in a release build, beyond what it finds out by
# itself. The UI tests run against the debug build, which isn't shrunk: after
# a change here, or a new way of reaching Rust, try the release build by hand
# (BUILDING.md, "Build and install").

# The UniFFI bindings (generated, package uniffi.specto_ffi) reach Rust through
# JNA, which looks up their native methods, callback interfaces and structure
# fields by name at run time.
-keep class uniffi.specto_ffi.** { *; }

# JNA itself: its native half calls back into these classes by name.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.Library { *; }
-keep class * implements com.sun.jna.Callback { *; }
-keep class * extends com.sun.jna.Structure { *; }
# JNA's desktop parts refer to AWT, which Android doesn't have.
-dontwarn java.awt.**

# `Native` (Native.kt) is bound to Rust functions named after the class and
# its methods (crates/specto-ffi/src/jni.rs).
-keep class net.xfbs.specto.Native { *; }

# Stack traces in the crash log keep their file names and line numbers.
-keepattributes SourceFile,LineNumberTable
