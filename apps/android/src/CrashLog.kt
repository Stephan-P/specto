package net.xfbs.specto

import android.content.Context
import android.os.Build
import android.util.Log
import java.io.File
import java.io.PrintWriter
import java.io.StringWriter
import java.time.Instant
import java.time.format.DateTimeParseException
import java.time.temporal.ChronoUnit

/** A log longer than this is cut down to its newest entries when the app starts. */
private const val MAX_LOG_CHARS = 48_000

/** A stack trace is cut off after this much. */
private const val MAX_TRACE_CHARS = 12_000

/** What every entry starts with, on a line of its own, followed by the time and what happened. */
private const val ENTRY = "--- "

/**
 * A record of what went wrong, for bug reports: exceptions that ended the app, and panics in the
 * Rust library, which mostly don't (a page that fails to draw is skipped).
 *
 * It is a text [file] in the app's private storage that Settings shows and the reader can share.
 * Nothing is sent anywhere.
 *
 * A crash in native code that is no panic (out of memory, or a mapped file cut short by another
 * app) ends the process at once and leaves no entry.
 *
 * Both sides write entries in the same form, see `crash.rs` in `specto-ffi`: a line of "--- ", the
 * time in UTC and what happened, then the details.
 */
class CrashLog(private val file: File) {
    /**
     * From now on, records exceptions nothing caught, before whatever handled them so far (the
     * system, which ends the app) gets them, and panics in the Rust library. Call it once, as the
     * app starts.
     */
    fun install() {
        try {
            if (file.exists()) {
                val log = file.readText()
                val kept = trimmedLog(log, MAX_LOG_CHARS)
                if (kept.length < log.length) file.writeText(kept)
            }
        } catch (e: Exception) {
            Log.w("specto", "could not shorten $file", e)
        }
        val before = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            try {
                file.appendText(crashEntry(Instant.now(), thread.name, error))
            } catch (e: Throwable) {
                // Nowhere left to report it.
            }
            before?.uncaughtException(thread, error)
        }
        try {
            uniffi.specto_ffi.logPanicsTo(file.path)
        } catch (e: Throwable) {
            // The library didn't load. The app won't get far, so say why.
            runCatching { file.appendText(crashEntry(Instant.now(), Thread.currentThread().name, e)) }
        }
    }

    /** Everything recorded, oldest first, or empty if nothing was. */
    fun read(): String = try {
        if (file.exists()) file.readText() else ""
    } catch (e: Exception) {
        ""
    }

    fun clear() {
        file.delete()
    }
}

/**
 * Which app this is and what it runs on, to go with a shared log: "Specto 0.1.0 (1), Android 16
 * (API 36), Google Pixel 9a".
 */
fun Context.appAndDevice(): String {
    val app = try {
        val info = packageManager.getPackageInfo(packageName, 0)
        @Suppress("DEPRECATION") // The long variant needs Android 9.
        "${info.versionName} (${info.versionCode})"
    } catch (e: Exception) {
        "?"
    }
    return "Specto $app, Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}), " +
        "${Build.MANUFACTURER} ${Build.MODEL}"
}

/** The log entry for [error], which nothing caught on [thread] at [time]. */
internal fun crashEntry(time: Instant, thread: String, error: Throwable): String {
    val trace = StringWriter().also { error.printStackTrace(PrintWriter(it)) }.toString().trimEnd()
    val shown = if (trace.length > MAX_TRACE_CHARS) trace.take(MAX_TRACE_CHARS) + "\n…" else trace
    return "$ENTRY${time.truncatedTo(ChronoUnit.SECONDS)} Crash\nthread '$thread': $shown\n\n"
}

/** When the newest entry of [log] was written, if it has one that says. */
internal fun lastCrashTime(log: String): Instant? = log.lineSequence()
    .filter { it.startsWith(ENTRY) }
    .mapNotNull { line ->
        try {
            Instant.parse(line.removePrefix(ENTRY).substringBefore(' '))
        } catch (e: DateTimeParseException) {
            null
        }
    }
    .lastOrNull()

/**
 * The newest entries of [log] that fit into [max] characters, whole. If even the newest one alone
 * is longer, its end.
 */
internal fun trimmedLog(log: String, max: Int): String {
    if (log.length <= max) return log
    val tail = log.takeLast(max)
    val entry = tail.indexOf("\n$ENTRY")
    return when {
        tail.startsWith(ENTRY) && log[log.length - max - 1] == '\n' -> tail
        entry >= 0 -> tail.substring(entry + 1)
        else -> tail
    }
}
