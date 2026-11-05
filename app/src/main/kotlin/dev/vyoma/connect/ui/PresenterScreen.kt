package dev.vyoma.connect.ui

import android.os.SystemClock
import android.view.KeyEvent
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material.icons.rounded.PlayArrow
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.Tonality
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.vyoma.connect.designsystem.ConnectionOrb
import dev.vyoma.connect.designsystem.Glass
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.OrbState
import dev.vyoma.connect.designsystem.SmoothCornerShape
import dev.vyoma.connect.designsystem.aurora
import dev.vyoma.connect.designsystem.glassCard
import dev.vyoma.connect.designsystem.glassRaised
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.designsystem.pressScale
import dev.vyoma.connect.designsystem.pressable
import dev.vyoma.connect.presenter.PointerGyro
import dev.vyoma.connect.protocol.plugin.builtin.MousePadPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PresenterPlugin
import dev.vyoma.connect.protocol.plugin.builtin.SpecialKey
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** Lets the screen on top take the phone's hardware keys ([MainActivity.dispatchKeyEvent]). */
object HardwareKeys {
    /** Returns true if it took the event. */
    var handler: ((KeyEvent) -> Boolean)? = null
}

/**
 * The phone as a slide clicker: Next and Previous (also the volume keys, so it works without looking),
 * start and end the show with a timer, black the screen, and hold to point with the phone. Keys are
 * what stock clients send (Page Down, Page Up, F5, Esc, B), which LibreOffice Impress and PowerPoint
 * understand; the pointer is KDE Connect's own overlay. The screen stays on while this is open.
 */
@Composable
fun PresenterScreen(vm: AppViewModel, deviceId: String?, onBack: () -> Unit) {
    val devices by vm.devices.collectAsStateWithLifecycle()
    val target = devices.firstOrNull { it.identity.deviceId == deviceId && it.isReachable && it.isPaired }
        ?: devices.firstOrNull { it.isPaired && it.isReachable && !it.isCompanion }
    val pad: MousePadPlugin? = remember(target?.plugins) { target?.let { vm.device(it.identity.deviceId)?.plugin(MousePadPlugin.ID) } }
    val pointer: PresenterPlugin? = remember(target?.plugins) { target?.let { vm.device(it.identity.deviceId)?.plugin(PresenterPlugin.ID) } }
    val scope = rememberCoroutineScope()
    fun press(key: SpecialKey) { pad?.let { p -> scope.launch { p.key(key) } } }

    val view = LocalView.current
    DisposableEffect(view) {
        view.keepScreenOn = true
        onDispose { view.keepScreenOn = false }
    }
    // Volume keys change slides. Both directions of each press are taken, or the volume panel still opens.
    val keys by rememberUpdatedState(::press)
    DisposableEffect(Unit) {
        HardwareKeys.handler = handler@{ e ->
            val key = when (e.keyCode) {
                KeyEvent.KEYCODE_VOLUME_DOWN -> SpecialKey.PAGE_DOWN
                KeyEvent.KEYCODE_VOLUME_UP -> SpecialKey.PAGE_UP
                else -> return@handler false
            }
            if (e.action == KeyEvent.ACTION_DOWN && e.repeatCount == 0) keys(key)
            true
        }
        onDispose { HardwareKeys.handler = null }
    }

    // When the show began (elapsedRealtime, 0 = not running) and how long the last one ran.
    var startedAt by rememberSaveable { mutableStateOf(0L) }
    var lastRun by rememberSaveable { mutableStateOf(0L) }
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(startedAt) {
        while (isActive && startedAt != 0L) {
            now = SystemClock.elapsedRealtime()
            delay(1_000 - (now - startedAt) % 1_000)
        }
    }
    val running = startedAt != 0L

    Surface(Modifier.fillMaxSize().aurora(), color = Color.Transparent, contentColor = MaterialTheme.colorScheme.onBackground) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).navigationBarsPadding()) {
            Row(Modifier.fillMaxWidth().height(64.dp).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(
                    Modifier.size(48.dp).pressable(glass = true, onClick = onBack).glassRaised(CircleShape),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back") }
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text("Presenter", style = MaterialTheme.typography.titleLarge)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        ConnectionOrb(if (target != null) OrbState.CONNECTED else OrbState.OFFLINE, size = 7.dp)
                        Text(
                            target?.identity?.deviceName ?: "No PC connected",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1, overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }

            Column(
                Modifier.weight(1f).fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 12.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                // The time, and what starts and ends the show.
                Column(
                    Modifier.fillMaxWidth().glassCard(SmoothCornerShape(28.dp)).padding(16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        clock(if (running) now - startedAt else lastRun),
                        style = MaterialTheme.typography.displayMedium.copy(fontFeatureSettings = "tnum"),
                        color = if (running) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(12.dp))
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Pill(
                            if (running) "End show" else "Start show",
                            if (running) Icons.Rounded.Stop else Icons.Rounded.PlayArrow,
                            Modifier.weight(1f), pad != null,
                        ) {
                            if (running) {
                                press(SpecialKey.ESCAPE)
                                lastRun = SystemClock.elapsedRealtime() - startedAt
                                startedAt = 0L
                            } else {
                                press(SpecialKey.F5)
                                startedAt = SystemClock.elapsedRealtime()
                                now = startedAt
                            }
                        }
                        Pill("Black screen", Icons.Rounded.Tonality, Modifier.weight(1f), pad != null) {
                            pad?.let { p -> scope.launch { p.chord("b") } }
                        }
                    }
                }

                SlideButton(
                    "Previous", Icons.AutoMirrored.Rounded.KeyboardArrowLeft, Modifier.fillMaxWidth().height(88.dp), pad != null, main = false,
                ) { press(SpecialKey.PAGE_UP) }
                SlideButton(
                    "Next", Icons.AutoMirrored.Rounded.KeyboardArrowRight, Modifier.fillMaxWidth().weight(1f), pad != null, main = true,
                ) { press(SpecialKey.PAGE_DOWN) }
                if (pointer != null) PointerButton(pointer)
                Text(
                    "Volume down for next, up for previous",
                    Modifier.fillMaxWidth(),
                    style = MaterialTheme.typography.labelSmall.copy(textAlign = TextAlign.Center),
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                )
            }
        }
    }
}

private const val POINTER_ALIVE_MS = 200L

/** Minutes and seconds, with hours only once a talk has run that long. */
internal fun clock(ms: Long): String {
    val s = ms.coerceAtLeast(0) / 1_000
    return if (s >= 3_600) "%d:%02d:%02d".format(s / 3_600, s / 60 % 60, s % 60) else "%d:%02d".format(s / 60, s % 60)
}

@Composable
private fun Pill(label: String, icon: ImageVector, modifier: Modifier, enabled: Boolean, onClick: () -> Unit) {
    Row(
        modifier.height(48.dp).pressable(enabled = enabled, glass = true, onClick = onClick).glassRaised(RoundedCornerShape(50)).padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Center,
    ) {
        Icon(icon, null, Modifier.size(18.dp))
        Spacer(Modifier.width(6.dp))
        Text(label, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/**
 * Clear glass even for Next: a filled panel this size would light up the speaker in a dark room.
 * The big one stacks its arrow over the word; the short one keeps them in a row.
 */
@Composable
private fun SlideButton(label: String, icon: ImageVector, modifier: Modifier, enabled: Boolean, main: Boolean, onClick: () -> Unit) {
    val press = modifier.pressable(enabled = enabled, haptic = Haptic.Key, onClick = onClick).glassRaised(SmoothCornerShape(28.dp))
    if (main) {
        Column(press, horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
            Icon(icon, null, Modifier.size(96.dp), tint = MaterialTheme.colorScheme.primary)
            Text(label, style = MaterialTheme.typography.headlineMedium)
        }
    } else {
        Row(press, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.Center) {
            Icon(icon, null, Modifier.size(32.dp))
            Text(label, style = MaterialTheme.typography.titleLarge)
        }
    }
}

/** Held: the phone's movement steers a dot on the PC's screen; let go and it goes away. */
@Composable
private fun PointerButton(plugin: PresenterPlugin) {
    val context = LocalContext.current
    val haptics = LocalHapticFeedback.current
    var pointing by remember { mutableStateOf(false) }
    val travel = remember { FloatArray(2) }
    val gyro = remember { PointerGyro(context) { dx, dy -> travel[0] += dx; travel[1] += dy } }
    if (!gyro.available) return

    DisposableEffect(pointing, plugin) {
        // Kept from now: by the time this is disposed `pointing` already holds its next value.
        val on = pointing
        if (on) {
            travel.fill(0f)
            gyro.start()
        }
        onDispose {
            if (on) {
                gyro.stop()
                plugin.stop()
            }
        }
    }
    // One packet per frame or so, however fast the sensor reads. The first brings the dot up at once;
    // a perfectly still phone still sends a few a second, as KDE hides a dot it stops hearing about.
    LaunchedEffect(pointing, plugin) {
        var sent = 0L
        while (isActive && pointing) {
            val dx = travel[0]
            val dy = travel[1]
            val at = SystemClock.elapsedRealtime()
            if (dx != 0f || dy != 0f || at - sent >= POINTER_ALIVE_MS) {
                travel.fill(0f)
                sent = at
                plugin.move(dx, dy)
            }
            delay(16)
        }
    }
    val ink = if (pointing) Color.White else MaterialTheme.colorScheme.onSurface
    Box(
        Modifier
            .fillMaxWidth()
            .height(72.dp)
            .pressScale(glass = true)
            .semantics { contentDescription = "Hold and move the phone to point" }
            .pointerInput(Unit) {
                awaitEachGesture {
                    awaitFirstDown()
                    pointing = true
                    haptics.perform(Haptic.Hold)
                    try {
                        waitForUpOrCancellation()
                    } finally {
                        pointing = false
                        haptics.perform(Haptic.Detent)
                    }
                }
            }
            .glassRaised(RoundedCornerShape(50), Glass.accent.takeIf { pointing }),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            if (pointing) "Move the phone to point" else "Hold to point",
            style = MaterialTheme.typography.titleMedium,
            color = ink,
        )
    }
}
