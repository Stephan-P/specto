import java.util.Properties
import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

fun MutableSet<String>.replaceWith(directory: String) {
    clear()
    add(directory)
}

val rustAbis = providers.gradleProperty("specto.abis").map { it.split(',').map(String::trim) }

/**
 * Where the release key is and how to open it: `keystore.properties` next to
 * `settings.gradle.kts`, which is never committed. Null without that file.
 */
val releaseSigning: Properties? = rootProject.file("keystore.properties").takeIf { it.isFile }?.let { file ->
    Properties().apply { file.inputStream().use(::load) }
}

android {
    namespace = "net.xfbs.specto"
    compileSdk {
        version = release(36) { minorApiLevel = 1 }
    }
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "net.xfbs.specto"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // Only package the ABIs we build the Rust library for.
        ndk {
            abiFilters += rustAbis.get()
        }
    }

    signingConfigs {
        // The key releases are signed with, where there is one: see
        // BUILDING.md ("A release for others").
        releaseSigning?.let { key ->
            create("release") {
                storeFile = rootProject.file(key.getProperty("storeFile"))
                storePassword = key.getProperty("storePassword")
                keyAlias = key.getProperty("keyAlias")
                keyPassword = key.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            // Without a key of its own, a release is signed with the debug
            // key: good for measuring performance and for trying the app as
            // users get it, not for distributing it.
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
            // R8 removes what the app doesn't use (most of Compose and the
            // icon set) and shortens names; proguard-rules.pro says what it
            // must leave alone.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    buildFeatures {
        compose = true
    }

    androidResources {
        // Lists the languages there are texts for (res/values-*), so that
        // Android 13 and later offer them in the system's settings for the
        // app; res/resources.properties names the language of res/values.
        generateLocaleConfig = true
    }

    testOptions {
        // android.util.Log and friends do nothing in JVM unit tests instead of throwing.
        unitTests.isReturnDefaultValues = true
    }

    // A flat layout instead of Android's usual src/main/kotlin/net/xfbs/…:
    // Kotlin doesn't need directories that repeat the package name.
    sourceSets {
        getByName("main") {
            manifest.srcFile("AndroidManifest.xml")
            kotlin.directories.replaceWith("src")
            res.directories.replaceWith("res")
        }
        getByName("test") {
            kotlin.directories.replaceWith("test")
            // Data written by Rust that the Kotlin side must read; see ViewFrameTest.
            resources.directories.replaceWith("../../crates/specto-ffi/fixtures")
        }
        // The UI tests, which run on an emulator or device.
        getByName("androidTest") {
            kotlin.directories.replaceWith("test-ui")
            assets.directories.replaceWith("test-ui/assets")
        }
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)
    implementation(libs.activity.compose)
    implementation(libs.androidx.core)
    implementation(libs.jna) { artifact { type = "aar" } }

    testImplementation(libs.junit)
    testImplementation(libs.json)

    // UI tests, on an emulator or device: see test-ui.
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.espresso.core)
}

// --- Rust -------------------------------------------------------------------
//
// `cargoNdkBuild` cross-compiles crates/specto-ffi into jniLibs, and
// `uniffiBindgen` generates the Kotlin bindings from the resulting library.
// Both are wired in as generated sources, so a normal build or "Run" in
// Android Studio picks up Rust changes.
//
// Requires: rustup target add aarch64-linux-android; cargo install cargo-ndk

/** Android Studio started from the Dock doesn't have ~/.cargo/bin on PATH. */
fun cargoBinDir(): String =
    (System.getenv("CARGO_HOME") ?: "${System.getProperty("user.home")}/.cargo") + "/bin"

abstract class CargoTask : DefaultTask() {
    @get:Inject abstract val execOps: ExecOperations
    @get:Internal abstract val workspaceDir: DirectoryProperty
    @get:Internal abstract val cargoBin: Property<String>

    protected fun cargo(vararg args: String, env: Map<String, String> = emptyMap()) {
        execOps.exec {
            workingDir = workspaceDir.get().asFile
            executable = "${cargoBin.get()}/cargo"
            args(*args)
            environment(env)
            environment("PATH", cargoBin.get() + File.pathSeparator + System.getenv("PATH"))
        }
    }
}

abstract class CargoNdkBuild : CargoTask() {
    @get:Input abstract val abis: ListProperty<String>
    @get:Input abstract val minSdk: Property<Int>
    @get:Internal abstract val ndkDir: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun build() {
        val args = mutableListOf("ndk", "--platform", minSdk.get().toString())
        abis.get().forEach { args += listOf("-t", it) }
        args += listOf("-o", outputDir.get().asFile.path)
        // Always optimized: the renderer is unusably slow in a debug build.
        args += listOf("build", "-p", "specto-ffi", "--release")
        cargo(*args.toTypedArray(), env = mapOf("ANDROID_NDK_HOME" to ndkDir.get().asFile.path))
    }
}

abstract class UniffiBindgen : CargoTask() {
    @get:InputDirectory abstract val jniLibs: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        val library = jniLibs.get().asFileTree.matching { include("**/libspecto_ffi.so") }.first()
        cargo(
            "run", "--quiet", "-p", "uniffi-bindgen", "--",
            "generate", library.path,
            "--language", "kotlin", "--no-format",
            "--out-dir", outputDir.get().asFile.path,
        )
    }
}

val workspace = rootProject.layout.projectDirectory.dir("../..")

val cargoNdkBuild = tasks.register<CargoNdkBuild>("cargoNdkBuild") {
    workspaceDir = workspace
    cargoBin = cargoBinDir()
    abis = rustAbis
    minSdk = android.defaultConfig.minSdk
    ndkDir = androidComponents.sdkComponents.ndkDirectory
    // Cargo does its own (fast) up-to-date check.
    outputs.upToDateWhen { false }
}

val uniffiBindgen = tasks.register<UniffiBindgen>("uniffiBindgen") {
    workspaceDir = workspace
    cargoBin = cargoBinDir()
    jniLibs = cargoNdkBuild.flatMap { it.outputDir }
}

androidComponents {
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoNdkBuild, CargoNdkBuild::outputDir)
        variant.sources.kotlin?.addGeneratedSourceDirectory(uniffiBindgen, UniffiBindgen::outputDir)
    }
}
