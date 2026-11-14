import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

val rustAbis = providers.gradleProperty("pdfrs.abis").map { it.split(',').map(String::trim) }

android {
    namespace = "net.xfbs.pdfrs"
    compileSdk {
        version = release(36) { minorApiLevel = 1 }
    }
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "net.xfbs.pdfrs"
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

    buildTypes {
        release {
            // Signed with the debug key: good for measuring performance and
            // for trying the app as users get it, not for distributing it.
            // A real release needs its own key, and code shrinking with keep
            // rules for the UniFFI bindings (see docs/TASKS.md).
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    buildFeatures {
        compose = true
    }

    testOptions {
        // android.util.Log and friends do nothing in JVM unit tests instead of throwing.
        unitTests.isReturnDefaultValues = true
    }

    // Data written by Rust that the Kotlin side must read; see ViewFrameTest.
    sourceSets.getByName("test").resources.directories.add("../../../crates/pdfrs-ffi/fixtures")
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

    // UI tests, on an emulator or device: see src/androidTest.
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.espresso.core)
}

// --- Rust -------------------------------------------------------------------
//
// `cargoNdkBuild` cross-compiles crates/pdfrs-ffi into jniLibs, and
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
        args += listOf("build", "-p", "pdfrs-ffi", "--release")
        cargo(*args.toTypedArray(), env = mapOf("ANDROID_NDK_HOME" to ndkDir.get().asFile.path))
    }
}

abstract class UniffiBindgen : CargoTask() {
    @get:InputDirectory abstract val jniLibs: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        val library = jniLibs.get().asFileTree.matching { include("**/libpdfrs_ffi.so") }.first()
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
