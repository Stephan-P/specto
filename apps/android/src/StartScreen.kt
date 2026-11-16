package net.xfbs.specto

import android.text.format.DateUtils
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/**
 * What the app shows without a document: a button to open one, and the documents read recently,
 * each with where the reader left off.
 */
@Composable
fun StartScreen(
    recents: List<DocumentStore.Entry>,
    onOpen: () -> Unit,
    onRecent: (DocumentStore.Entry) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (recents.isEmpty()) {
        Box(modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Button(onClick = onOpen) { Text(stringResource(R.string.open_pdf)) }
        }
        return
    }
    LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(vertical = 16.dp)) {
        item {
            Column(
                Modifier.fillMaxWidth().padding(horizontal = 24.dp),
                verticalArrangement = Arrangement.spacedBy(24.dp),
            ) {
                Button(onClick = onOpen, modifier = Modifier.align(Alignment.CenterHorizontally)) {
                    Text(stringResource(R.string.open_pdf))
                }
                Text(
                    stringResource(R.string.recent),
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
        }
        items(recents, key = { it.key }) { entry ->
            Column(
                Modifier
                    .fillMaxWidth()
                    .clickable { onRecent(entry) }
                    .padding(horizontal = 24.dp, vertical = 12.dp),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    entry.name,
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    entry.summary(),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            HorizontalDivider(Modifier.padding(horizontal = 24.dp))
        }
    }
}

/** "Page 12 of 240 · 2 hours ago" */
@Composable
private fun DocumentStore.Entry.summary(): String {
    val ago = DateUtils.getRelativeTimeSpanString(
        opened,
        System.currentTimeMillis(),
        DateUtils.MINUTE_IN_MILLIS,
    ).toString()
    val page = view?.page?.toInt()?.plus(1) ?: return ago
    val where =
        if (pageCount > 0) {
            stringResource(R.string.recent_page_of, page, pageCount)
        } else {
            stringResource(R.string.recent_page, page)
        }
    return stringResource(R.string.recent_summary, where, ago)
}
