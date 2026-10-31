package dev.vyoma.connect.ui

import androidx.compose.ui.graphics.luminance
import dev.vyoma.connect.designsystem.DialogButton
import dev.vyoma.connect.designsystem.RowChevron
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.platform.LocalHapticFeedback
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.designsystem.GlassDialog
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.UnfoldMore
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.glassRaised
import androidx.compose.ui.text.font.FontFamily
import dev.vyoma.connect.designsystem.fadingHorizontalScroll
import androidx.compose.material.icons.rounded.Edit
import kotlinx.coroutines.launch
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.heightIn
import dev.vyoma.connect.designsystem.glassCard
import androidx.compose.ui.unit.Dp
import dev.vyoma.connect.designsystem.glassLightOver
import androidx.compose.material.icons.automirrored.rounded.SendToMobile
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import androidx.compose.runtime.produceState
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.LiquidSlider
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.specularEdge
import dev.vyoma.connect.designsystem.toned
import dev.vyoma.connect.protocol.plugin.builtin.RemotePlayer
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.material.icons.rounded.Movie
import android.graphics.Bitmap
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.Crossfade
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
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.MusicNote
import androidx.compose.material.icons.rounded.Pause
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.SkipNext
import androidx.compose.material.icons.rounded.SkipPrevious
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
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
import kotlin.math.abs
import kotlinx.coroutines.delay

/** One UI-style square action: tinted icon over a soft container, squishes on press. */
@Composable
fun ActionTile(icon: ImageVector, label: String, modifier: Modifier = Modifier, active: Boolean = false, onClick: () -> Unit) {
    val bg = if (active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHigh
    val fg = if (active) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface
    Column(
        modifier
            .pressable(onClick = onClick)
            .clip(MaterialTheme.shapes.large)
            .background(bg)
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
 * Now-playing card, compact like an iOS widget: the artwork fills the left side edge to edge and
 * melts into a deep colour sampled from it; the title, a thin scrubber and the controls sit on the
 * right, with Continue on phone as a round button beside the title. The card is the same lit glass
 * as the others (tilt-lit rim, lit-side glow). A full-bleed artwork card was felt to be too big, and
 * a thumbnail in a plain card too plain.
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
    /** Shown as a Continue on phone button when the PC can hand this off (companion). */
    onContinueHere: (() -> Unit)? = null,
    /** Narrower beside Home's volume capsule. */
    artWidth: Dp = ART_WIDTH,
) {
    val p = np.player
    val accent = MaterialTheme.colorScheme.primary
    // Trimming the bars and sampling the palette scan the whole bitmap: off the main thread, once
    // per picture. Until it's ready the previous picture stays.
    val prepared by produceState<Pair<Bitmap, Color>?>(null, np.art) {
        val source = np.art
        value = if (source == null) null else withContext(Dispatchers.Default) {
            val trimmed = trimBars(source)
            val swatch = Palette.from(trimmed).maximumColorCount(12).generate().let { it.getDarkMutedColor(it.getDominantColor(0xFF20242B.toInt())) }
            // Kept deep whatever the art: a mid-tone tint (bright art with no dark swatch) left the
            // white secondary text at 2.5:1 and put a loud slab of colour on Home.
            trimmed to Color(swatch).let { if (it.luminance() > 0.06f) it.toned(lightness = 0.2f) else it }
        }
    }
    val art = prepared?.first
    val tint by animateColorAsState(prepared?.second ?: accent.toned(0.8f, 0.2f), VyomaMotion.effects(), label = "tint")
    val shape = MaterialTheme.shapes.large
    Box(
        modifier
            .fillMaxWidth()
            // The cards' glass, its light and tilt-lit rim drawn over everything (the artwork
            // included) so the edge reads as one pane; drawn under, it stopped dead at the art.
            .specularEdge(shape, 1.dp, Color.White.copy(alpha = 0.4f), Color.White.copy(alpha = 0.07f))
            .glassLightOver(shape)
            .glassCard(shape, fill = tint, lit = false)
            .animateContentSize(VyomaMotion.spatial()),
    ) {
        // The artwork fills the card's left side, top to bottom, melting into the card's colour;
        // everything else (volume, player chips too) sits in the column beside it.
        Box(Modifier.matchParentSize()) {
            ArtPanel(art, p, accent, tint, Modifier.width(artWidth).fillMaxHeight())
        }
        Column(Modifier.fillMaxWidth().padding(start = artWidth)) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = artWidth)
                    .padding(start = 12.dp, end = 10.dp, top = 12.dp, bottom = 4.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
                Row(verticalAlignment = Alignment.Top) {
                    Column(Modifier.weight(1f).padding(top = 2.dp)) {
                        Text(p.title.ifBlank { p.name }, style = MaterialTheme.typography.titleMedium, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        val sub = listOf(p.artist, p.album).filter { it.isNotBlank() }.joinToString(" · ").ifBlank { p.name.takeIf { p.title.isNotBlank() }.orEmpty() }
                        if (sub.isNotBlank()) Text(sub, style = MaterialTheme.typography.bodyMedium, color = Color.White.copy(alpha = 0.8f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    // Switching players is rare: a small button and a list, not a row of chips on the card.
                    if (np.players.size > 1) {
                        var picking by remember { mutableStateOf(false) }
                        Spacer(Modifier.width(6.dp))
                        Box(
                            Modifier
                                .size(36.dp)
                                .pressable(glass = true) { picking = true }
                                .glassRaised(CircleShape, Color.White.copy(alpha = 0.14f))
                                .semantics { contentDescription = "Choose player" },
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(Icons.Rounded.UnfoldMore, null, tint = Color.White, modifier = Modifier.size(20.dp))
                        }
                        if (picking) PlayerPicker(np.players, p.name, onDismiss = { picking = false }) { onPickPlayer(it); picking = false }
                    }
                    if (onContinueHere != null) {
                        Spacer(Modifier.width(6.dp))
                        Box(
                            Modifier
                                .size(36.dp)
                                .pressable(glass = true, onClick = onContinueHere)
                                .glassRaised(CircleShape, Color.White.copy(alpha = 0.14f))
                                .semantics { contentDescription = "Continue on phone" },
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(Icons.AutoMirrored.Rounded.SendToMobile, null, tint = Color.White, modifier = Modifier.size(18.dp))
                        }
                    }
                }
                // Browsers blank the length for a moment now and then (ads, hover previews): keep the track's last one.
                val length = remember(p.name, p.title) { mutableLongStateOf(0L) }
                if (p.lengthMs > 0 && length.longValue != p.lengthMs) length.longValue = p.lengthMs
                if (length.longValue > 0) Scrubber(np, length.longValue, tint, onSeek)
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
                    MediaButton(Icons.Rounded.SkipPrevious, "Previous", enabled = p.canPrevious, onClick = onPrevious)
                    BounceIconButton(onClick = onPlayPause, size = 52.dp) {
                        AnimatedContent(p.isPlaying, transitionSpec = { (scaleIn() + fadeIn()).togetherWith(scaleOut() + fadeOut()) }, label = "ppicon") { playing ->
                            Icon(if (playing) Icons.Rounded.Pause else Icons.Rounded.PlayArrow, if (playing) "Pause" else "Play", tint = Color.White, modifier = Modifier.size(40.dp))
                        }
                    }
                    MediaButton(Icons.Rounded.SkipNext, "Next", enabled = p.canNext, onClick = onNext)
                }
            }
            if (showVolume && p.volume >= 0) {
                var vol by remember(p.name) { mutableFloatStateOf(p.volume.toFloat()) }
                var dragging by remember { mutableStateOf(false) }
                LaunchedEffect(p.volume) { if (!dragging) vol = p.volume.toFloat() }
                Row(Modifier.padding(start = 12.dp, end = 16.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
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
        }
    }
}

/**
 * The card's left side: the artwork filling it edge to edge, its right edge fading into the card's
 * [tint]; without art, a deep accent panel with the player's glyph.
 */
@Composable
private fun ArtPanel(art: Bitmap?, p: RemotePlayer, accent: Color, tint: Color, modifier: Modifier) {
    Box(modifier.background(Brush.linearGradient(listOf(accent.toned(1.2f, 0.58f), accent.toned(1.2f, 0.42f))))) {
        Crossfade(art, Modifier.fillMaxSize(), label = "art") { a ->
            if (a != null) {
                Image(a.asImageBitmap(), null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            } else {
                // Centred in the clear part, ahead of the melt.
                Box(Modifier.fillMaxSize().padding(end = 28.dp), contentAlignment = Alignment.Center) {
                    val video = VIDEO_PLAYERS.any { p.name.contains(it, ignoreCase = true) }
                    Icon(if (video) Icons.Rounded.Movie else Icons.Rounded.MusicNote, null, tint = Color.White, modifier = Modifier.size(40.dp))
                }
            }
        }
        // A long, eased melt: a short one read as a seam.
        Box(Modifier.fillMaxSize().background(Brush.horizontalGradient(0.35f to Color.Transparent, 0.7f to tint.copy(alpha = 0.55f), 1f to tint)))
    }
}

private val ART_WIDTH = 132.dp


/** Players that are mostly for video, so their tile shows a film glyph rather than a note. */
private val VIDEO_PLAYERS = listOf("mpv", "vlc", "celluloid", "totem", "haruna", "smplayer", "mplayer", "kodi", "showtime", "dragon")

@Composable
private fun MediaButton(icon: ImageVector, label: String, enabled: Boolean, onClick: () -> Unit) {
    BounceIconButton(onClick = onClick, enabled = enabled, size = 48.dp) {
        Icon(icon, label, tint = Color.White.copy(alpha = if (enabled) 1f else 0.35f), modifier = Modifier.size(32.dp))
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
            Text(formatTime(shownMs), style = MaterialTheme.typography.labelSmall, color = Color.White.copy(alpha = 0.8f))
            Spacer(Modifier.weight(1f))
            Text("-" + formatTime((lengthMs - shownMs).coerceAtLeast(0)), style = MaterialTheme.typography.labelSmall, color = Color.White.copy(alpha = 0.8f))
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


/**
 * Video thumbnails often come letter- or pillarboxed (black bars, or a solid colour beside square
 * album art). Crops flat bars off the edges so the art can fill the card; leaves genuinely flat
 * artwork alone by sampling across each line.
 */
private fun trimBars(bmp: Bitmap): Bitmap {
    val w = bmp.width
    val h = bmp.height
    if (w < 32 || h < 32) return bmp
    // A bar is flat: every sample across the line matches the colour at that edge (black for
    // letterboxing, or the solid colour YouTube puts beside square album art). Compression noise
    // allowed.
    fun near(a: Int, b: Int) =
        abs((a shr 16 and 0xFF) - (b shr 16 and 0xFF)) + abs((a shr 8 and 0xFF) - (b shr 8 and 0xFF)) + abs((a and 0xFF) - (b and 0xFF)) < 36
    fun flatColumn(x: Int, edge: Int) = (0 until 16).all { near(bmp.getPixel(x, h * (2 * it + 1) / 32), edge) }
    fun flatRow(y: Int, edge: Int) = (0 until 16).all { near(bmp.getPixel(w * (2 * it + 1) / 32, y), edge) }
    val left = bmp.getPixel(0, h / 2)
    val right = bmp.getPixel(w - 1, h / 2)
    val top = bmp.getPixel(w / 2, 0)
    val bottom = bmp.getPixel(w / 2, h - 1)
    var l = 0; while (l < w / 3 && flatColumn(l, left)) l++
    var r = w - 1; while (r > w * 2 / 3 && flatColumn(r, right)) r--
    var t = 0; while (t < h / 3 && flatRow(t, top)) t++
    var b = h - 1; while (b > h * 2 / 3 && flatRow(b, bottom)) b--
    if (l == 0 && t == 0 && r == w - 1 && b == h - 1) return bmp
    // A whole flat third means a flat image (a dark scene, a plain cover), not bars.
    if (l >= w / 3 || t >= h / 3) return bmp
    return Bitmap.createBitmap(bmp, l, t, r - l + 1, b - t + 1)
}

/** The PC's media players, the one shown ticked. */
@Composable
private fun PlayerPicker(players: List<String>, current: String, onDismiss: () -> Unit, onPick: (String) -> Unit) {
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text("Players on the PC") },
        text = {
            Column {
                players.forEachIndexed { i, name ->
                    ListRow(
                        title = name.replaceFirstChar { it.uppercase() },
                        icon = Icons.Rounded.MusicNote,
                        iconTint = SystemColor.PINK.color,
                        showDivider = i < players.lastIndex,
                        horizontalPadding = 0.dp,
                        trailing = if (name == current) { { Icon(Icons.Rounded.Check, "Playing", tint = MaterialTheme.colorScheme.primary) } } else null,
                        haptic = Haptic.Select,
                        onClick = { onPick(name) },
                    )
                }
            }
        },
        confirmButton = { DialogButton(onClick = onDismiss) { Text("Close") } },
    )
}
