package net.xfbs.pdfrs

import android.app.ActivityManager
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
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
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.pdfrs_ffi.PdfException

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
    class Ready(val state: PdfViewState, val name: String) : Loaded
    class Locked(val pdf: ByteArray, val name: String, val wrongPassword: Boolean) : Loaded
    class Failed(val message: String) : Loaded
}

/** Opens [pdf]; a missing or wrong [password] leads to [Loaded.Locked]. */
private suspend fun Context.open(pdf: ByteArray, name: String, password: String?): Loaded = try {
    val cacheBytes = tileCacheBytes()
    Loaded.Ready(withContext(Dispatchers.Default) { PdfViewState(pdf, password, cacheBytes) }, name)
} catch (e: PdfException.PasswordRequired) {
    Loaded.Locked(pdf, name, wrongPassword = password != null)
} catch (e: Exception) {
    Loaded.Failed(e.message ?: e.toString())
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun App(document: Uri?, onPicked: (Uri) -> Unit, onViewState: (PdfViewState?) -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) onPicked(uri)
    }
    val pick = { picker.launch(arrayOf("application/pdf")) }

    var loaded by remember { mutableStateOf<Loaded?>(null) }
    var showPages by remember { mutableStateOf(false) }
    // Which tab the top-left button opens: the one used last, pages at first.
    var preferContents by remember { mutableStateOf(false) }
    var externalLink by remember { mutableStateOf<String?>(null) }
    var showGoTo by remember { mutableStateOf(false) }
    fun show(next: Loaded?) {
        (loaded as? Loaded.Ready)?.state?.close()
        loaded = next
        onViewState((next as? Loaded.Ready)?.state)
    }

    LaunchedEffect(document) {
        showPages = false
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
                context.open(pdf, name, password = null)
            } catch (e: Exception) {
                Loaded.Failed(e.message ?: e.toString())
            },
        )
    }
    DisposableEffect(Unit) {
        onDispose { show(null) }
    }

    val ready = loaded as? Loaded.Ready
    BackHandler(enabled = showPages) { showPages = false }

    Scaffold(
        topBar = {
            TopAppBar(
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
                is Loaded.Failed -> Message("Couldn't open this file", l.message, "Open another PDF", pick)
                is Loaded.Locked -> {
                    Message("${l.name} is password protected", "", "Open another PDF", pick)
                    PasswordDialog(
                        name = l.name,
                        wrongPassword = l.wrongPassword,
                        onDismiss = { show(Loaded.Failed("A password is needed to open ${l.name}.")) },
                        onPassword = { password ->
                            loaded = Loaded.Loading
                            scope.launch { show(context.open(l.pdf, l.name, password)) }
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
                                externalLink = l.state.openOutlineEntry(it)
                                showPages = false
                            },
                        )
                    } else {
                        PdfView(l.state, Modifier.fillMaxSize(), onExternalLink = { externalLink = it })
                        PageIndicator(l.state, Modifier.align(Alignment.BottomCenter).padding(16.dp))
                    }
            }
        }
    }

    externalLink?.let { link ->
        ExternalLinkDialog(link, onDismiss = { externalLink = null })
    }

    if (showGoTo && ready != null) {
        GoToPageDialog(
            pageCount = ready.state.pageCount,
            onDismiss = { showGoTo = false },
            onPage = {
                showGoTo = false
                showPages = false
                ready.state.goToPage(it)
            },
        )
    }
}

@Composable
private fun Message(title: String, detail: String, action: String, onAction: () -> Unit) {
    Column(
        Modifier.padding(24.dp),
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
    val background = Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface)
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
private fun GoToPageDialog(pageCount: Int, onDismiss: () -> Unit, onPage: (Int) -> Unit) {
    var text by remember { mutableStateOf("") }
    val page = text.toIntOrNull()?.takeIf { it in 1..pageCount }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Go to page") },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { new -> text = new.filter(Char::isDigit).take(9) },
                modifier = Modifier.focusRequester(focus),
                label = { Text("Page (1–$pageCount)") },
                singleLine = true,
                isError = text.isNotEmpty() && page == null,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { page?.let { onPage(it - 1) } }),
            )
        },
        confirmButton = {
            TextButton(onClick = { page?.let { onPage(it - 1) } }, enabled = page != null) { Text("Go") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
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
            "${frame.currentPage + 1} / ${state.pageCount}",
            Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
            style = MaterialTheme.typography.labelLarge,
        )
    }
}
