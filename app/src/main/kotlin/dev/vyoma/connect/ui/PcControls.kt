package dev.vyoma.connect.ui

import android.app.Application
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Logout
import androidx.compose.material.icons.automirrored.rounded.VolumeOff
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.Bedtime
import androidx.compose.material.icons.rounded.BrightnessMedium
import androidx.compose.material.icons.rounded.GraphicEq
import androidx.compose.material.icons.rounded.PowerSettingsNew
import androidx.compose.material.icons.rounded.RestartAlt
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import dev.vyoma.connect.platform.AppLock
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import dev.vyoma.connect.designsystem.BounceIconButton
import dev.vyoma.connect.designsystem.DialogButton
import dev.vyoma.connect.designsystem.Glass
import dev.vyoma.connect.designsystem.GlassDialog
import dev.vyoma.connect.designsystem.Haptic
import dev.vyoma.connect.designsystem.IconBadge
import dev.vyoma.connect.designsystem.ListRow
import dev.vyoma.connect.designsystem.LiquidSlider
import dev.vyoma.connect.designsystem.LocalStatus
import dev.vyoma.connect.designsystem.LiquidReveal
import dev.vyoma.connect.designsystem.SectionCard
import dev.vyoma.connect.designsystem.SectionLabel
import dev.vyoma.connect.designsystem.SystemColor
import dev.vyoma.connect.designsystem.VyomaMotion
import dev.vyoma.connect.designsystem.isDarkUi
import dev.vyoma.connect.designsystem.perform
import dev.vyoma.connect.protocol.plugin.builtin.CompanionPlugin
import dev.vyoma.connect.protocol.plugin.builtin.PcControls
import dev.vyoma.connect.protocol.plugin.builtin.PcPower
import dev.vyoma.connect.container
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** The PC's levels, kept fresh every few seconds while the page is visible; null until the first answer. */
@Composable
fun rememberPcControls(plugin: CompanionPlugin): PcControls? {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(plugin) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                plugin.requestControls()
                delay(CONTROLS_REFRESH_MS)
            }
        }
    }
    return plugin.controls.collectAsStateWithLifecycle().value
}

/** Asks before [action]: every one of them interrupts whoever is at the PC. */
@Composable
fun PcPowerDialog(plugin: CompanionPlugin, pcName: String, action: PcPower, onDismiss: () -> Unit) {
    val status = LocalStatus.current
    val context = LocalContext.current
    GlassDialog(
        onDismissRequest = onDismiss,
        title = { Text("${action.label} $pcName?") },
        text = { Text(action.warning) },
        confirmButton = {
            DialogButton(destructive = action != PcPower.SLEEP, onClick = {
                onDismiss()
                AppLock.ask(context, "To ${action.label.lowercase()} $pcName") {
                    // Not this dialog's scope: it closes before the answer, and took the command with it.
                    (context.applicationContext as Application).container.appScope.launch(Dispatchers.Main) {
                        plugin.power(action)
                            .onSuccess { status.show(action.done, icon = action.icon) }
                            .onFailure { status.show(it.message ?: "The PC refused", failure = true) }
                    }
                }
            }) { Text(action.label) }
        },
        dismissButton = { DialogButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/**
 * Each screen's brightness and the volume of each app playing sound, on Home under the PC's own
 * volume. Sliders own their value while touched and briefly after (the PC's echoes lag). Rows come
 * and go with what's playing, and it's nothing at all with neither (the companion hasn't answered,
 * or a desktop with no dimmable screen and silence): a "nothing playing" row would be noise on Home.
 */
@Composable
internal fun LevelsCard(plugin: CompanionPlugin, modifier: Modifier = Modifier) {
    val c = rememberPcControls(plugin)
    LiquidReveal(c != null && (c.displays.isNotEmpty() || c.apps.isNotEmpty()), modifier) {
        if (c != null) Levels(plugin, c)
    }
}

@Composable
private fun Levels(plugin: CompanionPlugin, c: PcControls) {
    SectionCard(Modifier.animateContentSize(VyomaMotion.spatial())) {
        c.displays.forEach { d ->
            LevelRow(
                key = "d:${d.id}",
                title = d.label.ifEmpty { if (d.internal) "Built-in screen" else "Monitor" },
                icon = Icons.Rounded.BrightnessMedium,
                tint = SystemColor.YELLOW,
                reported = d.level,
                onSet = { plugin.setBrightness(d.id, it) },
                divider = d != c.displays.last() || c.apps.isNotEmpty(),
            )
        }
        c.apps.forEachIndexed { i, a ->
            LevelRow(
                key = "a:${a.id}",
                title = a.name,
                icon = Icons.Rounded.GraphicEq,
                tint = SystemColor.PINK,
                reported = a.volume,
                muted = a.muted,
                onSet = { plugin.setAppVolume(a.id, it) },
                onMute = { plugin.setAppMuted(a.id, !a.muted) },
                divider = i < c.apps.lastIndex,
            )
        }
    }
}

/**
 * One level: icon, name and value on a line, the slider under them, lined up with the name. With
 * [onMute] the value is a speaker button that mutes the app (the badge stays just a badge).
 */
@Composable
private fun LevelRow(
    key: String,
    title: String,
    icon: ImageVector,
    tint: SystemColor,
    reported: Float,
    onSet: (Float) -> Unit,
    divider: Boolean,
    muted: Boolean = false,
    onMute: (() -> Unit)? = null,
) {
    var level by remember(key) { mutableFloatStateOf(reported.coerceIn(0f, 1f)) }
    var dragging by remember { mutableStateOf(false) }
    var lastTouch by remember { mutableLongStateOf(0L) }
    LaunchedEffect(reported, dragging) {
        if (dragging) return@LaunchedEffect
        delay((ECHO_QUIET_MS - (System.currentTimeMillis() - lastTouch)).coerceAtLeast(0))
        level = reported.coerceIn(0f, 1f)
    }
    val shown by animateFloatAsState(level, if (dragging) snap() else VyomaMotion.spatial(), label = "level")
    val haptics = LocalHapticFeedback.current
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    // A boosted app reads its real level, though the slider tops out at 100%.
    val percent = if (!dragging && level >= 1f && reported > 1f) reported else level

    Column(Modifier.fillMaxWidth().padding(start = 20.dp, end = if (onMute != null) 8.dp else 20.dp, top = 12.dp, bottom = 6.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBadge(icon, tint.color)
            Spacer(Modifier.width(16.dp))
            Text(title, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
            Text(
                if (muted) "Muted" else "${(percent * 100).roundToInt()}%",
                style = MaterialTheme.typography.titleSmall.copy(fontFeatureSettings = "tnum"),
                color = quiet,
                textAlign = TextAlign.End,
                modifier = Modifier.padding(start = 12.dp).widthIn(min = 44.dp),
            )
            if (onMute != null) {
                BounceIconButton(onClick = { haptics.perform(if (muted) Haptic.ToggleOn else Haptic.ToggleOff); onMute() }, size = 44.dp) {
                    Icon(
                        if (muted) Icons.AutoMirrored.Rounded.VolumeOff else Icons.AutoMirrored.Rounded.VolumeUp,
                        if (muted) "Unmute $title" else "Mute $title",
                        tint = quiet, modifier = Modifier.size(20.dp),
                    )
                }
            }
        }
        LiquidSlider(
            value = { shown },
            onChange = { v ->
                val n = v.coerceIn(0f, 1f)
                dragging = true
                if (n != level) {
                    when {
                        n == 0f || n == 1f -> haptics.perform(Haptic.Edge)
                        (n * 10).toInt() != (level * 10).toInt() -> haptics.perform(Haptic.Detent)
                    }
                    level = n
                    onSet(n)
                }
            },
            onDone = {
                lastTouch = System.currentTimeMillis()
                dragging = false
            },
            modifier = Modifier.padding(start = 50.dp, end = if (onMute != null) 12.dp else 0.dp),
            fill = if (muted) quiet else Glass.accent,
            track = MaterialTheme.colorScheme.onSurface.copy(alpha = if (isDarkUi) 0.16f else 0.1f),
            underlay = MaterialTheme.colorScheme.surfaceContainerLow,
            edgeHaptics = false,
        )
    }
    if (divider) HorizontalDivider(Modifier.padding(start = 70.dp, end = 20.dp), color = MaterialTheme.colorScheme.outlineVariant)
}

internal val PcPower.label get() = when (this) {
    PcPower.SLEEP -> "Sleep"
    PcPower.RESTART -> "Restart"
    PcPower.SHUTDOWN -> "Shut down"
    PcPower.LOGOUT -> "Log out"
}
internal val PcPower.icon get() = when (this) {
    PcPower.SLEEP -> Icons.Rounded.Bedtime
    PcPower.RESTART -> Icons.Rounded.RestartAlt
    PcPower.SHUTDOWN -> Icons.Rounded.PowerSettingsNew
    PcPower.LOGOUT -> Icons.AutoMirrored.Rounded.Logout
}
private val PcPower.tint get() = when (this) {
    PcPower.SLEEP -> SystemColor.BLUE
    PcPower.RESTART -> SystemColor.ORANGE
    PcPower.SHUTDOWN -> SystemColor.RED
    PcPower.LOGOUT -> SystemColor.GRAY
}
private val PcPower.warning get() = when (this) {
    PcPower.SLEEP -> "Wake it at the PC with a key or the mouse. Vyoma reconnects once it's awake."
    PcPower.RESTART, PcPower.SHUTDOWN, PcPower.LOGOUT -> "Apps on the PC are asked to close. Unsaved work there may ask first."
}
private val PcPower.done get() = when (this) {
    PcPower.SLEEP -> "PC going to sleep"
    PcPower.RESTART -> "PC restarting"
    PcPower.SHUTDOWN -> "PC shutting down"
    PcPower.LOGOUT -> "Logging out of the PC"
}

private const val CONTROLS_REFRESH_MS = 3_000L
private const val ECHO_QUIET_MS = 700L
