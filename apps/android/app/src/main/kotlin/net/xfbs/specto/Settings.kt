package net.xfbs.specto

import android.content.SharedPreferences
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
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
fun SettingsScreen(settings: AppSettings, onBack: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                title = { Text("Settings") },
                windowInsets = WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Top),
            )
        },
        contentWindowInsets = WindowInsets.safeDrawing,
    ) { insets ->
        Column(Modifier.fillMaxSize().padding(insets).verticalScroll(rememberScrollState())) {
            Setting(
                "Keep the screen on",
                "While a document is open, for $KEEP_SCREEN_ON_MINUTES minutes after the last touch",
                settings.keepScreenOn,
            ) { settings.keepScreenOn = it }
            Setting(
                "Volume keys turn pages",
                "Volume down goes on, volume up goes back",
                settings.volumeKeysTurnPages,
            ) { settings.volumeKeysTurnPages = it }
        }
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
