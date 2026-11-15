package net.xfbs.specto

import android.content.res.Resources
import androidx.annotation.StringRes

/**
 * The app's texts (`res/values/strings.xml`), for code that puts a text
 * together outside a composable. Composables use `stringResource`; this is
 * for functions the unit tests call, which run without Android's resources
 * and read the English texts from the file itself.
 */
fun interface Texts {
    /** The text [id], with [args] in its placeholders. */
    fun get(@StringRes id: Int, vararg args: Any): String
}

/** The texts in the language of the device. */
fun Resources.texts() = Texts { id, args -> getString(id, *args) }
