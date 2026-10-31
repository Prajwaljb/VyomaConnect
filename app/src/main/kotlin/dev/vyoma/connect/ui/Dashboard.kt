package dev.vyoma.connect.ui

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
) {
    val id = state.identity.deviceId
    val pc = rememberPcState(vm, state)
    var showCommands by rememberSaveable { mutableStateOf(false) }
    if (showCommands) CommandsSheet(vm, id) { showCommands = false }
    val context = LocalContext.current
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
            VolumeBar(
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
          CaptureOutcome(shot, "Screenshot")
          CaptureOutcome(photo, "Photo")
          ControlGrid(
            listOf(
                // One job per button. Send drops phone files in the PC's Downloads (works with plain KDE
                // Connect); PC files browses and uploads into a chosen folder, which needs the companion.
                // Slots never move: the companion swaps Clipboard (also a tile and a tab) for PC files,
                // Ping for Screen and Find PC for Camera (both stay in the PC's details).
                // Files are blue (both file tiles), so Home carries fewer hues.
                Control(Icons.Rounded.UploadFile, "Send files", SystemColor.BLUE) { vm.pickFiles(context) },
                if (shots != null) Control(Icons.Rounded.Folder, "PC files", SystemColor.BLUE, onClick = onBrowseFiles)
                else Control(Icons.Rounded.ContentPaste, "Clipboard", SystemColor.ORANGE, onClick = onClipboard),
                // The touchpad has its own round button by the tab bar; this opens the same screen ready to type.
                Control(Icons.Rounded.Keyboard, "Keyboard", SystemColor.TEAL, onClick = onTouchpad),
                // The live screen; its shutter takes full-resolution screenshots.
                if (shots != null) Control(Icons.Rounded.ScreenShare, "Screen", SystemColor.ORANGE) { PcScreenActivity.open(context, id) }
                else Control(Icons.Rounded.Sensors, "Ping", SystemColor.GREEN) { vm.ping(id) },
                // Toggles say their state when on ("Locked", "Muted"): the user wants it in words, not
                // only in the fill.
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
                ) { pc.sink?.let { vm.setSinkMuted(id, it.name, !it.muted) } ?: toast(context, "Volume control isn't on for this PC") },
                // The companion's camera takes Find PC's slot; Find PC is then in the PC's details.
                if (shots != null) Control(
                    Icons.Rounded.PhotoCamera,
                    if (photo == CaptureState.Capturing) "Taking" else "Camera",
                    SystemColor.YELLOW,
                    active = photo == CaptureState.Capturing,
                ) { vm.capture(shots.photo) }
                else Control(Icons.Rounded.NotificationsActive, "Find PC", SystemColor.YELLOW) { vm.ringPc(id) },
                // The PC's details have their own button in the header now; this slot runs commands.
                Control(Icons.Rounded.Terminal, "Commands", SystemColor.GRAY) { showCommands = true },
            ),
            modifier,
          )
        }
    }
}

/** Previews a fresh capture in place (it was asked for just now), or says why it failed. */
@Composable
private fun CaptureOutcome(state: CaptureState, what: String) {
    val context = LocalContext.current
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
                toast(context, "$what failed: ${state.reason}")
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
 * The PC's volume, iOS style: the output's name and level over a slim slider with a liquid glass
 * knob, between a quiet and a loud speaker. Drag relative like iOS (a vertical swipe still scrolls
 * the page); it tracks the finger 1:1 and springs to the PC's value otherwise. A detent every 10%,
 * a firmer one at the ends.
 */
@Composable
internal fun VolumeBar(
    sink: AudioSink,
    onVolume: (Int) -> Unit,
    modifier: Modifier = Modifier,
    /** With a choice of outputs, they sit under the slider and name it; the title is then just "Volume". */
    outputs: List<AudioSink>? = null,
    onOutput: (AudioSink) -> Unit = {},
) {
    val max = sink.maxVolume.coerceAtLeast(1).toFloat()
    var level by remember(sink.name) { mutableFloatStateOf(sink.volume / max) }
    var dragging by remember { mutableStateOf(false) }
    var lastTouch by remember { mutableLongStateOf(0L) }
    // The PC echoes every value we send, late. Ignore echoes while the finger is down and shortly
    // after; once they stop, settle on whatever the PC really ended up at.
    LaunchedEffect(sink.volume, dragging) {
        if (dragging) return@LaunchedEffect
        delay((ECHO_QUIET_MS - (System.currentTimeMillis() - lastTouch)).coerceAtLeast(0))
        level = sink.volume / max
    }
    val shown by animateFloatAsState(level, if (dragging) snap() else VyomaMotion.spatial(), label = "vol")
    val haptics = LocalHapticFeedback.current
    val fill by animateColorAsState(
        if (sink.muted) MaterialTheme.colorScheme.onSurfaceVariant else Glass.accent,
        VyomaMotion.effects(), label = "volFill",
    )
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    val card = MaterialTheme.colorScheme.surfaceContainerLow

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

    Column(modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.large).padding(start = 20.dp, end = 20.dp, top = 14.dp, bottom = if (outputs != null) 14.dp else 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                if (outputs != null) "Volume" else prettySinkName(sink.description),
                style = MaterialTheme.typography.labelLarge,
                color = muted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Text(
                if (sink.muted) "Muted" else "${(level * 100).roundToInt()}%",
                style = MaterialTheme.typography.titleSmall.copy(fontFeatureSettings = "tnum"),
            )
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.AutoMirrored.Rounded.VolumeMute, null, tint = muted, modifier = Modifier.size(20.dp))
            LiquidSlider(
                value = { if (dragging) level else shown },
                onChange = { dragging = true; set(it) },
                onDone = {
                    lastTouch = System.currentTimeMillis()
                    dragging = false
                },
                modifier = Modifier.weight(1f).padding(horizontal = 12.dp),
                fill = fill,
                track = MaterialTheme.colorScheme.onSurface.copy(alpha = if (isDarkUi) 0.16f else 0.1f),
                underlay = card,
                edgeHaptics = false,
            )
            Icon(if (sink.muted) Icons.AutoMirrored.Rounded.VolumeOff else Icons.AutoMirrored.Rounded.VolumeUp, null, tint = muted, modifier = Modifier.size(20.dp))
        }
        if (outputs != null) OutputPicker(outputs, onOutput, Modifier.padding(top = 4.dp), bleed = 20.dp)
    }
}

private const val ECHO_QUIET_MS = 700L


private const val STATS_RENEW_MS = 5_000L

private class Control(
    val icon: ImageVector,
    val label: String,
    val color: SystemColor,
    val active: Boolean = false,
    /** Flips [active]: ticks on/off instead of a plain tap. */
    val toggle: Boolean = false,
    val onClick: () -> Unit,
)

/**
 * Apple's Shortcuts widget: four across in one glass card, each action a square tile tinted in its
 * colour, glyph top left and label bottom left, in white. A toggle that's off is clear glass with
 * a coloured glyph and fills with its colour when on. Corners are concentric (card radius = tile
 * radius + padding) and the gaps tight and even, as Apple's are. Tried before and rejected: round
 * buttons (plastic balls), wide tiles two across (a stack of pills), app icons with labels under.
 */
@Composable
private fun ControlGrid(controls: List<Control>, modifier: Modifier = Modifier) {
    Column(
        modifier.fillMaxWidth().glassCard(RoundedCornerShape(TileRadius + TilePad)).padding(TilePad),
        verticalArrangement = Arrangement.spacedBy(TileGap),
    ) {
        controls.chunked(4).forEach { row ->
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(TileGap)) {
                row.forEach { c -> ControlTile(c, Modifier.weight(1f)) }
                repeat(4 - row.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

@Composable
private fun ControlTile(c: Control, modifier: Modifier) {
    val tone = c.color.color
    val on by animateFloatAsState(if (!c.toggle || c.active) 1f else 0f, VyomaMotion.effects(), label = "ctlOn")
    val haptic = when {
        !c.toggle -> Haptic.Tap
        c.active -> Haptic.ToggleOff
        else -> Haptic.ToggleOn
    }
    val shape = RoundedCornerShape(TileRadius)
    // Legible on every colour: the brightest (yellow) keeps its tone and takes dark ink; mid tones
    // (cyan, teal, gray) deepen so white text holds 4.5:1 (darkening yellow only made it mustard).
    val lum = remember(tone) { tone.luminance() }
    val fill = if (lum in 0.2f..0.5f) lerp(tone, Color.Black, 0.30f) else tone
    val onFill = if (lum > 0.5f) Color(0xFF1C1C1E) else Color.White
    val ink = lerp(MaterialTheme.colorScheme.onSurface, onFill, on)
    // A toggle that's off is a faint pane of its colour, not clear glass: clear read as disabled
    // beside six filled tiles. Its glyph deepens in light mode to hold 3:1 on the pale pane.
    val offGlyph = if (isDarkUi) tone else lerp(tone, Color.Black, 0.35f)
    Column(
        modifier
            .aspectRatio(1f)
            .pressable(haptic = haptic, onClick = c.onClick)
            .glassRaised(shape, fill.copy(alpha = 0.22f + 0.70f * on))
            .padding(start = 10.dp, end = 6.dp, top = 10.dp, bottom = 10.dp),
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        Icon(c.icon, null, tint = lerp(offGlyph, onFill, on), modifier = Modifier.size(22.dp))
        // One size for every tile (12 sp fits "Commands"); shrinks only past that, at large font
        // scales, rather than cutting a word.
        BasicText(
            c.label,
            style = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.SemiBold, color = ink),
            maxLines = 1,
            autoSize = TextAutoSize.StepBased(minFontSize = 9.sp, maxFontSize = 12.sp),
        )
    }
}

private val TileRadius = 18.dp
private val TilePad = 10.dp
private val TileGap = 8.dp

/** "Just now" for the first minute, then "5 min. ago" and so on. */
internal fun ago(time: Long): CharSequence {
    val now = System.currentTimeMillis()
    return if (now - time < DateUtils.MINUTE_IN_MILLIS) "Just now"
    else DateUtils.getRelativeTimeSpanString(time, now, DateUtils.MINUTE_IN_MILLIS, DateUtils.FORMAT_ABBREV_RELATIVE)
}

/** Where the PC's sound goes: one chip per output, the default one filled. */
@Composable
internal fun OutputPicker(sinks: List<AudioSink>, onDefault: (AudioSink) -> Unit, modifier: Modifier = Modifier, bleed: Dp = 16.dp) {
    val chip: @Composable (AudioSink, Modifier) -> Unit = { sink, m ->
        val bg by animateColorAsState(
            if (sink.isDefault) Glass.accent else Glass.raised,
            VyomaMotion.effects(), label = "outBg",
        )
        Text(
            prettySinkName(sink.description),
            style = MaterialTheme.typography.labelLarge,
            color = if (sink.isDefault) Color.White else MaterialTheme.colorScheme.onSurface,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = m.pressable(haptic = Haptic.Select, glass = true) { onDefault(sink) }
                .glassRaised(CircleShape, bg.takeIf { sink.isDefault })
                .padding(horizontal = 14.dp, vertical = 9.dp),
        )
    }
    val sorted = sinks.sortedByDescending { it.isDefault }
    // Full names, scrolling out to the card's edge and fading there (sharing the row truncated three
    // outputs to "Laptop …", worse than scrolling).
    Row(modifier.fadingHorizontalScroll(rememberScrollState(), bleed = bleed).padding(horizontal = bleed), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        sorted.forEach { chip(it, Modifier) }
    }
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
        modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.large).padding(horizontal = 20.dp, vertical = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (battery != null) {
            val color = if (battery.charge <= 15 && !battery.charging) SystemColor.RED else SystemColor.GREEN
            // A cached reading shows dimmed until the PC sends a fresh one.
            Box(Modifier.graphicsLayer { alpha = if (stale) 0.5f else 1f }) {
                Gauge(
                    if (battery.charging) "Charging" else "Battery", "${battery.charge}", "%",
                    battery.charge / 100f, color, Modifier, size = 96.dp, stroke = 8.dp,
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
                Meter(
                    "Memory", stats?.takeIf { it.memTotal > 0 }?.let { "%.1f".format(it.memUsed / 1e9) }, "G",
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
            Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
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
private fun CommandsSheet(vm: AppViewModel, id: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
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
                                toast(context, "Running ${cmd.name}")
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
