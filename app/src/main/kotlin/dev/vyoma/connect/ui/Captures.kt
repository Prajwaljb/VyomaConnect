package dev.vyoma.connect.ui

import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.util.Size
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Screenshot
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.PillButton
import dev.vyoma.connect.designsystem.fadingHorizontalScroll
import dev.vyoma.connect.designsystem.glassCard
import dev.vyoma.connect.designsystem.glassRaised
import dev.vyoma.connect.designsystem.pressable
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** A screenshot or photo from the PC, saved in Pictures/Vyoma. */
private class Capture(val uri: Uri, val thumb: ImageBitmap?)

/**
 * The PC's screenshots and camera photos, newest first, as a strip of glass-framed thumbnails
 * (tap to open). Only Vyoma's own saves are visible to it (no media permission), so nothing else
 * from the gallery can show here. Re-read whenever the screen comes back and when [refresh] changes
 * (a capture just landed). Empty, it says what will appear and offers [onTake].
 */
@Composable
fun CapturesStrip(refresh: Any?, onTake: (() -> Unit)?, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val status = LocalStatus.current
    var resumed by remember { mutableIntStateOf(0) }
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { resumed++ }
    val captures by produceState<List<Capture>?>(null, refresh, resumed) {
        value = withContext(Dispatchers.IO) { runCatching { loadCaptures(context) }.getOrDefault(emptyList()) }
    }
    val list = captures ?: return
    if (list.isEmpty()) {
        Row(
            modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.large).padding(horizontal = 20.dp, vertical = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Rounded.Screenshot, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(28.dp))
            Spacer(Modifier.width(14.dp))
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
        list.forEach { c ->
            Box(
                Modifier
                    .size(THUMB_W, THUMB_H)
                    .semantics { contentDescription = "Open capture" }
                    .pressable {
                        val open = Intent(Intent.ACTION_VIEW).setDataAndType(c.uri, "image/*").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        runCatching { context.startActivity(open) }.onFailure { status.show("No app here opens pictures", failure = true) }
                    }
                    .glassRaised(RoundedCornerShape(18.dp))
                    .padding(3.dp)
                    .clip(RoundedCornerShape(15.dp)),
            ) {
                c.thumb?.let { Image(it, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop) }
            }
        }
    }
}

private val THUMB_W = 168.dp
private val THUMB_H = 104.dp

private fun loadCaptures(context: Context): List<Capture> {
    val resolver = context.contentResolver
    val collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
    val out = mutableListOf<Capture>()
    resolver.query(
        collection,
        arrayOf(MediaStore.Images.Media._ID),
        "${MediaStore.Images.Media.RELATIVE_PATH} LIKE ?",
        arrayOf("${Environment.DIRECTORY_PICTURES}/Vyoma%"),
        "${MediaStore.Images.Media.DATE_ADDED} DESC",
    )?.use { cursor ->
        val idCol = cursor.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
        while (cursor.moveToNext() && out.size < MAX_CAPTURES) {
            val uri = ContentUris.withAppendedId(collection, cursor.getLong(idCol))
            val thumb = runCatching { resolver.loadThumbnail(uri, Size(480, 300), null).asImageBitmap() }.getOrNull()
            out += Capture(uri, thumb)
        }
    }
    return out
}

private const val MAX_CAPTURES = 12
