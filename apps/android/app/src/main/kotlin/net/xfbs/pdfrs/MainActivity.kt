package net.xfbs.pdfrs

import android.app.ActivityManager
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
    /** The document to show: from "Open with", or picked in the app. */
    private var document by mutableStateOf<Uri?>(null)
    private var viewState: PdfViewState? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        document = intent.viewedDocument()
        setContent {
            MaterialTheme {
                App(
                    document = document,
                    onPicked = { document = it },
                    onViewState = { viewState = it },
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        intent.viewedDocument()?.let { document = it }
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        // In the background, or the system is short on memory: keep only
        // what is on screen. Tiles are re-rendered on demand.
        if (level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW) viewState?.trimMemory()
    }
}

private fun Intent.viewedDocument(): Uri? = data.takeIf { action == Intent.ACTION_VIEW }

/** Budget for tile bitmaps: a share of what the system considers normal for an app here. */
private fun Context.tileCacheBytes(): Long {
    val memoryClassMb = (getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).memoryClass
    return (memoryClassMb / 2).coerceIn(48, 256) * 1024L * 1024L
}

private fun Context.displayName(uri: Uri): String {
    val fromProvider = runCatching {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
            if (it.moveToFirst()) it.getString(0) else null
        }
    }.getOrNull()
    return fromProvider ?: uri.lastPathSegment ?: "PDF"
}

private sealed interface Loaded {
    data object Loading : Loaded
    data class Ready(val state: PdfViewState, val name: String) : Loaded
    data class Failed(val message: String) : Loaded
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun App(document: Uri?, onPicked: (Uri) -> Unit, onViewState: (PdfViewState?) -> Unit) {
    val context = LocalContext.current
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) onPicked(uri)
    }
    val pick = { picker.launch(arrayOf("application/pdf")) }

    var loaded by remember { mutableStateOf<Loaded?>(null) }
    LaunchedEffect(document) {
        (loaded as? Loaded.Ready)?.state?.close()
        onViewState(null)
        loaded = null
        if (document == null) return@LaunchedEffect
        loaded = Loaded.Loading
        loaded = try {
            val bytes = withContext(Dispatchers.IO) {
                checkNotNull(context.contentResolver.openInputStream(document)) { "cannot open $document" }
                    .use { it.readBytes() }
            }
            val state = withContext(Dispatchers.Default) { PdfViewState(bytes, context.tileCacheBytes()) }
            onViewState(state)
            Loaded.Ready(state, context.displayName(document))
        } catch (e: Exception) {
            Loaded.Failed(e.message ?: e.toString())
        }
    }
    DisposableEffect(Unit) {
        onDispose {
            (loaded as? Loaded.Ready)?.state?.close()
            onViewState(null)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        (loaded as? Loaded.Ready)?.name ?: "pdfrs",
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                actions = { TextButton(onClick = pick) { Text("Open") } },
            )
        },
    ) { insets ->
        Box(
            Modifier
                .fillMaxSize()
                .padding(insets)
                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
            contentAlignment = Alignment.Center,
        ) {
            when (val l = loaded) {
                null -> Button(onClick = pick) { Text("Open PDF") }
                Loaded.Loading -> CircularProgressIndicator()
                is Loaded.Failed -> Column(
                    Modifier.padding(24.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text("Couldn't open this file", style = MaterialTheme.typography.titleMedium)
                    Text(l.message, textAlign = TextAlign.Center)
                    Button(onClick = pick) { Text("Open another PDF") }
                }
                is Loaded.Ready -> {
                    PdfView(l.state, Modifier.fillMaxSize())
                    PageIndicator(l.state, Modifier.align(Alignment.BottomCenter).padding(16.dp))
                }
            }
        }
    }
}

@Composable
private fun PageIndicator(state: PdfViewState, modifier: Modifier = Modifier) {
    val frame = state.frame ?: return
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(50),
        color = MaterialTheme.colorScheme.inverseSurface.copy(alpha = 0.8f),
        contentColor = MaterialTheme.colorScheme.inverseOnSurface,
    ) {
        Text(
            "${frame.currentPage.toInt() + 1} / ${state.pageCount}",
            Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
            style = MaterialTheme.typography.labelLarge,
        )
    }
}
