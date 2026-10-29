package dev.vyoma.connect.ui

import android.graphics.Bitmap
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.VolumeOff
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.MusicNote
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.SkipPrevious
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.palette.graphics.Palette
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.platform.NowPlaying
import dev.vyoma.connect.protocol.plugin.builtin.AudioSink
import dev.vyoma.connect.protocol.plugin.builtin.RemoteCommand
import kotlinx.coroutines.delay

/** One UI-style square action: tinted icon over a soft container, squishes on press. */
@Composable
fun ActionTile(icon: ImageVector, label: String, modifier: Modifier = Modifier, active: Boolean = false, onClick: () -> Unit) {
    val bg = if (active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHigh
    val fg = if (active) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface
    Column(
        modifier
            .clip(MaterialTheme.shapes.large)
            .background(bg)
            .pressable(onClick = onClick)
            .padding(vertical = 14.dp, horizontal = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, null, tint = if (active) fg else MaterialTheme.colorScheme.primary, modifier = Modifier.size(24.dp))
        Spacer(Modifier.height(6.dp))
        Text(label, style = MaterialTheme.typography.labelMedium, color = fg, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** Rows of [ActionTile]s, [columns] per row, equal widths. */
@Composable
fun ActionGrid(actions: List<Triple<ImageVector, String, () -> Unit>>, columns: Int = 4, modifier: Modifier = Modifier) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(10.dp)) {
        actions.chunked(columns).forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                row.forEach { (icon, label, onClick) -> ActionTile(icon, label, Modifier.weight(1f), onClick = onClick) }
                repeat(columns - row.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

/**
 * Now-playing card: full-bleed artwork, a scrim tinted from the art itself, a live scrubber
 * and big controls. The artwork crossfades between tracks.
 */
@Composable
fun MediaCard(
    np: NowPlaying,
    onPlayPause: () -> Unit,
    onNext: () -> Unit,
    onPrevious: () -> Unit,
    onSeek: (Long) -> Unit,
    onVolume: (Int) -> Unit,
    onPickPlayer: (String) -> Unit,
    modifier: Modifier = Modifier,
    showVolume: Boolean = true,
) {
    val p = np.player
    val art = remember(np.art) { np.art?.let(::trimBars) }
    val tint = remember(art) {
        art?.let { bmp -> Palette.from(bmp).maximumColorCount(12).generate().let { it.getDarkMutedColor(it.getDominantColor(0xFF20242B.toInt())) } }
            ?.let { Color(it) } ?: Color(0xFF20242B)
    }
    Surface(modifier.fillMaxWidth(), shape = MaterialTheme.shapes.extraLarge, color = tint) {
        Column {
            Box(Modifier.fillMaxWidth().aspectRatio(16f / 10f)) {
                Crossfade(art, Modifier.fillMaxSize(), label = "art") { art ->
                    if (art != null) {
                        Image(art.asImageBitmap(), null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                    } else {
                        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                            Icon(Icons.Rounded.MusicNote, null, tint = Color.White.copy(alpha = 0.5f), modifier = Modifier.size(64.dp))
                        }
                    }
                }
                Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.45f to Color.Transparent, 1f to tint)))
                Column(Modifier.align(Alignment.BottomStart).padding(horizontal = 20.dp, vertical = 12.dp)) {
                    Text(p.title.ifBlank { p.name }, style = MaterialTheme.typography.titleLarge, color = Color.White, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    val sub = listOf(p.artist, p.album).filter { it.isNotBlank() }.joinToString(" · ")
                    if (sub.isNotBlank()) Text(sub, style = MaterialTheme.typography.bodyMedium, color = Color.White.copy(alpha = 0.75f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                if (p.lengthMs > 0) Scrubber(np, onSeek)
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
                    MediaButton(Icons.Rounded.SkipPrevious, "Previous", enabled = p.canPrevious, onClick = onPrevious)
                    val scale by animateFloatAsState(if (p.isPlaying) 1f else 1.06f, VyomaMotion.spatial(), label = "pp")
                    Box(
                        Modifier.size(68.dp).clip(CircleShape).background(Color.White).pressable(pressedScale = 0.9f, onClick = onPlayPause),
                        contentAlignment = Alignment.Center,
                    ) {
                        AnimatedContent(p.isPlaying, transitionSpec = { (scaleIn() + fadeIn()).togetherWith(scaleOut() + fadeOut()) }, label = "ppicon") { playing ->
                            Icon(if (playing) Icons.Rounded.Pause else Icons.Rounded.PlayArrow, if (playing) "Pause" else "Play", tint = tint, modifier = Modifier.size(36.dp * scale))
                        }
                    }
                    MediaButton(Icons.Rounded.SkipNext, "Next", enabled = p.canNext, onClick = onNext)
                }
                if (showVolume && p.volume >= 0) {
                    var vol by remember(p.name) { mutableFloatStateOf(p.volume.toFloat()) }
                    var dragging by remember { mutableStateOf(false) }
                    LaunchedEffect(p.volume) { if (!dragging) vol = p.volume.toFloat() }
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.AutoMirrored.Rounded.VolumeUp, null, tint = Color.White.copy(alpha = 0.7f), modifier = Modifier.size(18.dp))
                        Slider(
                            value = vol, valueRange = 0f..100f,
                            onValueChange = { dragging = true; vol = it; onVolume(it.toInt()) },
                            onValueChangeFinished = { dragging = false },
                            colors = SliderDefaults.colors(thumbColor = Color.White, activeTrackColor = Color.White, inactiveTrackColor = Color.White.copy(alpha = 0.25f)),
                            modifier = Modifier.padding(start = 8.dp).weight(1f),
                        )
                    }
                }
                if (np.players.size > 1) {
                    Row(Modifier.horizontalScroll(rememberScrollState()).padding(bottom = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        np.players.forEach { name ->
                            val selected = name == p.name
                            Text(
                                name,
                                style = MaterialTheme.typography.labelMedium,
                                color = if (selected) tint else Color.White,
                                modifier = Modifier.clip(CircleShape)
                                    .background(if (selected) Color.White else Color.White.copy(alpha = 0.14f))
                                    .pressable { onPickPlayer(name) }
                                    .padding(horizontal = 12.dp, vertical = 6.dp),
                            )
                        }
                    }
                } else {
                    Spacer(Modifier.height(8.dp))
                }
            }
        }
    }
}

@Composable
private fun MediaButton(icon: ImageVector, label: String, enabled: Boolean, onClick: () -> Unit) {
    IconButton(onClick = onClick, enabled = enabled, modifier = Modifier.size(56.dp)) {
        Icon(icon, label, tint = Color.White.copy(alpha = if (enabled) 1f else 0.35f), modifier = Modifier.size(34.dp))
    }
}

/** Ticks locally between PC updates so the bar moves smoothly without network chatter. */
@Composable
private fun Scrubber(np: NowPlaying, onSeek: (Long) -> Unit) {
    val p = np.player
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(p.isPlaying, p.positionAt) {
        while (p.isPlaying) {
            now = System.currentTimeMillis()
            delay(250)
        }
        now = System.currentTimeMillis()
    }
    var dragValue by remember { mutableStateOf<Float?>(null) }
    val position = dragValue ?: p.currentPosition(now).toFloat()
    Column {
        Slider(
            value = position.coerceIn(0f, p.lengthMs.toFloat()),
            valueRange = 0f..p.lengthMs.toFloat(),
            enabled = p.canSeek,
            onValueChange = { dragValue = it },
            onValueChangeFinished = { dragValue?.let { onSeek(it.toLong()) }; dragValue = null },
            colors = SliderDefaults.colors(
                thumbColor = Color.White, activeTrackColor = Color.White, inactiveTrackColor = Color.White.copy(alpha = 0.25f),
                disabledThumbColor = Color.White.copy(alpha = 0.6f), disabledActiveTrackColor = Color.White.copy(alpha = 0.6f),
            ),
        )
        Row(Modifier.fillMaxWidth()) {
            Text(formatTime(position.toLong()), style = MaterialTheme.typography.labelSmall, color = Color.White.copy(alpha = 0.7f))
            Spacer(Modifier.weight(1f))
            Text(formatTime(p.lengthMs), style = MaterialTheme.typography.labelSmall, color = Color.White.copy(alpha = 0.7f))
        }
    }
}

fun formatTime(ms: Long): String {
    val s = ms / 1000
    return if (s >= 3600) "%d:%02d:%02d".format(s / 3600, s / 60 % 60, s % 60) else "%d:%02d".format(s / 60, s % 60)
}

/** PC audio outputs: a slider each, tap the icon to mute, tap the name to make it the default. */
@Composable
fun VolumeCard(sinks: List<AudioSink>, onVolume: (AudioSink, Int) -> Unit, onMute: (AudioSink) -> Unit, onDefault: (AudioSink) -> Unit) {
    SectionCard(contentPadding = PaddingValues(vertical = 8.dp)) {
        sinks.sortedByDescending { it.isDefault }.forEach { sink ->
            var value by remember(sink.name) { mutableFloatStateOf(sink.volume.toFloat()) }
            LaunchedEffect(sink.volume) { value = sink.volume.toFloat() }
            Column(Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.pressable(pressedScale = 0.99f) { onDefault(sink) }) {
                    Text(
                        prettySinkName(sink.description),
                        style = MaterialTheme.typography.bodyMedium,
                        color = if (sink.isDefault) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
                    )
                    if (sink.isDefault) Icon(Icons.Rounded.CheckCircle, "Default", tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(18.dp))
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    IconButton(onClick = { onMute(sink) }) {
                        Icon(
                            if (sink.muted) Icons.AutoMirrored.Rounded.VolumeOff else Icons.AutoMirrored.Rounded.VolumeUp,
                            if (sink.muted) "Unmute" else "Mute",
                            tint = if (sink.muted) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
                        )
                    }
                    Slider(
                        value = value, valueRange = 0f..sink.maxVolume.toFloat(),
                        onValueChange = { value = it; onVolume(sink, it.toInt()) },
                        modifier = Modifier.weight(1f),
                    )
                    Text("${(value * 100 / sink.maxVolume).toInt()}%", style = MaterialTheme.typography.labelMedium, modifier = Modifier.width(44.dp).padding(start = 8.dp))
                }
            }
        }
    }
}

/** PulseAudio descriptions are long ("500 Series Chipset ... Headphones"); keep the useful tail. */
fun prettySinkName(description: String): String {
    val parts = description.split(" (HD Audio) ", " Audio ").map { it.trim() }
    return parts.last().ifBlank { description }
}

@Composable
fun CommandsCard(commands: List<RemoteCommand>, onRun: (RemoteCommand) -> Unit) {
    SectionCard {
        commands.forEachIndexed { i, cmd ->
            ListRow(
                title = cmd.name,
                subtitle = cmd.command,
                icon = Icons.Rounded.Terminal,
                showDivider = i < commands.lastIndex,
                onClick = { onRun(cmd) },
            )
        }
    }
}

/**
 * Video thumbnails often come letter- or pillarboxed. Crops near-black bars off the edges so the
 * art can fill the card; leaves genuinely dark artwork alone by sampling across each line.
 */
private fun trimBars(bmp: Bitmap): Bitmap {
    val w = bmp.width
    val h = bmp.height
    if (w < 32 || h < 32) return bmp
    fun dark(c: Int) = (c shr 16 and 0xFF) + (c shr 8 and 0xFF) + (c and 0xFF) < 54
    fun darkColumn(x: Int) = (0 until 16).all { dark(bmp.getPixel(x, h * (2 * it + 1) / 32)) }
    fun darkRow(y: Int) = (0 until 16).all { dark(bmp.getPixel(w * (2 * it + 1) / 32, y)) }
    var l = 0; while (l < w / 3 && darkColumn(l)) l++
    var r = w - 1; while (r > w * 2 / 3 && darkColumn(r)) r--
    var t = 0; while (t < h / 3 && darkRow(t)) t++
    var b = h - 1; while (b > h * 2 / 3 && darkRow(b)) b--
    if (l == 0 && t == 0 && r == w - 1 && b == h - 1) return bmp
    // A whole dark third means a dark image, not bars.
    if (l >= w / 3 || t >= h / 3) return bmp
    return Bitmap.createBitmap(bmp, l, t, r - l + 1, b - t + 1)
}
