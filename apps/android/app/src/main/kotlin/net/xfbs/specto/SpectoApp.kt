package net.xfbs.specto

import android.app.Application
import java.io.File

/** The app as a whole: what is set up before any of its screens. */
class SpectoApp : Application() {
    /** What went wrong, for bug reports; shown in Settings. */
    lateinit var crashLog: CrashLog
        private set

    override fun onCreate() {
        super.onCreate()
        crashLog = CrashLog(File(filesDir, CRASH_LOG_FILE))
        crashLog.install()
    }

    companion object {
        /** The crash log's name in the app's files directory. */
        const val CRASH_LOG_FILE = "crash.log"
    }
}
