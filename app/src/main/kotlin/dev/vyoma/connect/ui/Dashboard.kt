package dev.vyoma.connect.ui

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
private class PcState(val battery: BatteryState?, val batteryStale: Boolean, val sink: AudioSink?, val locked: Boolean?)

@Composable
private fun rememberPcState(vm: AppViewModel, state: DeviceState): PcState {
    val device = remember(state.identity.deviceId, state.plugins) { vm.device(state.identity.deviceId) }
    val battery by (device?.plugin<BatteryPlugin>(BatteryPlugin.ID)?.remote ?: remember { MutableStateFlow(null) }).collectAsState()
    val sinks by (device?.plugin<SystemVolumePlugin>(SystemVolumePlugin.ID)?.sinks ?: remember { MutableStateFlow(emptyList()) }).collectAsState()
    val locked by (device?.plugin<LockPlugin>(LockPlugin.ID)?.locked ?: remember { MutableStateFlow(null) }).collectAsState()
    val id = state.identity.deviceId
    LaunchedEffect(id, battery) { battery?.let { vm.cacheBattery(id, it) } }
    val cached = remember(id) { vm.cachedBattery(id) }
    return PcState(battery ?: cached, battery == null && cached != null, sinks.firstOrNull { it.isDefault } ?: sinks.firstOrNull(), locked)
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
    onOpenDevice: () -> Unit,
    onTouchpad: () -> Unit,
    onClipboard: () -> Unit,
    modifier: Modifier = Modifier,
    inDetails: Boolean = false,
    onBrowseFiles: () -> Unit = {},
) {
    val id = state.identity.deviceId
    val pc = rememberPcState(vm, state)
    val context = LocalContext.current
    val haptics = LocalHapticFeedback.current
    when (section) {
        DashboardSection.BENTO -> Row(modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            val phone by vm.phoneBattery.collectAsStateWithLifecycle()
            BatteryTile(pc.battery, pc.batteryStale, phone, Modifier.weight(1f), onClick = onOpenDevice)
            val clips by vm.clipHistory.collectAsStateWithLifecycle()
            ClipboardTile(
                clips.firstOrNull(),
                Modifier.weight(1f),
                onOpen = onClipboard,
                onSend = {
                    val to = vm.sendClipboard()
                    haptics.perform(if (to != null) Haptic.Success else Haptic.Failure)
                    toast(context, if (to != null) "Clipboard sent to $to" else "Copy something first")
                },
            )
        }
        DashboardSection.VITALS -> {
            val devices by vm.devices.collectAsStateWithLifecycle()
            val companion = devices.firstOrNull { it.identity.companionOf == id && it.isPaired && it.isReachable }
            val plugin = remember(companion?.identity?.deviceId, companion?.plugins) {
                companion?.let { vm.device(it.identity.deviceId)?.plugin<CompanionPlugin>(CompanionPlugin.ID) }
            }
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
                val stats by plugin.stats.collectAsStateWithLifecycle()
                PcVitals(stats, modifier)
            }
        }
        DashboardSection.VOLUME -> pc.sink?.let { sink ->
            VolumeBar(
                sink,
                onVolume = { vm.setSinkVolume(id, sink.name, it) },
                modifier = modifier,
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
                Control(Icons.Rounded.UploadFile, "Send", SystemColor.CYAN) { vm.pickFiles(context) },
                if (shots != null) Control(Icons.Rounded.Folder, "PC files", SystemColor.BLUE, onClick = onBrowseFiles)
                else Control(Icons.Rounded.ContentPaste, "Clipboard", SystemColor.ORANGE, onClick = onClipboard),
                // The touchpad has its own round button by the tab bar; this opens the same screen ready to type.
                Control(Icons.Rounded.Keyboard, "Keyboard", SystemColor.TEAL, onClick = onTouchpad),
                // The live screen; its shutter takes full-resolution screenshots.
                if (shots != null) Control(Icons.Rounded.ScreenShare, "Screen", SystemColor.PINK) { PcScreenActivity.open(context, id) }
                else Control(Icons.Rounded.Sensors, "Ping", SystemColor.GREEN) { vm.ping(id) },
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
                if (inDetails) Control(Icons.Rounded.Terminal, "Commands", SystemColor.GRAY) { vm.openCommandSetup(id) }
                else Control(Icons.Rounded.MoreHoriz, "More", SystemColor.GRAY, onClick = onOpenDevice),
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

enum class DashboardSection { BENTO, VITALS, VOLUME, CONTROLS }

private fun toast(context: Context, text: String) =
    Toast.makeText(context, text, Toast.LENGTH_SHORT).show()

/** Square tile: PC battery as the outer ring, this phone as the inner one. */
@Composable
private fun BatteryTile(pc: BatteryState?, stale: Boolean, phone: BatteryState?, modifier: Modifier, onClick: () -> Unit) {
    // A cached reading shows dimmed until the PC sends a fresh one.
    val accent = SystemColor.GREEN.color.copy(alpha = if (stale) 0.45f else 1f)
    val second = SystemColor.CYAN.color
    val track = Glass.raised
    val low = SystemColor.RED.color
    val pcSweep by animateFloatAsState((pc?.charge ?: 0) / 100f, VyomaMotion.spatialSlow(), label = "pcRing")
    val phoneSweep by animateFloatAsState((phone?.charge ?: 0) / 100f, VyomaMotion.spatialSlow(), label = "phoneRing")
    val pcColor by animateColorAsState(if (pc != null && pc.charge <= 15 && !pc.charging) low else accent, VyomaMotion.effects(), label = "pcColor")

    Box(modifier.aspectRatio(1f).pressable(pressedScale = 0.97f, onClick = onClick).glassCard(MaterialTheme.shapes.large)) {
        Column(Modifier.padding(16.dp)) {
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Canvas(Modifier.fillMaxHeight().aspectRatio(1f)) {
                    val w = size.minDimension * 0.085f
                    fun ring(inset: Float, sweep: Float, color: Color) {
                        val topLeft = Offset(inset + w / 2, inset + w / 2)
                        val s = Size(size.width - 2 * inset - w, size.height - 2 * inset - w)
                        drawArc(track, 0f, 360f, false, topLeft, s, style = Stroke(w))
                        if (sweep > 0f) drawArc(color, -90f, 360f * sweep.coerceIn(0f, 1f), false, topLeft, s, style = Stroke(w, cap = StrokeCap.Round))
                    }
                    ring(0f, pcSweep, pcColor)
                    ring(w * 1.3f, phoneSweep, second)
                }
                // Sized to sit in the inner ring with air around it, digits not jumping as they change.
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    if (pc?.charging == true) Icon(Icons.Rounded.Bolt, "Charging", tint = accent, modifier = Modifier.size(14.dp))
                    Row(verticalAlignment = Alignment.Bottom) {
                        Text(
                            pc?.let { "${it.charge}" } ?: "–",
                            style = MaterialTheme.typography.titleMedium.copy(fontFeatureSettings = "tnum"),
                        )
                        if (pc != null) Text("%", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(start = 1.dp, bottom = 2.dp))
                    }
                }
            }
            Spacer(Modifier.height(10.dp))
            BatteryLegend("PC", pc, pcColor)
            BatteryLegend("Phone", phone, second)
        }
    }
}

@Composable
private fun BatteryLegend(label: String, b: BatteryState?, color: Color) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(8.dp).clip(CircleShape).background(color))
        Spacer(Modifier.width(6.dp))
        Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
        Text(
            b?.let { "${it.charge}%" + if (it.charging) " ⚡" else "" } ?: "—",
            style = MaterialTheme.typography.labelMedium,
        )
    }
}

/** Square tile: the latest clip, where it came from, and a one-tap send. */
@Composable
private fun ClipboardTile(clip: ClipEntry?, modifier: Modifier, onOpen: () -> Unit, onSend: () -> Unit) {
    Box(modifier.aspectRatio(1f).pressable(pressedScale = 0.97f, onClick = onOpen).glassCard(MaterialTheme.shapes.large)) {
        Column(Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Rounded.ContentPaste, null, tint = SystemColor.ORANGE.color, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Clipboard", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Spacer(Modifier.height(8.dp))
            Text(
                clip?.text?.trim() ?: "Copy something and it shows up here",
                style = MaterialTheme.typography.bodyMedium,
                color = if (clip != null) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    clip?.let { (if (it.origin != null) "From PC\n" else "") + ago(it.timestamp) } ?: "",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Box(
                    Modifier.size(40.dp).pressable(pressedScale = 0.84f, haptic = null, glass = true, onClick = onSend).clip(CircleShape).background(MaterialTheme.colorScheme.primary),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Rounded.Send, "Send to PC", tint = MaterialTheme.colorScheme.onPrimary, modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

/**
 * The PC's volume, iOS style: the output's name and level over a slim slider with a liquid glass
 * knob, between a quiet and a loud speaker. Drag relative like iOS (a vertical swipe still scrolls
 * the page); it tracks the finger 1:1 and springs to the PC's value otherwise. A detent every 10%,
 * a firmer one at the ends.
 */
@Composable
internal fun VolumeBar(sink: AudioSink, onVolume: (Int) -> Unit, modifier: Modifier = Modifier) {
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
        if (sink.muted) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.primary,
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

    Column(modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.large).padding(start = 18.dp, end = 18.dp, top = 14.dp, bottom = 8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                prettySinkName(sink.description),
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

/** Four across, Control Center style: colored glyphs on glass; toggles fill with their color while on. */
@Composable
private fun ControlGrid(controls: List<Control>, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.extraLarge)) {
        Column(Modifier.padding(vertical = 18.dp, horizontal = 8.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            controls.chunked(4).forEach { row ->
                Row(Modifier.fillMaxWidth()) {
                    row.forEach { c -> ControlButton(c, Modifier.weight(1f)) }
                    repeat(4 - row.size) { Spacer(Modifier.weight(1f)) }
                }
            }
        }
    }
}

@Composable
private fun ControlButton(c: Control, modifier: Modifier) {
    val tone = c.color.color
    val bg by animateColorAsState(if (c.active) tone else Glass.raised, VyomaMotion.effects(), label = "ctlBg")
    val fg by animateColorAsState(if (c.active) Color.White else tone, VyomaMotion.effects(), label = "ctlFg")
    val haptic = when {
        !c.toggle -> Haptic.Tap
        c.active -> Haptic.ToggleOff
        else -> Haptic.ToggleOn
    }
    Column(modifier.pressable(pressedScale = 1f, haptic = haptic, onClick = c.onClick), horizontalAlignment = Alignment.CenterHorizontally) {
        // iOS 26 interactive glass: the circle swells under the finger, clears and lights up where touched.
        Box(Modifier.size(56.dp).pressScale(1f, glass = true).glassRaised(CircleShape, bg.takeIf { c.active || it != Glass.raised }), contentAlignment = Alignment.Center) {
            Icon(c.icon, null, tint = fg, modifier = Modifier.size(24.dp))
        }
        Spacer(Modifier.height(6.dp))
        Text(c.label, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** "Just now" for the first minute, then "5 min. ago" and so on. */
internal fun ago(time: Long): CharSequence {
    val now = System.currentTimeMillis()
    return if (now - time < DateUtils.MINUTE_IN_MILLIS) "Just now"
    else DateUtils.getRelativeTimeSpanString(time, now, DateUtils.MINUTE_IN_MILLIS, DateUtils.FORMAT_ABBREV_RELATIVE)
}

/** Where the PC's sound goes: one chip per output, the default one filled. */
@Composable
internal fun OutputPicker(sinks: List<AudioSink>, onDefault: (AudioSink) -> Unit, modifier: Modifier = Modifier) {
    Row(modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        sinks.sortedByDescending { it.isDefault }.forEach { sink ->
            val bg by animateColorAsState(
                if (sink.isDefault) MaterialTheme.colorScheme.primary else Glass.raised,
                VyomaMotion.effects(), label = "outBg",
            )
            Text(
                prettySinkName(sink.description),
                style = MaterialTheme.typography.labelLarge,
                color = if (sink.isDefault) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                modifier = Modifier.glassRaised(CircleShape, bg.takeIf { sink.isDefault }).pressable { onDefault(sink) }.padding(horizontal = 14.dp, vertical = 9.dp),
            )
        }
    }
}

/** CPU, memory, GPU and temperature as four small gauges, Control Center style. */
@Composable
private fun PcVitals(stats: PcStats?, modifier: Modifier = Modifier) {
    val temp = stats?.cpuTemp
    val tempColor = when {
        temp == null -> SystemColor.GRAY
        temp < 65f -> SystemColor.GREEN
        temp < 85f -> SystemColor.YELLOW
        else -> SystemColor.RED
    }
    val memFraction = stats?.takeIf { it.memTotal > 0 }?.let { it.memUsed.toFloat() / it.memTotal } ?: 0f
    Row(modifier.fillMaxWidth().glassCard(MaterialTheme.shapes.large).padding(vertical = 16.dp)) {
        Gauge("CPU", stats?.let { "${it.cpu.roundToInt()}%" }, (stats?.cpu ?: 0f) / 100f, SystemColor.BLUE, Modifier.weight(1f))
        Gauge(
            "Memory", stats?.takeIf { it.memTotal > 0 }?.let { "%.1f".format(it.memUsed / 1e9) + "G" },
            memFraction, SystemColor.MINT, Modifier.weight(1f),
        )
        Gauge("GPU", stats?.gpu?.let { "$it%" }, (stats?.gpu ?: 0) / 100f, SystemColor.ORANGE, Modifier.weight(1f))
        Gauge("Temp", temp?.let { "${it.roundToInt()}°" }, (temp ?: 0f) / 100f, tempColor, Modifier.weight(1f))
    }
}

@Composable
private fun Gauge(label: String, value: String?, fraction: Float, color: SystemColor, modifier: Modifier) {
    val tone = color.color
    val track = Glass.raised
    // Readings arrive every 2 s and wobble by a point or two. Whole percent steps and a short glide
    // keep the redraw (which every glass element over the page repeats) to a fraction of a second.
    // Small moves (most updates) jump in one frame; only real changes glide.
    val step = (fraction.coerceIn(0f, 1f) * 100).roundToInt() / 100f
    val gauge = remember { mutableFloatStateOf(step) }
    LaunchedEffect(step) {
        val from = gauge.floatValue
        // Lighter glass: every frame of a glide costs too much on this phone, so it jumps too.
        if (abs(step - from) < 0.03f || GlassQuality.lite) {
            gauge.floatValue = step
            return@LaunchedEffect
        }
        // Paced at ~30 fps by hand: a dial needs no more, and at 120 Hz the glide (and every glass
        // element over the page) redrew 4x as often for nothing.
        val start = withFrameMillis { it }
        while (true) {
            val f = ((withFrameMillis { it } - start) / 380f).coerceAtMost(1f)
            gauge.floatValue = from + (step - from) * FastOutSlowInEasing.transform(f)
            if (f >= 1f) break
            delay(33)
        }
    }
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(58.dp), contentAlignment = Alignment.Center) {
            // Read while drawing: the glide never recomposes.
            Canvas(Modifier.fillMaxSize()) {
                val sweep = gauge.floatValue
                val w = 6.dp.toPx()
                val inset = Offset(w / 2, w / 2)
                val arc = Size(size.width - w, size.height - w)
                // A 270° arc open at the bottom, like a speedometer.
                drawArc(track, 135f, 270f, false, inset, arc, style = Stroke(w, cap = StrokeCap.Round))
                if (sweep > 0f) drawArc(tone, 135f, 270f * sweep, false, inset, arc, style = Stroke(w, cap = StrokeCap.Round))
            }
            Text(value ?: "–", style = MaterialTheme.typography.labelLarge, maxLines = 1)
        }
        Spacer(Modifier.height(4.dp))
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
