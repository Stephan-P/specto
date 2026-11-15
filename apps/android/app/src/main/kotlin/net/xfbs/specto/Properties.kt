package net.xfbs.specto

import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import java.time.DateTimeException
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale
import kotlin.math.abs
import kotlin.math.roundToInt
import uniffi.specto_ffi.DocumentInfo
import uniffi.specto_ffi.PageSize

/**
 * What is known about the open document: what it says about itself (which
 * may be nothing, or wrong) and what the app can see. The values can be
 * selected and copied.
 */
@Composable
fun PropertiesDialog(state: PdfViewState, name: String, bytes: Long, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val rows = remember(state) {
        propertyRows(
            name = name,
            info = state.info(),
            pageCount = state.pageCount,
            pageSize = pageSizesText(state.pageSizes),
            fileSize = Formatter.formatShortFileSize(context, bytes),
            date = ::documentDate,
        )
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Properties") },
        text = {
            SelectionContainer {
                Column(
                    Modifier.verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    for ((label, value) in rows) {
                        Column {
                            Text(
                                label,
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                            Text(value, style = MaterialTheme.typography.bodyLarge)
                        }
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Close") } },
    )
}

/**
 * The dialog's rows, as label and value, in the order shown. What the
 * document doesn't state is left out. [date] turns a date as the document
 * gives it (ISO 8601) into what the reader sees.
 */
internal fun propertyRows(
    name: String,
    info: DocumentInfo,
    pageCount: Int,
    pageSize: String?,
    fileSize: String,
    date: (String) -> String,
): List<Pair<String, String>> = listOfNotNull(
    "File name" to name,
    info.title?.let { "Title" to it },
    info.author?.let { "Author" to it },
    info.subject?.let { "Subject" to it },
    info.keywords?.let { "Keywords" to it },
    info.creator?.let { "Created with" to it },
    info.producer?.let { "PDF producer" to it },
    info.created?.let { "Created" to date(it) },
    info.modified?.let { "Modified" to date(it) },
    "Pages" to pageCount.toString(),
    pageSize?.let { "Page size" to it },
    "File size" to fileSize,
    "PDF version" to info.pdfVersion,
)

/**
 * A date from a document (ISO 8601 with an offset), on the reader's clock
 * and in the reader's language. A date that isn't one (the 31st of
 * February) is shown as it is.
 */
internal fun documentDate(
    iso: String,
    zone: ZoneId = ZoneId.systemDefault(),
    locale: Locale = Locale.getDefault(),
): String = try {
    OffsetDateTime.parse(iso)
        .atZoneSameInstant(zone)
        .format(DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT).withLocale(locale))
} catch (e: DateTimeException) {
    iso
}

/** Paper sizes people know by name: shorter side, longer side (in points), name, and whether they are measured in inches. */
private val PAPER_SIZES = listOf(
    PaperSize(841.9f, 1190.6f, "A3"),
    PaperSize(595.3f, 841.9f, "A4"),
    PaperSize(419.5f, 595.3f, "A5"),
    PaperSize(498.9f, 708.7f, "B5"),
    PaperSize(612f, 792f, "Letter", inches = true),
    PaperSize(612f, 1008f, "Legal", inches = true),
    PaperSize(792f, 1224f, "Tabloid", inches = true),
)

private class PaperSize(val short: Float, val long: Float, val name: String, val inches: Boolean = false)

/** How far a page may be from a paper size and still be called by its name, in points. */
private const val PAPER_TOLERANCE = 2f

/**
 * A page size for people: "210 × 297 mm (A4)", "11 × 8.5 in (Letter,
 * landscape)", or just "152 × 229 mm" for a size without a name. Width
 * first; [width] and [height] are in points.
 */
internal fun pageSizeText(width: Float, height: Float): String {
    val (short, long) = if (width <= height) width to height else height to width
    val paper = PAPER_SIZES.firstOrNull {
        abs(it.short - short) <= PAPER_TOLERANCE && abs(it.long - long) <= PAPER_TOLERANCE
    }
    val size = if (paper?.inches == true) {
        fun inches(points: Float) = "%.2f".format(Locale.ROOT, points / 72).trimEnd('0').trimEnd('.')
        "${inches(width)} × ${inches(height)} in"
    } else {
        fun millimetres(points: Float) = (points / 72 * 25.4f).roundToInt()
        "${millimetres(width)} × ${millimetres(height)} mm"
    }
    val landscape = width > height
    return when {
        paper != null && landscape -> "$size (${paper.name}, landscape)"
        paper != null -> "$size (${paper.name})"
        else -> size
    }
}

/** The size of a document's pages: that of the first, with a note if others differ. */
internal fun pageSizesText(sizes: List<PageSize>): String? {
    val first = sizes.firstOrNull() ?: return null
    val same = sizes.all { abs(it.width - first.width) <= 1 && abs(it.height - first.height) <= 1 }
    return pageSizeText(first.width, first.height) + if (same) "" else "; other pages differ"
}
