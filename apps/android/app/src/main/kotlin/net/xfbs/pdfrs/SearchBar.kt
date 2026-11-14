package net.xfbs.pdfrs

import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import uniffi.pdfrs_ffi.SearchStatus

/** Identifies the search field to UI tests. */
const val SEARCH_FIELD_TAG = "search-field"

/**
 * The top bar while searching: a field to type into, how many matches
 * there are and which one is shown, and buttons to the previous and next
 * one. Matches follow the typing. [onClose] leaves it (the arrow, or
 * Escape); the search itself is ended by whoever shows this.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SearchBar(state: PdfViewState, onClose: () -> Unit) {
    var query by remember(state) { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    val focusManager = LocalFocusManager.current
    // Typing starts right away, with the keyboard up.
    LaunchedEffect(state) { focus.requestFocus() }

    val status = state.searchStatus
    val found = (status?.matches ?: 0u) > 0u
    TopAppBar(
        windowInsets = WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Top),
        navigationIcon = {
            IconButton(onClick = onClose) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Close search")
            }
        },
        title = {
            val style = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface)
            BasicTextField(
                value = query,
                onValueChange = {
                    query = it
                    state.search(it)
                },
                modifier = Modifier
                    .fillMaxWidth()
                    .focusRequester(focus)
                    .testTag(SEARCH_FIELD_TAG)
                    // With a keyboard attached: Enter and Shift+Enter step
                    // through the matches, Escape leaves.
                    .onPreviewKeyEvent { event ->
                        if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                        when (event.key) {
                            Key.Enter, Key.NumPadEnter -> state.searchStep(forward = !event.isShiftPressed)
                            Key.Escape -> onClose()
                            else -> return@onPreviewKeyEvent false
                        }
                        true
                    },
                textStyle = style,
                singleLine = true,
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                // The matches are there already: make room to see them.
                keyboardActions = KeyboardActions(onSearch = { focusManager.clearFocus() }),
                decorationBox = { field ->
                    if (query.isEmpty()) {
                        Text("Search", style = style, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    field()
                },
            )
        },
        actions = {
            Text(
                searchSummary(status),
                style = LocalTextStyle.current.merge(MaterialTheme.typography.bodyMedium),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                modifier = Modifier.padding(horizontal = 4.dp),
            )
            IconButton(onClick = { state.searchStep(forward = false) }, enabled = found) {
                Icon(Icons.Filled.KeyboardArrowUp, contentDescription = "Previous match")
            }
            IconButton(onClick = { state.searchStep(forward = true) }, enabled = found) {
                Icon(Icons.Filled.KeyboardArrowDown, contentDescription = "Next match")
            }
        },
    )
}

/**
 * How a search stands, in a few characters: "3 of 17", with "…" while more
 * of the document is still being searched, or "No matches" once all of it
 * has been.
 */
internal fun searchSummary(status: SearchStatus?): String {
    status ?: return ""
    val searching = status.pagesSearched < status.pages
    return when {
        status.matches > 0u -> "${(status.current ?: 0u) + 1u} of ${status.matches}" + if (searching) "…" else ""
        searching -> "…"
        else -> "No matches"
    }
}
