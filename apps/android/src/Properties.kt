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
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import java.text.DecimalFormat
import java.text.DecimalFormatSymbols
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
        val texts = context.resources.texts()
        propertyRows(
            name = name,
            info = state.info(),
            pageCount = state.pageCount,
            pageSize = pageSizesText(state.pageSizes, texts),
            fileSize = Formatter.formatShortFileSize(context, bytes),
            texts = texts,
            date = ::documentDate,
        )
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.menu_properties)) },
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
        confirmButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.close)) } },
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
    texts: Texts,
    date: (String) -> String,
): List<Pair<String, String>> = listOfNotNull(
    R.string.property_file_name to name,
    info.title?.let { R.string.property_title to it },
    info.author?.let { R.string.property_author to it },
    info.subject?.let { R.string.property_subject to it },
    info.keywords?.let { R.string.property_keywords to it },
    info.creator?.let { R.string.property_creator to it },
    info.producer?.let { R.string.property_producer to it },
    info.created?.let { R.string.property_created to date(it) },
    info.modified?.let { R.string.property_modified to date(it) },
    R.string.property_pages to pageCount.toString(),
    pageSize?.let { R.string.property_page_size to it },
    R.string.property_file_size to fileSize,
    R.string.property_pdf_version to info.pdfVersion,
).map { (label, value) -> texts.get(label) to value }

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
internal fun pageSizeText(
    width: Float,
    height: Float,
    texts: Texts,
    locale: Locale = Locale.getDefault(),
): String {
    val (short, long) = if (width <= height) width to height else height to width
    val paper = PAPER_SIZES.firstOrNull {
        abs(it.short - short) <= PAPER_TOLERANCE && abs(it.long - long) <= PAPER_TOLERANCE
    }
    val size = if (paper?.inches == true) {
        // "8.5", or "8,5" where that is how it is written.
        val number = DecimalFormat("0.##", DecimalFormatSymbols.getInstance(locale))
        fun inches(points: Float) = number.format(points / 72.0)
        texts.get(R.string.page_size_in, inches(width), inches(height))
    } else {
        fun millimetres(points: Float) = (points / 72 * 25.4f).roundToInt()
        texts.get(R.string.page_size_mm, millimetres(width), millimetres(height))
    }
    val landscape = width > height
    return when {
        paper != null && landscape -> texts.get(R.string.page_size_named_landscape, size, paper.name)
        paper != null -> texts.get(R.string.page_size_named, size, paper.name)
        else -> size
    }
}

/** The size of a document's pages: that of the first, with a note if others differ. */
internal fun pageSizesText(
    sizes: List<PageSize>,
    texts: Texts,
    locale: Locale = Locale.getDefault(),
): String? {
    val first = sizes.firstOrNull() ?: return null
    val same = sizes.all { abs(it.width - first.width) <= 1 && abs(it.height - first.height) <= 1 }
    val size = pageSizeText(first.width, first.height, texts, locale)
    return if (same) size else texts.get(R.string.page_size_others_differ, size)
}
