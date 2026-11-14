package net.xfbs.pdfrs

import android.graphics.Bitmap
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import java.nio.ByteBuffer
import kotlin.system.measureNanoTime
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.pdfrs_ffi.PdfDocument

private const val TAG = "pdfrs"

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // Shown until the user picks a file.
        val sample = assets.open("sample.pdf").use { it.readBytes() }
        // adb shell am start -n net.xfbs.pdfrs/.MainActivity --ez bench true
        val autoBench = intent.getBooleanExtra("bench", false)
        setContent {
            MaterialTheme {
                SpikeScreen(sample, autoBench)
            }
        }
    }
}

private sealed interface Page {
    data object Loading : Page
    data class Ready(val bitmap: Bitmap, val stats: String) : Page
    data class Failed(val message: String) : Page
}

private fun ms(ns: Long) = "%.1f ms".format(ns / 1e6)

/** Device pixels per PDF point that makes page 1 `widthPx` wide. */
private fun PdfDocument.scaleForWidth(widthPx: Int): Float {
    val size = checkNotNull(pageSize(0u)) { "document has no pages" }
    return widthPx / size.width
}

private fun PdfDocument.newBitmap(scale: Float): Bitmap {
    val px = checkNotNull(pixelSize(0u, scale))
    return Bitmap.createBitmap(px.width.toInt(), px.height.toInt(), Bitmap.Config.ARGB_8888)
}

/** Path A: pixels come back through UniFFI as a ByteArray and are copied in. */
private fun PdfDocument.renderViaUniffi(scale: Float, bitmap: Bitmap) {
    val rendered = renderPage(0u, scale)
    bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(rendered.pixels))
}

/** Path B: Rust writes into the Bitmap's memory through JNI. */
private fun PdfDocument.renderViaJni(scale: Float, bitmap: Bitmap) {
    val status = NativeBitmap.renderInto(rawHandle().toLong(), 0, scale, bitmap)
    check(status == 0) { "renderInto failed with $status" }
}

/** Opens the PDF and renders page 1 at `widthPx` device pixels wide. */
private fun renderFirstPage(pdf: ByteArray, widthPx: Int): Page.Ready {
    val doc: PdfDocument
    val openNs = measureNanoTime { doc = PdfDocument.open(pdf, null) }
    doc.use {
        val scale = doc.scaleForWidth(widthPx)
        val bitmap = doc.newBitmap(scale)
        val renderNs = measureNanoTime { doc.renderViaJni(scale, bitmap) }
        return Page.Ready(
            bitmap,
            "${doc.pageCount()} page(s) · ${bitmap.width}×${bitmap.height} px\n" +
                "open ${ms(openNs)} · render ${ms(renderNs)} (cold)",
        )
    }
}

/** Warm comparison of the two pixel-transfer paths, interleaved. */
private fun benchmark(pdf: ByteArray, widthPx: Int, rounds: Int = 40): String {
    PdfDocument.open(pdf, null).use { doc ->
        val scale = doc.scaleForWidth(widthPx)
        val bitmap = doc.newBitmap(scale)
        repeat(5) {
            doc.renderViaUniffi(scale, bitmap)
            doc.renderViaJni(scale, bitmap)
        }
        val uniffi = LongArray(rounds)
        val jni = LongArray(rounds)
        for (i in 0 until rounds) {
            uniffi[i] = measureNanoTime { doc.renderViaUniffi(scale, bitmap) }
            jni[i] = measureNanoTime { doc.renderViaJni(scale, bitmap) }
        }
        fun LongArray.summary() = sorted().let { "median ${ms(it[size / 2])}, min ${ms(it.first())}" }
        return "Benchmark, ${bitmap.width}×${bitmap.height} px, $rounds rounds\n" +
            "UniFFI ByteArray + copy: ${uniffi.summary()}\n" +
            "JNI into Bitmap: ${jni.summary()}"
    }
}

@Composable
private fun SpikeScreen(sample: ByteArray, autoBench: Boolean) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var pdf by remember { mutableStateOf(sample) }
    var message by remember { mutableStateOf<String?>(null) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) {
            try {
                pdf = checkNotNull(context.contentResolver.openInputStream(uri)).use { it.readBytes() }
                message = null
            } catch (e: Exception) {
                message = e.toString()
            }
        }
    }

    Scaffold { insets ->
        BoxWithConstraints(Modifier.fillMaxSize().padding(insets)) {
            val widthPx = constraints.maxWidth
            val page by produceState<Page>(Page.Loading, pdf, widthPx) {
                value = Page.Loading
                value = withContext(Dispatchers.Default) {
                    try {
                        renderFirstPage(pdf, widthPx)
                    } catch (e: Exception) {
                        Page.Failed(e.toString())
                    }
                }
            }
            suspend fun runBenchmark() {
                message = "Benchmarking…"
                message = withContext(Dispatchers.Default) {
                    try {
                        benchmark(pdf, widthPx).also { Log.i(TAG, it) }
                    } catch (e: Exception) {
                        e.toString()
                    }
                }
            }
            if (autoBench) {
                LaunchedEffect(Unit) { runBenchmark() }
            }

            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Button(onClick = { picker.launch(arrayOf("application/pdf")) }) {
                        Text("Open PDF")
                    }
                    OutlinedButton(onClick = { scope.launch { runBenchmark() } }) {
                        Text("Benchmark")
                    }
                }
                when (val p = page) {
                    Page.Loading -> Text("Rendering…", Modifier.padding(horizontal = 16.dp))
                    is Page.Failed -> Text(p.message, Modifier.padding(horizontal = 16.dp))
                    is Page.Ready -> Text(p.stats, Modifier.padding(horizontal = 16.dp))
                }
                message?.let { Text(it, Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) }
                (page as? Page.Ready)?.let {
                    Image(
                        bitmap = it.bitmap.asImageBitmap(),
                        contentDescription = "Page 1",
                        contentScale = ContentScale.FillWidth,
                        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                    )
                }
            }
        }
    }
}
