package dev.vyoma.connect.ui

import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.util.Size
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.rounded.Screenshot
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.SmoothCornerShape
import dev.vyoma.connect.designsystem.CollapsingHeaderScaffold
import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.RowChevron
import dev.vyoma.connect.designsystem.StatusController
import dev.vyoma.connect.designsystem.fadingHorizontalScroll
import dev.vyoma.connect.designsystem.glassCard
import dev.vyoma.connect.designsystem.glassRaised
import dev.vyoma.connect.designsystem.isDarkUi
import dev.vyoma.connect.designsystem.pressable
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext

/** A screenshot or photo from the PC, saved in Pictures/Vyoma; [thumb] is loaded for the Home strip only. */
class CaptureItem(val uri: Uri, val thumb: ImageBitmap? = null)

/** The newest captures and how many there are, for Home and Files; refreshed off the main thread. */
class RecentCaptures(val items: List<CaptureItem>, val total: Int)

/**
 * Reads Vyoma's own captures (no media permission: only what Vyoma saved is visible, so nothing
 * else from the gallery can show here). Home asks for a refresh as soon as it opens, so the strip
 * is ready before you scroll to it (loading it lazily left the list ending short and then jumping).
 */
object CaptureStore {
    private val _recent = MutableStateFlow<RecentCaptures?>(null)
    val recent: StateFlow<RecentCaptures?> = _recent.asStateFlow()

    suspend fun refresh(context: Context) = withContext(Dispatchers.IO) {
        val all = runCatching { query(context, limit = 200) }.getOrDefault(emptyList())
        val resolver = context.contentResolver
        val shown = all.take(STRIP_COUNT).map { uri ->
            CaptureItem(uri, runCatching { resolver.loadThumbnail(uri, Size(480, 300), null).asImageBitmap() }.getOrNull())
        }
        _recent.value = RecentCaptures(shown, all.size)
    }

    suspend fun all(context: Context): List<Uri> = withContext(Dispatchers.IO) { runCatching { query(context, limit = 500) }.getOrDefault(emptyList()) }

    private fun query(context: Context, limit: Int): List<Uri> {
        val collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
        val out = mutableListOf<Uri>()
        context.contentResolver.query(
            collection,
            arrayOf(MediaStore.Images.Media._ID),
            "${MediaStore.Images.Media.RELATIVE_PATH} LIKE ?",
            arrayOf("${Environment.DIRECTORY_PICTURES}/Vyoma%"),
            "${MediaStore.Images.Media.DATE_ADDED} DESC",
        )?.use { cursor ->
            val idCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
            while (cursor.moveToNext() && out.size < limit) out += ContentUris.withAppendedId(collection, cursor.getLong(idCol))
        }
        return out
    }

    const val STRIP_COUNT = 5
}

/** Refreshes [CaptureStore] when the screen comes back and whenever [key] changes (a capture just landed). */
@Composable
fun RefreshCaptures(key: Any?) {
    val context = LocalContext.current
    var resumed by remember { mutableIntStateOf(0) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { resumed++ }
    LaunchedEffect(key, resumed) { CaptureStore.refresh(context) }
}

/**
 * The newest five captures as glass-framed thumbnails (tap to open), then "See all" when there are
 * more. Holds its height while loading so the page never changes length under the finger. Empty,
 * it says what will appear and offers [onTake].
 */
@Composable
fun CapturesStrip(onTake: (() -> Unit)?, onSeeAll: () -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val status = LocalStatus.current
    val recent by CaptureStore.recent.collectAsStateWithLifecycle()
    val r = recent
    if (r == null) {
        Spacer(modifier.fillMaxWidth().height(THUMB_H))
        return
    }
    if (r.items.isEmpty()) {
        Row(
            modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.large).padding(horizontal = 20.dp, vertical = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // The icon slot matches a list row's badge, so the text lines up with row titles elsewhere.
            Box(Modifier.size(34.dp), contentAlignment = Alignment.Center) {
                Icon(Icons.Rounded.Screenshot, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(28.dp))
            }
            Spacer(Modifier.width(16.dp))
            Text(
                "Screenshots and photos from the PC land here",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.weight(1f),
            )
            if (onTake != null) {
                Spacer(Modifier.width(12.dp))
                PillButton("Take one", prominent = false, onClick = onTake)
            }
        }
        return
    }
    Row(
        modifier.fillMaxWidth().fadingHorizontalScroll(rememberScrollState(), bleed = 16.dp).padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        r.items.forEach { c ->
            Thumb(c.thumb, Modifier.size(THUMB_W, THUMB_H)) { open(context, c.uri, status) }
        }
        if (r.total > r.items.size) {
            Column(
                Modifier
                    .size(96.dp, THUMB_H)
                    .semantics { contentDescription = "See all ${r.total} captures" }
                    .pressable(onClick = onSeeAll)
                    .glassRaised(SmoothCornerShape(18.dp)),
                verticalArrangement = Arrangement.Center,
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text("${r.total}", style = MaterialTheme.typography.titleLarge)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("See all", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
                    Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

/**
 * Home's compact entry to the captures: the newest thumbnail, a count and a chevron in one row
 * (the strip of five on Home added to its congestion; Files keeps the strip). Empty, it says what
 * lands here and offers [onTake].
 */
@Composable
fun CapturesRow(onTake: (() -> Unit)?, onOpen: () -> Unit, modifier: Modifier = Modifier) {
    val recent by CaptureStore.recent.collectAsStateWithLifecycle()
    val r = recent
    val any = r != null && r.total > 0
    Row(
        modifier
            .fillMaxWidth()
            .then(if (any) Modifier.pressable(onClick = onOpen) else Modifier)
            .glassCard(MaterialTheme.shapes.large)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val thumb = r?.items?.firstOrNull()?.thumb
        Box(
            // Plain like the Captures page's thumbnails: a photo needs no glass.
            Modifier.size(56.dp, 40.dp).clip(RoundedCornerShape(10.dp))
                .background(if (isDarkUi) Color.White.copy(alpha = 0.06f) else Color(0xFF1A2B4C).copy(alpha = 0.06f)),
            contentAlignment = Alignment.Center,
        ) {
            if (thumb != null) Image(thumb, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
            else Icon(Icons.Rounded.Screenshot, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(22.dp))
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text("Captures", style = MaterialTheme.typography.titleSmall)
            Text(
                when {
                    r == null -> " "
                    r.total == 0 -> "None from the PC yet"
                    r.total == 1 -> "1 from your PC"
                    else -> "${r.total} from your PC"
                },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        when {
            any -> RowChevron()
            onTake != null && r != null -> PillButton("Take one", prominent = false, onClick = onTake)
        }
    }
}

/** Every capture, two across, newest first; each thumbnail loads as its row comes into view. */
@Composable
fun CapturesScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val status = LocalStatus.current
    val all by produceState<List<Uri>?>(null) { value = CaptureStore.all(context) }
    CollapsingHeaderScaffold(
        title = "Captures",
        subtitle = all?.let { if (it.size == 1) "1 from your PC" else "${it.size} from your PC" } ?: "Loading…",
        navigation = { BounceIconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") } },
    ) {
        val list = all.orEmpty()
        list.chunked(2).forEachIndexed { i, row ->
            item(key = "row$i") {
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    row.forEach { uri ->
                        val thumb by produceState<ImageBitmap?>(null, uri) {
                            value = withContext(Dispatchers.IO) {
                                runCatching { context.contentResolver.loadThumbnail(uri, Size(480, 300), null).asImageBitmap() }.getOrNull()
                            }
                        }
                        Thumb(thumb, Modifier.weight(1f).aspectRatio(THUMB_W / THUMB_H)) { open(context, uri, status) }
                    }
                    if (row.size == 1) Spacer(Modifier.weight(1f))
                }
            }
        }
    }
}

/**
 * A photo, not a control: a plain rounded image with a still hairline so dark shots keep an edge on
 * black. It was framed in raised glass, whose tilt-lit rim redrew every thumbnail as the phone moved.
 */
@Composable
private fun Thumb(thumb: ImageBitmap?, modifier: Modifier, onClick: () -> Unit) {
    val dark = isDarkUi
    val shape = RoundedCornerShape(16.dp)
    Box(
        modifier
            .semantics { contentDescription = "Open capture" }
            .pressable(onClick = onClick)
            .clip(shape)
            // Shown until the thumbnail loads.
            .background(if (dark) Color.White.copy(alpha = 0.06f) else Color(0xFF1A2B4C).copy(alpha = 0.06f))
            .border(0.75.dp, if (dark) Color.White.copy(alpha = 0.1f) else Color.Black.copy(alpha = 0.08f), shape),
    ) {
        thumb?.let { Image(it, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop) }
    }
}

private fun open(context: Context, uri: Uri, status: StatusController) {
    val intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, "image/*").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    runCatching { context.startActivity(intent) }.onFailure { status.show("No app here opens pictures", failure = true) }
}

private val THUMB_W = 168.dp
private val THUMB_H = 104.dp
