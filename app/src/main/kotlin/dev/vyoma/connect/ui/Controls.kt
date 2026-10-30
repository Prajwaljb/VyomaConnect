package dev.vyoma.connect.ui

import kotlinx.coroutines.launch
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.LiquidSlider
import dev.vyoma.connect.designsystem.backdropSource
import dev.vyoma.connect.designsystem.glassChrome
import dev.vyoma.connect.designsystem.pressScale
import dev.vyoma.connect.designsystem.rememberBackdrop
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.specularEdge
import dev.vyoma.connect.designsystem.toned
import dev.vyoma.connect.protocol.plugin.builtin.RemotePlayer
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.rounded.Movie
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
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.MusicNote
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.SkipPrevious
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material3.Icon
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
import dev.vyoma.connect.protocol.plugin.builtin.RemoteCommand
import kotlin.math.abs
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
 * and big controls. The artwork crossfades between tracks. Without artwork (mpv, local files, art
 * that didn't load) it's a compact card instead, like Apple Music's: a deep accent tile with the
 * player's glyph beside the title, on a deep shade of the accent, rather than a big empty frame.
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
    /** Shown as "Continue on phone" over the artwork when the PC can hand this off (companion). */
    onContinueHere: (() -> Unit)? = null,
) {
    val p = np.player
    val art = remember(np.art) { np.art?.let(::trimBars) }
    val accent = MaterialTheme.colorScheme.primary
    val tint = remember(art, accent) {
        art?.let { bmp -> Palette.from(bmp).maximumColorCount(12).generate().let { it.getDarkMutedColor(it.getDominantColor(0xFF20242B.toInt())) } }
            ?.let { Color(it) } ?: accent.toned(0.8f, 0.2f)
    }
    val tintAnimated by animateColorAsState(tint, VyomaMotion.effects(), label = "tint")
    Surface(modifier.fillMaxWidth(), shape = MaterialTheme.shapes.extraLarge, color = tintAnimated) {
        Column(Modifier.animateContentSize(VyomaMotion.spatial())) {
            if (art == null) {
                PlainHeader(p, accent, onContinueHere)
            } else Box(Modifier.fillMaxWidth().aspectRatio(16f / 10f)) {
                val behind = rememberBackdrop()
                Crossfade(art, Modifier.fillMaxSize().backdropSource(behind), label = "art") { art ->
                    Image(art.asImageBitmap(), null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                }
                Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.45f to Color.Transparent, 1f to tint)))
                if (onContinueHere != null) {
                    // Liquid glass over the artwork, smoked so white text reads on any thumbnail.
                    Row(
                        Modifier
                            .align(Alignment.TopEnd)
                            .padding(12.dp)
                            .pressScale(0.92f)
                            .glassChrome(behind, CircleShape, blur = 8.dp, tint = Color.Black.copy(alpha = 0.95f), bevel = 16.dp)
                            .clip(CircleShape)
                            .pressable(pressedScale = 1f, onClick = onContinueHere)
                            .padding(start = 10.dp, end = 14.dp, top = 8.dp, bottom = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(Icons.Rounded.PhoneAndroid, null, tint = Color.White, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Continue on phone", style = MaterialTheme.typography.labelLarge, color = Color.White)
                    }
                }
                Column(Modifier.align(Alignment.BottomStart).padding(horizontal = 20.dp, vertical = 12.dp)) {
                    Text(p.title.ifBlank { p.name }, style = MaterialTheme.typography.titleLarge, color = Color.White, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    val sub = listOf(p.artist, p.album).filter { it.isNotBlank() }.joinToString(" · ")
                    if (sub.isNotBlank()) Text(sub, style = MaterialTheme.typography.bodyMedium, color = Color.White.copy(alpha = 0.75f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
                // Browsers blank the length for a moment now and then (ads, hover previews): keep the track's last one.
                val length = remember(p.name, p.title) { mutableLongStateOf(0L) }
                if (p.lengthMs > 0 && length.longValue != p.lengthMs) length.longValue = p.lengthMs
                if (length.longValue > 0) Scrubber(np, length.longValue, tint, onSeek)
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
                    MediaButton(Icons.Rounded.SkipPrevious, "Previous", enabled = p.canPrevious, onClick = onPrevious)
                    val scale by animateFloatAsState(if (p.isPlaying) 1f else 1.06f, VyomaMotion.spatial(), label = "pp")
                    Box(
                        Modifier.size(68.dp).pressable(pressedScale = 0.9f, glass = true, onClick = onPlayPause).clip(CircleShape).background(Color.White),
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
                        LiquidSlider(
                            value = { vol / 100f },
                            onChange = { dragging = true; vol = it * 100f; onVolume(vol.toInt()) },
                            onDone = { dragging = false },
                            modifier = Modifier.padding(start = 10.dp).weight(1f),
                            underlay = tint,
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

/** The no-artwork top of [MediaCard]: glyph tile, title, artist, and which player it is. */
@Composable
private fun PlainHeader(p: RemotePlayer, accent: Color, onContinueHere: (() -> Unit)?) {
    val video = VIDEO_PLAYERS.any { p.name.contains(it, ignoreCase = true) }
    Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            val tile = RoundedCornerShape(18.dp)
            Box(
                Modifier
                    .size(72.dp)
                    .specularEdge(tile, 1.dp)
                    .clip(tile)
                    .background(Brush.linearGradient(listOf(accent.toned(1.2f, 0.62f), accent.toned(1.2f, 0.46f)))),
                contentAlignment = Alignment.Center,
            ) {
                Icon(if (video) Icons.Rounded.Movie else Icons.Rounded.MusicNote, null, tint = Color.White, modifier = Modifier.size(36.dp))
            }
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(p.title.ifBlank { p.name }, style = MaterialTheme.typography.titleMedium, color = Color.White, maxLines = 2, overflow = TextOverflow.Ellipsis)
                val sub = listOf(p.artist, p.album).filter { it.isNotBlank() }.joinToString(" · ")
                if (sub.isNotBlank()) Text(sub, style = MaterialTheme.typography.bodyMedium, color = Color.White.copy(alpha = 0.75f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (p.title.isNotBlank()) Text(p.name, style = MaterialTheme.typography.labelMedium, color = Color.White.copy(alpha = 0.55f), maxLines = 1)
            }
        }
        if (onContinueHere != null) {
            Row(
                Modifier
                    .padding(top = 12.dp)
                    .clip(CircleShape)
                    .background(Color.White.copy(alpha = 0.14f))
                    .pressable(onClick = onContinueHere)
                    .padding(start = 10.dp, end = 14.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Rounded.PhoneAndroid, null, tint = Color.White, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Continue on phone", style = MaterialTheme.typography.labelLarge, color = Color.White)
            }
        }
    }
}

/** Players that are mostly for video, so their card shows a film glyph rather than a note. */
private val VIDEO_PLAYERS = listOf("mpv", "vlc", "celluloid", "totem", "haruna", "smplayer", "mplayer", "kodi", "showtime", "dragon")

@Composable
private fun MediaButton(icon: ImageVector, label: String, enabled: Boolean, onClick: () -> Unit) {
    BounceIconButton(onClick = onClick, enabled = enabled, size = 56.dp) {
        Icon(icon, label, tint = Color.White.copy(alpha = if (enabled) 1f else 0.35f), modifier = Modifier.size(34.dp))
    }
}

/**
 * Apple Music's scrubber: a bare track that thickens under the finger, no knob. One animated
 * progress value drives it: it ticks locally between PC updates (on whole seconds of playback,
 * only while the app is on screen), small drifts are absorbed in place and real jumps (a seek from
 * the PC, a new position report) glide there instead of teleporting. After our own seek it holds
 * the seek target until the PC's position agrees: browsers report a stale, often zero, position
 * for a while after a seek, which used to fling the bar back to the start.
 */
@Composable
private fun Scrubber(np: NowPlaying, lengthMs: Long, underlay: Color, onSeek: (Long) -> Unit) {
    val player by rememberUpdatedState(np.player)
    val length by rememberUpdatedState(lengthMs)
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val p = np.player
    LaunchedEffect(p.isPlaying, p.positionAt) {
        now = System.currentTimeMillis()
        if (!p.isPlaying) return@LaunchedEffect
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                now = System.currentTimeMillis()
                // On the next whole second of playback: the labels change then, and the bar moves
                // less than a pixel in between, so ticking faster only costs redraws.
                delay((1000 - p.currentPosition(now) % 1000).coerceAtLeast(16))
            }
        }
    }
    val scope = rememberCoroutineScope()
    var dragValue by remember { mutableStateOf<Float?>(null) }
    // Our last seek: target and when.
    var seek by remember { mutableStateOf<Pair<Long, Long>?>(null) }
    fun positionAt(at: Long): Long {
        val reported = player.currentPosition(at).coerceIn(0, length)
        val (target, t0) = seek ?: return reported
        val expected = (target + if (player.isPlaying) at - t0 else 0).coerceIn(0, length)
        return if (at - t0 > SEEK_SETTLE_MS || abs(reported - expected) < SEEK_AGREE_MS) reported else expected
    }
    val progress = remember { Animatable((positionAt(now) / lengthMs.toFloat()).coerceIn(0f, 1f)) }
    LaunchedEffect(Unit) {
        snapshotFlow { if (dragValue != null) null else positionAt(now) }.collect { ms ->
            if (ms == null) return@collect
            val t = (ms / length.toFloat()).coerceIn(0f, 1f)
            if (abs(t - progress.value) * length > JUMP_MS) launch { progress.animateTo(t, VyomaMotion.page()) } else progress.snapTo(t)
        }
    }
    val shownMs = dragValue?.let { (it * lengthMs).toLong() } ?: positionAt(now)
    Column {
        LiquidSlider(
            value = { dragValue ?: progress.value },
            onChange = { dragValue = it },
            onDone = {
                dragValue?.let {
                    val target = (it * lengthMs).toLong()
                    seek = target to System.currentTimeMillis()
                    scope.launch { progress.snapTo(it) }
                    onSeek(target)
                }
                dragValue = null
            },
            enabled = p.canSeek,
            knob = null,
            fill = Color.White.copy(alpha = if (p.canSeek) 1f else 0.6f),
            underlay = underlay,
        )
        Row(Modifier.fillMaxWidth()) {
            Text(formatTime(shownMs), style = MaterialTheme.typography.labelSmall, color = Color.White.copy(alpha = 0.7f))
            Spacer(Modifier.weight(1f))
            Text("-" + formatTime((lengthMs - shownMs).coerceAtLeast(0)), style = MaterialTheme.typography.labelSmall, color = Color.White.copy(alpha = 0.7f))
        }
    }
}

private const val JUMP_MS = 1_500L
private const val SEEK_SETTLE_MS = 5_000L
private const val SEEK_AGREE_MS = 2_500L

fun formatTime(ms: Long): String {
    val s = ms / 1000
    return if (s >= 3600) "%d:%02d:%02d".format(s / 3600, s / 60 % 60, s % 60) else "%d:%02d".format(s / 60, s % 60)
}

/** PulseAudio descriptions are long ("(HD Audio) HDMI / DisplayPort 3 Output (Stereo)"); keep the useful part. */
fun prettySinkName(description: String): String {
    // HDMI outputs carry the monitor's name in brackets; that's what people recognise.
    Regex("""\[([^]]+)]""").find(description)?.groupValues?.get(1)?.trim()?.takeIf { it.isNotEmpty() }?.let { return it }
    val name = description
        .replace(Regex("""\((HD Audio|Stereo|Mono|Analog Stereo|Digital Stereo[^)]*)\)"""), "")
        .replace(Regex("""\[[^]]*]"""), "")
        .substringAfterLast(" Audio ")
        .replace(" Output", "")
        .trim()
    return name.ifBlank { description }
}

@Composable
fun CommandsCard(commands: List<RemoteCommand>, onRun: (RemoteCommand) -> Unit) {
    SectionCard {
        commands.forEachIndexed { i, cmd ->
            ListRow(
                title = cmd.name,
                subtitle = cmd.command,
                icon = Icons.Rounded.Terminal,
                iconTint = SystemColor.GRAY.color,
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
    // Bars are rarely pure black after compression; anything this dark and flat counts.
    fun dark(c: Int) = (c shr 16 and 0xFF) + (c shr 8 and 0xFF) + (c and 0xFF) < 96
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
