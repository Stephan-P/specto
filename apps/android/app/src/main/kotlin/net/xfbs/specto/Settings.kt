package net.xfbs.specto

import android.content.SharedPreferences
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

/** How long the screen is kept on after the last touch or key, with [AppSettings.keepScreenOn]. */
const val KEEP_SCREEN_ON_MINUTES = 10

/**
 * What the reader set for the whole app, as opposed to what is remembered
 * for each document (see [DocumentStore]). Observable, and stored as soon as
 * it changes. Main thread only.
 */
class AppSettings(private val stored: SharedPreferences) {
    /** While a document is open, the screen stays on for [KEEP_SCREEN_ON_MINUTES] after the last touch. */
    var keepScreenOn: Boolean by flag("keep_screen_on")

    /** The volume keys turn pages (or scroll) instead of changing the volume. */
    var volumeKeysTurnPages: Boolean by flag("volume_keys_turn_pages")

    /** A switch that is off until the reader turns it on. */
    private fun flag(key: String) = object {
        private var value by mutableStateOf(stored.getBoolean(key, false))

        operator fun getValue(owner: Any?, property: Any?): Boolean = value

        operator fun setValue(owner: Any?, property: Any?, new: Boolean) {
            value = new
            stored.edit().putBoolean(key, new).apply()
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(settings: AppSettings, crashLog: CrashLog, onBack: () -> Unit) {
    var showCrashLog by remember { mutableStateOf(false) }
    // Read again after the dialog, which can delete it.
    val recorded = remember(showCrashLog) { crashLog.read() }
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = stringResource(R.string.back))
                    }
                },
                title = { Text(stringResource(R.string.menu_settings)) },
                windowInsets = WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Top),
            )
        },
        contentWindowInsets = WindowInsets.safeDrawing,
    ) { insets ->
        Column(Modifier.fillMaxSize().padding(insets).verticalScroll(rememberScrollState())) {
            Setting(
                stringResource(R.string.setting_keep_screen_on),
                pluralStringResource(
                    R.plurals.setting_keep_screen_on_detail, KEEP_SCREEN_ON_MINUTES, KEEP_SCREEN_ON_MINUTES,
                ),
                settings.keepScreenOn,
            ) { settings.keepScreenOn = it }
            Setting(
                stringResource(R.string.setting_volume_keys),
                stringResource(R.string.setting_volume_keys_detail),
                settings.volumeKeysTurnPages,
            ) { settings.volumeKeysTurnPages = it }
            // What went wrong, for a bug report; see [CrashLog].
            val last = remember(recorded) { lastCrashTime(recorded) }
            Entry(
                stringResource(R.string.crash_log),
                if (recorded.isBlank()) {
                    stringResource(R.string.crash_log_empty)
                } else {
                    stringResource(R.string.crash_log_detail, last?.let { documentDate(it.toString()) } ?: "")
                },
                enabled = recorded.isNotBlank(),
            ) { showCrashLog = true }
        }
    }
    if (showCrashLog) {
        CrashLogDialog(
            recorded,
            onDelete = {
                crashLog.clear()
                showCrashLog = false
            },
            onDismiss = { showCrashLog = false },
        )
    }
}

/** Identifies the text of the crash log to UI tests. */
const val CRASH_LOG_TAG = "crash-log"

/**
 * The crash log as it is, to read, to share (with a line saying which app
 * and device it is from) or to delete.
 */
@Composable
private fun CrashLogDialog(log: String, onDelete: () -> Unit, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val text = remember(log) { context.appAndDevice() + "\n\n" + log.trimEnd() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.crash_log)) },
        text = {
            SelectionContainer {
                Text(
                    text,
                    Modifier.verticalScroll(rememberScrollState()).testTag(CRASH_LOG_TAG),
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    try {
                        context.shareText(text)
                    } catch (e: RuntimeException) {
                        // Nothing on this device takes text.
                    }
                },
            ) { Text(stringResource(R.string.share)) }
        },
        dismissButton = {
            TextButton(onClick = onDelete) { Text(stringResource(R.string.delete)) }
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.close)) }
        },
    )
}

/** Something to open, with its explanation; the whole row is the button. */
@Composable
private fun Entry(title: String, detail: String, enabled: Boolean, onClick: () -> Unit) {
    Column(
        Modifier
            .fillMaxWidth()
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 24.dp, vertical = 16.dp)
            // Greyed out, as Material does for what can't be used.
            .alpha(if (enabled) 1f else 0.38f),
    ) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        Text(
            detail,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** One switch with its explanation; the whole row toggles it. */
@Composable
private fun Setting(title: String, detail: String, on: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .toggleable(on, role = Role.Switch, onValueChange = onChange)
            .padding(horizontal = 24.dp, vertical = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f).padding(end = 16.dp)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(
                detail,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        // The row is the control; the switch only shows the state.
        Switch(checked = on, onCheckedChange = null)
    }
}
