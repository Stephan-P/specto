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
import android.os.Handler
import android.os.Looper
import android.provider.OpenableColumns
import android.view.KeyEvent
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
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
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import java.io.File
import kotlinx.coroutines.CancellationException
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
import uniffi.pdfrs_ffi.PdfFile
import uniffi.pdfrs_ffi.PdfException

class MainActivity : ComponentActivity() {
    /** The document to show: from "Open with", or picked in the app. */
    private var document by mutableStateOf<Uri?>(null)
    private var current: Loaded.Ready? = null
    private lateinit var store: DocumentStore
    private lateinit var settings: AppSettings

    /** Moves on or back through the document being read, for the volume keys; null while none is on screen. */
    private var pageKeys: ((forward: Boolean) -> Unit)? = null

    private val main = Handler(Looper.getMainLooper())
    private val letScreenTurnOff = Runnable { window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        store = DocumentStore(File(filesDir, "documents.json"))
        settings = AppSettings(getSharedPreferences("settings", Context.MODE_PRIVATE))
        document = intent.viewedDocument()
        setContent {
            // Also when the switch itself is flipped.
            val keepScreenOn = settings.keepScreenOn
            LaunchedEffect(keepScreenOn) { stayAwake() }
            AppTheme {
                App(
                    document = document,
                    store = store,
                    settings = settings,
                    onDocument = { document = it },
                    onReady = {
                        current = it
                        stayAwake()
                    },
                    onPageKeys = { pageKeys = it },
                )
            }
        }
    }

    /**
     * With [AppSettings.keepScreenOn] and a document open, keeps the screen
     * on for a while from now; called whenever the reader does something. A
     * phone left lying on the table still goes dark in the end.
     */
    private fun stayAwake() {
        main.removeCallbacks(letScreenTurnOff)
        if (settings.keepScreenOn && current != null) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            main.postDelayed(letScreenTurnOff, KEEP_SCREEN_ON_MINUTES * 60_000L)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }

    override fun onUserInteraction() {
        super.onUserInteraction()
        stayAwake()
    }

    override fun onDestroy() {
        main.removeCallbacks(letScreenTurnOff)
        super.onDestroy()
    }

    /** Whether [keyCode] is a volume key that is to move through the document, and which way: true for volume down (onwards). */
    private fun pageKey(keyCode: Int): Boolean? = when {
        pageKeys == null || !settings.volumeKeysTurnPages -> null
        keyCode == KeyEvent.KEYCODE_VOLUME_DOWN -> true
        keyCode == KeyEvent.KEYCODE_VOLUME_UP -> false
        else -> null
    }

    // Only keys nothing else wanted arrive here: with a menu or dialog open,
    // the volume keys change the volume as usual.
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        val forward = pageKey(keyCode) ?: return super.onKeyDown(keyCode, event)
        pageKeys?.invoke(forward)
        return true
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean =
        // Its key down was used, so the system shouldn't see half a key press.
        pageKey(keyCode) != null || super.onKeyUp(keyCode, event)

    override fun onStop() {
        super.onStop()
        // The app may not come back: remember where the reader is.
        current?.let { store.saveView(it.key, it.state.savedView(), it.state.sidebarOpen) }
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

/** Identifies the page area to UI tests. */
const val PAGE_VIEW_TAG = "page-view"

/** Identifies the sidebar of wide windows to UI tests. */
const val SIDEBAR_TAG = "sidebar"

/** Windows at least this wide (in dp) show Pages and Contents beside the document. */
const val WIDE_WINDOW_DP = 840

private val SIDEBAR_WIDTH = 300.dp

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
    class Ready(val state: PdfViewState, val name: String, val key: String, val bytes: Long) : Loaded
    /** Keeps the [file], so that trying a password doesn't read it again. */
    class Locked(val file: PdfFile, val name: String, val uri: Uri, val wrongPassword: Boolean) : Loaded
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
 * Hands the document at [uri] to Rust, which reads it straight from the
 * file, or for a large one maps it and reads only the parts it shows. Either
 * way its bytes never pass through this app's own (much smaller) Java memory.
 * Throws if it can't be read.
 */
private fun Context.pdfFile(uri: Uri): PdfFile {
    val descriptor = checkNotNull(contentResolver.openFileDescriptor(uri, "r")) { "cannot open $uri" }
    // Rust takes the descriptor over and closes it.
    return PdfFile.fromFd(descriptor.detachFd())
}

/**
 * Opens [file], continuing where the reader left off if it was open before;
 * a missing or wrong [password] leads to [Loaded.Locked]. Unless it ends
 * there, [file] is closed: an open document keeps its own hold on the bytes.
 */
private suspend fun Context.open(
    file: PdfFile,
    name: String,
    uri: Uri,
    password: String?,
    store: DocumentStore,
): Loaded {
    val opened = try {
        val cacheBytes = tileCacheBytes()
        val size = file.size().toLong()
        val key = withContext(Dispatchers.Default) {
            DocumentStore.fingerprint(size) { offset, length -> file.read(offset.toULong(), length.toUInt()) }
        }
        val known = store.find(key)
        val wide = resources.configuration.screenWidthDp >= WIDE_WINDOW_DP
        val state = withContext(Dispatchers.Default) {
            PdfViewState(file, password, cacheBytes, known?.view, wide, known?.sidebar ?: true)
        }
        store.opened(key, name, uri.toString().takeIf { canReopen(uri) }, state.pageCount)
        releaseUnused(store)
        Loaded.Ready(state, name, key, size)
    } catch (e: PdfException.PasswordRequired) {
        return Loaded.Locked(file, name, uri, wrongPassword = password != null)
    } catch (e: Exception) {
        Loaded.Failed(e.message ?: e.toString())
    }
    file.close()
    return opened
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun App(
    document: Uri?,
    store: DocumentStore,
    settings: AppSettings,
    onDocument: (Uri?) -> Unit,
    onReady: (Loaded.Ready?) -> Unit,
    /** Hands over what the volume keys should do; null while no document is on screen. */
    onPageKeys: (((forward: Boolean) -> Unit)?) -> Unit,
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
    var showProperties by remember { mutableStateOf(false) }
    var showSettings by remember { mutableStateOf(false) }
    fun show(next: Loaded?) {
        (loaded as? Loaded.Ready)?.let {
            store.saveView(it.key, it.state.savedView(), it.state.sidebarOpen)
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
                val file = withContext(Dispatchers.IO) { context.pdfFile(document) }
                context.open(file, name, document, password = null, store)
            } catch (e: Exception) {
                // Moved, deleted, or the permission to read it is gone.
                store.unreachable(document.toString())
                Loaded.Failed((e as? PdfException.Io)?.reason ?: e.message ?: e.toString())
            },
        )
    }
    DisposableEffect(Unit) {
        onDispose { show(null) }
    }

    val ready = loaded as? Loaded.Ready
    // A wide window (a tablet, but not one sharing its screen) has room for
    // Pages and Contents in a sidebar next to the document. Narrower ones
    // show them full screen instead, on request.
    val wide = LocalConfiguration.current.screenWidthDp >= WIDE_WINDOW_DP
    val overlay = showPages && !wide
    val immersive = fullScreen && ready != null && !overlay
    val sidebarShown = wide && ready != null && ready.state.sidebarOpen && !immersive
    // Two pages side by side need the room, too.
    ready?.state?.let { SideEffect { it.wideWindow = wide } }
    BackHandler(enabled = overlay) { showPages = false }
    BackHandler(enabled = immersive) { fullScreen = false }
    StatusBar(hidden = immersive)

    // The sidebar shows the same thumbnails, so the strip would only repeat them.
    val hasStrip = ready != null && !overlay && !immersive && !sidebarShown && ready.state.pageCount > 1
    // The document whose strip is showing, kept while the strip slides out.
    var stripState by remember { mutableStateOf<PdfViewState?>(null) }
    if (hasStrip) stripState = ready?.state else if (ready == null) stripState = null

    if (showSettings) {
        // In place of everything else; the document stays open underneath.
        BackHandler { showSettings = false }
        SideEffect { onPageKeys(null) }
        SettingsScreen(settings, onBack = { showSettings = false })
        return
    }
    // The volume keys move through the document like Page Down and Page Up,
    // while it is what the reader sees.
    val pageKeys: ((Boolean) -> Unit)? =
        if (ready != null && !overlay) { forward -> ready.state.pageBy(scope, forward) } else null
    SideEffect { onPageKeys(pageKeys) }

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
                        IconButton(
                            onClick = {
                                if (wide) ready.state.sidebarOpen = !ready.state.sidebarOpen else showPages = !showPages
                            },
                        ) {
                            if (overlay) {
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
                    // How this document is shown; everything else is behind ⋮.
                    if (ready != null && !overlay) ViewMenu(ready.state, wide)
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
                        if (ready != null && document != null) {
                            DropdownMenuItem(
                                text = { Text("Share…") },
                                onClick = {
                                    menu = false
                                    scope.launch {
                                        try {
                                            context.shareDocument(document, ready.name)
                                        } catch (e: CancellationException) {
                                            throw e
                                        } catch (e: Exception) {
                                            // Moved or deleted since it was opened.
                                            Toast.makeText(context, "Couldn't share this file", Toast.LENGTH_LONG).show()
                                        }
                                    }
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
                            DropdownMenuItem(
                                text = { Text("Properties") },
                                onClick = {
                                    menu = false
                                    showProperties = true
                                },
                            )
                        }
                        DropdownMenuItem(
                            text = { Text("Settings") },
                            onClick = {
                                menu = false
                                showSettings = true
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
                    // A fresh dialog for every attempt, so a wrong password
                    // doesn't stay in the field.
                    key(l) { PasswordDialog(
                        name = l.name,
                        wrongPassword = l.wrongPassword,
                        onDismiss = {
                            l.file.close()
                            show(Loaded.Failed("A password is needed to open ${l.name}."))
                        },
                        onPassword = { password ->
                            loaded = Loaded.Loading
                            scope.launch { show(context.open(l.file, l.name, l.uri, password, store)) }
                        },
                    ) }
                }
                is Loaded.Ready -> {
                    val navigator: @Composable (Modifier, WindowInsetsSides, Boolean) -> Unit =
                        { modifier, insets, closeAfterwards ->
                            Navigator(
                                l.state,
                                startOnContents = preferContents,
                                onTab = { preferContents = it },
                                onPage = {
                                    l.state.goToPage(it)
                                    if (closeAfterwards) showPages = false
                                },
                                onOutlineEntry = {
                                    (l.state.openOutlineEntry(it) as? Activated.Uri)?.let { link ->
                                        externalLink = link.uri
                                    }
                                    if (closeAfterwards) showPages = false
                                },
                                modifier = modifier,
                                insets = insets,
                                // Beside the document, it keeps up with it.
                                follow = !closeAfterwards,
                            )
                        }
                    if (overlay) {
                        navigator(Modifier.fillMaxSize(), besideAndBelow, true)
                    } else Row(Modifier.fillMaxSize()) {
                        // The sidebar takes its width as it slides, so the
                        // page area resizes smoothly, as with the bars.
                        AnimatedVisibility(
                            visible = sidebarShown,
                            enter = expandHorizontally(tween(BARS_MS), expandFrom = Alignment.End),
                            exit = shrinkHorizontally(tween(BARS_MS), shrinkTowards = Alignment.End),
                        ) {
                            navigator(
                                Modifier.width(SIDEBAR_WIDTH).fillMaxHeight().testTag(SIDEBAR_TAG),
                                WindowInsetsSides.Start + WindowInsetsSides.Bottom,
                                false,
                            )
                        }
                        Box(Modifier.weight(1f).fillMaxHeight()) {
                                PdfView(
                                    l.state,
                                    // The tag is how UI tests find the page area.
                                    Modifier.fillMaxSize().testTag(PAGE_VIEW_TAG),
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
                                // Fully qualified: inside the Row, the plain name
                                // would mean the variant that animates a row's child.
                                androidx.compose.animation.AnimatedVisibility(
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
        }
    }

    externalLink?.let { link ->
        ExternalLinkDialog(link, onDismiss = { externalLink = null })
    }

    if (showProperties && ready != null) {
        PropertiesDialog(ready.state, ready.name, ready.bytes, onDismiss = { showProperties = false })
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
 * What the top-left button opens, full screen or as a sidebar: the page grid
 * and, if the document has an outline, its contents as a second tab. The
 * tabs can also be swiped.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Navigator(
    state: PdfViewState,
    startOnContents: Boolean,
    onTab: (contents: Boolean) -> Unit,
    onPage: (Int) -> Unit,
    onOutlineEntry: (Int) -> Unit,
    modifier: Modifier,
    /** Which screen edges it touches, to keep clear of cutouts and the navigation bar there. */
    insets: WindowInsetsSides,
    /** Whether the page list keeps the current page in view (shown beside the document). */
    follow: Boolean,
) {
    val background = modifier
        .background(MaterialTheme.colorScheme.surface)
        .windowInsetsPadding(WindowInsets.safeDrawing.only(insets))
    if (state.outline.isEmpty()) {
        PagesGrid(state, onPage, background, beside = follow)
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
                PagesGrid(state, onPage, Modifier.fillMaxSize(), beside = follow)
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
        // "4 / 120", or for a pair "4–5 / 120"; with printed page numbers
        // "iv (4 / 120)" and "iv–v (4–5 / 120)".
        val last = state.lastPage
        val positions = if (last > current) "${current + 1}–${last + 1}" else "${current + 1}"
        Text(
            if (state.hasPageLabels) {
                val labels =
                    if (last > current) "${state.pageLabel(current)}–${state.pageLabel(last)}"
                    else state.pageLabel(current)
                "$labels ($positions / ${state.pageCount})"
            } else {
                "$positions / ${state.pageCount}"
            },
            Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
            style = MaterialTheme.typography.labelLarge,
        )
    }
}
