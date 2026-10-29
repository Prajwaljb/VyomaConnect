package dev.vyoma.connect.ui

import kotlin.math.roundToInt
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
import androidx.compose.material.icons.rounded.TouchApp
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
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
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.protocol.device.DeviceState
import dev.vyoma.connect.protocol.plugin.builtin.AudioSink
import dev.vyoma.connect.protocol.plugin.builtin.BatteryPlugin
import dev.vyoma.connect.protocol.plugin.builtin.BatteryState
import dev.vyoma.connect.protocol.plugin.builtin.ClipEntry
import dev.vyoma.connect.protocol.plugin.builtin.LockPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SystemVolumePlugin
import kotlinx.coroutines.delay
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

/** Hero glyph: the PC in an accent disc, with the live orb pinned to its corner. */
@Composable
fun DeviceGlyph(state: DeviceState, size: Dp = 76.dp) {
    Box(Modifier.size(size)) {
        Box(
            Modifier.fillMaxSize().clip(CircleShape).background(MaterialTheme.colorScheme.primaryContainer),
            contentAlignment = Alignment.Center,
        ) {
            Icon(state.identity.deviceType.icon(), null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(size * 0.48f))
        }
        Box(
            Modifier.align(Alignment.BottomEnd).padding(2.dp).size(22.dp).clip(CircleShape).background(MaterialTheme.colorScheme.background),
            contentAlignment = Alignment.Center,
        ) {
            ConnectionOrb(if (state.isReachable) OrbState.CONNECTED else OrbState.OFFLINE, size = 12.dp)
        }
    }
}

@Composable
private fun rememberEntrance(index: Int): Float {
    var played by rememberSaveable { mutableStateOf(false) }
    val anim = remember { Animatable(if (played) 1f else 0f) }
    LaunchedEffect(Unit) {
        if (!played) {
            delay(40L * index)
            anim.animateTo(1f, VyomaMotion.spatialSlow())
            played = true
        }
    }
    return anim.value
}

/**
 * Fades and rises in once, staggered by [index]. Saved with the lazy item, so cards scrolling
 * back into view don't replay it.
 */
@Composable
fun Staggered(index: Int, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val p = rememberEntrance(index)
    Box(
        modifier.graphicsLayer {
            alpha = p.coerceIn(0f, 1f)
            translationY = (1f - p) * 28.dp.toPx()
            val s = 0.96f + 0.04f * p
            scaleX = s; scaleY = s
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
) {
    val id = state.identity.deviceId
    val pc = rememberPcState(vm, state)
    val context = LocalContext.current
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
                    toast(context, if (to != null) "Clipboard sent to $to" else "Copy something first")
                },
            )
        }
        DashboardSection.VOLUME -> pc.sink?.let { sink ->
            VolumeBar(
                sink,
                onVolume = { vm.setSinkVolume(id, sink.name, it) },
                modifier = modifier,
            )
        }
        DashboardSection.CONTROLS -> ControlGrid(
            listOf(
                Control(Icons.Rounded.UploadFile, "Files") { vm.pickFiles(context) },
                Control(Icons.Rounded.TouchApp, "Touchpad", onClick = onTouchpad),
                Control(Icons.Rounded.ContentPaste, "Clipboard", onClick = onClipboard),
                Control(
                    if (pc.locked == true) Icons.Rounded.Lock else Icons.Rounded.LockOpen,
                    if (pc.locked == true) "Locked" else "Lock",
                    active = pc.locked == true,
                ) { vm.setPcLocked(id, pc.locked != true) },
                Control(
                    if (pc.sink?.muted == true) Icons.AutoMirrored.Rounded.VolumeOff else Icons.AutoMirrored.Rounded.VolumeUp,
                    if (pc.sink?.muted == true) "Muted" else "Mute",
                    active = pc.sink?.muted == true,
                ) { pc.sink?.let { vm.setSinkMuted(id, it.name, !it.muted) } ?: toast(context, "Volume control isn't on for this PC") },
                Control(Icons.Rounded.NotificationsActive, "Find PC") { vm.ringPc(id) },
                Control(Icons.Rounded.Sensors, "Ping") { vm.ping(id) },
                if (inDetails) Control(Icons.Rounded.Terminal, "Commands") { vm.openCommandSetup(id) }
                else Control(Icons.Rounded.MoreHoriz, "More", onClick = onOpenDevice),
            ),
            modifier,
        )
    }
}

enum class DashboardSection { BENTO, VOLUME, CONTROLS }

private fun toast(context: Context, text: String) =
    Toast.makeText(context, text, Toast.LENGTH_SHORT).show()

/** Square tile: PC battery as the outer ring, this phone as the inner one. */
@Composable
private fun BatteryTile(pc: BatteryState?, stale: Boolean, phone: BatteryState?, modifier: Modifier, onClick: () -> Unit) {
    // A cached reading shows dimmed until the PC sends a fresh one.
    val accent = MaterialTheme.colorScheme.primary.copy(alpha = if (stale) 0.45f else 1f)
    val second = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.75f)
    val track = MaterialTheme.colorScheme.surfaceContainerHigh
    val low = MaterialTheme.colorScheme.error
    val pcSweep by animateFloatAsState((pc?.charge ?: 0) / 100f, VyomaMotion.spatialSlow(), label = "pcRing")
    val phoneSweep by animateFloatAsState((phone?.charge ?: 0) / 100f, VyomaMotion.spatialSlow(), label = "phoneRing")
    val pcColor by animateColorAsState(if (pc != null && pc.charge <= 15 && !pc.charging) low else accent, VyomaMotion.effects(), label = "pcColor")

    Surface(
        modifier.aspectRatio(1f).pressable(pressedScale = 0.97f, onClick = onClick),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLowest,
    ) {
        Column(Modifier.padding(16.dp)) {
            Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Canvas(Modifier.fillMaxHeight().aspectRatio(1f)) {
                    val w = size.minDimension * 0.11f
                    fun ring(inset: Float, sweep: Float, color: Color) {
                        val topLeft = Offset(inset + w / 2, inset + w / 2)
                        val s = Size(size.width - 2 * inset - w, size.height - 2 * inset - w)
                        drawArc(track, 0f, 360f, false, topLeft, s, style = Stroke(w))
                        if (sweep > 0f) drawArc(color, -90f, 360f * sweep.coerceIn(0f, 1f), false, topLeft, s, style = Stroke(w, cap = StrokeCap.Round))
                    }
                    ring(0f, pcSweep, pcColor)
                    ring(w * 1.45f, phoneSweep, second)
                }
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    if (pc?.charging == true) Icon(Icons.Rounded.Bolt, "Charging", tint = accent, modifier = Modifier.size(16.dp))
                    Row(verticalAlignment = Alignment.Bottom) {
                        Text(pc?.let { "${it.charge}" } ?: "–", style = MaterialTheme.typography.titleLarge)
                        if (pc != null) Text("%", style = MaterialTheme.typography.labelSmall, modifier = Modifier.padding(bottom = 3.dp))
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
    Surface(
        modifier.aspectRatio(1f).pressable(pressedScale = 0.97f, onClick = onOpen),
        shape = MaterialTheme.shapes.large,
        color = MaterialTheme.colorScheme.surfaceContainerLowest,
    ) {
        Column(Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Rounded.ContentPaste, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(18.dp))
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
                    Modifier.size(40.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary).pressable(pressedScale = 0.88f, onClick = onSend),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Rounded.Send, "Send to PC", tint = MaterialTheme.colorScheme.onPrimary, modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

/**
 * iOS-style chunky volume: drag or tap anywhere on the bar. Tracks the finger 1:1 while
 * dragging and springs to the PC's value otherwise. A tick at both ends.
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
    val onFill = MaterialTheme.colorScheme.onPrimary

    fun set(v: Float) {
        val n = v.coerceIn(0f, 1f)
        if ((n == 0f || n == 1f) && n != level) haptics.performHapticFeedback(HapticFeedbackType.SegmentTick)
        if (n == level) return
        level = n
        onVolume((n * max).roundToInt())
    }

    Row(modifier.fillMaxWidth().height(64.dp)) {
        BoxWithConstraints(
            Modifier
                .weight(1f)
                .fillMaxHeight()
                .clip(MaterialTheme.shapes.large)
                .background(MaterialTheme.colorScheme.surfaceContainerLowest)
                .pointerInput(sink.name) {
                    // Relative drag like iOS: the bar follows the finger's movement and never jumps
                    // to where it landed, and a vertical swipe still scrolls the page.
                    awaitEachGesture {
                        val down = awaitFirstDown(requireUnconsumed = false)
                        val start = awaitHorizontalTouchSlopOrCancellation(down.id) { change, over ->
                            change.consume()
                            dragging = true
                            set(level + over / size.width)
                        } ?: return@awaitEachGesture
                        horizontalDrag(start.id) { change ->
                            // Read the movement before consuming: consumed changes report zero.
                            val dx = change.positionChange().x
                            change.consume()
                            lastTouch = System.currentTimeMillis()
                            set(level + dx / size.width)
                        }
                        lastTouch = System.currentTimeMillis()
                        dragging = false
                    }
                },
        ) {
            val barWidth = maxWidth.value
            Box(Modifier.fillMaxHeight().fillMaxWidth(shown.coerceIn(0f, 1f)).background(fill))
            Row(
                Modifier.fillMaxSize().padding(horizontal = 18.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // The icon sits on the fill once there's enough of it, so it flips color with it.
                val onBar = shown * barWidth > 52f
                Icon(
                    if (sink.muted) Icons.AutoMirrored.Rounded.VolumeOff else Icons.AutoMirrored.Rounded.VolumeUp, null,
                    tint = if (onBar) onFill else MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.width(10.dp))
                Text(
                    prettySinkName(sink.description),
                    style = MaterialTheme.typography.labelLarge,
                    color = if (shown * barWidth > 160f) onFill else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Text(
                    if (sink.muted) "Muted" else "${(level * 100).roundToInt()}%",
                    style = MaterialTheme.typography.titleSmall,
                    color = if (shown > 0.92f) onFill else MaterialTheme.colorScheme.onSurface,
                )
            }
        }
    }
}

private const val ECHO_QUIET_MS = 700L

private class Control(val icon: ImageVector, val label: String, val active: Boolean = false, val onClick: () -> Unit)

/** Four across, Control Center style: round toggles that fill with the accent while on. */
@Composable
private fun ControlGrid(controls: List<Control>, modifier: Modifier = Modifier) {
    Surface(modifier.fillMaxWidth(), shape = MaterialTheme.shapes.extraLarge, color = MaterialTheme.colorScheme.surfaceContainerLowest) {
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
    val bg by animateColorAsState(
        if (c.active) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHigh,
        VyomaMotion.effects(), label = "ctlBg",
    )
    val fg by animateColorAsState(
        if (c.active) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
        VyomaMotion.effects(), label = "ctlFg",
    )
    Column(modifier.pressable(pressedScale = 0.9f, onClick = c.onClick), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(56.dp).clip(CircleShape).background(bg), contentAlignment = Alignment.Center) {
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
                if (sink.isDefault) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHigh,
                VyomaMotion.effects(), label = "outBg",
            )
            Text(
                prettySinkName(sink.description),
                style = MaterialTheme.typography.labelLarge,
                color = if (sink.isDefault) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                modifier = Modifier.clip(CircleShape).background(bg).pressable { onDefault(sink) }.padding(horizontal = 14.dp, vertical = 9.dp),
            )
        }
    }
}
