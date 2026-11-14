package net.xfbs.pdfrs

import android.app.Activity
import android.app.ActivityManager
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.res.Configuration
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import uniffi.pdfrs_ffi.Activated
import uniffi.pdfrs_ffi.PdfException

class MainActivity : ComponentActivity() {
    /** The document to show: from "Open with", or picked in the app. */
    private var document by mutableStateOf<Uri?>(null)
    private var current: Loaded.Ready? = null
    private lateinit var store: DocumentStore

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        store = DocumentStore(File(filesDir, "documents.json"))
        document = intent.viewedDocument()
        setContent {
            AppTheme {
                App(
                    document = document,
                    store = store,
                    onDocument = { document = it },
                    onReady = { current = it },
                )
            }
        }
    }

    override fun onStop() {
        super.onStop()
        // The app may not come back: remember where the reader is.
        current?.let { store.saveView(it.key, it.state.savedView()) }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        // The activity isn't recreated when dark mode is switched, so the
        // system bar icons have to be told about the new background.
        enableEdgeToEdge()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        intent.viewedDocument()?.let { document = it }
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        // In the background, or the system is short on memory: keep only
        // what is on screen. Tiles are re-rendered on demand.
        if (level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW) current?.state?.trimMemory()
    }
}

private fun Intent.viewedDocument(): Uri? = data.takeIf { action == Intent.ACTION_VIEW }

/**
 * Follows the system: dark or light, and on Android 12+ the colours derived
 * from the wallpaper. Pages themselves stay as the document defines them.
 */
@Composable
private fun AppTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val colours = when {
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> darkColorScheme()
        else -> lightColorScheme()
    }
    MaterialTheme(colorScheme = colours, content = content)
}

/** Keeps content that isn't the page itself clear of the camera cutout and the navigation bar. */
private val besideAndBelow = WindowInsetsSides.Horizontal + WindowInsetsSides.Bottom

/** Duration of the bars sliding in and out. */
private const val BARS_MS = 220

/** How long the page number stays after a page turn in full screen. */
private const val INDICATOR_MS = 1500L

/**
 * Budget for tile bitmaps: a share of what the system considers normal for an
 * app here. A drawn tile also has a copy on the GPU, so the real cost is up
 * to twice this.
 */
private fun Context.tileCacheBytes(): Long {
    val memoryClassMb = (getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).memoryClass
    return (memoryClassMb / 4).coerceIn(32, 128) * 1024L * 1024L
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

    /** [key] identifies the document in the [DocumentStore]. */
    class Ready(val state: PdfViewState, val name: String, val key: String) : Loaded
    class Locked(val pdf: ByteArray, val name: String, val uri: Uri, val wrongPassword: Boolean) : Loaded
    class Failed(val message: String) : Loaded
}

/**
 * Makes sure the app can read [uri] again later, so that the document can be
 * offered on the start screen. Only some addresses allow that: those from the
 * system file picker do, most handed over by other apps don't.
 */
private fun Context.canReopen(uri: Uri): Boolean {
    if (uri.scheme == "file") return true
    return try {
        contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        true
    } catch (e: SecurityException) {
        false
    }
}

/** Gives back read permissions for documents the start screen no longer offers. */
private fun Context.releaseUnused(store: DocumentStore) {
    val keep = store.reopenable()
    for (permission in contentResolver.persistedUriPermissions) {
        if (permission.uri.toString() !in keep) {
            contentResolver.releasePersistableUriPermission(permission.uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
    }
}

/**
 * Opens [pdf], continuing where the reader left off if it was open before; a
 * missing or wrong [password] leads to [Loaded.Locked].
 */
private suspend fun Context.open(
    pdf: ByteArray,
    name: String,
    uri: Uri,
    password: String?,
    store: DocumentStore,
): Loaded = try {
    val cacheBytes = tileCacheBytes()
    val key = withContext(Dispatchers.Default) { DocumentStore.fingerprint(pdf) }
    val restore = store.find(key)?.view
    val state = withContext(Dispatchers.Default) { PdfViewState(pdf, password, cacheBytes, restore) }
    store.opened(key, name, uri.toString().takeIf { canReopen(uri) }, state.pageCount)
    releaseUnused(store)
    Loaded.Ready(state, name, key)
} catch (e: PdfException.PasswordRequired) {
    Loaded.Locked(pdf, name, uri, wrongPassword = password != null)
} catch (e: Exception) {
    Loaded.Failed(e.message ?: e.toString())
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun App(
    document: Uri?,
    store: DocumentStore,
    onDocument: (Uri?) -> Unit,
    onReady: (Loaded.Ready?) -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) onDocument(uri)
    }
    val pick = { picker.launch(arrayOf("application/pdf")) }

    var loaded by remember { mutableStateOf<Loaded?>(null) }
    var showPages by remember { mutableStateOf(false) }
    // Which tab the top-left button opens: the one used last, pages at first.
    var preferContents by remember { mutableStateOf(false) }
    var externalLink by remember { mutableStateOf<String?>(null) }
    // Tapping the page hides everything but the page; tapping again brings
    // the bars back.
    var fullScreen by remember { mutableStateOf(false) }
    var showGoTo by remember { mutableStateOf(false) }
    fun show(next: Loaded?) {
        (loaded as? Loaded.Ready)?.let {
            store.saveView(it.key, it.state.savedView())
            it.state.close()
        }
        loaded = next
        onReady(next as? Loaded.Ready)
    }

    LaunchedEffect(document) {
        showPages = false
        fullScreen = false
        show(null)
        if (document == null) return@LaunchedEffect
        loaded = Loaded.Loading
        show(
            try {
                val name = context.displayName(document)
                val pdf = withContext(Dispatchers.IO) {
                    checkNotNull(context.contentResolver.openInputStream(document)) { "cannot open $document" }
                        .use { it.readBytes() }
                }
                context.open(pdf, name, document, password = null, store)
            } catch (e: Exception) {
                // Moved, deleted, or the permission to read it is gone.
                store.unreachable(document.toString())
                Loaded.Failed(e.message ?: e.toString())
            },
        )
    }
    DisposableEffect(Unit) {
        onDispose { show(null) }
    }

    val ready = loaded as? Loaded.Ready
    val immersive = fullScreen && ready != null && !showPages
    BackHandler(enabled = showPages) { showPages = false }
    BackHandler(enabled = immersive) { fullScreen = false }
    StatusBar(hidden = immersive)

    val hasStrip = ready != null && !showPages && !immersive && ready.state.pageCount > 1
    // The document whose strip is showing, kept while the strip slides out.
    var stripState by remember { mutableStateOf<PdfViewState?>(null) }
    if (hasStrip) stripState = ready?.state else if (ready == null) stripState = null

    Scaffold(
        // The page view goes edge to edge: under the camera cutout (in
        // landscape, beside it) and under the navigation handle. Everything
        // else keeps clear of them by itself.
        contentWindowInsets = WindowInsets(0, 0, 0, 0),
        // The bars slide in and out. They take up their space as they go, so
        // the page area between them resizes smoothly and the page glides
        // rather than jumps.
        bottomBar = {
            AnimatedVisibility(
                visible = hasStrip,
                enter = expandVertically(tween(BARS_MS), expandFrom = Alignment.Top),
                exit = shrinkVertically(tween(BARS_MS), shrinkTowards = Alignment.Top),
            ) {
                // Still there while sliding out, when `ready` may be gone.
                stripState?.let { ThumbnailStrip(it) }
            }
        },
        topBar = {
            AnimatedVisibility(
                visible = !immersive,
                enter = expandVertically(tween(BARS_MS), expandFrom = Alignment.Bottom),
                exit = shrinkVertically(tween(BARS_MS), shrinkTowards = Alignment.Bottom),
            ) { TopAppBar(
                windowInsets = WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Top),
                navigationIcon = {
                    if (ready != null) {
                        IconButton(onClick = { showPages = !showPages }) {
                            if (showPages) {
                                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back to document")
                            } else {
                                Icon(Icons.Filled.Menu, contentDescription = "Contents and pages")
                            }
                        }
                    }
                },
                title = {
                    Text(
                        ready?.name ?: "pdfrs",
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                actions = {
                    var menu by remember { mutableStateOf(false) }
                    IconButton(onClick = { menu = true }) {
                        Icon(Icons.Filled.MoreVert, contentDescription = "More")
                    }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(
                            text = { Text("Open…") },
                            onClick = {
                                menu = false
                                pick()
                            },
                        )
                        if (document != null) {
                            // Back to the start screen and its recent documents.
                            DropdownMenuItem(
                                text = { Text("Close") },
                                onClick = {
                                    menu = false
                                    onDocument(null)
                                },
                            )
                        }
                        if (ready != null) {
                            DropdownMenuItem(
                                text = { Text("Go to page…") },
                                onClick = {
                                    menu = false
                                    showGoTo = true
                                },
                            )
                            // Not remembered: every document opens paged.
                            val continuous = !ready.state.paged
                            DropdownMenuItem(
                                text = { Text("Continuous scrolling") },
                                trailingIcon = {
                                    if (continuous) Icon(Icons.Filled.Check, contentDescription = "On")
                                },
                                onClick = {
                                    menu = false
                                    ready.state.paged = continuous
                                },
                            )
                        }
                    }
                },
            ) }
        },
    ) { insets ->
        Box(
            Modifier
                .fillMaxSize()
                .padding(insets)
                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
            contentAlignment = Alignment.Center,
        ) {
            val safe = Modifier.windowInsetsPadding(WindowInsets.safeDrawing.only(besideAndBelow))
            when (val l = loaded) {
                null -> StartScreen(
                    store.recents,
                    onOpen = pick,
                    onRecent = { entry -> entry.uri?.let { onDocument(Uri.parse(it)) } },
                    modifier = safe,
                )
                Loaded.Loading -> CircularProgressIndicator(safe)
                is Loaded.Failed -> Message("Couldn't open this file", l.message, "Open another PDF", pick, safe)
                is Loaded.Locked -> {
                    Message("${l.name} is password protected", "", "Open another PDF", pick, safe)
                    PasswordDialog(
                        name = l.name,
                        wrongPassword = l.wrongPassword,
                        onDismiss = { show(Loaded.Failed("A password is needed to open ${l.name}.")) },
                        onPassword = { password ->
                            loaded = Loaded.Loading
                            scope.launch { show(context.open(l.pdf, l.name, l.uri, password, store)) }
                        },
                    )
                }
                is Loaded.Ready ->
                    if (showPages) {
                        Navigator(
                            l.state,
                            startOnContents = preferContents,
                            onTab = { preferContents = it },
                            onPage = {
                                l.state.goToPage(it)
                                showPages = false
                            },
                            onOutlineEntry = {
                                (l.state.openOutlineEntry(it) as? Activated.Uri)?.let { link ->
                                    externalLink = link.uri
                                }
                                showPages = false
                            },
                        )
                    } else {
                        PdfView(
                            l.state,
                            Modifier.fillMaxSize(),
                            onExternalLink = { externalLink = it },
                            onTap = { fullScreen = !fullScreen },
                        )
                        // Always there in the normal view; in full screen it
                        // shows up on a page turn and leaves again.
                        var turned by remember(l.state) { mutableStateOf(false) }
                        LaunchedEffect(l.state, immersive) {
                            turned = false
                            snapshotFlow { l.state.currentPage }.drop(1).collectLatest {
                                turned = true
                                delay(INDICATOR_MS)
                                turned = false
                            }
                        }
                        AnimatedVisibility(
                            visible = !immersive || turned,
                            modifier = Modifier
                                .align(Alignment.BottomCenter)
                                // Above the strip if there is one, else above the navigation bar.
                                .then(if (hasStrip) Modifier else Modifier.navigationBarsPadding())
                                .padding(12.dp),
                            enter = fadeIn(),
                            exit = fadeOut(),
                        ) {
                            PageIndicator(l.state)
                        }
                    }
            }
        }
    }

    externalLink?.let { link ->
        ExternalLinkDialog(link, onDismiss = { externalLink = null })
    }

    if (showGoTo && ready != null) {
        GoToPageDialog(
            ready.state,
            onDismiss = { showGoTo = false },
            onPage = {
                showGoTo = false
                showPages = false
                ready.state.goToPage(it)
            },
        )
    }
}

/**
 * Hides or shows the status bar, for full screen. The navigation bar stays: hiding it makes
 * Android show its "viewing full screen" explanation and turns going home
 * into two swipes, for the sake of a thin handle.
 */
@Composable
private fun StatusBar(hidden: Boolean) {
    val view = LocalView.current
    LaunchedEffect(hidden) {
        val window = (view.context as? Activity)?.window ?: return@LaunchedEffect
        val bars = WindowCompat.getInsetsController(window, view)
        if (hidden) {
            bars.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            bars.hide(WindowInsetsCompat.Type.statusBars())
        } else {
            bars.show(WindowInsetsCompat.Type.statusBars())
        }
    }
}

@Composable
private fun Message(
    title: String,
    detail: String,
    action: String,
    onAction: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier.padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
        if (detail.isNotEmpty()) Text(detail, textAlign = TextAlign.Center)
        Button(onClick = onAction) { Text(action) }
    }
}

/**
 * What the top-left button opens: the page grid and, if the document has an
 * outline, its contents as a second tab. The tabs can also be swiped.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Navigator(
    state: PdfViewState,
    startOnContents: Boolean,
    onTab: (contents: Boolean) -> Unit,
    onPage: (Int) -> Unit,
    onOutlineEntry: (Int) -> Unit,
) {
    val background = Modifier
        .fillMaxSize()
        .background(MaterialTheme.colorScheme.surface)
        .windowInsetsPadding(WindowInsets.safeDrawing.only(besideAndBelow))
    if (state.outline.isEmpty()) {
        PagesGrid(state, onPage, background)
        return
    }
    val scope = rememberCoroutineScope()
    val pager = rememberPagerState(initialPage = if (startOnContents) 1 else 0) { 2 }
    LaunchedEffect(pager.settledPage) { onTab(pager.settledPage == 1) }
    Column(background) {
        PrimaryTabRow(selectedTabIndex = pager.currentPage) {
            for ((index, title) in listOf("Pages", "Contents").withIndex()) {
                Tab(
                    selected = pager.currentPage == index,
                    onClick = { scope.launch { pager.animateScrollToPage(index) } },
                    text = { Text(title) },
                )
            }
        }
        HorizontalPager(pager, Modifier.fillMaxSize()) { tab ->
            if (tab == 0) {
                PagesGrid(state, onPage, Modifier.fillMaxSize())
            } else {
                OutlineList(state, onOutlineEntry, Modifier.fillMaxSize())
            }
        }
    }
}

/** Schemes a link in a document may open, after the user confirmed. */
private val OPENABLE_SCHEMES = setOf("http", "https", "mailto")

/**
 * A link leaving the document was tapped. Documents can link anywhere, so
 * the address is shown and nothing opens without confirmation.
 */
@Composable
private fun ExternalLinkDialog(link: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val uri = remember(link) { Uri.parse(link) }
    val openable = uri.scheme?.lowercase() in OPENABLE_SCHEMES
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (openable) "Open this link?" else "This link can't be opened") },
        text = { Text(link, maxLines = 6, overflow = TextOverflow.Ellipsis) },
        confirmButton = {
            if (openable) {
                TextButton(
                    onClick = {
                        onDismiss()
                        try {
                            context.startActivity(Intent(Intent.ACTION_VIEW, uri))
                        } catch (e: ActivityNotFoundException) {
                            // No app for it; nothing to do.
                        }
                    },
                ) { Text("Open") }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(if (openable) "Cancel" else "Close") } },
    )
}

@Composable
private fun PasswordDialog(
    name: String,
    wrongPassword: Boolean,
    onDismiss: () -> Unit,
    onPassword: (String) -> Unit,
) {
    var password by remember { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Password required") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("Enter the password to open $name.")
                OutlinedTextField(
                    value = password,
                    onValueChange = { password = it },
                    modifier = Modifier.focusRequester(focus),
                    label = { Text("Password") },
                    singleLine = true,
                    isError = wrongPassword,
                    supportingText = if (wrongPassword) {
                        { Text("That password didn't work.") }
                    } else {
                        null
                    },
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { onPassword(password) }),
                )
            }
        },
        confirmButton = { TextButton(onClick = { onPassword(password) }) { Text("Open") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun GoToPageDialog(state: PdfViewState, onDismiss: () -> Unit, onPage: (Int) -> Unit) {
    var text by remember { mutableStateOf("") }
    val page = if (text.isBlank()) null else state.findPage(text)
    val last = state.pageCount - 1
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Go to page") },
        text = {
            OutlinedTextField(
                value = text,
                // Printed page numbers can be anything ("iv", "A-3"); plain
                // ones are digits.
                onValueChange = { new ->
                    text = if (state.hasPageLabels) new.take(20) else new.filter(Char::isDigit).take(9)
                },
                modifier = Modifier.focusRequester(focus),
                label = { Text("Page (${state.pageLabel(0)}–${state.pageLabel(last)})") },
                supportingText = if (state.hasPageLabels) {
                    {
                        // A typed number may be a printed number or a position.
                        Text(page?.let { "Page ${it + 1} of ${state.pageCount}" } ?: "Or a position, 1–${state.pageCount}")
                    }
                } else {
                    null
                },
                singleLine = true,
                isError = text.isNotBlank() && page == null,
                keyboardOptions = KeyboardOptions(
                    keyboardType = if (state.hasPageLabels) KeyboardType.Ascii else KeyboardType.Number,
                    imeAction = ImeAction.Go,
                ),
                keyboardActions = KeyboardActions(onGo = { page?.let(onPage) }),
            )
        },
        confirmButton = {
            TextButton(onClick = { page?.let(onPage) }, enabled = page != null) { Text("Go") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun PageIndicator(state: PdfViewState, modifier: Modifier = Modifier) {
    val current = state.currentPage
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(50),
        color = MaterialTheme.colorScheme.inverseSurface.copy(alpha = 0.8f),
        contentColor = MaterialTheme.colorScheme.inverseOnSurface,
    ) {
        Text(
            // With printed page numbers: "iv (4 / 120)".
            if (state.hasPageLabels) {
                "${state.pageLabel(current)} (${current + 1} / ${state.pageCount})"
            } else {
                "${current + 1} / ${state.pageCount}"
            },
            Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
            style = MaterialTheme.typography.labelLarge,
        )
    }
}
