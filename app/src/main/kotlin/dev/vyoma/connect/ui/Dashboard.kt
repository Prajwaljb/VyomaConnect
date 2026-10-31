package dev.vyoma.connect.ui

import dev.vyoma.connect.designsystem.LocalStatus

import androidx.compose.foundation.layout.widthIn
import dev.vyoma.connect.designsystem.DialogButton
import androidx.compose.ui.text.TextStyle
import dev.vyoma.connect.designsystem.fadingVerticalScroll
import dev.vyoma.connect.designsystem.fadingHorizontalScroll
import dev.vyoma.connect.designsystem.ListRow
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.verticalScroll
import dev.vyoma.connect.designsystem.GlassDialog
import dev.vyoma.connect.protocol.plugin.builtin.RunCommandPlugin
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.draw.drawBehind
import androidx.compose.runtime.withFrameMillis
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.Lifecycle
import dev.vyoma.connect.protocol.plugin.builtin.PcStats
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.Brush
import androidx.compose.foundation.shape.RoundedCornerShape
import dev.vyoma.connect.designsystem.GlassQuality
import dev.vyoma.connect.designsystem.specularEdge
import dev.vyoma.connect.designsystem.toned
import dev.vyoma.connect.designsystem.glassRaised
import dev.vyoma.connect.designsystem.glassCard
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.foundation.text.BasicText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import dev.vyoma.connect.designsystem.IconBadge
import dev.vyoma.connect.designsystem.LiquidSlider
import dev.vyoma.connect.designsystem.isDarkUi
import androidx.compose.material.icons.automirrored.rounded.VolumeMute
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.Glass
import kotlin.math.roundToInt
import android.graphics.ImageDecoder
import androidx.compose.foundation.Image
import androidx.compose.runtime.produceState
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.vyoma.connect.designsystem.PillButton
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import android.content.Intent
import android.net.Uri
import androidx.compose.material.icons.rounded.ScreenShare
import dev.vyoma.connect.protocol.plugin.builtin.CaptureState
import kotlin.math.abs
import kotlin.math.sign
import androidx.compose.ui.graphics.TransformOrigin
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.perform
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.ui.platform.LocalConfiguration
import dev.vyoma.connect.designsystem.MaxContentWidth
import dev.vyoma.connect.designsystem.liquidPress
import androidx.compose.material.icons.rounded.ContentPasteGo
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import dev.vyoma.connect.protocol.plugin.builtin.Transfer
import dev.vyoma.connect.protocol.plugin.builtin.Transfers
import androidx.compose.foundation.gestures.awaitVerticalTouchSlopOrCancellation
import androidx.compose.foundation.gestures.verticalDrag
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.material.icons.rounded.Speaker
import androidx.compose.material.icons.rounded.Check
import androidx.compose.foundation.gestures.horizontalDrag
import androidx.compose.foundation.gestures.awaitHorizontalTouchSlopOrCancellation
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitEachGesture
import android.content.Context
import android.text.format.DateUtils
import android.widget.Toast
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.snap
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.VolumeOff
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.LockOpen
import androidx.compose.material.icons.rounded.MoreHoriz
import androidx.compose.material.icons.rounded.NotificationsActive
import androidx.compose.material.icons.rounded.Send
import androidx.compose.material.icons.rounded.Sensors
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.PhotoCamera
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import dev.vyoma.connect.designsystem.frostedKnob
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.ConnectionOrb
import dev.vyoma.connect.designsystem.OrbState
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.pressScale
import androidx.compose.ui.geometry.Rect
import dev.vyoma.connect.designsystem.LensStyle
import dev.vyoma.connect.designsystem.liquidLens
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.plugin.builtin.AudioSink
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.plugin.builtin.BatteryState
import dev.vyoma.connect.protocol.plugin.builtin.ClipEntry
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SystemVolumePlugin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableStateFlow

/** Live plugin state for one PC; plugins come and go with the link, so this re-reads on every plugin change. */
private class PcState(val battery: BatteryState?, val batteryStale: Boolean, val sink: AudioSink?, val locked: Boolean?, val sinks: List<AudioSink> = emptyList())

@Composable
private fun rememberPcState(vm: AppViewModel, state: DeviceState): PcState {
    val device = remember(state.identity.deviceId, state.plugins) { vm.device(state.identity.deviceId) }
    val battery by (device?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote ?: remember { MutableStateFlow(null) }).collectAsState()
    val sinks by (device?.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.sinks ?: remember { MutableStateFlow(emptyList()) }).collectAsState()
    val locked by (device?.plugin<LockPlugin>(LockPlugin.ID)?.locked ?: remember { MutableStateFlow(null) }).collectAsState()
    val id = state.identity.deviceId
    LaunchedEffect(id, battery) { battery?.let { vm.cacheBattery(id, it) } }
    val cached = remember(id) { vm.cachedBattery(id) }
    return PcState(battery ?: cached, battery == null && cached != null, sinks.firstOrNull { it.isDefault } ?: sinks.firstOrNull(), locked, sinks)
}

/** Hero glyph: the PC in a deep accent squircle whose edge catches the light, with the live orb pinned to its corner. */
@Composable
fun DeviceGlyph(state: DeviceState, size: Dp = 76.dp) {
    Box(Modifier.size(size)) {
        val accent = MaterialTheme.colorScheme.primary
        Box(
            Modifier
                .fillMaxSize()
                .specularEdge(RoundedCornerShape(size * 0.3f), 1.2.dp)
                .clip(RoundedCornerShape(size * 0.3f))
                // A deep shade of the accent, like an app icon: the white glyph stands out on it (a pastel
                // accent left it washed out).
                .background(Brush.linearGradient(listOf(accent.toned(1.2f, 0.64f), accent.toned(1.2f, 0.5f)))),
            contentAlignment = Alignment.Center,
        ) {
            Icon(state.identity.deviceType.icon(), null, tint = Color.White, modifier = Modifier.size(size * 0.5f))
        }
        Box(
            Modifier.align(Alignment.BottomEnd).offset(4.dp, 4.dp).size(22.dp).clip(CircleShape).background(MaterialTheme.colorScheme.background),
            contentAlignment = Alignment.Center,
        ) {
            ConnectionOrb(if (state.isReachable) OrbState.CONNECTED else OrbState.OFFLINE, size = 12.dp)
        }
    }
}

/** Rise ([rise]) and the softer jelly of its height ([wobble]); read only in the draw phase. */
private class Entrance(played: Boolean) {
    val rise = Animatable(if (played) 1f else 0f)
    val wobble = Animatable(if (played) 1f else 0f)
}

@Composable
private fun rememberEntrance(index: Int): Entrance {
    var played by rememberSaveable { mutableStateOf(false) }
    val e = remember { Entrance(played) }
    LaunchedEffect(Unit) {
        if (!played) {
            delay(40L * index)
            launch { e.wobble.animateTo(1f, VyomaMotion.jelly()) }
            e.rise.animateTo(1f, VyomaMotion.spatialSlow())
            played = true
        }
    }
    return e
}

/**
 * Fades and rises in once, staggered by [index], landing like a drop: its height settles on a
 * softer spring than its width, so it stretches a touch and wobbles into shape. Saved with the
 * lazy item, so cards scrolling back into view don't replay it. Animates in the layer only.
 */
@Composable
fun Staggered(index: Int, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val e = rememberEntrance(index)
    Box(
        modifier.graphicsLayer {
            val p = e.rise.value
            alpha = p.coerceIn(0f, 1f)
            compositingStrategy = CompositingStrategy.ModulateAlpha
            translationY = (1f - p) * 28.dp.toPx()
            scaleX = 0.96f + 0.04f * p
            scaleY = 0.93f + 0.07f * e.wobble.value
        },
    ) { content() }
}

/** The connected PC at a glance: battery and clipboard bento, media, volume, then the controls. */
@Composable
fun PcDashboard(
    vm: AppViewModel,
    state: DeviceState,
    section: DashboardSection,
    onTouchpad: () -> Unit,
    onClipboard: () -> Unit,
    modifier: Modifier = Modifier,
    onBrowseFiles: () -> Unit = {},
    onFiles: () -> Unit = {},
) {
    val id = state.identity.deviceId
    val pc = rememberPcState(vm, state)
    var showCommands by rememberSaveable { mutableStateOf(false) }
    if (showCommands) CommandsSheet(vm, id) { showCommands = false }
    val context = LocalContext.current
    val status = LocalStatus.current
    val haptics = LocalHapticFeedback.current
    when (section) {
        // One card for the PC's state: its battery, and with the companion its load and heat, as
        // one row of gauges (a battery ring, a clipboard tile and a gauge card were three).
        DashboardSection.STATUS -> {
            val devices by vm.devices.collectAsStateWithLifecycle()
            val companion = devices.firstOrNull { it.identity.companionOf == id && it.isPaired && it.isReachable }
            val plugin = remember(companion?.identity?.deviceId, companion?.plugins) {
                companion?.let { vm.device(it.identity.deviceId)?.plugin<CompanionPlugin>(CompanionPlugin.ID) }
            }
            var stats: PcStats? = null
            if (plugin != null) {
                // Renew the companion's stats lease only while this card is on screen and the app is visible.
                val lifecycle = LocalLifecycleOwner.current.lifecycle
                LaunchedEffect(plugin) {
                    lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
                        while (true) {
                            plugin.requestStats()
                            delay(STATS_RENEW_MS)
                        }
                    }
                }
                stats = plugin.stats.collectAsStateWithLifecycle().value
            }
            if (pc.battery != null || plugin != null) PcStatus(pc.battery, pc.batteryStale, stats, showLoad = plugin != null, modifier)
        }
        // Everything about sound in one place: the level, and where it plays when there's a choice.
        DashboardSection.VOLUME -> pc.sink?.let { sink ->
            VolumeCapsule(
                sink, onVolume = { vm.setSinkVolume(id, sink.name, it) }, modifier,
                outputs = pc.sinks.takeIf { it.size > 1 }, onOutput = { vm.makeSinkDefault(id, it.name) },
            )
        }
        DashboardSection.CONTROLS -> {
          val devices by vm.devices.collectAsStateWithLifecycle()
          val companion = devices.firstOrNull { it.identity.companionOf == id && it.isPaired && it.isReachable }
          val shots = remember(companion?.identity?.deviceId, companion?.plugins) {
              companion?.let { vm.device(it.identity.deviceId)?.plugin<CompanionPlugin>(CompanionPlugin.ID) }
          }
          val shot by (shots?.screenshot?.state ?: remember { MutableStateFlow(CaptureState.Idle) }).collectAsStateWithLifecycle()
          val photo by (shots?.photo?.state ?: remember { MutableStateFlow(CaptureState.Idle) }).collectAsStateWithLifecycle()
          val commandList by (vm.device(id)?.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.commands ?: remember { MutableStateFlow(emptyList()) }).collectAsStateWithLifecycle()
          CaptureOutcome(shot, "Screenshot")
          CaptureOutcome(photo, "Photo")
          // Live: the Send tile rings with the progress of whatever's moving.
          val transfers by Transfers.all.collectAsStateWithLifecycle()
          val moving = transfers.filter { it.state == Transfer.State.RUNNING }
          val sending = moving.takeIf { it.isNotEmpty() }?.let { m -> m.sumOf { it.doneBytes }.toFloat() / m.sumOf { it.totalBytes }.coerceAtLeast(1) }
          ControlGrid(
            actions = listOf(
                // One job per tap, a second on a long press (held like water, see liquidPress). Slots
                // never move: without the companion, PC files becomes Files (the tab), Screen becomes
                // Ping and Camera becomes Send clipboard. Hues stay off the meters' green/yellow/red.
                Control(Icons.Rounded.UploadFile, "Send files", SystemColor.BLUE, progress = sending, longLabel = "Send photos", onLong = { vm.pickPhotos(context) }) { vm.pickFiles(context) },
                if (shots != null) Control(Icons.Rounded.Folder, "PC files", SystemColor.BLUE, onClick = onBrowseFiles)
                else Control(Icons.Rounded.Folder, "Files", SystemColor.BLUE, onClick = onFiles),
                if (shots != null) Control(Icons.Rounded.ScreenShare, "Screen", SystemColor.PINK, longLabel = "Take a screenshot", onLong = { vm.capture(shots.screenshot) }) { PcScreenActivity.open(context, id) }
                else Control(Icons.Rounded.Sensors, "Ping", SystemColor.CYAN) { vm.ping(id); status.show("Pinged: look for it on the PC") },
                if (shots != null) Control(Icons.Rounded.PhotoCamera, "Camera", SystemColor.ORANGE) { vm.capture(shots.photo) }
                else Control(Icons.Rounded.ContentPasteGo, "Send clip", SystemColor.ORANGE, longLabel = "Clipboard history", onLong = onClipboard) {
                    if (vm.sendClipboard() != null) status.show("Clipboard sent") else status.show("Nothing copied to send", failure = true)
                },
                Control(Icons.Rounded.NotificationsActive, "Find PC", SystemColor.TEAL) { vm.ringPc(id); status.show("Ringing the PC", icon = Icons.Rounded.NotificationsActive, haptic = false) },
                Control(
                    Icons.Rounded.Terminal, "Commands", SystemColor.GRAY, opens = true,
                    longLabel = "Run the last command",
                    onLong = {
                        val last = vm.lastCommand(id)?.let { key -> commandList.firstOrNull { it.key == key } }
                        if (last != null) { vm.runCommand(id, last.key); status.show("Running ${last.name}", icon = Icons.Rounded.Terminal, haptic = false) }
                        else showCommands = true
                    },
                ) { showCommands = true },
            ),
            // Toggles say their state when on ("Locked", "Muted"): the user wants it in words, not
            // only in the fill.
            toggles = listOf(
                Control(
                    if (pc.locked == true) Icons.Rounded.Lock else Icons.Rounded.LockOpen,
                    if (pc.locked == true) "Locked" else "Lock",
                    SystemColor.MINT,
                    active = pc.locked == true,
                    toggle = true,
                ) { vm.setPcLocked(id, pc.locked != true) },
                Control(
                    if (pc.sink?.muted == true) Icons.AutoMirrored.Rounded.VolumeOff else Icons.AutoMirrored.Rounded.VolumeUp,
                    if (pc.sink?.muted == true) "Muted" else "Mute",
                    SystemColor.RED,
                    active = pc.sink?.muted == true,
                    toggle = pc.sink != null,
                ) { pc.sink?.let { vm.setSinkMuted(id, it.name, !it.muted) } ?: status.show("Volume control isn't on for this PC", failure = true) },
            ),
            modifier = modifier,
            // Away, the tiles keep their places but rest, quiet and untappable.
            enabled = state.isReachable,
          )
        }
    }
}

/** Previews a fresh capture in place (it was asked for just now), or says why it failed. */
@Composable
private fun CaptureOutcome(state: CaptureState, what: String) {
    val status = LocalStatus.current
    val haptics = LocalHapticFeedback.current
    var preview by remember { mutableStateOf<CaptureState.Done?>(null) }
    LaunchedEffect(state) {
        // Only react to outcomes that just happened, not ones left over from before a recomposition.
        when (state) {
            is CaptureState.Done -> if (System.currentTimeMillis() - state.at < 5_000) {
                haptics.perform(Haptic.Success)
                preview = state
            }
            is CaptureState.Failed -> if (System.currentTimeMillis() - state.at < 5_000) {
                haptics.perform(Haptic.Failure)
                status.show("$what failed: ${state.reason}", failure = true, haptic = false)
            }
            else -> Unit
        }
    }
    preview?.let { CapturePreview(Uri.parse(it.location), it.mime, what, onDismiss = { preview = null }) }
}

/**
 * The picture on a glass card that springs up from the controls, like iOS's screenshot
 * thumbnail: share it, open it in the gallery, or just close it (it's already in Pictures/Vyoma).
 */
@Composable
private fun CapturePreview(uri: Uri, mime: String, what: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val image by produceState<ImageBitmap?>(null, uri) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
                    // Screen-sized is plenty for a preview; a 4K desktop would otherwise be ~33 MB.
                    val scale = (info.size.width / 1440f).coerceAtLeast(1f)
                    decoder.setTargetSize((info.size.width / scale).roundToInt(), (info.size.height / scale).roundToInt())
                }.asImageBitmap()
            }.getOrNull()
        }
    }
    val appear = remember { Animatable(0f) }
    LaunchedEffect(Unit) { appear.animateTo(1f, VyomaMotion.release()) }
    fun launch(intent: Intent) = runCatching { context.startActivity(intent) }

    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Column(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .graphicsLayer {
                    val p = appear.value
                    alpha = p.coerceIn(0f, 1f)
                    compositingStrategy = CompositingStrategy.ModulateAlpha
                    val sc = 0.86f + 0.14f * p
                    scaleX = sc; scaleY = sc
                    translationY = (1f - p) * 60.dp.toPx()
                }
                // Dialogs get no backdrop blur, so the glass is made solid over the app background.
                .glassCard(MaterialTheme.shapes.extraLarge, fill = Glass.chrome.compositeOver(MaterialTheme.colorScheme.background))
                .padding(14.dp),
        ) {
            Box(
                Modifier.fillMaxWidth().aspectRatio(image?.let { it.width.toFloat() / it.height } ?: (16f / 9f)).clip(MaterialTheme.shapes.large).background(Glass.raised),
                contentAlignment = Alignment.Center,
            ) {
                image?.let { Image(it, what, Modifier.fillMaxSize(), contentScale = ContentScale.Fit) }
            }
            Spacer(Modifier.height(12.dp))
            Text("Saved to Pictures/Vyoma", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 4.dp))
            Spacer(Modifier.height(12.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PillButton("Share", prominent = false, modifier = Modifier.weight(1f), onClick = {
                    launch(
                        Intent.createChooser(
                            Intent(Intent.ACTION_SEND).setType(mime).putExtra(Intent.EXTRA_STREAM, uri)
                                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION),
                            null,
                        ),
                    )
                })
                PillButton("Open", prominent = false, modifier = Modifier.weight(1f), onClick = {
                    launch(Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
                })
                PillButton("Done", modifier = Modifier.weight(1f), onClick = onDismiss)
            }
        }
    }
}

enum class DashboardSection { STATUS, VOLUME, CONTROLS }

private fun toast(context: Context, text: String) =
    Toast.makeText(context, text, Toast.LENGTH_SHORT).show()

/**
 * The PC's volume as iOS Control Center has it: a tall glass capsule beside the media card that
 * fills from the bottom. Drag anywhere on it, relative like iOS (a touch never jumps the level; it
 * swells a little while held); a detent every 10%, a firmer one at the ends. With a choice of
 * outputs, a tap lists them (chips under a wide slider were cut off by the card's edge).
 */
@Composable
internal fun VolumeCapsule(
    sink: AudioSink,
    onVolume: (Int) -> Unit,
    modifier: Modifier = Modifier,
    outputs: List<AudioSink>? = null,
    onOutput: (AudioSink) -> Unit = {},
) {
    val max = sink.maxVolume.coerceAtLeast(1).toFloat()
    var level by remember(sink.name) { mutableFloatStateOf(sink.volume / max) }
    var dragging by remember { mutableStateOf(false) }
    var lastTouch by remember { mutableLongStateOf(0L) }
    var picking by rememberSaveable { mutableStateOf(false) }
    // The PC echoes every value we send, late. Ignore echoes while the finger is down and shortly
    // after; once they stop, settle on whatever the PC really ended up at.
    LaunchedEffect(sink.volume, dragging) {
        if (dragging) return@LaunchedEffect
        delay((ECHO_QUIET_MS - (System.currentTimeMillis() - lastTouch)).coerceAtLeast(0))
        level = sink.volume / max
    }
    val shown = animateFloatAsState(level, if (dragging) snap() else VyomaMotion.spatial(), label = "vol")
    val swell by animateFloatAsState(if (dragging) 1.04f else 1f, VyomaMotion.release(), label = "volSwell")
    val haptics = LocalHapticFeedback.current
    val fill by animateColorAsState(
        if (sink.muted) MaterialTheme.colorScheme.onSurfaceVariant else Glass.accent,
        VyomaMotion.effects(), label = "volFill",
    )

    fun set(v: Float) {
        val n = v.coerceIn(0f, 1f)
        if (n == level) return
        when {
            n == 0f || n == 1f -> haptics.perform(Haptic.Edge)
            (n * 10).toInt() != (level * 10).toInt() -> haptics.perform(Haptic.Detent)
        }
        level = n
        onVolume((n * max).roundToInt())
    }
    val pick by rememberUpdatedState(outputs?.let { { haptics.perform(Haptic.Tap); picking = true } })
    val percent = "${(level * 100).roundToInt()}%"
    Box(
        modifier
            .fillMaxHeight()
            .heightIn(min = DeckMinHeight)
            .graphicsLayer { scaleX = swell; scaleY = swell }
            .semantics(mergeDescendants = true) {
                contentDescription = "PC volume"
                stateDescription = if (sink.muted) "Muted" else percent
                progressBarRangeInfo = ProgressBarRangeInfo(level, 0f..1f)
                setProgress { set(it); true }
                pick?.let { onClick("Choose output") { it(); true } }
            }
            .pointerInput(Unit) {
                awaitEachGesture {
                    val down = awaitFirstDown()
                    val start = awaitVerticalTouchSlopOrCancellation(down.id) { c, _ -> c.consume() }
                    if (start != null) {
                        dragging = true
                        verticalDrag(start.id) { c ->
                            set(level - c.positionChange().y / size.height)
                            c.consume()
                        }
                        lastTouch = System.currentTimeMillis()
                        dragging = false
                    } else {
                        // A tap only if the finger lifted here, not if a scroll took it.
                        val up = currentEvent.changes.firstOrNull { it.id == down.id }
                        if (up != null && !up.pressed && !up.isConsumed) pick?.invoke()
                    }
                }
            }
            .glassCard(MaterialTheme.shapes.large),
    ) {
        // Read while drawing: a glide or a drag never recomposes the card.
        Canvas(Modifier.matchParentSize()) {
            val h = size.height * shown.value
            if (h > 0f) drawRect(fill, topLeft = Offset(0f, size.height - h), size = Size(size.width, h))
        }
        val high = shown.value > 0.86f
        val low = shown.value > 0.14f
        Text(
            if (sink.muted) "Off" else percent,
            style = MaterialTheme.typography.titleSmall.copy(fontFeatureSettings = "tnum"),
            color = if (high) Color.White else MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.align(Alignment.TopCenter).padding(top = 14.dp),
        )
        Icon(
            if (sink.muted) Icons.AutoMirrored.Rounded.VolumeOff else Icons.AutoMirrored.Rounded.VolumeUp, null,
            tint = if (low) Color.White else MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 14.dp).size(24.dp),
        )
    }
    if (picking && outputs != null) OutputDialog(outputs, onDismiss = { picking = false }) { onOutput(it); picking = false }
}

/** Where the PC's sound plays, the current output ticked. */
@Composable
private fun OutputDialog(sinks: List<AudioSink>, onDismiss: () -> Unit, onPick: (AudioSink) -> Unit) {
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text("Play PC sound on") },
        text = {
            Column(Modifier.heightIn(max = 360.dp).fadingVerticalScroll(rememberScrollState())) {
                sinks.forEachIndexed { i, sink ->
                    ListRow(
                        title = prettySinkName(sink.description),
                        icon = Icons.Rounded.Speaker,
                        iconTint = SystemColor.BLUE.color,
                        showDivider = i < sinks.lastIndex,
                        horizontalPadding = 0.dp,
                        trailing = if (sink.isDefault) { { Icon(Icons.Rounded.Check, "Playing here", tint = MaterialTheme.colorScheme.primary) } } else null,
                        haptic = Haptic.Select,
                        onClick = { onPick(sink) },
                    )
                }
            }
        },
        confirmButton = { DialogButton(onClick = onDismiss) { Text("Close") } },
    )
}

/** The media card and the volume capsule share a row at least this tall. */
internal val DeckMinHeight = 148.dp

private const val ECHO_QUIET_MS = 700L


private const val STATS_RENEW_MS = 5_000L

private class Control(
    val icon: ImageVector,
    val label: String,
    val color: SystemColor,
    val active: Boolean = false,
    /** Flips [active]: ticks on/off instead of a plain tap. */
    val toggle: Boolean = false,
    /** Live progress (0..1) ringed round the glyph, e.g. files sending. */
    val progress: Float? = null,
    /** Opens a sheet rather than acting at once: a small chevron says so. */
    val opens: Boolean = false,
    val longLabel: String? = null,
    val onLong: (() -> Unit)? = null,
    val onClick: () -> Unit,
)

/**
 * Apple's Shortcuts widget for the actions, Control Center for the switches. Six one-shot actions
 * are filled square tiles (glyph top left, label bottom left, white ink); the two toggles share
 * one 2-wide glass module in the thumb's corner, each half clear with a coloured glyph when off and
 * filled with its colour when on, so an ON switch never looks like an action (they did as tiles).
 * Corners are concentric (card radius = tile radius + padding); the tile size is shared with the
 * volume capsule above so their edges line up ([deckVolumeWidth]). Rejected before: round buttons
 * (plastic balls), wide tiles two across (a stack of pills), app icons with labels under.
 */
@Composable
private fun ControlGrid(actions: List<Control>, toggles: List<Control>, modifier: Modifier = Modifier, enabled: Boolean = true) {
    BoxWithConstraints(modifier.fillMaxWidth().glassCard(RoundedCornerShape(TileRadius + TilePad)).padding(TilePad)) {
        val tile = (maxWidth - TileGap * 3) / 4
        Column(verticalArrangement = Arrangement.spacedBy(TileGap)) {
            Row(horizontalArrangement = Arrangement.spacedBy(TileGap)) {
                actions.take(4).forEach { ControlTile(it, Modifier.size(tile), enabled) }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(TileGap)) {
                actions.drop(4).forEach { ControlTile(it, Modifier.size(tile), enabled) }
                ToggleModule(toggles, Modifier.size(tile * 2 + TileGap, tile), enabled)
            }
        }
    }
}

/** The capsule beside the media card is one tile column plus the card's padding wide, so it lines up with the grid. */
@Composable
internal fun deckVolumeWidth(): Dp {
    val config = LocalConfiguration.current
    val card = minOf(config.screenWidthDp.dp, MaxContentWidth) - 32.dp
    return (card - TilePad * 2 - TileGap * 3) / 4 + TilePad
}

@Composable
private fun ControlTile(c: Control, modifier: Modifier, enabled: Boolean = true) {
    val tone = c.color.color
    val lum = remember(tone) { tone.luminance() }
    val fill = tileFill(tone, lum)
    val ink = if (lum > 0.5f) Color(0xFF1C1C1E) else Color.White
    Column(
        modifier
            .graphicsLayer { if (!enabled) { alpha = 0.4f; compositingStrategy = CompositingStrategy.ModulateAlpha } }
            .liquidPress(enabled = enabled, onLongClickLabel = c.longLabel, onLongClick = c.onLong, onClick = c.onClick)
            .glassRaised(RoundedCornerShape(TileRadius), fill.copy(alpha = 0.92f))
            .padding(start = 10.dp, end = 6.dp, top = 10.dp, bottom = 10.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
            Box(Modifier.size(26.dp), contentAlignment = Alignment.Center) {
                val progress = c.progress
                if (progress != null) {
                    val sweep = rememberGlide(progress)
                    Canvas(Modifier.fillMaxSize()) {
                        val w = 2.5.dp.toPx()
                        val inset = Offset(w / 2, w / 2)
                        val arc = Size(size.width - w, size.height - w)
                        drawArc(ink.copy(alpha = 0.3f), 0f, 360f, false, inset, arc, style = Stroke(w))
                        drawArc(ink, -90f, 360f * sweep.floatValue, false, inset, arc, style = Stroke(w, cap = StrokeCap.Round))
                    }
                    Icon(c.icon, null, tint = ink, modifier = Modifier.size(14.dp))
                } else {
                    Icon(c.icon, null, tint = ink, modifier = Modifier.size(22.dp))
                }
            }
            Spacer(Modifier.weight(1f))
            if (c.opens) Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, null, tint = ink.copy(alpha = 0.7f), modifier = Modifier.size(18.dp))
        }
        TileLabel(c.label, ink)
    }
}

/** Lock and Mute in one glass module: clear halves with coloured glyphs when off, filled when on. */
@Composable
private fun ToggleModule(toggles: List<Control>, modifier: Modifier, enabled: Boolean) {
    Row(
        modifier
            .graphicsLayer { if (!enabled) { alpha = 0.4f; compositingStrategy = CompositingStrategy.ModulateAlpha } }
            .glassRaised(RoundedCornerShape(TileRadius))
            .padding(ModulePad),
        horizontalArrangement = Arrangement.spacedBy(ModulePad),
    ) {
        toggles.forEach { c ->
            val tone = c.color.color
            val on by animateFloatAsState(if (c.active) 1f else 0f, VyomaMotion.effects(), label = "toggleOn")
            val lum = remember(tone) { tone.luminance() }
            val fill = tileFill(tone, lum)
            val onInk = if (lum > 0.5f) Color(0xFF1C1C1E) else Color.White
            // Off, the glyph carries the colour (deepened in light mode to hold 3:1 on clear glass).
            val offGlyph = if (isDarkUi) tone else lerp(tone, Color.Black, 0.35f)
            val ink = lerp(MaterialTheme.colorScheme.onSurface, onInk, on)
            Column(
                Modifier
                    .weight(1f)
                    .fillMaxHeight()
                    .pressable(enabled = enabled && c.toggle, haptic = if (c.active) Haptic.ToggleOff else Haptic.ToggleOn, onClick = c.onClick)
                    .clip(RoundedCornerShape(TileRadius - ModulePad))
                    .background(fill.copy(alpha = 0.92f * on))
                    .padding(start = 8.dp, end = 4.dp, top = 8.dp, bottom = 6.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
                Icon(c.icon, null, tint = lerp(offGlyph, onInk, on), modifier = Modifier.size(22.dp))
                TileLabel(c.label, ink)
            }
        }
    }
}

/**
 * Legible on every colour: the brightest (yellow) keeps its tone and takes dark ink; mid tones
 * (orange, cyan, teal, gray) drop to 38% lightness at the same hue and saturation so white text
 * holds ~3.5:1 and up. Mixing in black instead turned orange brown.
 */
private fun tileFill(tone: Color, lum: Float) = if (lum in 0.2f..0.5f) tone.toned(lightness = 0.38f) else tone

@Composable
private fun TileLabel(text: String, ink: Color) {
    // One size for every tile (12 sp); at large font scales it shrinks to 10 sp at most, rather than
    // cutting a word (9 sp left "Commands" visibly smaller than its neighbours).
    BasicText(
        text,
        style = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.SemiBold, color = ink),
        maxLines = 1,
        autoSize = TextAutoSize.StepBased(minFontSize = 10.sp, maxFontSize = 12.sp),
    )
}

private val TileRadius = 16.dp
private val TilePad = 12.dp
private val TileGap = 8.dp
private val ModulePad = 4.dp

/** "Just now" for the first minute, then "5 min. ago" and so on. */
internal fun ago(time: Long): CharSequence {
    val now = System.currentTimeMillis()
    return if (now - time < DateUtils.MINUTE_IN_MILLIS) "Just now"
    else DateUtils.getRelativeTimeSpanString(time, now, DateUtils.MINUTE_IN_MILLIS, DateUtils.FORMAT_ABBREV_RELATIVE)
}

/**
 * Colour means how hard it's working, the same on every gauge (temperature has its own cut-offs
 * into the same three): fixed colours per gauge made a quiet GPU look like a warning in orange.
 */
private fun loadColor(f: Float) = when {
    f < 0.7f -> SystemColor.GREEN
    f < 0.9f -> SystemColor.YELLOW
    else -> SystemColor.RED
}

/** A number with its unit smaller and quieter beside it, as on every reading on Home. */
@Composable
private fun Reading(value: String?, unit: String, style: TextStyle) {
    Row(verticalAlignment = Alignment.Bottom) {
        Text(value ?: "–", style = style.copy(fontFeatureSettings = "tnum"), maxLines = 1)
        // A fixed slot for the unit, so digits line up whether it's %, G or °.
        if (value != null) Text(
            unit,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            modifier = Modifier.padding(bottom = 2.dp).widthIn(min = 11.dp),
        )
    }
}

/**
 * The PC at a glance, like an iOS widget: the battery as the hero ring on the left and, with the
 * companion, CPU, memory, GPU and temperature as a compact 2×2 of meters beside it. (Five equal
 * gauges in a row read as a strip with nothing leading.) Without the companion, the battery's
 * state in words takes the right side; without a battery, the meters take the card.
 */
@Composable
private fun PcStatus(battery: BatteryState?, stale: Boolean, stats: PcStats?, showLoad: Boolean, modifier: Modifier = Modifier) {
    Row(
        modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.large).padding(horizontal = 20.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (battery != null) {
            val color = if (battery.charge <= 15 && !battery.charging) SystemColor.RED else SystemColor.GREEN
            // A cached reading shows dimmed until the PC sends a fresh one.
            Box(Modifier.graphicsLayer { alpha = if (stale) 0.5f else 1f }) {
                Gauge(
                    if (battery.charging) "Charging" else "Battery", "${battery.charge}", "%",
                    battery.charge / 100f, color, Modifier, size = 80.dp, stroke = 7.dp,
                    valueStyle = MaterialTheme.typography.headlineSmall,
                )
            }
            Spacer(Modifier.width(20.dp))
        }
        if (!showLoad) {
            if (battery != null) Column(Modifier.weight(1f)) {
                Text(if (battery.charging) "Charging" else if (battery.low) "Battery low" else "On battery", style = MaterialTheme.typography.titleMedium)
                Text(
                    if (stale) "Last known level" else "PC battery",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            return@Row
        }
        val temp = stats?.cpuTemp
        val tempColor = when {
            temp == null -> SystemColor.GRAY
            temp < 65f -> SystemColor.GREEN
            temp < 85f -> SystemColor.YELLOW
            else -> SystemColor.RED
        }
        val memFraction = stats?.takeIf { it.memTotal > 0 }?.let { it.memUsed.toFloat() / it.memTotal } ?: 0f
        val cpu = (stats?.cpu ?: 0f) / 100f
        val gpu = (stats?.gpu ?: 0) / 100f
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                Meter("CPU", stats?.let { "${it.cpu.roundToInt()}" }, "%", cpu, loadColor(cpu), Modifier.weight(1f))
                // "RAM", short enough beside a wide reading like 14.6G ("Memory" wrapped there).
                Meter(
                    "RAM", stats?.takeIf { it.memTotal > 0 }?.let { "%.1f".format(it.memUsed / 1e9) }, "G",
                    memFraction, loadColor(memFraction), Modifier.weight(1f),
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                Meter("GPU", stats?.gpu?.let { "$it" }, "%", gpu, loadColor(gpu), Modifier.weight(1f))
                Meter("Temp", temp?.let { "${it.roundToInt()}" }, "°", (temp ?: 0f) / 100f, tempColor, Modifier.weight(1f))
            }
        }
    }
}

/**
 * A live fraction for drawing: readings arrive every 2 s and wobble by a point or two, so whole
 * percent steps, small moves jumping in one frame and only real changes gliding, keep the redraw
 * (which every glass element over the page repeats) to a fraction of a second.
 */
@Composable
private fun rememberGlide(fraction: Float): androidx.compose.runtime.FloatState {
    val step = (fraction.coerceIn(0f, 1f) * 100).roundToInt() / 100f
    val value = remember { mutableFloatStateOf(step) }
    LaunchedEffect(step) {
        val from = value.floatValue
        // Lighter glass: every frame of a glide costs too much on this phone, so it jumps too.
        if (abs(step - from) < 0.03f || GlassQuality.lite) {
            value.floatValue = step
            return@LaunchedEffect
        }
        // Paced at ~30 fps by hand: a dial needs no more, and at 120 Hz the glide (and every glass
        // element over the page) redrew 4x as often for nothing.
        val start = withFrameMillis { it }
        while (true) {
            val f = ((withFrameMillis { it } - start) / 380f).coerceAtMost(1f)
            value.floatValue = from + (step - from) * FastOutSlowInEasing.transform(f)
            if (f >= 1f) break
            delay(33)
        }
    }
    return value
}

/** A 270° ring open at the bottom, like a speedometer, the reading in its middle and the label under it. */
@Composable
private fun Gauge(
    label: String, value: String?, unit: String, fraction: Float, color: SystemColor, modifier: Modifier,
    size: Dp = 54.dp, stroke: Dp = 6.dp, valueStyle: TextStyle = MaterialTheme.typography.labelLarge,
) {
    val tone = color.color
    val track = Glass.raised
    val gauge = rememberGlide(fraction)
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(size), contentAlignment = Alignment.Center) {
            // Read while drawing: the glide never recomposes.
            Canvas(Modifier.fillMaxSize()) {
                val sweep = gauge.floatValue
                val w = stroke.toPx()
                val inset = Offset(w / 2, w / 2)
                val arc = Size(this.size.width - w, this.size.height - w)
                drawArc(track, 135f, 270f, false, inset, arc, style = Stroke(w, cap = StrokeCap.Round))
                if (sweep > 0f) drawArc(tone, 135f, 270f * sweep, false, inset, arc, style = Stroke(w, cap = StrokeCap.Round))
            }
            Reading(value, unit, valueStyle)
        }
        Spacer(Modifier.height(4.dp))
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** A label and reading over a slim capsule bar: the compact sibling of [Gauge], same colours and track. */
@Composable
private fun Meter(label: String, value: String?, unit: String, fraction: Float, color: SystemColor, modifier: Modifier) {
    val tone = color.color
    val track = Glass.raised
    val level = rememberGlide(fraction)
    Column(modifier) {
        Row(verticalAlignment = Alignment.Bottom) {
            Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            Reading(value, unit, MaterialTheme.typography.titleSmall)
        }
        Spacer(Modifier.height(6.dp))
        Canvas(Modifier.fillMaxWidth().height(6.dp)) {
            val r = CornerRadius(size.height / 2)
            drawRoundRect(track, cornerRadius = r)
            val f = level.floatValue
            if (f > 0f) drawRoundRect(tone, size = Size(maxOf(size.height, size.width * f), size.height), cornerRadius = r)
        }
    }
}

/** The PC's commands, run from where you are; set up (or edited) on the PC itself. */
@Composable
internal fun CommandsSheet(vm: AppViewModel, id: String, onDismiss: () -> Unit) {
    val status = LocalStatus.current
    val device = remember(id) { vm.device(id) }
    val commands by (device?.plugin<RunCommandPlugin>(RunCommandPlugin.ID)?.commands ?: remember { MutableStateFlow(emptyList()) }).collectAsState()
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text("Commands") },
        text = {
            if (commands.isEmpty()) {
                Text("None yet. Add shutdown, sleep or your own scripts on the PC, and run them from here.")
            } else {
                Column(Modifier.heightIn(max = 360.dp).fadingVerticalScroll(rememberScrollState())) {
                    commands.forEachIndexed { i, cmd ->
                        // Names only: the shell line is for editing, on the PC or in its details.
                        ListRow(
                            title = cmd.name,
                            icon = Icons.Rounded.Terminal,
                            iconTint = SystemColor.GRAY.color,
                            showDivider = i < commands.lastIndex,
                            horizontalPadding = 0.dp,
                            onClick = {
                                vm.runCommand(id, cmd.key)
                                status.show("Running ${cmd.name}", icon = Icons.Rounded.Terminal)
                                onDismiss()
                            },
                        )
                    }
                }
            }
        },
        confirmButton = { DialogButton(onClick = onDismiss) { Text("Close") } },
        dismissButton = {
            DialogButton(onClick = { vm.openCommandSetup(id); onDismiss() }) { Text(if (commands.isEmpty()) "Set up on PC" else "Edit") }
        },
    )
}
